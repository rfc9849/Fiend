//! Front end for sockets that the kernel fills with TPROXY: the local address
//! of each accepted TCP stream and the original destination of each UDP
//! datagram are the addresses that the app asked for.
//!
//! DNS of the captured apps also comes here. Answers are real: a query for a
//! site with a direct rule goes to the resolver that the app asked, other
//! queries go through the tunnel. The answers fill a table from address to
//! name, so that domain rules also work for UDP and for TCP without a name
//! in the first data.

use std::collections::HashMap;
use std::io::{self, IoSliceMut};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::num::NonZeroUsize;
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context as Task, Poll};
use std::time::Duration;

use fiend_core::addr::Address;
use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::RData;
use lru::LruCache;
use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, SockaddrIn, SockaddrIn6, SockaddrLike, SockaddrStorage, recvmsg,
};
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Interest, ReadBuf, copy_bidirectional,
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::desync;
use crate::dns::valid_domain;
use crate::engine::{
    Context, FlowCount, Plan, Running, TunEngine, UdpReply, protected_udp, udp_flow,
};
use crate::gateway::{self, GatewayDns};
use crate::packet::FlowKey;
use crate::rules::{Host, Route, is_blocked_udp_port};
use crate::sniff;

const SNIFF_PORTS: [u16; 4] = [80, 443, 8080, 8443];
const DIRECT_DIAL: Duration = Duration::from_secs(10);
const MAX_DATAGRAM: usize = 65535;
const DNS_PORT: u16 = 53;
const TUNNEL_RESOLVER: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), DNS_PORT);
const DNS_WAIT: Duration = Duration::from_secs(5);
pub(crate) const DNS_TCP_IDLE: Duration = Duration::from_secs(10);
pub(crate) const MAX_DNS_MESSAGE: usize = 4096;
pub(crate) const MAX_DNS_QUERIES: usize = 256;
const NAMES: usize = 8192;

/// Opens UDP sockets bound to a foreign address, so that replies come from
/// the address that the app sent to. The kernel needs privileges for this.
pub trait ReplySockets: Send + Sync + 'static {
    fn open(&self, local: SocketAddr) -> io::Result<std::net::UdpSocket>;
}

pub struct TransparentSockets {
    pub tcp: Vec<std::net::TcpListener>,
    pub udp: Vec<std::net::UdpSocket>,
    pub replies: Arc<dyn ReplySockets>,
    pub dns: Option<GatewayDns>,
}

enum Event {
    Tcp(TcpStream, SocketAddr),
    Udp {
        payload: Vec<u8>,
        source: SocketAddr,
        destination: SocketAddr,
    },
}

enum Done {
    Infrastructure,
    Tcp,
    Udp(FlowKey, u64),
    Dns,
}

/// The site names of the addresses in recent DNS answers.
pub(crate) struct Names(std::sync::Mutex<LruCache<IpAddr, String>>);

impl Names {
    fn new() -> Self {
        Self(std::sync::Mutex::new(LruCache::new(
            NonZeroUsize::new(NAMES).unwrap_or(NonZeroUsize::MIN),
        )))
    }

    fn get(&self, ip: IpAddr) -> Option<String> {
        self.lock().get(&ip).cloned()
    }

    pub(crate) fn remember(&self, name: &str, answer: &[u8]) {
        let Ok(message) = Message::from_vec(answer) else {
            return;
        };
        let mut names = self.lock();
        for record in &message.answers {
            let ip = match &record.data {
                RData::A(address) => IpAddr::V4(address.0),
                RData::AAAA(address) => IpAddr::V6(address.0),
                _ => continue,
            };
            names.put(ip, name.to_owned());
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LruCache<IpAddr, String>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl TunEngine {
    pub async fn run_transparent(&self, sockets: TransparentSockets) -> io::Result<()> {
        if self.running.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "TUN engine is already running",
            ));
        }
        let _running = Running(&self.running);
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Ok(()),
            result = serve(self.context.clone(), sockets) => result,
        }
    }
}

