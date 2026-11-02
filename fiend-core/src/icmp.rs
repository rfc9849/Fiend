use std::io;
use std::net::{IpAddr, SocketAddr};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

const ECHO_REQUEST_V4: u8 = 8;
const ECHO_REPLY_V4: u8 = 0;
const ECHO_REQUEST_V6: u8 = 128;
const ECHO_REPLY_V6: u8 = 129;
const MAX_DATA: usize = 1472;

pub struct EchoSocket {
    socket: UdpSocket,
    target: IpAddr,
    ident: Option<u16>,
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut total = 0u32;
    let (pairs, rest) = bytes.as_chunks::<2>();
    for pair in pairs {
        total += u32::from(u16::from_be_bytes(*pair));
    }
    if let [last] = rest {
        total += u32::from(*last) << 8;
    }
    while total > 0xffff {
        total = (total & 0xffff) + (total >> 16);
    }
    !(total as u16)
}

impl EchoSocket {
    pub fn open(target: IpAddr) -> io::Result<Self> {
        let (domain, protocol) = match target {
            IpAddr::V4(_) => (Domain::IPV4, Protocol::ICMPV4),
            IpAddr::V6(_) => (Domain::IPV6, Protocol::ICMPV6),
        };
        let (socket, ident) = match Socket::new(domain, Type::DGRAM, Some(protocol)) {
            Ok(socket) => (socket, None),
            Err(error) => match Socket::new(domain, Type::RAW, Some(protocol)) {
                Ok(socket) => (socket, Some(rand::random::<u16>())),
                Err(_) => return Err(error),
            },
        };
        socket.set_nonblocking(true)?;
        let socket = UdpSocket::from_std(std::net::UdpSocket::from(socket))?;
        Ok(Self {
            socket,
            target,
            ident,
        })
    }

    pub fn target(&self) -> IpAddr {
        self.target
    }

    pub async fn send(&self, sequence: u16, data: &[u8]) -> io::Result<()> {
        let data = &data[..data.len().min(MAX_DATA)];
        let kind = if self.target.is_ipv4() {
            ECHO_REQUEST_V4
        } else {
            ECHO_REQUEST_V6
        };
        let mut body = Vec::with_capacity(8 + data.len());
        body.extend_from_slice(&[kind, 0, 0, 0]);
        body.extend_from_slice(&self.ident.unwrap_or(0).to_be_bytes());
        body.extend_from_slice(&sequence.to_be_bytes());
        body.extend_from_slice(data);
        if self.target.is_ipv4() {
            let sum = checksum(&body);
            body[2..4].copy_from_slice(&sum.to_be_bytes());
        }
        self.socket
            .send_to(&body, SocketAddr::new(self.target, 0))
            .await?;
        Ok(())
    }

    pub async fn recv(&self) -> io::Result<(u16, Vec<u8>)> {
        let mut buffer = vec![0u8; 65535];
        loop {
            let (size, from) = self.socket.recv_from(&mut buffer).await?;
            if from.ip().to_canonical() != self.target.to_canonical() {
                continue;
            }
            if let Some((ident, sequence, data)) =
                parse_reply(&buffer[..size], self.target.is_ipv4())
                && self.ident.is_none_or(|own| own == ident)
            {
                return Ok((sequence, data));
            }
        }
    }
}

fn parse_reply(message: &[u8], ipv4: bool) -> Option<(u16, u16, Vec<u8>)> {
    let body = if ipv4 && message.len() >= 20 && message[0] >> 4 == 4 && message[9] == 1 {
        let header = usize::from(message[0] & 0x0f) * 4;
        message.get(header..)?
    } else {
        message
    };
    let reply = if ipv4 { ECHO_REPLY_V4 } else { ECHO_REPLY_V6 };
    if body.len() < 8 || body[0] != reply || body[1] != 0 {
        return None;
    }
    Some((
        u16::from_be_bytes([body[4], body[5]]),
        u16::from_be_bytes([body[6], body[7]]),
        body[8..].to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_read_with_or_without_an_ip_header() {
        let body = [ECHO_REPLY_V4, 0, 0, 0, 0, 1, 0, 9, b'h', b'i'];
        assert_eq!(parse_reply(&body, true), Some((1, 9, b"hi".to_vec())));
        let mut with_header = vec![
            0x45, 0, 0, 30, 0, 0, 0, 0, 64, 1, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1,
        ];
        with_header.extend_from_slice(&body);
        assert_eq!(
            parse_reply(&with_header, true),
            Some((1, 9, b"hi".to_vec()))
        );
        let mut request = body;
        request[0] = ECHO_REQUEST_V4;
        assert_eq!(parse_reply(&request, true), None);
        let v6 = [ECHO_REPLY_V6, 0, 0, 0, 0, 1, 0, 4];
        assert_eq!(parse_reply(&v6, false), Some((1, 4, Vec::new())));
    }

    #[tokio::test]
    async fn loopback_answers_an_echo_when_the_system_allows_it() {
        let socket = match EchoSocket::open("127.0.0.1".parse().unwrap()) {
            Ok(socket) => socket,
            Err(error) => {
                eprintln!("ICMP sockets are not available here: {error}");
                return;
            }
        };
        socket.send(42, b"fiend").await.unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(2), socket.recv())
            .await
            .expect("echo reply")
            .unwrap();
        assert_eq!(reply, (42, b"fiend".to_vec()));
    }
}
