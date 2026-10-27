use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::split;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, Semaphore};
use tokio::time::{sleep, timeout};

use crate::addr::Address;
use crate::mux::{self, Incoming, LaneKind, ServerConnection, handle_tcp};
use crate::target::{self, TargetPolicy};
use crate::transport::{self, FreshnessCheck, OriginProfile, Session};
use crate::udp;

const UDP_BUFFER: usize = u16::MAX as usize;

#[derive(Clone)]
pub struct ServerConfig {
    pub secret: Vec<u8>,
    pub origin_host: String,
    pub origin_port: u16,
    pub origin: OriginProfile,
    pub replay_ttl: Duration,
    pub mux: mux::Config,
    pub udp_idle: Duration,
    pub max_connections: usize,
    pub target: TargetPolicy,
    pub tcp_notsent_lowat: u32,
}

impl ServerConfig {
    pub fn new(
        secret: impl Into<Vec<u8>>,
        origin_host: impl Into<String>,
        origin_port: u16,
    ) -> Self {
        Self {
            secret: secret.into(),
            origin_host: origin_host.into(),
            origin_port,
            origin: OriginProfile::default(),
            replay_ttl: transport::AUTH_WINDOW + Duration::from_secs(300),
            mux: mux::Config::default(),
            udp_idle: Duration::from_secs(60),
            max_connections: 1024,
            target: TargetPolicy::default(),
            tcp_notsent_lowat: 0,
        }
    }
}

#[derive(Clone)]
pub struct Server {
    inner: Arc<ServerInner>,
}

struct ServerInner {
    config: ServerConfig,
    replay: FreshnessCheck,
    connections: Arc<Semaphore>,
}

impl Server {
    pub fn new(config: ServerConfig) -> Self {
        let replay = FreshnessCheck::new(config.replay_ttl.max(transport::AUTH_WINDOW));
        let connections = Arc::new(Semaphore::new(config.max_connections.max(1)));
        let mut config = config;
        if config.origin.record_sizes.is_empty() {
            let suite = if config.origin.cipher_suite == 0 {
                0x1301
            } else {
                config.origin.cipher_suite
            };
            config.origin.record_sizes = transport::default_flight_sizes(suite);
        }
        Self {
            inner: Arc::new(ServerInner {
                config,
                replay,
                connections,
            }),
        }
    }

    pub async fn serve(&self, listener: TcpListener) -> io::Result<()> {
        loop {
            let permit = self
                .inner
                .connections
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| io::Error::other("connection limit closed"))?;
            let (stream, _) = crate::net::accept(&listener).await;
            if let Err(error) = stream.set_nodelay(true) {
                tracing::debug!(%error, "cannot set TCP_NODELAY");
            }
            if let Err(error) = crate::net::set_tcp_keepalive(&stream) {
                tracing::debug!(%error, "cannot set TCP keepalive");
            }
            if let Err(error) =
                crate::net::set_tcp_notsent_lowat(&stream, self.inner.config.tcp_notsent_lowat)
            {
                tracing::warn!(%error, "cannot set TCP_NOTSENT_LOWAT");
            }
            let server = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                if let Err(error) = server.handle(stream).await {
                    tracing::debug!(%error, "connection handler failed");
                }
            });
        }
    }

    async fn handle(&self, stream: TcpStream) -> mux::Result<()> {
        let config = &self.inner.config;
        let outcome = transport::serve_connection(
            stream,
            &config.secret,
            &self.inner.replay,
            &config.origin,
            &config.origin_host,
            config.origin_port,
        )
        .await?;

        if let Some(session) = outcome {
            serve_mux(session, config.clone()).await?;
        }
        Ok(())
    }
}

async fn serve_mux(session: Session<TcpStream>, config: ServerConfig) -> mux::Result<()> {
    let mut connection = ServerConnection::handshake(session, &config.mux).await?;
    while let Some(incoming) = connection.accept().await? {
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_incoming(incoming, config).await {
                tracing::debug!(%error, "lane handler failed");
            }
        });
    }
    Ok(())
}

