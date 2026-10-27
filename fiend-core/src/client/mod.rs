use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional, split};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval, timeout};

use crate::addr::Address;
use crate::mux::{self, Lane, LaneKind, LinkPool, PoolStats};
use crate::socks5::{self, Command, REP_CMD_NOT_SUPPORTED, REP_GENERAL_FAILURE, REP_SUCCESS};
use crate::transport;
use crate::udp;

#[derive(Clone)]
pub struct ClientConfig {
    pub server: String,
    pub secret: Vec<u8>,
    pub template: crate::fingerprint::Template,
    pub mux: mux::Config,
    pub udp_idle: Duration,
    pub udp_keepalive: Duration,
    pub max_udp_flows: usize,
    pub handshake_timeout: Duration,
    pub max_connections: usize,
    pub tcp_notsent_lowat: u32,
}

impl ClientConfig {
    pub fn new(
        server: impl Into<String>,
        secret: impl Into<Vec<u8>>,
        template: crate::fingerprint::Template,
    ) -> Self {
        Self {
            server: server.into(),
            secret: secret.into(),
            template,
            mux: mux::Config::default(),
            udp_idle: Duration::from_secs(60),
            udp_keepalive: Duration::from_secs(20),
            max_udp_flows: 64,
            handshake_timeout: Duration::from_secs(10),
            max_connections: 1024,
            tcp_notsent_lowat: 0,
        }
    }
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    pool: LinkPool<TcpStream>,
    config: ClientConfig,
    connections: Arc<Semaphore>,
}

impl Client {
    pub fn new(config: ClientConfig) -> Self {
        let server = config.server.clone();
        let secret = Arc::new(config.secret.clone());
        let template = Arc::new(config.template.clone());
        let tcp_notsent_lowat = config.tcp_notsent_lowat;
        let pool = LinkPool::new(config.mux.clone(), move || {
            let server = server.clone();
            let secret = secret.clone();
            let template = template.clone();
            Box::pin(async move {
                let stream = TcpStream::connect(server)
                    .await
                    .map_err(transport::Error::from)?;
                stream.set_nodelay(true).map_err(transport::Error::from)?;
                if let Err(error) = crate::net::set_tcp_notsent_lowat(&stream, tcp_notsent_lowat) {
                    tracing::warn!(%error, "cannot set TCP_NOTSENT_LOWAT");
                }
                transport::connect(stream, &secret, &template).await
            })
        });

        Self {
            inner: Arc::new(ClientInner {
                pool,
                connections: Arc::new(Semaphore::new(config.max_connections.max(1))),
                config,
            }),
        }
    }

    pub fn stats(&self) -> PoolStats {
        self.inner.pool.stats()
    }

