use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fiend_rules::{Host, RuleList};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

use crate::addr::Address;
use crate::client::{Client, ClientConfig, UdpReceiver, UdpSender};
use crate::socks5;
use crate::target::{self, TargetPolicy};

pub const DIRECT: &str = "direct";
pub const MAX_OUTBOUNDS: usize = 64;
pub const MAX_RULES: usize = 256;
const MAX_RULE_TEXT: usize = 256 * 1024;
const UDP_BUFFER: usize = u16::MAX as usize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Outbound {
    pub id: String,
    pub kind: OutboundKind,
    #[serde(default)]
    pub udp_fallback: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutboundKind {
    Direct,
    Block,
    Socks5 {
        server: String,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        udp: bool,
    },
    Http {
        server: String,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    Fiend {
        link: String,
    },
    Warp,
}

impl OutboundKind {
    pub fn supports_udp(&self) -> bool {
        match self {
            Self::Direct | Self::Fiend { .. } => true,
            Self::Socks5 { udp, .. } => *udp,
            Self::Block | Self::Http { .. } | Self::Warp => false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub pattern: String,
    pub outbound: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WarpSettings {
    pub enabled: bool,
    #[serde(default)]
    pub rotate_every: Option<u64>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn check_credentials(username: &Option<String>, password: &Option<String>) -> io::Result<()> {
    for value in [username, password].into_iter().flatten() {
        if value.is_empty() || value.len() > 255 || value.chars().any(char::is_control) {
            return Err(invalid(
                "proxy user names and passwords need 1 to 255 bytes and no control characters",
            ));
        }
    }
    if username.is_some() != password.is_some() {
        return Err(invalid("set both the proxy user name and the password"));
    }
    Ok(())
}

pub fn validate(outbounds: &[Outbound], rules: &[Rule], warp: &WarpSettings) -> io::Result<()> {
    if outbounds.len() > MAX_OUTBOUNDS || rules.len() > MAX_RULES {
        return Err(invalid("too many outbounds or rules"));
    }
    let mut total = 0usize;
    for (index, outbound) in outbounds.iter().enumerate() {
        if outbound.id.is_empty()
            || outbound.id.len() > 64
            || outbound.id == DIRECT
            || !outbound
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            || outbounds[..index]
                .iter()
                .any(|other| other.id == outbound.id)
        {
            return Err(invalid(
                "outbound ids must be unique, 1 to 64 letters, digits, '-', '_', or '.', and not 'direct'",
            ));
        }
        match &outbound.kind {
            OutboundKind::Direct | OutboundKind::Block => {}
            OutboundKind::Socks5 {
                server,
                username,
                password,
                ..
            }
            | OutboundKind::Http {
                server,
                username,
                password,
            } => {
                crate::share::check_server(server)
                    .map_err(|_| invalid("proxy server must be host:port"))?;
                check_credentials(username, password)?;
            }
            OutboundKind::Fiend { link } => {
                crate::share::decode(link)
                    .map_err(|error| invalid(format!("fiend outbound link: {error}")))?;
            }
            OutboundKind::Warp => {
                if !warp.enabled {
                    return Err(invalid("a WARP outbound needs WARP to be enabled"));
                }
            }
        }
        if let Some(fallback) = &outbound.udp_fallback {
            let target = find(outbounds, fallback)
                .ok_or_else(|| invalid("UDP fallback must name a known outbound"))?;
            if fallback == &outbound.id || !target.supports_udp() {
                return Err(invalid("UDP fallback must be another outbound with UDP"));
            }
        }
    }
    for rule in rules {
        total = total.saturating_add(rule.pattern.len());
        if rule.pattern.len() > MAX_RULE_TEXT || total > MAX_RULE_TEXT {
            return Err(invalid("rules are too large"));
        }
        if rule.outbound != DIRECT && !outbounds.iter().any(|item| item.id == rule.outbound) {
            return Err(invalid(format!(
                "rule names an unknown outbound: {}",
                rule.outbound
            )));
        }
        fiend_rules::parse_list(&rule.pattern, true)
            .map_err(|error| invalid(format!("rule pattern: {error}")))?;
    }
    if let Some(every) = warp.rotate_every
        && !(3600..=30 * 86400).contains(&every)
    {
        return Err(invalid("WARP rotation must be from 1 hour to 30 days"));
    }
    Ok(())
}

fn find<'a>(outbounds: &'a [Outbound], id: &str) -> Option<&'a OutboundKind> {
    if id == DIRECT {
        return Some(&OutboundKind::Direct);
    }
    outbounds
        .iter()
        .find(|outbound| outbound.id == id)
        .map(|outbound| &outbound.kind)
}

#[derive(Default)]
pub struct Counters {
    pub up: AtomicU64,
    pub down: AtomicU64,
    pub errors: AtomicU64,
}

pub struct Runtime {
    pub config: Outbound,
    pub counters: Arc<Counters>,
    client: Option<Client>,
}

pub struct Router {
    rules: Vec<(RuleList, usize)>,
    outbounds: Vec<Arc<Runtime>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundStats {
    pub id: String,
    pub up: u64,
    pub down: u64,
    pub errors: u64,
}

impl Router {
    pub fn build(outbounds: &[Outbound], rules: &[Rule], old: Option<&Router>) -> io::Result<Self> {
        let mut runtimes = vec![
            old.and_then(|old| old.outbounds.first().cloned())
                .unwrap_or_else(|| {
                    Arc::new(Runtime {
                        config: Outbound {
                            id: DIRECT.into(),
                            kind: OutboundKind::Direct,
                            udp_fallback: None,
                        },
                        counters: Arc::default(),
                        client: None,
                    })
                }),
        ];
        for outbound in outbounds {
            if let Some(previous) = old.and_then(|old| {
                old.outbounds
                    .iter()
                    .find(|runtime| runtime.config == *outbound)
            }) {
                runtimes.push(previous.clone());
                continue;
            }
            let client = match &outbound.kind {
                OutboundKind::Fiend { link } => {
                    let share =
                        crate::share::decode(link).map_err(|error| invalid(error.to_string()))?;
                    let mut config =
                        ClientConfig::new(String::new(), share.secret.to_vec(), share.template);
                    config.servers = share.servers;
                    Some(Client::new(
                        crate::config::build_client(config)
                            .map_err(|error| invalid(error.to_string()))?,
                    ))
                }
                _ => None,
            };
            runtimes.push(Arc::new(Runtime {
                config: outbound.clone(),
                counters: Arc::default(),
                client,
            }));
        }
        let mut table = Vec::with_capacity(rules.len());
        for rule in rules {
            let list = fiend_rules::parse_list(&rule.pattern, true)
                .map_err(|error| invalid(error.to_string()))?;
            let index = runtimes
                .iter()
                .position(|runtime| runtime.config.id == rule.outbound)
                .ok_or_else(|| invalid("rule names an unknown outbound"))?;
            table.push((list, index));
        }
        Ok(Self {
            rules: table,
            outbounds: runtimes,
        })
    }

    pub fn route(&self, address: &Address) -> Arc<Runtime> {
        let host = match address {
            Address::Ip(address) => Host::Ip(address.ip()),
            Address::Domain { host, .. } => Host::Name(host),
        };
        let index = self
            .rules
            .iter()
            .find(|(list, _)| list.matches(host))
            .map_or(0, |(_, index)| *index);
        self.outbounds[index].clone()
    }

    pub fn udp_route(&self, address: &Address) -> Option<Arc<Runtime>> {
        let runtime = self.route(address);
        if runtime.config.kind.supports_udp() {
            return Some(runtime);
        }
        let fallback = runtime.config.udp_fallback.as_ref()?;
        self.outbounds
            .iter()
            .find(|candidate| &candidate.config.id == fallback)
            .cloned()
    }

    pub fn stats(&self) -> Vec<OutboundStats> {
        self.outbounds
            .iter()
            .map(|runtime| OutboundStats {
                id: runtime.config.id.clone(),
                up: runtime.counters.up.swap(0, Ordering::Relaxed),
                down: runtime.counters.down.swap(0, Ordering::Relaxed),
                errors: runtime.counters.errors.swap(0, Ordering::Relaxed),
            })
            .collect()
    }

    pub fn close(&self) {
        for runtime in &self.outbounds {
            if let Some(client) = &runtime.client {
                client.close();
            }
        }
    }

    pub fn runtimes(&self) -> &[Arc<Runtime>] {
        &self.outbounds
    }
}

pub struct Metered<S> {
    inner: S,
    counters: Arc<Counters>,
}

impl<S> Metered<S> {
    pub fn new(inner: S, counters: Arc<Counters>) -> Self {
        Self { inner, counters }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Metered<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        let count = buf.filled().len().saturating_sub(before) as u64;
        self.counters.down.fetch_add(count, Ordering::Relaxed);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Metered<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(count)) = &result {
            self.counters.up.fetch_add(*count as u64, Ordering::Relaxed);
        }
        result
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub type BoxStream = Pin<Box<dyn Stream>>;

pub type Resolver = Arc<
    dyn Fn(
            String,
            u16,
        )
            -> Pin<Box<dyn std::future::Future<Output = io::Result<Vec<SocketAddr>>> + Send>>
        + Send
        + Sync,
>;

pub struct Context {
    pub policy: TargetPolicy,
    pub timeout: Duration,
    pub warp: Option<SocketAddr>,
    pub resolver: Option<Resolver>,
}

impl Context {
    async fn check(&self, address: &Address) -> io::Result<()> {
        match (&self.resolver, address) {
            (Some(resolver), Address::Domain { host, port })
                if host.parse::<std::net::IpAddr>().is_err() =>
            {
                if let Ok(Ok(resolved)) =
                    timeout(Duration::from_secs(5), resolver(host.clone(), *port)).await
                {
                    for address in resolved {
                        self.policy.check(address)?;
                    }
                }
                Ok(())
            }
            _ => check_remote(address, &self.policy).await,
        }
    }
}

pub async fn check_remote(address: &Address, policy: &TargetPolicy) -> io::Result<()> {
    match address {
        Address::Ip(address) => policy.check(*address),
        Address::Domain { host, port } => {
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                return policy.check(SocketAddr::new(ip, *port));
            }
            let lookup =
                tokio::time::timeout(Duration::from_secs(5), crate::resolve::lookup(host, *port))
                    .await;
            if let Ok(Ok(resolved)) = lookup {
                for address in resolved {
                    policy.check(address)?;
                }
            }
            Ok(())
        }
    }
}

fn blocked() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "a routing rule blocks this target",
    )
}

fn warp_address(context: &Context) -> io::Result<SocketAddr> {
    context
        .warp
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "WARP is not running"))
}

impl Runtime {
    pub fn client(&self) -> Option<&Client> {
        self.client.as_ref()
    }

    pub async fn connect_tcp(&self, address: &Address, context: &Context) -> io::Result<BoxStream> {
        let result = timeout(context.timeout, self.connect_inner(address, context))
            .await
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "outbound connect timed out",
                ))
            });
        if result.is_err() {
            self.counters.errors.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    async fn connect_inner(&self, address: &Address, context: &Context) -> io::Result<BoxStream> {
        match &self.config.kind {
            OutboundKind::Direct => Ok(Box::pin(
                target::connect_tcp(address, &context.policy, context.timeout).await?,
            )),
            OutboundKind::Block => Err(blocked()),
            OutboundKind::Socks5 {
                server,
                username,
                password,
                ..
            } => {
                context.check(address).await?;
                let mut stream = TcpStream::connect(server.as_str()).await?;
                stream.set_nodelay(true)?;
                socks_connect(
                    &mut stream,
                    address,
                    username.as_deref().zip(password.as_deref()),
                    socks5::CMD_CONNECT,
                )
                .await?;
                Ok(Box::pin(stream))
            }
            OutboundKind::Warp => {
                context.check(address).await?;
                let mut stream = TcpStream::connect(warp_address(context)?).await?;
                stream.set_nodelay(true)?;
                socks_connect(&mut stream, address, None, socks5::CMD_CONNECT).await?;
                Ok(Box::pin(stream))
            }
            OutboundKind::Http {
                server,
                username,
                password,
            } => {
                context.check(address).await?;
                let mut stream = TcpStream::connect(server.as_str()).await?;
                stream.set_nodelay(true)?;
                http_connect(
                    &mut stream,
                    address,
                    username.as_deref().zip(password.as_deref()),
                )
                .await?;
                Ok(Box::pin(stream))
            }
            OutboundKind::Fiend { .. } => {
                context.check(address).await?;
                let client = self
                    .client
                    .as_ref()
                    .ok_or_else(|| io::Error::other("fiend outbound is not ready"))?;
                let lane = client.open_tcp(address).await.map_err(io::Error::other)?;
                Ok(Box::pin(lane))
            }
        }
    }

    pub async fn open_udp(&self, address: &Address, context: &Context) -> io::Result<UdpPath> {
        let result = timeout(context.timeout, self.open_udp_inner(address, context))
            .await
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "outbound connect timed out",
                ))
            });
        if result.is_err() {
            self.counters.errors.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    async fn open_udp_inner(&self, address: &Address, context: &Context) -> io::Result<UdpPath> {
        match &self.config.kind {
            OutboundKind::Direct => {
                let targets = target::resolve_all(address, &context.policy).await?;
                let mut last = None;
                for target in targets {
                    let bind = if target.is_ipv4() {
                        "0.0.0.0:0"
                    } else {
                        "[::]:0"
                    };
                    let socket = UdpSocket::bind(bind).await?;
                    match socket.connect(target).await {
                        Ok(()) => {
                            let socket = Arc::new(socket);
                            return Ok(UdpPath {
                                tx: UdpTx::Socket(socket.clone()),
                                rx: UdpRx::Socket(socket),
                                guard: None,
                            });
                        }
                        Err(error) => last = Some(error),
                    }
                }
                Err(last.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "target has no addresses")
                }))
            }
            OutboundKind::Socks5 {
                server,
                username,
                password,
                udp: true,
            } => {
                context.check(address).await?;
                let mut control = TcpStream::connect(server.as_str()).await?;
                let proxy = control.peer_addr()?;
                let any = Address::Ip(SocketAddr::from(([0, 0, 0, 0], 0)));
                let mut relay = match socks_connect(
                    &mut control,
                    &any,
                    username.as_deref().zip(password.as_deref()),
                    socks5::CMD_UDP_ASSOCIATE,
                )
                .await?
                {
                    Address::Ip(relay) => relay,
                    Address::Domain { .. } => proxy,
                };
                if relay.ip().is_unspecified() {
                    relay.set_ip(proxy.ip());
                }
                let bind = if relay.is_ipv4() {
                    "0.0.0.0:0"
                } else {
                    "[::]:0"
                };
                let socket = UdpSocket::bind(bind).await?;
                socket.connect(relay).await?;
                let socket = Arc::new(socket);
                Ok(UdpPath {
                    tx: UdpTx::Socks(socket.clone(), address.clone()),
                    rx: UdpRx::Socks(socket),
                    guard: Some(control),
                })
            }
            OutboundKind::Fiend { .. } => {
                context.check(address).await?;
                let client = self
                    .client
                    .as_ref()
                    .ok_or_else(|| io::Error::other("fiend outbound is not ready"))?;
                let (tx, rx) = client
                    .open_udp(address)
                    .await
                    .map_err(io::Error::other)?
                    .into_split();
                Ok(UdpPath {
                    tx: UdpTx::Fiend(tx),
                    rx: UdpRx::Fiend(rx),
                    guard: None,
                })
            }
            _ => Err(blocked()),
        }
    }
}

