use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};
use tokio::net::{TcpListener, TcpSocket, TcpStream, lookup_host};

pub type ProtectSocket = std::sync::Arc<dyn Fn(&Socket) -> io::Result<()> + Send + Sync>;

pub const DEFAULT_TCP_NOTSENT_LOWAT: u32 = 128 << 10;

#[doc(hidden)]
pub trait AsSocketRef {
    fn socket_ref(&self) -> socket2::SockRef<'_>;
}

#[cfg(unix)]
impl<T: std::os::fd::AsFd> AsSocketRef for T {
    fn socket_ref(&self) -> socket2::SockRef<'_> {
        socket2::SockRef::from(self)
    }
}

#[cfg(windows)]
impl<T: std::os::windows::io::AsSocket> AsSocketRef for T {
    fn socket_ref(&self) -> socket2::SockRef<'_> {
        socket2::SockRef::from(self)
    }
}

pub const CONNECT_ATTEMPTS: usize = 3;
pub const CONNECT_ATTEMPT_DELAY: Duration = Duration::from_millis(500);

pub async fn dial(server: &str, protect: &ProtectSocket) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "server has no addresses");
    for address in lookup_host(server).await? {
        let protect = protect.clone();
        let attempt = move || connect_once(address, protect.clone());
        match race_connects(CONNECT_ATTEMPTS, CONNECT_ATTEMPT_DELAY, attempt).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last = error,
        }
    }
    Err(last)
}

async fn connect_once(address: SocketAddr, protect: ProtectSocket) -> io::Result<TcpStream> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.set_nonblocking(true)?;
    protect(&socket)?;
    TcpSocket::from_std_stream(socket.into())
        .connect(address)
        .await
}

async fn race_connects<T, F, Fut>(attempts: usize, delay: Duration, mut start: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = io::Result<T>> + Send + 'static,
{
    let attempts = attempts.max(1);
    let mut running = tokio::task::JoinSet::new();
    let mut started = 0;
    let mut last = None;
    loop {
        if started < attempts && running.is_empty() {
            running.spawn(start());
            started += 1;
        }
        let next = tokio::time::sleep(delay);
        tokio::pin!(next);
        tokio::select! {
            joined = running.join_next(), if !running.is_empty() => match joined {
                Some(Ok(Ok(stream))) => {
                    if started > 1 {
                        tracing::debug!(attempt = started, "TCP connect won after a retry");
                    }
                    return Ok(stream);
                }
                Some(Ok(Err(error))) => last = Some(error),
                Some(Err(error)) => last = Some(io::Error::other(error)),
                None => {}
            },
            _ = &mut next, if started < attempts => {
                running.spawn(start());
                started += 1;
            }
        }
        if running.is_empty() && started >= attempts {
            return Err(last.unwrap_or_else(|| io::Error::other("connect failed")));
        }
    }
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
    S: AsSocketRef,
{
    let socket = stream.socket_ref();
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

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn ack_probe(stream: &TcpStream) -> Option<AckProbe> {
    use std::os::fd::{AsFd, AsRawFd};
    let fd = stream.as_fd().try_clone_to_owned().ok()?;
    bytes_acked(fd.as_raw_fd())?;
    Some(Box::new(move || bytes_acked(fd.as_raw_fd())))
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn ack_probe(_stream: &TcpStream) -> Option<AckProbe> {
    None
}

pub type AckProbe = Box<dyn Fn() -> Option<u64> + Send + Sync>;

#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(unsafe_code)]
fn bytes_acked(fd: std::os::fd::RawFd) -> Option<u64> {
    const BYTES_ACKED: std::ops::Range<usize> = 120..128;
    let mut info = [0u8; 256];
    let mut length = info.len() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_INFO,
            info.as_mut_ptr().cast(),
            &mut length,
        )
    };
    if result != 0 || (length as usize) < BYTES_ACKED.end {
        return None;
    }
    Some(u64::from_ne_bytes(info[BYTES_ACKED].try_into().ok()?))
}

#[cfg(not(target_os = "linux"))]
pub fn set_tcp_notsent_lowat<S>(_stream: &S, bytes: u32) -> io::Result<()> {
    let _ = bytes;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type Attempt = std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<u32>> + Send>>;

    fn scripted(
        script: Vec<Option<io::Result<u32>>>,
    ) -> (
        impl FnMut() -> Attempt,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let script = std::sync::Arc::new(std::sync::Mutex::new(script.into_iter()));
        let start = move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let step = script.lock().expect("script").next().flatten();
            Box::pin(async move {
                match step {
                    Some(result) => result,
                    None => std::future::pending().await,
                }
            }) as Attempt
        };
        (start, calls)
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_connect_is_raced_after_the_delay() {
        let (start, calls) = scripted(vec![None, Some(Ok(2))]);
        let began = tokio::time::Instant::now();
        let won = race_connects(3, CONNECT_ATTEMPT_DELAY, start)
            .await
            .expect("second attempt");
        assert_eq!(won, 2);
        assert_eq!(began.elapsed(), CONNECT_ATTEMPT_DELAY);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_fast_connect_starts_no_second_attempt() {
        let (start, calls) = scripted(vec![Some(Ok(1))]);
        assert_eq!(
            race_connects(3, CONNECT_ATTEMPT_DELAY, start)
                .await
                .expect("first attempt"),
            1
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_connect_retries_at_once() {
        let (start, calls) = scripted(vec![
            Some(Err(io::ErrorKind::ConnectionRefused.into())),
            Some(Ok(2)),
        ]);
        let began = tokio::time::Instant::now();
        assert_eq!(
            race_connects(3, CONNECT_ATTEMPT_DELAY, start)
                .await
                .expect("retry"),
            2
        );
        assert!(began.elapsed() < CONNECT_ATTEMPT_DELAY);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn all_failed_connects_return_the_last_error() {
        let (start, calls) = scripted(vec![
            Some(Err(io::ErrorKind::ConnectionRefused.into())),
            Some(Err(io::ErrorKind::ConnectionRefused.into())),
            Some(Err(io::ErrorKind::ConnectionReset.into())),
        ]);
        let error = race_connects(3, CONNECT_ATTEMPT_DELAY, start)
            .await
            .expect_err("all attempts fail");
        assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn attempts_stop_at_the_limit() {
        let (start, calls) = scripted(vec![None, None, None, Some(Ok(4))]);
        let result = tokio::time::timeout(
            CONNECT_ATTEMPT_DELAY * 10,
            race_connects(3, CONNECT_ATTEMPT_DELAY, start),
        )
        .await;
        assert!(
            result.is_err(),
            "stalled attempts keep waiting for the caller timeout"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn dials_a_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address").to_string();
        let protect: ProtectSocket = std::sync::Arc::new(|_| Ok(()));
        let (dialed, accepted) = tokio::join!(dial(&address, &protect), listener.accept());
        let dialed = dialed.expect("dial");
        let (_, peer) = accepted.expect("accept");
        assert_eq!(dialed.local_addr().expect("local"), peer);
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