    pub fn close(&self) {
        self.inner.pool.close();
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
            let (stream, peer) = listener.accept().await?;
            let client = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ = client.handle(stream, peer).await;
            });
        }
    }

    async fn handle(&self, mut stream: TcpStream, peer: SocketAddr) -> io::Result<()> {
        stream.set_nodelay(true)?;
        let handshake_timeout = self.inner.config.handshake_timeout;
        within(handshake_timeout, socks5::handshake(&mut stream)).await?;
        let request = within(handshake_timeout, socks5::read_request(&mut stream)).await?;

        match request.command {
            Command::Connect => self.connect(stream, request.address).await,
            Command::UdpAssociate => self.udp_associate(stream, peer).await,
            Command::Unsupported(_) => {
                socks5::send_reply(&mut stream, REP_CMD_NOT_SUPPORTED, &socks5::zero_address())
                    .await
            }
        }
    }

    async fn connect(&self, mut stream: TcpStream, address: Address) -> io::Result<()> {
        let authority = address.to_string();
        match self.inner.pool.open_lane(&authority, LaneKind::Tcp).await {
            Ok(mut lane) => {
                if let Err(error) =
                    socks5::send_reply(&mut stream, REP_SUCCESS, &socks5::zero_address()).await
                {
                    lane.reset();
                    return Err(error);
                }
                match copy_bidirectional(&mut stream, &mut lane).await {
                    Ok(_) => Ok(()),
                    Err(error) if is_disconnect(&error) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Err(_) => {
                let _ =
                    socks5::send_reply(&mut stream, REP_GENERAL_FAILURE, &socks5::zero_address())
                        .await;
                Ok(())
            }
        }
    }

    async fn udp_associate(&self, mut stream: TcpStream, peer: SocketAddr) -> io::Result<()> {
        let mut bind_ip = stream.local_addr()?.ip();
        if bind_ip.is_unspecified() {
            bind_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        }
        let socket = UdpSocket::bind(SocketAddr::new(bind_ip, 0)).await?;
        let relay = socket.local_addr()?;
        socks5::send_reply(&mut stream, REP_SUCCESS, &Address::Ip(relay)).await?;

        let association = UdpAssociation::new(self.inner.clone(), socket, peer.ip());
        tokio::select! {
            result = association.run() => result,
            _ = wait_control_close(&mut stream) => Ok(()),
        }
    }
}

struct UdpAssociation {
    inner: Arc<ClientInner>,
    socket: Arc<UdpSocket>,
    client_ip: IpAddr,
    client_addr: Arc<Mutex<Option<SocketAddr>>>,
    flows: HashMap<Address, Flow>,
    dialing: HashSet<Address>,
    events: mpsc::UnboundedSender<FlowEvent>,
    receiver: mpsc::UnboundedReceiver<FlowEvent>,
    next_id: u64,
    epoch: Instant,
    tick: Duration,
}

impl UdpAssociation {
    fn new(inner: Arc<ClientInner>, socket: UdpSocket, client_ip: IpAddr) -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        let tick = inner
            .config
            .udp_keepalive
            .min(Duration::from_secs(1))
            .max(Duration::from_millis(20));
        Self {
            inner,
            socket: Arc::new(socket),
            client_ip,
            client_addr: Arc::new(Mutex::new(None)),
            flows: HashMap::new(),
            dialing: HashSet::new(),
            events,
            receiver,
            next_id: 1,
            epoch: Instant::now(),
            tick,
        }
    }

    async fn run(mut self) -> io::Result<()> {
        let mut buffer = vec![0u8; u16::MAX as usize];
        let mut ticker = interval(self.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = self.socket.recv_from(&mut buffer) => {
                    let (length, from) = result?;
                    if from.ip() != self.client_ip {
                        continue;
                    }
                    *self.client_addr.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(from);
                    let Some((destination, payload)) = socks5::parse_udp_request(&buffer[..length]) else {
                        continue;
                    };
                    if payload.is_empty() {
                        continue;
                    }
                    self.forward(destination, payload);
                }
                Some(event) = self.receiver.recv() => self.apply(event),
                _ = ticker.tick() => self.sweep(),
            }
        }
    }

    fn forward(&mut self, destination: Address, payload: &[u8]) {
        let Ok(frame) = udp::encode_frame(payload) else {
            return;
        };

        let mut frame = Some(frame);
        if let Some(flow) = self.flows.get_mut(&destination) {
            if !flow.closed.load(Ordering::Acquire) && !flow.sender.is_closed() {
                touch(&flow.activity, self.epoch);
                if let Some(pending) = frame.take() {
                    match flow.sender.try_send(pending) {
                        Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => return,
                        Err(mpsc::error::TrySendError::Closed(_)) => {}
                    }
                }
            }
            self.flows.remove(&destination);
        }
        let Some(frame) = frame else {
            return;
        };

        if self.dialing.contains(&destination)
            || self.flows.len() + self.dialing.len() >= self.inner.config.max_udp_flows
        {
            return;
        }

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.dialing.insert(destination.clone());

        let inner = self.inner.clone();
        let socket = self.socket.clone();
        let client_addr = self.client_addr.clone();
        let events = self.events.clone();
        let epoch = self.epoch;
        let activity = Arc::new(AtomicU64::new(elapsed_ms(epoch)));
        tokio::spawn(async move {
            open_flow(
                inner,
                socket,
                destination,
                id,
                client_addr,
                events,
                epoch,
                activity,
                frame,
            )
            .await;
        });
    }

    fn apply(&mut self, event: FlowEvent) {
        match event {
            FlowEvent::Ready { address, flow } => {
                self.dialing.remove(&address);
                if !flow.closed.load(Ordering::Acquire) && !self.flows.contains_key(&address) {
                    self.flows.insert(address, flow);
                }
            }
            FlowEvent::Failed { address } => {
                self.dialing.remove(&address);
            }
            FlowEvent::Closed { address, id } => {
                if self.flows.get(&address).map(|flow| flow.id) == Some(id) {
                    self.flows.remove(&address);
                }
            }
        }
    }

    fn sweep(&mut self) {
        let now = elapsed_ms(self.epoch);
        let idle_limit = millis(self.inner.config.udp_idle);
        let keepalive_limit = millis(self.inner.config.udp_keepalive);
        let mut stale = Vec::new();

        for (address, flow) in &self.flows {
            let idle = now.saturating_sub(flow.activity.load(Ordering::Relaxed));
            if idle >= idle_limit {
                stale.push(address.clone());
            } else if idle >= keepalive_limit {
                let _ = flow.sender.try_send(vec![0, 0]);
            }
        }
        for address in stale {
            self.flows.remove(&address);
        }
    }
}

