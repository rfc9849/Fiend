use std::io;
use std::net::SocketAddr;

use ppp::{HeaderResult, PartialResult, v1, v2};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

const MAX_HEADER: usize = 16 + u16::MAX as usize;
const PEEK: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub source: SocketAddr,
    pub destination: SocketAddr,
}

impl Endpoints {
    pub fn of(stream: &TcpStream) -> io::Result<Self> {
        Ok(Self {
            source: stream.peer_addr()?,
            destination: stream.local_addr()?,
        })
    }

    pub fn is_loop(&self) -> bool {
        let source = self.source.ip().to_canonical();
        source == self.destination.ip().to_canonical()
            && !source.is_loopback()
            && !source.is_unspecified()
    }

    pub fn header(&self) -> Vec<u8> {
        let source = canonical(self.source);
        let destination = canonical(self.destination);
        let pair = match (source, destination) {
            (SocketAddr::V4(_), SocketAddr::V4(_)) | (SocketAddr::V6(_), SocketAddr::V6(_)) => {
                (source, destination)
            }
            _ => (mapped(source), mapped(destination)),
        };
        v2::Builder::with_addresses(
            v2::Version::Two | v2::Command::Proxy,
            v2::Protocol::Stream,
            pair,
        )
        .build()
        .unwrap_or_else(|_| local_header())
    }
}

pub fn local_header() -> Vec<u8> {
    let mut header = v2::PROTOCOL_PREFIX.to_vec();
    header.extend_from_slice(&[0x20, 0x00, 0x00, 0x00]);
    header
}

fn canonical(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(address.ip().to_canonical(), address.port())
}

fn mapped(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V4(v4) => SocketAddr::new(v4.ip().to_ipv6_mapped().into(), v4.port()),
        other => other,
    }
}

pub async fn read_header(stream: &mut TcpStream) -> io::Result<Option<Endpoints>> {
    let mut header = Vec::new();
    let mut consumed = 0;
    let mut scratch = vec![0u8; PEEK];
    loop {
        let room = (MAX_HEADER - consumed).min(PEEK);
        if room == 0 {
            return Err(invalid("PROXY protocol header is too long"));
        }
        let count = stream.peek(&mut scratch[..room]).await?;
        if count == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        header.truncate(consumed);
        header.extend_from_slice(&scratch[..count]);
        let parsed = HeaderResult::parse(&header);
        if parsed.is_incomplete() {
            stream.read_exact(&mut scratch[..count]).await?;
            consumed += count;
            continue;
        }
        let (length, endpoints) = match parsed {
            HeaderResult::V1(Ok(found)) => (found.header.len(), v1_endpoints(&found.addresses)),
            HeaderResult::V2(Ok(found)) => (found.len(), v2_endpoints(&found)),
            _ => return Err(invalid("invalid PROXY protocol header")),
        };
        let rest = length
            .checked_sub(consumed)
            .filter(|rest| *rest <= count)
            .ok_or_else(|| invalid("invalid PROXY protocol header length"))?;
        stream.read_exact(&mut scratch[..rest]).await?;
        return Ok(endpoints);
    }
}

fn v1_endpoints(addresses: &v1::Addresses) -> Option<Endpoints> {
    match addresses {
        v1::Addresses::Tcp4(ip) => Some(Endpoints {
            source: SocketAddr::new(ip.source_address.into(), ip.source_port),
            destination: SocketAddr::new(ip.destination_address.into(), ip.destination_port),
        }),
        v1::Addresses::Tcp6(ip) => Some(Endpoints {
            source: SocketAddr::new(ip.source_address.into(), ip.source_port),
            destination: SocketAddr::new(ip.destination_address.into(), ip.destination_port),
        }),
        v1::Addresses::Unknown => None,
    }
}

