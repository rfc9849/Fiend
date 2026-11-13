use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fiend_core::addr::Address;
use fiend_core::client::Client;
use fiend_core::net::ProtectSocket;
use futures::{SinkExt, StreamExt};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, interval, timeout};
use tokio_util::sync::CancellationToken;

use crate::desync;
use crate::dns::FakeDns;
use crate::icmp::{self, Echo};
use crate::packet::{FlowKey, Packet, Reassembly, Transport, abort_fin, udp_packets};
use crate::resolve::{Resolver, parse_server};
use crate::rules::{Route, RuleFile, Rules, is_blocked_udp_port, is_null_network};

const QUIC_PORT: u16 = 443;
const DNS_CAPACITY: usize = 8192;
const DNS_TTL: u32 = 60;

pub fn shared_fake_dns() -> Arc<Mutex<FakeDns>> {
    static SHARED: OnceLock<Arc<Mutex<FakeDns>>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            Arc::new(Mutex::new(
                FakeDns::new(DNS_CAPACITY, DNS_TTL).expect("default fake DNS settings are valid"),
            ))
        })
        .clone()
}

pub trait PacketIo: Send + Sync + 'static {
    fn recv(&self, packet: &mut [u8]) -> impl Future<Output = io::Result<usize>> + Send;
    fn send(&self, packet: &[u8]) -> impl Future<Output = io::Result<usize>> + Send;
}

impl<T: PacketIo> PacketIo for Arc<T> {
    async fn recv(&self, packet: &mut [u8]) -> io::Result<usize> {
        (**self).recv(packet).await
    }

    async fn send(&self, packet: &[u8]) -> io::Result<usize> {
        (**self).send(packet).await
    }
}

impl PacketIo for tun_rs::AsyncDevice {
    async fn recv(&self, packet: &mut [u8]) -> io::Result<usize> {
        self.recv(packet).await
    }
    async fn send(&self, packet: &[u8]) -> io::Result<usize> {
        self.send(packet).await
    }
}

#[derive(Clone)]
pub struct TunOptions {
    pub mtu: u16,
    pub ipv6: bool,
    pub bypass_lan: bool,
    pub default_route: crate::rules::DefaultRoute,
    pub direct_rules: String,
    pub block_rules: String,
    pub proxy_rules: String,
    pub dpi_rules: String,
    pub rule_files: Vec<RuleFile>,
    pub direct_resolver: String,
    pub max_tcp_flows: usize,
    pub max_udp_flows: usize,
    pub dns_capacity: usize,
    pub dns_ttl: u32,
    pub fake_dns: Option<Arc<Mutex<FakeDns>>>,
    pub udp_idle: Duration,
    pub protect_socket: ProtectSocket,
}