pub struct UdpPath {
    pub tx: UdpTx,
    pub rx: UdpRx,
    pub guard: Option<TcpStream>,
}

pub enum UdpTx {
    Socket(Arc<UdpSocket>),
    Socks(Arc<UdpSocket>, Address),
    Fiend(UdpSender),
}

pub enum UdpRx {
    Socket(Arc<UdpSocket>),
    Socks(Arc<UdpSocket>),
    Fiend(UdpReceiver),
}

impl UdpTx {
    pub async fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        match self {
            Self::Socket(socket) => socket.send(payload).await.map(drop),
            Self::Socks(socket, destination) => {
                let datagram = socks5::encode_udp_request(destination, payload)?;
                socket.send(&datagram).await.map(drop)
            }
            Self::Fiend(sender) => sender.send(payload).await,
        }
    }
}

impl UdpRx {
    pub async fn recv(&mut self) -> io::Result<Option<(Address, Vec<u8>)>> {
        match self {
            Self::Socket(socket) => {
                let mut buffer = vec![0u8; UDP_BUFFER];
                let (length, source) = socket.recv_from(&mut buffer).await?;
                buffer.truncate(length);
                Ok(Some((Address::Ip(source), buffer)))
            }
            Self::Socks(socket) => loop {
                let mut buffer = vec![0u8; UDP_BUFFER];
                let length = socket.recv(&mut buffer).await?;
                if let Some((source, payload)) = socks5::parse_udp_request(&buffer[..length]) {
                    return Ok(Some((source, payload.to_vec())));
                }
            },
            Self::Fiend(receiver) => loop {
                match receiver.recv().await? {
                    None => return Ok(None),
                    Some(response) => {
                        if let Some(source) = response.source {
                            return Ok(Some((source, response.payload)));
                        }
                    }
                }
            },
        }
    }
}