async fn serve(context: Arc<Context>, sockets: TransparentSockets) -> io::Result<()> {
    if sockets.tcp.is_empty() && sockets.udp.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no transparent sockets",
        ));
    }
    let (events, mut incoming) = mpsc::channel::<Event>(256);
    let mut tasks = JoinSet::new();
    for listener in sockets.tcp {
        listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(listener)?;
        let events = events.clone();
        tasks.spawn(async move { (Done::Infrastructure, accept_loop(listener, events).await) });
    }
    for socket in sockets.udp {
        socket.set_nonblocking(true)?;
        let socket = UdpSocket::from_std(socket)?;
        let events = events.clone();
        tasks.spawn(async move { (Done::Infrastructure, receive_loop(socket, events).await) });
    }
    drop(events);
    let replies = Arc::new(Replies {
        opener: sockets.replies,
        open: std::sync::Mutex::new(HashMap::new()),
        limit: context.options.max_udp_flows,
    });
    let names = Arc::new(Names::new());
    if let Some(gateway) = sockets.dns {
        let server = gateway::Server::new(context.clone(), names.clone(), &gateway);
        for socket in gateway.udp {
            socket.set_nonblocking(true)?;
            let socket = UdpSocket::from_std(socket)?;
            let server = server.clone();
            tasks.spawn(async move { (Done::Infrastructure, server.serve_udp(socket).await) });
        }
        for listener in gateway.tcp {
            listener.set_nonblocking(true)?;
            let listener = TcpListener::from_std(listener)?;
            let server = server.clone();
            tasks.spawn(async move { (Done::Infrastructure, server.serve_tcp(listener).await) });
        }
    }
    let mut tcp = 0usize;
    let mut dns = 0usize;
    let mut udp: HashMap<FlowKey, (u64, mpsc::Sender<Vec<u8>>)> = HashMap::new();
    let mut next_id = 0u64;
    loop {
        tokio::select! {
            event = incoming.recv() => {
                let Some(event) = event else {
                    return Err(io::Error::other("transparent sockets are closed"));
                };
                match event {
                    Event::Tcp(stream, original) => {
                        if tcp >= context.options.max_tcp_flows {
                            context.drop_packet();
                            reset(&stream);
                            continue;
                        }
                        tcp += 1;
                        let flow_context = context.clone();
                        let flow_names = names.clone();
                        let count = FlowCount::new(context.clone());
                        tasks.spawn(async move {
                            let _count = count;
                            (Done::Tcp, tcp_flow(flow_context, flow_names, stream, original).await)
                        });
                    }
                    Event::Udp { payload, source, destination } => {
                        if payload.is_empty() || is_blocked_udp_port(destination.port()) {
                            context.drop_packet();
                            continue;
                        }
                        context.counters.uploaded.fetch_add(payload.len() as u64, Ordering::Relaxed);
                        if destination.port() == DNS_PORT {
                            if dns >= MAX_DNS_QUERIES {
                                context.drop_packet();
                                continue;
                            }
                            dns += 1;
                            let reply = TransparentReply {
                                context: context.clone(),
                                replies: replies.clone(),
                                socket: OnceCell::new(),
                                local: destination,
                                client: source,
                            };
                            let query_context = context.clone();
                            let query_names = names.clone();
                            tasks.spawn(async move {
                                let result = async {
                                    let answer = resolve(&query_context, &query_names, destination, &payload).await?;
                                    reply.send(&answer).await
                                }.await;
                                (Done::Dns, result)
                            });
                            continue;
                        }
                        let key = FlowKey { source, destination };
                        if let Some((_, sender)) = udp.get(&key) && !sender.is_closed() {
                            if sender.try_send(payload).is_err() { context.drop_packet(); }
                            continue;
                        }
                        udp.remove(&key);
                        if udp.len() >= context.options.max_udp_flows {
                            context.drop_packet();
                            continue;
                        }
                        let (sender, receiver) = mpsc::channel(16);
                        let _ = sender.try_send(payload);
                        next_id = next_id.wrapping_add(1);
                        let id = next_id;
                        udp.insert(key, (id, sender));
                        let reply = TransparentReply {
                            context: context.clone(),
                            replies: replies.clone(),
                            socket: OnceCell::new(),
                            local: destination,
                            client: source,
                        };
                        let plan = plan(&context, names.get(destination.ip()).as_deref(), destination);
                        let flow_context = context.clone();
                        let count = FlowCount::new(context.clone());
                        tasks.spawn(async move {
                            let _count = count;
                            let result = udp_flow(flow_context, destination.port(), Address::Ip(destination), plan, receiver, reply).await;
                            (Done::Udp(key, id), result)
                        });
                    }
                }
            }
            joined = tasks.join_next() => {
                let Some(joined) = joined else {
                    return Err(io::Error::other("transparent tasks are closed"));
                };
                let (done, result) = joined.map_err(io::Error::other)?;
                match done {
                    Done::Infrastructure => {
                        return result.and(Err(io::Error::other("transparent socket stopped")));
                    }
                    Done::Tcp => tcp -= 1,
                    Done::Dns => dns -= 1,
                    Done::Udp(key, id) => {
                        if udp.get(&key).is_some_and(|(current, _)| *current == id) {
                            udp.remove(&key);
                        }
                    }
                }
                if let Err(error) = result {
                    tracing::debug!(%error, "transparent flow closed");
                }
            }
        }
    }
}

