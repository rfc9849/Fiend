use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use fiend_core::addr::Address;
use hickory_proto::op::{Edns, Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::{DNSClass, RData, Record, RecordType, rdata::A};
use lru::LruCache;
use thiserror::Error;

pub const DNS_ADDRESS: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
const FIRST_IP: u32 = u32::from_be_bytes([198, 18, 0, 2]);
const LAST_IP: u32 = u32::from_be_bytes([198, 19, 255, 254]);
const MAX_ENTRIES: usize = (LAST_IP - FIRST_IP + 1) as usize;
const MAX_QUERY: usize = 4096;

#[derive(Debug, Error)]
pub enum DnsError {
    #[error("DNS cache capacity must be between 1 and {MAX_ENTRIES}")]
    Capacity,
    #[error("DNS TTL must be greater than zero")]
    Ttl,
    #[error("DNS packet is too large")]
    PacketSize,
    #[error("DNS packet is not valid")]
    Packet,
    #[error("fake IP address has no active domain")]
    MissingDomain,
}

struct Entry {
    domain: String,
    expires: Instant,
}

pub struct FakeDns {
    entries: LruCache<Ipv4Addr, Entry>,
    domains: HashMap<String, Ipv4Addr>,
    next: u32,
    ttl: u32,
}

impl FakeDns {
    pub fn new(capacity: usize, ttl_seconds: u32) -> Result<Self, DnsError> {
        if capacity > MAX_ENTRIES {
            return Err(DnsError::Capacity);
        }
        let capacity = NonZeroUsize::new(capacity).ok_or(DnsError::Capacity)?;
        if ttl_seconds == 0 {
            return Err(DnsError::Ttl);
        }
        Ok(Self {
            entries: LruCache::new(capacity),
            domains: HashMap::new(),
            next: FIRST_IP,
            ttl: ttl_seconds,
        })
    }

    pub fn answer(&mut self, packet: &[u8], now: Instant) -> Result<Vec<u8>, DnsError> {
        if packet.len() > MAX_QUERY {
            return Err(DnsError::PacketSize);
        }
        let request = match Message::from_vec(packet) {
            Ok(request) => request,
            Err(_) => {
                let id = packet.get(..2).ok_or(DnsError::Packet)?;
                return Message::error_msg(
                    u16::from_be_bytes([id[0], id[1]]),
                    OpCode::Query,
                    ResponseCode::FormErr,
                )
                .to_vec()
                .map_err(|_| DnsError::Packet);
            }
        };
        let mut response = Message::response(request.metadata.id, request.metadata.op_code);
        response.metadata.recursion_desired = request.metadata.recursion_desired;
        response.metadata.recursion_available = true;
        if let Some(edns) = &request.edns {
            let mut reply_edns = Edns::new();
            reply_edns.set_max_payload(edns.max_payload().clamp(512, 1232));
            response.edns = Some(reply_edns);
            if edns.version() != 0 {
                response.metadata.response_code = ResponseCode::BADVERS;
                return response.to_vec().map_err(|_| DnsError::Packet);
            }
        }
        if request.metadata.message_type != MessageType::Query || request.queries.len() != 1 {
            response.metadata.response_code = ResponseCode::FormErr;
        } else if request.metadata.op_code != OpCode::Query {
            response.metadata.response_code = ResponseCode::NotImp;
        } else {
            let query = request.queries[0].clone();
            response.add_query(query.clone());
            if query.query_class != DNSClass::IN {
                response.metadata.response_code = ResponseCode::Refused;
            } else if query.query_type == RecordType::A {
                let domain = query
                    .name
                    .to_ascii()
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                if !valid_domain(&domain) {
                    response.metadata.response_code = ResponseCode::Refused;
                } else if let Some(ip) = self.allocate(&domain, now) {
                    response.add_answer(Record::from_rdata(query.name, self.ttl, RData::A(A(ip))));
                } else {
                    response.metadata.response_code = ResponseCode::ServFail;
                }
            }
        }
        response.to_vec().map_err(|_| DnsError::Packet)
    }

    pub fn destination(&mut self, address: SocketAddr, now: Instant) -> Result<Address, DnsError> {
        let IpAddr::V4(ip) = address.ip() else {
            return Ok(Address::Ip(address));
        };
        if !is_fake_ip(ip) {
            return Ok(Address::Ip(address));
        }
        if self
            .entries
            .peek(&ip)
            .is_some_and(|entry| entry.expires <= now)
        {
            if let Some(entry) = self.entries.pop(&ip) {
                self.domains.remove(&entry.domain);
            }
            return Err(DnsError::MissingDomain);
        }
        let entry = self.entries.get(&ip).ok_or(DnsError::MissingDomain)?;
        Ok(Address::domain(entry.domain.clone(), address.port()))
    }

    fn allocate(&mut self, domain: &str, now: Instant) -> Option<Ipv4Addr> {
        let expires = now.checked_add(Duration::from_secs(u64::from(self.ttl)))?;
        if let Some(ip) = self.domains.get(domain).copied() {
            if let Some(entry) = self.entries.get_mut(&ip)
                && entry.expires > now
            {
                entry.expires = expires;
                return Some(ip);
            }
            self.entries.pop(&ip);
            self.domains.remove(domain);
        }
        if self.next > LAST_IP {
            return None;
        }
        let ip = Ipv4Addr::from(self.next);
        self.next += 1;
        let entry = Entry {
            domain: domain.to_owned(),
            expires,
        };
        if let Some((_, removed)) = self.entries.push(ip, entry) {
            self.domains.remove(&removed.domain);
        }
        self.domains.insert(domain.to_owned(), ip);
        Some(ip)
    }
}

pub fn is_fake_ip(ip: Ipv4Addr) -> bool {
    u32::from(ip) & 0xfffe_0000 == u32::from_be_bytes([198, 18, 0, 0])
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::Query;
    use hickory_proto::rr::Name;

    fn query(domain: &str, kind: RecordType) -> Vec<u8> {
        let mut request = Message::query();
        request.metadata.id = 42;
        request.metadata.recursion_desired = true;
        request.add_query(Query::query(Name::from_ascii(domain).unwrap(), kind));
        request.to_vec().unwrap()
    }

    fn allocate(dns: &mut FakeDns, domain: &str, now: Instant) -> Ipv4Addr {
        let response =
            Message::from_vec(&dns.answer(&query(domain, RecordType::A), now).unwrap()).unwrap();
        assert_eq!(response.metadata.id, 42);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert!(response.metadata.recursion_desired);
        assert!(!response.metadata.authentic_data);
        assert_eq!(response.answers.len(), 1);
        match &response.answers[0].data {
            RData::A(ip) => ip.0,
            _ => panic!("A record required"),
        }
    }

    #[test]
    fn answers_map_back_to_canonical_domains() {
        let now = Instant::now();
        let mut dns = FakeDns::new(16, 60).unwrap();
        let ip = allocate(&mut dns, "WWW.Example.COM.", now);
        assert!(is_fake_ip(ip));
        assert_ne!(ip, DNS_ADDRESS);
        assert_eq!(ip, allocate(&mut dns, "www.example.com", now));
        assert_eq!(
            dns.destination(SocketAddr::new(ip.into(), 443), now)
                .unwrap(),
            Address::domain("www.example.com", 443)
        );
        assert_eq!(dns.entries.len(), 1);
        assert_eq!(dns.domains.len(), 1);
    }

    #[test]
    fn aaaa_https_svcb_and_other_queries_are_empty() {
        let now = Instant::now();
        let mut dns = FakeDns::new(16, 60).unwrap();
        for kind in [
            RecordType::AAAA,
            RecordType::HTTPS,
            RecordType::SVCB,
            RecordType::TXT,
            RecordType::ANY,
        ] {
            let response =
                Message::from_vec(&dns.answer(&query("example.com", kind), now).unwrap()).unwrap();
            assert_eq!(response.metadata.response_code, ResponseCode::NoError);
            assert!(response.answers.is_empty());
            assert_eq!(response.queries[0].query_type, kind);
        }
        assert!(dns.entries.is_empty());
    }

    #[test]
    fn expiry_and_lru_eviction_fail_closed_without_ip_reuse() {
        let now = Instant::now();
        let mut dns = FakeDns::new(2, 60).unwrap();
        let first = allocate(&mut dns, "first.example", now);
        let second = allocate(&mut dns, "second.example", now);
        dns.destination(SocketAddr::new(first.into(), 80), now)
            .unwrap();
        let third = allocate(&mut dns, "third.example", now);
        assert!(
            dns.destination(SocketAddr::new(second.into(), 80), now)
                .is_err()
        );
        assert!(
            dns.destination(SocketAddr::new(third.into(), 80), now)
                .is_ok()
        );
        assert_eq!(dns.domains.len(), 2);
        let later = now + Duration::from_secs(60);
        assert!(
            dns.destination(SocketAddr::new(first.into(), 80), later)
                .is_err()
        );
        let renewed = allocate(&mut dns, "first.example", later);
        assert_ne!(renewed, first);
        assert_ne!(renewed, second);
        assert_ne!(renewed, third);
    }

    #[test]
    fn exhausted_address_space_returns_servfail() {
        let now = Instant::now();
        let mut dns = FakeDns::new(2, 60).unwrap();
        dns.next = LAST_IP;
        allocate(&mut dns, "last.example", now);
        let response = Message::from_vec(
            &dns.answer(&query("next.example", RecordType::A), now)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(response.metadata.response_code, ResponseCode::ServFail);
        assert!(response.answers.is_empty());
    }

    #[test]
    fn invalid_packets_do_not_create_mappings() {
        let mut dns = FakeDns::new(2, 60).unwrap();
        assert!(dns.answer(&[0], Instant::now()).is_err());
        assert!(dns.answer(&vec![0; MAX_QUERY + 1], Instant::now()).is_err());
        let response =
            Message::from_vec(&dns.answer(&[0, 42, 0], Instant::now()).unwrap()).unwrap();
        assert_eq!(response.metadata.id, 42);
        assert_eq!(response.metadata.response_code, ResponseCode::FormErr);
        assert!(dns.entries.is_empty());
        assert!(FakeDns::new(0, 60).is_err());
        assert!(FakeDns::new(MAX_ENTRIES + 1, 60).is_err());
        assert!(FakeDns::new(2, 0).is_err());
    }

    #[test]
    fn rejects_multiple_questions_and_non_in_class() {
        let mut dns = FakeDns::new(2, 60).unwrap();
        let mut request = Message::from_vec(&query("example.com", RecordType::A)).unwrap();
        request.add_query(request.queries[0].clone());
        let response = Message::from_vec(
            &dns.answer(&request.to_vec().unwrap(), Instant::now())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(response.metadata.response_code, ResponseCode::FormErr);
        request.queries.pop();
        request.queries[0].query_class = DNSClass::CH;
        let response = Message::from_vec(
            &dns.answer(&request.to_vec().unwrap(), Instant::now())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(response.metadata.response_code, ResponseCode::Refused);
        assert!(dns.entries.is_empty());
    }

    #[test]
    fn edns_is_bounded_and_never_claims_dnssec() {
        let mut dns = FakeDns::new(2, 60).unwrap();
        let mut request = Message::from_vec(&query("example.com", RecordType::A)).unwrap();
        let mut edns = Edns::new();
        edns.set_max_payload(65535).set_dnssec_ok(true);
        request.edns = Some(edns);
        let response = Message::from_vec(
            &dns.answer(&request.to_vec().unwrap(), Instant::now())
                .unwrap(),
        )
        .unwrap();
        let edns = response.edns.unwrap();
        assert_eq!(edns.max_payload(), 1232);
        assert!(!edns.flags().dnssec_ok);
        assert!(!response.metadata.authentic_data);
        request.edns.as_mut().unwrap().set_version(1);
        let response = Message::from_vec(
            &dns.answer(&request.to_vec().unwrap(), Instant::now())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(u16::from(response.metadata.response_code), 16);
    }

    #[test]
    fn unmapped_fake_addresses_never_become_ip_targets() {
        let now = Instant::now();
        let mut dns = FakeDns::new(2, 60).unwrap();
        for ip in ["198.18.0.0", "198.18.0.1", "198.19.255.255"] {
            assert!(
                dns.destination(format!("{ip}:443").parse().unwrap(), now)
                    .is_err()
            );
        }
        for address in ["1.1.1.1:53", "[2606:4700:4700::1111]:443", "198.20.0.1:443"] {
            let address = address.parse().unwrap();
            assert_eq!(dns.destination(address, now).unwrap(), Address::Ip(address));
        }
    }
}
