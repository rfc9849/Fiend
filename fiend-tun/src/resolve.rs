use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fiend_core::net::ProtectSocket;
use hickory_proto::op::{Edns, Message, MessageType, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use lru::LruCache;

use crate::engine::protected_udp;

pub const DEFAULT_SERVER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(77, 88, 8, 8)), 53);
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const TRIES: usize = 2;
const EDNS_PAYLOAD: u16 = 1232;
const MAX_POSITIVE_TTL: u32 = 300;
const NEGATIVE_TTL: Duration = Duration::from_secs(30);
const CACHE_CAPACITY: usize = 512;

struct CacheEntry {
    addresses: Vec<IpAddr>,
    expires: Instant,
}

pub struct Resolver {
    server: SocketAddr,
    timeout: Duration,
    protect: ProtectSocket,
    cache: Mutex<LruCache<(String, bool), CacheEntry>>,
}

pub fn parse_server(value: &str) -> io::Result<SocketAddr> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(DEFAULT_SERVER);
    }
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Ok(address);
    }
    let bare = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(value);
    bare.parse::<IpAddr>()
        .map(|ip| SocketAddr::new(ip, 53))
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "direct resolver must be an IP address with an optional port",
            )
        })
}

impl Resolver {
    pub fn new(server: SocketAddr, protect: ProtectSocket) -> Self {
        Self::with_timeout(server, protect, QUERY_TIMEOUT)
    }

