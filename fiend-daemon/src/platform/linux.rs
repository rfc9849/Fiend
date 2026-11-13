//! Linux network setup. TPROXY is the default: the kernel gives the TCP
//! streams and UDP datagrams of the system to transparent sockets of the
//! daemon, and DNS stays as it is. When the kernel or iptables cannot do
//! TPROXY, or `FIEND_NETWORK_MODE=tun` is set, the daemon uses a TUN device
//! with policy routing as in wg-quick, and DNS through systemd-resolved with
//! a resolv.conf fallback.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use fiend_core::net::ProtectSocket;
use fiend_ipc::ConnectStep;
use tun_rs::{AsyncDevice, DeviceBuilder};

use fiend_root::gateway::{self, DNS_SET_TIMEOUT_SECONDS, DNS_SET_V4, DNS_SET_V6, NFT_TABLE};
use fiend_root::rules::{self as tproxy, Desktop};
use fiend_tun::rules::{DefaultRoute, RuleKind};

use super::commands::{
    DNSMASQ, DNSMASQ_CONFIG, Runner, Step, SystemRunner, apply_steps, dns_steps, route_steps,
    undo_all,
};
use super::{NetworkRequest, Platform};
use crate::journal::{Journal, JournalFile};

const IFACE: &str = "fiend0";
const TABLE: u32 = 51820;
const MARK: u32 = 51820;
const TUN_V4: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 2);
const TUN_V6: Ipv6Addr = Ipv6Addr::new(0xfd42, 0x6669, 0x656e, 0x6400, 0, 0, 0, 2);
const DNS_ADDRESS: &str = "198.18.0.1";
const RESOLV: &str = "/etc/resolv.conf";
const MONITOR_PERIOD: Duration = Duration::from_secs(2);
const EVENT_WAIT: Duration = Duration::from_millis(100);
const EVENT_SETTLE: Duration = Duration::from_millis(300);
const RULE_CHECK: Duration = Duration::from_secs(10);
const PORT_ATTEMPTS: usize = 32;
const MODE: &str = "FIEND_NETWORK_MODE";

/// Tries every match and target that the TPROXY rules use.
fn tproxy_available(ipv6: bool) -> io::Result<()> {
    let (setup, cleanup) = tproxy::probe(ipv6);
    let mut runner = SystemRunner;
    for command in &cleanup {
        let _ = runner.run(command);
    }
    let result = setup.iter().try_for_each(|command| runner.run(command));
    for command in &cleanup {
        let _ = runner.run(command);
    }
    result
}

struct LocalReplies;

impl fiend_tun::ReplySockets for LocalReplies {
    fn open(&self, local: SocketAddr) -> io::Result<std::net::UdpSocket> {
        fiend_root::sockets::reply(local, Some(MARK)).map(Into::into)
    }
}

/// The TPROXY rules that the monitor keeps in place.
struct WatchedRules {
    steps: Vec<Step>,
    check: Vec<Vec<String>>,
    state: Arc<Mutex<Journal>>,
    file: JournalFile,
}

