use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::transport::PassthroughReason;

const MAX_PROBES: usize = 128;
pub const MAX_HEAD: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Probe {
    pub at: u64,
    pub peer: String,
    pub reason: String,
    #[serde(default)]
    pub sni: Option<String>,
    pub size: u64,
    pub head: String,
}

impl Probe {
    pub fn new(peer: SocketAddr, reason: PassthroughReason, buffered: &[u8]) -> Self {
        let sni = (buffered.first() == Some(&0x16))
            .then(|| crate::fingerprint::parse_client_hello(buffered).ok())
            .flatten()
            .map(|template| template.domain)
            .filter(|domain| !domain.is_empty());
        Self {
            at: super::unix_time(),
            peer: peer.ip().to_canonical().to_string(),
            reason: reason.as_str().into(),
            sni,
            size: buffered.len() as u64,
            head: hex::encode(&buffered[..buffered.len().min(MAX_HEAD)]),
        }
    }
}

#[derive(Default)]
pub struct Recorder {
    probes: Mutex<VecDeque<Probe>>,
}

impl Recorder {
    pub fn push(&self, probe: Probe) {
        let mut probes = self
            .probes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if probes.len() >= MAX_PROBES {
            probes.pop_front();
        }
        probes.push_back(probe);
    }

    pub fn drain(&self) -> Vec<Probe> {
        self.probes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain(..)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_keeps_peer_reason_sni_and_head() {
        let hello = crate::transport::build_probe_hello("www.example.com");
        let peer: SocketAddr = "[::ffff:203.0.113.7]:4431".parse().expect("peer");
        let probe = Probe::new(peer, PassthroughReason::UnknownKey, &hello);
        assert_eq!(probe.peer, "203.0.113.7");
        assert_eq!(probe.reason, "unknown_key");
        assert_eq!(probe.sni.as_deref(), Some("www.example.com"));
        assert_eq!(probe.size, hello.len() as u64);
        assert_eq!(probe.head.len(), hello.len().min(MAX_HEAD) * 2);
    }

    #[test]
    fn recorder_keeps_the_newest_probes() {
        let recorder = Recorder::default();
        let peer: SocketAddr = "198.51.100.1:80".parse().expect("peer");
        for _ in 0..MAX_PROBES + 5 {
            recorder.push(Probe::new(
                peer,
                PassthroughReason::NotTls,
                b"GET / HTTP/1.1\r\n",
            ));
        }
        let probes = recorder.drain();
        assert_eq!(probes.len(), MAX_PROBES);
        assert_eq!(probes[0].sni, None);
        assert!(recorder.drain().is_empty());
    }
}
