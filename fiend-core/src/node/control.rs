use std::future::poll_fn;
use std::io;
use std::time::Duration;

use bytes::Bytes;
use h2::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

use super::{DesiredState, MAX_STATE_BYTES, Node, PROTOCOL_VERSION};
use crate::client::ClientConfig;

pub const CONTROL_PATH: &str = "/_fiend/control/v1";
const CALL_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_CALLS: usize = 16;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub call: Call,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Call {
    Health,
    ApplyState(DesiredState),
    State,
    Stats,
    Sessions {
        user_id: String,
    },
    KillSessions {
        user_id: String,
    },
    ProbeOrigin {
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
    },
    Logs {
        limit: usize,
    },
    Restart,
    WarpStatus,
    WarpRegister,
    WarpRotate,
    WarpDelete,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

async fn dispatch(node: Node, request: Request) -> Response {
    let result = if request.version != PROTOCOL_VERSION {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported control version",
        ))
    } else {
        match request.call {
            Call::Health => serde_json::to_value(node.health()).map_err(io::Error::other),
            Call::Logs { limit } => {
                serde_json::to_value(super::logs::recent(limit)).map_err(io::Error::other)
            }
            Call::ApplyState(state) => node
                .apply(state)
                .await
                .map(|version| serde_json::json!({"applied_version": version})),
            Call::Stats => serde_json::to_value(node.stats()).map_err(io::Error::other),
            Call::State => serde_json::to_value(node.desired_state()).map_err(io::Error::other),
            Call::Sessions { user_id } => {
                serde_json::to_value(node.sessions(&user_id)).map_err(io::Error::other)
            }
            Call::KillSessions { user_id } => {
                Ok(serde_json::json!({"closed": node.kill_sessions(&user_id)}))
            }
            Call::ProbeOrigin { host, port } => {
                let state = node.desired_state();
                let host = host.unwrap_or(state.origin_host);
                let port = port.unwrap_or(state.origin_port);
                if host.is_empty() || host.len() > 253 || port == 0 {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid origin",
                    ))
                } else {
                    let hello = crate::transport::build_probe_hello(&host);
                    let target = node.origin_target(&host, port);
                    match crate::transport::probe_origin(&target, &hello).await {
                        Ok(profile) => serde_json::to_value(profile).map_err(io::Error::other),
                        Err(error) => {
                            Err(io::Error::other(format!("origin probe failed: {error}")))
                        }
                    }
                }
            }
            Call::Restart => {
                let node = node.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    node.request_restart();
                });
                Ok(serde_json::json!({"restarting": true}))
            }
            Call::WarpStatus => {
                serde_json::to_value(node.warp().status().await).map_err(io::Error::other)
            }
            Call::WarpRegister => {
                detached(node, |node| async move { node.warp().register().await })
                    .await
                    .and_then(|status| serde_json::to_value(status).map_err(io::Error::other))
            }
            Call::WarpRotate => detached(node, |node| async move { node.warp().rotate().await })
                .await
                .and_then(|rotation| serde_json::to_value(rotation).map_err(io::Error::other)),
            Call::WarpDelete => detached(node, |node| async move { node.warp().delete().await })
                .await
                .map(|()| serde_json::json!({"deleted": true})),
        }
    };
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Response {
        version: PROTOCOL_VERSION,
        id: request.id,
        result,
        error,
    }
}

async fn detached<T, F, Fut>(node: Node, operation: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce(Node) -> Fut,
    Fut: std::future::Future<Output = io::Result<T>> + Send + 'static,
{
    tokio::spawn(operation(node))
        .await
        .map_err(io::Error::other)?
}

pub async fn serve<S>(stream: S, node: Node) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut builder = h2::server::Builder::new();
    builder
        .max_concurrent_streams(MAX_CALLS as u32)
        .max_header_list_size(8192);
    let mut connection = timeout(CALL_TIMEOUT, builder.handshake::<_, Bytes>(stream))
        .await
        .map_err(|_| timed_out())?
        .map_err(io::Error::other)?;
    let mut calls = JoinSet::new();
    loop {
        tokio::select! {
            request = connection.accept() => {
                let Some(request) = request else { return Ok(()); };
                let (request, mut respond) = request.map_err(io::Error::other)?;
                if request.method() != http::Method::POST || request.uri().path() != CONTROL_PATH || calls.len() >= MAX_CALLS {
                    let response = http::Response::builder().status(http::StatusCode::FORBIDDEN).body(()).map_err(io::Error::other)?;
                    respond.send_response(response, true).map_err(io::Error::other)?;
                    continue;
                }
                let node = node.clone();
                calls.spawn(async move {
                    let operation = async {
                        let raw = read_body(request.into_body()).await?;
                        let request: Request = serde_json::from_slice(&raw).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid control request"))?;
                        let response = dispatch(node, request).await;
                        let mut bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
                        if bytes.len() > MAX_STATE_BYTES {
                            bytes = serde_json::to_vec(&Response { version: PROTOCOL_VERSION, id: response.id, result: None, error: Some("control response is too large".into()) }).map_err(io::Error::other)?;
                        }
                        let response = http::Response::builder().status(200).header(http::header::CONTENT_TYPE, "application/json").body(()).map_err(io::Error::other)?;
                        let send = respond.send_response(response, false).map_err(io::Error::other)?;
                        send_body(send, bytes).await?;
                        Ok::<(), io::Error>(())
                    };
                    let _ = timeout(CALL_TIMEOUT, operation).await;
                });
            }
            _ = calls.join_next(), if !calls.is_empty() => {},
        }
    }
}