/// Watches the primary network, and in TPROXY mode also the rules, which a
/// firewall manager can remove.
fn monitor(
    stop: Arc<AtomicBool>,
    network_changed: Arc<dyn Fn() + Send + Sync>,
    rules: Option<WatchedRules>,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("fiend-network-monitor".into())
        .spawn(move || {
            let events = route_events()
                .inspect_err(|error| tracing::warn!(%error, "cannot watch netlink route events"))
                .ok();
            let mut last = current_network();
            let mut since_check = Duration::ZERO;
            loop {
                let mut waited = Duration::ZERO;
                let mut changed_at = None;
                while waited < MONITOR_PERIOD
                    && changed_at.is_none_or(|at: Duration| waited < at + EVENT_SETTLE)
                {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let event = match &events {
                        Some(events) => route_event(events),
                        None => {
                            std::thread::sleep(EVENT_WAIT);
                            false
                        }
                    };
                    waited += EVENT_WAIT;
                    if event {
                        changed_at = Some(waited);
                    }
                }
                let now = current_network();
                if now != last {
                    tracing::info!("primary network changed");
                    last = now;
                    network_changed();
                }
                since_check += waited;
                if since_check < RULE_CHECK {
                    continue;
                }
                since_check = Duration::ZERO;
                let Some(WatchedRules {
                    steps,
                    check,
                    state,
                    file,
                }) = &rules
                else {
                    continue;
                };
                let mut runner = SystemRunner;
                if check.iter().all(|command| runner.run(command).is_ok()) {
                    continue;
                }
                tracing::warn!("TPROXY rules are missing, adding them again");
                let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
                undo_all(&journal, &mut runner);
                *journal = Journal::default();
                if let Err(error) = apply_steps(steps, &mut journal, file, &mut runner) {
                    tracing::error!(%error, "cannot add the TPROXY rules again");
                }
            }
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gateway {
    pub interfaces: Vec<String>,
    pub dns: SocketAddr,
    pub dns_redirect: bool,
}

pub struct LinuxPlatform {
    dir: PathBuf,
    journal: JournalFile,
    backup: PathBuf,
    gateway: Option<Gateway>,
}

impl LinuxPlatform {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            dir: state_dir.to_owned(),
            journal: JournalFile::new(state_dir.join("network-journal.json")),
            backup: state_dir.join("resolv.conf.backup"),
            gateway: None,
        }
    }

    pub fn with_gateway(mut self, gateway: Gateway) -> Self {
        self.gateway = Some(gateway);
        self
    }
}

struct NftCapture {
    writer: fiend_root::nft::SetWriter,
    ipv6: bool,
}

impl fiend_tun::CaptureSet for NftCapture {
    fn add(&self, addresses: &[std::net::IpAddr]) -> io::Result<()> {
        let (v4, v6): (Vec<_>, Vec<_>) = addresses.iter().partition(|ip| ip.is_ipv4());
        let v6 = if self.ipv6 { v6 } else { Vec::new() };
        self.writer.add(
            &[
                fiend_root::nft::Update {
                    set: DNS_SET_V4,
                    addresses: &v4,
                },
                fiend_root::nft::Update {
                    set: DNS_SET_V6,
                    addresses: &v6,
                },
            ],
            Duration::from_secs(DNS_SET_TIMEOUT_SECONDS.into()),
        )
    }
}

const DEFAULT_UPSTREAM: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53);
const UPSTREAM_CHECK: Duration = Duration::from_secs(10);

fn server_address(value: &str) -> Option<SocketAddr> {
    let (ip, port) = match value.split_once('#') {
        Some((ip, port)) => (ip, port.parse().ok()?),
        None => (value, 53),
    };
    let ip: std::net::IpAddr = ip.trim().parse().ok()?;
    (!ip.is_loopback() || port != 53).then_some(SocketAddr::new(ip, port))
}

pub fn dnsmasq_upstream(config: &Path, own: SocketAddr) -> Option<SocketAddr> {
    let mut resolv_files = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(config).ok()?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("dnsmasq.conf")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for line in text.lines().map(str::trim) {
            if let Some(value) = line.strip_prefix("server=")
                && !value.contains('/')
                && let Some(address) = server_address(value)
                && address != own
            {
                return Some(address);
            }
            if let Some(path) = line.strip_prefix("resolv-file=") {
                resolv_files.push(PathBuf::from(path));
            }
        }
    }
    resolv_files.iter().find_map(|path| {
        std::fs::read_to_string(path)
            .ok()?
            .lines()
            .find_map(|line| {
                let address = line.trim().strip_prefix("nameserver")?.trim();
                let ip: std::net::IpAddr = address.split('%').next()?.parse().ok()?;
                (!ip.is_loopback()).then_some(SocketAddr::new(ip, 53))
            })
    })
}

fn upstream(resolver: Option<SocketAddr>, own: SocketAddr) -> fiend_tun::Upstream {
    if let Some(resolver) = resolver {
        return Arc::new(move || resolver);
    }
    let cache: Mutex<Option<(std::time::Instant, SocketAddr)>> = Mutex::new(None);
    Arc::new(move || {
        let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((checked, address)) = *cache
            && checked.elapsed() < UPSTREAM_CHECK
        {
            return address;
        }
        let address = dnsmasq_upstream(Path::new(DNSMASQ_CONFIG), own).unwrap_or(DEFAULT_UPSTREAM);
        *cache = Some((std::time::Instant::now(), address));
        address
    })
}

