use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::addr::Address;
use crate::client::ClientConfig;
use crate::fingerprint::Template;
use crate::mux;
use crate::server::ServerConfig;
use crate::target::TargetPolicy;
use crate::transport::OriginProfile;

const DEFAULT_CLIENT_LISTEN: &str = "127.0.0.1:1080";
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

pub struct ClientSettings {
    pub listen: SocketAddr,
    pub config: ClientConfig,
}

pub struct ServerSettings {
    pub listen: SocketAddr,
    pub config: ServerConfig,
}

pub fn load_client(path: impl AsRef<Path>) -> Result<ClientSettings, ConfigError> {
    let path = path.as_ref();
    let file: ClientFile = read_config(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));

    let listen = parse_listen(
        file.listen.as_deref().unwrap_or(DEFAULT_CLIENT_LISTEN),
        "client listen",
    )?;
    let server = required(file.server, "client server")?;
    let server = server.trim().to_owned();
    Address::from_authority(&server).map_err(|error| invalid(format!("client server: {error}")))?;

    let secret = load_secret(base, file.secret, file.secret_file)?;
    let template_path = required(file.template, "client template")?;
    let template = Template::load(resolve(base, &template_path))
        .map_err(|error| invalid(format!("client template {template_path}: {error}")))?;

    let handshake_timeout = option_duration(
        file.handshake_timeout_ms,
        Duration::from_secs(10),
        "client handshake_timeout_ms",
    )?;
    let udp_idle = option_duration(
        file.udp.idle_ms,
        Duration::from_secs(60),
        "client udp idle_ms",
    )?;
    let udp_keepalive = option_duration(
        file.udp.keepalive_ms,
        Duration::from_secs(20),
        "client udp keepalive_ms",
    )?;
    if udp_keepalive >= udp_idle {
        return Err(invalid("client udp keepalive_ms must be less than idle_ms"));
    }
    let max_udp_flows = file.udp.max_flows.unwrap_or(64);
    if max_udp_flows == 0 {
        return Err(invalid("client udp max_flows must be greater than zero"));
    }
    let max_connections = file.max_connections.unwrap_or(DEFAULT_MAX_CONNECTIONS);
    if max_connections == 0 {
        return Err(invalid("client max_connections must be greater than zero"));
    }

    let mut mux = mux::Config::default();
    file.mux.apply(&mut mux);
    validate_mux(&mux, "client")?;

    let mut config = ClientConfig::new(server, secret, template);
    config.mux = mux;
    config.udp_idle = udp_idle;
    config.udp_keepalive = udp_keepalive;
    config.max_udp_flows = max_udp_flows;
    config.handshake_timeout = handshake_timeout;
    config.max_connections = max_connections;
    config.tcp_notsent_lowat = socket_size(file.tcp_notsent_lowat, "client", "tcp_notsent_lowat")?;

    Ok(ClientSettings { listen, config })
}

pub fn load_server(path: impl AsRef<Path>) -> Result<ServerSettings, ConfigError> {
    let path = path.as_ref();
    let file: ServerFile = read_config(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));

    let listen = parse_listen(
        file.listen.as_deref().unwrap_or(DEFAULT_SERVER_LISTEN),
        "server listen",
    )?;
    let secret = load_secret(base, file.secret, file.secret_file)?;

    let origin_host = required(file.origin_host, "server origin_host")?;
    let origin_host = origin_host.trim().to_owned();
    if origin_host.is_empty() {
        return Err(invalid("server origin_host must not be empty"));
    }
    let origin_port = file.origin_port.unwrap_or(DEFAULT_ORIGIN_PORT);
    if origin_port == 0 {
        return Err(invalid("server origin_port must be greater than zero"));
    }

    let replay_ttl = option_duration(
        file.replay_ttl_ms,
        Duration::from_secs(3600),
        "server replay_ttl_ms",
    )?;
    let udp_idle = option_duration(
        file.udp.idle_ms,
        Duration::from_secs(60),
        "server udp idle_ms",
    )?;

    let mut mux = mux::Config::default();
    file.mux.apply(&mut mux);
    validate_mux(&mux, "server")?;

    let max_connections = file.max_connections.unwrap_or(DEFAULT_MAX_CONNECTIONS);
    if max_connections == 0 {
        return Err(invalid("server max_connections must be greater than zero"));
    }

    let mut config = ServerConfig::new(secret, origin_host, origin_port);
    config.origin = file.origin.into_profile()?;
    config.replay_ttl = replay_ttl;
    config.mux = mux;
    config.udp_idle = udp_idle;
    config.max_connections = max_connections;
    config.target = file.target.into_policy();
    config.tcp_notsent_lowat = socket_size(file.tcp_notsent_lowat, "server", "tcp_notsent_lowat")?;

    Ok(ServerSettings { listen, config })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientFile {
    listen: Option<String>,
    server: Option<String>,
    secret: Option<String>,
    secret_file: Option<String>,
    template: Option<String>,
    handshake_timeout_ms: Option<u64>,
    max_connections: Option<usize>,
    tcp_notsent_lowat: Option<u32>,
    #[serde(default)]
    udp: ClientUdpFile,
    #[serde(default)]
    mux: MuxFile,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ClientUdpFile {
    idle_ms: Option<u64>,
    keepalive_ms: Option<u64>,
    max_flows: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerFile {
    listen: Option<String>,
    secret: Option<String>,
    secret_file: Option<String>,
    origin_host: Option<String>,
    origin_port: Option<u16>,
    replay_ttl_ms: Option<u64>,
    max_connections: Option<usize>,
    tcp_notsent_lowat: Option<u32>,
    #[serde(default)]
    origin: OriginFile,
    #[serde(default)]
    target: TargetFile,
    #[serde(default)]
    udp: ServerUdpFile,
    #[serde(default)]
    mux: MuxFile,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ServerUdpFile {
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
        if record_sizes.contains(&0) {
            return Err(invalid(
                "server origin record_sizes must be greater than zero",
            ));
        }
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

fn load_secret(
    base: &Path,
    inline: Option<String>,
    secret_file: Option<String>,
) -> Result<Vec<u8>, ConfigError> {
    match (inline, secret_file) {
        (Some(_), Some(_)) => Err(invalid("set either secret or secret_file, not both")),
        (Some(hex), None) => decode_secret(hex.trim()),
        (None, Some(path)) => {
            let path = resolve(base, &path);
            let raw = fs::read(&path).map_err(|source| ConfigError::Read {
                path: path.clone(),
                source,
            })?;
            if raw.len() == SECRET_LEN {
                return Ok(raw);
            }
            let text = String::from_utf8(raw).map_err(|_| {
                invalid(format!(
                    "secret file {} must contain {SECRET_LEN} raw bytes or hex text",
                    path.display()
                ))
            })?;
            decode_secret(text.trim())
        }
        (None, None) => Err(invalid("secret or secret_file is required")),
    }
}

fn decode_secret(text: &str) -> Result<Vec<u8>, ConfigError> {
    let secret =
        hex::decode(text).map_err(|error| invalid(format!("secret is not valid hex: {error}")))?;
    if secret.len() != SECRET_LEN {
        return Err(invalid(format!(
            "secret must be exactly {SECRET_LEN} bytes ({} hex characters)",
            SECRET_LEN * 2
        )));
    }
    Ok(secret)
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

fn resolve(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
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