async fn accept_loop(listener: TcpListener, events: mpsc::Sender<Event>) -> io::Result<()> {
    loop {
        let (stream, _) = fiend_core::net::accept(&listener).await;
        let original = match stream.local_addr() {
            Ok(address) => canonical(address),
            Err(error) => {
                tracing::debug!(%error, "accepted stream has no address");
                continue;
            }
        };
        events
            .send(Event::Tcp(stream, original))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "engine is closed"))?;
    }
}

async fn receive_loop(socket: UdpSocket, events: mpsc::Sender<Event>) -> io::Result<()> {
    let mut buffer = vec![0; MAX_DATAGRAM];
    loop {
        let (size, source, destination) = socket
            .async_io(Interest::READABLE, || receive(&socket, &mut buffer))
            .await?;
        let (Some(source), Some(destination)) = (source, destination) else {
            tracing::debug!("datagram has no original destination");
            continue;
        };
        events
            .send(Event::Udp {
                payload: buffer[..size].to_vec(),
                source: canonical(source),
                destination: canonical(destination),
            })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "engine is closed"))?;
    }
}

fn receive(
    socket: &UdpSocket,
    buffer: &mut [u8],
) -> io::Result<(usize, Option<SocketAddr>, Option<SocketAddr>)> {
    let mut iov = [IoSliceMut::new(buffer)];
    let mut space = nix::cmsg_space!(nix::libc::sockaddr_in6);
    let message = recvmsg::<SockaddrStorage>(
        socket.as_raw_fd(),
        &mut iov,
        Some(&mut space),
        MsgFlags::empty(),
    )?;
    let source = message.address.as_ref().and_then(socket_address);
    let mut destination = None;
    for control in message.cmsgs()? {
        match control {
            ControlMessageOwned::Ipv4OrigDstAddr(address) => {
                destination = Some(SocketAddr::V4(SocketAddrV4::from(SockaddrIn::from(
                    address,
                ))));
            }
            ControlMessageOwned::Ipv6OrigDstAddr(address) => {
                destination = Some(SocketAddr::V6(SocketAddrV6::from(SockaddrIn6::from(
                    address,
                ))));
            }
            _ => {}
        }
    }
    Ok((message.bytes, source, destination))
}

fn socket_address(address: &SockaddrStorage) -> Option<SocketAddr> {
    if let Some(v4) = address.as_sockaddr_in() {
        return Some(SocketAddr::V4(SocketAddrV4::from(*v4)));
    }
    let v6 = address.as_sockaddr_in6()?;
    (v6.family() == Some(nix::sys::socket::AddressFamily::Inet6))
        .then(|| SocketAddr::V6(SocketAddrV6::from(*v6)))
}

/// Turns an IPv4-mapped address into IPv4. Other IPv6 addresses keep their
/// scope, which a link-local resolver needs.
fn canonical(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(v4.into(), v6.port()),
            None => address,
        },
        SocketAddr::V4(_) => address,
    }
}

fn reset(stream: &TcpStream) {
    let _ = socket2::SockRef::from(stream).set_linger(Some(Duration::ZERO));
}

fn route(context: &Context, name: Option<&str>, original: SocketAddr) -> Route {
    let host = match name {
        Some(name) => Host::Name(name),
        None => Host::Ip(original.ip()),
    };
    context.rules.route(host, context.options.bypass_lan)
}