pub async fn socks_connect<S>(
    stream: &mut S,
    address: &Address,
    credentials: Option<(&str, &str)>,
    command: u8,
) -> io::Result<Address>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let failed =
        |message: &str| io::Error::new(io::ErrorKind::ConnectionRefused, message.to_owned());
    let methods: &[u8] = if credentials.is_some() { &[2] } else { &[0] };
    let mut greeting = vec![socks5::VERSION, methods.len() as u8];
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice[0] != socks5::VERSION || !methods.contains(&choice[1]) {
        return Err(failed("SOCKS5 proxy refused the authentication method"));
    }
    if let Some((user, password)) = credentials {
        let mut auth = vec![1, user.len() as u8];
        auth.extend_from_slice(user.as_bytes());
        auth.push(password.len() as u8);
        auth.extend_from_slice(password.as_bytes());
        stream.write_all(&auth).await?;
        let mut status = [0u8; 2];
        stream.read_exact(&mut status).await?;
        if status[1] != 0 {
            return Err(failed("SOCKS5 proxy refused the user name or password"));
        }
    }
    let mut request = vec![socks5::VERSION, command, 0];
    address
        .encode_into(&mut request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    stream.write_all(&request).await?;
    stream.flush().await?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != socks5::VERSION {
        return Err(failed("SOCKS5 proxy sent an invalid reply"));
    }
    let bound = socks5::read_address(stream, reply[3]).await?;
    if reply[1] != socks5::REP_SUCCESS {
        return Err(failed(&format!(
            "SOCKS5 proxy refused the request (code {})",
            reply[1]
        )));
    }
    Ok(bound)
}

pub async fn http_connect<S>(
    stream: &mut S,
    address: &Address,
    credentials: Option<(&str, &str)>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let authority = match address {
        Address::Ip(address) => address.to_string(),
        Address::Domain { host, port } => format!("{host}:{port}"),
    };
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some((user, password)) = credentials {
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            STANDARD.encode(format!("{user}:{password}"))
        ));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;
    let mut head = Vec::with_capacity(256);
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP proxy reply is too large",
            ));
        }
        let mut byte = [0u8; 1];
        if stream.read(&mut byte).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP proxy closed the connection",
            ));
        }
        head.push(byte[0]);
    }
    let line = head.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line);
    let mut parts = line.split_whitespace();
    let version = parts.next().unwrap_or_default();
    let status = parts
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    if !version.starts_with("HTTP/1.") || !(200..300).contains(&status) {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("HTTP proxy refused the request (status {status})"),
        ));
    }
    Ok(())
}