fn ranges(
    rules: &fiend_tun::rules::Rules,
    kinds: &[RuleKind],
) -> Vec<(std::net::IpAddr, std::net::IpAddr)> {
    kinds
        .iter()
        .flat_map(|kind| rules.networks(*kind).ranges())
        .collect()
}

fn resolved_available() -> bool {
    std::process::Command::new("resolvectl")
        .arg("status")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The default gateway and interface from the main routing table.
pub fn default_route(table: &str) -> Option<String> {
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.len() > 7 && fields[1] == "00000000" && fields[7] == "00000000")
            .then(|| format!("{} {}", fields[0], fields[2]))
    })
}

fn current_default() -> Option<String> {
    std::fs::read_to_string("/proc/net/route")
        .ok()
        .and_then(|text| default_route(&text))
}

pub fn default_routes_v6(table: &str) -> Vec<String> {
    const RTF_UP: u32 = 0x0001;
    const RTF_REJECT: u32 = 0x0200;
    let mut routes: Vec<String> = table
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let flags = u32::from_str_radix(fields.get(8)?, 16).ok()?;
            let default = fields[0].bytes().all(|byte| byte == b'0') && fields[1] == "00";
            (fields.len() >= 10
                && default
                && flags & RTF_UP != 0
                && flags & RTF_REJECT == 0
                && fields[9] != "lo"
                && fields[9] != IFACE)
                .then(|| format!("{} {}", fields[9], fields[4]))
        })
        .collect();
    routes.sort();
    routes.dedup();
    routes
}

fn network_state(
    v4: Option<String>,
    v6: Vec<String>,
    addresses: impl IntoIterator<Item = (String, IpAddr)>,
) -> Vec<String> {
    let interfaces: Vec<&str> = v4
        .iter()
        .chain(&v6)
        .filter_map(|route| route.split_whitespace().next())
        .collect();
    let mut state: Vec<String> = v4.iter().chain(&v6).cloned().collect();
    for (interface, address) in addresses {
        if !interfaces.contains(&interface.as_str()) {
            continue;
        }
        let address = match address {
            IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => v4.to_string(),
            IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 != 0xfe80 && !v6.is_loopback() => {
                let segments = v6.segments();
                format!(
                    "{:x}:{:x}:{:x}:{:x}::/64",
                    segments[0], segments[1], segments[2], segments[3]
                )
            }
            _ => continue,
        };
        state.push(format!("{interface} {address}"));
    }
    state.sort();
    state.dedup();
    state
}

