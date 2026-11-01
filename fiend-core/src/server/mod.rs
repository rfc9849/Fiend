use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, split};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::mux::{self, Incoming, LaneKind, ServerConnection};
use crate::target::TargetPolicy;
use crate::transport::{self, FreshnessCheck, OriginProfile, Session};
use crate::udp;

#[derive(Clone)]
pub struct ServerConfig {
    pub origin_host: String,
    pub origin_port: u16,
    pub origin: OriginProfile,
    pub replay_ttl: Duration,
    pub mux: mux::Config,
    pub udp_idle: Duration,
    pub max_connections: usize,
    pub max_user_sessions: usize,
    pub max_user_lanes: usize,
    pub tcp_idle: Duration,
    pub target: TargetPolicy,
    pub tcp_notsent_lowat: u32,
}

impl ServerConfig {
    pub fn new(origin_host: impl Into<String>, origin_port: u16) -> Self {
        Self {
            origin_host: origin_host.into(),
            origin_port,
            origin: OriginProfile::default(),
            replay_ttl: transport::AUTH_WINDOW + Duration::from_secs(300),
            mux: mux::Config::default(),
            udp_idle: Duration::from_secs(60),
            max_connections: 1024,
            max_user_sessions: 64,
            max_user_lanes: 1024,
            tcp_idle: Duration::from_secs(300),
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
    node: crate::node::Node,
}

impl Server {
    pub fn managed(config: ServerConfig, node: crate::node::Node) -> Self {
        let replay = FreshnessCheck::new(config.replay_ttl.max(transport::AUTH_WINDOW));
        let connections = Arc::new(Semaphore::new(config.max_connections.max(1)));
        Self {
            inner: Arc::new(ServerInner {
                config,
                replay,
                connections,
                node,
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
            let (stream, peer) = crate::net::accept(&listener).await;
            if let Err(error) = crate::net::set_link_buffers(&stream) {
                tracing::debug!(%error, "cannot set link buffers");
            }
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
                if let Err(error) = server.handle(stream, peer).await {
                    tracing::debug!(%error, "connection handler failed");
                }
            });
        }
    }

    fn start_origin_probe(&self) {
        let node = self.inner.node.clone();
        tokio::spawn(async move {
            node.refresh_origin().await;
        });
    }

    async fn handle(&self, stream: TcpStream, peer: std::net::SocketAddr) -> mux::Result<()> {
        use crate::node::Principal;
        use crate::transport::AcceptOutcome;
        let node = self.inner.node.clone();
        let mut config = self.inner.config.clone();
        (config.origin_host, config.origin_port, config.origin) = node.origin();
        let mut principal = None;
        let outcome =
            transport::accept_with(stream, &self.inner.replay, &config.origin, |body, time| {
                let (secret, identity) = node.authenticate(body, time)?;
                principal = Some(identity);
                Some(secret)
            })
            .await?;
        match outcome {
            AcceptOutcome::Passthrough {
                stream,
                mut buffered,
                reason,
            } => {
                if reason == transport::PassthroughReason::NotTls && !buffered.is_empty() {
                    read_more(&stream, &mut buffered).await;
                }
                node.sent_to_origin(crate::node::probes::Probe::new(peer, reason, &buffered));
                transport::bridge_passthrough(
                    stream,
                    buffered,
                    &config.origin_host,
                    config.origin_port,
                )
                .await?;
            }
            AcceptOutcome::Session(session) => {
                node.accepted();
                match principal {
                    Some(Principal::Control) => crate::node::control::serve(*session, node).await?,
                    Some(Principal::User(entry)) => {
                        let Some(mut guard) =
                            node.begin_session_within(entry.clone(), config.max_user_sessions)
                        else {
                            return Ok(());
                        };
                        tokio::select! {
                            result = serve_mux(*session, config, entry, node.clone()) => result?,
                            _ = guard.revoked() => {},
                        }
                    }
                    None => return Err(io::Error::other("missing authenticated identity").into()),
                }
            }
        }
        Ok(())
    }
}

async fn read_more(stream: &TcpStream, buffered: &mut Vec<u8>) {
    let wait = tokio::time::Instant::now() + Duration::from_millis(250);
    loop {
        if buffered.len() >= crate::node::probes::MAX_HEAD
            || tokio::time::timeout_at(wait, stream.readable())
                .await
                .is_err()
        {
            return;
        }
        let start = buffered.len();
        buffered.resize(crate::node::probes::MAX_HEAD, 0);
        match stream.try_read(&mut buffered[start..]) {
            Ok(count) => {
                buffered.truncate(start + count);
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => buffered.truncate(start),
            Err(_) => {
                buffered.truncate(start);
                return;
            }
        }
    }
}

async fn serve_mux(
    session: Session<TcpStream>,
    config: ServerConfig,
    user: Arc<crate::node::UserEntry>,
    node: crate::node::Node,
) -> mux::Result<()> {
    let mut connection = ServerConnection::handshake(session, &config.mux).await?;
    let mut lanes = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            incoming = connection.accept() => {
                let Some(incoming) = incoming? else { return Ok(()); };
                if !user.active(crate::node::unix_time()) { return Ok(()); }
                let Some(lane) = user.begin_lane(config.max_user_lanes) else {
                    tracing::debug!(user = %user.id(), "user lane limit reached");
                    incoming.reject(http::StatusCode::SERVICE_UNAVAILABLE);
                    continue;
                };
                let meter = user.clone();
                let incoming = incoming.with_meter(Arc::new(move |count, inbound| meter.note_bytes(count, inbound)));
                let config = config.clone();
                let node = node.clone();
                lanes.spawn(async move {
                    let _lane = lane;
                    if let Err(error) = handle_routed(incoming, config, node).await {
                        tracing::debug!(%error, "lane handler failed");
                    }
                });
            }
            _ = lanes.join_next(), if !lanes.is_empty() => {},
        }
    }
}

fn reject_for(incoming: Incoming, error: &io::Error) {
    let status = match error.kind() {
        io::ErrorKind::PermissionDenied => http::StatusCode::FORBIDDEN,
        io::ErrorKind::TimedOut => http::StatusCode::GATEWAY_TIMEOUT,
        _ => http::StatusCode::BAD_GATEWAY,
    };
    incoming.reject(status);
}

async fn handle_routed(
    incoming: Incoming,
    config: ServerConfig,
    node: crate::node::Node,
) -> mux::Result<()> {
    use std::sync::atomic::Ordering;
    let router = node.router();
    let context = node.outbound_context(config.target, config.mux.target_timeout);
    let target = incoming.address.clone();
    match incoming.kind {
        LaneKind::Tcp => {
            let runtime = router.route(&target);
            tracing::debug!(outbound = %runtime.config.id, authority = %target, "routed TCP lane");
            let upstream = match runtime.connect_tcp(&target, &context).await {
                Ok(upstream) => {
                    crate::node::outbound::Metered::new(upstream, runtime.counters.clone())
                }
                Err(error) => {
                    tracing::debug!(outbound = %runtime.config.id, %error, "outbound connect failed");
                    reject_for(incoming, &error);
                    return Ok(());
                }
            };
            let mut lane = incoming.accept().await?;
            let activity = Arc::new(AtomicU64::new(0));
            let epoch = Instant::now();
            let mut upstream = Active {
                inner: upstream,
                activity: activity.clone(),
                epoch,
            };
            tokio::select! {
                _ = tokio::io::copy_bidirectional(&mut lane, &mut upstream) => {}
                _ = idle(&activity, epoch, config.tcp_idle) => {
                    tracing::debug!(authority = %target, "TCP lane idle timeout");
                }
            }
            Ok(())
        }
        LaneKind::Udp => {
            let Some(runtime) = router.udp_route(&target) else {
                incoming.reject(http::StatusCode::FORBIDDEN);
                return Ok(());
            };
            let path = match runtime.open_udp(&target, &context).await {
                Ok(path) => path,
                Err(error) => {
                    reject_for(incoming, &error);
                    return Ok(());
                }
            };
            let lane = incoming.accept().await?;
            let (up, down) = bridge_udp(lane, path, config.udp_idle).await?;
            runtime.counters.up.fetch_add(up, Ordering::Relaxed);
            runtime.counters.down.fetch_add(down, Ordering::Relaxed);
            Ok(())
        }
    }
}

async fn bridge_udp(
    lane: crate::mux::H2Lane,
    path: crate::node::outbound::UdpPath,
    idle_timeout: Duration,
) -> io::Result<(u64, u64)> {
    let crate::node::outbound::UdpPath {
        mut tx,
        mut rx,
        guard,
    } = path;
    let _guard = guard;
    let (mut read, mut write) = split(lane);
    let activity = Arc::new(AtomicU64::new(0));
    let sent = AtomicU64::new(0);
    let received = AtomicU64::new(0);
    let epoch = Instant::now();

    let lane_activity = activity.clone();
    let lane_to_socket = async {
        loop {
            match udp::read_frame(&mut read).await {
                Ok(Some(payload)) => {
                    if tx.send(&payload).await.is_err() {
                        break;
                    }
                    sent.fetch_add(payload.len() as u64, Ordering::Relaxed);
                    touch(&lane_activity, epoch);
                }
                Ok(None) => touch(&lane_activity, epoch),
                Err(_) => break,
            }
        }
    };

    let socket_activity = activity.clone();
    let socket_to_lane = async {
        while let Ok(Some((source, payload))) = rx.recv().await {
            if udp::write_response(&mut write, &source, &payload)
                .await
                .is_err()
            {
                break;
            }
            received.fetch_add(payload.len() as u64, Ordering::Relaxed);
            touch(&socket_activity, epoch);
        }
    };

    let idle = idle(&activity, epoch, idle_timeout);

    tokio::select! {
        _ = lane_to_socket => {}
        _ = socket_to_lane => {}
        _ = idle => {}
    }
    Ok((
        sent.load(Ordering::Relaxed),
        received.load(Ordering::Relaxed),
    ))
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

async fn idle(activity: &AtomicU64, epoch: Instant, timeout: Duration) {
    let check = timeout
        .min(Duration::from_secs(1))
        .max(Duration::from_millis(10));
    loop {
        sleep(check).await;
        if elapsed_ms(epoch).saturating_sub(activity.load(Ordering::Relaxed)) >= millis(timeout) {
            return;
        }
    }
}

struct Active<S> {
    inner: S,
    activity: Arc<AtomicU64>,
    epoch: Instant,
}

impl<S: AsyncRead + Unpin> AsyncRead for Active<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            touch(&self.activity, self.epoch);
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Active<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, data);
        if matches!(result, Poll::Ready(Ok(count)) if count > 0) {
            touch(&self.activity, self.epoch);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
