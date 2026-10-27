use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use thiserror::Error;

pub const ATYP_IPV4: u8 = 0x01;
pub const ATYP_DOMAIN: u8 = 0x03;
pub const ATYP_IPV6: u8 = 0x04;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Address {
    Ip(SocketAddr),
    Domain { host: String, port: u16 },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("address buffer is too short")]
    TooShort,
    #[error("unknown address type 0x{0:02x}")]
    UnknownType(u8),
    #[error("domain is empty")]
    EmptyDomain,
    #[error("domain is too long")]
    DomainTooLong,
    #[error("domain is not valid UTF-8")]
    InvalidDomain,
    #[error("invalid authority")]
    InvalidAuthority,
}

impl Address {
    pub fn ip(addr: SocketAddr) -> Self {
        Self::Ip(addr)
    }

    pub fn domain(host: impl Into<String>, port: u16) -> Self {
        Self::Domain {
            host: host.into(),
            port,
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            Self::Ip(addr) => addr.port(),
            Self::Domain { port, .. } => *port,
        }
    }

    pub fn from_authority(authority: &str) -> Result<Self, AddressError> {
        if let Ok(addr) = authority.parse::<SocketAddr>() {
            return Ok(Self::Ip(addr));
        }
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or(AddressError::InvalidAuthority)?;
        if host.is_empty() || host.contains(':') {
            return Err(AddressError::InvalidAuthority);
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| AddressError::InvalidAuthority)?;
        if host.len() > u8::MAX as usize {
            return Err(AddressError::DomainTooLong);
        }
        Ok(Self::Domain {
            host: host.to_owned(),
            port,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, AddressError> {
        let mut output = Vec::new();
        self.encode_into(&mut output)?;
        Ok(output)
    }

    pub fn encode_into(&self, output: &mut Vec<u8>) -> Result<(), AddressError> {
        match self {
            Self::Ip(SocketAddr::V4(addr)) => {
                output.push(ATYP_IPV4);
                output.extend_from_slice(&addr.ip().octets());
                output.extend_from_slice(&addr.port().to_be_bytes());
            }
            Self::Ip(SocketAddr::V6(addr)) => {
                output.push(ATYP_IPV6);
                output.extend_from_slice(&addr.ip().octets());
                output.extend_from_slice(&addr.port().to_be_bytes());
            }
            Self::Domain { host, port } => {
                let host = host.as_bytes();
                if host.is_empty() {
                    return Err(AddressError::EmptyDomain);
                }
                let len = u8::try_from(host.len()).map_err(|_| AddressError::DomainTooLong)?;
                output.push(ATYP_DOMAIN);
                output.push(len);
                output.extend_from_slice(host);
                output.extend_from_slice(&port.to_be_bytes());
            }
        }
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<(Self, usize), AddressError> {
        let atyp = *input.first().ok_or(AddressError::TooShort)?;
        match atyp {
            ATYP_IPV4 => {
                let len = 7;
                if input.len() < len {
                    return Err(AddressError::TooShort);
                }
                let ip = Ipv4Addr::new(input[1], input[2], input[3], input[4]);
                let port = u16::from_be_bytes([input[5], input[6]]);
                Ok((Self::Ip(SocketAddr::V4(SocketAddrV4::new(ip, port))), len))
            }
            ATYP_IPV6 => {
                let len = 19;
                if input.len() < len {
                    return Err(AddressError::TooShort);
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&input[1..17]);
                let ip = Ipv6Addr::from(octets);
                let port = u16::from_be_bytes([input[17], input[18]]);
                Ok((
                    Self::Ip(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0))),
                    len,
                ))
            }
            ATYP_DOMAIN => {
                let host_len = *input.get(1).ok_or(AddressError::TooShort)? as usize;
                if host_len == 0 {
                    return Err(AddressError::EmptyDomain);
                }
                let len = 2 + host_len + 2;
                if input.len() < len {
                    return Err(AddressError::TooShort);
                }
                let host = std::str::from_utf8(&input[2..2 + host_len])
                    .map_err(|_| AddressError::InvalidDomain)?;
                let port = u16::from_be_bytes([input[2 + host_len], input[3 + host_len]]);
                Ok((
                    Self::Domain {
                        host: host.to_owned(),
                        port,
                    },
                    len,
                ))
            }
            other => Err(AddressError::UnknownType(other)),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ip(addr) => write!(f, "{addr}"),
            Self::Domain { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

impl From<SocketAddr> for Address {
    fn from(addr: SocketAddr) -> Self {
        Self::Ip(addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_ipv4() {
        let address = Address::ip("192.0.2.1:443".parse().expect("socket address"));
        let encoded = address.encode().expect("encode address");
        let (decoded, used) = Address::decode(&encoded).expect("decode address");
        assert_eq!(decoded, address);
        assert_eq!(used, encoded.len());
    }

    #[test]
    fn round_trips_ipv6() {
        let address = Address::ip("[2001:db8::1]:8443".parse().expect("socket address"));
        let encoded = address.encode().expect("encode address");
        let (decoded, used) = Address::decode(&encoded).expect("decode address");
        assert_eq!(decoded, address);
        assert_eq!(used, encoded.len());
    }

    #[test]
    fn round_trips_domain() {
        let address = Address::domain("example.com", 443);
        let encoded = address.encode().expect("encode address");
        let (decoded, used) = Address::decode(&encoded).expect("decode address");
        assert_eq!(decoded, address);
        assert_eq!(used, encoded.len());
    }

    #[test]
    fn decode_returns_consumed_length() {
        let address = Address::domain("example.com", 443);
        let mut encoded = address.encode().expect("encode address");
        encoded.extend_from_slice(b"tail");
        let (decoded, used) = Address::decode(&encoded).expect("decode address");
        assert_eq!(decoded, address);
        assert_eq!(used, encoded.len() - 4);
    }

    #[test]
    fn rejects_short_input() {
        assert_eq!(Address::decode(&[]), Err(AddressError::TooShort));
        assert_eq!(
            Address::decode(&[ATYP_IPV4, 192]),
            Err(AddressError::TooShort)
        );
    }

    #[test]
    fn rejects_empty_domain() {
        assert_eq!(
            Address::decode(&[ATYP_DOMAIN, 0, 0, 80]),
            Err(AddressError::EmptyDomain)
        );
    }

    #[test]
    fn rejects_long_domain() {
        let address = Address::domain("a".repeat(256), 443);
        assert_eq!(address.encode(), Err(AddressError::DomainTooLong));
    }

    #[test]
    fn rejects_unknown_type() {
        assert_eq!(
            Address::decode(&[0x02, 0, 0]),
            Err(AddressError::UnknownType(0x02))
        );
    }

    #[test]
    fn parses_authorities() {
        assert_eq!(
            Address::from_authority("example.com:443"),
            Ok(Address::domain("example.com", 443))
        );
        assert_eq!(
            Address::from_authority("192.0.2.1:8443"),
            Ok(Address::ip("192.0.2.1:8443".parse().expect("socket")))
        );
        assert_eq!(
            Address::from_authority("[2001:db8::1]:443"),
            Ok(Address::ip("[2001:db8::1]:443".parse().expect("socket")))
        );
        assert_eq!(
            Address::from_authority("example.com"),
            Err(AddressError::InvalidAuthority)
        );
        assert_eq!(
            Address::from_authority("example.com:not-a-port"),
            Err(AddressError::InvalidAuthority)
        );
        assert_eq!(
            Address::from_authority("2001:db8::1:443"),
            Err(AddressError::InvalidAuthority)
        );
    }
}