fn v2_endpoints(header: &v2::Header<'_>) -> Option<Endpoints> {
    if header.command != v2::Command::Proxy {
        return None;
    }
    match header.addresses {
        v2::Addresses::IPv4(ip) => Some(Endpoints {
            source: SocketAddr::new(ip.source_address.into(), ip.source_port),
            destination: SocketAddr::new(ip.destination_address.into(), ip.destination_port),
        }),
        v2::Addresses::IPv6(ip) => Some(Endpoints {
            source: SocketAddr::new(ip.source_address.into(), ip.source_port),
            destination: SocketAddr::new(ip.destination_address.into(), ip.destination_port),
        }),
        _ => None,
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    async fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    async fn rest(stream: &mut TcpStream, length: usize) -> Vec<u8> {
        let mut data = vec![0u8; length];
        stream.read_exact(&mut data).await.unwrap();
        data
    }

    #[tokio::test]
    async fn a_v1_header_is_read_and_the_data_after_it_stays() {
        let (mut client, mut server) = pair().await;
        client
            .write_all(b"PROXY TCP4 203.0.113.7 198.51.100.1 51000 443\r\n\x16\x03\x01")
            .await
            .unwrap();
        let endpoints = read_header(&mut server).await.unwrap().unwrap();
        assert_eq!(endpoints.source, "203.0.113.7:51000".parse().unwrap());
        assert_eq!(endpoints.destination, "198.51.100.1:443".parse().unwrap());
        assert_eq!(rest(&mut server, 3).await, b"\x16\x03\x01");
    }

    #[tokio::test]
    async fn a_v2_header_in_parts_is_read() {
        let (mut client, mut server) = pair().await;
        let endpoints = Endpoints {
            source: "[2001:db8::7]:51000".parse().unwrap(),
            destination: "[2001:db8::1]:443".parse().unwrap(),
        };
        let mut data = endpoints.header();
        data.extend_from_slice(b"hello");
        let reader = tokio::spawn(async move {
            let found = read_header(&mut server).await.unwrap();
            (found, rest(&mut server, 5).await)
        });
        for part in data.chunks(7) {
            client.write_all(part).await.unwrap();
            client.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let (found, tail) = reader.await.unwrap();
        assert_eq!(found, Some(endpoints));
        assert_eq!(tail, b"hello");
    }

    #[tokio::test]
    async fn local_and_unknown_headers_have_no_endpoints() {
        for data in [local_header(), b"PROXY UNKNOWN\r\n".to_vec()] {
            let (mut client, mut server) = pair().await;
            client.write_all(&data).await.unwrap();
            client.write_all(b"x").await.unwrap();
            assert_eq!(read_header(&mut server).await.unwrap(), None);
            assert_eq!(rest(&mut server, 1).await, b"x");
        }
    }

    #[tokio::test]
    async fn data_without_a_header_is_refused() {
        for data in [
            &b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03"[..],
            b"GET / HTTP/1.1\r\n",
        ] {
            let (mut client, mut server) = pair().await;
            client.write_all(data).await.unwrap();
            assert!(read_header(&mut server).await.is_err());
        }
    }

    #[test]
    fn mixed_families_are_sent_as_ipv6() {
        let endpoints = Endpoints {
            source: "203.0.113.7:51000".parse().unwrap(),
            destination: "[2001:db8::1]:443".parse().unwrap(),
        };
        let header = endpoints.header();
        let parsed = v2::Header::try_from(header.as_slice()).unwrap();
        assert_eq!(parsed.command, v2::Command::Proxy);
        assert!(matches!(parsed.addresses, v2::Addresses::IPv6(_)));
        assert!(matches!(
            v2::Header::try_from(local_header().as_slice())
                .unwrap()
                .command,
            v2::Command::Local
        ));
    }

    #[test]
    fn only_a_connection_from_this_host_to_itself_is_a_loop() {
        let looped = Endpoints {
            source: "198.51.100.1:40000".parse().unwrap(),
            destination: "198.51.100.1:443".parse().unwrap(),
        };
        assert!(looped.is_loop());
        let mapped = Endpoints {
            source: "[::ffff:198.51.100.1]:40000".parse().unwrap(),
            destination: "198.51.100.1:443".parse().unwrap(),
        };
        assert!(mapped.is_loop());
        let local = Endpoints {
            source: "127.0.0.1:40000".parse().unwrap(),
            destination: "127.0.0.1:443".parse().unwrap(),
        };
        assert!(!local.is_loop());
        let remote = Endpoints {
            source: "203.0.113.7:40000".parse().unwrap(),
            destination: "198.51.100.1:443".parse().unwrap(),
        };
        assert!(!remote.is_loop());
    }
}
