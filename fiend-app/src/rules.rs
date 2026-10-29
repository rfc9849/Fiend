use crate::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Direct,
    Block,
    Proxy,
}

impl From<RuleKind> for fiend_tun::rules::RuleKind {
    fn from(kind: RuleKind) -> Self {
        match kind {
            RuleKind::Direct => Self::Direct,
            RuleKind::Block => Self::Block,
            RuleKind::Proxy => Self::Proxy,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuleStats {
    pub domains: u32,
    pub exact: u32,
    pub keywords: u32,
    pub cidrs: u32,
}

impl From<fiend_tun::rules::RuleCounts> for RuleStats {
    fn from(counts: fiend_tun::rules::RuleCounts) -> Self {
        let count = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        Self {
            domains: count(counts.domains),
            exact: count(counts.exact),
            keywords: count(counts.keywords),
            cidrs: count(counts.cidrs),
        }
    }
}

impl RuleStats {
    pub fn total(&self) -> u64 {
        u64::from(self.domains)
            + u64::from(self.exact)
            + u64::from(self.keywords)
            + u64::from(self.cidrs)
    }
}

pub fn validate_rules(text: &str, allow_ip: bool) -> Result<RuleStats, AppError> {
    fiend_tun::rules::parse_list(text, allow_ip)
        .map(|list| list.counts().into())
        .map_err(AppError::rules)
}

pub fn inspect_rule_file(path: &str, kind: RuleKind) -> Result<RuleStats, AppError> {
    let mut rules = fiend_tun::rules::Rules::default();
    rules
        .add_file(&fiend_tun::rules::RuleFile {
            path: path.to_owned(),
            kind: kind.into(),
        })
        .map(Into::into)
        .map_err(AppError::rules)
}

pub fn validate_resolver(value: &str) -> Result<(), AppError> {
    fiend_tun::parse_resolver(value)
        .map(drop)
        .map_err(AppError::rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_validation_counts_entries_and_reports_errors() {
        let stats =
            validate_rules("domain:a.ru\nfull:b.ru\nkeyword:x\nip:1.2.3.0/24\n", true).unwrap();
        assert_eq!(
            (stats.domains, stats.exact, stats.keywords, stats.cidrs),
            (1, 1, 1, 1)
        );
        assert_eq!(stats.total(), 4);
        assert!(validate_rules("ip:1.2.3.0/24", false).is_err());
        assert!(validate_rules("regexp:x", true).is_err());
        assert!(validate_resolver("").is_ok());
        assert!(validate_resolver("dns.example").is_err());
    }

    #[test]
    fn rule_files_are_inspected() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fiend-tun/tests/data/geosite-2gis.srs")
            .to_string_lossy()
            .into_owned();
        let stats = inspect_rule_file(&path, RuleKind::Direct).unwrap();
        assert!(stats.domains > 0);
        assert!(inspect_rule_file(&path, RuleKind::Block).is_ok());
        assert!(inspect_rule_file("/nonexistent/fiend.srs", RuleKind::Direct).is_err());
    }
}
