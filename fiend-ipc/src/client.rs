use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, oneshot, watch};

use crate::{
    ClientMessage, Event, FrameError, HANDSHAKE_FRAME_BYTES, Hello, HelloAck, IpcError, LogEntry,
    MAX_FRAME_BYTES, PROTOCOL_MAJOR, Profile, ProfileInput, Reply, Request, RuleKind, RuleStats,
    ServerMessage, Settings, Status, Subscription, read_frame, write_frame,
};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Daemon(#[from] IpcError),
    #[error("cannot reach the daemon: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Frame(#[from] FrameError),
    #[error("the daemon uses protocol {major}, this client uses {PROTOCOL_MAJOR}")]
    UnsupportedVersion { major: u16 },
    #[error("the daemon sent a message that was not expected")]
    Unexpected,
    #[error("the connection to the daemon is closed")]
    Closed,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Reply, IpcError>>>>>;

type Writer = Box<dyn AsyncWrite + Send + Unpin>;

pub struct Client {
    writer: tokio::sync::Mutex<Writer>,
    pending: Pending,
    next_id: AtomicU64,
    events: broadcast::Sender<Event>,
    closed: watch::Receiver<bool>,
    ack: HelloAck,
}

impl Client {
    #[cfg(unix)]
    pub async fn connect(path: &std::path::Path, name: &str) -> Result<Self, ClientError> {
        let stream = tokio::net::UnixStream::connect(path).await?;
        Self::over(stream, name).await
    }

    pub async fn over<S>(stream: S, name: &str) -> Result<Self, ClientError>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(stream);
        write_frame(
            &mut writer,
            &ClientMessage::Hello(Hello::new(name)),
            HANDSHAKE_FRAME_BYTES,
        )
        .await?;
        let ack = match read_frame::<_, ServerMessage>(&mut reader, HANDSHAKE_FRAME_BYTES).await? {
            Some(ServerMessage::Hello(ack)) => ack,
            Some(ServerMessage::Response {
                result: Err(error), ..
            }) => return Err(error.into()),
            Some(_) => return Err(ClientError::Unexpected),
            None => return Err(ClientError::Closed),
        };
        if ack.major != PROTOCOL_MAJOR {
            return Err(ClientError::UnsupportedVersion { major: ack.major });
        }
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(256);
        let (closed_sender, closed) = watch::channel(false);
        let task_pending = pending.clone();
        let task_events = events.clone();
        tokio::spawn(async move {
            loop {
                match read_frame::<_, ServerMessage>(&mut reader, MAX_FRAME_BYTES).await {
                    Ok(Some(ServerMessage::Response { id, result })) => {
                        let sender = task_pending
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .remove(&id);
                        if let Some(sender) = sender {
                            let _ = sender.send(result);
                        }
                    }
                    Ok(Some(ServerMessage::Event(event))) => {
                        let _ = task_events.send(event);
                    }
                    Ok(Some(ServerMessage::Hello(_))) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            task_pending
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            let _ = closed_sender.send(true);
        });
        Ok(Self {
            writer: tokio::sync::Mutex::new(Box::new(writer)),
            pending,
            next_id: AtomicU64::new(1),
            events,
            closed,
            ack,
        })
    }

    pub fn daemon_version(&self) -> &str {
        &self.ack.daemon_version
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    pub async fn closed(&self) {
        let mut closed = self.closed.clone();
        let _ = closed.wait_for(|value| *value).await;
    }

    pub async fn request(&self, request: Request) -> Result<Reply, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id, sender);
        let sent = {
            let mut writer = self.writer.lock().await;
            write_frame(
                &mut *writer,
                &ClientMessage::Request { id, request },
                MAX_FRAME_BYTES,
            )
            .await
        };
        if let Err(error) = sent {
            self.pending
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&id);
            return Err(error.into());
        }
        receiver
            .await
            .map_err(|_| ClientError::Closed)?
            .map_err(Into::into)
    }

    async fn expect_ok(&self, request: Request) -> Result<(), ClientError> {
        match self.request(request).await? {
            Reply::Ok => Ok(()),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn status(&self) -> Result<Status, ClientError> {
        match self.request(Request::Status).await? {
            Reply::Status(status) => Ok(status),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn connect_profile(&self, profile_id: Option<String>) -> Result<(), ClientError> {
        self.expect_ok(Request::Connect { profile_id }).await
    }

    pub async fn disconnect(&self) -> Result<(), ClientError> {
        self.expect_ok(Request::Disconnect).await
    }

    pub async fn subscribe(&self) -> Result<(), ClientError> {
        self.expect_ok(Request::Subscribe).await
    }

    pub async fn unsubscribe(&self) -> Result<(), ClientError> {
        self.expect_ok(Request::Unsubscribe).await
    }

    pub async fn list_profiles(&self) -> Result<Vec<Profile>, ClientError> {
        match self.request(Request::ListProfiles).await? {
            Reply::Profiles { profiles } => Ok(profiles),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn save_profile(&self, profile: ProfileInput) -> Result<Profile, ClientError> {
        match self.request(Request::SaveProfile { profile }).await? {
            Reply::Profile(profile) => Ok(profile),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn delete_profile(&self, id: String) -> Result<(), ClientError> {
        self.expect_ok(Request::DeleteProfile { id }).await
    }

    pub async fn import_link(&self, link: String) -> Result<Profile, ClientError> {
        match self.request(Request::ImportLink { link }).await? {
            Reply::Profile(profile) => Ok(profile),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn share_link(&self, id: String) -> Result<String, ClientError> {
        match self.request(Request::ShareLink { id }).await? {
            Reply::Link { link } => Ok(link),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn settings(&self) -> Result<Settings, ClientError> {
        match self.request(Request::GetSettings).await? {
            Reply::Settings(settings) => Ok(settings),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn set_settings(&self, settings: Settings) -> Result<Settings, ClientError> {
        match self.request(Request::SetSettings { settings }).await? {
            Reply::Settings(settings) => Ok(settings),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn set_rule_text(
        &self,
        kind: RuleKind,
        text: String,
    ) -> Result<RuleStats, ClientError> {
        match self.request(Request::SetRuleText { kind, text }).await? {
            Reply::RuleStats(stats) => Ok(stats),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn add_subscription(
        &self,
        url: String,
        kind: RuleKind,
    ) -> Result<Subscription, ClientError> {
        match self.request(Request::AddSubscription { url, kind }).await? {
            Reply::Subscription(subscription) => Ok(subscription),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn remove_subscription(&self, id: String) -> Result<(), ClientError> {
        self.expect_ok(Request::RemoveSubscription { id }).await
    }

    pub async fn refresh_subscriptions(&self) -> Result<Settings, ClientError> {
        match self.request(Request::RefreshSubscriptions).await? {
            Reply::Settings(settings) => Ok(settings),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn measure_latency(&self, profile_id: String) -> Result<u32, ClientError> {
        match self.request(Request::MeasureLatency { profile_id }).await? {
            Reply::Latency { ms } => Ok(ms),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn logs(&self, limit: u32) -> Result<Vec<LogEntry>, ClientError> {
        match self.request(Request::Logs { limit }).await? {
            Reply::Logs { entries } => Ok(entries),
            _ => Err(ClientError::Unexpected),
        }
    }

    pub async fn clear_logs(&self) -> Result<(), ClientError> {
        self.expect_ok(Request::ClearLogs).await
    }
}
