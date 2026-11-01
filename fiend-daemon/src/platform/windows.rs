#![allow(unsafe_code)]

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::os::windows::io::AsRawSocket;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use fiend_core::net::ProtectSocket;
use fiend_ipc::ConnectStep;
use tun_rs::{AsyncDevice, DeviceBuilder};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, FreeMibTable, GetIpForwardTable2, GetIpInterfaceEntry,
    InitializeIpInterfaceEntry, MIB_IPFORWARD_TABLE2, MIB_IPINTERFACE_ROW,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, SO_PROTOCOL_INFOW,
    SOCKET, SOL_SOCKET, WSAPROTOCOL_INFOW, getsockopt, setsockopt,
};

use super::commands::{Runner, Step, SystemRunner, apply_steps, undo_all};
use super::local_dns::LocalDns;
use super::split_routes;
use super::{NetworkRequest, Platform};
use crate::journal::{Journal, JournalFile};

const IFACE: &str = "Fiend";
const TUN_V4: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 2);
const TUN_V6: Ipv6Addr = Ipv6Addr::new(0xfd42, 0x6669, 0x656e, 0x6400, 0, 0, 0, 2);
const LOCAL_DNS: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
const NRPT_NAME: &str = "Fiend VPN";
const NRPT_KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig\{4669656E-6456-504E-0000-000000000002}";
const MONITOR_PERIOD: Duration = Duration::from_secs(2);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ADAPTER_GUID: u128 = 0x4669_656e_6456_504e_0000_0000_0000_0001;

pub struct WindowsPlatform {
    journal: JournalFile,
    bound_interface: Arc<AtomicU32>,
}

impl WindowsPlatform {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            journal: JournalFile::new(state_dir.join("network-journal.json")),
            bound_interface: Arc::new(AtomicU32::new(0)),
        }
    }
}

fn system_root() -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
}

fn netsh_path() -> String {
    system_root()
        .join(r"System32\netsh.exe")
        .to_string_lossy()
        .into_owned()
}

fn powershell_path() -> String {
    system_root()
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
        .to_string_lossy()
        .into_owned()
}

fn hidden(command: &mut Command) -> &mut Command {
    command.creation_flags(CREATE_NO_WINDOW)
}

fn netsh(arguments: &[&str]) -> Vec<String> {
    let mut argv = vec![netsh_path()];
    argv.extend(arguments.iter().map(|argument| (*argument).to_owned()));
    argv
}

fn run_powershell(script: &str) -> io::Result<String> {
    let output = hidden(&mut Command::new(powershell_path()))
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn interface_metric(family: u16, index: u32) -> u32 {
    let mut row: MIB_IPINTERFACE_ROW = unsafe { std::mem::zeroed() };
    unsafe { InitializeIpInterfaceEntry(&mut row) };
    row.Family = family;
    row.InterfaceIndex = index;
    if unsafe { GetIpInterfaceEntry(&mut row) } == 0 {
        row.Metric
    } else {
        0
    }
}

fn best_default(family: u16, exclude: u32) -> io::Result<Option<u32>> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = ptr::null_mut();
    let status = unsafe { GetIpForwardTable2(family, &mut table) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let mut best: Option<(u64, u32)> = None;
    {
        let table = unsafe { &*table };
        let rows =
            unsafe { std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize) };
        for row in rows {
            if row.DestinationPrefix.PrefixLength != 0 || row.InterfaceIndex == exclude {
                continue;
            }
            let metric =
                u64::from(row.Metric) + u64::from(interface_metric(family, row.InterfaceIndex));
            if best.is_none_or(|(lowest, _)| metric < lowest) {
                best = Some((metric, row.InterfaceIndex));
            }
        }
    }
    unsafe { FreeMibTable(table as *const _) };
    Ok(best.map(|(_, index)| index))
}

fn default_route(exclude: Option<u32>) -> io::Result<u32> {
    let exclude = exclude.unwrap_or(0);
    for family in [AF_INET, AF_INET6] {
        if let Some(index) = best_default(family, exclude)? {
            return Ok(index);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "There is no network connection",
    ))
}