impl Default for TunOptions {
    fn default() -> Self {
        Self {
            mtu: 1500,
            ipv6: true,
            bypass_lan: false,
            default_route: crate::rules::DefaultRoute::default(),
            direct_rules: String::new(),
            block_rules: String::new(),
            proxy_rules: String::new(),
            dpi_rules: String::new(),
            rule_files: Vec::new(),
            direct_resolver: String::new(),
            max_tcp_flows: 256,
            max_udp_flows: 256,
            dns_capacity: DNS_CAPACITY,
            dns_ttl: DNS_TTL,
            fake_dns: None,
            udp_idle: Duration::from_secs(60),
            protect_socket: Arc::new(|_| Ok(())),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TunnelStats {
    pub uploaded: u64,
    pub downloaded: u64,
    pub dropped_packets: u64,
    pub flows: u64,
}

#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) uploaded: AtomicU64,
    pub(crate) downloaded: AtomicU64,
    pub(crate) dropped: AtomicU64,
    flows: AtomicU64,
    packet_id: AtomicU32,
}

pub struct TunEngine {
    pub(crate) context: Arc<Context>,
    pub(crate) cancel: CancellationToken,
    network: watch::Sender<u64>,
    pub(crate) running: AtomicBool,
}

pub(crate) struct Context {
    pub(crate) client: Client,
    pub(crate) options: TunOptions,
    pub(crate) rules: Arc<Rules>,
    resolver: Resolver,
    dns: Arc<Mutex<FakeDns>>,
    pub(crate) counters: Counters,
    aborted: Mutex<HashMap<FlowKey, Instant>>,
}

const ABORT_TTL: Duration = Duration::from_secs(30);

impl TunEngine {
    pub fn new(client: Client, options: TunOptions) -> io::Result<Self> {
        let (network, _) = watch::channel(0);
        Ok(Self {
            context: Arc::new(Context::new(client, options)?),
            cancel: CancellationToken::new(),
            network,
            running: AtomicBool::new(false),
        })
    }

    pub fn stats(&self) -> TunnelStats {
        let counters = &self.context.counters;
        TunnelStats {
            uploaded: counters.uploaded.load(Ordering::Relaxed),
            downloaded: counters.downloaded.load(Ordering::Relaxed),
            dropped_packets: counters.dropped.load(Ordering::Relaxed),
            flows: counters.flows.load(Ordering::Relaxed),
        }
    }

    pub fn rules(&self) -> Arc<Rules> {
        self.context.rules.clone()
    }

    pub fn stop(&self) {
        self.cancel.cancel();
        self.context.client.close();
    }

    pub fn network_changed(&self) {
        self.context.resolver.clear();
        self.context.client.network_changed();
        self.network
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    pub fn answer_dns(&self, query: &[u8]) -> Option<Vec<u8>> {
        self.context
            .dns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .answer(query)
            .ok()
    }

    pub async fn run<D: PacketIo>(&self, device: D) -> io::Result<()> {
        if self.running.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "TUN engine is already running",
            ));
        }
        let _running = Running(&self.running);
        let device = Arc::new(device);
        let mut network = self.network.subscribe();
        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Ok(()),
                _ = network.changed() => {},
                result = run_generation(device.clone(), self.context.clone()) => return result,
            }
        }
    }
}

