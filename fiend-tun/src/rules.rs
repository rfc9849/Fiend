use std::collections::HashSet;
use std::net::{IpAddr, Ipv6Addr};

use fiend_core::addr::Address;
use hickory_proto::rr::Name;
use thiserror::Error;

pub const MAX_LIST_BYTES: usize = 1 << 20;
pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_KEYWORDS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    Block,
    Proxy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuleError {
    #[error("rule list is larger than {MAX_LIST_BYTES} bytes")]
    TooLarge,
    #[error("rule list has more than {MAX_ENTRIES} entries")]
    TooManyEntries,
    #[error("rule list has more than {MAX_KEYWORDS} keyword entries")]
    TooManyKeywords,
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: &'static str },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuleCounts {
    pub domains: usize,
    pub exact: usize,
    pub keywords: usize,
    pub cidrs: usize,
}

impl RuleCounts {
    pub fn total(&self) -> usize {
        self.domains + self.exact + self.keywords + self.cidrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct DomainSet {
    exact: HashSet<String>,
    suffixes: HashSet<String>,
    keywords: Vec<String>,
}

impl DomainSet {
    pub fn contains(&self, host: &str) -> bool {
        if self.exact.contains(host) {
            return true;
        }
        let mut rest = host;
        loop {
            if self.suffixes.contains(rest) {
                return true;
            }
            match rest.split_once('.') {
                Some((_, tail)) => rest = tail,
                None => break,
            }
        }
        self.keywords
            .iter()
            .any(|keyword| host.contains(keyword.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.suffixes.is_empty() && self.keywords.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CidrSet {
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
}

impl CidrSet {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match ip.to_canonical() {
            IpAddr::V4(ip) => covers(&self.v4, u32::from(ip)),
            IpAddr::V6(ip) => covers(&self.v6, u128::from(ip)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.v4.is_empty() && self.v6.is_empty()
    }

    fn finish(&mut self) {
        merge(&mut self.v4);
        merge(&mut self.v6);
    }
}

fn covers<T: Ord + Copy>(ranges: &[(T, T)], value: T) -> bool {
    let index = ranges.partition_point(|range| range.0 <= value);
    index > 0 && ranges[index - 1].1 >= value
}

fn merge<T>(ranges: &mut Vec<(T, T)>)
where
    T: Ord + Copy + std::ops::Add<Output = T> + From<u8>,
{
    ranges.sort_unstable();
    let mut merged: Vec<(T, T)> = Vec::with_capacity(ranges.len());
    for &(start, end) in ranges.iter() {
        if let Some(last) = merged.last_mut()
            && (start <= last.1 || (last.1 < end && start == last.1 + T::from(1)))
        {
            if end > last.1 {
                last.1 = end;
            }
            continue;
        }
        merged.push((start, end));
    }
    *ranges = merged;
}

#[derive(Clone, Debug, Default)]
pub struct RuleList {
    pub domains: DomainSet,
    pub cidrs: CidrSet,
    counts: RuleCounts,
}

impl RuleList {
    pub fn counts(&self) -> RuleCounts {
        self.counts
    }
}

pub fn parse_list(text: &str, allow_ip: bool) -> Result<RuleList, RuleError> {
    if text.len() > MAX_LIST_BYTES {
        return Err(RuleError::TooLarge);
    }
    let mut list = RuleList::default();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |reason| RuleError::Line { line, reason };
        let entry = raw.split('#').next().unwrap_or("").trim();
        if entry.is_empty() {
            continue;
        }
        if entry.contains(char::is_whitespace) {
            return Err(fail("entry has spaces"));
        }
        if list.counts.total() >= MAX_ENTRIES {
            return Err(RuleError::TooManyEntries);
        }
        let (kind, value) = match entry.split_once(':') {
            Some((kind, value)) => (kind, value),
            None => ("domain", entry),
        };
        match kind {
            "domain" => {
                let name = normalize(value).ok_or_else(|| fail("domain is not valid"))?;
                list.domains.suffixes.insert(name);
                list.counts.domains += 1;
            }
            "full" => {
                let name = normalize(value).ok_or_else(|| fail("domain is not valid"))?;
                list.domains.exact.insert(name);
                list.counts.exact += 1;
            }
            "keyword" => {
                let keyword = value.to_ascii_lowercase();
                if keyword.is_empty() {
                    return Err(fail("keyword is empty"));
                }
                if list.domains.keywords.len() >= MAX_KEYWORDS {
                    return Err(RuleError::TooManyKeywords);
                }
                list.domains.keywords.push(keyword);
                list.counts.keywords += 1;
            }
            "ip" => {
                if !allow_ip {
                    return Err(fail("ip entries are not allowed in this list"));
                }
                add_cidr(&mut list.cidrs, value).ok_or_else(|| fail("CIDR is not valid"))?;
                list.counts.cidrs += 1;
            }
            _ => return Err(fail("entry type is not known")),
        }
    }
    list.cidrs.finish();
    Ok(list)
}

fn normalize(value: &str) -> Option<String> {
    let value = value.trim_matches('.');
    if value.is_empty() || value.contains(':') || value.contains('/') {
        return None;
    }
    let name = if value.is_ascii() {
        value.to_ascii_lowercase()
    } else {
        Name::from_utf8(value)
            .ok()?
            .to_ascii()
            .trim_end_matches('.')
            .to_ascii_lowercase()
    };
    let valid = name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        });
    valid.then_some(name)
}

fn add_cidr(set: &mut CidrSet, value: &str) -> Option<()> {
    let (address, prefix) = match value.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix.parse::<u8>().ok()?)),
        None => (value, None),
    };
    match address.parse::<IpAddr>().ok()? {
        IpAddr::V4(ip) => {
            let prefix = prefix.unwrap_or(32);
            if prefix > 32 {
                return None;
            }
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            let start = u32::from(ip) & mask;
            set.v4.push((start, start | !mask));
        }
        IpAddr::V6(ip) => {
            let prefix = prefix.unwrap_or(128);
            if prefix > 128 {
                return None;
            }
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            let start = u128::from(ip) & mask;
            set.v6.push((start, start | !mask));
        }
    }
    Some(())
}

pub fn is_local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
    }
}

