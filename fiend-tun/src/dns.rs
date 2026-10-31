use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::sync::Arc;

use fiend_core::addr::Address;
use hickory_proto::op::{Edns, Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::{DNSClass, RData, Record, RecordType, rdata::A};
use lru::LruCache;

use crate::rules::Rules;
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

pub struct FakeDns {
    entries: LruCache<Ipv4Addr, String>,
    domains: HashMap<String, Ipv4Addr>,
    next: u32,
    ttl: u32,
    rules: Option<Arc<Rules>>,
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
            rules: None,
        })
    }

    pub fn with_rules(mut self, rules: Arc<Rules>) -> Self {
        self.rules = Some(rules);
        self
    }

    pub fn set_rules(&mut self, rules: Arc<Rules>) {
        self.rules = Some(rules);
    }

    pub fn answer(&mut self, packet: &[u8]) -> Result<Vec<u8>, DnsError> {
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
                } else if self
                    .rules
                    .as_ref()
                    .is_some_and(|rules| rules.is_blocked(&domain))
                {
                    response.metadata.response_code = ResponseCode::NXDomain;
                } else {
                    let ip = self.allocate(&domain);
                    response.add_answer(Record::from_rdata(query.name, self.ttl, RData::A(A(ip))));
                }
            }
        }
        response.to_vec().map_err(|_| DnsError::Packet)
    }

    pub fn destination(&mut self, address: SocketAddr) -> Result<Address, DnsError> {
        let IpAddr::V4(ip) = address.ip() else {
            return Ok(Address::Ip(address));
        };
        if !is_fake_ip(ip) {
            return Ok(Address::Ip(address));
        }
        let domain = self.entries.get(&ip).ok_or(DnsError::MissingDomain)?;
        Ok(Address::domain(domain.clone(), address.port()))
    }

    fn allocate(&mut self, domain: &str) -> Ipv4Addr {
        if let Some(ip) = self.domains.get(domain).copied()
            && self.entries.get(&ip).is_some()
        {
            return ip;
        }
        let ip = loop {
            let ip = Ipv4Addr::from(self.next);
            self.next = if self.next >= LAST_IP {
                FIRST_IP
            } else {
                self.next + 1
            };
            if !self.entries.contains(&ip) {
                break ip;
            }
            if self.entries.len() >= self.entries.cap().get()
                && let Some((ip, removed)) = self.entries.pop_lru()
            {
                self.domains.remove(&removed);
                break ip;
            }
        };
        if let Some((_, removed)) = self.entries.push(ip, domain.to_owned()) {
            self.domains.remove(&removed);
        }
        self.domains.insert(domain.to_owned(), ip);
        ip
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

    fn allocate(dns: &mut FakeDns, domain: &str) -> Ipv4Addr {
        let response =
            Message::from_vec(&dns.answer(&query(domain, RecordType::A)).unwrap()).unwrap();
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
        let mut dns = FakeDns::new(16, 60).unwrap();
        let ip = allocate(&mut dns, "WWW.Example.COM.");
        assert!(is_fake_ip(ip));
        assert_ne!(ip, DNS_ADDRESS);
        assert_eq!(ip, allocate(&mut dns, "www.example.com"));
        assert_eq!(
            dns.destination(SocketAddr::new(ip.into(), 443)).unwrap(),
            Address::domain("www.example.com", 443)
        );
        assert_eq!(dns.entries.len(), 1);
        assert_eq!(dns.domains.len(), 1);
    }

    #[test]
    fn aaaa_https_svcb_and_other_queries_are_empty() {
        let mut dns = FakeDns::new(16, 60).unwrap();
        for kind in [
            RecordType::AAAA,
            RecordType::HTTPS,
            RecordType::SVCB,
            RecordType::TXT,
            RecordType::ANY,
        ] {
            let response =
                Message::from_vec(&dns.answer(&query("example.com", kind)).unwrap()).unwrap();
            assert_eq!(response.metadata.response_code, ResponseCode::NoError);
            assert!(response.answers.is_empty());
            assert_eq!(response.queries[0].query_type, kind);
        }
        assert!(dns.entries.is_empty());
    }

    #[test]
    fn mappings_outlive_the_ttl_and_lru_eviction_fails_closed() {
        let mut dns = FakeDns::new(2, 1).unwrap();
        let first = allocate(&mut dns, "first.example");
        let second = allocate(&mut dns, "second.example");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(
            dns.destination(SocketAddr::new(first.into(), 80)).unwrap(),
            Address::domain("first.example", 80)
        );
        assert_eq!(first, allocate(&mut dns, "first.example"));
        let third = allocate(&mut dns, "third.example");
        assert!(dns.destination(SocketAddr::new(second.into(), 80)).is_err());
        assert!(dns.destination(SocketAddr::new(third.into(), 80)).is_ok());
        assert_eq!(dns.domains.len(), 2);
        let renewed = allocate(&mut dns, "second.example");
        assert_ne!(renewed, first);
        assert_ne!(renewed, second);
        assert_ne!(renewed, third);
    }

    #[test]
    fn exhausted_address_space_wraps_and_skips_live_addresses() {
        let mut dns = FakeDns::new(3, 60).unwrap();
        let first = allocate(&mut dns, "first.example");
        assert_eq!(first, Ipv4Addr::from(FIRST_IP));
        dns.next = LAST_IP;
        assert_eq!(allocate(&mut dns, "last.example"), Ipv4Addr::from(LAST_IP));
        allocate(&mut dns, "first.example");
        let wrapped = allocate(&mut dns, "next.example");
        assert_eq!(wrapped, Ipv4Addr::from(FIRST_IP + 1));
        assert_eq!(
            dns.destination(SocketAddr::new(first.into(), 443)).unwrap(),
            Address::domain("first.example", 443)
        );
        assert_eq!(
            dns.destination(SocketAddr::new(Ipv4Addr::from(LAST_IP).into(), 443))
                .unwrap(),
            Address::domain("last.example", 443)
        );
    }

    #[test]
    fn a_full_pool_reuses_the_least_recent_address() {
        let mut dns = FakeDns::new(MAX_ENTRIES, 60).unwrap();
        for index in 0..MAX_ENTRIES {
            dns.allocate(&format!("d{index}.example"));
        }
        allocate(&mut dns, "d0.example");
        let reused = allocate(&mut dns, "new.example");
        assert_eq!(reused, Ipv4Addr::from(FIRST_IP + 1));
        assert!(!dns.domains.contains_key("d1.example"));
        assert_eq!(
            dns.destination(SocketAddr::new(Ipv4Addr::from(FIRST_IP).into(), 443))
                .unwrap(),
            Address::domain("d0.example", 443)
        );
        assert_eq!(dns.entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn invalid_packets_do_not_create_mappings() {
        let mut dns = FakeDns::new(2, 60).unwrap();
        assert!(dns.answer(&[0]).is_err());
        assert!(dns.answer(&vec![0; MAX_QUERY + 1]).is_err());
        let response = Message::from_vec(&dns.answer(&[0, 42, 0]).unwrap()).unwrap();
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
        let response = Message::from_vec(&dns.answer(&request.to_vec().unwrap()).unwrap()).unwrap();
        assert_eq!(response.metadata.response_code, ResponseCode::FormErr);
        request.queries.pop();
        request.queries[0].query_class = DNSClass::CH;
        let response = Message::from_vec(&dns.answer(&request.to_vec().unwrap()).unwrap()).unwrap();
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
        let response = Message::from_vec(&dns.answer(&request.to_vec().unwrap()).unwrap()).unwrap();
        let edns = response.edns.unwrap();
        assert_eq!(edns.max_payload(), 1232);
        assert!(!edns.flags().dnssec_ok);
        assert!(!response.metadata.authentic_data);
        request.edns.as_mut().unwrap().set_version(1);
        let response = Message::from_vec(&dns.answer(&request.to_vec().unwrap()).unwrap()).unwrap();
        assert_eq!(u16::from(response.metadata.response_code), 16);
    }

    #[test]
    fn unmapped_fake_addresses_never_become_ip_targets() {
        let mut dns = FakeDns::new(2, 60).unwrap();
        for ip in ["198.18.0.0", "198.18.0.1", "198.19.255.255"] {
            assert!(
                dns.destination(format!("{ip}:443").parse().unwrap())
                    .is_err()
            );
        }
        for address in ["1.1.1.1:53", "[2606:4700:4700::1111]:443", "198.20.0.1:443"] {
            let address = address.parse().unwrap();
            assert_eq!(dns.destination(address).unwrap(), Address::Ip(address));
        }
    }
}
