use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};
use tokio::net::{TcpListener, TcpStream};

pub const DEFAULT_BACKLOG: i32 = 1024;
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
const KEEPALIVE_IDLE: Duration = Duration::from_secs(60);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEPALIVE_RETRIES: u32 = 4;

pub async fn accept(listener: &TcpListener) -> (TcpStream, SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(accepted) => return accepted,
            Err(error) if is_connection_error(&error) => {
                tracing::debug!(%error, "accepted connection failed");
            }
            Err(error) => {
                tracing::warn!(%error, "accept failed");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

pub fn set_tcp_keepalive<S>(stream: &S) -> io::Result<()>
where
    S: std::os::fd::AsFd,
{
    let socket = socket2::SockRef::from(stream);
    socket.set_keepalive(true)?;
    socket.set_tcp_keepalive(
        &TcpKeepalive::new()
            .with_time(KEEPALIVE_IDLE)
            .with_interval(KEEPALIVE_INTERVAL)
            .with_retries(KEEPALIVE_RETRIES),
    )
}

fn is_connection_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_tcp_keepalive() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let stream = std::net::TcpStream::connect(address).expect("connect");
        let _peer = listener.accept().expect("accept");
        set_tcp_keepalive(&stream).expect("set keepalive");
        assert!(socket2::SockRef::from(&stream).keepalive().expect("read"));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::super::*;

    #[test]
    fn sets_low_water_mark_on_loopback() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let stream = std::net::TcpStream::connect(address).expect("connect");
        let _peer = listener.accept().expect("accept");
        set_tcp_notsent_lowat(&stream, 64 << 10).expect("set option");
    }
}
