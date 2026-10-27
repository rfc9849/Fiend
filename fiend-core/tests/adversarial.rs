use std::time::Duration;

use fiend_core::addr::Address;
use fiend_core::fingerprint::parse_client_hello;
use fiend_core::mux::{Config, ServerConnection};
use fiend_core::socks5;
use fiend_core::udp;
use tokio::io::{AsyncWriteExt, duplex};
use tokio::time::timeout;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        let mut output = vec![0u8; length];
        for chunk in output.chunks_mut(8) {
            let value = self.next().to_le_bytes();
            chunk.copy_from_slice(&value[..chunk.len()]);
        }
        output
    }
}

#[test]
fn parsers_do_not_panic_on_random_input() {
    let mut rng = Rng::new(0x5eed);
    for length in 0..=256 {
        for _ in 0..4 {
            let data = rng.bytes(length);
            let _ = Address::decode(&data);
            let _ = parse_client_hello(&data);
            let _ = socks5::parse_udp_request(&data);
        }
    }
}

#[tokio::test]
async fn udp_response_parser_handles_random_input() {
    let mut rng = Rng::new(0x1bad);
    for length in 0..=512 {
        let data = rng.bytes(length);
        let mut reader = data.as_slice();
        let _ = udp::read_response(&mut reader).await;
        let mut reader = data.as_slice();
        let _ = udp::read_frame(&mut reader).await;
        let mut reader = data.as_slice();
        let _ = socks5::read_request(&mut reader).await;
    }
}

#[tokio::test]
async fn udp_response_rejects_truncated_addresses() {
    let data = [0, 3, 1, 1, 127];
    let mut reader = data.as_slice();
    assert!(udp::read_response(&mut reader).await.is_err());
}

#[tokio::test]
async fn socks5_handshake_handles_random_input() {
    let mut rng = Rng::new(0x510c);
    for length in 0..=128 {
        let data = rng.bytes(length);
        let (mut client, mut server) = duplex(4096);
        tokio::spawn(async move {
            let _ = server.write_all(&data).await;
            let _ = server.shutdown().await;
        });
        let _ = timeout(Duration::from_millis(200), socks5::handshake(&mut client)).await;
    }
}

#[tokio::test]
async fn h2_handshake_handles_random_prefaces() {
    let mut rng = Rng::new(0x2c0f);
    for length in [0, 1, 16, 24, 64, 128, 256] {
        let data = rng.bytes(length);
        let (client, mut server) = duplex(4096);
        tokio::spawn(async move {
            let _ = server.write_all(&data).await;
            let _ = server.shutdown().await;
        });
        let result = timeout(
            Duration::from_secs(1),
            ServerConnection::handshake(client, &Config::default()),
        )
        .await;
        assert!(result.is_ok(), "h2 handshake hung on {length} bytes");
    }
}
