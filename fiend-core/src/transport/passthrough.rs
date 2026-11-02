use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use super::error::{Error, Result};
use crate::proxy::Endpoints;

const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3600);
const COPY_BUFFER: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginTarget {
    pub host: String,
    pub port: u16,
    pub proxy_protocol: bool,
}

impl OriginTarget {
    pub fn direct(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            proxy_protocol: false,
        }
    }

    pub fn for_host(fallbacks: &[Fallback], host: &str, port: u16) -> Self {
        fallbacks
            .iter()
            .find(|fallback| fallback.host.eq_ignore_ascii_case(host))
            .map(|fallback| fallback.origin.clone())
            .unwrap_or_else(|| Self::direct(host, port))
    }

    pub(crate) async fn connect(&self, client: Option<&Endpoints>) -> Result<TcpStream> {
        let mut stream = tokio::time::timeout(
            DIAL_TIMEOUT,
            TcpStream::connect((self.host.as_str(), self.port)),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        if self.proxy_protocol {
            let header = client
                .map(Endpoints::header)
                .unwrap_or_else(crate::proxy::local_header);
            tokio::time::timeout(DIAL_TIMEOUT, stream.write_all(&header))
                .await
                .map_err(|_| Error::Timeout)??;
        }
        Ok(stream)
    }
}

impl std::fmt::Display for OriginTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.contains(':') {
            write!(formatter, "[{}]:{}", self.host, self.port)
        } else {
            write!(formatter, "{}:{}", self.host, self.port)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    pub host: String,
    pub origin: OriginTarget,
}

async fn dial_origin(target: &OriginTarget, client: Option<&Endpoints>) -> Result<TcpStream> {
    target.connect(client).await
}

pub struct EarlyOrigin {
    task: Mutex<Option<JoinHandle<Result<TcpStream>>>>,
    started: Arc<AtomicBool>,
    target: OriginTarget,
    client: Option<Endpoints>,
}

impl EarlyOrigin {
    pub fn spawn(target: OriginTarget, client: Option<Endpoints>, delay: Duration) -> Self {
        let started = Arc::new(AtomicBool::new(false));
        let task = {
            let started = started.clone();
            let target = target.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                started.store(true, Ordering::Release);
                dial_origin(&target, client.as_ref()).await
            })
        };
        Self {
            task: Mutex::new(Some(task)),
            started,
            target,
            client,
        }
    }

    fn take(&self) -> Option<JoinHandle<Result<TcpStream>>> {
        self.task
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }

    pub fn cancel(&self) {
        if let Some(task) = self.take() {
            task.abort();
        }
    }

    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    pub async fn connect(self) -> Result<TcpStream> {
        match self.take() {
            Some(task) if self.is_started() || task.is_finished() => match task.await {
                Ok(result) => result,
                Err(_) => dial_origin(&self.target, self.client.as_ref()).await,
            },
            Some(task) => {
                task.abort();
                dial_origin(&self.target, self.client.as_ref()).await
            }
            None => dial_origin(&self.target, self.client.as_ref()).await,
        }
    }
}

