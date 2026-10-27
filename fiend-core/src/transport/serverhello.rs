use ml_kem::{Decapsulate, Encapsulate, EncapsulationKey768, Key};
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use super::error::{Error, Result};
use crate::fingerprint::{
    GeneratedKeyShare, KeyShareSecret, KeyShareSpec, P256, Template, X25519, X25519_MLKEM768,
    X25519_MLKEM768_SIZE, X25519_SIZE,
};

pub const REC_HANDSHAKE: u8 = 0x16;
pub const REC_APPDATA: u8 = 0x17;
pub const TLS_VERSION: [u8; 2] = [0x03, 0x03];

const EXT_SUPPORTED_VERSIONS: u16 = 0x002b;
const EXT_KEY_SHARE: u16 = 0x0033;
const TLS13_VERSION: [u8; 2] = [0x03, 0x04];

const MLKEM_CIPHERTEXT_SIZE: usize = 1088;
const MLKEM_PUBLIC_SIZE: usize = 1184;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerHello {
    pub random: [u8; 32],
    pub session_id: Vec<u8>,
    pub cipher_suite: u16,
    pub group: u16,
    pub key_share: Vec<u8>,
}

pub fn build_server_hello(
    random: [u8; 32],
    session_id: &[u8],
    cipher_suite: u16,
    group: u16,
    key: &[u8],
    order: &[u16],
) -> Result<Vec<u8>> {
    if session_id.len() != 32 {
        return Err(Error::invalid("ServerHello session_id must be 32 bytes"));
    }
    if key.len() > u16::MAX as usize - 4 {
        return Err(Error::invalid("ServerHello key_share is too large"));
    }

    let mut extensions = Vec::with_capacity(16 + key.len());
    let mut added_versions = false;
    let mut added_key_share = false;
    let default_order = [EXT_SUPPORTED_VERSIONS, EXT_KEY_SHARE];
    for extension in order.iter().chain(default_order.iter()).copied() {
        match extension {
            EXT_SUPPORTED_VERSIONS if !added_versions => {
                added_versions = true;
                extensions.extend_from_slice(&EXT_SUPPORTED_VERSIONS.to_be_bytes());
                extensions.extend_from_slice(&2u16.to_be_bytes());
                extensions.extend_from_slice(&TLS13_VERSION);
            }
            EXT_KEY_SHARE if !added_key_share => {
                added_key_share = true;
                let share_len = key.len() as u16 + 4;
                extensions.extend_from_slice(&EXT_KEY_SHARE.to_be_bytes());
                extensions.extend_from_slice(&share_len.to_be_bytes());
                extensions.extend_from_slice(&group.to_be_bytes());
                extensions.extend_from_slice(&(key.len() as u16).to_be_bytes());
                extensions.extend_from_slice(key);
            }
            _ => {}
        }
    }
    if !added_versions || !added_key_share {
        return Err(Error::invalid(
            "ServerHello is missing a required extension",
        ));
    }
    if extensions.len() > u16::MAX as usize {
        return Err(Error::invalid("ServerHello extensions are too large"));
    }

    let body_len = 4 + 2 + 32 + 1 + 32 + 2 + 1 + 2 + extensions.len();
    if body_len > u16::MAX as usize {
        return Err(Error::invalid("ServerHello is too large"));
    }

    let mut record = Vec::with_capacity(5 + body_len);
    record.extend_from_slice(&[REC_HANDSHAKE, TLS_VERSION[0], TLS_VERSION[1]]);
    record.extend_from_slice(&(body_len as u16).to_be_bytes());
    record.push(0x02);
    record.extend_from_slice(&(body_len as u32 - 4).to_be_bytes()[1..]);
    record.extend_from_slice(&TLS_VERSION);
    record.extend_from_slice(&random);
    record.push(session_id.len() as u8);
    record.extend_from_slice(session_id);
    record.extend_from_slice(&cipher_suite.to_be_bytes());
    record.push(0);
    record.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    record.extend_from_slice(&extensions);
    Ok(record)
}

