use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::config::LookupIpStrategy;

const CACHE_SIZE: u64 = 4096;
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const ATTEMPTS: usize = 2;
const MIN_TTL: Duration = Duration::from_secs(30);
const MAX_NEGATIVE_TTL: Duration = Duration::from_secs(30);

fn resolver() -> Option<&'static TokioResolver> {
    static RESOLVER: OnceLock<Option<TokioResolver>> = OnceLock::new();
    RESOLVER
        .get_or_init(|| {
            let mut builder = match TokioResolver::builder_tokio() {
                Ok(builder) => builder,
                Err(error) => {
                    tracing::warn!(%error, "cannot read the DNS settings, using the system resolver");
                    return None;
                }
            };
            let options = builder.options_mut();
            options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
            options.cache_size = CACHE_SIZE;
            options.timeout = QUERY_TIMEOUT;
            options.attempts = ATTEMPTS;
            options.positive_min_ttl = Some(MIN_TTL);
            options.negative_max_ttl = Some(MAX_NEGATIVE_TTL);
            match builder.build() {
                Ok(resolver) => Some(resolver),
                Err(error) => {
                    tracing::warn!(%error, "cannot start the DNS resolver, using the system resolver");
                    None
                }
            }
        })
        .as_ref()
}

pub fn prefer_ipv4(addresses: &mut [SocketAddr]) {
    addresses.sort_by_key(SocketAddr::is_ipv6);
}

pub async fn lookup(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    if let Some(resolver) = resolver() {
        match resolver.lookup_ip(host).await {
            Ok(found) => {
                let mut addresses: Vec<SocketAddr> =
                    found.iter().map(|ip| SocketAddr::new(ip, port)).collect();
                prefer_ipv4(&mut addresses);
                if !addresses.is_empty() {
                    return Ok(addresses);
                }
            }
            Err(error) if error.is_nx_domain() || error.is_no_records_found() => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{host} has no addresses"),
                ));
            }
            Err(error) => {
                tracing::debug!(%error, host, "DNS lookup failed, trying the system resolver")
            }
        }
    }
    let mut addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
    prefer_ipv4(&mut addresses);
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_comes_first_and_order_is_kept() {
        let mut addresses: Vec<SocketAddr> = [
            "[2001:db8::1]:443",
            "192.0.2.1:443",
            "[2001:db8::2]:443",
            "192.0.2.2:443",
        ]
        .iter()
        .map(|text| text.parse().unwrap())
        .collect();
        prefer_ipv4(&mut addresses);
        let text: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        assert_eq!(
            text,
            [
                "192.0.2.1:443",
                "192.0.2.2:443",
                "[2001:db8::1]:443",
                "[2001:db8::2]:443"
            ]
        );
    }

    #[tokio::test]
    async fn literal_addresses_skip_dns() {
        let found = lookup("192.0.2.7", 80).await.unwrap();
        assert_eq!(found, vec!["192.0.2.7:80".parse().unwrap()]);
        let found = lookup("::1", 80).await.unwrap();
        assert_eq!(found, vec!["[::1]:80".parse().unwrap()]);
    }

    #[tokio::test]
    async fn localhost_resolves() {
        let found = lookup("localhost", 80).await.unwrap();
        assert!(found.iter().all(|address| address.ip().is_loopback()));
        assert!(!found.is_empty());
    }
}
