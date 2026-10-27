use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::error::{Error, Result};
use super::keyshare::{self, KeyShareSpec, KeyShares};
use super::parse::parse_client_hello;
use crate::crypto;

pub const DEFAULT_RECORD_MAX: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOffset {
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GreaseKind {
    Cipher,
    Group,
    Extension1,
    Extension2,
    Version,
    SignatureAlgorithm,
    #[default]
    Other,
}

impl GreaseKind {
    const COUNT: usize = 7;

    fn index(self) -> usize {
        match self {
            Self::Cipher => 0,
            Self::Group => 1,
            Self::Extension1 => 2,
            Self::Extension2 => 3,
            Self::Version => 4,
            Self::SignatureAlgorithm => 5,
            Self::Other => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GreaseField {
    pub value_offset: usize,
    #[serde(default)]
    pub kind: GreaseKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_offset: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_size: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EchGrease {
    pub extension: usize,
    pub config_id: usize,
    pub enc: FieldOffset,
    pub payload: FieldOffset,
}

const ECH_PAYLOAD_OVERHEAD: usize = 16;
const ECH_PAYLOAD_BLOCKS: std::ops::RangeInclusive<usize> = 4..=7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub domain: String,
    #[serde(with = "raw_hex")]
    pub raw: Vec<u8>,
    #[serde(
        default = "default_record_max",
        deserialize_with = "deserialize_record_max"
    )]
    pub record_max: usize,
    pub tls_random: FieldOffset,
    pub session_id: FieldOffset,
    pub key_share: FieldOffset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psk_identity: Option<FieldOffset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_age: Option<FieldOffset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psk_binder: Option<FieldOffset>,
    #[serde(default)]
    pub grease: Vec<GreaseField>,
    #[serde(skip)]
    pub key_shares: Vec<KeyShareSpec>,
    #[serde(skip)]
    pub cipher_suites: Vec<u16>,
    #[serde(skip)]
    pub early_data: bool,
    #[serde(skip)]
    pub ech: Option<EchGrease>,
    #[serde(skip)]
    pub extensions_offset: usize,
}

pub struct BuiltClientHello {
    pub record: Vec<u8>,
    pub key_shares: KeyShares,
}

impl BuiltClientHello {
    pub fn into_parts(self) -> (Vec<u8>, KeyShares) {
        (self.record, self.key_shares)
    }
}

impl AsRef<[u8]> for BuiltClientHello {
    fn as_ref(&self) -> &[u8] {
        &self.record
    }
}

impl Template {
    pub fn from_json(json: &str) -> Result<Self> {
        let mut template: Self = serde_json::from_str(json)?;
        let parsed = parse_client_hello(&template.raw)?;
        template.key_shares = parsed.key_shares;
        template.cipher_suites = parsed.cipher_suites;
        template.early_data = parsed.early_data;
        template.grease = parsed.grease;
        template.ech = parsed.ech;
        template.extensions_offset = parsed.extensions_offset;
        template.validate()?;
        Ok(template)
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let full = json_path(path.as_ref());
        let data = fs::read(&full).map_err(|source| Error::Read {
            path: full.clone(),
            source,
        })?;
        let json = String::from_utf8(data)
            .map_err(|_| Error::invalid(format!("template {} is not UTF-8", full.display())))?;
        Self::from_json(&json)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let json = self.to_json_pretty()?;
        fs::write(path, json).map_err(|source| Error::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }

    pub fn build(&self, secret: &[u8]) -> Result<BuiltClientHello> {
        self.build_at(secret, now_unix_seconds())
    }

    pub fn build_at(&self, secret: &[u8], unix_time: i64) -> Result<BuiltClientHello> {
        self.build_with(secret, unix_time, true)
    }

    fn build_with(&self, secret: &[u8], unix_time: i64, vary: bool) -> Result<BuiltClientHello> {
        self.validate()?;
        let mut record = self.raw.clone();

        rand::fill(&mut record[self.tls_random.offset..self.tls_random.offset + 32]);
        let hour_bucket = unix_time / 3600;
        let token = crypto::auth_token(
            secret,
            &record[self.tls_random.offset..self.tls_random.offset + 32],
            hour_bucket,
        );
        record[self.session_id.offset..self.session_id.offset + 32].copy_from_slice(&token);

        let mut generated = Vec::with_capacity(self.key_shares.len());
        for spec in &self.key_shares {
            let share = keyshare::generate(spec.group)?;
            if share.public.len() != spec.size {
                return Err(Error::KeyShareSize {
                    group: spec.group,
                    generated: share.public.len(),
                    expected: spec.size,
                });
            }
            record[spec.offset..spec.offset + spec.size].copy_from_slice(&share.public);
            generated.push(share);
        }

        if vary {
            if let Some(field) = self.psk_identity {
                rand::fill(&mut record[field.offset..field.offset + field.size]);
            }
            if let Some(field) = self.ticket_age {
                rand::fill(&mut record[field.offset..field.offset + field.size]);
            }
        }

        if let Some(field) = self.psk_binder {
            rand::fill(&mut record[field.offset..field.offset + field.size]);
        }

        if !self.grease.is_empty() {
            let values = grease_values();
            for field in &self.grease {
                let value = values[field.kind.index()];
                record[field.value_offset..field.value_offset + 2]
                    .copy_from_slice(&value.to_be_bytes());
            }
        }

        if let Some(ech) = self.ech {
            let mut config_id = [0u8; 1];
            rand::fill(&mut config_id);
            record[ech.config_id] = config_id[0];
            let enc = EphemeralSecret::random_from_rng(&mut rand::rng());
            record[ech.enc.offset..ech.enc.offset + ech.enc.size]
                .copy_from_slice(PublicKey::from(&enc).as_bytes());
            rand::fill(&mut record[ech.payload.offset..ech.payload.offset + ech.payload.size]);
            if vary && let Some(size) = ech_payload_size(ech.payload.size) {
                self.resize_ech(&mut record, ech, size);
            }
        }

        Ok(BuiltClientHello {
            record,
            key_shares: KeyShares { shares: generated },
        })
    }

    fn resize_ech(&self, record: &mut Vec<u8>, ech: EchGrease, size: usize) {
        let end = ech.payload.offset + ech.payload.size;
        if size > ech.payload.size {
            let mut extra = vec![0u8; size - ech.payload.size];
            rand::fill(&mut extra[..]);
            record.splice(end..end, extra);
        } else {
            record.drain(end - (ech.payload.size - size)..end);
        }
        let delta = size as i64 - ech.payload.size as i64;
        add_length(record, ech.payload.offset - 2, 2, delta);
        add_length(record, ech.extension + 2, 2, delta);
        add_length(record, self.extensions_offset, 2, delta);
        add_length(record, 6, 3, delta);
        add_length(record, 3, 2, delta);
    }

    pub fn verify(&self, original: &[u8]) -> Result<()> {
        let built = self.build_with(&[0u8; 16], now_unix_seconds(), false)?;
        if built.record.len() != original.len() {
            return Err(Error::invalid(format!(
                "length mismatch: original {} vs built {}",
                original.len(),
                built.record.len()
            )));
        }

        let mut mask = vec![false; original.len()];
        let mut mark = |offset: usize, size: usize| {
            let end = offset.saturating_add(size).min(mask.len());
            if offset < mask.len() {
                mask[offset..end].fill(true);
            }
        };
        mark(self.tls_random.offset, self.tls_random.size);
        mark(self.session_id.offset, self.session_id.size);
        mark(self.key_share.offset, self.key_share.size);
        for spec in &self.key_shares {
            mark(spec.offset, spec.size);
        }
        if let Some(field) = self.psk_identity {
            mark(field.offset, field.size);
        }
        if let Some(field) = self.ticket_age {
            mark(field.offset, field.size);
        }
        if let Some(field) = self.psk_binder {
            mark(field.offset, field.size);
        }
        for field in &self.grease {
            mark(field.value_offset, 2);
        }
        if let Some(ech) = self.ech {
            mark(ech.config_id, 1);
            mark(ech.enc.offset, ech.enc.size);
            mark(ech.payload.offset, ech.payload.size);
        }

        let mut mismatches = Vec::new();
        for (index, (&before, &after)) in original.iter().zip(&built.record).enumerate() {
            if !mask[index] && before != after {
                mismatches.push((index, before, after));
            }
        }
        if mismatches.is_empty() {
            return Ok(());
        }

        let mut details = String::new();
        for (index, before, after) in mismatches.iter().take(10) {
            write!(
                details,
                "\n  byte {index}: original 0x{before:02x} vs built 0x{after:02x}"
            )
            .expect("writing to a string cannot fail");
        }
        Err(Error::Verify {
            count: mismatches.len(),
            details,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.record_max < 1 || self.record_max > DEFAULT_RECORD_MAX {
            return Err(Error::RecordMax {
                value: self.record_max,
                max: DEFAULT_RECORD_MAX,
            });
        }

        let length = self.raw.len();
        check_field("tls_random", self.tls_random, length)?;
        check_field("session_id", self.session_id, length)?;
        check_field("key_share", self.key_share, length)?;
        if let Some(field) = self.psk_identity {
            check_field("psk_identity", field, length)?;
        }
        if let Some(field) = self.ticket_age {
            check_field("ticket_age", field, length)?;
        }
        if let Some(field) = self.psk_binder {
            check_field("psk_binder", field, length)?;
        }

        if self.tls_random.size != 32 {
            return Err(Error::invalid(format!(
                "tls_random must be 32 bytes, got {}",
                self.tls_random.size
            )));
        }
        if self.session_id.size != 32 {
            return Err(Error::invalid(format!(
                "session_id must be 32 bytes, got {}",
                self.session_id.size
            )));
        }
        if self.key_shares.is_empty() {
            return Err(Error::invalid("no parsed key shares"));
        }
        for (index, spec) in self.key_shares.iter().enumerate() {
            let end = spec.offset.checked_add(spec.size).ok_or_else(|| {
                Error::invalid(format!(
                    "key_share[{index}] offset {} + size {} overflows",
                    spec.offset, spec.size
                ))
            })?;
            if end > length {
                return Err(Error::invalid(format!(
                    "key_share[{index}] offset {}+{} exceeds record length",
                    spec.offset, spec.size
                )));
            }
            keyshare::validate_group(spec.group, spec.size)?;
        }

        for (index, field) in self.grease.iter().enumerate() {
            if field
                .value_offset
                .checked_add(2)
                .is_none_or(|end| end > length)
            {
                return Err(Error::invalid(format!(
                    "grease[{index}] value_offset {} exceeds record length",
                    field.value_offset
                )));
            }
            if let (Some(offset), Some(size)) = (field.data_offset, field.data_size)
                && offset.checked_add(size).is_none_or(|end| end > length)
            {
                return Err(Error::invalid(format!(
                    "grease[{index}] data {}+{} exceeds record length",
                    offset, size
                )));
            }
        }
        if let Some(ech) = self.ech {
            check_field("ech enc", ech.enc, length)?;
            check_field("ech payload", ech.payload, length)?;
            if ech.enc.size != 32
                || ech.config_id >= length
                || ech.payload.offset < 2
                || ech.extension.checked_add(4).is_none_or(|end| end > length)
                || self
                    .extensions_offset
                    .checked_add(2)
                    .is_none_or(|end| end > length)
            {
                return Err(Error::invalid("invalid ECH extension offsets"));
            }
        }
        Ok(())
    }
}

fn grease_values() -> [u16; GreaseKind::COUNT] {
    let mut seed = [0u8; GreaseKind::COUNT];
    rand::fill(&mut seed);
    let mut values = seed.map(|byte| u16::from((byte & 0xf0) | 0x0a) * 0x0101);
    let first = GreaseKind::Extension1.index();
    let second = GreaseKind::Extension2.index();
    if values[first] == values[second] {
        values[second] ^= 0x1010;
    }
    values
}

fn ech_payload_size(current: usize) -> Option<usize> {
    let body = current.checked_sub(ECH_PAYLOAD_OVERHEAD)?;
    if body % 32 != 0 || !ECH_PAYLOAD_BLOCKS.contains(&(body / 32)) {
        return None;
    }
    let mut byte = [0u8; 1];
    rand::fill(&mut byte);
    let span = ECH_PAYLOAD_BLOCKS.end() - ECH_PAYLOAD_BLOCKS.start() + 1;
    let blocks = ECH_PAYLOAD_BLOCKS.start() + usize::from(byte[0]) % span;
    Some(32 * blocks + ECH_PAYLOAD_OVERHEAD)
}

fn add_length(record: &mut [u8], offset: usize, width: usize, delta: i64) {
    let field = &mut record[offset..offset + width];
    let current = field
        .iter()
        .fold(0i64, |value, byte| (value << 8) | i64::from(*byte));
    let updated = (current + delta) as u32;
    field.copy_from_slice(&updated.to_be_bytes()[4 - width..]);
}

fn check_field(name: &'static str, field: FieldOffset, length: usize) -> Result<()> {
    if field
        .offset
        .checked_add(field.size)
        .is_none_or(|end| end > length)
    {
        return Err(Error::FieldRange {
            name,
            offset: field.offset,
            size: field.size,
            length,
        });
    }
    Ok(())
}

fn default_record_max() -> usize {
    DEFAULT_RECORD_MAX
}

fn deserialize_record_max<'de, D>(deserializer: D) -> std::result::Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    Ok(if value == 0 {
        DEFAULT_RECORD_MAX
    } else {
        value
    })
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn json_path(path: &Path) -> PathBuf {
    let mut full = path.to_path_buf();
    if !full.to_string_lossy().ends_with(".json") {
        full.as_mut_os_string().push(".json");
    }
    full
}

mod raw_hex {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        hex::decode(value).map_err(serde::de::Error::custom)
    }
}
