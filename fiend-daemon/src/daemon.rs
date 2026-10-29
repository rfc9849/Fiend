use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fiend_ipc::{
    ConnectStep, DEFAULT_MTU, ErrorCode, Event, IpcError, MAX_MTU, MIN_MTU, Profile, ProfileInput,
    Reply, Request, RuleKind, RuleStats, Settings, Stats, Status, Subscription,
};
use fiend_tun::TunEngine;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::logs::{LogBuffer, now_ms};
use crate::machine::{Applied, EngineSettings, Machine};
use crate::platform::{NetworkRequest, Platform};
use crate::session::{app_profile, build_client, resolve_server, tun_options};
use crate::store::Store;

type Result<T> = std::result::Result<T, IpcError>;

const STOP_TIMEOUT: Duration = Duration::from_secs(30);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn error(code: ErrorCode, message: impl Into<String>) -> IpcError {
    IpcError::new(code, message)
}

fn from_app(error: fiend_app::AppError) -> IpcError {
    let code = match &error {
        fiend_app::AppError::InvalidProfile { .. } | fiend_app::AppError::InvalidLink => {
            ErrorCode::InvalidProfile
        }
        fiend_app::AppError::InvalidRules { .. } => ErrorCode::InvalidRules,
        fiend_app::AppError::Engine { .. } => ErrorCode::Engine,
    };
    IpcError::new(code, error.to_string())
}

fn internal(error: impl std::fmt::Display) -> IpcError {
    tracing::error!(%error, "internal error");
    IpcError::new(ErrorCode::Internal, error.to_string())
}

fn app_kind(kind: RuleKind) -> fiend_app::RuleKind {
    match kind {
        RuleKind::Direct => fiend_app::RuleKind::Direct,
        RuleKind::Block => fiend_app::RuleKind::Block,
        RuleKind::Proxy => fiend_app::RuleKind::Proxy,
    }
}

fn ipc_stats(stats: fiend_app::RuleStats) -> RuleStats {
    RuleStats {
        domains: stats.domains,
        exact: stats.exact,
        keywords: stats.keywords,
        cidrs: stats.cidrs,
    }
}

fn engine_settings(store: &Store) -> EngineSettings {
    let settings = store.settings();
    EngineSettings {
        bypass_lan: settings.bypass_lan,
        ipv6: settings.ipv6,
        direct_mode: settings.direct_mode,
        mtu: settings.mtu,
        direct_resolver: settings.direct_resolver.clone(),
        direct_rules: settings.direct_rules.clone(),
        block_rules: settings.block_rules.clone(),
        proxy_rules: settings.proxy_rules.clone(),
        rule_files: settings
            .subscriptions
            .iter()
            .filter(|item| store.rule_path(&item.id).is_file())
            .map(|item| (item.id.clone(), item.kind, item.updated_at_ms))
            .collect(),
    }
}

