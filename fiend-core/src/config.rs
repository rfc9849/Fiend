use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::client::ClientConfig;
use crate::mux;
use crate::server::ServerConfig;
use crate::target::TargetPolicy;
use crate::transport::{AUTH_WINDOW, OriginProfile};

const DEFAULT_SERVER_LISTEN: &str = "0.0.0.0:443";
const DEFAULT_ORIGIN_PORT: u16 = 443;
const DEFAULT_MAX_CONNECTIONS: usize = 1024;
const SECRET_LEN: usize = 32;
const MIN_FRAME_SIZE: u32 = 16 << 10;
const MAX_FRAME_SIZE: u32 = 0x00ff_ffff;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(String),
}

pub struct ServerSettings {
    pub listen: SocketAddr,
    pub config: ServerConfig,
    pub node: NodeSettings,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodeSettings {
    pub state_file: PathBuf,
    pub control_secret_file: PathBuf,
    pub control_socket: PathBuf,
    pub wireproxy: PathBuf,
}

impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            state_file: PathBuf::from("/var/lib/fiend/state.json"),
            control_secret_file: PathBuf::from("/etc/fiend/control.secret"),
            control_socket: PathBuf::from("/run/fiend/control.sock"),
            wireproxy: PathBuf::from("/usr/local/bin/wireproxy"),
        }
    }
}

pub fn load_server(path: impl AsRef<Path>) -> Result<ServerSettings, ConfigError> {
    let path = path.as_ref();
    let file: ServerFile = read_config(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));

    let listen = parse_listen(
        file.listen.as_deref().unwrap_or(DEFAULT_SERVER_LISTEN),
        "server listen",
    )?;
    let origin_host = required(file.origin_host, "server origin_host")?;
    let origin_host = origin_host.trim().to_owned();
    let origin_port = file.origin_port.unwrap_or(DEFAULT_ORIGIN_PORT);

    let replay_ttl = option_duration(
        file.replay_ttl_ms,
        AUTH_WINDOW + Duration::from_secs(300),
        "server replay_ttl_ms",
    )?;
    let udp_idle = option_duration(
        file.udp.idle_ms,
        Duration::from_secs(60),
        "server udp idle_ms",
    )?;

    let tcp_idle = option_duration(
        file.tcp.idle_ms,
        Duration::from_secs(300),
        "server tcp idle_ms",
    )?;

    let mut mux = mux::Config::default();
    file.mux.apply(&mut mux);

    let max_connections = file.max_connections.unwrap_or(DEFAULT_MAX_CONNECTIONS);

    let mut config = ServerConfig::new(origin_host, origin_port);
    config.origin = file.origin.into_profile()?;
    config.replay_ttl = replay_ttl;
    config.mux = mux;
    config.udp_idle = udp_idle;
    config.max_connections = max_connections;
    if let Some(limit) = file.max_user_sessions {
        config.max_user_sessions = limit;
    }
    if let Some(limit) = file.max_user_lanes {
        config.max_user_lanes = limit;
    }
    config.tcp_idle = tcp_idle;
    config.target = file.target.into_policy();
    config.tcp_notsent_lowat = socket_size(file.tcp_notsent_lowat, "server", "tcp_notsent_lowat")?;

    let mut node = file.node;
    let node_base = std::fs::canonicalize(if base.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base
    })
    .unwrap_or_else(|_| base.to_path_buf());
    for path in [
        &mut node.state_file,
        &mut node.control_secret_file,
        &mut node.control_socket,
        &mut node.wireproxy,
    ] {
        if path.is_relative() {
            *path = node_base.join(&path);
        }
    }
    Ok(ServerSettings {
        listen,
        config: build_server(config)?,
        node,
    })
}

pub fn build_client(mut config: ClientConfig) -> Result<ClientConfig, ConfigError> {
    for server in &mut config.servers {
        *server = server.trim().to_owned();
    }
    crate::share::check_servers(&config.servers)
        .map_err(|error| invalid(format!("client servers: {error}")))?;
    validate_secret(&config.secret)?;
    config
        .template
        .validate()
        .map_err(|error| invalid(format!("client template: {error}")))?;
    validate_mux(&config.mux, "client")?;
    validate_duration(config.handshake_timeout, "client handshake_timeout_ms")?;
    socket_size(
        Some(config.tcp_notsent_lowat),
        "client",
        "tcp_notsent_lowat",
    )?;
    Ok(config)
}

