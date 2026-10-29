use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::DnsAnswer;

const MAX_QUERY: usize = 4096;
const MAX_TCP_CLIENTS: usize = 32;
const TCP_IDLE: Duration = Duration::from_secs(10);
const ERROR_PAUSE: Duration = Duration::from_millis(50);

pub struct LocalDns {
    cancel: CancellationToken,
    address: SocketAddr,
}

impl LocalDns {
    pub fn start(address: SocketAddr, answer: DnsAnswer, runtime: &Handle) -> io::Result<Self> {
        let _context = runtime.enter();
        let udp = UdpSocket::from_std(bind(address, Type::DGRAM, Protocol::UDP)?.into())?;
        let address = udp.local_addr()?;
        let tcp = bind(address, Type::STREAM, Protocol::TCP)?;
        tcp.listen(128)?;
        let tcp = TcpListener::from_std(tcp.into())?;
        let cancel = CancellationToken::new();
        runtime.spawn(serve_udp(udp, answer.clone(), cancel.clone()));
        runtime.spawn(serve_tcp(tcp, answer, cancel.clone()));
        tracing::info!(%address, "local DNS started");
        Ok(Self { cancel, address })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for LocalDns {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn bind(address: SocketAddr, kind: Type, protocol: Protocol) -> io::Result<Socket> {
    let socket = Socket::new(Domain::for_address(address), kind, Some(protocol))?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    Ok(socket)
}

async fn serve_udp(socket: UdpSocket, answer: DnsAnswer, cancel: CancellationToken) {
    let mut buffer = vec![0; MAX_QUERY];
    loop {
        let received = tokio::select! {
            _ = cancel.cancelled() => return,
            received = socket.recv_from(&mut buffer) => received,
        };
        let (size, peer) = match received {
            Ok(received) => received,
            Err(error) => {
                tracing::debug!(%error, "local DNS receive failed");
                sleep(ERROR_PAUSE).await;
                continue;
            }
        };
        if let Some(reply) = answer(&buffer[..size])
            && let Err(error) = socket.send_to(&reply, peer).await
        {
            tracing::debug!(%error, "local DNS send failed");
        }
    }
}

async fn serve_tcp(listener: TcpListener, answer: DnsAnswer, cancel: CancellationToken) {
    let clients = Arc::new(Semaphore::new(MAX_TCP_CLIENTS));
    loop {
        let permit = tokio::select! {
            _ = cancel.cancelled() => return,
            permit = clients.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => return,
            },
        };
        let accepted = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let stream = match accepted {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::debug!(%error, "local DNS accept failed");
                sleep(ERROR_PAUSE).await;
                continue;
            }
        };
        let answer = answer.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _permit = permit;
            tokio::select! {
                _ = cancel.cancelled() => {}
                result = serve_stream(stream, &answer) => {
                    if let Err(error) = result {
                        tracing::debug!(%error, "local DNS connection closed");
                    }
                }
            }
        });
    }
}

async fn serve_stream(mut stream: TcpStream, answer: &DnsAnswer) -> io::Result<()> {
    loop {
        let length = usize::from(timeout(TCP_IDLE, stream.read_u16()).await??);
        if length == 0 || length > MAX_QUERY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DNS query size is not valid",
            ));
        }
        let mut query = vec![0; length];
        timeout(TCP_IDLE, stream.read_exact(&mut query)).await??;
        let Some(reply) = answer(&query) else {
            return Ok(());
        };
        let length = u16::try_from(reply.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "DNS answer is too large"))?;
        stream.write_u16(length).await?;
        stream.write_all(&reply).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo() -> DnsAnswer {
        Arc::new(|query: &[u8]| {
            if query == b"drop" {
                None
            } else {
                let mut reply = b"re:".to_vec();
                reply.extend_from_slice(query);
                Some(reply)
            }
        })
    }

    #[tokio::test]
    async fn answers_over_udp_and_tcp_until_dropped() {
        let dns = LocalDns::start("127.0.0.1:0".parse().unwrap(), echo(), &Handle::current())
            .unwrap();
        let address = dns.address();

        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(b"one", address).await.unwrap();
        let mut buffer = [0; 64];
        let size = timeout(Duration::from_secs(2), client.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buffer[..size], b"re:one");

        let mut stream = TcpStream::connect(address).await.unwrap();
        for query in [&b"two"[..], b"three"] {
            stream.write_u16(query.len() as u16).await.unwrap();
            stream.write_all(query).await.unwrap();
            let length = stream.read_u16().await.unwrap();
            let mut reply = vec![0; usize::from(length)];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply[3..], query);
        }
        stream.write_u16(4).await.unwrap();
        stream.write_all(b"drop").await.unwrap();
        assert_eq!(stream.read(&mut buffer).await.unwrap(), 0);

        drop(dns);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(TcpStream::connect(address).await.is_err());
    }
}