impl Drop for TunEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct Running<'a>(pub(crate) &'a AtomicBool);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) struct FlowCount(Arc<Context>);
impl FlowCount {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        context.counters.flows.fetch_add(1, Ordering::Relaxed);
        Self(context)
    }
}
impl Drop for FlowCount {
    fn drop(&mut self) {
        self.0.counters.flows.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) enum Plan {
    Proxy,
    Block,
    Direct {
        addresses: Vec<SocketAddr>,
        dpi: bool,
    },
}

enum Completed {
    Tcp(FlowKey),
    Udp(FlowKey, u64),
    Icmp(FlowKey, u64),
    Infrastructure,
}

const MAX_ICMP_FLOWS: usize = 64;

async fn run_generation<D: PacketIo>(device: Arc<D>, context: Arc<Context>) -> io::Result<()> {
    let (stack, runner, _, listener) = netstack_smoltcp::StackBuilder::default()
        .enable_tcp(true)
        .enable_udp(false)
        .enable_icmp(false)
        .mtu(usize::from(context.options.mtu))
        .stack_buffer_size(64)
        .tcp_buffer_size(64)
        .tcp_recv_buffer_size(64 << 10)
        .tcp_send_buffer_size(64 << 10)
        .build()?;
    let (mut stack_input, mut stack_output) = stack.split();
    let mut listener = listener.ok_or_else(|| io::Error::other("TCP listener is not available"))?;
    let mut tasks = JoinSet::new();
    let runner = runner.ok_or_else(|| io::Error::other("TCP runner is not available"))?;
    tasks.spawn(async move {
        let result = runner.await;
        (Completed::Infrastructure, result)
    });
    let (output, mut packets) = mpsc::channel::<Vec<u8>>(64);
    let writer_device = device.clone();
    let writer_context = context.clone();
    tasks.spawn(async move {
        let result = async {
            while let Some(packet) = packets.recv().await {
                let size = writer_device.send(&packet).await?;
                if size != packet.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "TUN packet write is incomplete",
                    ));
                }
                writer_context
                    .counters
                    .downloaded
                    .fetch_add(size as u64, Ordering::Relaxed);
            }
            Ok(())
        }
        .await;
        (Completed::Infrastructure, result)
    });
    let stack_writer = output.clone();
    let stack_context = context.clone();
    tasks.spawn(async move {
        let result = async {
            while let Some(packet) = stack_output.next().await {
                let packet = packet?;
                match stack_context.reset_instead_of_fin(&packet) {
                    Some(packets) => {
                        for packet in packets {
                            stack_writer.send(packet).await.map_err(io::Error::other)?;
                        }
                    }
                    None => stack_writer.send(packet).await.map_err(io::Error::other)?,
                }
            }
            Ok(())
        }
        .await;
        (Completed::Infrastructure, result)
    });
    let mut tcp = HashSet::new();
    let mut udp: HashMap<FlowKey, (u64, mpsc::Sender<Vec<u8>>)> = HashMap::new();
    let mut echoes: HashMap<FlowKey, (u64, mpsc::Sender<Echo>)> = HashMap::new();
    let mut next_id = 0u64;
    let mut reassembly = Reassembly::default();
    let mut buffer = vec![0; 65575];
    loop {
        tokio::select! {
            result = device.recv(&mut buffer) => {
                let size = result?;
                if size == 0 { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "TUN device is closed")); }
                context.counters.uploaded.fetch_add(size as u64, Ordering::Relaxed);
                let raw = match reassembly.ingest(&buffer[..size], Instant::now()) {
                    Ok(Some(raw)) => raw,
                    Ok(None) => continue,
                    Err(_) => { context.drop_packet(); continue; }
                };
                if let Some(echo) = icmp::parse_echo_request(&raw) {
                    if echo.destination.is_ipv6() && !context.options.ipv6 || is_null_network(echo.destination) {
                        context.drop_packet();
                        continue;
                    }
                    let key = FlowKey { source: SocketAddr::new(echo.source, echo.ident), destination: SocketAddr::new(echo.destination, 0) };
                    if let Some((_, sender)) = echoes.get(&key) && !sender.is_closed() {
                        if sender.try_send(echo).is_err() { context.drop_packet(); }
                        continue;
                    }
                    echoes.remove(&key);
                    if echoes.len() >= MAX_ICMP_FLOWS { context.drop_packet(); continue; }
                    let Ok(destination) = context.destination(key.destination) else { context.drop_packet(); continue; };
                    let (sender, receiver) = mpsc::channel(16);
                    let _ = sender.try_send(echo);
                    next_id = next_id.wrapping_add(1);
                    let id = next_id;
                    echoes.insert(key, (id, sender));
                    let flow_context = context.clone();
                    let output = output.clone();
                    tasks.spawn(async move { let result = echo_flow(flow_context, destination, receiver, output).await; (Completed::Icmp(key, id), result) });
                    continue;
                }
                let packet = match Packet::parse(&raw) { Ok(packet) => packet, Err(_) => { context.drop_packet(); continue; } };
                if packet.key.destination.is_ipv6() && !context.options.ipv6 {
                    context.drop_packet();
                    if let Ok(reset) = packet.reset() { let _ = output.try_send(reset); }
                    continue;
                }
                if is_null_network(packet.key.destination.ip()) {
                    context.drop_packet();
                    if matches!(packet.transport, Transport::Tcp { .. }) && let Ok(reset) = packet.reset() { let _ = output.try_send(reset); }
                    continue;
                }
                match &packet.transport {
                    Transport::Tcp { syn, rst, .. } => {
                        if *syn && (tcp.contains(&packet.key) || tcp.len() >= context.options.max_tcp_flows) {
                            context.drop_packet();
                            if !tcp.contains(&packet.key) && let Ok(reset) = packet.reset() { let _ = output.try_send(reset); }
                            continue;
                        }
                        if *syn { tcp.insert(packet.key); context.forget_abort(&packet.key); }
                        if tcp.contains(&packet.key) || *rst { stack_input.send(raw).await?; }
                        else { context.drop_packet(); if let Ok(reset) = packet.reset() { let _ = output.try_send(reset); } }
                    }
                    Transport::Udp(payload) => {
                        if packet.key.destination.port() == 53 {
                            if let Ok(answer) = context.dns.lock().unwrap_or_else(|e| e.into_inner()).answer(payload) {
                                context.queue_udp(&output, packet.key, &answer);
                            } else { context.drop_packet(); }
                            continue;
                        }
                        if payload.is_empty() || is_blocked_udp_port(packet.key.destination.port()) { context.drop_packet(); continue; }
                        if let Some((_, sender)) = udp.get(&packet.key) && !sender.is_closed() {
                            if sender.try_send(payload.clone()).is_err() { context.drop_packet(); }
                            continue;
                        }
                        udp.remove(&packet.key);
                        if udp.len() >= context.options.max_udp_flows { context.drop_packet(); continue; }
                        let Ok(destination) = context.destination(packet.key.destination) else { context.drop_packet(); continue; };
                        let (sender, receiver) = mpsc::channel(16);
                        let _ = sender.try_send(payload.clone());
                        next_id = next_id.wrapping_add(1);
                        let id = next_id;
                        udp.insert(packet.key, (id, sender));
                        let flow_context = context.clone();
                        let output = output.clone();
                        let key = packet.key;
                        let count = FlowCount::new(context.clone());
                        let reply = TunReply { context: flow_context.clone(), output, key };
                        tasks.spawn(async move { let _count = count; let plan = flow_context.plan(&destination).await; let result = udp_flow(flow_context, key.destination.port(), destination, plan, receiver, reply).await; (Completed::Udp(key, id), result) });
                    }
                }
            }
            stream = listener.next() => {
                let Some((stream, source, destination)) = stream else { return Err(io::Error::other("TCP listener is closed")); };
                let key = FlowKey { source, destination };
                let flow_context = context.clone();
                let count = FlowCount::new(context.clone());
                tasks.spawn(async move { let _count = count; let result = tcp_flow(flow_context, stream, key).await; (Completed::Tcp(key), result) });
            }
            result = tasks.join_next() => {
                let Some(result) = result else { return Err(io::Error::other("TUN tasks are closed")); };
                let (completed, result) = result.map_err(io::Error::other)?;
                match completed {
                    Completed::Infrastructure => return result.and(Err(io::Error::other("TUN task stopped"))),
                    Completed::Tcp(key) => { tcp.remove(&key); }
                    Completed::Udp(key, id) => { if udp.get(&key).is_some_and(|(current, _)| *current == id) { udp.remove(&key); } }
                    Completed::Icmp(key, id) => { if echoes.get(&key).is_some_and(|(current, _)| *current == id) { echoes.remove(&key); } }
                }
                if let Err(error) = result { tracing::debug!(%error, "TUN flow closed"); }
            }
        }
    }
}

