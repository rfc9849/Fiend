use std::net::IpAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use super::error::{Error, Result};
use super::serverhello::{
    REC_APPDATA, REC_HANDSHAKE, parse_server_hello, server_hello_extension_order,
};
use crate::fingerprint::DEFAULT_RECORD_MAX;

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_READ_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_RECORD_TIMEOUT: Duration = Duration::from_millis(150);
const PROBE_RECORDS: usize = 8;
const MAX_PROBE_RECORD: usize = DEFAULT_RECORD_MAX + 16 + 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginProfile {
    pub cipher_suite: u16,
    pub key_share_group: u16,
    pub extension_order: Vec<u16>,
    pub record_sizes: Vec<usize>,
}

pub fn build_probe_hello(domain: &str) -> Vec<u8> {
    let mut random = [0u8; 32];
    rand::fill(&mut random);
    let mut session_id = [0u8; 32];
    rand::fill(&mut session_id);
    let secret = EphemeralSecret::random_from_rng(&mut rand::rng());
    let public = X25519PublicKey::from(&secret);

    let mut extensions = Vec::with_capacity(256);
    if domain.parse::<IpAddr>().is_err() {
        let name = domain.as_bytes();
        add_extension(&mut extensions, 0x0000, |data| {
            data.extend_from_slice(&((name.len() + 3) as u16).to_be_bytes());
            data.push(0);
            data.extend_from_slice(&(name.len() as u16).to_be_bytes());
            data.extend_from_slice(name);
        });
    }
    add_extension(&mut extensions, 0x0005, |data| {
        data.extend_from_slice(&[1, 0, 0, 0, 0]);
    });
    add_extension(&mut extensions, 0x000a, |data| {
        data.extend_from_slice(&4u16.to_be_bytes());
        data.extend_from_slice(&0x001du16.to_be_bytes());
        data.extend_from_slice(&0x0017u16.to_be_bytes());
    });
    add_extension(&mut extensions, 0x000d, |data| {
        let algorithms = [0x0403u16, 0x0804, 0x0401, 0x0503, 0x0805, 0x0806];
        data.extend_from_slice(&((algorithms.len() * 2) as u16).to_be_bytes());
        for algorithm in algorithms {
            data.extend_from_slice(&algorithm.to_be_bytes());
        }
    });
    add_extension(&mut extensions, 0x0010, |data| {
        data.extend_from_slice(&12u16.to_be_bytes());
        data.extend_from_slice(&[2, b'h', b'2', 8]);
        data.extend_from_slice(b"http/1.1");
    });
    add_extension(&mut extensions, 0x0012, |_| {});
    add_extension(&mut extensions, 0x002b, |data| {
        data.extend_from_slice(&[2, 0x03, 0x04]);
    });
    add_extension(&mut extensions, 0x0033, |data| {
        data.extend_from_slice(&36u16.to_be_bytes());
        data.extend_from_slice(&0x001du16.to_be_bytes());
        data.extend_from_slice(&32u16.to_be_bytes());
        data.extend_from_slice(public.as_bytes());
    });

    let cipher_suites = [0x1301u16, 0x1302, 0x1303];
    let mut body = Vec::with_capacity(128 + extensions.len());
    body.extend_from_slice(&[0x03, 0x03]);
    body.extend_from_slice(&random);
    body.push(session_id.len() as u8);
    body.extend_from_slice(&session_id);
    body.extend_from_slice(&((cipher_suites.len() * 2) as u16).to_be_bytes());
    for suite in cipher_suites {
        body.extend_from_slice(&suite.to_be_bytes());
    }
    body.extend_from_slice(&[1, 0]);
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);

    let mut record = Vec::with_capacity(9 + body.len());
    record.extend_from_slice(&[REC_HANDSHAKE, 0x03, 0x01]);
    record.extend_from_slice(&((body.len() + 4) as u16).to_be_bytes());
    record.push(0x01);
    record.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    record.extend_from_slice(&body);
    record
}

fn add_extension(output: &mut Vec<u8>, kind: u16, fill: impl FnOnce(&mut Vec<u8>)) {
    output.extend_from_slice(&kind.to_be_bytes());
    let length_at = output.len();
    output.extend_from_slice(&[0, 0]);
    let start = output.len();
    fill(output);
    let length = (output.len() - start) as u16;
    output[length_at..length_at + 2].copy_from_slice(&length.to_be_bytes());
}

pub async fn probe_origin_at(
    domain: &str,
    port: u16,
    client_hello: &[u8],
) -> Result<OriginProfile> {
    let mut origin =
        tokio::time::timeout(PROBE_CONNECT_TIMEOUT, TcpStream::connect((domain, port)))
            .await
            .map_err(|_| Error::Timeout)??;

    tokio::time::timeout(PROBE_READ_TIMEOUT, origin.write_all(client_hello))
        .await
        .map_err(|_| Error::Timeout)??;

    let mut header = [0u8; 5];
    read_exact_with_timeout(&mut origin, &mut header, PROBE_READ_TIMEOUT).await?;
    if header[0] != REC_HANDSHAKE {
        return Err(Error::invalid(format!(
            "origin sent record type 0x{:02x}",
            header[0]
        )));
    }
    let length = u16::from_be_bytes([header[3], header[4]]) as usize;
    if length > MAX_PROBE_RECORD {
        return Err(Error::invalid("origin ServerHello too large"));
    }
    let mut server_hello = header.to_vec();
    let start = server_hello.len();
    server_hello.resize(start + length, 0);
    read_exact_with_timeout(&mut origin, &mut server_hello[start..], PROBE_READ_TIMEOUT).await?;

    let parsed = parse_server_hello(&server_hello)?;
    let mut profile = OriginProfile {
        cipher_suite: parsed.cipher_suite,
        key_share_group: parsed.group,
        extension_order: server_hello_extension_order(&server_hello).unwrap_or_default(),
        record_sizes: Vec::new(),
    };

    for _ in 0..PROBE_RECORDS {
        let mut record_header = [0u8; 5];
        if read_exact_with_timeout(&mut origin, &mut record_header, PROBE_RECORD_TIMEOUT)
            .await
            .is_err()
        {
            break;
        }
        let length = u16::from_be_bytes([record_header[3], record_header[4]]) as usize;
        if length > MAX_PROBE_RECORD {
            break;
        }
        let mut body = vec![0u8; length];
        if read_exact_with_timeout(&mut origin, &mut body, PROBE_RECORD_TIMEOUT)
            .await
            .is_err()
        {
            break;
        }
        if record_header[0] == REC_APPDATA {
            profile.record_sizes.push(5 + length);
        }
    }

    Ok(profile)
}

async fn read_exact_with_timeout(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    timeout: Duration,
) -> Result<()> {
    tokio::time::timeout(timeout, stream.read_exact(buffer))
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::parse_client_hello;

    #[test]
    fn probe_hello_parses_as_a_tls13_client_hello() {
        let hello = build_probe_hello("www.example.com");
        let template = parse_client_hello(&hello).expect("parse probe hello");
        assert_eq!(template.domain, "www.example.com");
        assert!(template.cipher_suites.contains(&0x1301));
        assert_eq!(template.key_shares.len(), 1);
        assert_eq!(template.key_shares[0].group, 0x001d);
        assert_eq!(template.key_shares[0].size, 32);
        assert!(!template.early_data);
        assert!(template.psk_identity.is_none());
    }

    #[test]
    fn probe_hello_omits_server_name_for_ip_targets() {
        let hello = build_probe_hello("127.0.0.1");
        assert!(parse_client_hello(&hello).is_err());
    }
}
