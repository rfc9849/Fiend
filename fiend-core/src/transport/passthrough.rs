use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::error::{Error, Result};

const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const COPY_BUFFER: usize = 32 * 1024;

pub async fn bridge_passthrough<S>(
    stream: S,
    buffered: Vec<u8>,
    host: &str,
    port: u16,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut origin = tokio::time::timeout(DIAL_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| Error::Timeout)??;
    origin.write_all(&buffered).await?;

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