impl Context {
    fn new(client: Client, mut options: TunOptions) -> io::Result<Self> {
        if !(1280..=9000).contains(&options.mtu)
            || options.max_tcp_flows == 0
            || options.max_tcp_flows > 4096
            || options.max_udp_flows == 0
            || options.max_udp_flows > 4096
            || options.udp_idle.is_zero()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TUN options are not valid",
            ));
        }
        let mut rules = Rules::parse(
            &std::mem::take(&mut options.direct_rules),
            &std::mem::take(&mut options.block_rules),
            &std::mem::take(&mut options.proxy_rules),
        )
        .and_then(|rules| rules.with_dpi(&std::mem::take(&mut options.dpi_rules)))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        .with_default_route(options.default_route);
        for file in std::mem::take(&mut options.rule_files) {
            match rules.add_file(&file) {
                Ok(counts) => tracing::info!(
                    path = %file.path,
                    kind = ?file.kind,
                    entries = counts.total(),
                    "loaded rule file"
                ),
                Err(error) => tracing::warn!(%error, "skipped rule file"),
            }
        }
        rules.finish();
        let rules = Arc::new(rules);
        let resolver = Resolver::new(
            parse_server(&options.direct_resolver)?,
            options.protect_socket.clone(),
        );
        let dns = match options.fake_dns.take() {
            Some(dns) => {
                dns.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .set_rules(rules.clone());
                dns
            }
            None => Arc::new(Mutex::new(
                FakeDns::new(options.dns_capacity, options.dns_ttl)
                    .map_err(io::Error::other)?
                    .with_rules(rules.clone()),
            )),
        };
        Ok(Self {
            client,
            options,
            rules,
            resolver,
            dns,
            counters: Counters::default(),
            aborted: Mutex::new(HashMap::new()),
        })
    }

    fn abort_tcp(&self, key: FlowKey) {
        let now = Instant::now();
        let mut aborted = self.aborted.lock().unwrap_or_else(|e| e.into_inner());
        aborted.retain(|_, since| now.duration_since(*since) < ABORT_TTL);
        aborted.insert(key, now);
    }

    fn forget_abort(&self, key: &FlowKey) {
        let mut aborted = self.aborted.lock().unwrap_or_else(|e| e.into_inner());
        if !aborted.is_empty() {
            aborted.remove(key);
        }
    }

    fn reset_instead_of_fin(&self, packet: &[u8]) -> Option<Vec<Vec<u8>>> {
        let mut aborted = self.aborted.lock().unwrap_or_else(|e| e.into_inner());
        if aborted.is_empty() {
            return None;
        }
        let (key, packets) = abort_fin(packet)?;
        aborted.remove(&key)?;
        Some(packets)
    }

    fn destination(&self, address: SocketAddr) -> io::Result<Address> {
        self.dns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .destination(address)
            .map_err(io::Error::other)
    }

    pub(crate) fn drop_packet(&self) {
        self.counters.dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) async fn plan(&self, destination: &Address) -> Plan {
        let host = match destination {
            Address::Ip(address) => crate::rules::Host::Ip(address.ip()),
            Address::Domain { host, .. } => crate::rules::Host::Name(host),
        };
        let dpi = match self.rules.route(host, self.options.bypass_lan) {
            Route::Block => return Plan::Block,
            Route::Proxy => return Plan::Proxy,
            Route::Direct => false,
            Route::Dpi => true,
        };
        match destination {
            Address::Ip(address) => Plan::Direct {
                addresses: vec![*address],
                dpi,
            },
            Address::Domain { host, port } => {
                match self.resolver.lookup(host, self.options.ipv6).await {
                    Ok(found) if !found.is_empty() => Plan::Direct {
                        addresses: found
                            .into_iter()
                            .map(|ip| SocketAddr::new(ip, *port))
                            .collect(),
                        dpi,
                    },
                    Ok(_) => {
                        tracing::debug!(%host, "direct resolver found no address, using proxy");
                        Plan::Proxy
                    }
                    Err(error) => {
                        tracing::warn!(%host, %error, "direct resolver failed, using proxy");
                        Plan::Proxy
                    }
                }
            }
        }
    }

    async fn send_udp(
        &self,
        output: &mpsc::Sender<Vec<u8>>,
        key: FlowKey,
        payload: &[u8],
    ) -> io::Result<()> {
        let id = self.counters.packet_id.fetch_add(1, Ordering::Relaxed);
        let Ok(packets) = udp_packets(key.destination, key.source, payload, self.options.mtu, id)
        else {
            self.drop_packet();
            return Ok(());
        };
        for packet in packets {
            output
                .send(packet)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "TUN writer is closed"))?;
        }
        Ok(())
    }

    fn queue_udp(&self, output: &mpsc::Sender<Vec<u8>>, key: FlowKey, payload: &[u8]) {
        let id = self.counters.packet_id.fetch_add(1, Ordering::Relaxed);
        let Ok(packets) = udp_packets(key.destination, key.source, payload, self.options.mtu, id)
        else {
            self.drop_packet();
            return;
        };
        for packet in packets {
            if output.try_send(packet).is_err() {
                self.drop_packet();
                break;
            }
        }
    }
}