async fn read_body(mut body: RecvStream) -> io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(io::Error::other)?;
        if raw.len().saturating_add(chunk.len()) > MAX_STATE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "control body is too large",
            ));
        }
        raw.extend_from_slice(&chunk);
        body.flow_control()
            .release_capacity(chunk.len())
            .map_err(io::Error::other)?;
    }
    Ok(raw)
}

async fn send_body(mut stream: SendStream<Bytes>, data: Vec<u8>) -> io::Result<()> {
    let mut data = Bytes::from(data);
    while !data.is_empty() {
        stream.reserve_capacity(data.len().min(16 * 1024));
        let capacity = poll_fn(|cx| stream.poll_capacity(cx))
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "control stream closed"))?
            .map_err(io::Error::other)?;
        if capacity == 0 {
            continue;
        }
        let count = capacity.min(data.len()).min(16 * 1024);
        let part = data.split_to(count);
        stream
            .send_data(part, data.is_empty())
            .map_err(io::Error::other)?;
    }
    Ok(())
}

pub struct Client {
    sender: h2::client::SendRequest<Bytes>,
    driver: JoinHandle<()>,
    next_id: u64,
}

impl Client {
    pub async fn connect(config: &ClientConfig) -> io::Result<Self> {
        let dial = async {
            let server = config
                .servers
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no node address"))?;
            let stream = crate::net::dial(server, &config.protect_socket).await?;
            stream.set_nodelay(true)?;
            let session = crate::transport::connect(stream, &config.secret, &config.template)
                .await
                .map_err(io::Error::other)?;
            Self::handshake(session).await
        };
        timeout(CALL_TIMEOUT, dial).await.map_err(|_| timed_out())?
    }

    pub async fn handshake<S>(stream: S) -> io::Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (sender, connection) = timeout(CALL_TIMEOUT, h2::client::handshake(stream))
            .await
            .map_err(|_| timed_out())?
            .map_err(io::Error::other)?;
        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self {
            sender,
            driver,
            next_id: 1,
        })
    }

    pub async fn call(&mut self, call: Call) -> io::Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("control request id overflow"))?;
        let request = Request {
            version: PROTOCOL_VERSION,
            id,
            call,
        };
        let bytes = serde_json::to_vec(&request).map_err(io::Error::other)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control body is too large",
            ));
        }
        let operation = async {
            let mut sender = self
                .sender
                .clone()
                .ready()
                .await
                .map_err(io::Error::other)?;
            let request = http::Request::builder()
                .method("POST")
                .version(http::Version::HTTP_2)
                .uri(format!("https://fiend.local{CONTROL_PATH}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(())
                .map_err(io::Error::other)?;
            let (response, send) = sender
                .send_request(request, false)
                .map_err(io::Error::other)?;
            send_body(send, bytes).await?;
            let response = response.await.map_err(io::Error::other)?;
            if response.status() != http::StatusCode::OK {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "control request refused",
                ));
            }
            let raw = read_body(response.into_body()).await?;
            let response: Response = serde_json::from_slice(&raw).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid control response")
            })?;
            if response.version != PROTOCOL_VERSION || response.id != id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "control response version or id mismatch",
                ));
            }
            match (response.result, response.error) {
                (Some(result), None) => Ok(result),
                (None, Some(error)) => Err(io::Error::other(error)),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid control result",
                )),
            }
        };
        timeout(CALL_TIMEOUT, operation)
            .await
            .map_err(|_| timed_out())?
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

#[cfg(unix)]
pub async fn serve_unix(path: &std::path::Path, node: Node) -> io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use tokio::net::{UnixListener, UnixStream};
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "control path is not a socket",
            ));
        }
        match UnixStream::connect(path).await {
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(path)?
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "control socket is in use",
                ));
            }
        }
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept(), if connections.len() < 32 => {
                let (stream, _) = accepted?;
                let node = node.clone();
                connections.spawn(async move { let _ = serve(stream, node).await; });
            }
            _ = connections.join_next(), if !connections.is_empty() => {},
        }
    }
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "control request timed out")
}
