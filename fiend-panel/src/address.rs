use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

const TRACE_URL: &str = "https://1.1.1.1/cdn-cgi/trace";
const TRACE_TIMEOUT: Duration = Duration::from_secs(4);

pub async fn detect() -> Option<Ipv4Addr> {
    tokio::task::spawn_blocking(|| route_source().filter(is_public).or_else(trace))
        .await
        .ok()
        .flatten()
}

fn route_source() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(1, 1, 1, 1), 53)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(address) => Some(address),
        IpAddr::V6(_) => None,
    }
}

fn trace() -> Option<Ipv4Addr> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TRACE_TIMEOUT))
        .build()
        .into();
    let text = agent
        .get(TRACE_URL)
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    parse_trace(&text).filter(is_public)
}

fn parse_trace(text: &str) -> Option<Ipv4Addr> {
    text.lines()
        .find_map(|line| line.strip_prefix("ip="))
        .and_then(|value| value.trim().parse().ok())
}

pub fn is_public(address: &Ipv4Addr) -> bool {
    let [a, b, ..] = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && address.octets()[2] == 0)
        || (a == 198 && (b == 18 || b == 19)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_filter() {
        for address in ["9.9.9.9", "8.8.8.8", "104.16.0.1"] {
            assert!(is_public(&address.parse().unwrap()), "{address}");
        }
        for address in [
            "10.0.0.5",
            "172.16.1.1",
            "192.168.1.10",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "203.0.113.10",
            "198.18.0.1",
            "192.0.0.8",
            "255.255.255.255",
            "240.0.0.1",
        ] {
            assert!(!is_public(&address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn trace_reply() {
        let text = "fl=123\nh=1.1.1.1\nip=9.9.9.9\nts=1\nwarp=off\n";
        assert_eq!(parse_trace(text), Some(Ipv4Addr::new(9, 9, 9, 9)));
        assert_eq!(parse_trace("ip=2001:db8::1\n"), None);
        assert_eq!(parse_trace("h=x\n"), None);
    }
}
