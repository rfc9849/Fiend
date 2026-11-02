use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fiend_core::node::control::{Call, Client as ControlClient};
use fiend_core::node::{Health, Stats};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::state::{self, NodeRow};
use crate::{App, now};

const TICK: Duration = Duration::from_secs(5);
const SAMPLES: usize = 60;

#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub ts: i64,
    pub bytes: u64,
    pub online: u64,
    pub origin: u64,
}

#[derive(Clone, Default)]
pub struct Live {
    pub cpu: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub samples: VecDeque<Sample>,
    pub online: HashMap<i64, i64>,
}

impl Live {
    pub fn rate(&self) -> f64 {
        self.samples
            .back()
            .map_or(0.0, |sample| sample.bytes as f64 / TICK.as_secs_f64())
    }
}

struct Handle {
    wake: Arc<Notify>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub struct Sync {
    handles: Mutex<HashMap<i64, Handle>>,
    live: Mutex<HashMap<i64, Live>>,
    pushed_usage: Mutex<i64>,
}

impl Sync {
    pub fn live(&self, node: i64) -> Live {
        self.live
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&node)
            .cloned()
            .unwrap_or_default()
    }

    pub fn all_live(&self) -> HashMap<i64, Live> {
        self.live
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn wake(&self, node: i64) {
        if let Some(handle) = self
            .handles
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&node)
        {
            handle.wake.notify_one();
        }
    }

    pub fn wake_all(&self) {
        for handle in self
            .handles
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
        {
            handle.wake.notify_one();
        }
    }

    pub fn remove(&self, node: i64) {
        if let Some(handle) = self
            .handles
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&node)
        {
            handle.task.abort();
        }
        self.live
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&node);
    }

    pub fn ensure(&self, app: &App, node: i64) {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if handles
            .get(&node)
            .is_some_and(|handle| !handle.task.is_finished())
        {
            return;
        }
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(run(app.clone(), node, wake.clone()));
        handles.insert(node, Handle { wake, task });
    }
}

pub fn start(app: App) {
    let supervisor = app.clone();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(30));
        loop {
            timer.tick().await;
            if let Ok(nodes) = state::nodes(&supervisor).await {
                for node in nodes {
                    supervisor.sync.ensure(&supervisor, node.id);
                }
            }
        }
    });
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(60));
        loop {
            timer.tick().await;
            if let Err(error) = maintenance(&app).await {
                tracing::warn!(%error, "panel maintenance failed");
            }
        }
    });
}

pub async fn connect(app: &App, node: &NodeRow) -> io::Result<ControlClient> {
    if !node.local {
        return Err(io::Error::other("only the local node is supported"));
    }
    let settings = fiend_core::config::load_server(&app.config.local_node)
        .map_err(|error| io::Error::other(format!("local node config: {error}")))?;
    #[cfg(unix)]
    {
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::UnixStream::connect(&settings.node.control_socket),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "local control socket timed out"))??;
        ControlClient::handshake(stream).await
    }
    #[cfg(not(unix))]
    {
        let _ = settings;
        Err(io::Error::other("local control needs Unix"))
    }
}

pub async fn call(app: &App, node_id: i64, call: Call) -> io::Result<serde_json::Value> {
    let node = state::node(app, node_id)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "node not found"))?;
    let mut client = connect(app, &node).await?;
    client.call(call).await
}

