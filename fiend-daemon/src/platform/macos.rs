//! macOS network setup: utun device, split default routes, `IP_BOUND_IF`
//! socket protection, and DNS override through SCDynamicStore.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::propertylist::CFPropertyListSubClass;
use core_foundation::string::CFString;
use fiend_core::net::ProtectSocket;
use fiend_ipc::ConnectStep;
use route_manager::{Route, RouteManager};
use system_configuration::dynamic_store::{SCDynamicStore, SCDynamicStoreBuilder};
use tun_rs::{AsyncDevice, DeviceBuilder};

use super::commands::{Step, SystemRunner, apply_steps, undo_all};
use super::local_dns::LocalDns;
use super::{NetworkRequest, Platform};
use crate::journal::{Journal, JournalFile, RouteEntry};

const TUN_V4: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 2);
const TUN_V6: Ipv6Addr = Ipv6Addr::new(0xfd42, 0x6669, 0x656e, 0x6400, 0, 0, 0, 2);
const LOCAL_DNS: Ipv4Addr = Ipv4Addr::new(127, 42, 0, 53);
const DNS_PATTERN: &str = "(State|Setup):/Network/Service/[^/]+/DNS";
const MONITOR_PERIOD: Duration = Duration::from_secs(2);

pub struct MacPlatform {
    journal: JournalFile,
    bound_interface: Arc<AtomicU32>,
}

impl MacPlatform {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            journal: JournalFile::new(state_dir.join("network-journal.json")),
            bound_interface: Arc::new(AtomicU32::new(0)),
        }
    }
}

fn split_routes() -> Vec<(IpAddr, u8)> {
    let mut routes = Vec::with_capacity(16);
    for prefix in 1..=8u8 {
        let first = if prefix == 1 { 128 } else { 1u8 << (8 - prefix) };
        routes.push((IpAddr::V4(Ipv4Addr::new(first, 0, 0, 0)), prefix));
        let first = if prefix == 1 { 0x8000 } else { 1u16 << (16 - prefix) };
        routes.push((IpAddr::V6(Ipv6Addr::new(first, 0, 0, 0, 0, 0, 0, 0)), prefix));
    }
    routes
}

/// The interface of the system default route, without our own tunnel.
fn default_interface(exclude: Option<u32>) -> io::Result<u32> {
    let routes = RouteManager::new()?.list()?;
    let find = |want_v4: bool| {
        routes
            .iter()
            .filter(|route| route.prefix() == 0 && route.destination().is_ipv4() == want_v4)
            .filter(|route| !route.if_scope())
            .filter_map(Route::if_index)
            .find(|index| Some(*index) != exclude)
    };
    find(true).or_else(|| find(false)).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "There is no network connection",
        )
    })
}

/// The interface for protected sockets. The server handshake runs before the
/// tunnel is up, so the first call finds the system default interface.
fn bound_interface(bound: &AtomicU32) -> io::Result<NonZeroU32> {
    if let Some(index) = NonZeroU32::new(bound.load(Ordering::Acquire)) {
        return Ok(index);
    }
    let index = NonZeroU32::new(default_interface(None)?).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "There is no network connection",
        )
    })?;
    bound.store(index.get(), Ordering::Release);
    Ok(index)
}

fn protect(bound: Arc<AtomicU32>) -> ProtectSocket {
    Arc::new(move |socket| {
        let index = bound_interface(&bound)?;
        if socket.local_addr()?.is_ipv6() {
            socket.bind_device_by_index_v6(Some(index))
        } else {
            socket.bind_device_by_index_v4(Some(index))
        }
    })
}

mod plist {
    //! Raw property list calls. The only unsafe code in the daemon.
    #![allow(unsafe_code)]

    use core_foundation::data::CFData;
    use core_foundation::propertylist::{
        CFPropertyList, create_data, create_with_data, kCFPropertyListImmutable,
        kCFPropertyListXMLFormat_v1_0,
    };

    pub fn to_xml(value: &CFPropertyList) -> Option<String> {
        let data = create_data(value.as_concrete_TypeRef(), kCFPropertyListXMLFormat_v1_0).ok()?;
        String::from_utf8(data.bytes().to_vec()).ok()
    }

    pub fn from_xml(xml: &str) -> Option<CFPropertyList> {
        let (pointer, _) = create_with_data(
            CFData::from_buffer(xml.as_bytes()),
            kCFPropertyListImmutable,
        )
        .ok()?;
        Some(unsafe { CFPropertyList::wrap_under_create_rule(pointer) })
    }
}

struct Dns {
    store: SCDynamicStore,
    ours_xml: String,
}

fn ours() -> CFDictionary {
    let servers = CFArray::from_CFTypes(&[CFString::new(&LOCAL_DNS.to_string())]);
    CFDictionary::from_CFType_pairs(&[(CFString::new("ServerAddresses"), servers.as_CFType())])
        .to_untyped()
}

