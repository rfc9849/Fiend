use super::error::{Error, Result};
use super::keyshare::{self, KeyShareSpec};
use super::template::{DEFAULT_RECORD_MAX, FieldOffset, GreaseField, Template};

pub fn parse_client_hello(raw: &[u8]) -> Result<Template> {
    if raw.len() < 44 {
        return Err(Error::invalid("record too short"));
    }
    if raw[0] != 0x16 {
        return Err(Error::invalid(format!(
            "not a TLS handshake record (0x{:02x})",
            raw[0]
        )));
    }
    let record_len = u16_at(raw, 3)? as usize;
    if raw.len() < 5 + record_len {
        return Err(Error::invalid("record truncated"));
    }
    if raw[5] != 0x01 {
        return Err(Error::invalid(format!(
            "not a ClientHello (handshake type 0x{:02x})",
            raw[5]
        )));
    }

    let tls_random = FieldOffset {
        offset: 11,
        size: 32,
    };

    let sid_len = raw[43] as usize;
    if sid_len != 32 {
        return Err(Error::invalid(format!(
            "session_id must be 32 bytes, got {sid_len}"
        )));
    }
    let session_id = FieldOffset {
        offset: 44,
        size: 32,
    };

    let mut grease = Vec::new();
    let mut cipher_suites = Vec::new();

    let mut pos = 44 + sid_len;
    if pos + 2 > raw.len() {
        return Err(Error::invalid("truncated at cipher suites length"));
    }
    let cs_len = u16_at(raw, pos)? as usize;
    pos += 2;
    if pos + cs_len > raw.len() {
        return Err(Error::invalid("cipher suites truncated"));
    }
    scan_grease_u16_list(raw, pos, cs_len, &mut grease);
    for [high, low] in raw[pos..pos + cs_len].as_chunks::<2>().0 {
        cipher_suites.push(u16::from_be_bytes([*high, *low]));
    }
    pos += cs_len;

    if pos >= raw.len() {
        return Err(Error::invalid("truncated at compression length"));
    }
    let comp_len = raw[pos] as usize;
    if pos + 1 + comp_len > raw.len() {
        return Err(Error::invalid("compression methods truncated"));
    }
    pos += 1 + comp_len;

    if raw.len() < pos + 2 {
        return Err(Error::invalid("truncated at extensions length"));
    }
    let ext_total_len = u16_at(raw, pos)? as usize;
    let ext_start = pos + 2;
    let ext_end = ext_start + ext_total_len;
    if raw.len() < ext_end {
        return Err(Error::invalid("extensions truncated"));
    }

    let mut domain = None;
    let mut early_data = false;
    let mut key_share = None;
    let mut key_shares = Vec::new();
    let mut psk_identity = None;
    let mut ticket_age = None;
    let mut psk_binder = None;

    let mut p = ext_start;
    while p < ext_end {
        if p + 4 > ext_end {
            return Err(Error::invalid("trailing bytes in extensions"));
        }
        let ext_type = u16_at(raw, p)?;
        let ext_len = u16_at(raw, p + 2)? as usize;
        let ext_data = p + 4;
        if ext_data + ext_len > ext_end {
            return Err(Error::invalid(format!(
                "extension 0x{ext_type:04x} truncated"
            )));
        }

        if keyshare::is_grease(ext_type) {
            grease.push(GreaseField {
                value_offset: p,
                data_offset: (ext_len > 0).then_some(ext_data),
                data_size: (ext_len > 0).then_some(ext_len),
            });
        } else {
            match ext_type {
                0x002a => {
                    if ext_len != 0 {
                        return Err(Error::invalid("early_data extension must be empty"));
                    }
                    early_data = true;
                }
                0x0000 => {
                    domain = Some(parse_sni(&raw[ext_data..ext_data + ext_len])?);
                }
                0x000a => {
                    if ext_len < 2 {
                        return Err(Error::invalid("supported_groups extension too short"));
                    }
                    scan_grease_u16_list(raw, ext_data + 2, ext_len - 2, &mut grease);
                }
                0x000d => {
                    if ext_len < 2 {
                        return Err(Error::invalid("signature_algorithms extension too short"));
                    }
                    scan_grease_u16_list(raw, ext_data + 2, ext_len - 2, &mut grease);
                }
                0x002b => {
                    if ext_len >= 2 {
                        let versions_len = raw[ext_data] as usize;
                        scan_grease_u16_list(raw, ext_data + 1, versions_len, &mut grease);
                    }
                }
                0x0033 => {
                    let (specs, mut greases) = parse_key_share(raw, ext_data, ext_len)?;
                    key_share = Some(FieldOffset {
                        offset: specs[0].offset,
                        size: specs[0].size,
                    });
                    key_shares = specs;
                    grease.append(&mut greases);
                }
                0x0029 => {
                    let parsed = parse_psk(raw, ext_data, ext_len)?;
                    psk_identity = parsed.0;
                    ticket_age = parsed.1;
                    psk_binder = parsed.2;
                }
                _ => {}
            }
        }
        p = ext_data + ext_len;
    }

    let domain = domain.ok_or_else(|| Error::invalid("no SNI extension found"))?;
    let key_share = key_share.ok_or_else(|| Error::invalid("no key_share extension found"))?;

    let template = Template {
        domain,
        raw: raw.to_vec(),
        record_max: DEFAULT_RECORD_MAX,
        tls_random,
        session_id,
        key_share,
        psk_identity,
        ticket_age,
        psk_binder,
        grease,
        key_shares,
        cipher_suites,
        early_data,
    };
    template.validate()?;
    Ok(template)
}

