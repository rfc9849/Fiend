use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use serde::{Deserialize, Serialize};

use crate::addr::Address;
use crate::fingerprint::{Template, parse_client_hello};

pub const PREFIX: &str = "fiend://";
pub const MAX_LINK: usize = 65536;
pub const MAX_SERVERS: usize = 16;
const MAX_JSON: usize = 65536;
const MAX_HELLO: usize = 16 * 1024 + 5;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("The link is not a valid fiend link")]
    Link,
    #[error("{0}")]
    Invalid(&'static str),
}

#[derive(Clone, Debug)]
pub struct Share {
    pub name: String,
    pub servers: Vec<String>,
    pub secret: [u8; 32],
    pub template: Template,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    n: String,
    s: Vec<String>,
    k: String,
    t: String,
}

pub fn check_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "Name must contain 1 to 128 bytes and no control characters",
        ));
    }
    Ok(())
}

pub fn check_server(server: &str) -> Result<(), Error> {
    const MESSAGE: &str = "Each server must be host:port with a nonzero port";
    if server.is_empty() || server.len() > 261 || server.trim() != server {
        return Err(Error::Invalid(MESSAGE));
    }
    match Address::from_authority(server).map_err(|_| Error::Invalid(MESSAGE))? {
        Address::Ip(address) if address.port() != 0 => Ok(()),
        Address::Domain { host, port }
            if port != 0
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
                }) =>
        {
            Ok(())
        }
        _ => Err(Error::Invalid(MESSAGE)),
    }
}

pub fn check_servers(servers: &[String]) -> Result<(), Error> {
    if servers.is_empty() || servers.len() > MAX_SERVERS {
        return Err(Error::Invalid("A profile needs 1 to 16 servers"));
    }
    for (index, server) in servers.iter().enumerate() {
        check_server(server)?;
        if servers[..index]
            .iter()
            .any(|other| other.eq_ignore_ascii_case(server))
        {
            return Err(Error::Invalid("Each server must be different"));
        }
    }
    Ok(())
}

pub fn parse_servers(text: &str) -> Result<Vec<String>, Error> {
    let servers: Vec<String> = text
        .split([',', '\n', ' ', '\t', '\r'])
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    check_servers(&servers)?;
    Ok(servers)
}

pub fn compact_hello(template: &Template) -> Vec<u8> {
    let mut raw = template.raw.clone();
    let mut zero = |offset: usize, size: usize| {
        if let Some(part) = raw.get_mut(offset..offset.saturating_add(size)) {
            part.fill(0);
        }
    };
    zero(template.tls_random.offset, template.tls_random.size);
    zero(template.session_id.offset, template.session_id.size);
    for spec in &template.key_shares {
        zero(spec.offset, spec.size);
    }
    for field in [
        template.psk_identity,
        template.ticket_age,
        template.psk_binder,
    ]
    .into_iter()
    .flatten()
    {
        zero(field.offset, field.size);
    }
    if let Some(ech) = template.ech {
        zero(ech.enc.offset, ech.enc.size);
        zero(ech.payload.offset, ech.payload.size);
    }
    raw
}

pub fn template_from_hello(raw: &[u8]) -> Result<Template, Error> {
    const MESSAGE: &str = "The TLS template is not valid";
    if raw.len() > MAX_HELLO {
        return Err(Error::Invalid(MESSAGE));
    }
    let template = parse_client_hello(raw).map_err(|_| Error::Invalid(MESSAGE))?;
    template.verify(raw).map_err(|_| Error::Invalid(MESSAGE))?;
    template
        .build(&[0; 32])
        .map_err(|_| Error::Invalid(MESSAGE))?;
    Ok(template)
}

pub fn encode(share: &Share) -> Result<String, Error> {
    check_name(&share.name)?;
    check_servers(&share.servers)?;
    let hello = compact_hello(&share.template);
    template_from_hello(&hello)?;
    let wire = Wire {
        n: share.name.clone(),
        s: share.servers.clone(),
        k: hex::encode(share.secret),
        t: STANDARD.encode(hello),
    };
    let raw = serde_json::to_vec(&wire).map_err(|_| Error::Link)?;
    if raw.len() > MAX_JSON {
        return Err(Error::Invalid("The link content is too large"));
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::best());
    compressor.write_all(&raw).map_err(|_| Error::Link)?;
    let compressed = compressor.finish().map_err(|_| Error::Link)?;
    let link = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(compressed));
    if link.len() > MAX_LINK {
        return Err(Error::Invalid("The link content is too large"));
    }
    Ok(link)
}

pub fn decode(link: &str) -> Result<Share, Error> {
    let link = link.trim();
    if link.len() > MAX_LINK {
        return Err(Error::Link);
    }
    let encoded = link.strip_prefix(PREFIX).ok_or(Error::Link)?;
    if encoded.is_empty()
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(Error::Link);
    }
    let compressed = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| Error::Link)?;
    let mut decoder = ZlibDecoder::new(compressed.as_slice());
    let mut raw = Vec::new();
    decoder
        .by_ref()
        .take(MAX_JSON as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| Error::Link)?;
    if raw.len() > MAX_JSON || !decoder.get_ref().is_empty() {
        return Err(Error::Link);
    }
    let wire: Wire = serde_json::from_slice(&raw).map_err(|_| Error::Link)?;
    check_name(&wire.n)?;
    check_servers(&wire.s)?;
    let secret = decode_secret(&wire.k)?;
    let hello = STANDARD.decode(&wire.t).map_err(|_| Error::Link)?;
    let template = template_from_hello(&hello)?;
    Ok(Share {
        name: wire.n,
        servers: wire.s,
        secret,
        template,
    })
}