pub fn is_null_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.octets()[0] == 0,
        IpAddr::V6(ip) => {
            ip == Ipv6Addr::UNSPECIFIED
                || matches!(ip.to_ipv4_mapped(), Some(v4) if v4.octets()[0] == 0)
        }
    }
}

pub fn is_blocked_udp_port(port: u16) -> bool {
    matches!(port, 135 | 137 | 138 | 139)
}

#[derive(Clone, Debug, Default)]
pub struct Rules {
    direct: RuleList,
    block: RuleList,
    all_direct: bool,
}

impl Rules {
    pub fn parse(direct: &str, block: &str) -> Result<Self, RuleError> {
        Ok(Self {
            direct: parse_list(direct, true)?,
            block: parse_list(block, false)?,
            all_direct: false,
        })
    }

    pub fn with_all_direct(mut self, all_direct: bool) -> Self {
        self.all_direct = all_direct;
        self
    }

    pub fn direct_counts(&self) -> RuleCounts {
        self.direct.counts()
    }

    pub fn block_counts(&self) -> RuleCounts {
        self.block.counts()
    }

    pub fn is_blocked(&self, domain: &str) -> bool {
        self.block.domains.contains(domain)
    }

    pub fn route(&self, address: &Address, bypass_lan: bool) -> Route {
        match address {
            Address::Ip(address) => {
                let ip = address.ip();
                if (bypass_lan && is_local(ip)) || self.all_direct || self.direct.cidrs.contains(ip)
                {
                    Route::Direct
                } else {
                    Route::Proxy
                }
            }
            Address::Domain { host, .. } => {
                let host = host.trim_end_matches('.').to_ascii_lowercase();
                if self.block.domains.contains(&host) {
                    Route::Block
                } else if self.all_direct || self.direct.domains.contains(&host) {
                    Route::Direct
                } else {
                    Route::Proxy
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(host: &str) -> Address {
        Address::domain(host, 443)
    }

    fn ip(address: &str) -> Address {
        Address::Ip(address.parse().unwrap())
    }

    #[test]
    fn suffix_matches_on_label_boundaries() {
        let rules = Rules::parse("domain:ru\nyandex.net\n", "").unwrap();
        for host in ["yandex.ru", "ru", "a.b.yandex.net", "yandex.net"] {
            assert_eq!(rules.route(&domain(host), false), Route::Direct, "{host}");
        }
        for host in ["peru", "notyandex.net", "yandex.net.example.com", "ru.com"] {
            assert_eq!(rules.route(&domain(host), false), Route::Proxy, "{host}");
        }
    }

    #[test]
    fn full_entry_matches_only_the_exact_name() {
        let rules = Rules::parse("full:Example.com.\n", "").unwrap();
        assert_eq!(rules.route(&domain("example.com"), false), Route::Direct);
        assert_eq!(rules.route(&domain("EXAMPLE.com."), false), Route::Direct);
        assert_eq!(rules.route(&domain("www.example.com"), false), Route::Proxy);
    }

    #[test]
    fn keyword_matches_a_substring() {
        let rules = Rules::parse("keyword:Yandex\n", "keyword:track\n").unwrap();
        assert_eq!(
            rules.route(&domain("my-yandex-cdn.net"), false),
            Route::Direct
        );
        assert_eq!(rules.route(&domain("tracker.example"), false), Route::Block);
        assert_eq!(rules.route(&domain("example.org"), false), Route::Proxy);
    }

    #[test]
    fn cidr_v4_and_v6_with_overlap_and_adjacent_ranges() {
        let rules = Rules::parse(
            "ip:10.0.0.0/8\nip:10.1.0.0/16\nip:1.2.3.0/25\nip:1.2.3.128/25\nip:5.5.5.5\nip:2a00:1450::/32\nip:::1\nip:0.0.0.0/0\n",
            "",
        )
        .unwrap();
        assert!(rules.direct.cidrs.contains("10.200.0.1".parse().unwrap()));
        assert!(rules.direct.cidrs.contains("1.2.3.0".parse().unwrap()));
        assert!(rules.direct.cidrs.contains("1.2.3.255".parse().unwrap()));
        assert!(rules.direct.cidrs.contains("5.5.5.5".parse().unwrap()));
        assert!(
            rules
                .direct
                .cidrs
                .contains("2a00:1450:4001::1".parse().unwrap())
        );
        assert!(!rules.direct.cidrs.contains("2a01::1".parse().unwrap()));
        assert!(rules.direct.cidrs.contains("::1".parse().unwrap()));
        assert!(
            rules
                .direct
                .cidrs
                .contains("::ffff:10.0.0.1".parse().unwrap())
        );
        let rules = Rules::parse("ip:1.2.3.0/24\nip:2a00:1450::/32\nip:5.5.5.5/32\n", "").unwrap();
        assert_eq!(rules.route(&ip("1.2.3.4:80"), false), Route::Direct);
        assert_eq!(rules.route(&ip("1.2.4.4:80"), false), Route::Proxy);
        assert_eq!(rules.route(&ip("1.2.2.255:80"), false), Route::Proxy);
        assert_eq!(rules.route(&ip("[2a00:1450::1]:80"), false), Route::Direct);
        assert_eq!(rules.route(&ip("[2a00:1451::1]:80"), false), Route::Proxy);
        assert_eq!(rules.route(&ip("5.5.5.4:80"), false), Route::Proxy);
        assert_eq!(rules.route(&ip("5.5.5.6:80"), false), Route::Proxy);
    }

    #[test]
    fn full_range_cidr_does_not_overflow() {
        let rules = Rules::parse("ip:0.0.0.0/0\nip:::/0\nip:255.255.255.255\n", "").unwrap();
        assert_eq!(rules.route(&ip("255.255.255.255:1"), false), Route::Direct);
        assert_eq!(rules.route(&ip("[ffff::]:1"), false), Route::Direct);
    }

    #[test]
    fn decision_order_is_lan_then_block_then_direct_then_proxy() {
        let rules = Rules::parse(
            "domain:both.example\nip:192.168.0.0/16\nip:8.8.8.0/24\n",
            "domain:both.example\n",
        )
        .unwrap();
        assert_eq!(rules.route(&ip("192.168.1.1:80"), true), Route::Direct);
        assert_eq!(rules.route(&ip("192.168.1.1:80"), false), Route::Direct);
        assert_eq!(rules.route(&ip("10.0.0.1:80"), true), Route::Direct);
        assert_eq!(rules.route(&ip("10.0.0.1:80"), false), Route::Proxy);
        assert_eq!(rules.route(&domain("both.example"), true), Route::Block);
        assert_eq!(
            rules.route(&domain("www.both.example"), false),
            Route::Block
        );
        assert_eq!(rules.route(&ip("8.8.8.8:53"), false), Route::Direct);
        assert_eq!(rules.route(&domain("other.example"), true), Route::Proxy);
    }

    #[test]
    fn all_direct_sends_everything_direct_but_keeps_blocks() {
        let rules = Rules::parse("", "domain:ads.example\n")
            .unwrap()
            .with_all_direct(true);
        assert_eq!(rules.route(&domain("example.org"), false), Route::Direct);
        assert_eq!(rules.route(&ip("1.1.1.1:443"), false), Route::Direct);
        assert_eq!(rules.route(&domain("ads.example"), false), Route::Block);
    }

    #[test]
    fn comments_blank_lines_and_unicode_are_handled() {
        let text = "# comment\n\n  domain:example.com   # inline\nfull:a.example\r\nрф\nip:1.0.0.0/8 # net\n";
        let list = parse_list(text, true).unwrap();
        assert_eq!(
            list.counts(),
            RuleCounts {
                domains: 2,
                exact: 1,
                keywords: 0,
                cidrs: 1
            }
        );
        assert!(list.domains.contains("example.com"));
        assert!(list.domains.contains("xn--p1ai"));
        assert!(list.domains.contains("site.xn--p1ai"));
    }

    #[test]
    fn malformed_input_is_rejected_with_a_line_number() {
        for (text, line) in [
            ("ok.example\nregexp:a.*\n", 2),
            ("domain:\n", 1),
            ("domain:a b\n", 1),
            ("full:a..b\n", 1),
            ("ip:1.2.3.4/33\n", 1),
            ("ip:300.1.1.1\n", 1),
            ("ip:::/129\n", 1),
            ("keyword:\n", 1),
            ("bad_host:80\n", 1),
            ("a.com @cn\n", 1),
        ] {
            assert!(
                matches!(parse_list(text, true), Err(RuleError::Line { line: found, .. }) if found == line),
                "{text}"
            );
        }
        assert!(matches!(
            Rules::parse("", "ip:1.2.3.0/24\n"),
            Err(RuleError::Line { line: 1, .. })
        ));
    }

    #[test]
    fn size_and_entry_limits_are_enforced() {
        assert_eq!(
            parse_list(&"a".repeat(MAX_LIST_BYTES + 1), true).unwrap_err(),
            RuleError::TooLarge
        );
        let many = (0..=MAX_ENTRIES)
            .map(|index| format!("{index}.x\n"))
            .collect::<String>();
        assert!(many.len() < MAX_LIST_BYTES);
        assert_eq!(
            parse_list(&many, true).unwrap_err(),
            RuleError::TooManyEntries
        );
        let ok = (0..MAX_ENTRIES)
            .map(|index| format!("{index}.x\n"))
            .collect::<String>();
        assert_eq!(parse_list(&ok, true).unwrap().counts().domains, MAX_ENTRIES);
        let keywords = (0..=MAX_KEYWORDS)
            .map(|index| format!("keyword:k{index}\n"))
            .collect::<String>();
        assert_eq!(
            parse_list(&keywords, true).unwrap_err(),
            RuleError::TooManyKeywords
        );
    }

    #[test]
    fn empty_rules_route_everything_to_the_proxy() {
        let rules = Rules::default();
        assert_eq!(rules.route(&domain("yandex.ru"), true), Route::Proxy);
        assert_eq!(rules.route(&ip("8.8.8.8:53"), true), Route::Proxy);
        assert_eq!(rules.route(&ip("192.168.0.1:53"), true), Route::Direct);
    }

    #[test]
    fn helper_predicates() {
        assert!(is_null_network("0.1.2.3".parse().unwrap()));
        assert!(is_null_network("::".parse().unwrap()));
        assert!(!is_null_network("1.0.0.1".parse().unwrap()));
        assert!(is_blocked_udp_port(137));
        assert!(!is_blocked_udp_port(53));
        assert!(is_local("fd00::1".parse().unwrap()));
        assert!(!is_local("8.8.8.8".parse().unwrap()));
    }
}
