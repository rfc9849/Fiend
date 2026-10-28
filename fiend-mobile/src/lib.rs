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
use std::sync::atomic::{AtomicBool, Ordering};
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
    Engine { reason: String },
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

#[derive(uniffi::Record)]
pub struct TunOptions {
    pub mtu: u16,
    pub ipv6: bool,
    pub bypass_lan: bool,
    pub handshake_timeout_ms: u64,
    pub max_connections: u32,
    pub udp_idle_ms: u64,
    pub udp_keepalive_ms: u64,
    pub max_udp_flows: u32,
}

#[derive(uniffi::Record)]
pub struct TunnelStats {
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub active_flows: u64,
    pub dropped_packets: u64,
}

#[derive(uniffi::Object)]
pub struct Tunnel {
    engine: Arc<TunEngine>,
    client: Client,
    platform: Arc<dyn Platform>,
    stopped: Arc<AtomicBool>,
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
        if platform.protect(socket.as_raw_fd()).unwrap_or(false) {
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
        if config.server.parse::<SocketAddr>().is_err() {
            return Err(engine_error(
                "Use a numeric server address to prevent bootstrap DNS leaks",
            ));
        }
        config.handshake_timeout = Duration::from_millis(options.handshake_timeout_ms);
        config.max_connections = options.max_connections as usize;
        config.udp_idle = Duration::from_millis(options.udp_idle_ms);
        config.udp_keepalive = Duration::from_millis(options.udp_keepalive_ms);
        config.max_udp_flows = options.max_udp_flows as usize;
        config.protect_socket = protect(platform.clone());
        let config = fiend_core::config::build_client(config).map_err(engine_error)?;
        let direct_protect = config.protect_socket.clone();
        let max_connections = config.max_connections.min(256);
        let max_udp_flows = config.max_udp_flows;
        let udp_idle = config.udp_idle;
        let client = Client::new(config);
        let engine = Arc::new(
            TunEngine::new(
                client.clone(),
                fiend_tun::TunOptions {
                    mtu: options.mtu,
                    ipv6: options.ipv6,
                    bypass_lan: options.bypass_lan,
                    max_tcp_flows: max_connections,
                    max_udp_flows,
                    udp_idle,
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
        let tunnel = Arc::new(Self {
            engine: engine.clone(),
            client: client.clone(),
            platform: platform.clone(),
            stopped: stopped.clone(),
            task: Mutex::new(None),
        });
        let _ = platform.on_state(TunnelState::Connecting);
        let task = runtime.spawn(async move {
            let result = async {
                client.ready().await.map_err(engine_error)?;
                if stopped.load(Ordering::Acquire) {
                    return Ok(());
                }
                let _ = platform.on_state(TunnelState::Connected);
                engine.run(device).await.map_err(engine_error)
            }
            .await;
            engine.stop();
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
        runtime.spawn(async move {
            let result = client.ready().await;
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
            server: "127.0.0.1:443".into(),
            secret: "42".repeat(32),
            template_json: bundled_templates().remove(0).template_json,
        }
    }

    fn options() -> TunOptions {
        TunOptions {
            mtu: 1500,
            ipv6: true,
            bypass_lan: false,
            handshake_timeout_ms: 10000,
            max_connections: 256,
            udp_idle_ms: 60000,
            udp_keepalive_ms: 20000,
            max_udp_flows: 64,
        }
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

    #[test]
    fn latency_uses_a_real_fiend_handshake() {
        let address =
            runtime().unwrap().block_on(async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let server = fiend_core::server::Server::new(
                    fiend_core::server::ServerConfig::new([0x42; 32], "127.0.0.1", 443),
                );
                tokio::spawn(async move {
                    server.serve(listener).await.unwrap();
                });
                address
            });
        let mut profile = profile();
        profile.server = address.to_string();
        assert!(measure_latency(profile).unwrap() < 10000);
        assert_eq!(runtime().unwrap().metrics().num_workers(), 3);
    }
}

#[uniffi::export]
pub fn measure_latency(profile: ProfileConfig) -> Result<u32, FiendError> {
    let config = profile::client_config(&profile)?;
    if config.server.parse::<SocketAddr>().is_err() {
        return Err(engine_error(
            "Use a numeric server address to prevent bootstrap DNS leaks",
        ));
    }
    let platform = logging::platform();
    let hook = platform
        .map(protect)
        .unwrap_or_else(|| Arc::new(|_| Ok(())));
    runtime()?.block_on(async {
        tokio::time::timeout(config.handshake_timeout, async {
            let start = Instant::now();
            let stream = fiend_core::net::dial(&config.server, &hook)
                .await
                .map_err(engine_error)?;
            let _session = fiend_core::transport::connect(stream, &config.secret, &config.template)
                .await
                .map_err(engine_error)?;
            Ok(u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX))
        })
        .await
        .map_err(engine_error)?
    })
}