fn bound_interface(bound: &AtomicU32) -> io::Result<NonZeroU32> {
    if let Some(index) = NonZeroU32::new(bound.load(Ordering::Acquire)) {
        return Ok(index);
    }
    NonZeroU32::new(default_route(None)?).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "There is no network connection",
        )
    })
}

fn address_family(socket: SOCKET) -> io::Result<i32> {
    let mut info: WSAPROTOCOL_INFOW = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<WSAPROTOCOL_INFOW>() as i32;
    let result = unsafe {
        getsockopt(
            socket,
            SOL_SOCKET,
            SO_PROTOCOL_INFOW,
            &mut info as *mut WSAPROTOCOL_INFOW as *mut u8,
            &mut length,
        )
    };
    if result == 0 {
        Ok(info.iAddressFamily)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn unicast_option(family: i32, index: u32) -> (i32, i32, u32) {
    if family == i32::from(AF_INET6) {
        (IPPROTO_IPV6, IPV6_UNICAST_IF, index)
    } else {
        (IPPROTO_IP, IP_UNICAST_IF, index.to_be())
    }
}

fn protect(bound: Arc<AtomicU32>) -> ProtectSocket {
    Arc::new(move |socket| {
        let index = bound_interface(&bound)?;
        let raw = socket.as_raw_socket() as SOCKET;
        let (level, option, value) = unicast_option(address_family(raw)?, index.get());
        let result = unsafe {
            setsockopt(
                raw,
                level,
                option,
                &value as *const u32 as *const u8,
                std::mem::size_of::<u32>() as i32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    })
}

fn split_routes_remain(alias: &str) -> bool {
    let wide: Vec<u16> = alias.encode_utf16().chain(std::iter::once(0)).collect();
    let mut luid: NET_LUID_LH = unsafe { std::mem::zeroed() };
    if unsafe { ConvertInterfaceAliasToLuid(wide.as_ptr(), &mut luid) } != 0 {
        return false;
    }
    let luid = unsafe { luid.Value };
    [AF_INET, AF_INET6].into_iter().any(|family| {
        let mut table: *mut MIB_IPFORWARD_TABLE2 = ptr::null_mut();
        if unsafe { GetIpForwardTable2(family, &mut table) } != 0 {
            return true;
        }
        let found = {
            let table = unsafe { &*table };
            let rows = unsafe {
                std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize)
            };
            rows.iter().any(|row| {
                let row_luid = unsafe { row.InterfaceLuid.Value };
                row_luid == luid && (1..=8).contains(&row.DestinationPrefix.PrefixLength)
            })
        };
        unsafe { FreeMibTable(table as *const _) };
        found
    })
}

fn route_undo_interface(argv: &[String]) -> Option<&str> {
    let program = argv.first()?;
    if !program.to_ascii_lowercase().ends_with("netsh.exe")
        || !argv.iter().any(|word| word == "delete")
        || !argv.iter().any(|word| word == "route")
    {
        return None;
    }
    argv.iter().find_map(|word| word.strip_prefix("interface="))
}

fn pending_undo(journal: &Journal, routes_remain: impl Fn(&str) -> bool) -> Journal {
    let mut journal = journal.clone();
    journal
        .undo
        .retain(|argv| route_undo_interface(argv).is_none_or(&routes_remain));
    journal
}

fn restore(journal: &Journal) {
    let pending = pending_undo(journal, split_routes_remain);
    let skipped = journal.undo.len() - pending.undo.len();
    if skipped > 0 {
        tracing::info!(skipped, "the tunnel routes are already removed");
    }
    undo_all(&pending, &mut SystemRunner);
}

fn route_steps(iface: &str) -> Vec<Step> {
    let interface = format!("interface={iface}");
    split_routes()
        .into_iter()
        .map(|(address, prefix)| {
            let destination = format!("prefix={address}/{prefix}");
            let family = if address.is_ipv4() { "ipv4" } else { "ipv6" };
            Step {
                apply: netsh(&[
                    "interface",
                    family,
                    "add",
                    "route",
                    &destination,
                    &interface,
                    "metric=1",
                    "store=active",
                ]),
                undo: Some(netsh(&[
                    "interface",
                    family,
                    "delete",
                    "route",
                    &destination,
                    &interface,
                    "store=active",
                ])),
            }
        })
        .collect()
}

fn nrpt_remove_script() -> String {
    format!(
        "$d='{NRPT_NAME}'; \
         Get-DnsClientNrptRule -ErrorAction SilentlyContinue | \
           Where-Object {{ $_.DisplayName -eq $d }} | \
           Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue"
    )
}

pub fn remove_nrpt() -> io::Result<()> {
    let _ = SystemRunner.run(&nrpt_delete());
    run_powershell(&nrpt_remove_script()).map(|_| ())
}

fn reg_path() -> String {
    system_root()
        .join(r"System32\reg.exe")
        .to_string_lossy()
        .into_owned()
}

fn nrpt_delete() -> Vec<String> {
    vec![reg_path(), "delete".into(), NRPT_KEY.into(), "/f".into()]
}

fn nrpt_steps() -> Vec<Step> {
    let servers = LOCAL_DNS.to_string();
    let values = [
        ("Version", "REG_DWORD", "2"),
        ("ConfigOptions", "REG_DWORD", "8"),
        ("GenericDNSServers", "REG_SZ", servers.as_str()),
        ("DisplayName", "REG_SZ", NRPT_NAME),
        ("Name", "REG_MULTI_SZ", "."),
    ];
    values
        .iter()
        .enumerate()
        .map(|(index, (name, kind, data))| Step {
            apply: [
                reg_path().as_str(),
                "add",
                NRPT_KEY,
                "/v",
                name,
                "/t",
                kind,
                "/d",
                data,
                "/f",
            ]
            .map(str::to_owned)
            .to_vec(),
            undo: (index == 0).then(nrpt_delete),
        })
        .collect()
}

pub struct WindowsGuard {
    file: JournalFile,
    state: Arc<Mutex<Journal>>,
    stop: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
    local_dns: Option<LocalDns>,
    bound: Arc<AtomicU32>,
}

impl Drop for WindowsGuard {
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
        restore(&journal);
        if let Err(error) = self.file.clear() {
            tracing::warn!(%error, "cannot remove the network journal");
        }
        self.bound.store(0, Ordering::Release);
        tracing::info!("network settings restored");
    }
}

impl Platform for WindowsPlatform {
    type Device = AsyncDevice;
    type Guard = WindowsGuard;

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
        restore(&journal);
        self.journal.clear()
    }

    fn open(
        &self,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(AsyncDevice, WindowsGuard)> {
        self.recover()?;
        let primary = default_route(None)?;

        let wintun = crate::windows::source_wintun()
            .unwrap_or_else(crate::windows::wintun_path)
            .to_string_lossy()
            .into_owned();
        let device = DeviceBuilder::new()
            .name(IFACE)
            .description("Fiend VPN")
            .device_guid(ADAPTER_GUID)
            .wintun_file(wintun)
            .ipv4(TUN_V4, 32, None)
            .ipv6(TUN_V6, 128)
            .mtu(request.mtu)
            .metric(1)
            .build_async()?;
        let tun_index = device.if_index()?;
        let iface = device.name().unwrap_or_else(|_| IFACE.to_owned());

        let state = Arc::new(Mutex::new(Journal::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = WindowsGuard {
            file: self.journal.clone(),
            state: state.clone(),
            stop: stop.clone(),
            monitor: None,
            local_dns: None,
            bound: self.bound_interface.clone(),
        };
        self.bound_interface.store(primary, Ordering::Release);

        progress(ConnectStep::Routes);
        {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(
                &route_steps(&iface),
                &mut journal,
                &self.journal,
                &mut SystemRunner,
            )?;
        }

        progress(ConnectStep::Dns);
        guard.local_dns = Some(LocalDns::start(
            SocketAddr::new(LOCAL_DNS.into(), 53),
            request.dns.clone(),
            &tokio::runtime::Handle::try_current().map_err(io::Error::other)?,
        )?);
        device.set_dns_servers(&[IpAddr::V4(LOCAL_DNS)])?;
        {
            let mut journal = state.lock().unwrap_or_else(|error| error.into_inner());
            apply_steps(
                &nrpt_steps(),
                &mut journal,
                &self.journal,
                &mut SystemRunner,
            )?;
        }

        let bound = self.bound_interface.clone();
        guard.monitor = Some(
            std::thread::Builder::new()
                .name("fiend-network-monitor".into())
                .spawn(move || {
                    loop {
                        let mut waited = Duration::ZERO;
                        while waited < MONITOR_PERIOD {
                            if stop.load(Ordering::Acquire) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                            waited += Duration::from_millis(100);
                        }
                        match default_route(Some(tun_index)) {
                            Ok(index) if index != bound.load(Ordering::Acquire) => {
                                tracing::info!(interface = index, "primary network changed");
                                bound.store(index, Ordering::Release);
                                network_changed();
                            }
                            Ok(_) => {}
                            Err(error) => tracing::warn!(%error, "no primary network"),
                        }
                    }
                })?,
        );
        Ok((device, guard))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use socket2::{Domain, Socket, Type};

    #[test]
    fn the_interface_index_byte_order_depends_on_the_family() {
        let (_, _, v4) = unicast_option(i32::from(AF_INET), 0x0102_0304);
        let (_, _, v6) = unicast_option(i32::from(AF_INET6), 0x0102_0304);
        assert_eq!(v4, 0x0102_0304u32.to_be());
        assert_eq!(v6, 0x0102_0304);
    }

    #[test]
    fn protect_accepts_unbound_sockets_of_both_families() {
        let protect = protect(Arc::new(AtomicU32::new(1)));
        for domain in [Domain::IPV4, Domain::IPV6] {
            let Ok(socket) = Socket::new(domain, Type::STREAM, None) else {
                continue;
            };
            protect(&socket).unwrap();
        }
    }

    #[test]
    fn the_default_route_lookup_does_not_fail_in_an_unexpected_way() {
        match default_route(None) {
            Ok(index) => assert_ne!(index, 0),
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotConnected),
        }
    }

    #[test]
    fn split_routes_create_sixteen_netsh_steps() {
        let steps = route_steps(IFACE);
        assert_eq!(steps.len(), 16);
        assert!(steps.iter().all(|step| step.undo.is_some()));
        assert_eq!(
            steps
                .iter()
                .filter(|step| step.apply.iter().any(|word| word == "ipv4"))
                .count(),
            8
        );
        assert!(
            steps
                .iter()
                .all(|step| step.apply.iter().any(|word| word == "store=active"))
        );
    }

    #[test]
    fn route_undo_is_skipped_only_when_its_interface_is_gone() {
        let mut journal = Journal::default();
        journal
            .undo
            .extend(route_steps(IFACE).into_iter().filter_map(|step| step.undo));
        journal.undo.push(nrpt_delete());
        let kept = pending_undo(&journal, |_| true);
        assert_eq!(kept.undo.len(), 17);
        let kept = pending_undo(&journal, |_| false);
        assert_eq!(kept.undo, vec![nrpt_delete()]);
        assert!(!split_routes_remain("fiend-no-such-interface"));
    }

    #[test]
    fn the_nrpt_rule_uses_the_local_resolver() {
        let steps = nrpt_steps();
        let text: Vec<String> = steps.iter().map(|step| step.apply.join(" ")).collect();
        assert!(text.iter().all(|line| line.contains(NRPT_KEY)));
        assert!(
            text.iter()
                .any(|line| line.contains("/v GenericDNSServers /t REG_SZ /d 127.0.0.1"))
        );
        assert!(
            text.iter()
                .any(|line| line.contains("/v ConfigOptions /t REG_DWORD /d 8"))
        );
        assert!(
            text.last()
                .is_some_and(|line| line.contains("/v Name /t REG_MULTI_SZ /d ."))
        );
        assert_eq!(steps[0].undo.as_deref(), Some(nrpt_delete().as_slice()));
        assert!(steps[1..].iter().all(|step| step.undo.is_none()));
    }
}