/// The plan for a UDP flow: the route comes from the site name when it is
/// known, and a direct flow goes to the address that the app chose.
fn plan(context: &Context, name: Option<&str>, original: SocketAddr) -> Plan {
    match route(context, name, original) {
        Route::Block => Plan::Block,
        Route::Proxy => Plan::Proxy,
        Route::Direct => Plan::Direct {
            addresses: vec![original],
            dpi: false,
        },
        Route::Dpi => Plan::Direct {
            addresses: vec![original],
            dpi: true,
        },
    }
}

async fn tcp_flow(
    context: Arc<Context>,
    names: Arc<Names>,
    stream: TcpStream,
    original: SocketAddr,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    if original.port() == DNS_PORT {
        return dns_stream(&context, &names, stream, original).await;
    }
    let mut stream = Counted {
        inner: stream,
        context: context.clone(),
    };
    let first = if SNIFF_PORTS.contains(&original.port()) {
        desync::read_first(&mut stream, desync::FIRST_DATA_WAIT).await?
    } else {
        Vec::new()
    };
    let sniffed = sniff::host(&first);
    let name = sniffed.clone().or_else(|| names.get(original.ip()));
    let route = route(&context, name.as_deref(), original);
    let destination = match sniffed {
        Some(host) => Address::domain(host, original.port()),
        None => Address::Ip(original),
    };
    tracing::debug!(%destination, ?name, ?route, "transparent TCP flow");
    match route {
        Route::Block => {
            reset(&stream.inner);
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "destination is blocked",
            ))
        }
        Route::Direct | Route::Dpi => {
            let mut direct = timeout(
                DIRECT_DIAL,
                fiend_core::net::dial(&original.to_string(), &context.options.protect_socket),
            )
            .await
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "direct connect timed out",
                ))
            })
            .inspect_err(|_| reset(&stream.inner))?;
            if !first.is_empty() {
                if matches!(route, Route::Dpi) {
                    direct.set_nodelay(true)?;
                    desync::write_first(&mut direct, &first).await?;
                } else {
                    direct.write_all(&first).await?;
                }
            }
            copy_bidirectional(&mut stream, &mut direct).await?;
            Ok(())
        }
        Route::Proxy => {
            let mut lane = context
                .client
                .open_tcp(&destination)
                .await
                .map_err(io::Error::other)
                .inspect_err(|_| reset(&stream.inner))?;
            if !first.is_empty() {
                lane.write_all(&first)
                    .await
                    .inspect_err(|_| reset(&stream.inner))?;
            }
            copy_bidirectional(&mut stream, &mut lane)
                .await
                .inspect_err(|_| reset(&stream.inner))?;
            Ok(())
        }
    }
}

/// Counts the bytes of an app stream: reads are uploads, writes are
/// downloads.
struct Counted {
    inner: TcpStream,
    context: Arc<Context>,
}

impl AsyncRead for Counted {
    fn poll_read(
        mut self: Pin<&mut Self>,
        task: &mut Task<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(task, buffer);
        let read = buffer.filled().len() - before;
        self.context
            .counters
            .uploaded
            .fetch_add(read as u64, Ordering::Relaxed);
        result
    }
}