async fn handle_incoming(incoming: Incoming, config: ServerConfig) -> mux::Result<()> {
    match incoming.kind {
        LaneKind::Tcp => handle_tcp(incoming, config.mux.target_timeout, config.target).await,
        LaneKind::Udp => handle_udp(incoming, &config).await,
    }
}

async fn handle_udp(incoming: Incoming, config: &ServerConfig) -> mux::Result<()> {
    let target = incoming.address.clone();
    let socket = match timeout(
        config.mux.target_timeout,
        connect_udp(&target, config.target),
    )
    .await
    {
        Ok(Ok(socket)) => socket,
        Ok(Err(error)) if error.kind() == io::ErrorKind::PermissionDenied => {
            incoming.reject(http::StatusCode::FORBIDDEN);
            return Ok(());
        }
        Ok(Err(_)) => {
            incoming.reject(http::StatusCode::BAD_GATEWAY);
            return Ok(());
        }
        Err(_) => {
            incoming.reject(http::StatusCode::GATEWAY_TIMEOUT);
            return Ok(());
        }
    };
    let lane = incoming.accept().await?;
    bridge_udp(lane, socket, config.udp_idle).await?;
    Ok(())
}

async fn connect_udp(address: &Address, policy: TargetPolicy) -> io::Result<UdpSocket> {
    let targets = target::resolve_all(address, &policy).await?;
    let mut last = None;
    for target in targets {
        let bind = if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind).await?;
        match socket.connect(target).await {
            Ok(()) => return Ok(socket),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target has no addresses")))
}

async fn bridge_udp(
    lane: crate::mux::H2Lane,
    socket: UdpSocket,
    idle_timeout: Duration,
) -> io::Result<()> {
    let (mut read, mut write) = split(lane);
    let socket = Arc::new(socket);
    let activity = Arc::new(AtomicU64::new(0));
    let done = Arc::new(Notify::new());
    let epoch = Instant::now();

    let lane_socket = socket.clone();
    let lane_done = done.clone();
    let lane_activity = activity.clone();
    let lane_to_socket = tokio::spawn(async move {
        loop {
            match udp::read_frame(&mut read).await {
                Ok(Some(payload)) => {
                    if lane_socket.send(&payload).await.is_err() {
                        break;
                    }
                    touch(&lane_activity, epoch);
                }
                Ok(None) => touch(&lane_activity, epoch),
                Err(_) => break,
            }
        }
        lane_done.notify_one();
    });

    let socket_lane = socket.clone();
    let socket_done = done.clone();
    let socket_activity = activity.clone();
    let socket_to_lane = tokio::spawn(async move {
        let mut buffer = vec![0u8; UDP_BUFFER];
        while let Ok((length, source)) = socket_lane.recv_from(&mut buffer).await {
            if udp::write_response(&mut write, &Address::Ip(source), &buffer[..length])
                .await
                .is_err()
            {
                break;
            }
            touch(&socket_activity, epoch);
        }
        socket_done.notify_one();
    });

    let check = idle_timeout
        .min(Duration::from_secs(1))
        .max(Duration::from_millis(10));
    let idle = async {
        loop {
            sleep(check).await;
            if elapsed_ms(epoch).saturating_sub(activity.load(Ordering::Relaxed))
                >= millis(idle_timeout)
            {
                return;
            }
        }
    };

    tokio::select! {
        _ = done.notified() => {}
        _ = idle => {}
    }
    lane_to_socket.abort();
    socket_to_lane.abort();
    Ok(())
}

fn elapsed_ms(epoch: Instant) -> u64 {
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn touch(activity: &AtomicU64, epoch: Instant) {
    activity.store(elapsed_ms(epoch), Ordering::Relaxed);
}
