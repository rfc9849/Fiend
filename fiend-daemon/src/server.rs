use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fiend_ipc::{
    ClientMessage, ErrorCode, Event, HANDSHAKE_FRAME_BYTES, HelloAck, IpcError, MAX_FRAME_BYTES,
    PROTOCOL_MAJOR, PROTOCOL_MINOR, ServerMessage, read_frame, write_frame,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast};
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use crate::Daemon;
#[cfg(unix)]
use crate::peer::SystemDirectory;
use crate::peer::{Directory, PeerPolicy};
use crate::platform::Platform;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PARALLEL_REQUESTS: usize = 16;
const MAX_CONNECTIONS: usize = 32;

#[cfg(unix)]
pub struct Listener(tokio::net::UnixListener);

#[cfg(windows)]
pub struct Listener {
    name: std::ffi::OsString,
    security: Arc<crate::windows::PipeSecurity>,
    first: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

#[cfg(unix)]
pub fn bind_socket(
    path: &Path,
    group: Option<&str>,
    directory: &dyn Directory,
) -> io::Result<Listener> {
    use std::os::unix::fs::PermissionsExt;

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
    let listener = tokio::net::UnixListener::bind(path)?;
    let gid = group.and_then(|name| directory.group(name)).map(|g| g.gid);
    match gid {
        Some(gid) => {
            std::os::unix::fs::chown(path, None, Some(gid))?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
        }
        None => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?,
    }
    Ok(Listener(listener))
}

#[cfg(windows)]
pub fn bind_socket(
    path: &Path,
    _group: Option<&str>,
    _directory: &dyn Directory,
) -> io::Result<Listener> {
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    let name = path.as_os_str().to_os_string();
    let security = crate::windows::PipeSecurity::new()?;
    let mut options = ServerOptions::new();
    options
        .pipe_mode(PipeMode::Byte)
        .max_instances(MAX_CONNECTIONS + 1);
    let first = security.create(&options, &name, true)?;
    Ok(Listener {
        name,
        security: Arc::new(security),
        first: Some(first),
    })
}

#[cfg(windows)]
impl Listener {
    fn create(&self) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

        let mut options = ServerOptions::new();
        options
            .pipe_mode(PipeMode::Byte)
            .max_instances(MAX_CONNECTIONS + 1);
        self.security.create(&options, &self.name, false)
    }
}

enum Access {
    #[cfg_attr(windows, allow(dead_code))]
    Allowed,
    #[cfg_attr(windows, allow(dead_code))]
    Denied,
    #[cfg_attr(unix, allow(dead_code))]
    AfterHello(Box<dyn FnOnce() -> bool + Send>),
}

pub async fn serve<P: Platform>(
    daemon: Daemon<P>,
    listener: Listener,
    policy: PeerPolicy,
    shutdown: CancellationToken,
) {
    #[cfg(unix)]
    let policy = Arc::new(policy);
    #[cfg(windows)]
    let _ = policy;
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));

    #[cfg(unix)]
    {
        let listener = listener.0;
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
                let allowed = stream
                    .peer_cred()
                    .map(|cred| policy.allows(cred.uid(), cred.gid(), &SystemDirectory))
                    .unwrap_or(false);
                let access = if allowed {
                    Access::Allowed
                } else {
                    Access::Denied
                };
                if let Err(error) = connection(daemon, stream, access, shutdown).await {
                    tracing::info!(%error, "client connection ended");
                }
            });
        }
    }

    #[cfg(windows)]
    {
        let mut listener = listener;
        let mut server = match listener.first.take() {
            Some(server) => server,
            None => return,
        };
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                connected = server.connect() => {
                    if let Err(error) = connected {
                        tracing::warn!(%error, "pipe connection failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        match listener.create() {
                            Ok(next) => server = next,
                            Err(error) => {
                                tracing::error!(%error, "cannot create a pipe instance");
                                return;
                            }
                        }
                        continue;
                    }
                }
            }
            let client = crate::windows::PipeClient::of(&server);
            let security = listener.security.clone();
            let access = Access::AfterHello(Box::new(move || match client.sid() {
                Ok(sid) => security.allows(&sid),
                Err(error) => {
                    tracing::warn!(%error, "cannot read the pipe client SID");
                    false
                }
            }));
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                tracing::warn!("too many clients");
                drop(server);
                match listener.create() {
                    Ok(next) => server = next,
                    Err(error) => {
                        tracing::error!(%error, "cannot create a pipe instance");
                        return;
                    }
                }
                continue;
            };
            let next = match listener.create() {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(%error, "cannot create a pipe instance");
                    return;
                }
            };
            let daemon = daemon.clone();
            let shutdown = shutdown.child_token();
            tokio::spawn(async move {
                let _slot = slot;
                if let Err(error) = connection(daemon, server, access, shutdown).await {
                    tracing::info!(%error, "client connection ended");
                }
            });
            server = next;
        }
    }
}

type Reader = Box<dyn AsyncRead + Send + Unpin>;
type Writer = Arc<AsyncMutex<Box<dyn AsyncWrite + Send + Unpin>>>;

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

async fn refuse(writer: &Writer) {
    tracing::warn!("client refused");
    let _ = send(
        writer,
        &ServerMessage::Response {
            id: 0,
            result: Err(IpcError::new(
                ErrorCode::Unauthorized,
                "You have no permission to use the daemon",
            )),
        },
    )
    .await;
}

async fn connection<P, S>(
    daemon: Daemon<P>,
    stream: S,
    access: Access,
    shutdown: CancellationToken,
) -> io::Result<()>
where
    P: Platform,
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    let mut reader: Reader = Box::new(reader);
    let writer: Writer = Arc::new(AsyncMutex::new(Box::new(writer)));
    if matches!(access, Access::Denied) {
        refuse(&writer).await;
        return Ok(());
    }
    let hello = tokio::time::timeout(
        HELLO_TIMEOUT,
        read_frame::<_, ClientMessage>(&mut reader, HANDSHAKE_FRAME_BYTES),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no hello"))?
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if let Access::AfterHello(check) = access
        && !check()
    {
        refuse(&writer).await;
        return Ok(());
    }
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
