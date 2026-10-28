use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};
use tokio::net::{TcpListener, TcpSocket, TcpStream, lookup_host};

pub type ProtectSocket = std::sync::Arc<dyn Fn(&Socket) -> io::Result<()> + Send + Sync>;

/// Per-link socket buffer. A link carries every lane of a session, so its
/// window must cover the bandwidth delay product of a long path. The system
/// default is smaller and limits one link to a few hundred Mbit/s.
pub const LINK_BUFFER_BYTES: usize = 4 << 20;

/// An explicit socket buffer turns off kernel autotuning, and the kernel caps
/// it at `net.core.{r,w}mem_max`. Some systems cap that below the autotuning
/// ceiling, for example an Android emulator with 256 KiB, so a fixed size
/// would make the window smaller. Set the size only when the cap can hold it.
fn buffer_target(hard_cap: Option<usize>) -> Option<usize> {
    let hard_cap = hard_cap?;
    (hard_cap >= LINK_BUFFER_BYTES).then_some(LINK_BUFFER_BYTES)
}

#[cfg(target_os = "linux")]
fn hard_cap(recv: bool) -> Option<usize> {
    let path = if recv {
        "/proc/sys/net/core/rmem_max"
    } else {
        "/proc/sys/net/core/wmem_max"
    };
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(not(target_os = "linux"))]
fn hard_cap(_recv: bool) -> Option<usize> {
    None
}

pub fn set_link_buffers<S>(stream: &S) -> io::Result<()>
where
    S: std::os::fd::AsFd,
{
    let socket = socket2::SockRef::from(stream);
    if let Some(size) = buffer_target(hard_cap(true)) {
        socket.set_recv_buffer_size(size)?;
    }
    if let Some(size) = buffer_target(hard_cap(false)) {
        socket.set_send_buffer_size(size)?;
    }
    Ok(())
}

pub async fn dial(server: &str, protect: &ProtectSocket) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "server has no addresses");
    for address in lookup_host(server).await? {
        let socket = Socket::new(
            Domain::for_address(address),
            Type::STREAM,
            Some(Protocol::TCP),
        )?;
        socket.set_nonblocking(true)?;
        if let Err(error) = set_link_buffers(&socket) {
            tracing::debug!(%error, "cannot set link buffers");
        }
        protect(&socket)?;
        match TcpSocket::from_std_stream(socket.into())
            .connect(address)
            .await
        {
            Ok(stream) => return Ok(stream),
            Err(error) => last = error,
        }
    }
    Err(last)
}

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
    fn sets_link_buffers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let stream = std::net::TcpStream::connect(address).expect("connect");
        let _peer = listener.accept().expect("accept");
        set_link_buffers(&stream).expect("set buffers");
        let socket = socket2::SockRef::from(&stream);
        if buffer_target(hard_cap(true)).is_some() {
            assert!(socket.recv_buffer_size().expect("read receive") >= LINK_BUFFER_BYTES);
        }
        if buffer_target(hard_cap(false)).is_some() {
            assert!(socket.send_buffer_size().expect("read send") >= LINK_BUFFER_BYTES);
        }
    }

    #[test]
    fn keeps_autotuning_below_the_hard_cap() {
        assert_eq!(buffer_target(None), None);
        assert_eq!(buffer_target(Some(LINK_BUFFER_BYTES - 1)), None);
        assert_eq!(
            buffer_target(Some(LINK_BUFFER_BYTES)),
            Some(LINK_BUFFER_BYTES)
        );
    }

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
    use super::*;

    #[test]
    fn sets_low_water_mark_on_loopback() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let stream = std::net::TcpStream::connect(address).expect("connect");
        let _peer = listener.accept().expect("accept");
        set_tcp_notsent_lowat(&stream, 64 << 10).expect("set option");
        assert_eq!(
            socket2::SockRef::from(&stream)
                .tcp_notsent_lowat()
                .expect("read option"),
            64 << 10
        );
    }
}