impl AsyncWrite for Counted {
    fn poll_write(
        mut self: Pin<&mut Self>,
        task: &mut Task<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(task, data);
        if let Poll::Ready(Ok(size)) = result {
            self.context
                .counters
                .downloaded
                .fetch_add(size as u64, Ordering::Relaxed);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, task: &mut Task<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(task)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, task: &mut Task<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(task)
    }
}

struct Replies {
    opener: Arc<dyn ReplySockets>,
    open: std::sync::Mutex<HashMap<SocketAddr, Arc<UdpSocket>>>,
    limit: usize,
}

impl Replies {
    async fn get(self: &Arc<Self>, local: SocketAddr) -> io::Result<Arc<UdpSocket>> {
        if let Some(socket) = self.lock().get(&local) {
            return Ok(socket.clone());
        }
        let replies = self.clone();
        let socket = tokio::task::spawn_blocking(move || replies.opener.open(local))
            .await
            .map_err(io::Error::other)??;
        socket.set_nonblocking(true)?;
        let socket = Arc::new(UdpSocket::from_std(socket)?);
        let mut open = self.lock();
        if open.len() >= self.limit {
            open.retain(|_, socket| Arc::strong_count(socket) > 1);
        }
        Ok(open.entry(local).or_insert(socket).clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SocketAddr, Arc<UdpSocket>>> {
        self.open.lock().unwrap_or_else(|error| error.into_inner())
    }
}

struct TransparentReply {
    context: Arc<Context>,
    replies: Arc<Replies>,
    socket: OnceCell<Arc<UdpSocket>>,
    local: SocketAddr,
    client: SocketAddr,
}

impl UdpReply for TransparentReply {
    async fn send(&self, payload: &[u8]) -> io::Result<()> {
        let socket = self
            .socket
            .get_or_try_init(|| self.replies.get(self.local))
            .await?;
        let size = socket.send_to(payload, self.client).await?;
        self.context
            .counters
            .downloaded
            .fetch_add(size as u64, Ordering::Relaxed);
        Ok(())
    }
}

async fn dns_stream(
    context: &Context,
    names: &Names,
    mut stream: TcpStream,
    server: SocketAddr,
) -> io::Result<()> {
    loop {
        let length = match timeout(DNS_TCP_IDLE, stream.read_u16()).await {
            Err(_) => return Ok(()),
            Ok(Err(error)) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Ok(result) => usize::from(result?),
        };
        if length > MAX_DNS_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DNS message is too large",
            ));
        }
        let mut query = vec![0; length];
        timeout(DNS_TCP_IDLE, stream.read_exact(&mut query)).await??;
        let answer = resolve(context, names, server, &query).await?;
        let length = u16::try_from(answer.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "DNS answer is too large"))?;
        stream.write_u16(length).await?;
        stream.write_all(&answer).await?;
    }
}

pub(crate) fn invalid_dns() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "DNS query is not valid")
}

/// Answers one DNS query. `server` is the resolver that the app asked.
async fn resolve(
    context: &Context,
    names: &Names,
    server: SocketAddr,
    query: &[u8],
) -> io::Result<Vec<u8>> {
    if query.len() > MAX_DNS_MESSAGE {
        return Err(invalid_dns());
    }
    let request = Message::from_vec(query).map_err(|_| invalid_dns())?;
    let question = request.queries.first().ok_or_else(invalid_dns)?;
    let name = question
        .name
        .to_ascii()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let id = request.metadata.id;
    if !valid_domain(&name) {
        return exchange_tunnel(context, query, id).await;
    }
    let answer = match context.rules.route(Host::Name(&name), false) {
        Route::Block => {
            let mut response = Message::response(id, request.metadata.op_code);
            response.metadata.recursion_desired = request.metadata.recursion_desired;
            response.metadata.recursion_available = true;
            response.metadata.response_code = ResponseCode::NXDomain;
            response.add_query(question.clone());
            return response.to_vec().map_err(|_| invalid_dns());
        }
        Route::Direct => exchange_direct(context, server, query, id).await?,
        Route::Proxy | Route::Dpi => exchange_tunnel(context, query, id).await?,
    };
    names.remember(&name, &answer);
    Ok(answer)
}

fn answers(answer: &[u8], id: u16) -> bool {
    answer.len() >= 12 && u16::from_be_bytes([answer[0], answer[1]]) == id && answer[2] & 0x80 != 0
}

pub(crate) async fn exchange_direct(
    context: &Context,
    server: SocketAddr,
    query: &[u8],
    id: u16,
) -> io::Result<Vec<u8>> {
    let socket = protected_udp(server, &context.options.protect_socket)?;
    socket.send(query).await?;
    let mut buffer = vec![0; MAX_DATAGRAM];
    timeout(DNS_WAIT, async {
        loop {
            let size = socket.recv(&mut buffer).await?;
            if answers(&buffer[..size], id) {
                return Ok(buffer[..size].to_vec());
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "DNS timed out")))
}

pub(crate) async fn exchange_tunnel(
    context: &Context,
    query: &[u8],
    id: u16,
) -> io::Result<Vec<u8>> {
    let flow = context
        .client
        .open_udp(&Address::Ip(TUNNEL_RESOLVER))
        .await
        .map_err(io::Error::other)?;
    let (mut sender, mut receiver) = flow.into_split();
    sender.send(query).await?;
    let answer = timeout(DNS_WAIT, async {
        loop {
            match receiver.recv().await? {
                Some(response) if answers(&response.payload, id) => return Ok(response.payload),
                Some(_) => {}
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "DNS flow closed",
                    ));
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "DNS timed out")));
    let _ = sender.shutdown().await;
    answer
}
