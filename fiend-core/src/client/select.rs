use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout};

use crate::fingerprint::Template;
use crate::net::ProtectSocket;
use crate::transport::{self, Session};

const RACE_AGAIN: Duration = Duration::from_secs(300);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerStatus {
    pub server: String,
    pub rtt: Option<Duration>,
    pub failures: u32,
    pub preferred: bool,
}

struct Entry {
    rtt: Option<Duration>,
    failures: u32,
    retry_at: Option<Instant>,
}

struct State {
    entries: Vec<Entry>,
    preferred: Option<usize>,
    raced_at: Option<Instant>,
}

pub(crate) struct Selector {
    servers: Vec<String>,
    secret: Arc<Vec<u8>>,
    template: Arc<Template>,
    protect: ProtectSocket,
    notsent_lowat: u32,
    attempt_timeout: Duration,
    state: Mutex<State>,
}

impl Selector {
    pub(crate) fn new(
        servers: Vec<String>,
        secret: Vec<u8>,
        template: Template,
        protect: ProtectSocket,
        notsent_lowat: u32,
        attempt_timeout: Duration,
    ) -> Self {
        let entries = servers
            .iter()
            .map(|_| Entry {
                rtt: None,
                failures: 0,
                retry_at: None,
            })
            .collect();
        Self {
            servers,
            secret: Arc::new(secret),
            template: Arc::new(template),
            protect,
            notsent_lowat,
            attempt_timeout,
            state: Mutex::new(State {
                entries,
                preferred: None,
                raced_at: None,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn status(&self) -> Vec<ServerStatus> {
        let state = self.state();
        self.servers
            .iter()
            .zip(&state.entries)
            .enumerate()
            .map(|(index, (server, entry))| ServerStatus {
                server: server.clone(),
                rtt: entry.rtt,
                failures: entry.failures,
                preferred: state.preferred == Some(index),
            })
            .collect()
    }

    pub(crate) fn reset(&self) {
        let mut state = self.state();
        for entry in &mut state.entries {
            entry.failures = 0;
            entry.retry_at = None;
        }
        state.raced_at = None;
    }

    fn plan(&self) -> (Option<usize>, Vec<usize>) {
        let state = self.state();
        let now = Instant::now();
        let ready: Vec<usize> = (0..self.servers.len())
            .filter(|index| state.entries[*index].retry_at.is_none_or(|at| at <= now))
            .collect();
        let candidates = if ready.is_empty() {
            (0..self.servers.len()).collect()
        } else {
            ready
        };
        let race_due = state
            .raced_at
            .is_none_or(|at| now.duration_since(at) >= RACE_AGAIN);
        let preferred = state
            .preferred
            .filter(|index| candidates.contains(index) && !race_due);
        (preferred, candidates)
    }

    fn succeeded(&self, index: usize, rtt: Duration, raced: bool) {
        let mut state = self.state();
        let entry = &mut state.entries[index];
        entry.rtt = Some(rtt);
        entry.failures = 0;
        entry.retry_at = None;
        if state.preferred != Some(index) {
            tracing::info!(server = %self.servers[index], rtt_ms = rtt.as_millis() as u64, "selected server");
        }
        state.preferred = Some(index);
        if raced {
            state.raced_at = Some(Instant::now());
        }
    }

    fn failed(&self, index: usize, error: &transport::Error) {
        let mut state = self.state();
        let entry = &mut state.entries[index];
        entry.failures = entry.failures.saturating_add(1);
        let delay = Duration::from_secs(1u64 << entry.failures.min(6)).min(MAX_BACKOFF);
        entry.retry_at = Some(Instant::now() + delay);
        if state.preferred == Some(index) {
            state.preferred = None;
        }
        tracing::warn!(server = %self.servers[index], %error, "server failed");
    }

    pub(crate) async fn dial(self: Arc<Self>) -> transport::Result<Session<TcpStream>> {
        let (preferred, mut candidates) = self.plan();
        if let Some(index) = preferred {
            match self.attempt(index).await {
                Ok((session, rtt)) => {
                    self.succeeded(index, rtt, false);
                    return Ok(session);
                }
                Err(error) => {
                    self.failed(index, &error);
                    candidates.retain(|other| *other != index);
                    if candidates.is_empty() {
                        return Err(error);
                    }
                }
            }
        }
        if candidates.len() == 1 {
            let index = candidates[0];
            return match self.attempt(index).await {
                Ok((session, rtt)) => {
                    self.succeeded(index, rtt, true);
                    Ok(session)
                }
                Err(error) => {
                    self.failed(index, &error);
                    Err(error)
                }
            };
        }
        let mut race = JoinSet::new();
        for index in candidates {
            let selector = self.clone();
            race.spawn(async move { (index, selector.attempt(index).await) });
        }
        let mut last = None;
        while let Some(joined) = race.join_next().await {
            let Ok((index, result)) = joined else {
                continue;
            };
            match result {
                Ok((session, rtt)) => {
                    self.succeeded(index, rtt, true);
                    race.abort_all();
                    return Ok(session);
                }
                Err(error) => {
                    self.failed(index, &error);
                    last = Some(error);
                }
            }
        }
        Err(last
            .unwrap_or_else(|| transport::Error::from(io::Error::other("no server is available"))))
    }

    async fn attempt(&self, index: usize) -> transport::Result<(Session<TcpStream>, Duration)> {
        let server = &self.servers[index];
        let operation = async {
            let started = Instant::now();
            let stream = crate::net::dial(server, &self.protect)
                .await
                .map_err(transport::Error::from)?;
            let rtt = started.elapsed();
            stream.set_nodelay(true).map_err(transport::Error::from)?;
            if let Err(error) = crate::net::set_tcp_notsent_lowat(&stream, self.notsent_lowat) {
                tracing::warn!(%error, "cannot set TCP_NOTSENT_LOWAT");
            }
            let session = transport::connect(stream, &self.secret, &self.template).await?;
            Ok((session, rtt))
        };
        timeout(self.attempt_timeout, operation)
            .await
            .map_err(|_| {
                transport::Error::from(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "server handshake timed out",
                ))
            })?
    }
}