async fn tcp_flow(
    context: Arc<Context>,
    mut stream: netstack_smoltcp::TcpStream,
    key: FlowKey,
) -> io::Result<()> {
    let target = key.destination;
    if target.port() == 53 {
        loop {
            let length = timeout(Duration::from_secs(10), stream.read_u16()).await??;
            if length > 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "DNS packet is too large",
                ));
            }
            let mut packet = vec![0; usize::from(length)];
            timeout(Duration::from_secs(10), stream.read_exact(&mut packet)).await??;
            let answer = context
                .dns
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .answer(&packet)
                .map_err(io::Error::other)?;
            stream.write_u16(answer.len() as u16).await?;
            stream.write_all(&answer).await?;
        }
    }
    let destination = context.destination(target)?;
    match context.plan(&destination).await {
        Plan::Block => {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "destination is blocked",
            ));
        }
        Plan::Direct { addresses, dpi } => {
            let dial = timeout(Duration::from_secs(10), async {
                let mut last =
                    io::Error::new(io::ErrorKind::AddrNotAvailable, "no direct addresses");
                for address in addresses {
                    match fiend_core::net::dial(
                        &address.to_string(),
                        &context.options.protect_socket,
                    )
                    .await
                    {
                        Ok(stream) => return Ok(stream),
                        Err(error) => last = error,
                    }
                }
                Err(last)
            });
            if !dpi {
                let mut direct = dial.await??;
                copy_bidirectional(&mut stream, &mut direct).await?;
                return Ok(());
            }
            let (direct, first) = tokio::join!(
                dial,
                desync::read_first(&mut stream, desync::FIRST_DATA_WAIT)
            );
            let mut direct = direct??;
            let first = first?;
            direct.set_nodelay(true)?;
            desync::write_first(&mut direct, &first).await?;
            copy_bidirectional(&mut stream, &mut direct).await?;
        }
        Plan::Proxy => {
            let result = async {
                let mut lane = context
                    .client
                    .open_tcp(&destination)
                    .await
                    .map_err(io::Error::other)?;
                copy_bidirectional(&mut stream, &mut lane).await
            }
            .await;
            if result.is_err() {
                context.abort_tcp(key);
            }
            result?;
        }
    }
    Ok(())
}

