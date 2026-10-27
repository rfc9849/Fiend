use std::io;
use std::net::SocketAddr;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;

pub const DEFAULT_BACKLOG: i32 = 1024;

pub fn bind_listener(address: SocketAddr) -> io::Result<TcpListener> {
    let domain = if address.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    socket.listen(DEFAULT_BACKLOG)?;
    TcpListener::from_std(socket.into())
}

#[cfg(target_os = "linux")]
pub fn set_tcp_notsent_lowat<S>(stream: &S, bytes: u32) -> io::Result<()>
where
    S: std::os::fd::AsFd,
{
    if bytes == 0 {
        return Ok(());
    }
    socket2::SockRef::from(stream).set_tcp_notsent_lowat(bytes)
}

#[cfg(not(target_os = "linux"))]
pub fn set_tcp_notsent_lowat<S>(_stream: &S, bytes: u32) -> io::Result<()> {
    let _ = bytes;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn sets_low_water_mark_on_loopback() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let stream = std::net::TcpStream::connect(address).expect("connect");
        let _peer = listener.accept().expect("accept");
        set_tcp_notsent_lowat(&stream, 64 << 10).expect("set option");
    }
}