struct Running {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

struct Inner<P: Platform> {
    platform: Arc<P>,
    store: Mutex<Store>,
    machine: Mutex<Machine>,
    control: tokio::sync::Mutex<Option<Running>>,
    subscriptions: tokio::sync::Mutex<()>,
    events: broadcast::Sender<Event>,
    logs: Arc<LogBuffer>,
}

pub struct Daemon<P: Platform> {
    inner: Arc<Inner<P>>,
}

impl<P: Platform> Clone for Daemon<P> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<P: Platform> Inner<P> {
    fn status(&self) -> Status {
        let machine = lock(&self.machine);
        let restart_required = match (machine.applied(), machine.is_active()) {
            (Some(applied), true) => {
                let store = lock(&self.store);
                store.profile(&applied.profile.id) != Some(&applied.profile)
                    || engine_settings(&store) != applied.engine
            }
            _ => false,
        };
        Status {
            state: machine.state().clone(),
            profile_id: machine.profile_id().map(str::to_owned),
            stats: machine.stats(),
            restart_required,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn emit_state(&self) {
        self.emit(Event::State(self.status()));
    }

    fn emit_settings(&self) {
        let settings = lock(&self.store).settings().clone();
        self.emit(Event::Settings(settings));
    }

    fn set_step(&self, step: ConnectStep) {
        if lock(&self.machine).set_step(step) {
            self.emit_state();
        }
    }

    async fn run_session(
        self: Arc<Self>,
        profile: Profile,
        engine: EngineSettings,
        rule_files: Vec<fiend_tun::rules::RuleFile>,
        cancel: CancellationToken,
    ) {
        let outcome = self.session(&profile, &engine, rule_files, &cancel).await;
        let failure = outcome.err();
        match &failure {
            Some(reason) => tracing::error!(%reason, "tunnel stopped with an error"),
            None => tracing::info!("tunnel stopped"),
        }
        if lock(&self.machine).finish(failure) {
            self.emit_state();
        }
    }

    async fn session(
        self: &Arc<Self>,
        profile: &Profile,
        engine: &EngineSettings,
        rule_files: Vec<fiend_tun::rules::RuleFile>,
        cancel: &CancellationToken,
    ) -> std::result::Result<(), String> {
        let protect = self.platform.protect_socket();
        tracing::info!(profile = %profile.name, "tunnel start");
        let server = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            server = resolve_server(&profile.server) => server?,
        };
        let client = build_client(profile, server, protect.clone())?;
        let started = Instant::now();
        tokio::select! {
            _ = cancel.cancelled() => {
                client.close();
                return Ok(());
            }
            ready = client.ready() => ready.map_err(|error| error.to_string())?,
        }
        let latency = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        tracing::info!(latency_ms = latency, "server handshake completed");

        let options = tun_options(engine, rule_files, protect);
        let tun = {
            let client = client.clone();
            tokio::task::spawn_blocking(move || TunEngine::new(client, options))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?
        };
        let tun = Arc::new(tun);

        let request = NetworkRequest {
            server,
            mtu: engine.mtu,
            ipv6: engine.ipv6,
            bypass_lan: engine.bypass_lan,
        };
        let opened = {
            let platform = self.platform.clone();
            let inner = self.clone();
            tokio::task::spawn_blocking(move || {
                platform.open(&request, &|step| inner.set_step(step))
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
        };
        let (device, guard) = opened;
        if cancel.is_cancelled() {
            drop(guard);
            return Ok(());
        }

        if lock(&self.machine).connected(now_ms(), Some(latency)) {
            self.emit_state();
        }
        tracing::info!("tunnel connected");
        let stats_task = tokio::spawn(self.clone().stats_loop(tun.clone(), latency));

        let run = tun.run(device);
        tokio::pin!(run);
        let result = tokio::select! {
            result = &mut run => result,
            _ = cancel.cancelled() => {
                tun.stop();
                run.await
            }
        };
        tun.stop();
        stats_task.abort();
        drop(guard);
        result.map_err(|error| error.to_string())
    }

    async fn stats_loop(self: Arc<Self>, tun: Arc<TunEngine>, latency: u32) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last = (Instant::now(), 0u64, 0u64);
        loop {
            interval.tick().await;
            let raw = tun.stats();
            let elapsed = last.0.elapsed().as_secs_f64().max(0.001);
            let rate = |now: u64, before: u64| (now.saturating_sub(before) as f64 / elapsed) as u64;
            let stats = Stats {
                sent_bytes: raw.uploaded,
                received_bytes: raw.downloaded,
                upload_bps: rate(raw.uploaded, last.1),
                download_bps: rate(raw.downloaded, last.2),
                active_flows: raw.flows,
                dropped_packets: raw.dropped_packets,
                latency_ms: Some(latency),
            };
            last = (Instant::now(), raw.uploaded, raw.downloaded);
            if lock(&self.machine).set_stats(stats) && self.events.receiver_count() > 0 {
                self.emit(Event::Stats(stats));
            }
        }
    }
}

impl<P: Platform> Daemon<P> {
    pub fn new(platform: P, store: Store, logs: Arc<LogBuffer>) -> Self {
        let (events, _) = broadcast::channel(256);
        let mut log_events = logs.subscribe();
        let sender = events.clone();
        tokio::spawn(async move {
            loop {
                match log_events.recv().await {
                    Ok(entry) => {
                        let _ = sender.send(Event::Log(entry));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Self {
            inner: Arc::new(Inner {
                platform: Arc::new(platform),
                store: Mutex::new(store),
                machine: Mutex::new(Machine::new()),
                control: tokio::sync::Mutex::new(None),
                subscriptions: tokio::sync::Mutex::new(()),
                events,
                logs,
            }),
        }
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn status(&self) -> Status {
        self.inner.status()
    }

    pub async fn start(&self) {
        let (auto, id) = {
            let store = lock(&self.inner.store);
            let settings = store.settings();
            (settings.connect_at_startup, settings.active_profile.clone())
        };
        if auto && id.is_some() {
            tracing::info!("connect at startup");
            if let Err(error) = self.connect(id).await {
                tracing::warn!(%error, "connect at startup failed");
            }
        }
        self.spawn_refresh_stale();
    }

    pub async fn shutdown(&self) {
        let _ = self.disconnect().await;
    }

    pub async fn handle(&self, request: Request) -> Result<Reply> {
        match request {
            Request::Connect { profile_id } => self.connect(profile_id).await,
            Request::Disconnect => self.disconnect().await,
            Request::Status => Ok(Reply::Status(self.status())),
            Request::Subscribe | Request::Unsubscribe => Ok(Reply::Ok),
            Request::ListProfiles => Ok(Reply::Profiles {
                profiles: lock(&self.inner.store).profiles().to_vec(),
            }),
            Request::SaveProfile { profile } => self.save_profile(profile),
            Request::DeleteProfile { id } => self.delete_profile(&id),
            Request::ImportLink { link } => self.import_link(link),
            Request::ShareLink { id } => self.share_link(&id),
            Request::GetSettings => Ok(Reply::Settings(lock(&self.inner.store).settings().clone())),
            Request::SetSettings { settings } => self.set_settings(settings),
            Request::SetRuleText { kind, text } => self.set_rule_text(kind, text),
            Request::AddSubscription { url, kind } => self.add_subscription(url, kind).await,
            Request::RemoveSubscription { id } => self.remove_subscription(&id).await,
            Request::RefreshSubscriptions => self.refresh_subscriptions().await,
            Request::MeasureLatency { profile_id } => self.measure_latency(&profile_id).await,
            Request::Logs { limit } => Ok(Reply::Logs {
                entries: self.inner.logs.recent(limit as usize),
            }),
            Request::ClearLogs => {
                self.inner.logs.clear();
                Ok(Reply::Ok)
            }
        }
    }

    async fn stop_running(&self, running: Running) {
        if lock(&self.inner.machine).begin_disconnect() {
            self.inner.emit_state();
        }
        running.cancel.cancel();
        let abort = running.task.abort_handle();
        if tokio::time::timeout(STOP_TIMEOUT, running.task)
            .await
            .is_err()
        {
            tracing::error!("tunnel did not stop in time");
            abort.abort();
            if lock(&self.inner.machine).finish(None) {
                self.inner.emit_state();
            }
        }
    }

    async fn connect(&self, profile_id: Option<String>) -> Result<Reply> {
        let mut control = self.inner.control.lock().await;
        let (profile, engine, rule_files) = {
            let store = lock(&self.inner.store);
            let id = profile_id
                .or_else(|| store.settings().active_profile.clone())
                .ok_or_else(|| error(ErrorCode::InvalidRequest, "No profile is selected"))?;
            let profile = store
                .profile(&id)
                .cloned()
                .ok_or_else(|| error(ErrorCode::NotFound, "The profile is not available"))?;
            let engine = engine_settings(&store);
            let rule_files = engine
                .rule_files
                .iter()
                .map(|(id, kind, _)| fiend_tun::rules::RuleFile {
                    path: store.rule_path(id).to_string_lossy().into_owned(),
                    kind: app_kind(*kind).into(),
                })
                .collect::<Vec<_>>();
            (profile, engine, rule_files)
        };
        fiend_app::validate_profile(app_profile(&profile)).map_err(from_app)?;

        if let Some(running) = control.take() {
            self.stop_running(running).await;
        }
        let changed = {
            let mut store = lock(&self.inner.store);
            if store.settings().active_profile.as_deref() == Some(profile.id.as_str()) {
                false
            } else {
                store
                    .update_settings(|settings| settings.active_profile = Some(profile.id.clone()))
                    .map_err(internal)?;
                true
            }
        };
        if changed {
            self.inner.emit_settings();
        }
        let started = lock(&self.inner.machine).begin_connect(Applied {
            profile: profile.clone(),
            engine: engine.clone(),
        });
        if !started {
            return Err(error(ErrorCode::Busy, "The tunnel is busy"));
        }
        self.inner.emit_state();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(self.inner.clone().run_session(
            profile,
            engine,
            rule_files,
            cancel.clone(),
        ));
        *control = Some(Running { cancel, task });
        drop(control);
        self.spawn_refresh_stale();
        Ok(Reply::Ok)
    }

    async fn disconnect(&self) -> Result<Reply> {
        let mut control = self.inner.control.lock().await;
        if let Some(running) = control.take() {
            self.stop_running(running).await;
        }
        if lock(&self.inner.machine).dismiss() {
            self.inner.emit_state();
        }
        Ok(Reply::Ok)
    }

    fn save_profile(&self, input: ProfileInput) -> Result<Reply> {
        fiend_app::validate_profile(fiend_app::ProfileConfig {
            name: input.name.clone(),
            server: input.server.clone(),
            secret: input.secret.clone(),
            template_json: input.template_json.clone(),
        })
        .map_err(from_app)?;
        let profile = lock(&self.inner.store)
            .upsert_profile(input)
            .map_err(internal)?
            .ok_or_else(|| error(ErrorCode::NotFound, "The profile is not available"))?;
        self.inner.emit(Event::ProfilesChanged);
        self.inner.emit_state();
        Ok(Reply::Profile(profile))
    }

    fn delete_profile(&self, id: &str) -> Result<Reply> {
        if lock(&self.inner.machine).is_active()
            && lock(&self.inner.machine).profile_id() == Some(id)
        {
            return Err(error(
                ErrorCode::Busy,
                "Disconnect before you delete the active profile",
            ));
        }
        let cleared = {
            let mut store = lock(&self.inner.store);
            if !store.delete_profile(id).map_err(internal)? {
                return Err(error(ErrorCode::NotFound, "The profile is not available"));
            }
            if store.settings().active_profile.as_deref() == Some(id) {
                store
                    .update_settings(|settings| settings.active_profile = None)
                    .map_err(internal)?;
                true
            } else {
                false
            }
        };
        self.inner.emit(Event::ProfilesChanged);
        if cleared {
            self.inner.emit_settings();
        }
        Ok(Reply::Ok)
    }

    fn import_link(&self, link: String) -> Result<Reply> {
        let parsed = fiend_app::parse_share_link(link).map_err(from_app)?;
        let profile = lock(&self.inner.store)
            .upsert_profile(ProfileInput {
                id: None,
                name: parsed.name,
                server: parsed.server,
                secret: parsed.secret,
                template_json: parsed.template_json,
            })
            .map_err(internal)?
            .ok_or_else(|| error(ErrorCode::Internal, "The profile was not saved"))?;
        self.inner.emit(Event::ProfilesChanged);
        Ok(Reply::Profile(profile))
    }

    fn share_link(&self, id: &str) -> Result<Reply> {
        let profile = lock(&self.inner.store)
            .profile(id)
            .cloned()
            .ok_or_else(|| error(ErrorCode::NotFound, "The profile is not available"))?;
        let link = fiend_app::make_share_link(app_profile(&profile)).map_err(from_app)?;
        Ok(Reply::Link { link })
    }

    fn set_settings(&self, mut settings: Settings) -> Result<Reply> {
        if !(MIN_MTU..=MAX_MTU).contains(&settings.mtu) {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!("MTU must be from {MIN_MTU} to {MAX_MTU} (default {DEFAULT_MTU})"),
            ));
        }
        fiend_app::validate_rules(&settings.direct_rules, true).map_err(from_app)?;
        fiend_app::validate_rules(&settings.block_rules, false).map_err(from_app)?;
        fiend_app::validate_rules(&settings.proxy_rules, true).map_err(from_app)?;
        fiend_app::validate_resolver(&settings.direct_resolver).map_err(from_app)?;
        let stored = {
            let mut store = lock(&self.inner.store);
            if let Some(id) = &settings.active_profile
                && store.profile(id).is_none()
            {
                return Err(error(ErrorCode::NotFound, "The profile is not available"));
            }
            settings.subscriptions = store.settings().subscriptions.clone();
            store
                .update_settings(|current| *current = settings)
                .map_err(internal)?;
            store.settings().clone()
        };
        self.inner.emit(Event::Settings(stored.clone()));
        self.inner.emit_state();
        Ok(Reply::Settings(stored))
    }

    fn set_rule_text(&self, kind: RuleKind, text: String) -> Result<Reply> {
        let stats = fiend_app::validate_rules(&text, kind != RuleKind::Block).map_err(from_app)?;
        lock(&self.inner.store)
            .update_settings(|settings| match kind {
                RuleKind::Direct => settings.direct_rules = text,
                RuleKind::Block => settings.block_rules = text,
                RuleKind::Proxy => settings.proxy_rules = text,
            })
            .map_err(internal)?;
        self.inner.emit_settings();
        self.inner.emit_state();
        Ok(Reply::RuleStats(ipc_stats(stats)))
    }

    async fn add_subscription(&self, url: String, kind: RuleKind) -> Result<Reply> {
        let url = fiend_app::subscription::validate_url(&url).map_err(from_app)?;
        let _guard = self.inner.subscriptions.lock().await;
        let id = crate::store::new_id();
        lock(&self.inner.store)
            .update_settings(|settings| {
                settings.subscriptions.push(Subscription {
                    id: id.clone(),
                    url,
                    kind,
                    updated_at_ms: 0,
                    entries: 0,
                    error: None,
                })
            })
            .map_err(internal)?;
        self.refresh_one(&id).await;
        self.inner.emit_settings();
        self.inner.emit_state();
        let subscription = lock(&self.inner.store)
            .settings()
            .subscriptions
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| error(ErrorCode::Internal, "The subscription was not saved"))?;
        Ok(Reply::Subscription(subscription))
    }

    async fn remove_subscription(&self, id: &str) -> Result<Reply> {
        let _guard = self.inner.subscriptions.lock().await;
        let path = {
            let mut store = lock(&self.inner.store);
            let removed = store
                .update_settings(|settings| {
                    let before = settings.subscriptions.len();
                    settings.subscriptions.retain(|item| item.id != id);
                    before != settings.subscriptions.len()
                })
                .map_err(internal)?;
            if !removed {
                return Err(error(
                    ErrorCode::NotFound,
                    "The subscription is not available",
                ));
            }
            store.rule_path(id)
        };
        let _ = tokio::fs::remove_file(path).await;
        self.inner.emit_settings();
        self.inner.emit_state();
        Ok(Reply::Ok)
    }

    async fn refresh_subscriptions(&self) -> Result<Reply> {
        let _guard = self.inner.subscriptions.lock().await;
        let ids: Vec<String> = lock(&self.inner.store)
            .settings()
            .subscriptions
            .iter()
            .map(|item| item.id.clone())
            .collect();
        for id in ids {
            self.refresh_one(&id).await;
        }
        self.inner.emit_settings();
        self.inner.emit_state();
        Ok(Reply::Settings(lock(&self.inner.store).settings().clone()))
    }

    async fn refresh_one(&self, id: &str) {
        let (url, kind, path) = {
            let store = lock(&self.inner.store);
            let Some(item) = store.settings().subscriptions.iter().find(|i| i.id == id) else {
                return;
            };
            (item.url.clone(), item.kind, store.rule_path(id))
        };
        let result = tokio::task::spawn_blocking(move || {
            fiend_app::subscription::refresh_rule_file(&url, app_kind(kind), &path)
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));
        let applied = lock(&self.inner.store).update_settings(|settings| {
            if let Some(item) = settings.subscriptions.iter_mut().find(|i| i.id == id) {
                match &result {
                    Ok(stats) => {
                        item.updated_at_ms = now_ms();
                        item.entries = ipc_stats(*stats).total();
                        item.error = None;
                    }
                    Err(reason) => item.error = Some(reason.clone()),
                }
            }
        });
        if let Err(error) = applied {
            tracing::error!(%error, "cannot save subscription state");
        }
        match result {
            Ok(_) => tracing::info!("rule list refreshed"),
            Err(reason) => tracing::warn!(%reason, "rule list refresh failed"),
        }
    }

    fn spawn_refresh_stale(&self) {
        let daemon = self.clone();
        tokio::spawn(async move {
            let Ok(_guard) = daemon.inner.subscriptions.try_lock() else {
                return;
            };
            let now = now_ms();
            let interval = fiend_app::subscription::REFRESH_INTERVAL.as_millis() as u64;
            let stale: Vec<String> = lock(&daemon.inner.store)
                .settings()
                .subscriptions
                .iter()
                .filter(|item| {
                    item.updated_at_ms == 0 || now.saturating_sub(item.updated_at_ms) >= interval
                })
                .map(|item| item.id.clone())
                .collect();
            if stale.is_empty() {
                return;
            }
            for id in stale {
                daemon.refresh_one(&id).await;
            }
            daemon.inner.emit_settings();
            daemon.inner.emit_state();
        });
    }

    async fn measure_latency(&self, id: &str) -> Result<Reply> {
        let profile = lock(&self.inner.store)
            .profile(id)
            .cloned()
            .ok_or_else(|| error(ErrorCode::NotFound, "The profile is not available"))?;
        let protect = self.inner.platform.protect_socket();
        let ms = fiend_app::measure_latency(&app_profile(&profile), &protect)
            .await
            .map_err(from_app)?;
        Ok(Reply::Latency { ms })
    }
}
