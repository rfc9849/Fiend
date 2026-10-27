use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::error::{Error, Result};
use super::serverhello::{
    REC_APPDATA, REC_HANDSHAKE, parse_server_hello, server_hello_extension_order,
};
use crate::fingerprint::{DEFAULT_RECORD_MAX, Result as FingerprintResult};

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_READ_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_RECORD_TIMEOUT: Duration = Duration::from_millis(150);
const PROBE_RECORDS: usize = 8;
const MAX_PROBE_RECORD: usize = DEFAULT_RECORD_MAX + 16 + 256;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OriginProfile {
    pub cipher_suite: u16,
    pub key_share_group: u16,
    pub extension_order: Vec<u16>,
    pub record_sizes: Vec<usize>,
}

struct CacheEntry {
    profile: OriginProfile,
    recorded: bool,
}

pub struct OriginProfileCache {
    entry: Mutex<CacheEntry>,
}

impl Default for OriginProfileCache {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginProfileCache {
    pub fn new() -> Self {
        Self {
            entry: Mutex::new(CacheEntry {
                profile: OriginProfile::default(),
                recorded: false,
            }),
        }
    }

    pub fn profile(&self) -> OriginProfile {
        self.entry
            .lock()
            .expect("origin profile lock")
            .profile
            .clone()
    }

    pub fn set_profile(&self, profile: OriginProfile) {
        let mut entry = self.entry.lock().expect("origin profile lock");
        entry.profile = profile;
        entry.recorded = true;
    }

    fn set_if_empty(&self, profile: OriginProfile) {
        let mut entry = self.entry.lock().expect("origin profile lock");
        if !entry.recorded {
            entry.profile = profile;
            entry.recorded = true;
        }
    }

    pub async fn refresh(&self, domain: &str, client_hello: &[u8]) {
        match probe_origin(domain, client_hello).await {
            Ok(profile) => self.set_profile(profile),
            Err(_) => self.set_if_empty(OriginProfile::default()),
        }
    }

    pub async fn start_warming<F>(
        self: &Arc<Self>,
        domain: impl Into<String>,
        hello: F,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn() -> FingerprintResult<Vec<u8>> + Send + Sync + 'static,
    {
        let domain = domain.into();
        let hello = Arc::new(hello);
        match hello() {
            Ok(client_hello) => self.refresh(&domain, &client_hello).await,
            Err(_) => self.set_if_empty(OriginProfile::default()),
        }

        let cache = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                match hello() {
                    Ok(client_hello) => cache.refresh(&domain, &client_hello).await,
                    Err(_) => cache.set_if_empty(OriginProfile::default()),
                }
            }
        })
    }
}

pub async fn probe_origin(domain: &str, client_hello: &[u8]) -> Result<OriginProfile> {
    probe_origin_at(domain, 443, client_hello).await
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
