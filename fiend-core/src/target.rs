use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use tokio::net::lookup_host;

use crate::addr::Address;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TargetPolicy {
    pub allow_loopback: bool,
    pub allow_private: bool,
    pub allow_link_local: bool,
    pub allow_cgnat: bool,
}

impl TargetPolicy {
    pub fn allow_all() -> Self {
        Self {
            allow_loopback: true,
            allow_private: true,
            allow_link_local: true,
            allow_cgnat: true,
        }
    }

    pub fn check(&self, address: SocketAddr) -> io::Result<()> {
        let ip = match address.ip() {
            IpAddr::V6(ip) if !ip.is_loopback() && !ip.is_unspecified() => {
                ip.to_ipv4().map(IpAddr::V4).unwrap_or(IpAddr::V6(ip))
            }
            ip => ip,
        };
        match ip {
            IpAddr::V4(ip) => self.check_v4(address, ip),
            IpAddr::V6(ip) => self.check_v6(address, ip),
        }
    }

    fn check_v4(&self, address: SocketAddr, ip: Ipv4Addr) -> io::Result<()> {
        let [first, second, _, _] = ip.octets();
        if ip.is_loopback() {
            return self.allow(address, self.allow_loopback);
        }
        if ip.is_private() {
            return self.allow(address, self.allow_private);
        }
        if ip.is_link_local() {
            return self.allow(address, self.allow_link_local);
        }
        if first == 100 && (64..=127).contains(&second) {
            return self.allow(address, self.allow_cgnat);
        }
        if first == 0 || first >= 240 || ip.is_multicast() {
            return Err(blocked(address));
        }
        if (first == 192 && second == 0) || (first == 198 && (18..=19).contains(&second)) {
            return Err(blocked(address));
        }
        Ok(())
    }

    fn check_v6(&self, address: SocketAddr, ip: Ipv6Addr) -> io::Result<()> {
        if ip.is_loopback() {
            return self.allow(address, self.allow_loopback);
        }
        let segments = ip.segments();
        let first = segments[0];
        if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfec0 {
            return self.allow(address, self.allow_private);
        }
        if (first & 0xffc0) == 0xfe80 {
            return self.allow(address, self.allow_link_local);
        }
        if ip.is_unspecified() || ip.is_multicast() {
            return Err(blocked(address));
        }
        if let Some(embedded) = embedded_ipv4(ip) {
            return self.check_v4(address, embedded);
        }
        if segments[..3] == [0x0064, 0xff9b, 1] {
            return Err(blocked(address));
        }
        Ok(())
    }

    fn allow(&self, address: SocketAddr, allowed: bool) -> io::Result<()> {
        if allowed {
            Ok(())
        } else {
            Err(blocked(address))
        }
    }
}

fn embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let octets = ip.octets();
    if segments[..5] == [0x0064, 0xff9b, 0, 0, 0] {
        return Some(Ipv4Addr::from(
            <[u8; 4]>::try_from(&octets[12..16]).expect("four bytes"),
        ));
    }
    if segments[0] == 0x2002 {
        return Some(Ipv4Addr::from(
            <[u8; 4]>::try_from(&octets[2..6]).expect("four bytes"),
        ));
    }
    None
}

pub async fn resolve_all(address: &Address, policy: &TargetPolicy) -> io::Result<Vec<SocketAddr>> {
    match address {
        Address::Ip(address) => {
            policy.check(*address)?;
            Ok(vec![*address])
        }
        Address::Domain { host, port } => {
            let mut denied = None;
            let mut allowed = Vec::new();
            for address in lookup_host((host.as_str(), *port)).await? {
                match policy.check(address) {
                    Ok(()) => allowed.push(address),
                    Err(error) => denied = Some(error),
                }
            }
            if allowed.is_empty() {
                return Err(denied.unwrap_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("target {host}:{port} has no addresses"),
                    )
                }));
            }
            allowed.dedup();
            Ok(allowed)
        }
    }
}