pub fn parse_server_hello(record: &[u8]) -> Result<ServerHello> {
    if record.len() < 5 || record[0] != REC_HANDSHAKE {
        return Err(Error::invalid("invalid ServerHello record"));
    }
    let record_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    if record_len != record.len() - 5 {
        return Err(Error::invalid("invalid ServerHello record length"));
    }
    let body = &record[5..];
    if body.len() < 4 || body[0] != 0x02 {
        return Err(Error::invalid("not a ServerHello"));
    }
    let body_len = u32::from_be_bytes([0, body[1], body[2], body[3]]) as usize;
    if body_len != body.len() - 4 || body_len < 38 {
        return Err(Error::invalid("invalid ServerHello length"));
    }

    let mut position = 4;
    if position + 2 + 32 + 1 > body.len() || body[position..position + 2] != TLS_VERSION {
        return Err(Error::invalid("unsupported ServerHello version"));
    }
    position += 2;
    let mut random = [0u8; 32];
    random.copy_from_slice(&body[position..position + 32]);
    position += 32;
    let session_id_len = body[position] as usize;
    position += 1;
    if session_id_len != 32 || position + session_id_len + 3 > body.len() {
        return Err(Error::invalid("invalid ServerHello session_id"));
    }
    let session_id = body[position..position + session_id_len].to_vec();
    position += session_id_len;
    let cipher_suite = u16::from_be_bytes([body[position], body[position + 1]]);
    position += 2;
    if body[position] != 0 {
        return Err(Error::invalid("unsupported ServerHello compression"));
    }
    position += 1;
    if position + 2 > body.len() {
        return Err(Error::invalid("truncated ServerHello extensions"));
    }
    let extension_end =
        position + 2 + u16::from_be_bytes([body[position], body[position + 1]]) as usize;
    position += 2;
    if extension_end != body.len() {
        return Err(Error::invalid("invalid ServerHello extensions length"));
    }

    let mut versions = false;
    let mut key_share = false;
    let mut group = 0;
    let mut share = Vec::new();
    while position < extension_end {
        if position + 4 > extension_end {
            return Err(Error::invalid("truncated ServerHello extension"));
        }
        let extension = u16::from_be_bytes([body[position], body[position + 1]]);
        let length = u16::from_be_bytes([body[position + 2], body[position + 3]]) as usize;
        position += 4;
        if position + length > extension_end {
            return Err(Error::invalid("truncated ServerHello extension data"));
        }
        let data = &body[position..position + length];
        match extension {
            EXT_SUPPORTED_VERSIONS => {
                if data != TLS13_VERSION {
                    return Err(Error::invalid(
                        "ServerHello lacks TLS 1.3 supported_versions",
                    ));
                }
                versions = true;
            }
            EXT_KEY_SHARE => {
                if data.len() < 4 {
                    return Err(Error::invalid("ServerHello key_share too short"));
                }
                group = u16::from_be_bytes([data[0], data[1]]);
                let key_len = u16::from_be_bytes([data[2], data[3]]) as usize;
                if key_len != data.len() - 4 {
                    return Err(Error::invalid("ServerHello key_share length mismatch"));
                }
                share = data[4..].to_vec();
                key_share = true;
            }
            _ => {}
        }
        position += length;
    }
    if !versions || !key_share {
        return Err(Error::invalid("ServerHello missing required extension"));
    }

    Ok(ServerHello {
        random,
        session_id,
        cipher_suite,
        group,
        key_share: share,
    })
}

pub fn server_hello_extension_order(record: &[u8]) -> Option<Vec<u16>> {
    if record.len() < 5 {
        return None;
    }
    let record_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    if record_len != record.len() - 5 {
        return None;
    }
    let body = &record[5..];
    let mut position = 4 + 2 + 32 + 1;
    if position > body.len() {
        return None;
    }
    let session_id_len = body[position - 1] as usize;
    position += session_id_len + 2 + 1;
    if position + 2 > body.len() {
        return None;
    }
    let end = position + 2 + u16::from_be_bytes([body[position], body[position + 1]]) as usize;
    position += 2;
    if end > body.len() {
        return None;
    }
    let mut order = Vec::new();
    while position + 4 <= end {
        let extension = u16::from_be_bytes([body[position], body[position + 1]]);
        let length = u16::from_be_bytes([body[position + 2], body[position + 3]]) as usize;
        position += 4;
        if position + length > end {
            return None;
        }
        order.push(extension);
        position += length;
    }
    Some(order)
}

pub fn choose_server_share(template: &Template, origin_group: u16) -> Result<KeyShareSpec> {
    if origin_group != 0
        && let Some(spec) = template
            .key_shares
            .iter()
            .find(|spec| spec.group == origin_group)
    {
        return Ok(spec.clone());
    }
    for group in [X25519_MLKEM768, X25519, P256] {
        if let Some(spec) = template.key_shares.iter().find(|spec| spec.group == group) {
            return Ok(spec.clone());
        }
    }
    Err(Error::invalid("ClientHello has no supported key share"))
}

pub fn choose_cipher_suite(template: &Template, origin_suite: u16) -> u16 {
    if origin_suite != 0 && template.cipher_suites.contains(&origin_suite) {
        return origin_suite;
    }
    for suite in [0x1301, 0x1302, 0x1303] {
        if template.cipher_suites.contains(&suite) {
            return suite;
        }
    }
    template.cipher_suites.first().copied().unwrap_or(0x1301)
}

pub fn finished_record_wire_size(suite: u16) -> usize {
    if suite == 0x1302 { 74 } else { 58 }
}

pub struct ServerShare {
    pub group: u16,
    pub public: Vec<u8>,
    pub shared: Vec<u8>,
}