impl Dns {
    fn open() -> io::Result<Self> {
        let store = SCDynamicStoreBuilder::new("fiend")
            .build()
            .ok_or_else(|| io::Error::other("Cannot open the system network configuration"))?;
        let ours_xml = plist::to_xml(&ours().to_CFPropertyList())
            .ok_or_else(|| io::Error::other("Cannot build the DNS record"))?;
        Ok(Self { store, ours_xml })
    }

    fn keys(&self) -> Vec<String> {
        self.store
            .get_keys(DNS_PATTERN)
            .map(|keys| keys.iter().map(|key| key.to_string()).collect())
            .unwrap_or_default()
    }

    fn is_ours(&self, key: &str) -> bool {
        self.store
            .get(key)
            .and_then(|value| plist::to_xml(&value))
            .is_some_and(|xml| xml == self.ours_xml)
    }

    /// Point every service DNS record at the tunnel resolver. Records that
    /// the journal does not know yet get their original saved first.
    fn apply(&self, journal: &JournalFile, state: &Mutex<Journal>) -> io::Result<usize> {
        let mut changed = 0;
        for key in self.keys() {
            if self.is_ours(&key) {
                continue;
            }
            let original = self.store.get(key.as_str()).and_then(|v| plist::to_xml(&v));
            {
                let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                if state.add_dns(&key, original) {
                    journal.save(&state)?;
                }
            }
            if self.store.set(key.as_str(), ours()) {
                changed += 1;
            } else {
                tracing::warn!(key, "cannot set the DNS servers");
            }
        }
        Ok(changed)
    }

    fn restore(&self, journal: &Journal) {
        for entry in &journal.dns {
            let Some(current) = self.store.get(entry.key.as_str()) else {
                continue;
            };
            if current.downcast::<CFDictionary>().is_none() {
                continue;
            }
            let restored = match entry.original.as_deref().and_then(plist::from_xml) {
                Some(original) => self.store.set_raw(entry.key.as_str(), &original),
                None => self.store.remove(entry.key.as_str()),
            };
            if !restored {
                tracing::warn!(key = entry.key, "cannot restore the DNS servers");
            }
        }
    }
}

fn delete_routes(journal: &Journal) {
    let Ok(mut manager) = RouteManager::new() else {
        return;
    };
    for entry in &journal.routes {
        let route = Route::new(entry.destination, entry.prefix).with_if_index(entry.if_index);
        let _ = manager.delete(&route);
    }
}

fn loopback_alias() -> Step {
    let address = LOCAL_DNS.to_string();
    Step {
        apply: ["/sbin/ifconfig", "lo0", "alias", &address, "netmask", "255.255.255.255"]
            .map(str::to_owned)
            .to_vec(),
        undo: Some(
            ["/sbin/ifconfig", "lo0", "-alias", &address]
                .map(str::to_owned)
                .to_vec(),
        ),
    }
}

fn undo(journal: &Journal) {
    delete_routes(journal);
    if !journal.dns.is_empty() {
        match Dns::open() {
            Ok(dns) => dns.restore(journal),
            Err(error) => tracing::warn!(%error, "cannot restore the DNS servers"),
        }
    }
    undo_all(journal, &mut SystemRunner);
}

pub struct MacGuard {
    file: JournalFile,
    state: Arc<Mutex<Journal>>,
    stop: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
    local_dns: Option<LocalDns>,
}

impl Drop for MacGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
        drop(self.local_dns.take());
        let journal = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        undo(&journal);
        if let Err(error) = self.file.clear() {
            tracing::warn!(%error, "cannot remove the network journal");
        }
        tracing::info!("network settings restored");
    }
}

impl Platform for MacPlatform {
    type Device = AsyncDevice;
    type Guard = MacGuard;

    fn protect_socket(&self) -> ProtectSocket {
        protect(self.bound_interface.clone())
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
        if journal.is_empty() {
            return self.journal.clear();
        }
        tracing::warn!("restoring network settings from an earlier run");
        undo(&journal);
        self.journal.clear()
    }

