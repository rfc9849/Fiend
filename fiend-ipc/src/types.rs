use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    pub handshake_timeout_ms: u32,
    pub max_connections: u32,
    pub udp_idle_ms: u32,
    pub udp_keepalive_ms: u32,
    pub max_udp_flows: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            handshake_timeout_ms: 10_000,
            max_connections: 256,
            udp_idle_ms: 60_000,
            udp_keepalive_ms: 20_000,
            max_udp_flows: 256,
        }
    }
}

impl Tuning {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1_000..=60_000).contains(&self.handshake_timeout_ms) {
            return Err("Handshake timeout must be from 1000 to 60000 ms");
        }
        if !(1..=4_096).contains(&self.max_connections) {
            return Err("Max connections must be from 1 to 4096");
        }
        if !(1_000..=600_000).contains(&self.udp_idle_ms) {
            return Err("UDP idle time must be from 1000 to 600000 ms");
        }
        if !(1_000..=600_000).contains(&self.udp_keepalive_ms) {
            return Err("UDP keepalive must be from 1000 to 600000 ms");
        }
        if !(1..=4_096).contains(&self.max_udp_flows) {
            return Err("Max UDP flows must be from 1 to 4096");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(alias = "server", deserialize_with = "servers")]
    pub servers: Vec<String>,
    pub secret: String,
    pub template_json: String,
    #[serde(default)]
    pub tuning: Tuning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub servers: Vec<String>,
    pub secret: String,
    pub template_json: String,
    #[serde(default)]
    pub tuning: Tuning,
}

fn servers<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Saved::deserialize(deserializer)? {
        Saved::One(server) => vec![server],
        Saved::Many(servers) => servers,
    })
}

impl Profile {
    pub fn server_list(&self) -> String {
        self.servers.join(", ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Direct,
    Block,
    Proxy,
    Dpi,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleStats {
    pub domains: u32,
    pub exact: u32,
    pub keywords: u32,
    pub cidrs: u32,
}

impl RuleStats {
    pub fn total(&self) -> u64 {
        u64::from(self.domains)
            + u64::from(self.exact)
            + u64::from(self.keywords)
            + u64::from(self.cidrs)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub url: String,
    pub kind: RuleKind,
    pub updated_at_ms: u64,
    pub entries: u64,
    pub error: Option<String>,
}

pub const MAX_SUBSCRIPTION_NAME: usize = 64;

impl Subscription {
    pub fn title(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            url_title(&self.url)
        } else {
            name.to_owned()
        }
    }
}

pub fn url_title(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let file = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path);
    let stem = [".srs", ".txt", ".list", ".json"]
        .iter()
        .find_map(|suffix| file.strip_suffix(suffix))
        .unwrap_or(file);
    if stem.is_empty() {
        url.to_owned()
    } else {
        stem.to_owned()
    }
}

pub const MIN_MTU: u16 = 1280;
pub const MAX_MTU: u16 = 9000;
pub const DEFAULT_MTU: u16 = 1500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupData {
    pub profiles: Vec<Profile>,
    pub settings: Settings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub active_profile: Option<String>,
    pub connect_at_startup: bool,
    pub bypass_lan: bool,
    pub ipv6: bool,
    pub mtu: u16,
    pub direct_resolver: String,
    pub direct_rules: String,
    pub block_rules: String,
    pub proxy_rules: String,
    pub dpi_rules: String,
    pub subscriptions: Vec<Subscription>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_profile: None,
            connect_at_startup: false,
            bypass_lan: true,
            ipv6: true,
            mtu: DEFAULT_MTU,
            direct_resolver: String::new(),
            direct_rules: String::new(),
            block_rules: String::new(),
            proxy_rules: String::new(),
            dpi_rules: String::new(),
            subscriptions: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectStep {
    Server,
    Routes,
    Dns,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TunnelState {
    Disconnected,
    Connecting { step: ConnectStep },
    Connected { since_ms: u64 },
    Disconnecting,
    Failed { reason: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub upload_bps: u64,
    pub download_bps: u64,
    pub active_flows: u64,
    pub dropped_packets: u64,
    pub latency_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub state: TunnelState,
    pub profile_id: Option<String>,
    pub stats: Option<Stats>,
    pub restart_required: bool,
    pub daemon_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    NotFound,
    InvalidProfile,
    InvalidRules,
    Busy,
    Engine,
    Unauthorized,
    UnsupportedVersion,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_title_is_the_name_or_the_file_name() {
        let mut item = Subscription {
            id: "a".into(),
            name: String::new(),
            url: "https://raw.example/rules/geosite-ru-blocked.srs?x=1".into(),
            kind: RuleKind::Proxy,
            updated_at_ms: 0,
            entries: 0,
            error: None,
        };
        assert_eq!(item.title(), "geosite-ru-blocked");
        item.name = "  Blocked in Russia ".into();
        assert_eq!(item.title(), "Blocked in Russia");
        assert_eq!(url_title("https://rules.example/"), "rules.example");
        assert_eq!(url_title("https://rules.example/list"), "list");
        let old: Subscription = serde_json::from_str(
            r#"{"id":"a","url":"https://x/y.txt","kind":"dpi","updated_at_ms":0,"entries":0,"error":null}"#,
        )
        .unwrap();
        assert_eq!(old.title(), "y");
    }

    #[test]
    fn saved_profiles_with_one_server_keep_it() {
        let old: Profile = serde_json::from_str(
            r#"{"id":"a","name":"n","server":"192.0.2.1:443","secret":"s","template_json":"{}"}"#,
        )
        .unwrap();
        assert_eq!(old.servers, vec!["192.0.2.1:443"]);
        let saved = serde_json::to_value(&old).unwrap();
        assert_eq!(saved["servers"], serde_json::json!(["192.0.2.1:443"]));
        assert!(saved.get("server").is_none());
        let many: Profile = serde_json::from_value(saved).unwrap();
        assert_eq!(many, old);
    }
}
