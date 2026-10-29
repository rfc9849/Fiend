use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fiend_ipc::{
    ClientMessage, ErrorCode, Event, HANDSHAKE_FRAME_BYTES, HelloAck, IpcError, MAX_FRAME_BYTES,
    PROTOCOL_MAJOR, PROTOCOL_MINOR, ServerMessage, read_frame, write_frame,
};
use tokio::io::WriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast};
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use crate::Daemon;
use crate::peer::{Directory, PeerPolicy, SystemDirectory};
use crate::platform::Platform;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PARALLEL_REQUESTS: usize = 16;
const MAX_CONNECTIONS: usize = 32;

pub fn bind_socket(
    path: &Path,
    group: Option<&str>,
    directory: &dyn Directory,
) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755))?;
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            use std::os::unix::fs::FileTypeExt;
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "the socket path is used by another file",
                ));
            }
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another daemon is running",
                ));
            }
            std::fs::remove_file(path)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    let gid = group.and_then(|name| directory.group(name)).map(|g| g.gid);
    match gid {
        Some(gid) => {
            std::os::unix::fs::chown(path, None, Some(gid))?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
        }
        None => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?,
    }
    Ok(listener)
}

pub async fn serve<P: Platform>(
    daemon: Daemon<P>,
    listener: UnixListener,
    policy: PeerPolicy,
    shutdown: CancellationToken,
) {
    let policy = Arc::new(policy);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let stream = tokio::select! {
            _ = shutdown.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(error) => {
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            tracing::warn!("too many clients");
            continue;
        };
        let daemon = daemon.clone();
        let policy = policy.clone();
        let shutdown = shutdown.child_token();
        tokio::spawn(async move {
            let _slot = slot;
            if let Err(error) = connection(daemon, stream, &policy, shutdown).await {
                tracing::info!(%error, "client connection ended");
            }
        });
    }
}

type Writer = Arc<AsyncMutex<WriteHalf<UnixStream>>>;

async fn send(writer: &Writer, message: &ServerMessage) -> io::Result<()> {
    let mut writer = writer.lock().await;
    match tokio::time::timeout(
        WRITE_TIMEOUT,
        write_frame(&mut *writer, message, MAX_FRAME_BYTES),
    )
    .await
    {
        Ok(result) => result.map_err(|error| io::Error::other(error.to_string())),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "client is too slow",
        )),
    }
}

async fn connection<P: Platform>(
    daemon: Daemon<P>,
    stream: UnixStream,
    policy: &PeerPolicy,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let cred = stream.peer_cred()?;
    let allowed = policy.allows(cred.uid(), cred.gid(), &SystemDirectory);
    let (mut reader, writer) = tokio::io::split(stream);
    let writer: Writer = Arc::new(AsyncMutex::new(writer));
    if !allowed {
        tracing::warn!(uid = cred.uid(), "client refused");
        let _ = send(
            &writer,
            &ServerMessage::Response {
                id: 0,
                result: Err(IpcError::new(
                    ErrorCode::Unauthorized,
                    "You have no permission to use the daemon",
                )),
            },
        )
        .await;
        return Ok(());
    }
    let hello = tokio::time::timeout(
        HELLO_TIMEOUT,
        read_frame::<_, ClientMessage>(&mut reader, HANDSHAKE_FRAME_BYTES),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no hello"))?
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    match hello {
        Some(ClientMessage::Hello(hello)) if hello.major == PROTOCOL_MAJOR => {}
        Some(ClientMessage::Hello(hello)) => {
            let _ = send(
                &writer,
                &ServerMessage::Response {
                    id: 0,
                    result: Err(IpcError::new(
                        ErrorCode::UnsupportedVersion,
                        format!(
                            "The daemon uses protocol {PROTOCOL_MAJOR}, the client uses {}",
                            hello.major
                        ),
                    )),
                },
            )
            .await;
            return Ok(());
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "no hello")),
    }
    send(
        &writer,
        &ServerMessage::Hello(HelloAck {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        }),
    )
    .await?;

    let permits = Arc::new(Semaphore::new(MAX_PARALLEL_REQUESTS));
    let forwarder: Arc<Mutex<Option<AbortHandle>>> = Arc::default();
    let result = loop {
        let message = tokio::select! {
            _ = shutdown.cancelled() => break Ok(()),
            message = read_frame::<_, ClientMessage>(&mut reader, MAX_FRAME_BYTES) => message,
        };
        let (id, request) = match message {
            Ok(Some(ClientMessage::Request { id, request })) => (id, request),
            Ok(Some(ClientMessage::Hello(_))) => {
                break Err(io::Error::new(io::ErrorKind::InvalidData, "second hello"));
            }
            Ok(None) => break Ok(()),
            Err(error) => {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    error.to_string(),
                ));
            }
        };
        let Ok(permit) = permits.clone().acquire_owned().await else {
            break Ok(());
        };
        let daemon = daemon.clone();
        let writer = writer.clone();
        let forwarder = forwarder.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = match request {
                fiend_ipc::Request::Subscribe => {
                    subscribe(&daemon, &writer, &forwarder, &shutdown);
                    Ok(fiend_ipc::Reply::Ok)
                }
                fiend_ipc::Request::Unsubscribe => {
                    if let Some(handle) = lock(&forwarder).take() {
                        handle.abort();
                    }
                    Ok(fiend_ipc::Reply::Ok)
                }
                request => daemon.handle(request).await,
            };
            if send(&writer, &ServerMessage::Response { id, result })
                .await
                .is_err()
            {
                shutdown.cancel();
            }
        });
    };
    if let Some(handle) = lock(&forwarder).take() {
        handle.abort();
    }
    result
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn subscribe<P: Platform>(
    daemon: &Daemon<P>,
    writer: &Writer,
    forwarder: &Arc<Mutex<Option<AbortHandle>>>,
    shutdown: &CancellationToken,
) {
    let mut slot = lock(forwarder);
    if slot.is_some() {
        return;
    }
    let mut events = daemon.events();
    let daemon = daemon.clone();
    let writer = writer.clone();
    let shutdown = shutdown.clone();
    let task = tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => Event::State(daemon.status()),
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if send(&writer, &ServerMessage::Event(event)).await.is_err() {
                shutdown.cancel();
                break;
            }
        }
    });
    *slot = Some(task.abort_handle());
}