fn u16_at(raw: &[u8], offset: usize) -> Result<u16> {
    let bytes = raw
        .get(offset..offset + 2)
        .ok_or_else(|| Error::invalid("truncated length field"))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn scan_grease_u16_list(raw: &[u8], start: usize, length: usize, out: &mut Vec<GreaseField>) {
    let end = start.saturating_add(length).min(raw.len());
    let mut i = start;
    while i + 1 < end {
        let value = u16::from_be_bytes([raw[i], raw[i + 1]]);
        if keyshare::is_grease(value) {
            out.push(GreaseField {
                value_offset: i,
                data_offset: None,
                data_size: None,
            });
        }
        i += 2;
    }
}

fn parse_sni(data: &[u8]) -> Result<String> {
    if data.len() < 5 {
        return Err(Error::invalid("SNI too short"));
    }
    let name_len = u16::from_be_bytes([data[3], data[4]]) as usize;
    if data.len() < 5 + name_len {
        return Err(Error::invalid("SNI name truncated"));
    }
    String::from_utf8(data[5..5 + name_len].to_vec())
        .map_err(|_| Error::invalid("SNI name is not valid UTF-8"))
}

fn parse_key_share(
    raw: &[u8],
    ext_data: usize,
    ext_len: usize,
) -> Result<(Vec<KeyShareSpec>, Vec<GreaseField>)> {
    if ext_len < 2 || ext_data + ext_len > raw.len() {
        return Err(Error::invalid("key_share too short"));
    }
    let mut grease = Vec::new();
    let mut result = Vec::new();
    let mut p = ext_data + 2;
    let end = ext_data + ext_len;
    let list_len = u16::from_be_bytes([raw[ext_data], raw[ext_data + 1]]) as usize;
    if list_len != ext_len - 2 {
        return Err(Error::invalid("key_share list length mismatch"));
    }
    while p < end {
        if p + 4 > end {
            return Err(Error::invalid("key_share entry truncated"));
        }
        let group = u16::from_be_bytes([raw[p], raw[p + 1]]);
        let kex_len = u16::from_be_bytes([raw[p + 2], raw[p + 3]]) as usize;
        let kex_start = p + 4;
        if kex_start + kex_len > end {
            return Err(Error::invalid("key_share key exchange truncated"));
        }
        if keyshare::is_grease(group) {
            grease.push(GreaseField {
                value_offset: p,
                data_offset: (kex_len > 0).then_some(kex_start),
                data_size: (kex_len > 0).then_some(kex_len),
            });
        } else {
            keyshare::validate_group(group, kex_len)?;
            result.push(KeyShareSpec {
                group,
                offset: kex_start,
                size: kex_len,
                public: raw[kex_start..kex_start + kex_len].to_vec(),
            });
        }
        p = kex_start + kex_len;
    }
    if result.is_empty() {
        return Err(Error::invalid(
            "no non-GREASE key exchange entry in key_share",
        ));
    }
    Ok((result, grease))
}

type PskFields = (
    Option<FieldOffset>,
    Option<FieldOffset>,
    Option<FieldOffset>,
);

fn parse_psk(raw: &[u8], ext_data: usize, ext_len: usize) -> Result<PskFields> {
    if ext_len < 2 {
        return Err(Error::invalid("psk extension too short"));
    }
    let end = ext_data + ext_len;
    let ids_len = u16::from_be_bytes([raw[ext_data], raw[ext_data + 1]]) as usize;
    let p = ext_data + 2;
    if p + ids_len > end || p + 2 > end {
        return Err(Error::invalid("psk identity truncated"));
    }
    let id_len = u16::from_be_bytes([raw[p], raw[p + 1]]) as usize;
    if p + 2 + id_len + 4 > end || 2 + id_len + 4 > ids_len {
        return Err(Error::invalid("PSK identity truncated"));
    }
    let psk_identity = Some(FieldOffset {
        offset: p + 2,
        size: id_len,
    });
    let ticket_age = Some(FieldOffset {
        offset: p + 2 + id_len,
        size: 4,
    });

    let binders_start = ext_data + 2 + ids_len;
    if binders_start + 3 > end {
        return Err(Error::invalid("psk binders truncated"));
    }
    let binder_len = raw[binders_start + 2] as usize;
    if binders_start + 3 + binder_len > end {
        return Err(Error::invalid("psk binder truncated"));
    }
    let psk_binder = Some(FieldOffset {
        offset: binders_start + 3,
        size: binder_len,
    });
    Ok((psk_identity, ticket_age, psk_binder))
}