impl Drop for EarlyOrigin {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub async fn bridge_passthrough<S>(
    stream: S,
    buffered: Vec<u8>,
    host: &str,
    port: u16,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let origin = dial_origin(&OriginTarget::direct(host, port), None).await?;
    bridge_passthrough_to(stream, buffered, origin).await
}

pub async fn bridge_passthrough_to<S>(
    stream: S,
    buffered: Vec<u8>,
    mut origin: TcpStream,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if !buffered.is_empty() {
        origin.write_all(&buffered).await?;
    }

    let started = Instant::now();
    let activity = Arc::new(AtomicU64::new(0));
    let (mut client_read, mut client_write) = tokio::io::split(stream);
    let (mut origin_read, mut origin_write) = origin.into_split();

    let client_activity = Arc::clone(&activity);
    let origin_activity = Arc::clone(&activity);
    let client_to_origin = async move {
        copy_with_idle(
            &mut client_read,
            &mut origin_write,
            client_activity,
            started,
        )
        .await;
    };
    let origin_to_client = async move {
        copy_with_idle(
            &mut origin_read,
            &mut client_write,
            origin_activity,
            started,
        )
        .await;
    };
    let copies = async {
        tokio::join!(client_to_origin, origin_to_client);
    };
    let watchdog = async {
        loop {
            tokio::time::sleep(IDLE_TIMEOUT / 4).await;
            let idle = (started.elapsed().as_millis() as u64)
                .saturating_sub(activity.load(Ordering::Relaxed));
            if idle >= IDLE_TIMEOUT.as_millis() as u64 {
                break;
            }
        }
    };

    tokio::select! {
        _ = copies => {}
        _ = watchdog => {}
    }
    Ok(())
}

async fn copy_with_idle<R, W>(
    reader: &mut R,
    writer: &mut W,
    activity: Arc<AtomicU64>,
    started: Instant,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        let count = match tokio::time::timeout(IDLE_TIMEOUT, reader.read(&mut buffer)).await {
            Ok(Ok(0)) => {
                let _ = writer.shutdown().await;
                break;
            }
            Ok(Ok(count)) => count,
            _ => break,
        };
        activity.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        if writer.write_all(&buffer[..count]).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn listener() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    async fn accepted_within(listener: &TcpListener, wait: Duration) -> bool {
        tokio::time::timeout(wait, listener.accept()).await.is_ok()
    }

    #[tokio::test]
    async fn a_silent_client_gets_an_origin_connection_after_the_delay() {
        let (listener, port) = listener().await;
        let early = EarlyOrigin::spawn(
            OriginTarget::direct("127.0.0.1", port),
            None,
            Duration::from_millis(50),
        );
        assert!(!accepted_within(&listener, Duration::from_millis(20)).await);
        assert!(accepted_within(&listener, Duration::from_secs(2)).await);
        assert!(early.is_started());
        early.connect().await.unwrap();
    }

    #[tokio::test]
    async fn an_authenticated_client_never_reaches_the_origin() {
        let (listener, port) = listener().await;
        let early = EarlyOrigin::spawn(
            OriginTarget::direct("127.0.0.1", port),
            None,
            Duration::from_millis(50),
        );
        early.cancel();
        assert!(!accepted_within(&listener, Duration::from_millis(200)).await);
    }

    #[tokio::test]
    async fn an_early_passthrough_dials_at_once() {
        let (listener, port) = listener().await;
        let early = EarlyOrigin::spawn(
            OriginTarget::direct("127.0.0.1", port),
            None,
            Duration::from_secs(30),
        );
        let started = Instant::now();
        let origin = early.connect().await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(accepted_within(&listener, Duration::from_secs(2)).await);
        drop(origin);
    }

    #[tokio::test]
    async fn a_fallback_gets_the_client_address_in_a_proxy_header() {
        let (listener, port) = listener().await;
        let fallbacks = [Fallback {
            host: "site.example".into(),
            origin: OriginTarget {
                host: "127.0.0.1".into(),
                port,
                proxy_protocol: true,
            },
        }];
        let target = OriginTarget::for_host(&fallbacks, "SITE.example", 443);
        assert_eq!(target, fallbacks[0].origin);
        assert_eq!(
            OriginTarget::for_host(&fallbacks, "other.example", 443),
            OriginTarget::direct("other.example", 443)
        );
        let client = Endpoints {
            source: "203.0.113.7:51000".parse().unwrap(),
            destination: "198.51.100.1:443".parse().unwrap(),
        };
        let early = EarlyOrigin::spawn(target, Some(client), Duration::from_secs(30));
        let mut origin = early.connect().await.unwrap();
        origin.write_all(b"hello").await.unwrap();
        let (mut accepted, _) = listener.accept().await.unwrap();
        let found = crate::proxy::read_header(&mut accepted).await.unwrap();
        assert_eq!(found, Some(client));
        let mut tail = [0u8; 5];
        accepted.read_exact(&mut tail).await.unwrap();
        assert_eq!(&tail, b"hello");
    }
}