struct Flow {
    id: u64,
    sender: mpsc::Sender<Vec<u8>>,
    activity: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl Drop for Flow {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}

enum FlowEvent {
    Ready { address: Address, flow: Flow },
    Failed { address: Address },
    Closed { address: Address, id: u64 },
}

#[allow(clippy::too_many_arguments)]
async fn open_flow(
    inner: Arc<ClientInner>,
    socket: Arc<UdpSocket>,
    destination: Address,
    id: u64,
    client_addr: Arc<Mutex<Option<SocketAddr>>>,
    events: mpsc::UnboundedSender<FlowEvent>,
    epoch: Instant,
    activity: Arc<AtomicU64>,
    initial: Vec<u8>,
) {
    let Ok(lane) = inner
        .pool
        .open_lane(&destination.to_string(), LaneKind::Udp)
        .await
    else {
        let _ = events.send(FlowEvent::Failed {
            address: destination,
        });
        return;
    };
    let (mut read, mut write) = split(lane);
    let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(16);
    let _ = sender.try_send(initial);
    let closed = Arc::new(AtomicBool::new(false));

    let writer_closed = closed.clone();
    let writer_events = events.clone();
    let writer_destination = destination.clone();
    let writer_activity = activity.clone();
    let writer = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if write.write_all(&frame).await.is_err() {
                break;
            }
            touch(&writer_activity, epoch);
        }
        let _ = write.shutdown().await;
        writer_closed.store(true, Ordering::Release);
        let _ = writer_events.send(FlowEvent::Closed {
            address: writer_destination,
            id,
        });
    });

    let reader_closed = closed.clone();
    let reader_events = events.clone();
    let reader_destination = destination.clone();
    let reader_activity = activity.clone();
    let reader = tokio::spawn(async move {
        loop {
            match udp::read_response(&mut read).await {
                Ok(Some(response)) => {
                    let source = response
                        .source
                        .unwrap_or_else(|| reader_destination.clone());
                    let Ok(datagram) = socks5::encode_udp_response(&source, &response.payload)
                    else {
                        break;
                    };
                    let target = *client_addr
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(target) = target
                        && socket.send_to(&datagram, target).await.is_err()
                    {
                        break;
                    }
                    touch(&reader_activity, epoch);
                }
                Ok(None) => touch(&reader_activity, epoch),
                Err(_) => break,
            }
        }
        reader_closed.store(true, Ordering::Release);
        let _ = reader_events.send(FlowEvent::Closed {
            address: reader_destination,
            id,
        });
    });

    let _ = events.send(FlowEvent::Ready {
        address: destination,
        flow: Flow {
            id,
            sender,
            activity,
            closed,
            reader,
            writer,
        },
    });
}

async fn wait_control_close(stream: &mut TcpStream) -> io::Result<()> {
    let mut buffer = [0u8; 1024];
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) => return Ok(()),
            Ok(_) => continue,
            Err(error) => return Err(error),
        }
    }
}

async fn within<T, F>(duration: Duration, future: F) -> io::Result<T>
where
    F: Future<Output = io::Result<T>>,
{
    timeout(duration, future)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SOCKS5 handshake timed out"))?
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

fn is_disconnect(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
    )
}