async fn echo_flow(
    context: Arc<Context>,
    destination: Address,
    mut requests: mpsc::Receiver<Echo>,
    output: mpsc::Sender<Vec<u8>>,
) -> io::Result<()> {
    let idle = context.options.udp_idle;
    let Some(first) = requests.recv().await else {
        return Ok(());
    };
    let flow = match context.plan(&destination).await {
        Plan::Proxy => context.client.open_icmp(&destination).await.ok(),
        Plan::Block | Plan::Direct { .. } => None,
    };
    let Some(flow) = flow else {
        context.drop_packet();
        while let Ok(Some(_)) = timeout(idle, requests.recv()).await {
            context.drop_packet();
        }
        return Ok(());
    };
    let (mut sender, mut receiver) = flow.into_split();
    sender
        .send(&icmp::encode_payload(first.sequence, &first.data))
        .await?;
    let template = first;
    loop {
        tokio::select! {
            request = timeout(idle, requests.recv()) => {
                let Ok(Some(echo)) = request else { break };
                sender.send(&icmp::encode_payload(echo.sequence, &echo.data)).await?;
            }
            response = receiver.recv() => {
                let Some(response) = response? else { break };
                let Some((sequence, data)) = icmp::decode_payload(&response.payload) else { continue };
                if let Some(reply) = icmp::build_echo_reply(&template, sequence, data) {
                    output.send(reply).await.map_err(io::Error::other)?;
                }
            }
        }
    }
    let _ = sender.shutdown().await;
    Ok(())
}

