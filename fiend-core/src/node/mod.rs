pub mod control;
pub mod logs;
pub mod metrics;
pub mod outbound;
pub mod update;
mod usage;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod warp;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "warp_stub.rs"]
pub mod warp;

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::crypto::AuthKey;
use crate::server::ServerConfig;
use crate::transport::OriginProfile;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_STATE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub id: String,
    pub secret: String,
    pub enabled: bool,
    #[serde(default = "yes")]
    pub allowed: bool,
    #[serde(default)]
    pub expire_at: Option<u64>,
    #[serde(default)]
    pub traffic_limit: Option<u64>,
    #[serde(default)]
    pub used_traffic: u64,
    #[serde(default)]
    pub usage_epoch: u64,
}

fn yes() -> bool {
    true
}

impl User {
    fn same_access(&self, other: &Self) -> bool {
        self.id == other.id
            && self.secret == other.secret
            && self.enabled == other.enabled
            && self.allowed == other.allowed
            && self.expire_at == other.expire_at
            && self.traffic_limit == other.traffic_limit
            && self.usage_epoch == other.usage_epoch
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesiredState {
    pub version: u64,
    pub users: Vec<User>,
    pub origin_host: String,
    pub origin_port: u16,
    pub origin_profile: OriginProfile,
    #[serde(default)]
    pub outbounds: Vec<outbound::Outbound>,
    #[serde(default)]
    pub rules: Vec<outbound::Rule>,
    #[serde(default)]
    pub warp: outbound::WarpSettings,
}

impl DesiredState {
    pub fn from_legacy(config: &ServerConfig) -> Self {
        Self {
            version: 1,
            users: vec![User {
                id: "legacy".into(),
                secret: hex::encode(&config.secret),
                enabled: true,
                allowed: true,
                expire_at: None,
                traffic_limit: None,
                used_traffic: 0,
                usage_epoch: 0,
            }],
            origin_host: config.origin_host.clone(),
            origin_port: config.origin_port,
            origin_profile: config.origin.clone(),
            outbounds: Vec::new(),
            rules: Vec::new(),
            warp: outbound::WarpSettings::default(),
        }
    }

    pub fn validate(&self, control_secret: &[u8]) -> io::Result<()> {
        if self.version == 0 || self.users.len() > 4096 {
            return Err(invalid(
                "state needs a nonzero version and at most 4096 users",
            ));
        }
        let mut config = ServerConfig::new(control_secret, &self.origin_host, self.origin_port);
        config.origin = self.origin_profile.clone();
        crate::config::build_server(config).map_err(|_| invalid("invalid state origin profile"))?;
        if self.origin_profile.record_sizes.len() > 64 || self.origin_host.len() > 253 {
            return Err(invalid("origin profile is too large"));
        }
        outbound::validate(&self.outbounds, &self.rules, &self.warp)?;
        let mut ids = HashSet::new();
        let mut secrets = HashSet::new();
        secrets.insert(control_secret.to_vec());
        for user in &self.users {
            if user.id.is_empty() || user.id.len() > 128 || !ids.insert(&user.id) {
                return Err(invalid("user ids must be unique and 1 to 128 bytes long"));
            }
            let secret = decode_secret(&user.secret)?;
            if !secrets.insert(secret.to_vec()) {
                return Err(invalid("user and control secrets must be distinct"));
            }
        }
        Ok(())
    }
}

pub struct UserEntry {
    user: User,
    secret: Zeroizing<Vec<u8>>,
    key: AuthKey,
    counters: Arc<Counters>,
    revoked: watch::Sender<bool>,
}

impl UserEntry {
    pub fn id(&self) -> &str {
        &self.user.id
    }

    pub fn active(&self, now: u64) -> bool {
        self.user.enabled
            && self.user.allowed
            && !self.revoked.borrow().to_owned()
            && self.user.expire_at.is_none_or(|expiry| now < expiry)
            && self
                .user
                .traffic_limit
                .is_none_or(|limit| self.counters.total.load(Ordering::Relaxed) < limit)
    }

    pub fn note_bytes(&self, count: usize, inbound: bool) {
        let counter = if inbound {
            &self.counters.up
        } else {
            &self.counters.down
        };
        counter.fetch_add(count as u64, Ordering::Relaxed);
        let used = self
            .counters
            .total
            .fetch_add(count as u64, Ordering::Relaxed)
            .saturating_add(count as u64);
        if self.user.traffic_limit.is_some_and(|limit| used >= limit) {
            self.revoked.send_replace(true);
        }
    }
}

#[derive(Default)]
struct Counters {
    up: AtomicU64,
    down: AtomicU64,
    total: AtomicU64,
    new_sessions: AtomicU64,
    open_sessions: AtomicU64,
}

struct Snapshot {
    state: DesiredState,
    users: Vec<Arc<UserEntry>>,
    router: Arc<outbound::Router>,
}

pub enum Principal {
    Control,
    User(Arc<UserEntry>),
}

#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

struct Inner {
    snapshot: RwLock<Arc<Snapshot>>,
    applying: tokio::sync::Mutex<()>,
    state_path: PathBuf,
    control_secret: Zeroizing<Vec<u8>>,
    control_key: AuthKey,
    accepted: AtomicU64,
    origin: AtomicU64,
    next_session: AtomicU64,
    sessions: Mutex<HashMap<u64, (SessionInfo, watch::Sender<bool>)>>,
    retired: Mutex<Vec<Arc<UserEntry>>>,
    started: Instant,
    started_at: u64,
    metrics: Mutex<metrics::Sampler>,
    updates: tokio::sync::Mutex<Option<update::Manager>>,
    restart: tokio::sync::Notify,
    origin_cache: Mutex<Option<(u64, OriginProfile)>>,
    warp: warp::Manager,
    resolver: Mutex<Option<outbound::Resolver>>,
    binary_sha256: std::sync::OnceLock<String>,
    listen_port: AtomicU64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: u64,
    pub user_id: String,
    pub started_at: u64,
}

#[derive(Serialize, Deserialize)]
pub struct UserStats {
    pub id: String,
    pub up: u64,
    pub down: u64,
    pub used_traffic: u64,
    pub new_sessions: u64,
    pub open_sessions: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Stats {
    pub system: metrics::Metrics,
    pub state_version: u64,
    pub users: Vec<UserStats>,
    pub handshakes_accepted: u64,
    pub handshakes_origin: u64,
    #[serde(default)]
    pub outbounds: Vec<outbound::OutboundStats>,
}

#[derive(Serialize, Deserialize)]
pub struct Health {
    pub protocol_version: u32,
    pub version: String,
    pub architecture: String,
    pub uptime: u64,
    pub started_at: u64,
    pub state_version: u64,
    #[serde(default)]
    pub binary_sha256: String,
    #[serde(default)]
    pub listen_port: u16,
}

fn binary_sha256() -> String {
    use sha2::{Digest, Sha256};
    std::env::current_exe()
        .and_then(fs::read)
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
        .unwrap_or_default()
}

impl Node {
    pub fn load(
        path: impl AsRef<Path>,
        control_secret: Vec<u8>,
        initial: DesiredState,
    ) -> io::Result<Self> {
        Self::load_with(
            path,
            control_secret,
            initial,
            PathBuf::from("/opt/fiend/bin/wireproxy"),
        )
    }

    pub fn load_with(
        path: impl AsRef<Path>,
        control_secret: Vec<u8>,
        initial: DesiredState,
        wireproxy: PathBuf,
    ) -> io::Result<Self> {
        if control_secret.len() != 32 {
            return Err(invalid("control secret must have 32 bytes"));
        }
        let path = path.as_ref();
        let state = if path.exists() {
            let raw = fs::read(path)?;
            if raw.len() > MAX_STATE_BYTES {
                return Err(invalid("saved state is too large"));
            }
            serde_json::from_slice(&raw).map_err(|_| invalid("invalid saved node state"))?
        } else {
            initial
        };
        state.validate(&control_secret)?;
        let snapshot = build_snapshot(state, None)?;
        if !path.exists() {
            save_state(path, &snapshot.state)?;
        }
        let node = Self {
            inner: Arc::new(Inner {
                snapshot: RwLock::new(Arc::new(snapshot)),
                applying: tokio::sync::Mutex::new(()),
                state_path: path.to_owned(),
                control_key: AuthKey::new(&control_secret),
                control_secret: Zeroizing::new(control_secret),
                accepted: AtomicU64::new(0),
                origin: AtomicU64::new(0),
                next_session: AtomicU64::new(1),
                sessions: Mutex::new(HashMap::new()),
                retired: Mutex::new(Vec::new()),
                started: Instant::now(),
                started_at: unix_time(),
                metrics: Mutex::new(metrics::Sampler::default()),
                updates: tokio::sync::Mutex::new(None),
                restart: tokio::sync::Notify::new(),
                origin_cache: Mutex::new(None),
                warp: warp::Manager::new(
                    path.parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or(Path::new("."))
                        .join("warp"),
                    wireproxy,
                ),
                resolver: Mutex::new(None),
                binary_sha256: std::sync::OnceLock::new(),
                listen_port: AtomicU64::new(0),
            }),
        };
        node.restore_usage()?;
        Ok(node)
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        self.inner
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub async fn configure_updates(&self, directory: PathBuf, config_path: PathBuf) {
        *self.inner.updates.lock().await = Some(update::Manager::new(
            directory,
            config_path,
            self.inner.state_path.clone(),
        ));
    }

    pub async fn update_begin(&self, request: update::Begin) -> io::Result<String> {
        self.inner
            .updates
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| invalid("binary updates are not configured"))?
            .begin(request)
    }

    pub async fn update_chunk(&self, request: update::Chunk) -> io::Result<u64> {
        self.inner
            .updates
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| invalid("binary updates are not configured"))?
            .chunk(request)
    }

    pub async fn update_finish(&self, id: &str) -> io::Result<()> {
        self.inner
            .updates
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| invalid("binary updates are not configured"))?
            .finish(id)
            .await?;
        let node = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            node.request_restart();
        });
        Ok(())
    }

    pub async fn wait_for_restart(&self) {
        self.inner.restart.notified().await;
    }

    pub fn request_restart(&self) {
        self.inner.restart.notify_one();
    }

    pub fn origin(&self) -> (String, u16, OriginProfile) {
        let snapshot = self.snapshot();
        let cache = self
            .inner
            .origin_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let profile = cache
            .as_ref()
            .filter(|(version, _)| *version == snapshot.state.version)
            .map(|(_, profile)| profile)
            .unwrap_or(&snapshot.state.origin_profile)
            .clone();
        (
            snapshot.state.origin_host.clone(),
            snapshot.state.origin_port,
            profile,
        )
    }

    pub async fn refresh_origin(&self) {
        let snapshot = self.snapshot();
        let state = &snapshot.state;
        if !state.origin_profile.record_sizes.is_empty() {
            return;
        }
        let hello = crate::transport::build_probe_hello(&state.origin_host);
        match crate::transport::probe_origin_at(&state.origin_host, state.origin_port, &hello).await
        {
            Ok(measured) => {
                let mut profile = state.origin_profile.clone();
                if profile.cipher_suite == 0 {
                    profile.cipher_suite = measured.cipher_suite;
                }
                if profile.key_share_group == 0 {
                    profile.key_share_group = measured.key_share_group;
                }
                if profile.extension_order.is_empty() {
                    profile.extension_order = measured.extension_order;
                }
                if measured.record_sizes.len() >= 2 {
                    profile.record_sizes = measured.record_sizes;
                }
                let current = self
                    .inner
                    .snapshot
                    .read()
                    .unwrap_or_else(|error| error.into_inner());
                if current.state.version == state.version {
                    *self
                        .inner
                        .origin_cache
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some((state.version, profile));
                }
            }
            Err(error) => tracing::debug!(%error, "node origin probe failed"),
        }
    }

    pub fn desired_state(&self) -> DesiredState {
        self.snapshot().state.clone()
    }

    pub fn router(&self) -> Arc<outbound::Router> {
        self.snapshot().router.clone()
    }

    pub fn warp(&self) -> &warp::Manager {
        &self.inner.warp
    }

    pub fn outbound_context(
        &self,
        policy: crate::target::TargetPolicy,
        timeout: std::time::Duration,
    ) -> outbound::Context {
        outbound::Context {
            policy,
            timeout,
            warp: self.inner.warp.socks(),
            resolver: self
                .inner
                .resolver
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        }
    }

    pub fn set_resolver(&self, resolver: Option<outbound::Resolver>) {
        *self
            .inner
            .resolver
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = resolver;
    }

    pub async fn start_warp(&self) {
        let settings = self.snapshot().state.warp.clone();
        self.inner.warp.apply(settings).await;
    }

    pub fn authenticate(&self, body: &[u8], time: i64) -> Option<(Vec<u8>, Principal)> {
        if body.len() < 71 || body[0] != 1 || body[38] != 32 {
            return None;
        }
        let random = &body[6..38];
        let token = &body[39..71];
        let buckets = [time / 3600 - 1, time / 3600, time / 3600 + 1];
        if buckets
            .iter()
            .any(|bucket| self.inner.control_key.matches(random, *bucket, token))
        {
            return Some((self.inner.control_secret.to_vec(), Principal::Control));
        }
        for entry in &self.snapshot().users {
            if entry.active(time.max(0) as u64)
                && buckets
                    .iter()
                    .any(|bucket| entry.key.matches(random, *bucket, token))
            {
                return Some((entry.secret.to_vec(), Principal::User(entry.clone())));
            }
        }
        None
    }

    pub async fn apply(&self, state: DesiredState) -> io::Result<u64> {
        let node = self.clone();
        tokio::spawn(async move { node.apply_inner(state).await })
            .await
            .map_err(io::Error::other)?
    }

    async fn apply_inner(&self, state: DesiredState) -> io::Result<u64> {
        let _lock = self.inner.applying.lock().await;
        state.validate(&self.inner.control_secret)?;
        let old = self.snapshot();
        if state.version <= old.state.version {
            return Err(invalid("state version must increase"));
        }
        let next = build_snapshot(state, Some(&old))?;
        let path = self.inner.state_path.clone();
        let saved = next.state.clone();
        tokio::task::spawn_blocking(move || save_state(&path, &saved))
            .await
            .map_err(io::Error::other)??;
        let version = next.state.version;
        let mut live = self
            .inner
            .snapshot
            .write()
            .unwrap_or_else(|error| error.into_inner());
        for (entry, desired) in next.users.iter().zip(&next.state.users) {
            let previous = old.users.iter().find(|old| old.user.id == entry.user.id);
            if previous.is_none_or(|old| old.user.usage_epoch != entry.user.usage_epoch) {
                entry
                    .counters
                    .total
                    .store(desired.used_traffic, Ordering::Relaxed);
            } else {
                entry
                    .counters
                    .total
                    .fetch_max(desired.used_traffic, Ordering::Relaxed);
            }
        }
        for entry in &next.users {
            if !entry.active(unix_time()) {
                entry.revoked.send_replace(true);
            }
        }
        for entry in &old.users {
            if !next.users.iter().any(|new| new.user.id == entry.user.id) {
                self.inner
                    .retired
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(entry.clone());
            }
            if !next.users.iter().any(|new| Arc::ptr_eq(new, entry)) {
                entry.revoked.send_replace(true);
            }
        }
        for runtime in old.router.runtimes() {
            if !next
                .router
                .runtimes()
                .iter()
                .any(|new| Arc::ptr_eq(new, runtime))
                && let Some(client) = runtime.client()
            {
                client.close();
            }
        }
        let warp_changed = old.state.warp != next.state.warp;
        let warp_settings = next.state.warp.clone();
        *live = Arc::new(next);
        drop(live);
        if warp_changed {
            let node = self.clone();
            tokio::spawn(async move {
                node.inner.warp.apply(warp_settings).await;
            });
        }
        let live = self.snapshot();
        if live.state.origin_profile.record_sizes.is_empty() {
            let node = self.clone();
            tokio::spawn(async move {
                node.refresh_origin().await;
            });
        }
        Ok(version)
    }

    pub fn accepted(&self) {
        self.inner.accepted.fetch_add(1, Ordering::Relaxed);
    }
    pub fn sent_to_origin(&self) {
        self.inner.origin.fetch_add(1, Ordering::Relaxed);
    }

    pub fn health(&self) -> Health {
        Health {
            protocol_version: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").into(),
            architecture: std::env::consts::ARCH.into(),
            uptime: self.inner.started.elapsed().as_secs(),
            started_at: self.inner.started_at,
            state_version: self.snapshot().state.version,
            binary_sha256: self.inner.binary_sha256.get_or_init(binary_sha256).clone(),
            listen_port: self.inner.listen_port.load(Ordering::Relaxed) as u16,
        }
    }

    pub fn set_listen_port(&self, port: u16) {
        self.inner
            .listen_port
            .store(u64::from(port), Ordering::Relaxed);
    }

    pub fn stats(&self) -> Stats {
        let snapshot = self
            .inner
            .snapshot
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let mut retired = self
            .inner
            .retired
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut rows = HashMap::<String, UserStats>::new();
        for entry in snapshot.users.iter().chain(retired.iter()) {
            let row = rows
                .entry(entry.user.id.clone())
                .or_insert_with(|| UserStats {
                    id: entry.user.id.clone(),
                    up: 0,
                    down: 0,
                    used_traffic: 0,
                    new_sessions: 0,
                    open_sessions: 0,
                });
            row.up = row
                .up
                .saturating_add(entry.counters.up.swap(0, Ordering::Relaxed));
            row.down = row
                .down
                .saturating_add(entry.counters.down.swap(0, Ordering::Relaxed));
            row.used_traffic = row
                .used_traffic
                .max(entry.counters.total.load(Ordering::Relaxed));
            row.new_sessions = row
                .new_sessions
                .saturating_add(entry.counters.new_sessions.swap(0, Ordering::Relaxed));
            row.open_sessions = row
                .open_sessions
                .saturating_add(entry.counters.open_sessions.load(Ordering::Relaxed));
        }
        retired.retain(|entry| entry.counters.open_sessions.load(Ordering::Relaxed) > 0);
        let mut users: Vec<_> = rows.into_values().collect();
        users.sort_by(|left, right| left.id.cmp(&right.id));
        Stats {
            system: self
                .inner
                .metrics
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .sample(),
            state_version: snapshot.state.version,
            users,
            handshakes_accepted: self.inner.accepted.swap(0, Ordering::Relaxed),
            handshakes_origin: self.inner.origin.swap(0, Ordering::Relaxed),
            outbounds: snapshot.router.stats(),
        }
    }

    pub fn sessions(&self, user_id: &str) -> Vec<SessionInfo> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|(info, _)| info.user_id == user_id)
            .map(|(info, _)| info.clone())
            .collect()
    }

    pub fn kill_sessions(&self, user_id: &str) -> usize {
        let sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut count = 0;
        for (info, cancel) in sessions.values() {
            if info.user_id == user_id {
                cancel.send_replace(true);
                count += 1;
            }
        }
        count
    }

    pub fn begin_session(&self, entry: Arc<UserEntry>) -> Option<SessionGuard> {
        let snapshot = self.snapshot();
        if !entry.active(unix_time())
            || !snapshot.users.iter().any(|user| Arc::ptr_eq(user, &entry))
        {
            return None;
        }
        let id = self.inner.next_session.fetch_add(1, Ordering::Relaxed);
        let (cancel, signal) = watch::channel(false);
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                (
                    SessionInfo {
                        id,
                        user_id: entry.user.id.clone(),
                        started_at: unix_time(),
                    },
                    cancel,
                ),
            );
        entry.counters.new_sessions.fetch_add(1, Ordering::Relaxed);
        entry.counters.open_sessions.fetch_add(1, Ordering::Relaxed);
        Some(SessionGuard {
            id,
            node: self.clone(),
            entry,
            signal,
        })
    }
}