pub fn build_server(mut config: ServerConfig) -> Result<ServerConfig, ConfigError> {
    config.origin_host = config.origin_host.trim().to_owned();
    if config.origin_host.is_empty() {
        return Err(invalid("server origin_host must not be empty"));
    }
    if config.origin_port == 0 {
        return Err(invalid("server origin_port must be greater than zero"));
    }
    if config.replay_ttl < AUTH_WINDOW {
        return Err(invalid(format!(
            "server replay_ttl_ms must be at least {} (the authentication window)",
            AUTH_WINDOW.as_millis()
        )));
    }
    validate_duration(config.udp_idle, "server udp idle_ms")?;
    validate_mux(&config.mux, "server")?;
    if config.max_connections == 0 {
        return Err(invalid("server max_connections must be greater than zero"));
    }
    if config.max_user_sessions == 0 || config.max_user_lanes == 0 {
        return Err(invalid(
            "server max_user_sessions and max_user_lanes must be greater than zero",
        ));
    }
    validate_duration(config.tcp_idle, "server tcp idle_ms")?;
    if config.origin.record_sizes.contains(&0) {
        return Err(invalid(
            "server origin record_sizes must be greater than zero",
        ));
    }
    socket_size(
        Some(config.tcp_notsent_lowat),
        "server",
        "tcp_notsent_lowat",
    )?;
    Ok(config)
}

fn validate_secret(secret: &[u8]) -> Result<(), ConfigError> {
    if secret.len() != SECRET_LEN {
        return Err(invalid(format!(
            "secret must be exactly {SECRET_LEN} bytes ({} hex characters)",
            SECRET_LEN * 2
        )));
    }
    Ok(())
}