pub(crate) trait UdpReply: Send + Sync + 'static {
    fn send(&self, payload: &[u8]) -> impl Future<Output = io::Result<()>> + Send;
}

struct TunReply {
    context: Arc<Context>,
    output: mpsc::Sender<Vec<u8>>,
    key: FlowKey,
}

impl UdpReply for TunReply {
    async fn send(&self, payload: &[u8]) -> io::Result<()> {
        self.context.send_udp(&self.output, self.key, payload).await
    }
}

pub(crate) async fn udp_flow<R: UdpReply>(
    context: Arc<Context>,
    port: u16,
    destination: Address,
    plan: Plan,
    mut packets: mpsc::Receiver<Vec<u8>>,
    reply: R,
) -> io::Result<()> {
    if matches!(plan, Plan::Block) {
        return Ok(());
    }
    if let Plan::Direct { dpi: true, .. } = plan
        && port == QUIC_PORT
    {
        while let Ok(Some(_)) = timeout(context.options.udp_idle, packets.recv()).await {
            context.drop_packet();
        }
        return Ok(());
    }
    if let Plan::Direct { addresses, .. } = plan {
        let target = *addresses.first().ok_or_else(|| {
            io::Error::new(io::ErrorKind::AddrNotAvailable, "no direct addresses")
        })?;
        let socket = protected_udp(target, &context.options.protect_socket)?;
        let mut buffer = vec![0; 65535];
        loop {
            let event = async {
                tokio::select! {
                    packet = packets.recv() => {
                        let Some(packet) = packet else { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "UDP flow is closed")); };
                        socket.send(&packet).await?;
                    }
                    size = socket.recv(&mut buffer) => { reply.send(&buffer[..size?]).await?; }
                }
                Ok(())
            };
            timeout(context.options.udp_idle, event).await??;
        }
    }
    let flow = context
        .client
        .open_udp(&destination)
        .await
        .map_err(io::Error::other)?;
    let (mut sender, mut receiver) = flow.into_split();
    let (activity, mut changed) = watch::channel(tokio::time::Instant::now());
    let writing = async {
        let mut keepalive = interval(Duration::from_secs(20));
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                packet = packets.recv() => {
                    let Some(packet) = packet else { return Ok(()); };
                    sender.send(&packet).await?;
                    activity.send_replace(tokio::time::Instant::now());
                }
                _ = keepalive.tick() => { sender.keepalive().await?; }
            }
        }
    };
    let reading = async {
        loop {
            match receiver.recv().await {
                Ok(Some(response)) => {
                    reply.send(&response.payload).await?;
                    activity.send_replace(tokio::time::Instant::now());
                }
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }
    };
    let idle = async {
        loop {
            let deadline = *changed.borrow_and_update() + context.options.udp_idle;
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return,
                result = changed.changed() => { if result.is_err() { return; } }
            }
        }
    };
    tokio::select! {
        result = writing => result,
        result = reading => result,
        _ = idle => Ok(()),
    }
}

pub(crate) fn protected_udp(target: SocketAddr, protect: &ProtectSocket) -> io::Result<UdpSocket> {
    let socket = Socket::new(
        Domain::for_address(target),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    socket.set_nonblocking(true)?;
    protect(&socket)?;
    let bind = SocketAddr::new(
        if target.is_ipv4() {
            "0.0.0.0".parse().map_err(io::Error::other)?
        } else {
            "::".parse().map_err(io::Error::other)?
        },
        0,
    );
    socket.bind(&bind.into())?;
    socket.connect(&target.into())?;
    UdpSocket::from_std(socket.into())
}