    fn open(
        &self,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(AsyncDevice, MacGuard)> {
        self.recover()?;
        let primary = default_interface(None)?;
        self.bound_interface.store(primary, Ordering::Release);

        let device = DeviceBuilder::new()
            .ipv4(TUN_V4, 32, None)
            .ipv6(TUN_V6, 128)
            .mtu(request.mtu)
            .build_async()?;
        let tun_index = device.if_index()?;

        let state = Arc::new(Mutex::new(Journal::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = MacGuard {
            file: self.journal.clone(),
            state: state.clone(),
            stop: stop.clone(),
            monitor: None,
            local_dns: None,
        };

        progress(ConnectStep::Routes);
        let mut manager = RouteManager::new()?;
        for (destination, prefix) in split_routes() {
            {
                let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
                journal.add_route(RouteEntry {
                    destination,
                    prefix,
                    if_index: tun_index,
                });
                self.journal.save(&journal)?;
            }
            manager.add(&Route::new(destination, prefix).with_if_index(tun_index))?;
        }

        progress(ConnectStep::Dns);
        {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(
                &[loopback_alias()],
                &mut journal,
                &self.journal,
                &mut SystemRunner,
            )?;
        }
        guard.local_dns = Some(LocalDns::start(
            SocketAddr::new(LOCAL_DNS.into(), 53),
            request.dns.clone(),
            &tokio::runtime::Handle::try_current().map_err(io::Error::other)?,
        )?);
        Dns::open()?.apply(&self.journal, &state)?;

        let journal_file = self.journal.clone();
        let bound = self.bound_interface.clone();
        guard.monitor = Some(
            std::thread::Builder::new()
                .name("fiend-network-monitor".into())
                .spawn(move || {
                    monitor(journal_file, state, stop, bound, tun_index, network_changed)
                })?,
        );
        Ok((device, guard))
    }
}

fn monitor(
    file: JournalFile,
    state: Arc<Mutex<Journal>>,
    stop: Arc<AtomicBool>,
    bound: Arc<AtomicU32>,
    tun_index: u32,
    network_changed: Arc<dyn Fn() + Send + Sync>,
) {
    let dns = match Dns::open() {
        Ok(dns) => dns,
        Err(error) => {
            tracing::error!(%error, "network monitor cannot start");
            return;
        }
    };
    loop {
        let mut waited = Duration::ZERO;
        while waited < MONITOR_PERIOD {
            if stop.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
            waited += Duration::from_millis(100);
        }
        match default_interface(Some(tun_index)) {
            Ok(index) if index != bound.load(Ordering::Acquire) => {
                tracing::info!(interface = index, "primary network changed");
                bound.store(index, Ordering::Release);
                network_changed();
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "no primary network"),
        }
        match dns.apply(&file, &state) {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "DNS servers set again"),
            Err(error) => tracing::warn!(%error, "cannot set the DNS servers"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_routes_cover_both_families_without_a_default_route() {
        let routes = split_routes();
        assert_eq!(routes.iter().filter(|(ip, _)| ip.is_ipv4()).count(), 8);
        assert_eq!(routes.iter().filter(|(ip, _)| ip.is_ipv6()).count(), 8);
        assert!(routes.iter().all(|(ip, _)| !ip.is_unspecified()));
        let v4 = |ip: Ipv4Addr| {
            routes.iter().any(|(net, prefix)| match net {
                IpAddr::V4(net) => {
                    let mask = u32::MAX << (32 - u32::from(*prefix));
                    u32::from(ip) & mask == u32::from(*net)
                }
                IpAddr::V6(_) => false,
            })
        };
        let v6 = |ip: Ipv6Addr| {
            routes.iter().any(|(net, prefix)| match net {
                IpAddr::V6(net) => {
                    let mask = u128::MAX << (128 - u32::from(*prefix));
                    u128::from(ip) & mask == u128::from(*net)
                }
                IpAddr::V4(_) => false,
            })
        };
        for ip in ["1.0.0.1", "9.9.9.9", "127.0.0.1", "198.18.0.1", "255.255.255.254"] {
            assert!(v4(ip.parse().unwrap()), "{ip}");
        }
        assert!(!v4("0.1.2.3".parse().unwrap()));
        for ip in ["100::1", "2001:db8::1", "fd42::1", "ffff::1"] {
            assert!(v6(ip.parse().unwrap()), "{ip}");
        }
        assert!(!v6("::ffff:1.2.3.4".parse().unwrap()));
        let mut seen = std::collections::HashSet::new();
        assert!(routes.iter().all(|route| seen.insert(*route)));
    }

    #[test]
    fn protect_finds_the_primary_interface_before_the_tunnel_is_up() {
        let bound = Arc::new(AtomicU32::new(0));
        let hook = protect(bound.clone());
        let socket = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .unwrap();
        match default_interface(None) {
            Ok(index) => {
                hook(&socket).unwrap();
                assert_eq!(bound.load(Ordering::Acquire), index);
            }
            Err(_) => assert!(hook(&socket).is_err()),
        }
    }

    #[test]
    fn protect_binds_a_socket_to_the_loopback_interface() {
        let index = default_interface(None).unwrap_or(1);
        let hook = protect(Arc::new(AtomicU32::new(index)));
        let socket = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .unwrap();
        hook(&socket).unwrap();
    }

    #[test]
    fn primary_interface_is_found_on_a_connected_machine() {
        if let Ok(index) = default_interface(None) {
            assert!(index > 0);
            assert!(default_interface(Some(index)).map_or(true, |other| other != index));
        }
    }

    #[test]
    fn dns_records_survive_the_journal_format() {
        let dns = Dns::open().unwrap();
        let record = ours().to_CFPropertyList();
        let xml = plist::to_xml(&record).unwrap();
        let back = plist::from_xml(&xml).unwrap();
        assert_eq!(plist::to_xml(&back).unwrap(), xml);
        for key in dns.keys() {
            let value = dns.store.get(key.as_str()).unwrap();
            let xml = plist::to_xml(&value).unwrap();
            let back = plist::from_xml(&xml).unwrap();
            assert_eq!(plist::to_xml(&back).unwrap(), xml, "{key}");
        }
        assert!(plist::from_xml("not a plist").is_none());
    }

    #[test]
    fn dns_store_can_be_read_without_root() {
        let dns = Dns::open().unwrap();
        let _ = dns.keys();
    }
}