fn validate_duration(value: Duration, field: &str) -> Result<(), ConfigError> {
    if value.is_zero() {
        return Err(invalid(format!("{field} must be greater than zero")));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerFile {
    #[serde(default)]
    node: NodeSettings,
    listen: Option<String>,
    origin_host: Option<String>,
    origin_port: Option<u16>,
    replay_ttl_ms: Option<u64>,
    max_connections: Option<usize>,
    max_user_sessions: Option<usize>,
    max_user_lanes: Option<usize>,
    tcp_notsent_lowat: Option<u32>,
    #[serde(default)]
    origin: OriginFile,
    #[serde(default)]
    target: TargetFile,
    #[serde(default)]
    tcp: IdleFile,
    #[serde(default)]
    udp: IdleFile,
    #[serde(default)]
    mux: MuxFile,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct IdleFile {
    idle_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TargetFile {
    allow_loopback: Option<bool>,
    allow_private: Option<bool>,
    allow_link_local: Option<bool>,
    allow_cgnat: Option<bool>,
}

impl TargetFile {
    fn into_policy(self) -> TargetPolicy {
        let default = TargetPolicy::default();
        TargetPolicy {
            allow_loopback: self.allow_loopback.unwrap_or(default.allow_loopback),
            allow_private: self.allow_private.unwrap_or(default.allow_private),
            allow_link_local: self.allow_link_local.unwrap_or(default.allow_link_local),
            allow_cgnat: self.allow_cgnat.unwrap_or(default.allow_cgnat),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OriginFile {
    cipher_suite: Option<u16>,
    key_share_group: Option<u16>,
    extension_order: Option<Vec<u16>>,
    record_sizes: Option<Vec<usize>>,
}

impl OriginFile {
    fn into_profile(self) -> Result<OriginProfile, ConfigError> {
        let default = OriginProfile::default();
        let record_sizes = self.record_sizes.unwrap_or(default.record_sizes);
        Ok(OriginProfile {
            cipher_suite: self.cipher_suite.unwrap_or(default.cipher_suite),
            key_share_group: self.key_share_group.unwrap_or(default.key_share_group),
            extension_order: self.extension_order.unwrap_or(default.extension_order),
            record_sizes,
        })
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MuxFile {
    max_links: Option<usize>,
    bulk_lane_bytes: Option<u64>,
    lane_share: Option<usize>,
    open_timeout_ms: Option<u64>,
    settings_timeout_ms: Option<u64>,
    target_timeout_ms: Option<u64>,
    ping_interval_ms: Option<u64>,
    probe_timeout_ms: Option<u64>,
    rtt_interval_ms: Option<u64>,
    surplus_idle_ms: Option<u64>,
    attempts: Option<usize>,
    backoff_floor_ms: Option<u64>,
    backoff_ceiling_ms: Option<u64>,
    max_frame_size: Option<u32>,
    max_concurrent_streams: Option<u32>,
    max_send_buffer_size: Option<usize>,
}

impl MuxFile {
    fn apply(self, config: &mut mux::Config) {
        if let Some(value) = self.max_links {
            config.max_links = value;
        }
        if let Some(value) = self.bulk_lane_bytes {
            config.bulk_lane_bytes = value;
        }
        if let Some(value) = self.lane_share {
            config.lane_share = value;
        }
        if let Some(value) = self.open_timeout_ms {
            config.open_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.settings_timeout_ms {
            config.settings_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.target_timeout_ms {
            config.target_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.ping_interval_ms {
            config.ping_interval = Duration::from_millis(value);
        }
        if let Some(value) = self.probe_timeout_ms {
            config.probe_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.rtt_interval_ms {
            config.rtt_interval = Some(Duration::from_millis(value));
        }
        if let Some(value) = self.surplus_idle_ms {
            config.surplus_idle = Duration::from_millis(value);
        }
        if let Some(value) = self.attempts {
            config.attempts = value;
        }
        if let Some(value) = self.backoff_floor_ms {
            config.backoff_floor = Duration::from_millis(value);
        }
        if let Some(value) = self.backoff_ceiling_ms {
            config.backoff_ceiling = Duration::from_millis(value);
        }
        if let Some(value) = self.max_frame_size {
            config.max_frame_size = value;
        }
        if let Some(value) = self.max_concurrent_streams {
            config.max_concurrent_streams = value;
        }
        if let Some(value) = self.max_send_buffer_size {
            config.max_send_buffer_size = value;
        }
    }
}

fn validate_mux(config: &mux::Config, role: &str) -> Result<(), ConfigError> {
    if config.max_links == 0 {
        return Err(invalid(format!(
            "{role} mux max_links must be greater than zero"
        )));
    }
    if config.attempts == 0 {
        return Err(invalid(format!(
            "{role} mux attempts must be greater than zero"
        )));
    }
    if config.bulk_lane_bytes == 0 {
        return Err(invalid(format!(
            "{role} mux bulk_lane_bytes must be greater than zero"
        )));
    }
    if config.lane_share == 0 {
        return Err(invalid(format!(
            "{role} mux lane_share must be greater than zero"
        )));
    }
    if config.max_send_buffer_size == 0 {
        return Err(invalid(format!(
            "{role} mux max_send_buffer_size must be greater than zero"
        )));
    }
    if config.max_send_buffer_size > u32::MAX as usize {
        return Err(invalid(format!(
            "{role} mux max_send_buffer_size must not exceed {}",
            u32::MAX
        )));
    }
    if config.max_concurrent_streams == 0 {
        return Err(invalid(format!(
            "{role} mux max_concurrent_streams must be greater than zero"
        )));
    }
    if !(MIN_FRAME_SIZE..=MAX_FRAME_SIZE).contains(&config.max_frame_size) {
        return Err(invalid(format!(
            "{role} mux max_frame_size must be between {MIN_FRAME_SIZE} and {MAX_FRAME_SIZE}"
        )));
    }
    for (name, duration) in [
        ("open_timeout_ms", config.open_timeout),
        ("settings_timeout_ms", config.settings_timeout),
        ("target_timeout_ms", config.target_timeout),
        ("ping_interval_ms", config.ping_interval),
        ("probe_timeout_ms", config.probe_timeout),
        ("surplus_idle_ms", config.surplus_idle),
        (
            "rtt_interval_ms",
            config.rtt_interval.unwrap_or(config.ping_interval),
        ),
    ] {
        if duration.is_zero() {
            return Err(invalid(format!(
                "{role} mux {name} must be greater than zero"
            )));
        }
    }
    if config.backoff_floor.is_zero() {
        return Err(invalid(format!(
            "{role} mux backoff_floor_ms must be greater than zero"
        )));
    }
    if config.backoff_ceiling < config.backoff_floor {
        return Err(invalid(format!(
            "{role} mux backoff_ceiling_ms must not be less than backoff_floor_ms"
        )));
    }
    Ok(())
}

fn read_config<T: DeserializeOwned>(path: &Path) -> Result<T, ConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&raw).map_err(|error| {
        let message = match error.span() {
            Some(span) => {
                let (line, column) = line_column(&raw, span.start);
                format!("{} at line {line}, column {column}", error.message())
            }
            None => error.message().to_owned(),
        };
        ConfigError::Parse {
            path: path.to_owned(),
            message,
        }
    })
}

fn option_duration(
    value: Option<u64>,
    default: Duration,
    field: &str,
) -> Result<Duration, ConfigError> {
    match value {
        Some(0) => Err(invalid(format!("{field} must be greater than zero"))),
        Some(value) => Ok(Duration::from_millis(value)),
        None => Ok(default),
    }
}

fn socket_size(value: Option<u32>, role: &str, field: &str) -> Result<u32, ConfigError> {
    let value = value.unwrap_or(0);
    if value > i32::MAX as u32 {
        return Err(invalid(format!(
            "{role} {field} must be between 0 and {}",
            i32::MAX
        )));
    }
    Ok(value)
}

fn parse_listen(value: &str, field: &str) -> Result<SocketAddr, ConfigError> {
    value
        .parse()
        .map_err(|error| invalid(format!("{field} {value}: {error}")))
}

fn required(value: Option<String>, field: &str) -> Result<String, ConfigError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid(format!("{field} is required")))
}

fn line_column(raw: &str, offset: usize) -> (usize, usize) {
    let prefix = raw.get(..offset).unwrap_or(raw);
    let mut line = 1;
    let mut column = 1;
    for character in prefix.chars() {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}