async fn run(app: App, node_id: i64, wake: Arc<Notify>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let node = match state::node(&app, node_id).await {
            Ok(node) => node,
            Err(_) => return,
        };
        match connect(&app, &node).await {
            Ok(mut client) => {
                backoff = Duration::from_secs(1);
                if let Err(error) = session(&app, node_id, &mut client, &wake).await {
                    mark(&app, node_id, "offline", Some(error.to_string())).await;
                }
            }
            Err(error) => mark(&app, node_id, "offline", Some(error.to_string())).await,
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {},
            _ = wake.notified() => {},
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

async fn register_warp(app: &App, node_id: i64, client: &mut ControlClient) -> io::Result<()> {
    let status = client.call(Call::WarpStatus).await?;
    if status
        .get("registered")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(());
    }
    match client.call(Call::WarpRegister).await {
        Ok(status) => registered(app, node_id, status).await,
        Err(error) => {
            tracing::warn!(node = node_id, %error, "cannot register WARP");
            let target = node_target(app, node_id).await;
            app.audit(
                None,
                "WARP registration failed",
                &target,
                &error.to_string(),
            )
            .await;
        }
    }
    Ok(())
}

pub async fn registered(app: &App, node_id: i64, mut status: serde_json::Value) {
    status["checked_at"] = now().into();
    let result = sqlx::query("UPDATE warp SET status = ?, exit_v4 = ?, exit_v6 = ?, checked_at = ?, rotated_at = ? WHERE node_id = ?")
        .bind(status.to_string())
        .bind(status.pointer("/v4/ip").and_then(serde_json::Value::as_str))
        .bind(status.pointer("/v6/ip").and_then(serde_json::Value::as_str))
        .bind(now())
        .bind(now())
        .bind(node_id)
        .execute(&app.db)
        .await;
    if let Err(error) = result {
        tracing::warn!(node = node_id, %error, "cannot save WARP status");
    }
    let target = node_target(app, node_id).await;
    app.audit(None, "WARP registered", &target, "").await;
    app.emit("nodes", Some(node_id.to_string()));
}

async fn node_target(app: &App, node_id: i64) -> String {
    match state::node(app, node_id).await {
        Ok(node) => format!("node {}", node.name),
        Err(_) => format!("node {node_id}"),
    }
}

async fn mark(app: &App, node: i64, status: &str, error: Option<String>) {
    let changed: Option<String> = sqlx::query_scalar("SELECT status FROM nodes WHERE id = ?")
        .bind(node)
        .fetch_optional(&app.db)
        .await
        .ok()
        .flatten();
    let _ = sqlx::query("UPDATE nodes SET status = ?, last_error = ? WHERE id = ?")
        .bind(status)
        .bind(error)
        .bind(node)
        .execute(&app.db)
        .await;
    if changed.as_deref() != Some(status) {
        app.emit("nodes", Some(node.to_string()));
    }
}

async fn session(
    app: &App,
    node_id: i64,
    client: &mut ControlClient,
    wake: &Notify,
) -> io::Result<()> {
    loop {
        let health: Health =
            serde_json::from_value(client.call(Call::Health).await?).map_err(io::Error::other)?;
        let node = state::node(app, node_id)
            .await
            .map_err(|_| io::Error::other("node removed"))?;
        sqlx::query("UPDATE nodes SET version = ?, arch = ?, last_seen = ?, started_at = ? WHERE id = ?")
            .bind(&health.version)
            .bind(&health.architecture)
            .bind(now())
            .bind(health.started_at as i64)
            .bind(node_id)
            .execute(&app.db)
            .await
            .map_err(io::Error::other)?;
        if node.local && health.public_port != 0 && i64::from(health.public_port) != node.port {
            sqlx::query("UPDATE nodes SET port = ? WHERE id = ?")
                .bind(i64::from(health.public_port))
                .bind(node_id)
                .execute(&app.db)
                .await
                .map_err(io::Error::other)?;
            tracing::info!(
                node = node_id,
                port = health.public_port,
                "local node public port changed"
            );
            app.emit("nodes", Some(node_id.to_string()));
        }
        let mut status = "online";
        let mut last_error = None;
        match state::desired(app, &node).await {
            Ok(Some(mut desired)) => {
                let in_sync = node.applied_version == desired.version as i64
                    && health.state_version == desired.version;
                if !in_sync && desired.version <= health.state_version {
                    crate::db::raise_version(&app.db, health.state_version as i64 + 1)
                        .await
                        .map_err(io::Error::other)?;
                    desired.version = crate::db::version(&app.db).await as u64;
                }
                if !in_sync {
                    match client.call(Call::ApplyState(desired.clone())).await {
                        Ok(_) => {
                            sqlx::query("UPDATE nodes SET applied_version = ? WHERE id = ?")
                                .bind(desired.version as i64)
                                .bind(node_id)
                                .execute(&app.db)
                                .await
                                .map_err(io::Error::other)?;
                            app.emit("nodes", Some(node_id.to_string()));
                            if desired.warp.enabled {
                                register_warp(app, node_id, client).await?;
                            }
                        }
                        Err(error) => {
                            status = "error";
                            last_error = Some(format!("The node refused the settings: {error}"));
                        }
                    }
                }
            }
            Ok(None) => {
                status = "setup";
                last_error = Some("Select a disguise for this node.".into());
            }
            Err(error) => {
                status = "error";
                last_error = Some(error.message);
            }
        }
        let stats: Stats =
            serde_json::from_value(client.call(Call::Stats).await?).map_err(io::Error::other)?;
        ingest(app, node_id, &stats)
            .await
            .map_err(io::Error::other)?;
        mark(app, node_id, status, last_error).await;
        app.emit("stats", Some(node_id.to_string()));
        tokio::select! {
            _ = tokio::time::sleep(TICK) => {},
            _ = wake.notified() => {},
        }
    }
}

async fn ingest(app: &App, node_id: i64, stats: &Stats) -> Result<(), sqlx::Error> {
    let at = now();
    let minute = at - at % 60;
    let mut bytes = 0u64;
    let mut online = HashMap::new();
    let mut changed = false;
    let mut transaction = app.db.begin().await?;
    for row in &stats.users {
        let Some(user_id) = state::parse_user_id(&row.id) else {
            continue;
        };
        let up = row.up.min(i64::MAX as u64) as i64;
        let down = row.down.min(i64::MAX as u64) as i64;
        bytes = bytes.saturating_add(row.up).saturating_add(row.down);
        online.insert(user_id, row.open_sessions as i64);
        if up > 0 || down > 0 {
            sqlx::query("UPDATE users SET used_up = used_up + ?, used_down = used_down + ?, last_seen = ? WHERE id = ?")
                .bind(up)
                .bind(down)
                .bind(at)
                .bind(user_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("INSERT INTO traffic_minute (ts, node_id, user_id, up, down) VALUES (?, ?, ?, ?, ?) ON CONFLICT(ts, node_id, user_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down")
                .bind(minute)
                .bind(node_id)
                .bind(user_id)
                .bind(up)
                .bind(down)
                .execute(&mut *transaction)
                .await?;
        } else if row.open_sessions > 0 {
            sqlx::query("UPDATE users SET last_seen = ? WHERE id = ?")
                .bind(at)
                .bind(user_id)
                .execute(&mut *transaction)
                .await?;
        }
        if row.new_sessions > 0 {
            let started = sqlx::query("UPDATE users SET expire_at = ? + hold_days * 86400 WHERE id = ? AND expire_at IS NULL AND hold_days IS NOT NULL")
                .bind(at)
                .bind(user_id)
                .execute(&mut *transaction)
                .await?;
            changed |= started.rows_affected() > 0;
        }
    }
    let system = &stats.system;
    sqlx::query("INSERT INTO node_metrics (ts, node_id, cpu, memory_used, memory_total, rx, tx, handshakes_ok, handshakes_origin) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(ts, node_id) DO UPDATE SET cpu = excluded.cpu, memory_used = excluded.memory_used, memory_total = excluded.memory_total, rx = rx + excluded.rx, tx = tx + excluded.tx, handshakes_ok = handshakes_ok + excluded.handshakes_ok, handshakes_origin = handshakes_origin + excluded.handshakes_origin")
        .bind(minute)
        .bind(node_id)
        .bind(f64::from(system.cpu_percent))
        .bind(system.memory_used as i64)
        .bind(system.memory_total as i64)
        .bind(system.network_rx as i64)
        .bind(system.network_tx as i64)
        .bind(stats.handshakes_accepted as i64)
        .bind(stats.handshakes_origin as i64)
        .execute(&mut *transaction)
        .await?;
    for probe in &stats.probes {
        sqlx::query("INSERT INTO probes (ts, node_id, peer, reason, sni, size, head) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(probe.at.min(i64::MAX as u64) as i64)
            .bind(node_id)
            .bind(&probe.peer)
            .bind(&probe.reason)
            .bind(&probe.sni)
            .bind(probe.size.min(i64::MAX as u64) as i64)
            .bind(hex::decode(&probe.head).unwrap_or_default())
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    {
        let mut live = app
            .sync
            .live
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = live.entry(node_id).or_default();
        if system.available {
            entry.cpu = Some(f64::from(system.cpu_percent));
            entry.memory_used = Some(system.memory_used as i64);
            entry.memory_total = Some(system.memory_total as i64);
        }
        entry.online = online;
        entry.samples.push_back(Sample {
            ts: at,
            bytes,
            online: entry.online.values().filter(|count| **count > 0).count() as u64,
            origin: stats.handshakes_origin,
        });
        while entry.samples.len() > SAMPLES {
            entry.samples.pop_front();
        }
    }
    let totals = {
        let live = app
            .sync
            .live
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut totals: HashMap<i64, i64> = HashMap::new();
        for node in live.values() {
            for (user, count) in &node.online {
                *totals.entry(*user).or_default() += count;
            }
        }
        totals
    };
    sqlx::query("UPDATE users SET online = 0")
        .execute(&app.db)
        .await?;
    for (user, count) in totals {
        sqlx::query("UPDATE users SET online = ? WHERE id = ?")
            .bind(count)
            .bind(user)
            .execute(&app.db)
            .await?;
    }
    if changed || limits_changed(app).await? {
        crate::db::bump_version(&app.db).await?;
        app.sync.wake_all();
        app.emit("users", None);
    }
    Ok(())
}

async fn limits_changed(app: &App) -> Result<bool, sqlx::Error> {
    let at = now();
    let users = state::users(app)
        .await
        .map_err(|error| sqlx::Error::Protocol(error.message))?;
    let fingerprint: Vec<(i64, bool)> = users
        .iter()
        .map(|user| (user.id, user.active(at)))
        .collect();
    let text = serde_json::to_string(&fingerprint).unwrap_or_default();
    let previous = crate::db::setting(&app.db, "active_users").await;
    if previous.as_deref() != Some(text.as_str()) {
        crate::db::set_setting(&app.db, "active_users", &text).await?;
        if previous.is_some() {
            return Ok(true);
        }
    }
    let limited = users.iter().any(|user| user.traffic_limit.is_some());
    let mut pushed = app
        .sync
        .pushed_usage
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if limited && at - *pushed >= 60 {
        *pushed = at;
        return Ok(true);
    }
    Ok(false)
}

pub fn next_reset(period: &str, from: i64) -> Option<i64> {
    let day = 86400;
    match period {
        "day" => Some(from - from % day + day),
        "week" => {
            let days = from / day;
            let weekday = (days + 3) % 7;
            Some((days - weekday + 7) * day)
        }
        "month" => {
            let (year, month, _) = civil(from / day);
            let (year, month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            Some(days_from_civil(year, month, 1) * day)
        }
        _ => None,
    }
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub async fn maintenance(app: &App) -> Result<(), sqlx::Error> {
    let at = now();
    let due: Vec<(i64, String)> = sqlx::query_as("SELECT id, reset_period FROM users WHERE reset_period != 'none' AND reset_at IS NOT NULL AND reset_at <= ?")
        .bind(at)
        .fetch_all(&app.db)
        .await?;
    for (id, period) in &due {
        sqlx::query("UPDATE users SET used_up = 0, used_down = 0, usage_epoch = usage_epoch + 1, reset_at = ? WHERE id = ?")
            .bind(next_reset(period, at))
            .bind(id)
            .execute(&app.db)
            .await?;
        app.audit(None, "usage reset", &format!("user {id}"), period)
            .await;
    }
    if !due.is_empty() {
        crate::db::bump_version(&app.db).await?;
        app.sync.wake_all();
        app.emit("users", None);
    }
    let hour = at - at % 3600;
    let day = at - at % 86400;
    sqlx::query("INSERT INTO traffic_hour (ts, node_id, user_id, up, down) SELECT ts - ts % 3600, node_id, user_id, SUM(up), SUM(down) FROM traffic_minute WHERE ts >= ? - 7200 AND ts < ? GROUP BY ts - ts % 3600, node_id, user_id ON CONFLICT(ts, node_id, user_id) DO UPDATE SET up = excluded.up, down = excluded.down")
        .bind(hour)
        .bind(hour)
        .execute(&app.db)
        .await?;
    sqlx::query("INSERT INTO traffic_day (ts, node_id, user_id, up, down) SELECT ts - ts % 86400, node_id, user_id, SUM(up), SUM(down) FROM traffic_hour WHERE ts >= ? - 172800 AND ts < ? GROUP BY ts - ts % 86400, node_id, user_id ON CONFLICT(ts, node_id, user_id) DO UPDATE SET up = excluded.up, down = excluded.down")
        .bind(day)
        .bind(day)
        .execute(&app.db)
        .await?;
    for (table, keep) in [
        ("traffic_minute", 48 * 3600),
        ("traffic_hour", 90 * 86400),
        ("traffic_day", 730 * 86400),
        ("node_metrics", 30 * 86400),
        ("audit", 365 * 86400),
        ("probes", 30 * 86400),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE ts < ?"
        )))
        .bind(at - keep)
        .execute(&app.db)
        .await?;
    }
    sqlx::query("DELETE FROM probes WHERE id <= (SELECT id FROM probes ORDER BY id DESC LIMIT 1 OFFSET 50000)")
        .execute(&app.db)
        .await?;
    let users = state::users(app)
        .await
        .map_err(|error| sqlx::Error::Protocol(error.message))?;
    let expired = users.iter().any(|user| {
        user.expire_at
            .is_some_and(|expiry| expiry <= at && expiry > at - 60)
    });
    if expired {
        crate::db::bump_version(&app.db).await?;
        app.sync.wake_all();
        app.emit("users", None);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resets_fall_on_calendar_boundaries() {
        let at = days_from_civil(2026, 10, 31) * 86400 + 3600;
        assert_eq!(
            next_reset("day", at),
            Some(days_from_civil(2026, 11, 1) * 86400)
        );
        assert_eq!(
            next_reset("month", at),
            Some(days_from_civil(2026, 11, 1) * 86400)
        );
        let december = days_from_civil(2026, 12, 15) * 86400;
        assert_eq!(
            next_reset("month", december),
            Some(days_from_civil(2027, 1, 1) * 86400)
        );
        let week = next_reset("week", at).unwrap();
        assert_eq!((week / 86400 + 3) % 7, 0);
        assert!(week > at && week - at <= 7 * 86400);
        assert_eq!(next_reset("none", at), None);
        assert_eq!(civil(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }
}