pub struct SessionGuard {
    id: u64,
    node: Node,
    entry: Arc<UserEntry>,
    signal: watch::Receiver<bool>,
}

impl SessionGuard {
    pub async fn revoked(&mut self) {
        let mut user_signal = self.entry.revoked.subscribe();
        if *self.signal.borrow() || !self.entry.active(unix_time()) {
            return;
        }
        let expiry = self
            .entry
            .user
            .expire_at
            .unwrap_or(u64::MAX)
            .saturating_sub(unix_time());
        tokio::select! {
            _ = self.signal.changed() => {},
            _ = user_signal.changed() => {},
            _ = tokio::time::sleep(std::time::Duration::from_secs(expiry.min(100 * 365 * 86400))) => {},
        }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.node
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        self.entry
            .counters
            .open_sessions
            .fetch_sub(1, Ordering::Relaxed);
    }
}

fn build_snapshot(state: DesiredState, old: Option<&Snapshot>) -> io::Result<Snapshot> {
    let mut users = Vec::with_capacity(state.users.len());
    for user in &state.users {
        let previous = old.and_then(|old| old.users.iter().find(|entry| entry.user.id == user.id));
        if let Some(previous) =
            previous.filter(|entry| entry.user.same_access(user) && !*entry.revoked.borrow())
        {
            users.push(previous.clone());
            continue;
        }
        let counters = previous
            .map(|entry| entry.counters.clone())
            .unwrap_or_default();
        if previous.is_none() {
            counters.total.store(user.used_traffic, Ordering::Relaxed);
        }
        let secret = decode_secret(&user.secret)?;
        let key = AuthKey::new(&secret);
        let (revoked, _) = watch::channel(false);
        users.push(Arc::new(UserEntry {
            user: user.clone(),
            secret,
            key,
            counters,
            revoked,
        }));
    }
    let router = Arc::new(outbound::Router::build(
        &state.outbounds,
        &state.rules,
        old.map(|old| old.router.as_ref()),
    )?);
    Ok(Snapshot {
        state,
        users,
        router,
    })
}

pub fn decode_secret(text: &str) -> io::Result<Zeroizing<Vec<u8>>> {
    let bytes =
        Zeroizing::new(hex::decode(text.trim()).map_err(|_| invalid("secret must be hex"))?);
    if bytes.len() != 32 {
        return Err(invalid("secret must have 32 bytes"));
    }
    Ok(bytes)
}

pub fn save_state(path: &Path, state: &DesiredState) -> io::Result<()> {
    let raw = Zeroizing::new(serde_json::to_vec(state).map_err(io::Error::other)?);
    save_private(path, &raw)
}

pub fn save_private(path: &Path, raw: &[u8]) -> io::Result<()> {
    if raw.len() > MAX_STATE_BYTES {
        return Err(invalid("saved document is too large"));
    }
    let directory = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = directory.join(format!(
        ".fiend-state-{}.tmp",
        hex::encode(rand::random::<[u8; 16]>())
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(raw)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