pub fn generate_server_share(spec: &KeyShareSpec) -> Result<ServerShare> {
    match spec.group {
        X25519 => {
            let client = X25519PublicKey::from(
                <[u8; X25519_SIZE]>::try_from(spec.public.as_slice())
                    .map_err(|_| Error::invalid("invalid X25519 client key_share"))?,
            );
            let secret = EphemeralSecret::random_from_rng(&mut rand::rng());
            let public = X25519PublicKey::from(&secret).as_bytes().to_vec();
            let shared = secret.diffie_hellman(&client).as_bytes().to_vec();
            Ok(ServerShare {
                group: X25519,
                public,
                shared,
            })
        }
        P256 => {
            let client = PublicKey::from_sec1_bytes(&spec.public)
                .map_err(|_| Error::invalid("invalid P-256 client key_share"))?;
            let secret = loop {
                let mut bytes = [0u8; 32];
                rand::fill(&mut bytes);
                if let Ok(secret) = SecretKey::from_slice(&bytes) {
                    break secret;
                }
            };
            let public = secret.public_key().to_sec1_point(false).as_bytes().to_vec();
            let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), client.as_affine())
                .raw_secret_bytes()
                .to_vec();
            Ok(ServerShare {
                group: P256,
                public,
                shared,
            })
        }
        X25519_MLKEM768 => {
            if spec.public.len() != X25519_MLKEM768_SIZE {
                return Err(Error::invalid(
                    "invalid X25519MLKEM768 client key_share length",
                ));
            }
            let key = Key::<EncapsulationKey768>::try_from(&spec.public[..MLKEM_PUBLIC_SIZE])
                .map_err(|_| Error::invalid("invalid ML-KEM public key length"))?;
            let encapsulation_key = EncapsulationKey768::new(&key)
                .map_err(|_| Error::invalid("invalid ML-KEM public key"))?;
            let (ciphertext, kem_shared) = encapsulation_key.encapsulate();

            let client_x25519 = X25519PublicKey::from(
                <[u8; X25519_SIZE]>::try_from(&spec.public[MLKEM_PUBLIC_SIZE..])
                    .map_err(|_| Error::invalid("invalid X25519 share in hybrid key_share"))?,
            );
            let x25519_secret = EphemeralSecret::random_from_rng(&mut rand::rng());
            let x25519_public = X25519PublicKey::from(&x25519_secret);
            let x25519_shared = x25519_secret.diffie_hellman(&client_x25519);

            let mut public = Vec::with_capacity(MLKEM_CIPHERTEXT_SIZE + X25519_SIZE);
            public.extend_from_slice(&ciphertext);
            public.extend_from_slice(x25519_public.as_bytes());

            let mut shared = Vec::with_capacity(64);
            shared.extend_from_slice(&kem_shared);
            shared.extend_from_slice(x25519_shared.as_bytes());

            Ok(ServerShare {
                group: X25519_MLKEM768,
                public,
                shared,
            })
        }
        group => Err(Error::invalid(format!(
            "unsupported key_share group 0x{group:04x}"
        ))),
    }
}

pub fn client_shared_secret(share: GeneratedKeyShare, server: &[u8]) -> Result<Vec<u8>> {
    match share.secret {
        KeyShareSecret::X25519(secret) => {
            let public = X25519PublicKey::from(
                <[u8; X25519_SIZE]>::try_from(server)
                    .map_err(|_| Error::invalid("invalid X25519 ServerHello key_share"))?,
            );
            Ok(secret.diffie_hellman(&public).as_bytes().to_vec())
        }
        KeyShareSecret::P256(secret) => {
            let public = PublicKey::from_sec1_bytes(server)
                .map_err(|_| Error::invalid("invalid P-256 ServerHello key_share"))?;
            Ok(
                p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), public.as_affine())
                    .raw_secret_bytes()
                    .to_vec(),
            )
        }
        KeyShareSecret::X25519MlKem768 { mlkem, x25519 } => {
            if server.len() != MLKEM_CIPHERTEXT_SIZE + X25519_SIZE {
                return Err(Error::invalid(
                    "invalid X25519MLKEM768 ServerHello key_share length",
                ));
            }
            let kem_shared = mlkem
                .decapsulate_slice(&server[..MLKEM_CIPHERTEXT_SIZE])
                .map_err(|_| Error::invalid("invalid ML-KEM ciphertext"))?;
            let public = X25519PublicKey::from(
                <[u8; X25519_SIZE]>::try_from(&server[MLKEM_CIPHERTEXT_SIZE..])
                    .map_err(|_| Error::invalid("invalid X25519 ServerHello key_share"))?,
            );
            let mut shared = Vec::with_capacity(64);
            shared.extend_from_slice(&kem_shared);
            shared.extend_from_slice(x25519.diffie_hellman(&public).as_bytes());
            Ok(shared)
        }
    }
}
