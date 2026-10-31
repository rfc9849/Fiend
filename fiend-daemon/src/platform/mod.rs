use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

pub mod commands;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod local_dns;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use fiend_core::net::ProtectSocket;
use fiend_ipc::ConnectStep;
use fiend_tun::PacketIo;

pub type DnsAnswer = Arc<dyn Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync>;

#[derive(Clone)]
pub struct NetworkRequest {
    pub servers: Vec<SocketAddr>,
    pub mtu: u16,
    pub ipv6: bool,
    pub bypass_lan: bool,
    pub dns: DnsAnswer,
}

pub trait Platform: Send + Sync + 'static {
    type Device: PacketIo;
    type Guard: Send + 'static;

    fn protect_socket(&self) -> ProtectSocket;

    /// Create the TUN device and apply routes and DNS. The returned guard
    /// restores the old network setup when it is dropped. `network_changed`
    /// runs when the primary network changes while the guard is alive.
    fn open(
        &self,
        request: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        network_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(Self::Device, Self::Guard)>;

    /// Undo changes that an earlier daemon process left after a crash.
    fn recover(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Engine only: no TUN device, no routes, no DNS change. For tests and for
/// checks that need no administrator rights.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullPlatform;

pub struct NullDevice;

impl PacketIo for NullDevice {
    async fn recv(&self, _: &mut [u8]) -> io::Result<usize> {
        std::future::pending().await
    }

    async fn send(&self, packet: &[u8]) -> io::Result<usize> {
        Ok(packet.len())
    }
}

impl Platform for NullPlatform {
    type Device = NullDevice;
    type Guard = ();

    fn protect_socket(&self) -> ProtectSocket {
        std::sync::Arc::new(|_| Ok(()))
    }

    fn open(
        &self,
        _: &NetworkRequest,
        progress: &dyn Fn(ConnectStep),
        _: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(NullDevice, ())> {
        progress(ConnectStep::Routes);
        progress(ConnectStep::Dns);
        Ok((NullDevice, ()))
    }
}

/// The network setup of this system. The system code is not written yet, so
/// it refuses to connect.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedPlatform;

impl Platform for UnsupportedPlatform {
    type Device = NullDevice;
    type Guard = ();

    fn protect_socket(&self) -> ProtectSocket {
        std::sync::Arc::new(|_| Ok(()))
    }

    fn open(
        &self,
        _: &NetworkRequest,
        _: &dyn Fn(ConnectStep),
        _: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(NullDevice, ())> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Network setup is not available on this system yet",
        ))
    }
}

fn split_routes() -> Vec<(IpAddr, u8)> {
    let mut routes = Vec::with_capacity(16);
    for prefix in 1..=8u8 {
        let first = if prefix == 1 {
            128
        } else {
            1u8 << (8 - prefix)
        };
        routes.push((IpAddr::V4(Ipv4Addr::new(first, 0, 0, 0)), prefix));
        let first = if prefix == 1 {
            0x8000
        } else {
            1u16 << (16 - prefix)
        };
        routes.push((
            IpAddr::V6(Ipv6Addr::new(first, 0, 0, 0, 0, 0, 0, 0)),
            prefix,
        ));
    }
    routes
}

#[cfg(target_os = "macos")]
pub type Native = macos::MacPlatform;
#[cfg(target_os = "linux")]
pub type Native = linux::LinuxPlatform;
#[cfg(target_os = "windows")]
pub type Native = windows::WindowsPlatform;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub type Native = UnsupportedPlatform;

#[cfg(target_os = "macos")]
pub fn native(state_dir: &Path) -> Native {
    macos::MacPlatform::new(state_dir)
}

#[cfg(target_os = "linux")]
pub fn native(state_dir: &Path) -> Native {
    linux::LinuxPlatform::new(state_dir)
}

#[cfg(target_os = "windows")]
pub fn native(state_dir: &Path) -> Native {
    windows::WindowsPlatform::new(state_dir)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn native(_: &Path) -> Native {
    UnsupportedPlatform
}
