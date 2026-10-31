#![deny(unsafe_op_in_unsafe_fn)]

mod logging;
mod profile;
pub use profile::{
    ProfileConfig, TemplateInfo, bundled_templates, make_share_link, parse_share_link,
    validate_profile,
};

use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fiend_core::client::Client;
use fiend_tun::TunEngine;
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};

uniffi::setup_scaffolding!();

#[derive(Debug, Error, uniffi::Error)]
pub enum FiendError {
    #[error("{reason}")]
    InvalidProfile { reason: String },
    #[error("Share link is not valid or is too large")]
    InvalidLink,
    #[error("{reason}")]
    InvalidRules { reason: String },
    #[error("{reason}")]
    Engine { reason: String },
}

impl From<fiend_app::AppError> for FiendError {
    fn from(error: fiend_app::AppError) -> Self {
        use fiend_app::AppError;
        match error {
            AppError::InvalidProfile { reason } => Self::InvalidProfile { reason },
            AppError::InvalidLink => Self::InvalidLink,
            AppError::InvalidRules { reason } => Self::InvalidRules { reason },
            AppError::Engine { reason } => Self::Engine { reason },
        }
    }
}

#[derive(Debug, Error, uniffi::Error)]
pub enum PlatformError {
    #[error("Platform callback failed")]
    Callback,
}

impl From<uniffi::UnexpectedUniFFICallbackError> for PlatformError {
    fn from(_: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Callback
    }
}

#[derive(Clone, uniffi::Enum)]
pub enum TunnelState {
    Disconnected,
    Connecting,
    Connected,
    Failed { reason: String },
}

#[derive(Clone, uniffi::Record)]
pub struct LogLine {
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
}

#[uniffi::export(with_foreign)]
pub trait Platform: Send + Sync {
    fn protect(&self, fd: i32) -> Result<bool, PlatformError>;
    fn on_state(&self, state: TunnelState) -> Result<(), PlatformError>;
    fn on_log(&self, line: LogLine) -> Result<(), PlatformError>;
}

#[derive(Clone, Copy, uniffi::Enum)]
pub enum RuleKind {
    Direct,
    Block,
    Proxy,
    Dpi,
}

#[derive(Clone, uniffi::Record)]
pub struct RuleFile {
    pub path: String,
    pub kind: RuleKind,
}

#[derive(uniffi::Record)]
pub struct TunOptions {
    pub mtu: u16,
    pub ipv6: bool,
    pub bypass_lan: bool,
    pub direct_rules: String,
    pub block_rules: String,
    pub proxy_rules: String,
    pub dpi_rules: String,
    pub rule_files: Vec<RuleFile>,
    pub direct_resolver: String,
    pub handshake_timeout_ms: u64,
    pub max_connections: u32,
    pub udp_idle_ms: u64,
    pub max_udp_flows: u32,
}

#[derive(uniffi::Record)]
pub struct RuleStats {
    pub domains: u32,
    pub exact: u32,
    pub keywords: u32,
    pub cidrs: u32,
}

impl From<RuleKind> for fiend_app::RuleKind {
    fn from(kind: RuleKind) -> Self {
        match kind {
            RuleKind::Direct => Self::Direct,
            RuleKind::Block => Self::Block,
            RuleKind::Proxy => Self::Proxy,
            RuleKind::Dpi => Self::Dpi,
        }
    }
}

impl From<fiend_app::RuleStats> for RuleStats {
    fn from(stats: fiend_app::RuleStats) -> Self {
        Self {
            domains: stats.domains,
            exact: stats.exact,
            keywords: stats.keywords,
            cidrs: stats.cidrs,
        }
    }
}

#[uniffi::export]
pub fn validate_rules(text: String, allow_ip: bool) -> Result<RuleStats, FiendError> {
    Ok(fiend_app::validate_rules(&text, allow_ip)?.into())
}