    pub fn with_timeout(server: SocketAddr, protect: ProtectSocket, timeout: Duration) -> Self {
        Self {
            server,
            timeout,
            protect,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).unwrap_or(NonZeroUsize::MIN),
            )),
        }
    }

    pub fn clear(&self) {
        self.cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    pub async fn lookup(&self, host: &str, ipv6: bool) -> io::Result<Vec<IpAddr>> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let (v4, v6) = if ipv6 {
            let (v4, v6) = tokio::join!(
                self.lookup_type(&host, false),
                self.lookup_type(&host, true)
            );
            (v4, Some(v6))
        } else {
            (self.lookup_type(&host, false).await, None)
        };
        let mut addresses = Vec::new();
        let mut failure = None;
        for result in std::iter::once(v4).chain(v6) {
            match result {
                Ok(found) => addresses.extend(found),
                Err(error) => failure = Some(error),
            }
        }
        match (addresses.is_empty(), failure) {
            (true, Some(error)) => Err(error),
            _ => Ok(addresses),
        }
    }

    async fn lookup_type(&self, host: &str, aaaa: bool) -> io::Result<Vec<IpAddr>> {
        let key = (host.to_owned(), aaaa);
        let now = Instant::now();
        {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(entry) = cache.get(&key) {
                if entry.expires > now {
                    return Ok(entry.addresses.clone());
                }
                cache.pop(&key);
            }
        }
        let mut last = io::Error::new(io::ErrorKind::TimedOut, "direct resolver timed out");
        for _ in 0..TRIES {
            match self.query(host, aaaa).await {
                Ok((addresses, ttl)) => {
                    self.cache
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .put(
                            key,
                            CacheEntry {
                                addresses: addresses.clone(),
                                expires: Instant::now() + ttl,
                            },
                        );
                    return Ok(addresses);
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    async fn query(&self, host: &str, aaaa: bool) -> io::Result<(Vec<IpAddr>, Duration)> {
        let kind = if aaaa {
            RecordType::AAAA
        } else {
            RecordType::A
        };
        let name = Name::from_ascii(host)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut id = [0u8; 2];
        rand::fill(&mut id);
        let id = u16::from_be_bytes(id);
        let mut request = Message::query();
        request.metadata.id = id;
        request.metadata.recursion_desired = true;
        request.add_query(Query::query(name.clone(), kind));
        let mut edns = Edns::new();
        edns.set_max_payload(EDNS_PAYLOAD);
        request.edns = Some(edns);
        let wire = request.to_vec().map_err(io::Error::other)?;
        let socket = protected_udp(self.server, &self.protect)?;
        tokio::time::timeout(self.timeout, async {
            socket.send(&wire).await?;
            let mut buffer = vec![0u8; usize::from(EDNS_PAYLOAD) + 512];
            loop {
                let size = socket.recv(&mut buffer).await?;
                let Ok(response) = Message::from_vec(&buffer[..size]) else {
                    continue;
                };
                if response.metadata.id != id
                    || response.metadata.message_type != MessageType::Response
                    || response.queries.len() != 1
                    || response.queries[0].query_type != kind
                    || !response.queries[0].name.eq_ignore_root_case(&name)
                {
                    continue;
                }
                return match response.metadata.response_code {
                    ResponseCode::NoError => Ok(extract(&response, aaaa)),
                    ResponseCode::NXDomain => Ok((Vec::new(), NEGATIVE_TTL)),
                    code => Err(io::Error::other(format!("resolver answered {code}"))),
                };
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "direct resolver timed out"))?
    }
}

fn extract(response: &Message, aaaa: bool) -> (Vec<IpAddr>, Duration) {
    let mut addresses = Vec::new();
    let mut ttl = u32::MAX;
    for record in &response.answers {
        let address = match (&record.data, aaaa) {
            (RData::A(a), false) => IpAddr::V4(a.0),
            (RData::AAAA(a), true) => IpAddr::V6(a.0),
            _ => continue,
        };
        ttl = ttl.min(record.ttl);
        addresses.push(address);
    }
    if addresses.is_empty() {
        return (addresses, NEGATIVE_TTL);
    }
    (
        addresses,
        Duration::from_secs(u64::from(ttl.min(MAX_POSITIVE_TTL))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::UdpSocket;

    async fn server(
        answer: impl Fn(&Message) -> Message + Send + 'static,
    ) -> (SocketAddr, Arc<AtomicUsize>) {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 2048];
            loop {
                let (size, from) = socket.recv_from(&mut buffer).await.unwrap();
                let request = Message::from_vec(&buffer[..size]).unwrap();
                seen.fetch_add(1, Ordering::SeqCst);
                let response = answer(&request);
                socket
                    .send_to(&response.to_vec().unwrap(), from)
                    .await
                    .unwrap();
            }
        });
        (address, count)
    }

    fn reply(request: &Message, code: ResponseCode, ttl: u32, aaaa: bool) -> Message {
        let mut response = Message::response(request.metadata.id, request.metadata.op_code);
        response.metadata.response_code = code;
        let query = request.queries[0].clone();
        response.add_query(query.clone());
        if code == ResponseCode::NoError {
            let data = if aaaa {
                RData::AAAA(hickory_proto::rr::rdata::AAAA("::1".parse().unwrap()))
            } else {
                RData::A(hickory_proto::rr::rdata::A(Ipv4Addr::LOCALHOST))
            };
            response.add_answer(hickory_proto::rr::Record::from_rdata(query.name, ttl, data));
        }
        response
    }

    fn resolver(server: SocketAddr) -> Resolver {
        Resolver::with_timeout(server, Arc::new(|_| Ok(())), Duration::from_millis(300))
    }

    #[test]
    fn server_setting_parses_with_default_port() {
        assert_eq!(parse_server("").unwrap(), DEFAULT_SERVER);
        assert_eq!(
            parse_server("1.1.1.1").unwrap(),
            "1.1.1.1:53".parse().unwrap()
        );
        assert_eq!(parse_server("1.1.1.1:5353").unwrap().port(), 5353);
        assert_eq!(parse_server("::1").unwrap(), "[::1]:53".parse().unwrap());
        assert_eq!(parse_server("[::1]").unwrap(), "[::1]:53".parse().unwrap());
        assert!(parse_server("dns.example").is_err());
    }

    #[tokio::test]
    async fn resolves_a_and_aaaa_and_caches_positive_answers() {
        let (address, count) = server(|request| {
            reply(
                request,
                ResponseCode::NoError,
                600,
                request.queries[0].query_type == RecordType::AAAA,
            )
        })
        .await;
        let resolver = resolver(address);
        let found = resolver.lookup("Direct.Example.", true).await.unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(found.contains(&"::1".parse().unwrap()));
        assert_eq!(count.load(Ordering::SeqCst), 2);
        resolver.lookup("direct.example", true).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let only_v4 = resolver.lookup("direct.example", false).await.unwrap();
        assert_eq!(only_v4, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        resolver.clear();
        resolver.lookup("direct.example", false).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn zero_ttl_answers_are_not_reused_and_nxdomain_is_cached() {
        let (address, count) =
            server(|request| reply(request, ResponseCode::NoError, 0, false)).await;
        let resolver = resolver(address);
        resolver.lookup("a.example", false).await.unwrap();
        resolver.lookup("a.example", false).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let (address, count) =
            server(|request| reply(request, ResponseCode::NXDomain, 0, false)).await;
        let resolver = self::resolver(address);
        assert!(
            resolver
                .lookup("b.example", false)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            resolver
                .lookup("b.example", false)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn server_failure_is_retried_then_reported() {
        let (address, count) =
            server(|request| reply(request, ResponseCode::ServFail, 0, false)).await;
        let resolver = resolver(address);
        assert!(resolver.lookup("c.example", false).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst), TRIES);
    }

    #[tokio::test]
    async fn silent_server_times_out() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let resolver = resolver(socket.local_addr().unwrap());
        let started = Instant::now();
        let error = resolver.lookup("d.example", false).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn every_query_uses_the_protect_hook() {
        let (address, _) = server(|request| reply(request, ResponseCode::NoError, 60, false)).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let resolver = Resolver::with_timeout(
            address,
            Arc::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Duration::from_millis(300),
        );
        resolver.lookup("e.example", true).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
