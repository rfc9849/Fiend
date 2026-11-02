use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use super::error::{Error, Result};

const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3600);
const COPY_BUFFER: usize = 32 * 1024;

async fn dial_origin(host: &str, port: u16) -> Result<TcpStream> {
    Ok(
        tokio::time::timeout(DIAL_TIMEOUT, TcpStream::connect((host, port)))
            .await
            .map_err(|_| Error::Timeout)??,
    )
}

pub struct EarlyOrigin {
    task: Mutex<Option<JoinHandle<Result<TcpStream>>>>,
    started: Arc<AtomicBool>,
    host: String,
    port: u16,
}

impl EarlyOrigin {
    pub fn spawn(host: String, port: u16, delay: Duration) -> Self {
        let started = Arc::new(AtomicBool::new(false));
        let task = {
            let started = started.clone();
            let host = host.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                started.store(true, Ordering::Release);
                dial_origin(&host, port).await
            })
        };
        Self {
            task: Mutex::new(Some(task)),
            started,
            host,
            port,
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
                Err(_) => dial_origin(&self.host, self.port).await,
            },
            Some(task) => {
                task.abort();
                dial_origin(&self.host, self.port).await
            }
            None => dial_origin(&self.host, self.port).await,
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
    let origin = dial_origin(host, port).await?;
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
        let early = EarlyOrigin::spawn("127.0.0.1".into(), port, Duration::from_millis(50));
        assert!(!accepted_within(&listener, Duration::from_millis(20)).await);
        assert!(accepted_within(&listener, Duration::from_secs(2)).await);
        assert!(early.is_started());
        early.connect().await.unwrap();
    }

    #[tokio::test]
    async fn an_authenticated_client_never_reaches_the_origin() {
        let (listener, port) = listener().await;
        let early = EarlyOrigin::spawn("127.0.0.1".into(), port, Duration::from_millis(50));
        early.cancel();
        assert!(!accepted_within(&listener, Duration::from_millis(200)).await);
    }

    #[tokio::test]
    async fn an_early_passthrough_dials_at_once() {
        let (listener, port) = listener().await;
        let early = EarlyOrigin::spawn("127.0.0.1".into(), port, Duration::from_secs(30));
        let started = Instant::now();
        let origin = early.connect().await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(accepted_within(&listener, Duration::from_secs(2)).await);
        drop(origin);
    }
}
