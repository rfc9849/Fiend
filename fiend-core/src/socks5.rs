use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::addr::{ATYP_DOMAIN, ATYP_IPV4, ATYP_IPV6, Address, AddressError};

pub const VERSION: u8 = 5;
pub const METHOD_NONE: u8 = 0;
pub const METHOD_UNACCEPTABLE: u8 = 0xff;

pub const CMD_CONNECT: u8 = 0x01;
pub const CMD_UDP_ASSOCIATE: u8 = 0x03;

pub const REP_SUCCESS: u8 = 0x00;
pub const REP_GENERAL_FAILURE: u8 = 0x01;
pub const REP_HOST_UNREACHABLE: u8 = 0x04;
pub const REP_CMD_NOT_SUPPORTED: u8 = 0x07;

const MAX_DOMAIN: usize = u8::MAX as usize;
const MAX_UDP_DATAGRAM: usize = u16::MAX as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Connect,
    UdpAssociate,
    Unsupported(u8),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub command: Command,
    pub address: Address,
}

pub async fn handshake<S>(stream: &mut S) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await?;
    if header[0] != VERSION {
        return Err(invalid("not SOCKS5"));
    }

    let mut methods = vec![0u8; header[1] as usize];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&METHOD_NONE) {
        stream.write_all(&[VERSION, METHOD_UNACCEPTABLE]).await?;
        return Err(invalid("no acceptable authentication method"));
    }

    stream.write_all(&[VERSION, METHOD_NONE]).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn read_request<S>(stream: &mut S) -> io::Result<Request>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    if header[0] != VERSION {
        return Err(invalid("invalid SOCKS5 request version"));
    }
    if header[2] != 0 {
        return Err(invalid("invalid SOCKS5 reserved byte"));
    }

    let address = read_address(stream, header[3]).await?;
    let command = match header[1] {
        CMD_CONNECT => Command::Connect,
        CMD_UDP_ASSOCIATE => Command::UdpAssociate,
        other => Command::Unsupported(other),
    };
    Ok(Request { command, address })
}

pub async fn read_address<R>(reader: &mut R, atyp: u8) -> io::Result<Address>
where
    R: AsyncRead + Unpin,
{
    let mut raw = vec![atyp];
    match atyp {
        ATYP_IPV4 => {
            let start = raw.len();
            raw.resize(start + 6, 0);
            reader.read_exact(&mut raw[start..]).await?;
        }
        ATYP_IPV6 => {
            let start = raw.len();
            raw.resize(start + 18, 0);
            reader.read_exact(&mut raw[start..]).await?;
        }
        ATYP_DOMAIN => {
            let mut length = [0u8; 1];
            reader.read_exact(&mut length).await?;
            let length = length[0] as usize;
            if length == 0 || length > MAX_DOMAIN {
                return Err(invalid("invalid SOCKS5 domain length"));
            }
            raw.push(length as u8);
            let start = raw.len();
            raw.resize(start + length + 2, 0);
            reader.read_exact(&mut raw[start..]).await?;
        }
        other => {
            return Err(invalid(format!(
                "unsupported SOCKS5 address type 0x{other:02x}"
            )));
        }
    }

    let (address, used) =
        Address::decode(&raw).map_err(|error| invalid(format!("invalid address: {error}")))?;
    if used != raw.len() {
        return Err(invalid("trailing address bytes"));
    }
    Ok(address)
}

pub async fn send_reply<W>(writer: &mut W, reply: u8, bind: &Address) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut message = Vec::with_capacity(3 + 19);
    message.extend_from_slice(&[VERSION, reply, 0]);
    bind.encode_into(&mut message).map_err(address_error)?;
    writer.write_all(&message).await?;
    writer.flush().await?;
    Ok(())
}

pub fn parse_udp_request(datagram: &[u8]) -> Option<(Address, &[u8])> {
    if datagram.len() < 4 || datagram[0] != 0 || datagram[1] != 0 || datagram[2] != 0 {
        return None;
    }
    let (address, used) = Address::decode(&datagram[3..]).ok()?;
    let payload_start = 3 + used;
    if payload_start > datagram.len() {
        return None;
    }
    Some((address, &datagram[payload_start..]))
}

pub fn encode_udp_request(destination: &Address, payload: &[u8]) -> io::Result<Vec<u8>> {
    let mut datagram = Vec::with_capacity(3 + 259 + payload.len());
    datagram.extend_from_slice(&[0, 0, 0]);
    destination
        .encode_into(&mut datagram)
        .map_err(address_error)?;
    datagram.extend_from_slice(payload);
    if datagram.len() > MAX_UDP_DATAGRAM {
        return Err(invalid("SOCKS5 UDP request is too large"));
    }
    Ok(datagram)
}

pub fn encode_udp_response(source: &Address, payload: &[u8]) -> io::Result<Vec<u8>> {
    encode_udp_request(source, payload)
}

pub fn zero_address() -> Address {
    Address::ip(std::net::SocketAddr::from(([0, 0, 0, 0], 0)))
}

fn address_error(error: AddressError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::AddressError;

    #[test]
    fn parses_udp_requests() {
        let destination = Address::domain("example.com", 53);
        let datagram = encode_udp_request(&destination, b"query").expect("encode");
        let (parsed, payload) = parse_udp_request(&datagram).expect("parse");
        assert_eq!(parsed, destination);
        assert_eq!(payload, b"query");
    }

    #[test]
    fn rejects_udp_fragments_and_reserved_bytes() {
        let destination = Address::ip("192.0.2.1:53".parse().expect("address"));
        let mut datagram = encode_udp_request(&destination, b"query").expect("encode");
        datagram[2] = 1;
        assert!(parse_udp_request(&datagram).is_none());
        datagram[2] = 0;
        datagram[0] = 1;
        assert!(parse_udp_request(&datagram).is_none());
    }

    #[test]
    fn rejects_oversize_udp_payloads() {
        let destination = Address::ip("192.0.2.1:53".parse().expect("address"));
        let payload = vec![0u8; MAX_UDP_DATAGRAM];
        assert_eq!(
            encode_udp_request(&destination, &payload)
                .expect_err("oversize")
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            Address::decode(&[ATYP_IPV4]).unwrap_err(),
            AddressError::TooShort
        );
    }
}
