//! Front end for sockets that the kernel fills with TPROXY: the local address
//! of each accepted TCP stream and the original destination of each UDP
//! datagram are the addresses that the app asked for.

use std::collections::HashMap;
use std::io::{self, IoSliceMut};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context as Task, Poll};
use std::time::Duration;

use fiend_core::addr::Address;
use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, SockaddrIn, SockaddrIn6, SockaddrLike, SockaddrStorage, recvmsg,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, Interest, ReadBuf, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::desync;
use crate::engine::{Context, FlowCount, Running, TunEngine, UdpReply, udp_flow};
use crate::packet::FlowKey;
use crate::rules::{Host, Route, is_blocked_udp_port};
use crate::sniff;

const SNIFF_PORTS: [u16; 4] = [80, 443, 8080, 8443];
const DIRECT_DIAL: Duration = Duration::from_secs(10);
const MAX_DATAGRAM: usize = 65535;

/// Opens UDP sockets bound to a foreign address, so that replies come from
/// the address that the app sent to. The kernel needs privileges for this.
pub trait ReplySockets: Send + Sync + 'static {
    fn open(&self, local: SocketAddr) -> io::Result<std::net::UdpSocket>;
}

pub struct TransparentSockets {
    pub tcp: Vec<std::net::TcpListener>,
    pub udp: Vec<std::net::UdpSocket>,
    pub replies: Arc<dyn ReplySockets>,
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
    let mut tcp = 0usize;
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
                        let count = FlowCount::new(context.clone());
                        tasks.spawn(async move {
                            let _count = count;
                            (Done::Tcp, tcp_flow(flow_context, stream, original).await)
                        });
                    }
                    Event::Udp { payload, source, destination } => {
                        if payload.is_empty() || is_blocked_udp_port(destination.port()) {
                            context.drop_packet();
                            continue;
                        }
                        context.counters.uploaded.fetch_add(payload.len() as u64, Ordering::Relaxed);
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
                        let flow_context = context.clone();
                        let count = FlowCount::new(context.clone());
                        tasks.spawn(async move {
                            let _count = count;
                            let result = udp_flow(flow_context, destination.port(), Address::Ip(destination), receiver, reply).await;
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

fn canonical(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(address.ip().to_canonical(), address.port())
}

fn reset(stream: &TcpStream) {
    let _ = socket2::SockRef::from(stream).set_linger(Some(Duration::ZERO));
}

fn route(context: &Context, destination: &Address) -> Route {
    let host = match destination {
        Address::Ip(address) => Host::Ip(address.ip()),
        Address::Domain { host, .. } => Host::Name(host),
    };
    context.rules.route(host, context.options.bypass_lan)
}

async fn tcp_flow(
    context: Arc<Context>,
    stream: TcpStream,
    original: SocketAddr,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut stream = Counted {
        inner: stream,
        context: context.clone(),
    };
    let first = if SNIFF_PORTS.contains(&original.port()) {
        desync::read_first(&mut stream, desync::FIRST_DATA_WAIT).await?
    } else {
        Vec::new()
    };
    let destination = match sniff::host(&first) {
        Some(host) => Address::domain(host, original.port()),
        None => Address::Ip(original),
    };
    let route = route(&context, &destination);
    tracing::debug!(%destination, ?route, "transparent TCP flow");
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
                lane.write_all(&first).await?;
            }
            copy_bidirectional(&mut stream, &mut lane).await?;
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
