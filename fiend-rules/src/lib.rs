use std::collections::HashSet;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use hickory_proto::rr::Name;
use thiserror::Error;

pub mod srs;

use crate::srs::{SrsList, SrsSkipped};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host<'a> {
    Ip(IpAddr),
    Name(&'a str),
}

pub const MAX_LIST_BYTES: usize = 16 << 20;
pub const MAX_ENTRIES: usize = 1_000_000;
pub const MAX_KEYWORDS: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    Block,
    Proxy,
    Dpi,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultRoute {
    #[default]
    Tunnel,
    Direct,
}

impl From<DefaultRoute> for Route {
    fn from(route: DefaultRoute) -> Self {
        match route {
            DefaultRoute::Tunnel => Self::Proxy,
            DefaultRoute::Direct => Self::Direct,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Direct,
    Block,
    Proxy,
    Dpi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleFile {
    pub path: String,
    pub kind: RuleKind,
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

#[derive(Debug, Error)]
pub enum RuleFileError {
    #[error("read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    List { path: PathBuf, source: RuleError },
    #[error("{path}: {source}")]
    Srs {
        path: PathBuf,
        source: srs::SrsError,
    },
    #[error("{path}: rule list is not UTF-8 text")]
    Utf8 { path: PathBuf },
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
    keywords: HashSet<String>,
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

    /// An exact entry that a suffix already matches adds no information.
    fn finish(&mut self) {
        if self.suffixes.is_empty() {
            return;
        }
        let suffixes = &self.suffixes;
        self.exact.retain(|host| {
            let mut rest = host.as_str();
            loop {
                if suffixes.contains(rest) {
                    return false;
                }
                match rest.split_once('.') {
                    Some((_, tail)) => rest = tail,
                    None => return true,
                }
            }
        });
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

    pub fn insert_range(&mut self, from: IpAddr, to: IpAddr) -> bool {
        match (from.to_canonical(), to.to_canonical()) {
            (IpAddr::V4(from), IpAddr::V4(to)) if from <= to => {
                self.v4.push((u32::from(from), u32::from(to)));
                true
            }
            (IpAddr::V6(from), IpAddr::V6(to)) if from <= to => {
                self.v6.push((u128::from(from), u128::from(to)));
                true
            }
            _ => false,
        }
    }

    pub fn ranges(&self) -> impl Iterator<Item = (IpAddr, IpAddr)> + '_ {
        self.v4
            .iter()
            .map(|(from, to)| {
                (
                    IpAddr::V4(Ipv4Addr::from(*from)),
                    IpAddr::V4(Ipv4Addr::from(*to)),
                )
            })
            .chain(self.v6.iter().map(|(from, to)| {
                (
                    IpAddr::V6(Ipv6Addr::from(*from)),
                    IpAddr::V6(Ipv6Addr::from(*to)),
                )
            }))
    }

    pub fn finish(&mut self) {
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

    fn add_suffix(&mut self, host: String) -> Result<(), RuleError> {
        if self.counts.total() >= MAX_ENTRIES {
            return Err(RuleError::TooManyEntries);
        }
        if self.domains.suffixes.insert(host) {
            self.counts.domains += 1;
        }
        Ok(())
    }

    fn add_exact(&mut self, host: String) -> Result<(), RuleError> {
        if self.counts.total() >= MAX_ENTRIES {
            return Err(RuleError::TooManyEntries);
        }
        if self.domains.exact.insert(host) {
            self.counts.exact += 1;
        }
        Ok(())
    }

    fn add_keyword(&mut self, keyword: String) -> Result<(), RuleError> {
        if self.counts.total() >= MAX_ENTRIES {
            return Err(RuleError::TooManyEntries);
        }
        if self.domains.keywords.contains(&keyword) {
            return Ok(());
        }
        if self.domains.keywords.len() >= MAX_KEYWORDS {
            return Err(RuleError::TooManyKeywords);
        }
        self.domains.keywords.insert(keyword);
        self.counts.keywords += 1;
        Ok(())
    }

    fn add_range(&mut self, from: IpAddr, to: IpAddr) -> Result<(), RuleError> {
        if self.counts.total() >= MAX_ENTRIES {
            return Err(RuleError::TooManyEntries);
        }
        if self.cidrs.insert_range(from, to) {
            self.counts.cidrs += 1;
        }
        Ok(())
    }

    pub fn matches(&self, host: Host<'_>) -> bool {
        match host {
            Host::Ip(ip) => self.cidrs.contains(ip),
            Host::Name(host) => {
                let host = host.trim_end_matches('.').to_ascii_lowercase();
                if let Ok(ip) = host.parse::<IpAddr>() {
                    return self.cidrs.contains(ip);
                }
                self.domains.contains(&host)
            }
        }
    }

    pub fn finish(&mut self) {
        self.domains.finish();
        self.cidrs.finish();
        self.counts.exact = self.domains.exact.len();
    }

    fn absorb(&mut self, other: RuleList) -> Result<(), RuleError> {
        for host in other.domains.suffixes {
            self.add_suffix(host)?;
        }
        for host in other.domains.exact {
            self.add_exact(host)?;
        }
        for keyword in other.domains.keywords {
            self.add_keyword(keyword)?;
        }
        for (from, to) in other.cidrs.ranges() {
            self.add_range(from, to)?;
        }
        Ok(())
    }

    fn extend_srs(&mut self, list: SrsList) -> Result<(), RuleError> {
        for host in list.suffixes {
            self.add_suffix(host)?;
        }
        for host in list.exact {
            self.add_exact(host)?;
        }
        for keyword in list.keywords {
            self.add_keyword(keyword.to_ascii_lowercase())?;
        }
        for (from, to) in list.ranges {
            self.add_range(from, to)?;
        }
        Ok(())
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
                list.add_suffix(name)?;
            }
            "full" => {
                let name = normalize(value).ok_or_else(|| fail("domain is not valid"))?;
                list.add_exact(name)?;
            }
            "keyword" => {
                let keyword = value.to_ascii_lowercase();
                if keyword.is_empty() {
                    return Err(fail("keyword is empty"));
                }
                list.add_keyword(keyword)?;
            }
            "ip" => {
                if !allow_ip {
                    return Err(fail("ip entries are not allowed in this list"));
                }
                let (from, to) = parse_cidr(value).ok_or_else(|| fail("CIDR is not valid"))?;
                list.add_range(from, to)?;
            }
            _ => return Err(fail("entry type is not known")),
        }
    }
    list.finish();
    Ok(list)
}

pub(crate) fn normalize(value: &str) -> Option<String> {
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

fn parse_cidr(value: &str) -> Option<(IpAddr, IpAddr)> {
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
            Some((
                IpAddr::V4(Ipv4Addr::from(start)),
                IpAddr::V4(Ipv4Addr::from(start | !mask)),
            ))
        }
        IpAddr::V6(ip) => {
            let prefix = prefix.unwrap_or(128);
            if prefix > 128 {
                return None;
            }
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            let start = u128::from(ip) & mask;
            Some((
                IpAddr::V6(Ipv6Addr::from(start)),
                IpAddr::V6(Ipv6Addr::from(start | !mask)),
            ))
        }
    }
}

pub fn is_local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
    }
}