#[uniffi::export]
pub fn inspect_rule_file(path: String, kind: RuleKind) -> Result<RuleStats, FiendError> {
    Ok(fiend_app::inspect_rule_file(&path, kind.into())?.into())
}

#[uniffi::export]
pub fn validate_resolver(value: String) -> Result<(), FiendError> {
    Ok(fiend_app::validate_resolver(&value)?)
}

#[derive(uniffi::Record)]
pub struct TunnelStats {
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub active_flows: u64,
    pub dropped_packets: u64,
    pub latency_ms: Option<u32>,
}

#[derive(uniffi::Object)]
pub struct Tunnel {
    engine: Arc<TunEngine>,
    client: Client,
    platform: Arc<dyn Platform>,
    stopped: Arc<AtomicBool>,
    latency_ms: Arc<AtomicU64>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

fn runtime() -> Result<&'static Runtime, FiendError> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(3)
                .thread_name("fiend-engine")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|reason| FiendError::Engine {
            reason: reason.clone(),
        })
}

fn engine_error(error: impl std::fmt::Display) -> FiendError {
    FiendError::Engine {
        reason: error.to_string(),
    }
}

fn protect(platform: Arc<dyn Platform>) -> fiend_core::net::ProtectSocket {
    Arc::new(move |socket| {
        let allowed = platform.protect(socket.as_raw_fd()).unwrap_or(false);
        tracing::info!(allowed, "socket protection result");
        #[cfg(target_os = "android")]
        tracing::info!(mark = ?socket.mark(), "protected socket mark");
        if allowed {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "VPN socket protection failed",
            ))
        }
    })
}

#[uniffi::export]
impl Tunnel {
    #[uniffi::constructor]
    pub fn start(
        tun_fd: i32,
        profile: ProfileConfig,
        options: TunOptions,
        platform: Arc<dyn Platform>,
    ) -> Result<Arc<Self>, FiendError> {
        if tun_fd < 0 {
            return Err(engine_error("TUN descriptor is not valid"));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(tun_fd) };
        let runtime = runtime()?;
        logging::initialize(runtime);
        let mut config = profile::client_config(&profile)?;
        numeric_servers(&config)?;
        config.handshake_timeout = Duration::from_millis(options.handshake_timeout_ms);
        config.protect_socket = protect(platform.clone());
        let config = fiend_core::config::build_client(config).map_err(engine_error)?;
        let direct_protect = config.protect_socket.clone();
        let max_connections = (options.max_connections as usize).min(256);
        let max_udp_flows = options.max_udp_flows as usize;
        let udp_idle = Duration::from_millis(options.udp_idle_ms);
        let client = Client::new(config);
        let engine = Arc::new(
            TunEngine::new(
                client.clone(),
                fiend_tun::TunOptions {
                    mtu: options.mtu,
                    ipv6: options.ipv6,
                    bypass_lan: options.bypass_lan,
                    direct_rules: options.direct_rules,
                    block_rules: options.block_rules,
                    proxy_rules: options.proxy_rules,
                    dpi_rules: options.dpi_rules,
                    rule_files: options
                        .rule_files
                        .into_iter()
                        .map(|file| fiend_tun::rules::RuleFile {
                            path: file.path,
                            kind: match file.kind {
                                RuleKind::Direct => fiend_tun::rules::RuleKind::Direct,
                                RuleKind::Block => fiend_tun::rules::RuleKind::Block,
                                RuleKind::Proxy => fiend_tun::rules::RuleKind::Proxy,
                                RuleKind::Dpi => fiend_tun::rules::RuleKind::Dpi,
                            },
                        })
                        .collect(),
                    direct_resolver: options.direct_resolver,
                    max_tcp_flows: max_connections,
                    max_udp_flows,
                    udp_idle,
                    fake_dns: Some(fiend_tun::shared_fake_dns()),
                    protect_socket: direct_protect,
                    ..fiend_tun::TunOptions::default()
                },
            )
            .map_err(engine_error)?,
        );
        let device = {
            let _enter = runtime.enter();
            unsafe { tun_rs::AsyncDevice::from_fd(fd.into_raw_fd()) }.map_err(engine_error)?
        };
        logging::set_platform(&platform);
        let stopped = Arc::new(AtomicBool::new(false));
        let latency_ms = Arc::new(AtomicU64::new(u64::MAX));
        let tunnel = Arc::new(Self {
            engine: engine.clone(),
            client: client.clone(),
            platform: platform.clone(),
            stopped: stopped.clone(),
            latency_ms: latency_ms.clone(),
            task: Mutex::new(None),
        });
        let _ = platform.on_state(TunnelState::Connecting);
        let task = runtime.spawn(async move {
            tracing::info!("tunnel handshake started");
            let result = async {
                let start = Instant::now();
                client.ready().await.map_err(engine_error)?;
                latency_ms.store(
                    start.elapsed().as_millis().min(u128::from(u32::MAX)) as u64,
                    Ordering::Release,
                );
                tracing::info!("tunnel handshake completed");
                if stopped.load(Ordering::Acquire) {
                    return Ok(());
                }
                let _ = platform.on_state(TunnelState::Connected);
                engine.run(device).await.map_err(engine_error)
            }
            .await;
            engine.stop();
            if let Err(error) = &result {
                tracing::error!(%error, "tunnel stopped with an error");
            }
            if stopped.load(Ordering::Acquire) {
                let _ = platform.on_state(TunnelState::Disconnected);
            } else if let Err(error) = result {
                let _ = platform.on_state(TunnelState::Failed {
                    reason: error.to_string(),
                });
            } else {
                let _ = platform.on_state(TunnelState::Disconnected);
            }
        });
        *tunnel
            .task
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(task);
        Ok(tunnel)
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.engine.stop();
    }

