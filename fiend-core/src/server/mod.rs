use std::io;
use std::sync::Arc;
use std::sync::RwLock;
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
    origin: RwLock<OriginProfile>,
}

impl ServerInner {
    fn origin(&self) -> OriginProfile {
        self.origin
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Server {
    pub fn new(config: ServerConfig) -> Self {
        let replay = FreshnessCheck::new(config.replay_ttl.max(transport::AUTH_WINDOW));
        let connections = Arc::new(Semaphore::new(config.max_connections.max(1)));
        let origin = RwLock::new(config.origin.clone());
        Self {
            inner: Arc::new(ServerInner {
                config,
                replay,
                connections,
                origin,
            }),
        }
    }

    pub async fn serve(&self, listener: TcpListener) -> io::Result<()> {
        self.start_origin_probe();
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

    fn start_origin_probe(&self) {
        if !self.inner.config.origin.record_sizes.is_empty() {
            return;
        }
        let server = self.clone();
        tokio::spawn(async move {
            server.probe_origin().await;
        });
    }

    async fn probe_origin(&self) {
        if !self.inner.config.origin.record_sizes.is_empty() {
            return;
        }
        let host = &self.inner.config.origin_host;
        let hello = transport::build_probe_hello(host);
        match transport::probe_origin_at(host, self.inner.config.origin_port, &hello).await {
            Ok(profile) => {
                let mut origin = self
                    .inner
                    .origin
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if origin.cipher_suite == 0 {
                    origin.cipher_suite = profile.cipher_suite;
                }
                if origin.key_share_group == 0 {
                    origin.key_share_group = profile.key_share_group;
                }
                if origin.extension_order.is_empty() {
                    origin.extension_order = profile.extension_order;
                }
                if origin.record_sizes.is_empty() && profile.record_sizes.len() >= 2 {
                    origin.record_sizes = profile.record_sizes;
                }
            }
            Err(error) => tracing::debug!(%error, "origin probe failed"),
        }
    }

    async fn handle(&self, stream: TcpStream) -> mux::Result<()> {
        let config = &self.inner.config;
        let origin = self.inner.origin();
        let outcome = transport::serve_connection(
            stream,
            &config.secret,
            &self.inner.replay,
            &origin,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn origin_probe_records_the_server_flight_shape() {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind origin");
        let port = listener.local_addr().expect("origin address").port();
        let origin = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept probe");
            let mut header = [0u8; 5];
            stream.read_exact(&mut header).await.expect("probe header");
            let length = u16::from_be_bytes([header[3], header[4]]) as usize;
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).await.expect("probe body");

            let mut reply = transport::build_server_hello(
                [9u8; 32],
                &[3u8; 32],
                0x1301,
                0x001d,
                &[7u8; 32],
                &[0x002b, 0x0033],
            )
            .expect("build ServerHello");
            reply.extend_from_slice(&[0x14, 0x03, 0x03, 0, 1, 1]);
            for size in [111usize, 222, 333] {
                reply.extend_from_slice(&[0x17, 0x03, 0x03]);
                reply.extend_from_slice(&(size as u16).to_be_bytes());
                reply.resize(reply.len() + size, 0x5a);
            }
            stream.write_all(&reply).await.expect("origin write");
            stream.shutdown().await.expect("origin shutdown");
        });

        let config = ServerConfig::new(vec![0x42u8; 32], "127.0.0.1", port);
        let server = Server::new(config);
        server.probe_origin().await;

        let profile = server.inner.origin();
        assert_eq!(profile.cipher_suite, 0x1301);
        assert_eq!(profile.key_share_group, 0x001d);
        assert_eq!(profile.extension_order, vec![0x002b, 0x0033]);
        assert_eq!(profile.record_sizes, vec![116, 227, 338]);
        origin.await.expect("origin task");
    }

    #[tokio::test]
    async fn explicit_record_sizes_skip_the_origin_probe() {
        let config = ServerConfig::new(vec![0x42u8; 32], "192.0.2.1", 443);
        let mut config = config;
        config.origin.record_sizes = vec![35, 4000, 300];
        let server = Server::new(config);
        server.probe_origin().await;
        assert_eq!(server.inner.origin().record_sizes, vec![35, 4000, 300]);
    }
}