pub fn is_global(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || first == 0
                || first >= 240
                || (first == 100 && second & 0xc0 == 64))
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
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
    default_route: DefaultRoute,
    direct: RuleList,
    block: RuleList,
    proxy: RuleList,
    dpi: RuleList,
    skipped: SrsSkipped,
}

impl Rules {
    pub fn parse(direct: &str, block: &str, proxy: &str) -> Result<Self, RuleError> {
        Ok(Self {
            default_route: DefaultRoute::default(),
            direct: parse_list(direct, true)?,
            block: parse_list(block, false)?,
            proxy: parse_list(proxy, true)?,
            dpi: RuleList::default(),
            skipped: SrsSkipped::default(),
        })
    }

    pub fn with_default_route(mut self, route: DefaultRoute) -> Self {
        self.default_route = route;
        self
    }

    pub fn with_dpi(mut self, dpi: &str) -> Result<Self, RuleError> {
        self.dpi = parse_list(dpi, true)?;
        Ok(self)
    }

    /// Add one rule file. The format is detected from the magic bytes: a
    /// sing-box binary rule set or a text list.
    pub fn add_file(&mut self, file: &RuleFile) -> Result<RuleCounts, RuleFileError> {
        let path = Path::new(&file.path);
        let fail = |source| RuleFileError::Read {
            path: path.to_owned(),
            source,
        };
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(fail)?
            .take((srs::MAX_SRS_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(fail)?;
        if bytes.len() > srs::MAX_SRS_BYTES {
            return Err(RuleFileError::List {
                path: path.to_owned(),
                source: RuleError::TooLarge,
            });
        }
        let list = match file.kind {
            RuleKind::Direct => &mut self.direct,
            RuleKind::Block => &mut self.block,
            RuleKind::Proxy => &mut self.proxy,
            RuleKind::Dpi => &mut self.dpi,
        };
        let before = list.counts();
        if srs::is_srs(&bytes) {
            let parsed = srs::parse(&bytes).map_err(|source| RuleFileError::Srs {
                path: path.to_owned(),
                source,
            })?;
            self.skipped.add(parsed.skipped);
            list.extend_srs(parsed)
                .map_err(|source| RuleFileError::List {
                    path: path.to_owned(),
                    source,
                })?;
        } else {
            let text = std::str::from_utf8(&bytes).map_err(|_| RuleFileError::Utf8 {
                path: path.to_owned(),
            })?;
            let parsed = parse_list(text, file.kind != RuleKind::Block).map_err(|source| {
                RuleFileError::List {
                    path: path.to_owned(),
                    source,
                }
            })?;
            list.absorb(parsed).map_err(|source| RuleFileError::List {
                path: path.to_owned(),
                source,
            })?;
        }
        list.finish();
        let after = list.counts();
        Ok(RuleCounts {
            domains: after.domains.saturating_sub(before.domains),
            exact: after.exact.saturating_sub(before.exact),
            keywords: after.keywords.saturating_sub(before.keywords),
            cidrs: after.cidrs.saturating_sub(before.cidrs),
        })
    }

    pub fn finish(&mut self) {
        self.direct.finish();
        self.block.finish();
        self.proxy.finish();
        self.dpi.finish();
    }

    pub fn skipped(&self) -> SrsSkipped {
        self.skipped
    }

    pub fn default_route(&self) -> DefaultRoute {
        self.default_route
    }

    pub fn networks(&self, kind: RuleKind) -> &CidrSet {
        match kind {
            RuleKind::Direct => &self.direct.cidrs,
            RuleKind::Block => &self.block.cidrs,
            RuleKind::Proxy => &self.proxy.cidrs,
            RuleKind::Dpi => &self.dpi.cidrs,
        }
    }

    pub fn is_blocked(&self, domain: &str) -> bool {
        self.block.domains.contains(domain)
    }

    pub fn route(&self, host: Host<'_>, bypass_lan: bool) -> Route {
        match host {
            Host::Ip(ip) => {
                if bypass_lan && is_local(ip) {
                    return Route::Direct;
                }
                if self.block.cidrs.contains(ip) {
                    return Route::Block;
                }
                let dpi = self.dpi.cidrs.contains(ip);
                if self.proxy.cidrs.contains(ip) {
                    return Route::Proxy;
                }
                if dpi {
                    return Route::Dpi;
                }
                if self.direct.cidrs.contains(ip) {
                    return Route::Direct;
                }
                self.default_route.into()
            }
            Host::Name(host) => {
                let host = host.trim_end_matches('.').to_ascii_lowercase();
                if self.block.domains.contains(&host) {
                    return Route::Block;
                }
                let dpi = self.dpi.domains.contains(&host);
                if self.proxy.domains.contains(&host) {
                    return Route::Proxy;
                }
                if dpi {
                    return Route::Dpi;
                }
                if self.direct.domains.contains(&host) {
                    return Route::Direct;
                }
                self.default_route.into()
            }
        }
    }
}

impl SrsSkipped {
    fn add(&mut self, other: SrsSkipped) {
        self.rules += other.rules;
        self.regex += other.regex;
        self.adguard += other.adguard;
        self.logical += other.logical;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(host: &str) -> Host<'_> {
        Host::Name(host)
    }

    fn ip(address: &str) -> Host<'static> {
        Host::Ip(address.parse::<std::net::SocketAddr>().unwrap().ip())
    }

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn default_route_applies_only_without_a_match() {
        for (default_route, expected) in [
            (DefaultRoute::Tunnel, Route::Proxy),
            (DefaultRoute::Direct, Route::Direct),
        ] {
            let rules = Rules::parse(
                "domain:direct.example\nip:192.0.2.0/24",
                "domain:blocked.example",
                "domain:proxy.example\nip:198.51.100.0/24",
            )
            .unwrap()
            .with_dpi("domain:dpi.example\nip:203.0.113.0/24")
            .unwrap()
            .with_default_route(default_route);
            for host in [
                domain("unlisted.example"),
                ip("8.8.8.8:443"),
                ip("[2001:db8::1]:443"),
            ] {
                assert_eq!(rules.route(host, false), expected);
            }
            for (host, route) in [
                (domain("direct.example"), Route::Direct),
                (domain("blocked.example"), Route::Block),
                (domain("proxy.example"), Route::Proxy),
                (domain("dpi.example"), Route::Dpi),
                (ip("192.0.2.1:443"), Route::Direct),
                (ip("198.51.100.1:443"), Route::Proxy),
                (ip("203.0.113.1:443"), Route::Dpi),
            ] {
                assert_eq!(rules.route(host, false), route);
            }
            assert_eq!(rules.route(ip("192.168.1.1:443"), true), Route::Direct);
        }
        assert_eq!(
            Rules::default().route(domain("unlisted.example"), false),
            Route::Proxy
        );
    }

    #[test]
    fn subscription_rules_override_the_default_route() {
        let mut rules = Rules::default().with_default_route(DefaultRoute::Direct);
        rules
            .add_file(&RuleFile {
                path: fixture("geosite-2gis.srs"),
                kind: RuleKind::Proxy,
            })
            .unwrap();
        assert_eq!(rules.route(domain("2gis.ru"), false), Route::Proxy);
        assert_eq!(
            rules.route(domain("unlisted.example"), false),
            Route::Direct
        );
    }

    #[test]
    fn suffix_matches_on_label_boundaries() {
        let rules = Rules::parse("domain:ru\nyandex.net\n", "", "").unwrap();
        for host in ["yandex.ru", "ru", "a.b.yandex.net", "yandex.net"] {
            assert_eq!(rules.route(domain(host), false), Route::Direct, "{host}");
        }
        for host in ["peru", "notyandex.net", "yandex.net.example.com", "ru.com"] {
            assert_eq!(rules.route(domain(host), false), Route::Proxy, "{host}");
        }
    }

    #[test]
    fn full_entry_matches_only_the_exact_name() {
        let rules = Rules::parse("full:Example.com.\n", "", "").unwrap();
        assert_eq!(rules.route(domain("example.com"), false), Route::Direct);
        assert_eq!(rules.route(domain("EXAMPLE.com."), false), Route::Direct);
        assert_eq!(rules.route(domain("www.example.com"), false), Route::Proxy);
    }

    #[test]
    fn keyword_matches_a_substring() {
        let rules = Rules::parse("keyword:Yandex\n", "keyword:track\n", "").unwrap();
        assert_eq!(
            rules.route(domain("my-yandex-cdn.net"), false),
            Route::Direct
        );
        assert_eq!(rules.route(domain("tracker.example"), false), Route::Block);
        assert_eq!(rules.route(domain("example.org"), false), Route::Proxy);
    }

    #[test]
    fn cidr_v4_and_v6_with_overlap_and_adjacent_ranges() {
        let rules = Rules::parse(
            "ip:10.0.0.0/8\nip:10.1.0.0/16\nip:1.2.3.0/25\nip:1.2.3.128/25\nip:5.5.5.5\nip:2a00:1450::/32\nip:::1\nip:0.0.0.0/0\n",
            "",
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
        let rules =
            Rules::parse("ip:1.2.3.0/24\nip:2a00:1450::/32\nip:5.5.5.5/32\n", "", "").unwrap();
        assert_eq!(rules.route(ip("1.2.3.4:80"), false), Route::Direct);
        assert_eq!(rules.route(ip("1.2.4.4:80"), false), Route::Proxy);
        assert_eq!(rules.route(ip("1.2.2.255:80"), false), Route::Proxy);
        assert_eq!(rules.route(ip("[2a00:1450::1]:80"), false), Route::Direct);
        assert_eq!(rules.route(ip("[2a00:1451::1]:80"), false), Route::Proxy);
        assert_eq!(rules.route(ip("5.5.5.4:80"), false), Route::Proxy);
        assert_eq!(rules.route(ip("5.5.5.6:80"), false), Route::Proxy);
    }

    #[test]
    fn full_range_cidr_does_not_overflow() {
        let rules = Rules::parse("ip:0.0.0.0/0\nip:::/0\nip:255.255.255.255\n", "", "").unwrap();
        assert_eq!(rules.route(ip("255.255.255.255:1"), false), Route::Direct);
        assert_eq!(rules.route(ip("[ffff::]:1"), false), Route::Direct);
    }

    #[test]
    fn decision_order_is_lan_then_block_then_direct_then_proxy() {
        let rules = Rules::parse(
            "domain:both.example\nip:192.168.0.0/16\nip:8.8.8.0/24\n",
            "domain:both.example\n",
            "",
        )
        .unwrap();
        assert_eq!(rules.route(ip("192.168.1.1:80"), true), Route::Direct);
        assert_eq!(rules.route(ip("192.168.1.1:80"), false), Route::Direct);
        assert_eq!(rules.route(ip("10.0.0.1:80"), true), Route::Direct);
        assert_eq!(rules.route(ip("10.0.0.1:80"), false), Route::Proxy);
        assert_eq!(rules.route(domain("both.example"), true), Route::Block);
        assert_eq!(rules.route(domain("www.both.example"), false), Route::Block);
        assert_eq!(rules.route(ip("8.8.8.8:53"), false), Route::Direct);
        assert_eq!(rules.route(domain("other.example"), true), Route::Proxy);
    }

    #[test]
    fn proxy_list_wins_over_the_direct_list() {
        let rules = Rules::parse(
            "domain:ru\nip:5.255.255.0/24\n",
            "domain:ads.example\n",
            "domain:ru-blocked.example\nip:5.255.255.0/24\n",
        )
        .unwrap();
        assert_eq!(
            rules.route(domain("www.ru-blocked.example"), false),
            Route::Proxy
        );
        assert_eq!(
            rules.route(domain("ru-blocked.example"), false),
            Route::Proxy
        );
        assert_eq!(rules.route(domain("yandex.ru"), false), Route::Direct);
        assert_eq!(rules.route(domain("ads.example"), false), Route::Block);
        assert_eq!(rules.route(ip("5.255.255.9:443"), false), Route::Proxy);
        assert_eq!(rules.route(ip("8.8.8.8:443"), false), Route::Proxy);
        assert_eq!(rules.proxy.counts().domains, 1);
    }

    #[test]
    fn dpi_list_is_between_the_proxy_and_direct_lists() {
        let rules = Rules::parse(
            "domain:example\nip:5.0.0.0/8\n",
            "domain:ads.example\n",
            "domain:proxy.example\n",
        )
        .unwrap()
        .with_dpi("domain:example\nip:5.5.0.0/16\n")
        .unwrap();
        assert_eq!(rules.route(domain("video.example"), false), Route::Dpi);
        assert_eq!(
            rules.route(domain("www.proxy.example"), false),
            Route::Proxy
        );
        assert_eq!(rules.route(domain("ads.example"), false), Route::Block);
        assert_eq!(rules.route(domain("other.org"), false), Route::Proxy);
        assert_eq!(rules.route(ip("5.5.1.1:443"), false), Route::Dpi);
        assert_eq!(rules.route(ip("5.6.1.1:443"), false), Route::Direct);
        assert_eq!(rules.dpi.counts().total(), 2);
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
    fn an_exact_entry_inside_a_suffix_is_dropped() {
        let list = parse_list(
            "domain:example.com\nfull:example.com\nfull:www.example.com\n",
            true,
        )
        .unwrap();
        assert_eq!(list.counts().domains, 1);
        assert_eq!(list.counts().exact, 0);
        assert!(list.domains.contains("example.com"));
        assert!(list.domains.contains("www.example.com"));
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
            Rules::parse("", "ip:1.2.3.0/24\n", ""),
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
        assert_eq!(rules.route(domain("yandex.ru"), true), Route::Proxy);
        assert_eq!(rules.route(ip("8.8.8.8:53"), true), Route::Proxy);
        assert_eq!(rules.route(ip("192.168.0.1:53"), true), Route::Direct);
    }

    #[test]
    fn helper_predicates() {
        assert!(is_null_network("0.1.2.3".parse().unwrap()));
        assert!(is_null_network("::".parse().unwrap()));
        assert!(!is_null_network("1.0.0.1".parse().unwrap()));
        assert!(is_blocked_udp_port(137));
        assert!(!is_blocked_udp_port(53));
        assert!(is_local("fd00::1".parse().unwrap()));
        for ip in ["8.8.8.8", "2606:4700::1111", "100.128.0.1"] {
            assert!(is_global(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "0.0.0.0",
            "127.0.0.1",
            "10.1.1.1",
            "192.168.1.1",
            "100.64.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_global(ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_local("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn binary_rule_sets_load_from_a_file() {
        let mut rules = Rules::default();
        let counts = rules
            .add_file(&RuleFile {
                path: fixture("geosite-2gis.srs"),
                kind: RuleKind::Direct,
            })
            .unwrap();
        assert!(counts.total() > 0);
        rules.finish();
        assert_eq!(rules.route(domain("2gis.ru"), false), Route::Direct);
        assert_eq!(rules.route(domain("www.2gis.ru"), false), Route::Direct);
        assert_eq!(rules.route(domain("2gis.example"), false), Route::Proxy);
        assert_eq!(rules.skipped(), SrsSkipped::default());
    }

    #[test]
    fn binary_ip_rule_sets_load_from_a_file() {
        let mut rules = Rules::default();
        rules
            .add_file(&RuleFile {
                path: fixture("geoip-ru.srs"),
                kind: RuleKind::Direct,
            })
            .unwrap();
        rules.finish();
        assert!(rules.direct.counts().cidrs > 1000);
        assert_eq!(rules.route(ip("5.255.255.1:443"), false), Route::Direct);
    }

    #[test]
    fn text_files_load_from_a_file() {
        let path = std::env::temp_dir().join("fiend-rules-text-test.txt");
        std::fs::write(&path, "domain:example.com\nip:10.0.0.0/8\n").unwrap();
        let mut rules = Rules::default();
        rules
            .add_file(&RuleFile {
                path: path.to_string_lossy().into_owned(),
                kind: RuleKind::Direct,
            })
            .unwrap();
        rules.finish();
        assert_eq!(rules.route(domain("a.example.com"), false), Route::Direct);
        assert_eq!(rules.route(ip("10.1.2.3:80"), false), Route::Direct);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let mut rules = Rules::default();
        let error = rules
            .add_file(&RuleFile {
                path: "/nonexistent/fiend.srs".to_owned(),
                kind: RuleKind::Direct,
            })
            .unwrap_err();
        assert!(matches!(error, RuleFileError::Read { .. }));
    }
}

#[cfg(test)]
mod match_tests {
    use super::*;

    #[test]
    fn rule_lists_match_domains_and_addresses() {
        let list = parse_list(
            "domain:example.com\nfull:exact.test\nkeyword:tracker\nip:10.0.0.0/8",
            true,
        )
        .unwrap();
        let domain = Host::Name;
        assert!(list.matches(domain("a.example.com")));
        assert!(list.matches(domain("Example.COM.")));
        assert!(list.matches(domain("exact.test")));
        assert!(!list.matches(domain("sub.exact.test")));
        assert!(list.matches(domain("my-tracker.net")));
        assert!(list.matches(Host::Ip("10.1.2.3".parse().unwrap())));
        assert!(list.matches(domain("10.1.2.3")));
        assert!(!list.matches(Host::Ip("11.1.2.3".parse().unwrap())));
        assert!(!list.matches(domain("other.org")));
    }
}