    pub fn stats(&self) -> TunnelStats {
        let stats = self.engine.stats();
        TunnelStats {
            sent_bytes: stats.uploaded,
            received_bytes: stats.downloaded,
            active_flows: stats.flows,
            dropped_packets: stats.dropped_packets,
            latency_ms: u32::try_from(self.latency_ms.load(Ordering::Acquire)).ok(),
        }
    }

    pub fn network_changed(&self) -> Result<(), FiendError> {
        if self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let runtime = runtime()?;
        let _enter = runtime.enter();
        let _ = self.platform.on_state(TunnelState::Connecting);
        self.engine.network_changed();
        let client = self.client.clone();
        let platform = self.platform.clone();
        let stopped = self.stopped.clone();
        let latency_ms = self.latency_ms.clone();
        runtime.spawn(async move {
            let start = Instant::now();
            let result = client.ready().await;
            if result.is_ok() {
                latency_ms.store(
                    start.elapsed().as_millis().min(u128::from(u32::MAX)) as u64,
                    Ordering::Release,
                );
            }
            if stopped.load(Ordering::Acquire) {
                return;
            }
            let state = match result {
                Ok(()) => TunnelState::Connected,
                Err(error) => TunnelState::Failed {
                    reason: error.to_string(),
                },
            };
            let _ = platform.on_state(state);
        });
        Ok(())
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicUsize;

    struct TestPlatform {
        allowed: bool,
        calls: AtomicUsize,
    }
    impl Platform for TestPlatform {
        fn protect(&self, fd: i32) -> Result<bool, PlatformError> {
            assert!(fd >= 0);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.allowed)
        }
        fn on_state(&self, _: TunnelState) -> Result<(), PlatformError> {
            Ok(())
        }
        fn on_log(&self, _: LogLine) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    fn profile() -> ProfileConfig {
        ProfileConfig {
            name: "Local test".into(),
            servers: vec!["127.0.0.1:443".into()],
            secret: "42".repeat(32),
            template_json: bundled_templates().remove(0).template_json,
        }
    }

    fn options() -> TunOptions {
        TunOptions {
            mtu: 1500,
            ipv6: true,
            bypass_lan: false,
            direct_rules: String::new(),
            block_rules: String::new(),
            proxy_rules: String::new(),
            dpi_rules: String::new(),
            rule_files: Vec::new(),
            direct_resolver: String::new(),
            handshake_timeout_ms: 10000,
            max_connections: 256,
            udp_idle_ms: 60000,
            max_udp_flows: 64,
        }
    }

    #[test]
    fn rule_functions_keep_the_exported_api() {
        let stats = validate_rules("domain:a.ru\nip:1.2.3.0/24\n".into(), true).unwrap();
        assert_eq!((stats.domains, stats.cidrs), (1, 1));
        assert!(matches!(
            validate_rules("regexp:x".into(), true),
            Err(FiendError::InvalidRules { .. })
        ));
        assert!(validate_resolver("dns.example".into()).is_err());
        assert!(inspect_rule_file("/nonexistent/fiend.srs".into(), RuleKind::Direct).is_err());
    }

    #[test]
    fn start_closes_transferred_fd_on_profile_error() {
        let (owned, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut profile = profile();
        profile.secret.clear();
        let platform = Arc::new(TestPlatform {
            allowed: true,
            calls: AtomicUsize::new(0),
        });
        assert!(Tunnel::start(owned.into_raw_fd(), profile, options(), platform).is_err());
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn protect_callback_failure_stops_the_dial() {
        let platform = Arc::new(TestPlatform {
            allowed: false,
            calls: AtomicUsize::new(0),
        });
        let hook = protect(platform.clone());
        let error = runtime()
            .unwrap()
            .block_on(fiend_core::net::dial("127.0.0.1:443", &hook))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(platform.calls.load(Ordering::SeqCst), 1);
    }

    fn managed_server(
        secret: &[u8],
        config: fiend_core::server::ServerConfig,
    ) -> fiend_core::server::Server {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "fiend-mobile-test-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let mut state = fiend_core::node::DesiredState::initial(&config);
        state.users.push(fiend_core::node::User {
            id: "test".into(),
            secret: secret.iter().map(|byte| format!("{byte:02x}")).collect(),
            enabled: true,
            allowed: true,
            expire_at: None,
            traffic_limit: None,
            used_traffic: 0,
            usage_epoch: 0,
        });
        let node = fiend_core::node::Node::load(&path, vec![0x5c; 32], state).unwrap();
        fiend_core::server::Server::managed(config, node)
    }

    #[test]
    fn latency_uses_a_real_fiend_handshake() {
        let address = runtime().unwrap().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = managed_server(
                &[0x42; 32],
                fiend_core::server::ServerConfig::new("127.0.0.1", 443),
            );
            tokio::spawn(async move {
                server.serve(listener).await.unwrap();
            });
            address
        });
        let mut profile = profile();
        profile.servers = vec![address.to_string()];
        assert!(measure_latency(profile).unwrap() < 10000);
        assert_eq!(runtime().unwrap().metrics().num_workers(), 3);
    }
}

fn numeric_servers(config: &fiend_core::client::ClientConfig) -> Result<(), FiendError> {
    if config
        .servers
        .iter()
        .any(|server| server.parse::<SocketAddr>().is_err())
    {
        return Err(engine_error(
            "Use numeric server addresses to prevent bootstrap DNS leaks",
        ));
    }
    Ok(())
}

#[uniffi::export]
pub fn measure_latency(profile: ProfileConfig) -> Result<u32, FiendError> {
    let profile: fiend_app::ProfileConfig = profile.into();
    let config = fiend_app::client_config(&profile)?;
    numeric_servers(&config)?;
    let hook = logging::platform()
        .map(protect)
        .unwrap_or_else(|| Arc::new(|_| Ok(())));
    Ok(runtime()?.block_on(fiend_app::measure_latency(&profile, &hook))?)
}
