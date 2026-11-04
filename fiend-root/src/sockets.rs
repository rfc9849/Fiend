//! Transparent sockets for TPROXY. They need `CAP_NET_ADMIN`.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nix::sys::socket::{setsockopt, sockopt};
use socket2::{Domain, Protocol, Socket, Type};

const BACKLOG: i32 = 1024;

/// A port in 20000..60000 that changes per call, so that the listener port
/// is not a fixed sign of Fiend.
pub fn random_port() -> u16 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos() as u64)
        .unwrap_or(0);
    let mut seed = nanos
        ^ (u64::from(std::process::id()) << 32)
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
    seed ^= seed >> 33;
    seed = seed.wrapping_mul(0xff51_afd7_ed55_8ccd);
    seed ^= seed >> 33;
    20000 + (seed % 40000) as u16
}

fn transparent(socket: &Socket, ipv6: bool) -> io::Result<()> {
    socket.set_reuse_address(true)?;
    if ipv6 {
        socket.set_only_v6(true)?;
        socket.set_ip_transparent_v6(true)
    } else {
        socket.set_ip_transparent_v4(true)
    }
}

/// Listeners on the loopback addresses at `port`: TCP listeners first, then
/// UDP sockets that report the original destination. With `mark`, the
/// packets of these sockets and of their accepted streams carry it, so that
/// rules can skip the replies to other hosts.
pub fn listeners(
    port: u16,
    ipv6: bool,
    mark: Option<u32>,
) -> io::Result<(Vec<Socket>, Vec<Socket>)> {
    let mut loopbacks = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    if ipv6 {
        loopbacks.push(IpAddr::V6(Ipv6Addr::LOCALHOST));
    }
    let mut tcp = Vec::new();
    let mut udp = Vec::new();
    for ip in loopbacks {
        let address = SocketAddr::new(ip, port);
        let v6 = ip.is_ipv6();
        let listener = Socket::new(
            Domain::for_address(address),
            Type::STREAM,
            Some(Protocol::TCP),
        )?;
        transparent(&listener, v6)?;
        if let Some(mark) = mark {
            listener.set_mark(mark)?;
        }
        listener.bind(&address.into())?;
        listener.listen(BACKLOG)?;
        tcp.push(listener);
        let receiver = Socket::new(
            Domain::for_address(address),
            Type::DGRAM,
            Some(Protocol::UDP),
        )?;
        transparent(&receiver, v6)?;
        if let Some(mark) = mark {
            receiver.set_mark(mark)?;
        }
        if v6 {
            setsockopt(&receiver, sockopt::Ipv6OrigDstAddr, &true)?;
        } else {
            setsockopt(&receiver, sockopt::Ipv4OrigDstAddr, &true)?;
        }
        receiver.bind(&address.into())?;
        udp.push(receiver);
    }
    Ok((tcp, udp))
}

/// A UDP socket bound to the foreign `address`, to send replies from it.
pub fn reply(address: SocketAddr, mark: Option<u32>) -> io::Result<Socket> {
    if address.ip().is_loopback() || address.ip().is_unspecified() || address.port() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reply address is not valid",
        ));
    }
    let socket = Socket::new(
        Domain::for_address(address),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    transparent(&socket, address.is_ipv6())?;
    if let Some(mark) = mark {
        socket.set_mark(mark)?;
    }
    socket.bind(&address.into())?;
    Ok(socket)
}
