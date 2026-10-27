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
        Ok(())
    }

    fn check_v6(&self, address: SocketAddr, ip: Ipv6Addr) -> io::Result<()> {
        if ip.is_loopback() {
            return self.allow(address, self.allow_loopback);
        }
        let first = ip.segments()[0];
        if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfec0 {
            return self.allow(address, self.allow_private);
        }
        if (first & 0xffc0) == 0xfe80 {
            return self.allow(address, self.allow_link_local);
        }
        if ip.is_unspecified() || ip.is_multicast() {
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

pub async fn resolve(address: &Address, policy: &TargetPolicy) -> io::Result<SocketAddr> {
    match address {
        Address::Ip(address) => {
            policy.check(*address)?;
            Ok(*address)
        }
        Address::Domain { host, port } => {
            let mut denied = None;
            let mut found = false;
            for address in lookup_host((host.as_str(), *port)).await? {
                found = true;
                match policy.check(address) {
                    Ok(()) => return Ok(address),
                    Err(error) => denied = Some(error),
                }
            }
            if found {
                Err(denied.expect("checked address"))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("target {host}:{port} has no addresses"),
                ))
            }
        }
    }
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
            "[::]:80",
            "[ff02::1]:80",
        ] {
            assert!(policy.check(ip(address)).is_err(), "{address}");
        }
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
