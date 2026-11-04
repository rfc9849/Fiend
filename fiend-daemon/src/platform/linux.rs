//! Linux network setup. TPROXY is the default: the kernel gives the TCP
//! streams and UDP datagrams of the system to transparent sockets of the
//! daemon, and DNS stays as it is. When the kernel or iptables cannot do
//! TPROXY, or `FIEND_NETWORK_MODE=tun` is set, the daemon uses a TUN device
//! with policy routing as in wg-quick, and DNS through systemd-resolved with
//! a resolv.conf fallback.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use fiend_core::net::ProtectSocket;
use fiend_ipc::ConnectStep;
use tun_rs::{AsyncDevice, DeviceBuilder};

use fiend_root::rules::{self as tproxy, Desktop};

use super::commands::{Runner, Step, SystemRunner, apply_steps, dns_steps, route_steps, undo_all};
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
            let mut last = current_default();
            let mut since_check = Duration::ZERO;
            loop {
                let mut waited = Duration::ZERO;
                while waited < MONITOR_PERIOD {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    waited += Duration::from_millis(100);
                }
                let now = current_default();
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

pub struct LinuxPlatform {
    journal: JournalFile,
    backup: PathBuf,
}

impl LinuxPlatform {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            journal: JournalFile::new(state_dir.join("network-journal.json")),
            backup: state_dir.join("resolv.conf.backup"),
        }
    }
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
        if std::env::var(MODE).is_ok_and(|mode| mode == "tun") {
            tracing::info!("{MODE}=tun is set, using the TUN device");
            return Ok(None);
        }
        self.recover()?;
        if let Err(error) = tproxy_available(request.ipv6) {
            tracing::info!(%error, "TPROXY is not available, using the TUN device");
            return Ok(None);
        }
        let mut last = io::Error::other("no free port");
        let mut found = None;
        for _ in 0..PORT_ATTEMPTS {
            let port = fiend_root::sockets::random_port();
            match fiend_root::sockets::listeners(port, request.ipv6, Some(MARK)) {
                Ok(sockets) => {
                    found = Some((port, sockets));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => last = error,
                Err(error) => return Err(error),
            }
        }
        let Some((port, (tcp, udp))) = found else {
            return Err(last);
        };
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
        };
        Ok(Some((sockets, guard)))
    }

    fn open(
        &self,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(AsyncDevice, LinuxGuard)> {
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
    fn the_default_route_is_read_from_the_route_table() {
        let table = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
        assert_eq!(default_route(table).as_deref(), Some("eth0 0101A8C0"));
        assert_eq!(default_route("Iface\n"), None);
    }
}
