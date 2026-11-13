use std::net::IpAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use super::error::{Error, Result};
use super::passthrough::OriginTarget;
use super::serverhello::{
    REC_APPDATA, REC_HANDSHAKE, client_shared_secret, parse_server_hello,
    server_hello_extension_order,
};
use super::tls13::{ClientHandshake, Step};
use crate::fingerprint::{BuiltClientHello, DEFAULT_RECORD_MAX, KeyShares};
use zeroize::Zeroizing;

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_READ_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_RECORD_TIMEOUT: Duration = Duration::from_millis(150);
const PROBE_RECORDS: usize = 8;
const PROBE_HANDSHAKE_RECORDS: usize = 32;
const MAX_PROBE_RECORD: usize = DEFAULT_RECORD_MAX + 16 + 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginProfile {
    pub cipher_suite: u16,
    pub key_share_group: u16,
    pub extension_order: Vec<u16>,
    pub record_sizes: Vec<usize>,
    #[serde(skip)]
    pub post_handshake_sizes: Vec<usize>,
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
    probe_origin(&OriginTarget::direct(domain, port), client_hello).await
}

pub async fn probe_origin(target: &OriginTarget, client_hello: &[u8]) -> Result<OriginProfile> {
    probe(target, client_hello, None).await
}

pub async fn probe_origin_handshake(
    target: &OriginTarget,
    hello: BuiltClientHello,
) -> Result<OriginProfile> {
    let (record, keys) = hello.into_parts();
    probe(target, &record, Some(keys)).await
}

async fn probe(
    target: &OriginTarget,
    client_hello: &[u8],
    keys: Option<KeyShares>,
) -> Result<OriginProfile> {
    let mut origin = tokio::time::timeout(PROBE_CONNECT_TIMEOUT, target.connect(None))
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
        ..OriginProfile::default()
    };
    let mut handshake = keys.and_then(|keys| {
        let share = keys
            .shares
            .into_iter()
            .find(|share| share.group == parsed.group)?;
        let shared = Zeroizing::new(client_shared_secret(share, &parsed.key_share).ok()?);
        ClientHandshake::new(
            parsed.cipher_suite,
            client_hello.get(5..)?,
            &server_hello[5..],
            &shared,
        )
    });
    let mut finished = false;

    let mut records = 0;
    while records < PROBE_RECORDS || handshake.is_some() && records < PROBE_HANDSHAKE_RECORDS {
        records += 1;
        let wait = if handshake.is_some() && profile.post_handshake_sizes.is_empty() {
            PROBE_READ_TIMEOUT
        } else {
            PROBE_RECORD_TIMEOUT
        };
        let mut record_header = [0u8; 5];
        if read_exact_with_timeout(&mut origin, &mut record_header, wait)
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
        if read_exact_with_timeout(&mut origin, &mut body, wait)
            .await
            .is_err()
        {
            break;
        }
        if record_header[0] != REC_APPDATA {
            continue;
        }
        if finished {
            profile.post_handshake_sizes.push(5 + length);
            continue;
        }
        profile.record_sizes.push(5 + length);
        let Some(client) = handshake.as_mut() else {
            continue;
        };
        match client.server_record(&record_header, &body) {
            Some(Step::More) => {}
            Some(Step::Finished(flight)) => {
                tokio::time::timeout(PROBE_READ_TIMEOUT, origin.write_all(&flight))
                    .await
                    .map_err(|_| Error::Timeout)??;
                finished = true;
            }
            None => handshake = None,
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

    #[tokio::test]
    async fn the_probe_finishes_the_handshake_and_records_post_handshake_sizes() {
        use super::super::serverhello::{build_server_hello, generate_server_share};
        use super::super::tls13::tests::Server;
        use crate::fingerprint::{Template, X25519};

        let template =
            Template::from_json(include_str!("../../testdata/template.json")).expect("template");
        let hello = template.build(&[0x11; 32]).expect("hello");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        let origin = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut header = [0u8; 5];
            stream.read_exact(&mut header).await.expect("hello header");
            let mut client_hello = header.to_vec();
            client_hello.resize(
                5 + usize::from(u16::from_be_bytes([header[3], header[4]])),
                0,
            );
            stream
                .read_exact(&mut client_hello[5..])
                .await
                .expect("hello body");
            let parsed = parse_client_hello(&client_hello).expect("parse hello");
            let spec = parsed
                .key_shares
                .iter()
                .find(|spec| spec.group == X25519)
                .expect("X25519 share");
            let share = generate_server_share(spec).expect("server share");
            let server_hello = build_server_hello(
                [5u8; 32],
                &client_hello[44..76],
                0x1301,
                X25519,
                &share.public,
                &[0x002b, 0x0033],
            )
            .expect("ServerHello");
            let mut server = Server::new(
                0x1301,
                &client_hello[5..],
                &server_hello[5..],
                &share.shared,
            );
            let mut flight = server_hello.clone();
            flight.extend_from_slice(&server.flight(3000));
            stream.write_all(&flight).await.expect("server flight");
            let mut change = [0u8; 6];
            stream.read_exact(&mut change).await.expect("CCS");
            assert_eq!(change, [0x14, 0x03, 0x03, 0x00, 0x01, 0x01]);
            let mut header = [0u8; 5];
            stream
                .read_exact(&mut header)
                .await
                .expect("Finished header");
            let mut body = vec![0u8; usize::from(u16::from_be_bytes([header[3], header[4]]))];
            stream.read_exact(&mut body).await.expect("Finished body");
            assert!(
                server.check_client_finished(&header, &body),
                "client Finished verifies"
            );
            for size in [250u16, 260] {
                let mut record = vec![REC_APPDATA, 0x03, 0x03];
                record.extend_from_slice(&size.to_be_bytes());
                record.resize(5 + usize::from(size), 0x33);
                stream.write_all(&record).await.expect("ticket");
            }
            let mut rest = Vec::new();
            let _ = stream.read_to_end(&mut rest).await;
        });
        let profile = probe_origin_handshake(&OriginTarget::direct("127.0.0.1", port), hello)
            .await
            .expect("probe");
        assert_eq!(profile.key_share_group, X25519);
        assert_eq!(profile.record_sizes.len(), 3);
        assert_eq!(profile.post_handshake_sizes, [255, 265]);
        origin.await.expect("origin");
    }

    #[test]
    fn probe_hello_omits_server_name_for_ip_targets() {
        let hello = build_probe_hello("127.0.0.1");
        assert!(parse_client_hello(&hello).is_err());
    }
}