pub async fn connect_tcp(
    address: &Address,
    policy: &TargetPolicy,
    budget: std::time::Duration,
) -> io::Result<tokio::net::TcpStream> {
    let deadline = tokio::time::Instant::now() + budget;
    let candidates = tokio::time::timeout_at(deadline, resolve_all(address, policy))
        .await
        .map_err(|_| timed_out())??;
    let mut last = None;
    for candidate in candidates {
        match tokio::time::timeout_at(deadline, tokio::net::TcpStream::connect(candidate)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = Some(error),
            Err(_) => {
                last = Some(timed_out());
                break;
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target has no addresses")))
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "target connect timed out")
}

pub async fn resolve(address: &Address, policy: &TargetPolicy) -> io::Result<SocketAddr> {
    resolve_all(address, policy)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target has no addresses"))
}

fn blocked(address: SocketAddr) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("target {address} is not allowed"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> SocketAddr {
        value.parse().expect("address")
    }

    #[test]
    fn public_targets_are_allowed() {
        let policy = TargetPolicy::default();
        assert!(policy.check(ip("8.8.8.8:443")).is_ok());
        assert!(policy.check(ip("[2001:4860:4860::8888]:53")).is_ok());
        assert!(policy.check(ip("[::ffff:8.8.8.8]:443")).is_ok());
    }

    #[test]
    fn loopback_requires_permission() {
        let policy = TargetPolicy::default();
        assert_eq!(
            policy.check(ip("127.0.0.1:80")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(policy.check(ip("[::1]:80")).is_err());
        assert!(policy.check(ip("[::ffff:127.0.0.1]:80")).is_err());
        assert!(
            TargetPolicy {
                allow_loopback: true,
                ..TargetPolicy::default()
            }
            .check(ip("127.0.0.1:80"))
            .is_ok()
        );
    }

    #[test]
    fn private_ranges_require_permission() {
        let policy = TargetPolicy::default();
        for address in [
            "10.0.0.1:80",
            "172.16.0.1:80",
            "192.168.0.1:80",
            "[fd00::1]:80",
        ] {
            assert!(policy.check(ip(address)).is_err(), "{address}");
        }
        assert!(
            TargetPolicy {
                allow_private: true,
                ..TargetPolicy::default()
            }
            .check(ip("192.168.0.1:80"))
            .is_ok()
        );
    }

    #[test]
    fn link_local_and_cgnat_require_permission() {
        let policy = TargetPolicy::default();
        assert!(policy.check(ip("169.254.169.254:80")).is_err());
        assert!(policy.check(ip("[fe80::1]:80")).is_err());
        assert!(policy.check(ip("100.64.0.1:80")).is_err());
        assert!(
            TargetPolicy {
                allow_link_local: true,
                ..TargetPolicy::default()
            }
            .check(ip("169.254.169.254:80"))
            .is_ok()
        );
        assert!(
            TargetPolicy {
                allow_cgnat: true,
                ..TargetPolicy::default()
            }
            .check(ip("100.64.0.1:80"))
            .is_ok()
        );
    }

    #[test]
    fn reserved_targets_are_always_blocked() {
        let policy = TargetPolicy::allow_all();
        for address in [
            "0.0.0.0:80",
            "255.255.255.255:80",
            "224.0.0.1:80",
            "192.0.0.1:80",
            "192.0.2.1:80",
            "198.18.0.1:80",
            "198.19.255.254:80",
            "[::]:80",
            "[ff02::1]:80",
            "[64:ff9b::1]:80",
            "[64:ff9b:1::c0a8:1]:80",
            "[2002:0:1::1]:80",
        ] {
            assert!(policy.check(ip(address)).is_err(), "{address}");
        }
        assert!(policy.check(ip("[64:ff9b::808:808]:80")).is_ok());
        assert!(policy.check(ip("[2002:808:808::1]:80")).is_ok());
    }

    #[test]
    fn embedded_ipv4_targets_follow_the_ipv4_policy() {
        let policy = TargetPolicy {
            allow_private: true,
            ..TargetPolicy::default()
        };
        for address in ["[64:ff9b::c0a8:1]:80", "[2002:c0a8:0001::1]:80"] {
            assert!(policy.check(ip(address)).is_ok(), "{address}");
        }
        assert!(
            TargetPolicy::default()
                .check(ip("[64:ff9b::c0a8:1]:80"))
                .is_err()
        );
        assert!(
            TargetPolicy::default()
                .check(ip("[2002:c0a8:0001::1]:80"))
                .is_err()
        );
    }

    #[tokio::test]
    async fn resolve_all_keeps_every_allowed_address() {
        let policy = TargetPolicy {
            allow_loopback: true,
            ..TargetPolicy::default()
        };
        let addresses = resolve_all(&Address::domain("localhost".to_owned(), 80), &policy)
            .await
            .expect("resolve localhost");
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|address| address.port() == 80));
        assert!(addresses.iter().all(|address| address.ip().is_loopback()));
        let mut unique = addresses.clone();
        unique.dedup();
        assert_eq!(unique.len(), addresses.len(), "addresses must be unique");
    }

    #[tokio::test]
    async fn resolve_all_rejects_a_fully_denied_domain() {
        let addresses = resolve_all(
            &Address::domain("localhost".to_owned(), 80),
            &TargetPolicy::default(),
        )
        .await;
        assert_eq!(
            addresses.expect_err("denied").kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[tokio::test]
    async fn resolve_applies_the_policy_to_ip_targets() {
        let policy = TargetPolicy::allow_all();
        assert_eq!(
            resolve(&Address::ip(ip("127.0.0.1:80")), &policy)
                .await
                .expect("allowed"),
            ip("127.0.0.1:80")
        );
        assert!(
            resolve(&Address::ip(ip("127.0.0.1:80")), &TargetPolicy::default())
                .await
                .is_err()
        );
    }
}