fn current_network() -> Vec<String> {
    let v6 = std::fs::read_to_string("/proc/net/ipv6_route")
        .map(|text| default_routes_v6(&text))
        .unwrap_or_default();
    let addresses = nix::ifaddrs::getifaddrs()
        .map(|addresses| {
            addresses
                .filter_map(|entry| {
                    let address = entry.address?;
                    let ip = if let Some(v4) = address.as_sockaddr_in() {
                        IpAddr::V4(v4.ip())
                    } else {
                        IpAddr::V6(address.as_sockaddr_in6()?.ip())
                    };
                    Some((entry.interface_name, ip))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    network_state(current_default(), v6, addresses)
}

fn route_events() -> io::Result<socket2::Socket> {
    use nix::sys::socket::{
        AddressFamily, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, socket,
    };
    let groups = nix::libc::RTMGRP_IPV4_IFADDR
        | nix::libc::RTMGRP_IPV4_ROUTE
        | nix::libc::RTMGRP_IPV6_IFADDR
        | nix::libc::RTMGRP_IPV6_ROUTE;
    let fd = socket(
        AddressFamily::Netlink,
        SockType::Raw,
        SockFlag::SOCK_CLOEXEC,
        SockProtocol::NetlinkRoute,
    )?;
    bind(
        std::os::fd::AsRawFd::as_raw_fd(&fd),
        &NetlinkAddr::new(0, groups as u32),
    )?;
    let socket = socket2::Socket::from(fd);
    socket.set_read_timeout(Some(EVENT_WAIT))?;
    Ok(socket)
}

fn route_event(socket: &socket2::Socket) -> bool {
    let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 8192];
    let mut seen = false;
    loop {
        match socket.recv(&mut buffer) {
            Ok(_) => {
                seen = true;
                if socket.set_nonblocking(true).is_err() {
                    break;
                }
            }
            Err(error) if error.raw_os_error() == Some(nix::libc::ENOBUFS) => seen = true,
            Err(_) => break,
        }
    }
    let _ = socket.set_nonblocking(false);
    seen
}

type Listeners = (Vec<socket2::Socket>, Vec<socket2::Socket>);

fn transparent_listeners(ipv6: bool) -> io::Result<(u16, Listeners)> {
    let mut last = io::Error::other("no free port");
    for _ in 0..PORT_ATTEMPTS {
        let port = fiend_root::sockets::random_port();
        match fiend_root::sockets::listeners(port, ipv6, Some(MARK)) {
            Ok(sockets) => return Ok((port, sockets)),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

impl LinuxPlatform {
    fn open_gateway(
        &self,
        gateway: &Gateway,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(fiend_tun::TransparentSockets, LinuxGuard)> {
        self.recover()?;
        let mut runner = SystemRunner;
        let delete = vec![
            "nft".to_owned(),
            "delete".into(),
            "table".into(),
            "inet".into(),
            NFT_TABLE.into(),
        ];
        let _ = runner.run(&delete);
        let (port, (tcp, udp)) = transparent_listeners(request.ipv6)?;
        let dns_udp = std::net::UdpSocket::bind(gateway.dns)?;
        let dns_tcp = std::net::TcpListener::bind(gateway.dns)?;
        let rules = &request.rules;
        let capture_all = rules.default_route() == DefaultRoute::Tunnel;
        let config = gateway::Gateway {
            interfaces: gateway.interfaces.clone(),
            port,
            ipv6: request.ipv6,
            capture_all,
            capture: ranges(rules, &[RuleKind::Proxy, RuleKind::Dpi, RuleKind::Block]),
            bypass: ranges(rules, &[RuleKind::Direct]),
            dns_redirect: gateway.dns_redirect,
        };
        std::fs::create_dir_all(&self.dir)?;
        let file = self.dir.join("gateway.nft");
        std::fs::write(&file, gateway::ruleset(&config))?;
        let mut steps = vec![Step {
            apply: vec![
                "nft".into(),
                "-f".into(),
                file.to_string_lossy().into_owned(),
            ],
            undo: Some(delete),
        }];
        steps.extend(
            gateway::routes(request.ipv6)
                .into_iter()
                .map(|(apply, undo)| Step {
                    apply,
                    undo: Some(undo),
                }),
        );
        let dns_steps = vec![Step {
            apply: vec![
                DNSMASQ.into(),
                format!("{}#{}", gateway.dns.ip(), gateway.dns.port()),
            ],
            undo: Some(vec![DNSMASQ.into()]),
        }];
        let writer = fiend_root::nft::SetWriter::open(NFT_TABLE)?;
        let state = Arc::new(Mutex::new(Journal::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = LinuxGuard {
            file: self.journal.clone(),
            state: state.clone(),
            stop: stop.clone(),
            monitor: None,
        };
        progress(ConnectStep::Routes);
        {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(&steps, &mut journal, &self.journal, &mut runner)?;
        }
        progress(ConnectStep::Dns);
        {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(&dns_steps, &mut journal, &self.journal, &mut runner)?;
        }
        tracing::info!(
            port,
            capture_all,
            networks = config.capture.len(),
            "router mode: listed traffic goes through TPROXY"
        );
        steps.extend(dns_steps);
        guard.monitor = Some(monitor(
            stop,
            network_changed,
            Some(WatchedRules {
                steps,
                check: vec![vec![
                    "nft".into(),
                    "list".into(),
                    "chain".into(),
                    "inet".into(),
                    NFT_TABLE.into(),
                    "prerouting".into(),
                ]],
                state,
                file: self.journal.clone(),
            }),
        )?);
        let sockets = fiend_tun::TransparentSockets {
            tcp: tcp.into_iter().map(Into::into).collect(),
            udp: udp.into_iter().map(Into::into).collect(),
            replies: Arc::new(LocalReplies),
            dns: Some(fiend_tun::GatewayDns {
                udp: vec![dns_udp],
                tcp: vec![dns_tcp],
                upstream: upstream(request.resolver, gateway.dns),
                capture: Arc::new(NftCapture {
                    writer,
                    ipv6: request.ipv6,
                }),
                capture_all,
            }),
        };
        Ok((sockets, guard))
    }
}

pub struct LinuxGuard {
    file: JournalFile,
    state: Arc<Mutex<Journal>>,
    stop: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
}

impl Drop for LinuxGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
        let journal = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        undo_all(&journal, &mut SystemRunner);
        if let Err(error) = self.file.clear() {
            tracing::warn!(%error, "cannot remove the network journal");
        }
        tracing::info!("network settings restored");
    }
}

impl Platform for LinuxPlatform {
    type Device = AsyncDevice;
    type Guard = LinuxGuard;

    fn protect_socket(&self) -> ProtectSocket {
        Arc::new(|socket| socket.set_mark(MARK))
    }

    fn blocks_after_crash(&self) -> bool {
        true
    }

    fn recover(&self) -> io::Result<()> {
        let journal = match self.journal.load() {
            Ok(journal) => journal,
            Err(error) => {
                tracing::error!(%error, "network journal is not readable");
                self.journal.quarantine();
                return Ok(());
            }
        };
        if !journal.is_empty() {
            tracing::warn!("restoring network settings from an earlier run");
            undo_all(&journal, &mut SystemRunner);
        }
        self.journal.clear()
    }

    fn open_transparent(
        &self,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Option<(fiend_tun::TransparentSockets, LinuxGuard)>> {
        if let Some(gateway) = &self.gateway {
            return self
                .open_gateway(gateway, request, progress, network_changed)
                .map(Some);
        }
        if std::env::var(MODE).is_ok_and(|mode| mode == "tun") {
            tracing::info!("{MODE}=tun is set, using the TUN device");
            return Ok(None);
        }
        self.recover()?;
        if let Err(error) = tproxy_available(request.ipv6) {
            tracing::info!(%error, "TPROXY is not available, using the TUN device");
            return Ok(None);
        }
        let (port, (tcp, udp)) = transparent_listeners(request.ipv6)?;
        let config = Desktop {
            ipv6: request.ipv6,
            bypass_lan: request.bypass_lan,
            protect_mark: MARK,
            port,
        };
        let steps: Vec<Step> = tproxy::desktop(&config)
            .into_iter()
            .map(|(apply, undo)| Step {
                apply,
                undo: Some(undo),
            })
            .collect();
        let state = Arc::new(Mutex::new(Journal::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = LinuxGuard {
            file: self.journal.clone(),
            state: state.clone(),
            stop: stop.clone(),
            monitor: None,
        };
        progress(ConnectStep::Routes);
        let applied = {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(&steps, &mut journal, &self.journal, &mut SystemRunner)
        };
        if let Err(error) = applied {
            drop(guard);
            tracing::warn!(%error, "TPROXY rules failed, using the TUN device");
            return Ok(None);
        }
        progress(ConnectStep::Dns);
        tracing::info!(port, "traffic goes through TPROXY");
        guard.monitor = Some(monitor(
            stop,
            network_changed,
            Some(WatchedRules {
                steps,
                check: tproxy::desktop_check(&config),
                state,
                file: self.journal.clone(),
            }),
        )?);
        let sockets = fiend_tun::TransparentSockets {
            tcp: tcp.into_iter().map(Into::into).collect(),
            udp: udp.into_iter().map(Into::into).collect(),
            replies: Arc::new(LocalReplies),
            dns: None,
        };
        Ok(Some((sockets, guard)))
    }

    fn open(
        &self,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(AsyncDevice, LinuxGuard)> {
        if self.gateway.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the router mode needs TPROXY",
            ));
        }
        self.recover()?;
        let device = DeviceBuilder::new()
            .name(IFACE)
            .ipv4(TUN_V4, 32, None)
            .ipv6(TUN_V6, 128)
            .mtu(request.mtu)
            .build_async()?;
        let state = Arc::new(Mutex::new(Journal::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = LinuxGuard {
            file: self.journal.clone(),
            state: state.clone(),
            stop: stop.clone(),
            monitor: None,
        };
        let mut runner = SystemRunner;

        progress(ConnectStep::Routes);
        {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(
                &route_steps(
                    IFACE,
                    TABLE,
                    &request
                        .servers
                        .iter()
                        .map(std::net::SocketAddr::ip)
                        .collect::<Vec<_>>(),
                ),
                &mut journal,
                &self.journal,
                &mut runner,
            )?;
        }

        progress(ConnectStep::Dns);
        {
            let steps = dns_steps(
                IFACE,
                DNS_ADDRESS,
                resolved_available(),
                &self.backup,
                Path::new(RESOLV),
            );
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(&steps, &mut journal, &self.journal, &mut runner)?;
        }

        guard.monitor = Some(monitor(stop, network_changed, None)?);
        Ok((device, guard))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upstream_is_the_server_that_dnsmasq_used() {
        let dir = std::env::temp_dir().join(format!("fiend-upstream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let own: SocketAddr = "127.0.0.1:5359".parse().unwrap();
        let resolv = dir.join("resolv.conf.auto");
        std::fs::write(
            &resolv,
            "# Interface wan\nnameserver 127.0.0.1\nnameserver 192.0.2.53\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("dnsmasq.conf.cfg01"),
            format!(
                "server=/lan/\nserver=127.0.0.1#5359\nresolv-file={}\n",
                resolv.display()
            ),
        )
        .unwrap();
        assert_eq!(
            dnsmasq_upstream(&dir, own),
            Some("192.0.2.53:53".parse().unwrap())
        );
        std::fs::write(dir.join("dnsmasq.conf.cfg02"), "server=127.0.0.1#5053\n").unwrap();
        assert_eq!(
            dnsmasq_upstream(&dir, own),
            Some("127.0.0.1:5053".parse().unwrap())
        );
        assert_eq!(dnsmasq_upstream(&dir.join("none"), own), None);
        assert_eq!(
            super::super::commands::dnsmasq_dirs(&dir),
            Vec::<PathBuf>::new()
        );
        std::fs::write(
            dir.join("dnsmasq.conf.cfg03"),
            "conf-dir=/tmp/dnsmasq.cfg03.d\n",
        )
        .unwrap();
        assert_eq!(
            super::super::commands::dnsmasq_dirs(&dir),
            [PathBuf::from("/tmp/dnsmasq.cfg03.d")]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_default_route_is_read_from_the_route_table() {
        let table = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
        assert_eq!(default_route(table).as_deref(), Some("eth0 0101A8C0"));
        assert_eq!(default_route("Iface\n"), None);
    }

    #[test]
    fn the_ipv6_default_route_skips_reject_and_tun_routes() {
        let table = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wlan0
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000001 00000000 00000001 fiend0
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo
20010db8000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 wlan0
";
        assert_eq!(
            default_routes_v6(table),
            ["wlan0 fe800000000000000000000000000001"]
        );
        assert!(default_routes_v6("").is_empty());
    }

    #[test]
    fn a_new_address_on_the_default_interface_is_a_new_network() {
        let route = || Some("eth0 0101A8C0".to_owned());
        let v6 = || vec!["eth0 fe800000000000000000000000000001".to_owned()];
        let at = |v4: &str, v6: &str| {
            vec![
                ("eth0".to_owned(), v4.parse().unwrap()),
                ("eth0".to_owned(), v6.parse().unwrap()),
                ("eth0".to_owned(), "fe80::1234".parse().unwrap()),
                ("docker0".to_owned(), "172.17.0.1".parse().unwrap()),
            ]
        };
        let before = network_state(route(), v6(), at("192.168.1.10", "2001:db8:1:2::10"));
        let rotated = network_state(route(), v6(), at("192.168.1.10", "2001:db8:1:2::99"));
        let renumbered = network_state(route(), v6(), at("192.168.1.11", "2001:db8:1:2::10"));
        let moved = network_state(route(), v6(), at("192.168.1.10", "2001:db8:9:2::10"));
        assert_eq!(before, rotated, "a new temporary address keeps the /64");
        assert_ne!(before, renumbered);
        assert_ne!(before, moved);
        assert!(!before.iter().any(|entry| entry.contains("docker0")));
        assert!(!before.iter().any(|entry| entry.contains("fe80::1234")));
    }
}