pub fn decode_secret(text: &str) -> Result<[u8; 32], Error> {
    const MESSAGE: &str = "Secret must contain 64 hex characters";
    if text.len() != 64 {
        return Err(Error::Invalid(MESSAGE));
    }
    let mut secret = [0u8; 32];
    hex::decode_to_slice(text, &mut secret).map_err(|_| Error::Invalid(MESSAGE))?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = include_str!("../testdata/template.json");

    fn share(servers: &[&str]) -> Share {
        Share {
            name: "Test".into(),
            servers: servers.iter().map(|server| (*server).into()).collect(),
            secret: rand::random(),
            template: Template::from_json(BUNDLED).unwrap(),
        }
    }

    #[test]
    fn round_trip_keeps_order_secret_and_template_fields() {
        let original = share(&["203.0.113.7:443", "node.example:8443", "[2001:db8::1]:443"]);
        let link = encode(&original).unwrap();
        assert!(link.starts_with("fiend://") && !link.starts_with("fiend://v"));
        let decoded = decode(&link).unwrap();
        assert_eq!(decoded.name, original.name);
        assert_eq!(decoded.servers, original.servers);
        assert_eq!(decoded.secret, original.secret);
        let reference = parse_client_hello(&original.template.raw).unwrap();
        assert_eq!(decoded.template.domain, reference.domain);
        assert_eq!(decoded.template.tls_random, reference.tls_random);
        assert_eq!(decoded.template.session_id, reference.session_id);
        assert_eq!(decoded.template.key_share, reference.key_share);
        assert_eq!(decoded.template.grease, reference.grease);
        assert_eq!(decoded.template.raw.len(), original.template.raw.len());
        assert_eq!(encode(&decoded).unwrap(), link);
    }

    #[test]
    fn zeroed_template_passes_checks_and_builds_a_valid_hello() {
        let template = Template::from_json(BUNDLED).unwrap();
        let compact = compact_hello(&template);
        assert!(compact[11..43].iter().all(|byte| *byte == 0));
        assert!(compact[44..76].iter().all(|byte| *byte == 0));
        let restored = template_from_hello(&compact).unwrap();
        let built = restored.build(&[7; 32]).unwrap();
        let parsed = parse_client_hello(&built.record).unwrap();
        assert_eq!(parsed.domain, template.domain);
        assert_ne!(&built.record[11..43], &[0; 32]);
        restored.verify(&template.raw).unwrap();
    }

    #[test]
    fn links_fit_in_a_qr_code() {
        let one = encode(&share(&["203.0.113.7:443"])).unwrap();
        let three = encode(&share(&[
            "203.0.113.7:443",
            "198.51.100.20:443",
            "node-three.example.net:8443",
        ]))
        .unwrap();
        eprintln!(
            "one node: {} chars, three nodes: {} chars",
            one.len(),
            three.len()
        );
        assert!(one.len() < 1200 && three.len() < 1300);
    }

    #[test]
    fn decoder_rejects_bad_input() {
        let valid = encode(&share(&["203.0.113.7:443"])).unwrap();
        let encoded = valid.strip_prefix(PREFIX).unwrap();
        let mut trailing = URL_SAFE_NO_PAD.decode(encoded).unwrap();
        trailing.push(0);
        let mut bomb = ZlibEncoder::new(Vec::new(), Compression::best());
        bomb.write_all(&vec![b' '; MAX_JSON + 1]).unwrap();
        let wire = |name: &str, servers: &[&str], secret: &str, hello: &[u8]| {
            let raw = serde_json::to_vec(&serde_json::json!({
                "n": name, "s": servers, "k": secret, "t": STANDARD.encode(hello)
            }))
            .unwrap();
            let mut compressor = ZlibEncoder::new(Vec::new(), Compression::best());
            compressor.write_all(&raw).unwrap();
            format!(
                "{PREFIX}{}",
                URL_SAFE_NO_PAD.encode(compressor.finish().unwrap())
            )
        };
        let hello = compact_hello(&Template::from_json(BUNDLED).unwrap());
        let secret = "ab".repeat(32);
        for link in [
            "fiend://v1/abc".to_owned(),
            "https://example.com".into(),
            "fiend://".into(),
            "fiend://!!".into(),
            format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(trailing)),
            format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bomb.finish().unwrap())),
            format!("{PREFIX}{}", "A".repeat(MAX_LINK)),
            wire("", &["203.0.113.7:443"], &secret, &hello),
            wire("x", &[], &secret, &hello),
            wire("x", &["no-port"], &secret, &hello),
            wire("x", &["203.0.113.7:0"], &secret, &hello),
            wire("x", &["a b:443"], &secret, &hello),
            wire("x", &["a:443", "A:443"], &secret, &hello),
            wire("x", &["a:443"; 17], &secret, &hello),
            wire("x", &["a:443"], "zz", &hello),
            wire("x", &["a:443"], &secret, &hello[..hello.len() / 2]),
        ] {
            assert!(decode(&link).is_err(), "accepted {link:.40}");
        }
        assert!(decode(&wire("x", &["a:443"], &secret, &hello)).is_ok());
    }

    #[test]
    fn server_lists_parse_from_text() {
        assert_eq!(
            parse_servers("a.example:443, 192.0.2.1:8443\n[2001:db8::1]:443").unwrap(),
            vec!["a.example:443", "192.0.2.1:8443", "[2001:db8::1]:443"]
        );
        assert!(parse_servers("").is_err());
        assert!(parse_servers("a:443,a:443").is_err());
    }
}
