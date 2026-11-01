use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fiend_core::node::control::Call;
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::{self, Admin};
use crate::state::{self, DisguiseRow, NodeRow, OutboundRow, RuleRow, UserRow};
use crate::types::*;
use crate::{ApiError, ApiResult, App, now};

const BUNDLED: &str = include_str!("../../fiend-core/testdata/template.json");

pub fn routes() -> Router<App> {
    Router::new()
        .route("/setup/{token}", get(auth::setup_status))
        .route("/setup/{token}/totp", get(auth::setup_totp))
        .route("/setup", post(auth::setup_admin))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/session", get(auth::session))
        .route("/events", get(events))
        .route("/wizard/local", get(local_defaults).post(create_local))
        .route("/wizard/done", post(wizard_done))
        .route("/dashboard", get(dashboard))
        .route("/traffic", get(traffic))
        .route("/probes", get(probes))
        .route("/nodes", get(list_nodes))
        .route(
            "/nodes/{id}",
            get(get_node).put(edit_node).delete(delete_node),
        )
        .route("/nodes/{id}/restart", post(restart_node))
        .route("/nodes/{id}/logs", get(node_logs))
        .route(
            "/nodes/{id}/warp",
            get(warp_status).put(warp_settings).delete(warp_delete),
        )
        .route("/nodes/{id}/warp/register", post(warp_register))
        .route("/nodes/{id}/warp/rotate", post(warp_rotate))
        .route("/users", get(list_users).post(create_user))
        .route(
            "/users/{id}",
            get(get_user).put(edit_user).delete(delete_user),
        )
        .route("/users/{id}/link", get(user_link))
        .route(
            "/users/{id}/secret",
            get(user_secret).post(regenerate_secret),
        )
        .route("/users/{id}/reset", post(reset_usage))
        .route(
            "/users/{id}/sessions",
            get(user_sessions).delete(kill_sessions),
        )
        .route("/disguises", get(list_disguises).post(create_disguise))
        .route("/disguises/presets", get(presets))
        .route("/disguises/{id}", delete(delete_disguise))
        .route("/disguises/{id}/probe", post(probe))
        .route("/routing", get(routing))
        .route("/outbounds", post(create_outbound))
        .route(
            "/outbounds/{id}",
            put(edit_outbound).delete(delete_outbound),
        )
        .route("/rules", post(create_rule))
        .route("/rules/order", post(order_rules))
        .route("/rules/{id}", put(edit_rule).delete(delete_rule))
        .route("/admins", get(list_admins).post(create_admin))
        .route("/admins/{id}", delete(delete_admin))
        .route("/account/password", post(change_password))
        .route(
            "/account/totp",
            get(new_totp).post(enable_totp).delete(disable_totp),
        )
        .route("/backup", post(backup))
        .route(
            "/restore",
            post(restore).layer(DefaultBodyLimit::max(256 * 1024 * 1024)),
        )
        .route("/audit", get(audit))
}

async fn changed(app: &App) -> ApiResult<()> {
    crate::db::bump_version(&app.db).await?;
    app.sync.wake_all();
    app.emit("nodes", None);
    Ok(())
}

fn control_error(error: std::io::Error) -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        format!("The node did not complete the request: {error}"),
    )
}

async fn events(
    State(app): State<App>,
    _admin: Admin,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(app.events.subscribe()).filter_map(|event| {
        let event = event.ok()?;
        Some(Ok(SseEvent::default()
            .event(event.kind.clone())
            .json_data(&event)
            .ok()?))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn local_defaults(
    State(app): State<App>,
    _admin: Admin,
) -> ApiResult<Json<LocalNodeDefaults>> {
    let settings = fiend_core::config::load_server(&app.config.local_node);
    let mut defaults = LocalNodeDefaults {
        available: false,
        port: 443,
        origin_host: "www.wikipedia.org".into(),
        origin_port: 443,
        address: crate::address::detect()
            .await
            .map(|address| address.to_string()),
        error: None,
    };
    match settings {
        Ok(settings) => {
            defaults.port = settings.listen.port();
            defaults.origin_host = settings.config.origin_host.clone();
            defaults.origin_port = settings.config.origin_port;
            match local_state(&app).await {
                Ok(desired) => {
                    defaults.available = true;
                    defaults.origin_host = desired.origin_host;
                    defaults.origin_port = desired.origin_port;
                }
                Err(error) => {
                    defaults.error = Some(format!("The local node does not answer: {error}"))
                }
            }
        }
        Err(error) => {
            defaults.error = Some(format!("The local node config cannot be read: {error}"))
        }
    }
    Ok(Json(defaults))
}

async fn local_state(app: &App) -> std::io::Result<fiend_core::node::DesiredState> {
    let probe = NodeRow {
        id: 0,
        name: "local".into(),
        address: "127.0.0.1".into(),
        port: 0,
        local: true,
        arch: None,
        control_secret: String::new(),
        service_secret: String::new(),
        version: None,
        applied_version: 0,
        status: String::new(),
        last_error: None,
        last_seen: None,
        started_at: None,
        disguise_id: None,
        created_at: 0,
    };
    let mut client = crate::sync::connect(app, &probe).await?;
    serde_json::from_value(client.call(Call::State).await?).map_err(std::io::Error::other)
}

fn check_address(address: &str) -> ApiResult<()> {
    let authority = state::authority(address, 443);
    fiend_core::share::check_server(&authority)
        .map_err(|_| ApiError::field("address", "Enter a public IP address or a host name."))
}

async fn create_local(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<LocalNodeInput>,
) -> ApiResult<Json<Created>> {
    auth::check_name(&input.name, "name")?;
    check_address(&input.address)?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE local = 1")
        .fetch_one(&app.db)
        .await?;
    if exists > 0 {
        return Err(ApiError::conflict("The local node is already added."));
    }
    let settings = fiend_core::config::load_server(&app.config.local_node)
        .map_err(|error| ApiError::bad(format!("The local node config cannot be read: {error}")))?;
    let control = std::fs::read_to_string(&settings.node.control_secret_file)
        .map_err(|_| ApiError::bad("The local control secret cannot be read."))?;
    fiend_core::share::decode_secret(control.trim())
        .map_err(|_| ApiError::bad("The local control secret is not valid."))?;
    let desired = local_state(&app).await.map_err(control_error)?;
    let mut transaction = app.db.begin().await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO nodes (name, address, port, local, control_secret, service_secret, status, created_at) VALUES (?, ?, ?, 1, ?, ?, 'unknown', ?) RETURNING id")
        .bind(&input.name)
        .bind(&input.address)
        .bind(i64::from(settings.listen.port()))
        .bind(control.trim())
        .bind(state::new_secret())
        .bind(now())
        .fetch_one(&mut *transaction)
        .await?;
    crate::db::set_setting_tx(&mut transaction, "local_origin", &serde_json::json!({
        "host": desired.origin_host, "port": desired.origin_port, "profile": desired.origin_profile,
    }).to_string()).await?;
    transaction.commit().await?;
    crate::db::raise_version(&app.db, desired.version as i64 + 1).await?;
    app.audit(
        Some(admin.id),
        "node added",
        &format!("node {}", input.name),
        "local node",
    )
    .await;
    app.sync.ensure(&app, id);
    app.emit("nodes", Some(id.to_string()));
    Ok(Json(Created { id }))
}

async fn wizard_done(State(app): State<App>, admin: Admin) -> ApiResult<StatusCode> {
    crate::db::set_setting(&app.db, "wizard_done", "1").await?;
    app.audit(Some(admin.id), "setup finished", "panel", "")
        .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn node_view(
    app: &App,
    node: &NodeRow,
    live: &HashMap<i64, crate::sync::Live>,
) -> ApiResult<NodeView> {
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE NOT EXISTS (SELECT 1 FROM user_nodes WHERE user_nodes.user_id = users.id) OR EXISTS (SELECT 1 FROM user_nodes WHERE user_nodes.user_id = users.id AND user_nodes.node_id = ?)")
        .bind(node.id)
        .fetch_one(&app.db)
        .await?;
    let live = live.get(&node.id).cloned().unwrap_or_default();
    let fresh = node.last_seen.is_some_and(|seen| now() - seen < 30);
    let status = if node.status == "online" && !fresh {
        "offline".to_owned()
    } else {
        node.status.clone()
    };
    Ok(NodeView {
        id: node.id,
        name: node.name.clone(),
        address: node.address.clone(),
        port: node.port as u16,
        local: node.local,
        arch: node.arch.clone(),
        version: node.version.clone(),
        status,
        last_error: node.last_error.clone(),
        last_seen: node.last_seen,
        started_at: node.started_at,
        cpu: live.cpu,
        memory_used: live.memory_used,
        memory_total: live.memory_total,
        traffic_rate: live.rate(),
        applied_version: node.applied_version,
        desired_version: crate::db::version(&app.db).await,
        disguise_id: node.disguise_id,
        users,
    })
}

async fn node_views(app: &App) -> ApiResult<Vec<NodeView>> {
    let live = app.sync.all_live();
    let mut views = Vec::new();
    for node in state::nodes(app).await? {
        views.push(node_view(app, &node, &live).await?);
    }
    Ok(views)
}

async fn dashboard(State(app): State<App>, _admin: Admin) -> ApiResult<Json<Dashboard>> {
    let nodes = node_views(&app).await?;
    let live = app.sync.all_live();
    let users_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&app.db)
        .await?;
    let users_online: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE online > 0")
        .fetch_one(&app.db)
        .await?;
    let origin_recent: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(handshakes_origin), 0) FROM node_metrics WHERE ts >= ?",
    )
    .bind(now() - 3600)
    .fetch_one(&app.db)
    .await?;
    let mut traffic = vec![0.0; 60];
    let mut online = vec![0.0; 60];
    let mut node_samples = vec![0.0; 60];
    let mut origin = vec![0.0; 60];
    for entry in live.values() {
        let offset = 60 - entry.samples.len();
        for (index, sample) in entry.samples.iter().enumerate() {
            traffic[offset + index] += sample.bytes as f64 / 5.0;
            online[offset + index] += sample.online as f64;
            node_samples[offset + index] += 1.0;
            origin[offset + index] += sample.origin as f64;
        }
    }
    Ok(Json(Dashboard {
        traffic_rate: nodes.iter().map(|node| node.traffic_rate).sum(),
        users_online,
        users_total,
        nodes_online: nodes.iter().filter(|node| node.status == "online").count() as i64,
        nodes_total: nodes.len() as i64,
        origin_recent,
        traffic_samples: traffic,
        online_samples: online,
        node_samples,
        origin_samples: origin,
        nodes,
    }))
}

#[derive(Deserialize)]
struct TrafficQuery {
    range: Option<String>,
    node: Option<i64>,
    user: Option<i64>,
}

async fn traffic(
    State(app): State<App>,
    _admin: Admin,
    Query(query): Query<TrafficQuery>,
) -> ApiResult<Json<TrafficSeries>> {
    let range = query.range.unwrap_or_else(|| "24h".into());
    let (table, span, step) = match range.as_str() {
        "24h" => ("traffic_minute", 86400, 3600),
        "7d" => ("traffic_hour", 7 * 86400, 6 * 3600),
        "30d" => ("traffic_hour", 30 * 86400, 86400),
        _ => return Err(ApiError::bad("Unknown range")),
    };
    let from = now() - span;
    let start = from - from % step;
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT ts - ts % ?, SUM(up), SUM(down) FROM {table} WHERE ts >= ? AND (? IS NULL OR node_id = ?) AND (? IS NULL OR user_id = ?) GROUP BY ts - ts % ? ORDER BY 1"
    )))
    .bind(step)
    .bind(start)
    .bind(query.node)
    .bind(query.node)
    .bind(query.user)
    .bind(query.user)
    .bind(step)
    .fetch_all(&app.db)
    .await?;
    let found: HashMap<i64, (i64, i64)> = rows
        .into_iter()
        .map(|(ts, up, down)| (ts, (up, down)))
        .collect();
    let mut points = Vec::new();
    let mut ts = start;
    while ts <= now() {
        let (up, down) = found.get(&ts).copied().unwrap_or((0, 0));
        points.push(TrafficPoint { ts, up, down });
        ts += step;
    }
    Ok(Json(TrafficSeries {
        range,
        step,
        points,
    }))
}

async fn list_nodes(State(app): State<App>, _admin: Admin) -> ApiResult<Json<Vec<NodeView>>> {
    Ok(Json(node_views(&app).await?))
}

async fn get_node(
    State(app): State<App>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<NodeView>> {
    let node = state::node(&app, id).await?;
    Ok(Json(node_view(&app, &node, &app.sync.all_live()).await?))
}

async fn edit_node(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Json(input): Json<NodeInput>,
) -> ApiResult<Json<NodeView>> {
    auth::check_name(&input.name, "name")?;
    check_address(&input.address)?;
    let node = state::node(&app, id).await?;
    if input.port != node.port as u16 {
        return Err(ApiError::field(
            "port",
            "A port change needs a node reinstall.",
        ));
    }
    if let Some(disguise) = input.disguise_id {
        state::disguise(&app, disguise).await?;
    }
    sqlx::query("UPDATE nodes SET name = ?, address = ?, disguise_id = ? WHERE id = ?")
        .bind(&input.name)
        .bind(&input.address)
        .bind(input.disguise_id)
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "node changed",
        &format!("node {}", input.name),
        &format!("address {}", input.address),
    )
    .await;
    changed(&app).await?;
    get_node(State(app), admin, Path(id)).await
}

async fn delete_node(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let node = state::node(&app, id).await?;
    let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbounds WHERE kind = 'fiend' AND json_extract(config, '$.node_id') = ?")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict("Other nodes chain to this node."));
    }
    app.sync.remove(id);
    sqlx::query("DELETE FROM nodes WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "node removed",
        &format!("node {}", node.name),
        "the node software stays on the server",
    )
    .await;
    changed(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn restart_node(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let node = state::node(&app, id).await?;
    crate::sync::call(&app, id, Call::Restart)
        .await
        .map_err(control_error)?;
    app.audit(
        Some(admin.id),
        "node restarted",
        &format!("node {}", node.name),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn node_logs(
    State(app): State<App>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<NodeLogs>> {
    let value = crate::sync::call(&app, id, Call::Logs { limit: 200 })
        .await
        .map_err(control_error)?;
    let lines = value
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| {
                    line.as_str()
                        .map(str::to_owned)
                        .or_else(|| Some(line.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(NodeLogs { lines }))
}

fn warp_view(
    row: Option<(bool, Option<i64>, Option<String>)>,
    status: Option<serde_json::Value>,
    error: Option<String>,
) -> WarpView {
    let (enabled, rotate_every, saved) = row.unwrap_or((false, None, None));
    let status = status
        .or_else(|| saved.and_then(|text| serde_json::from_str(&text).ok()))
        .unwrap_or_default();
    let text = |pointer: &str| {
        status
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    WarpView {
        enabled,
        registered: status
            .get("registered")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        running: status
            .get("running")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        exit_v4: text("/v4/ip"),
        exit_v6: text("/v6/ip"),
        colo: text("/v4/colo").or_else(|| text("/v6/colo")),
        warp_on: text("/v4/warp").as_deref() == Some("on")
            || text("/v6/warp").as_deref() == Some("on"),
        created_at: status.get("created_at").and_then(serde_json::Value::as_i64),
        rotated_at: status.get("rotated_at").and_then(serde_json::Value::as_i64),
        rotate_every,
        error: error.or_else(|| text("/error")),
        checked_at: status.get("checked_at").and_then(serde_json::Value::as_i64),
    }
}

async fn warp_row(app: &App, id: i64) -> ApiResult<Option<(bool, Option<i64>, Option<String>)>> {
    Ok(
        sqlx::query_as("SELECT enabled, rotate_every, status FROM warp WHERE node_id = ?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?,
    )
}

async fn save_warp_status(
    app: &App,
    id: i64,
    mut status: serde_json::Value,
) -> ApiResult<serde_json::Value> {
    status["checked_at"] = now().into();
    sqlx::query("INSERT INTO warp (node_id, status, exit_v4, exit_v6, checked_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(node_id) DO UPDATE SET status = excluded.status, exit_v4 = excluded.exit_v4, exit_v6 = excluded.exit_v6, checked_at = excluded.checked_at")
        .bind(id)
        .bind(status.to_string())
        .bind(status.pointer("/v4/ip").and_then(serde_json::Value::as_str))
        .bind(status.pointer("/v6/ip").and_then(serde_json::Value::as_str))
        .bind(now())
        .execute(&app.db)
        .await?;
    Ok(status)
}

async fn warp_status(
    State(app): State<App>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<WarpView>> {
    state::node(&app, id).await?;
    match crate::sync::call(&app, id, Call::WarpStatus).await {
        Ok(status) => {
            let status = save_warp_status(&app, id, status).await?;
            Ok(Json(warp_view(
                warp_row(&app, id).await?,
                Some(status),
                None,
            )))
        }
        Err(error) => Ok(Json(warp_view(
            warp_row(&app, id).await?,
            None,
            Some(format!("The node does not answer: {error}")),
        ))),
    }
}

async fn warp_settings(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Json(input): Json<WarpSettingsInput>,
) -> ApiResult<Json<WarpView>> {
    let node = state::node(&app, id).await?;
    if let Some(every) = input.rotate_every
        && ![6 * 3600, 12 * 3600, 24 * 3600, 7 * 86400].contains(&every)
    {
        return Err(ApiError::field(
            "rotate_every",
            "Choose off, 6 h, 12 h, 24 h, or 7 d.",
        ));
    }
    sqlx::query("INSERT INTO warp (node_id, enabled, rotate_every, chosen) VALUES (?, ?, ?, 1) ON CONFLICT(node_id) DO UPDATE SET enabled = excluded.enabled, rotate_every = excluded.rotate_every, chosen = 1")
        .bind(id)
        .bind(input.enabled)
        .bind(input.rotate_every)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "WARP settings",
        &format!("node {}", node.name),
        &match (input.enabled, input.rotate_every) {
            (false, _) => "off".to_owned(),
            (true, None) => "on".to_owned(),
            (true, Some(every)) if every % 86400 == 0 => {
                format!("on, New IP every {} d", every / 86400)
            }
            (true, Some(every)) => format!("on, New IP every {} h", every / 3600),
        },
    )
    .await;
    changed(&app).await?;
    if input.enabled {
        ensure_warp(&app, id).await?;
    }
    Ok(Json(warp_view(warp_row(&app, id).await?, None, None)))
}

async fn ensure_warp(app: &App, id: i64) -> ApiResult<()> {
    wait_applied(app, id).await;
    let status = crate::sync::call(app, id, Call::WarpStatus)
        .await
        .map_err(control_error)?;
    if status
        .get("registered")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        save_warp_status(app, id, status).await?;
        return Ok(());
    }
    let status = match crate::sync::call(app, id, Call::WarpRegister).await {
        Ok(status) => status,
        Err(error) => {
            let status = crate::sync::call(app, id, Call::WarpStatus)
                .await
                .map_err(control_error)?;
            if !status
                .get("registered")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                return Err(control_error(error));
            }
            status
        }
    };
    crate::sync::registered(app, id, status).await;
    Ok(())
}

async fn enable_warp_for(app: &App, node_id: Option<i64>) -> ApiResult<()> {
    let nodes: Vec<i64> = match node_id {
        Some(id) => vec![id],
        None => state::nodes(app)
            .await?
            .iter()
            .map(|node| node.id)
            .collect(),
    };
    for id in &nodes {
        sqlx::query("INSERT INTO warp (node_id, enabled, chosen) VALUES (?, 1, 1) ON CONFLICT(node_id) DO UPDATE SET enabled = 1, chosen = 1")
            .bind(id)
            .execute(&app.db)
            .await?;
    }
    changed(app).await
}

async fn wait_applied(app: &App, id: i64) {
    let version = crate::db::version(&app.db).await;
    for _ in 0..20 {
        if state::node(app, id)
            .await
            .is_ok_and(|node| node.applied_version >= version)
        {
            return;
        }
        app.sync.wake(id);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn warp_register(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<WarpView>> {
    let node = state::node(&app, id).await?;
    let enabled = warp_row(&app, id).await?.is_some_and(|row| row.0);
    if !enabled {
        return Err(ApiError::bad("Turn on WARP for this node first."));
    }
    wait_applied(&app, id).await;
    let status = crate::sync::call(&app, id, Call::WarpRegister)
        .await
        .map_err(control_error)?;
    let status = save_warp_status(&app, id, status).await?;
    sqlx::query("UPDATE warp SET rotated_at = ? WHERE node_id = ?")
        .bind(now())
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "WARP registered",
        &format!("node {}", node.name),
        "",
    )
    .await;
    Ok(Json(warp_view(
        warp_row(&app, id).await?,
        Some(status),
        None,
    )))
}

async fn warp_rotate(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<WarpRotation>> {
    let node = state::node(&app, id).await?;
    let rotation = crate::sync::call(&app, id, Call::WarpRotate)
        .await
        .map_err(control_error)?;
    let ip = |pointer: &str| {
        rotation
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let after = rotation.get("after").cloned().unwrap_or_default();
    let status = save_warp_status(&app, id, after).await?;
    sqlx::query("UPDATE warp SET rotated_at = ? WHERE node_id = ?")
        .bind(now())
        .bind(id)
        .execute(&app.db)
        .await?;
    let result = WarpRotation {
        before_v4: ip("/before/v4/ip"),
        before_v6: ip("/before/v6/ip"),
        after_v4: ip("/after/v4/ip"),
        after_v6: ip("/after/v6/ip"),
        warp: warp_view(warp_row(&app, id).await?, Some(status), None),
    };
    let shown = |value: &Option<String>| value.clone().unwrap_or_else(|| "none".into());
    app.audit(
        Some(admin.id),
        "WARP new IP",
        &format!("node {}", node.name),
        &format!(
            "IPv4 {} → {}, IPv6 {} → {}",
            shown(&result.before_v4),
            shown(&result.after_v4),
            shown(&result.before_v6),
            shown(&result.after_v6)
        ),
    )
    .await;
    Ok(Json(result))
}

async fn warp_delete(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<WarpView>> {
    let node = state::node(&app, id).await?;
    crate::sync::call(&app, id, Call::WarpDelete)
        .await
        .map_err(control_error)?;
    sqlx::query("UPDATE warp SET status = NULL, exit_v4 = NULL, exit_v6 = NULL, rotated_at = NULL WHERE node_id = ?").bind(id).execute(&app.db).await?;
    app.audit(
        Some(admin.id),
        "WARP account deleted",
        &format!("node {}", node.name),
        "",
    )
    .await;
    Ok(Json(warp_view(warp_row(&app, id).await?, None, None)))
}

fn user_view(user: &UserRow, nodes: &HashMap<i64, Vec<i64>>) -> UserView {
    UserView {
        id: user.id,
        name: user.name.clone(),
        enabled: user.enabled,
        state: user.state(now()).into(),
        note: user.note.clone(),
        traffic_limit: user.traffic_limit,
        used: user.used(),
        expire_at: user.expire_at,
        hold_days: user.hold_days,
        reset_period: user.reset_period.clone(),
        reset_at: user.reset_at,
        last_seen: user.last_seen,
        online: user.online,
        nodes: nodes.get(&user.id).cloned().unwrap_or_default(),
        created_at: user.created_at,
    }
}

async fn list_users(State(app): State<App>, _admin: Admin) -> ApiResult<Json<Vec<UserView>>> {
    let nodes = state::user_nodes(&app).await?;
    Ok(Json(
        state::users(&app)
            .await?
            .iter()
            .map(|user| user_view(user, &nodes))
            .collect(),
    ))
}

async fn check_user(app: &App, input: &UserInput) -> ApiResult<()> {
    auth::check_name(&input.name, "name")?;
    if input.note.len() > 1000 {
        return Err(ApiError::field("note", "Use 1000 characters or fewer."));
    }
    if input.traffic_limit.is_some_and(|limit| limit <= 0) {
        return Err(ApiError::field(
            "traffic_limit",
            "Enter a limit greater than zero, or no limit.",
        ));
    }
    if input
        .hold_days
        .is_some_and(|days| !(1..=3650).contains(&days))
    {
        return Err(ApiError::field("hold_days", "Enter 1 to 3650 days."));
    }
    if input.hold_days.is_some() && input.expire_at.is_some() {
        return Err(ApiError::field(
            "hold_days",
            "Use an expiry date or a start on first connect, not both.",
        ));
    }
    if !["none", "day", "week", "month"].contains(&input.reset_period.as_str()) {
        return Err(ApiError::field("reset_period", "Choose a reset period."));
    }
    for node in &input.nodes {
        state::node(app, *node)
            .await
            .map_err(|_| ApiError::field("nodes", "A selected node is not available."))?;
    }
    Ok(())
}

async fn set_user_nodes(app: &App, user: i64, nodes: &[i64]) -> ApiResult<()> {
    sqlx::query("DELETE FROM user_nodes WHERE user_id = ?")
        .bind(user)
        .execute(&app.db)
        .await?;
    for node in nodes {
        sqlx::query("INSERT OR IGNORE INTO user_nodes (user_id, node_id) VALUES (?, ?)")
            .bind(user)
            .bind(node)
            .execute(&app.db)
            .await?;
    }
    Ok(())
}

async fn create_user(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<UserInput>,
) -> ApiResult<Json<UserView>> {
    check_user(&app, &input).await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO users (name, secret, enabled, note, traffic_limit, expire_at, hold_days, reset_period, reset_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id")
        .bind(&input.name)
        .bind(state::new_secret())
        .bind(input.enabled)
        .bind(&input.note)
        .bind(input.traffic_limit)
        .bind(input.expire_at)
        .bind(input.hold_days)
        .bind(&input.reset_period)
        .bind(crate::sync::next_reset(&input.reset_period, now()))
        .bind(now())
        .fetch_one(&app.db)
        .await
        .map_err(|error| match ApiError::from(error) {
            error if error.status == StatusCode::CONFLICT => ApiError::field("name", "This name is already in use."),
            error => error,
        })?;
    set_user_nodes(&app, id, &input.nodes).await?;
    app.audit(
        Some(admin.id),
        "user added",
        &format!("user {}", input.name),
        "",
    )
    .await;
    changed(&app).await?;
    app.emit("users", Some(id.to_string()));
    let nodes = state::user_nodes(&app).await?;
    Ok(Json(user_view(&state::user(&app, id).await?, &nodes)))
}

async fn get_user(
    State(app): State<App>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserDetail>> {
    let user = state::user(&app, id).await?;
    let nodes = state::user_nodes(&app).await?;
    let usage: Vec<(i64, String, i64, i64)> = sqlx::query_as("SELECT nodes.id, nodes.name, COALESCE(SUM(traffic_day.up), 0) + COALESCE((SELECT SUM(up) FROM traffic_hour WHERE traffic_hour.node_id = nodes.id AND traffic_hour.user_id = ? AND traffic_hour.ts >= ?), 0), COALESCE(SUM(traffic_day.down), 0) + COALESCE((SELECT SUM(down) FROM traffic_hour WHERE traffic_hour.node_id = nodes.id AND traffic_hour.user_id = ? AND traffic_hour.ts >= ?), 0) FROM nodes LEFT JOIN traffic_day ON traffic_day.node_id = nodes.id AND traffic_day.user_id = ? AND traffic_day.ts >= ? AND traffic_day.ts < ? GROUP BY nodes.id ORDER BY nodes.id")
        .bind(id)
        .bind(now() - now() % 86400)
        .bind(id)
        .bind(now() - now() % 86400)
        .bind(id)
        .bind(now() - 30 * 86400)
        .bind(now() - now() % 86400)
        .fetch_all(&app.db)
        .await?;
    let mut usage: Vec<NodeUsage> = usage
        .into_iter()
        .map(|(node_id, node_name, up, down)| NodeUsage {
            node_id,
            node_name,
            up,
            down,
        })
        .collect();
    let recent: Vec<(i64, i64, i64)> = sqlx::query_as("SELECT node_id, SUM(up), SUM(down) FROM traffic_minute WHERE user_id = ? AND ts >= ? GROUP BY node_id")
        .bind(id)
        .bind(now() - now() % 3600)
        .fetch_all(&app.db)
        .await?;
    for (node, up, down) in recent {
        if let Some(row) = usage.iter_mut().find(|row| row.node_id == node) {
            row.up += up;
            row.down += down;
        }
    }
    Ok(Json(UserDetail {
        user: user_view(&user, &nodes),
        usage,
    }))
}

async fn edit_user(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Json(input): Json<UserInput>,
) -> ApiResult<Json<UserView>> {
    let previous = state::user(&app, id).await?;
    check_user(&app, &input).await?;
    let reset_at = if input.reset_period == previous.reset_period {
        previous.reset_at
    } else {
        crate::sync::next_reset(&input.reset_period, now())
    };
    let expire_at = if input.hold_days.is_some() && previous.hold_days == input.hold_days {
        previous.expire_at
    } else {
        input.expire_at
    };
    sqlx::query("UPDATE users SET name = ?, enabled = ?, note = ?, traffic_limit = ?, expire_at = ?, hold_days = ?, reset_period = ?, reset_at = ? WHERE id = ?")
        .bind(&input.name)
        .bind(input.enabled)
        .bind(&input.note)
        .bind(input.traffic_limit)
        .bind(expire_at)
        .bind(input.hold_days)
        .bind(&input.reset_period)
        .bind(reset_at)
        .bind(id)
        .execute(&app.db)
        .await
        .map_err(|error| match ApiError::from(error) {
            error if error.status == StatusCode::CONFLICT => ApiError::field("name", "This name is already in use."),
            error => error,
        })?;
    set_user_nodes(&app, id, &input.nodes).await?;
    app.audit(
        Some(admin.id),
        "user changed",
        &format!("user {}", input.name),
        &format!("enabled {}", input.enabled),
    )
    .await;
    changed(&app).await?;
    app.emit("users", Some(id.to_string()));
    let nodes = state::user_nodes(&app).await?;
    Ok(Json(user_view(&state::user(&app, id).await?, &nodes)))
}

async fn delete_user(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let user = state::user(&app, id).await?;
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "user removed",
        &format!("user {}", user.name),
        "",
    )
    .await;
    changed(&app).await?;
    app.emit("users", Some(id.to_string()));
    Ok(StatusCode::NO_CONTENT)
}

async fn user_link(
    State(app): State<App>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserLink>> {
    let user = state::user(&app, id).await?;
    let link = state::user_link(&app, &user).await?;
    let qr_svg = crate::qr::svg(&link.link).map_err(ApiError::bad)?;
    Ok(Json(UserLink {
        link: link.link,
        qr_svg,
        servers: link.servers,
        warnings: link.warnings,
    }))
}

async fn user_secret(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user = state::user(&app, id).await?;
    app.audit(
        Some(admin.id),
        "secret shown",
        &format!("user {}", user.name),
        "",
    )
    .await;
    let mut response = Json(UserSecret {
        secret: user.secret,
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn regenerate_secret(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserSecret>> {
    let user = state::user(&app, id).await?;
    let secret = state::new_secret();
    sqlx::query("UPDATE users SET secret = ? WHERE id = ?")
        .bind(&secret)
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "secret regenerated",
        &format!("user {}", user.name),
        "old links stop working",
    )
    .await;
    changed(&app).await?;
    Ok(Json(UserSecret { secret }))
}

async fn reset_usage(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<UserView>> {
    let user = state::user(&app, id).await?;
    sqlx::query(
        "UPDATE users SET used_up = 0, used_down = 0, usage_epoch = usage_epoch + 1 WHERE id = ?",
    )
    .bind(id)
    .execute(&app.db)
    .await?;
    app.audit(
        Some(admin.id),
        "usage reset",
        &format!("user {}", user.name),
        &format!("used {}", user.used()),
    )
    .await;
    changed(&app).await?;
    let nodes = state::user_nodes(&app).await?;
    Ok(Json(user_view(&state::user(&app, id).await?, &nodes)))
}

async fn user_sessions(
    State(app): State<App>,
    _admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<NodeSessions>>> {
    state::user(&app, id).await?;
    let mut result = Vec::new();
    for node in state::nodes(&app).await? {
        let call = crate::sync::call(
            &app,
            node.id,
            Call::Sessions {
                user_id: state::user_id(id),
            },
        )
        .await;
        let (started_at, error) = match call {
            Ok(value) => (
                value
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row["started_at"].as_i64())
                            .collect()
                    })
                    .unwrap_or_default(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        result.push(NodeSessions {
            node_id: node.id,
            node_name: node.name,
            started_at,
            error,
        });
    }
    Ok(Json(result))
}

async fn kill_sessions(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let user = state::user(&app, id).await?;
    for node in state::nodes(&app).await? {
        let _ = crate::sync::call(
            &app,
            node.id,
            Call::KillSessions {
                user_id: state::user_id(id),
            },
        )
        .await;
    }
    app.audit(
        Some(admin.id),
        "sessions closed",
        &format!("user {}", user.name),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn disguise_checks(row: &DisguiseRow) -> (String, Vec<Check>) {
    let mut checks = Vec::new();
    let template = row.template();
    let domain = template
        .as_ref()
        .map(|template| template.domain.clone())
        .unwrap_or_default();
    checks.push(Check {
        name: "Template".into(),
        ok: template.is_ok(),
        detail: match &template {
            Ok(template) => format!(
                "ClientHello with {} bytes, SNI {}",
                template.raw.len(),
                template.domain
            ),
            Err(error) => error.message.clone(),
        },
    });
    checks.push(Check {
        name: "Domain".into(),
        ok: !domain.is_empty()
            && (domain == row.origin_host
                || row.origin_host.ends_with(&format!(".{domain}"))
                || domain.ends_with(&format!(".{}", row.origin_host))),
        detail: format!("Template SNI {domain}, origin {}", row.origin_host),
    });
    let profile = row.profile();
    checks.push(Check {
        name: "Origin probe".into(),
        ok: row.probed_at.is_some()
            && row.probe_error.is_none()
            && !profile.record_sizes.is_empty(),
        detail: match (&row.probe_error, row.probed_at) {
            (Some(error), _) => error.clone(),
            (None, Some(_)) => format!(
                "Cipher 0x{:04x}, group 0x{:04x}, {} record sizes",
                profile.cipher_suite,
                profile.key_share_group,
                profile.record_sizes.len()
            ),
            (None, None) => "Not probed. Probe the origin from a node.".into(),
        },
    });
    (domain, checks)
}

async fn list_disguises(
    State(app): State<App>,
    _admin: Admin,
) -> ApiResult<Json<Vec<DisguiseView>>> {
    let rows: Vec<DisguiseRow> = sqlx::query_as("SELECT id, name, origin_host, origin_port, template_raw, origin_profile, probed_at, probe_error, created_at FROM disguises ORDER BY id")
        .fetch_all(&app.db)
        .await?;
    let mut views = Vec::new();
    for row in rows {
        let nodes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE disguise_id = ?")
            .bind(row.id)
            .fetch_one(&app.db)
            .await?;
        let (domain, checks) = disguise_checks(&row);
        let profile = row.profile();
        views.push(DisguiseView {
            id: row.id,
            name: row.name.clone(),
            origin_host: row.origin_host.clone(),
            origin_port: row.origin_port as u16,
            domain,
            template_size: row.template_raw.len() as i64,
            profile: (!profile.record_sizes.is_empty() || profile.cipher_suite != 0).then(|| {
                OriginProfileView {
                    cipher_suite: profile.cipher_suite,
                    key_share_group: profile.key_share_group,
                    extension_order: profile.extension_order.clone(),
                    record_sizes: profile
                        .record_sizes
                        .iter()
                        .map(|size| *size as u16)
                        .collect(),
                }
            }),
            probed_at: row.probed_at,
            probe_error: row.probe_error.clone(),
            nodes,
            checks,
        });
    }
    Ok(Json(views))
}

async fn presets(_admin: Admin) -> Json<Vec<PresetView>> {
    Json(vec![PresetView {
        id: "wikipedia".into(),
        name: "Wikipedia".into(),
        domain: fiend_core::fingerprint::Template::from_json(BUNDLED)
            .map(|template| template.domain)
            .unwrap_or_default(),
    }])
}

const ORIGIN_PORT: u16 = 443;

fn template_bytes(input: &DisguiseInput) -> ApiResult<(Vec<u8>, String)> {
    let template = match (&input.preset, &input.template_base64) {
        (Some(preset), _) if preset == "wikipedia" => {
            fiend_core::fingerprint::Template::from_json(BUNDLED).map_err(ApiError::internal)?
        }
        (Some(_), _) => return Err(ApiError::field("preset", "Unknown preset.")),
        (None, Some(data)) => {
            let bytes = STANDARD
                .decode(data.trim())
                .map_err(|_| ApiError::field("template", "The file cannot be read."))?;
            if bytes.len() > 128 * 1024 {
                return Err(ApiError::field("template", "The file is too large."));
            }
            fiend_app_template(&bytes)?
        }
        (None, None) => return Err(ApiError::field("template", "Choose a preset or a file.")),
    };
    let compact = fiend_core::share::compact_hello(&template);
    fiend_core::share::template_from_hello(&compact)
        .map_err(|error| ApiError::field("template", error.to_string()))?;
    let domain = template.domain.trim().to_owned();
    fiend_core::share::check_server(&state::authority(&domain, i64::from(ORIGIN_PORT)))
        .map_err(|_| ApiError::field("template", "The template has no valid SNI host name."))?;
    Ok((compact, domain))
}

async fn free_disguise_name(app: &App, base: &str) -> ApiResult<String> {
    for number in 1..=100 {
        let name = if number == 1 {
            base.to_owned()
        } else {
            format!("{base} {number}")
        };
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM disguises WHERE name = ?")
            .bind(&name)
            .fetch_one(&app.db)
            .await?;
        if used == 0 {
            return Ok(name);
        }
    }
    Err(ApiError::conflict("Too many disguises use this name."))
}

fn fiend_app_template(bytes: &[u8]) -> ApiResult<fiend_core::fingerprint::Template> {
    let field = |message: &str| ApiError::field("template", message.to_owned());
    if bytes.trim_ascii_start().starts_with(b"{") {
        let text =
            std::str::from_utf8(bytes).map_err(|_| field("The template is not valid UTF-8."))?;
        let template = fiend_core::fingerprint::Template::from_json(text)
            .map_err(|_| field("The template JSON is not valid."))?;
        let captured = fiend_core::fingerprint::parse_client_hello(&template.raw)
            .map_err(|_| field("The template capture is not valid."))?;
        if captured.domain != template.domain
            || captured.session_id != template.session_id
            || captured.key_share != template.key_share
        {
            return Err(field("The template fields do not match its capture."));
        }
        Ok(template)
    } else {
        let template = fiend_core::fingerprint::parse_client_hello(bytes)
            .map_err(|_| field("The file is not a ClientHello capture or a template."))?;
        template
            .verify(bytes)
            .map_err(|_| field("The ClientHello capture cannot be used as a template."))?;
        Ok(template)
    }
}

async fn create_disguise(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<DisguiseInput>,
) -> ApiResult<Json<Created>> {
    let (raw, origin_host) = template_bytes(&input)?;
    let same: Option<String> =
        sqlx::query_scalar("SELECT name FROM disguises WHERE template_raw = ?")
            .bind(&raw)
            .fetch_optional(&app.db)
            .await?;
    if let Some(same) = same {
        return Err(ApiError::field(
            "template",
            format!("This template is already added as {same}."),
        ));
    }
    let base = match input.preset.as_deref() {
        Some("wikipedia") => "Wikipedia",
        _ => origin_host.as_str(),
    };
    let name = free_disguise_name(&app, base).await?;
    let profile = match crate::db::setting(&app.db, "local_origin").await {
        Some(text) => {
            let saved: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            if saved["host"] == origin_host.as_str() && saved["port"] == ORIGIN_PORT {
                saved["profile"].to_string()
            } else {
                "{}".into()
            }
        }
        None => "{}".into(),
    };
    let id: i64 = sqlx::query_scalar("INSERT INTO disguises (name, origin_host, origin_port, template_raw, origin_profile, created_at) VALUES (?, ?, ?, ?, ?, ?) RETURNING id")
        .bind(&name)
        .bind(&origin_host)
        .bind(i64::from(ORIGIN_PORT))
        .bind(raw)
        .bind(profile)
        .bind(now())
        .fetch_one(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "disguise added",
        &format!("disguise {name}"),
        &format!("{origin_host}:{ORIGIN_PORT}"),
    )
    .await;
    Ok(Json(Created { id }))
}

async fn delete_disguise(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let row = state::disguise(&app, id).await?;
    let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE disguise_id = ?")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict("Nodes use this disguise."));
    }
    sqlx::query("DELETE FROM disguises WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "disguise removed",
        &format!("disguise {}", row.name),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn probe_disguise(app: &App, id: i64, node: i64) -> ApiResult<()> {
    let row = state::disguise(app, id).await?;
    let result = crate::sync::call(
        app,
        node,
        Call::ProbeOrigin {
            host: Some(row.origin_host.clone()),
            port: Some(row.origin_port as u16),
        },
    )
    .await;
    match result {
        Ok(value) => {
            let profile: fiend_core::transport::OriginProfile =
                serde_json::from_value(value).map_err(ApiError::internal)?;
            sqlx::query("UPDATE disguises SET origin_profile = ?, probed_at = ?, probe_error = NULL WHERE id = ?")
                .bind(serde_json::to_string(&profile).map_err(ApiError::internal)?)
                .bind(now())
                .bind(id)
                .execute(&app.db)
                .await?;
            changed(app).await?;
            Ok(())
        }
        Err(error) => {
            sqlx::query("UPDATE disguises SET probed_at = ?, probe_error = ? WHERE id = ?")
                .bind(now())
                .bind(error.to_string())
                .bind(id)
                .execute(&app.db)
                .await?;
            Err(control_error(error))
        }
    }
}

async fn probe(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Json(input): Json<ProbeInput>,
) -> ApiResult<StatusCode> {
    let row = state::disguise(&app, id).await?;
    let node = state::node(&app, input.node_id).await?;
    let result = probe_disguise(&app, id, input.node_id).await;
    app.audit(
        Some(admin.id),
        "origin probed",
        &format!("disguise {}", row.name),
        &format!(
            "from node {}: {}",
            node.name,
            if result.is_ok() { "ok" } else { "failed" }
        ),
    )
    .await;
    result.map(|()| StatusCode::NO_CONTENT)
}

fn outbound_view(row: &OutboundRow) -> OutboundView {
    let config: serde_json::Value = serde_json::from_str(&row.config).unwrap_or_default();
    let text = |key: &str| {
        config
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    OutboundView {
        id: row.id,
        node_id: row.node_id,
        name: row.name.clone(),
        kind: row.kind.clone(),
        server: text("server"),
        username: text("username"),
        has_password: text("password").is_some(),
        udp: match row.kind.as_str() {
            "socks5" => config
                .get("udp")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            "direct" | "fiend" => true,
            _ => false,
        },
        link_node_id: config.get("node_id").and_then(serde_json::Value::as_i64),
        udp_fallback: row.udp_fallback.clone(),
    }
}

#[derive(Deserialize)]
struct NodeQuery {
    node: Option<i64>,
}

async fn routing(
    State(app): State<App>,
    _admin: Admin,
    Query(query): Query<NodeQuery>,
) -> ApiResult<Json<Routing>> {
    let outbounds: Vec<OutboundRow> = sqlx::query_as("SELECT id, node_id, name, kind, config, udp_fallback FROM outbounds WHERE node_id IS ? ORDER BY id")
        .bind(query.node)
        .fetch_all(&app.db)
        .await?;
    let rules: Vec<RuleRow> = sqlx::query_as("SELECT id, node_id, position, pattern, outbound FROM rules WHERE node_id IS ? ORDER BY position, id")
        .bind(query.node)
        .fetch_all(&app.db)
        .await?;
    Ok(Json(Routing {
        outbounds: outbounds.iter().map(outbound_view).collect(),
        rules: rules
            .into_iter()
            .map(|rule| RuleView {
                id: rule.id,
                node_id: rule.node_id,
                position: rule.position,
                pattern: rule.pattern,
                outbound: rule.outbound,
            })
            .collect(),
    }))
}

async fn outbound_config(
    app: &App,
    input: &OutboundInput,
    previous: Option<&OutboundRow>,
) -> ApiResult<serde_json::Value> {
    let old: serde_json::Value = previous
        .and_then(|row| serde_json::from_str(&row.config).ok())
        .unwrap_or_default();
    let password = input
        .password
        .clone()
        .filter(|password| !password.is_empty())
        .or_else(|| {
            old.get("password")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .filter(|_| {
                    input
                        .username
                        .as_deref()
                        .is_some_and(|user| !user.is_empty())
                })
        });
    let username = input.username.clone().filter(|user| !user.is_empty());
    let config = match input.kind.as_str() {
        "direct" | "block" => serde_json::json!({}),
        "warp" => {
            if let Some(node) = input.node_id {
                state::node(app, node).await?;
            }
            serde_json::json!({})
        }
        "socks5" | "http" => {
            let server = input.server.clone().unwrap_or_default();
            fiend_core::share::check_server(&server)
                .map_err(|_| ApiError::field("server", "Enter host:port."))?;
            if username.is_some() != password.is_some() {
                return Err(ApiError::field(
                    "password",
                    "Enter both the user name and the password, or neither.",
                ));
            }
            let mut config = serde_json::json!({ "server": server });
            if let (Some(username), Some(password)) = (&username, &password) {
                config["username"] = username.as_str().into();
                config["password"] = password.as_str().into();
            }
            if input.kind == "socks5" {
                config["udp"] = input.udp.into();
            }
            config
        }
        "fiend" => {
            let target = input
                .link_node_id
                .ok_or_else(|| ApiError::field("link_node_id", "Choose a node."))?;
            if Some(target) == input.node_id {
                return Err(ApiError::field(
                    "link_node_id",
                    "A node cannot chain to itself.",
                ));
            }
            state::chain_link(app, target)
                .await
                .map_err(|error| ApiError::field("link_node_id", error.message))?;
            serde_json::json!({ "node_id": target })
        }
        _ => return Err(ApiError::field("kind", "Choose an outbound type.")),
    };
    if input.name.is_empty()
        || input.name.len() > 64
        || input.name == "direct"
        || !input
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(ApiError::field(
            "name",
            "Use 1 to 64 letters, digits, '-', '_', or '.'. The name 'direct' is reserved.",
        ));
    }
    if let Some(fallback) = &input.udp_fallback
        && fallback == &input.name
    {
        return Err(ApiError::field(
            "udp_fallback",
            "Choose another outbound for UDP.",
        ));
    }
    if let Some(node) = input.node_id {
        state::node(app, node).await?;
    }
    Ok(config)
}

async fn validate_routing(app: &App) -> ApiResult<()> {
    for node in state::nodes(app).await? {
        let (outbounds, rules, mut warp) = state::routing_for(app, node.id).await?;
        warp.enabled = true;
        fiend_core::node::outbound::validate(&outbounds, &rules, &warp)
            .map_err(|error| ApiError::bad(format!("Node {}: {error}", node.name)))?;
    }
    Ok(())
}

async fn create_outbound(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<OutboundInput>,
) -> ApiResult<Json<Created>> {
    let config = outbound_config(&app, &input, None).await?;
    let mut transaction = app.db.begin().await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO outbounds (node_id, name, kind, config, udp_fallback) VALUES (?, ?, ?, ?, ?) RETURNING id")
        .bind(input.node_id)
        .bind(&input.name)
        .bind(&input.kind)
        .bind(config.to_string())
        .bind(input.udp_fallback.as_deref().filter(|fallback| !fallback.is_empty()))
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| match ApiError::from(error) {
            error if error.status == StatusCode::CONFLICT => ApiError::field("name", "This name is already in use."),
            error => error,
        })?;
    transaction.commit().await?;
    if let Err(error) = validate_routing(&app).await {
        sqlx::query("DELETE FROM outbounds WHERE id = ?")
            .bind(id)
            .execute(&app.db)
            .await?;
        return Err(error);
    }
    app.audit(
        Some(admin.id),
        "outbound added",
        &format!("outbound {}", input.name),
        &input.kind,
    )
    .await;
    changed(&app).await?;
    if input.kind == "warp" {
        enable_warp_for(&app, input.node_id).await?;
    }
    Ok(Json(Created { id }))
}

async fn edit_outbound(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Json(input): Json<OutboundInput>,
) -> ApiResult<StatusCode> {
    let previous: OutboundRow = sqlx::query_as(
        "SELECT id, node_id, name, kind, config, udp_fallback FROM outbounds WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if previous.node_id != input.node_id {
        return Err(ApiError::bad("An outbound cannot move to another node."));
    }
    let config = outbound_config(&app, &input, Some(&previous)).await?;
    sqlx::query(
        "UPDATE outbounds SET name = ?, kind = ?, config = ?, udp_fallback = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.kind)
    .bind(config.to_string())
    .bind(
        input
            .udp_fallback
            .as_deref()
            .filter(|fallback| !fallback.is_empty()),
    )
    .bind(id)
    .execute(&app.db)
    .await?;
    if previous.name != input.name {
        sqlx::query(
            "UPDATE rules SET outbound = ? WHERE outbound = ? AND (node_id IS ? OR ? IS NULL)",
        )
        .bind(&input.name)
        .bind(&previous.name)
        .bind(previous.node_id)
        .bind(previous.node_id)
        .execute(&app.db)
        .await?;
    }
    if let Err(error) = validate_routing(&app).await {
        sqlx::query(
            "UPDATE outbounds SET name = ?, kind = ?, config = ?, udp_fallback = ? WHERE id = ?",
        )
        .bind(&previous.name)
        .bind(&previous.kind)
        .bind(&previous.config)
        .bind(&previous.udp_fallback)
        .bind(id)
        .execute(&app.db)
        .await?;
        return Err(error);
    }
    app.audit(
        Some(admin.id),
        "outbound changed",
        &format!("outbound {}", input.name),
        &input.kind,
    )
    .await;
    changed(&app).await?;
    if input.kind == "warp" && previous.kind != "warp" {
        enable_warp_for(&app, input.node_id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_outbound(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let row: OutboundRow = sqlx::query_as(
        "SELECT id, node_id, name, kind, config, udp_fallback FROM outbounds WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let used: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rules WHERE outbound = ? AND (node_id IS ? OR ? IS NULL)",
    )
    .bind(&row.name)
    .bind(row.node_id)
    .bind(row.node_id)
    .fetch_one(&app.db)
    .await?;
    if used > 0 {
        return Err(ApiError::conflict("Rules use this outbound."));
    }
    sqlx::query("DELETE FROM outbounds WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    sqlx::query("UPDATE outbounds SET udp_fallback = NULL WHERE udp_fallback = ?")
        .bind(&row.name)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "outbound removed",
        &format!("outbound {}", row.name),
        "",
    )
    .await;
    changed(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn check_rule(app: &App, input: &RuleInput) -> ApiResult<()> {
    if input.pattern.trim().is_empty() {
        return Err(ApiError::field("pattern", "Enter at least one pattern."));
    }
    fiend_rules::parse_list(&input.pattern, true)
        .map_err(|error| ApiError::field("pattern", error.to_string()))?;
    if input.outbound != "direct" {
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM outbounds WHERE name = ? AND (node_id IS NULL OR node_id IS ?)",
        )
        .bind(&input.outbound)
        .bind(input.node_id)
        .fetch_one(&app.db)
        .await?;
        if found == 0 {
            return Err(ApiError::field("outbound", "Choose an outbound."));
        }
    }
    if let Some(node) = input.node_id {
        state::node(app, node).await?;
    }
    Ok(())
}

async fn create_rule(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<RuleInput>,
) -> ApiResult<Json<Created>> {
    check_rule(&app, &input).await?;
    let position: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(position), 0) + 1 FROM rules WHERE node_id IS ?")
            .bind(input.node_id)
            .fetch_one(&app.db)
            .await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO rules (node_id, position, pattern, outbound) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(input.node_id)
    .bind(position)
    .bind(&input.pattern)
    .bind(&input.outbound)
    .fetch_one(&app.db)
    .await?;
    app.audit(
        Some(admin.id),
        "rule added",
        &format!("rule to {}", input.outbound),
        &input.pattern.lines().take(3).collect::<Vec<_>>().join(" "),
    )
    .await;
    changed(&app).await?;
    Ok(Json(Created { id }))
}

async fn edit_rule(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Json(input): Json<RuleInput>,
) -> ApiResult<StatusCode> {
    check_rule(&app, &input).await?;
    let result =
        sqlx::query("UPDATE rules SET pattern = ?, outbound = ? WHERE id = ? AND node_id IS ?")
            .bind(&input.pattern)
            .bind(&input.outbound)
            .bind(id)
            .bind(input.node_id)
            .execute(&app.db)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    app.audit(
        Some(admin.id),
        "rule changed",
        &format!("rule to {}", input.outbound),
        "",
    )
    .await;
    changed(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_rule(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM rules WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    app.audit(Some(admin.id), "rule removed", &format!("rule {id}"), "")
        .await;
    changed(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn order_rules(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<RuleOrder>,
) -> ApiResult<StatusCode> {
    let mut transaction = app.db.begin().await?;
    for (position, id) in input.ids.iter().enumerate() {
        sqlx::query("UPDATE rules SET position = ? WHERE id = ?")
            .bind(position as i64 + 1)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    app.audit(Some(admin.id), "rules reordered", "rules", "")
        .await;
    changed(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_admins(State(app): State<App>, _admin: Admin) -> ApiResult<Json<Vec<AdminView>>> {
    let rows: Vec<(i64, String, Option<String>, i64)> =
        sqlx::query_as("SELECT id, name, totp_secret, created_at FROM admins ORDER BY id")
            .fetch_all(&app.db)
            .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, totp, created_at)| AdminView {
                id,
                name,
                totp: totp.is_some(),
                created_at,
            })
            .collect(),
    ))
}

async fn create_admin(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<AdminInput>,
) -> ApiResult<Json<Created>> {
    auth::require_code(&app, &admin, &input.current_code)?;
    auth::check_name(&input.name, "name")?;
    auth::check_password_rules(&input.password)?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO admins (name, password_hash, created_at) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(&input.name)
    .bind(auth::hash_password(&input.password)?)
    .bind(now())
    .fetch_one(&app.db)
    .await
    .map_err(|error| match ApiError::from(error) {
        error if error.status == StatusCode::CONFLICT => {
            ApiError::field("name", "This name is already in use.")
        }
        error => error,
    })?;
    app.audit(
        Some(admin.id),
        "admin added",
        &format!("admin {}", input.name),
        "",
    )
    .await;
    Ok(Json(Created { id }))
}

#[derive(Deserialize)]
struct CodeQuery {
    code: String,
}

async fn delete_admin(
    State(app): State<App>,
    admin: Admin,
    Path(id): Path<i64>,
    Query(query): Query<CodeQuery>,
) -> ApiResult<StatusCode> {
    auth::require_code(&app, &admin, &query.code)?;
    if id == admin.id {
        return Err(ApiError::bad("You cannot remove your own account."));
    }
    let name: String = sqlx::query_scalar("SELECT name FROM admins WHERE id = ?")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    sqlx::query("DELETE FROM admins WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "admin removed",
        &format!("admin {name}"),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn change_password(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<PasswordInput>,
) -> ApiResult<StatusCode> {
    auth::confirm_identity(&app, &admin, &input.current, input.totp_code.as_deref()).await?;
    auth::check_password_rules(&input.password)?;
    sqlx::query("UPDATE admins SET password_hash = ? WHERE id = ?")
        .bind(auth::hash_password(&input.password)?)
        .bind(admin.id)
        .execute(&app.db)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE admin_id = ? AND id_hash != ?")
        .bind(admin.id)
        .bind(&admin.session_hash)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "password changed",
        &format!("admin {}", admin.name),
        "other sessions closed",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn new_totp(_app: State<App>, admin: Admin) -> ApiResult<Json<TotpSetup>> {
    Ok(Json(auth::new_totp(&admin.name)?))
}

async fn enable_totp(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<TotpInput>,
) -> ApiResult<StatusCode> {
    if admin.totp_secret.is_some() {
        return Err(ApiError::conflict("Two-factor sign-in is already on."));
    }
    let secret = auth::totp_secret_from_input(&input.secret, &input.code)?;
    sqlx::query("UPDATE admins SET totp_secret = ? WHERE id = ?")
        .bind(secret)
        .bind(admin.id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "two-factor on",
        &format!("admin {}", admin.name),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn disable_totp(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<CodeInput>,
) -> ApiResult<StatusCode> {
    auth::require_code(&app, &admin, &input.code)?;
    sqlx::query("UPDATE admins SET totp_secret = NULL WHERE id = ?")
        .bind(admin.id)
        .execute(&app.db)
        .await?;
    app.audit(
        Some(admin.id),
        "two-factor off",
        &format!("admin {}", admin.name),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn backup(
    State(app): State<App>,
    admin: Admin,
    Json(input): Json<CodeInput>,
) -> ApiResult<Response> {
    if let Some(secret) = &admin.totp_secret
        && !app.verify_code(admin.id, secret, &input.code)?
    {
        return Err(ApiError::field("code", "The code is not correct."));
    }
    let path = app
        .config
        .database
        .with_extension(format!("backup-{}", hex::encode(rand::random::<[u8; 6]>())));
    sqlx::query("VACUUM INTO ?")
        .bind(path.to_string_lossy().to_string())
        .execute(&app.db)
        .await?;
    let bytes = tokio::fs::read(&path).await.map_err(ApiError::internal);
    let _ = std::fs::remove_file(&path);
    let bytes = bytes?;
    app.audit(
        Some(admin.id),
        "backup downloaded",
        "database",
        &format!("{} bytes", bytes.len()),
    )
    .await;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.sqlite3"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"fiend-panel.db\""),
    );
    Ok(response)
}

async fn restore(
    State(app): State<App>,
    admin: Admin,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let password = headers
        .get("x-current-password")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| STANDARD.decode(value.trim()).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_default();
    let code = headers
        .get("x-current-code")
        .and_then(|value| value.to_str().ok());
    auth::confirm_identity(&app, &admin, &password, code).await?;
    if !body.starts_with(b"SQLite format 3\0") {
        return Err(ApiError::bad("The file is not a panel database backup."));
    }
    let staging = app.config.database.with_extension("db.check");
    tokio::fs::write(&staging, &body)
        .await
        .map_err(ApiError::internal)?;
    let check = async {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&staging)
            .read_only(true);
        let mut connection = sqlx::ConnectOptions::connect(&options).await?;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&mut connection)
            .await?;
        let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
            .fetch_one(&mut connection)
            .await?;
        let migrations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&mut connection)
            .await?;
        sqlx::Connection::close(connection).await?;
        Ok::<_, sqlx::Error>((integrity, admins, migrations))
    }
    .await;
    match check {
        Ok((integrity, admins, migrations))
            if integrity == "ok" && admins > 0 && migrations > 0 => {}
        _ => {
            let _ = std::fs::remove_file(&staging);
            return Err(ApiError::bad(
                "The file is not a valid panel database backup.",
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o600))
            .map_err(ApiError::internal)?;
    }
    std::fs::rename(&staging, app.config.database.with_extension("db.restore"))
        .map_err(ApiError::internal)?;
    app.audit(
        Some(admin.id),
        "restore staged",
        "database",
        "the panel restarts",
    )
    .await;
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::process::exit(0);
    });
    Ok(StatusCode::ACCEPTED)
}

type ProbeRow = (
    i64,
    i64,
    Option<String>,
    String,
    String,
    Option<String>,
    i64,
    Vec<u8>,
);

#[derive(Deserialize)]
struct ProbeQuery {
    range: Option<String>,
    peer: Option<String>,
}

async fn probes(
    State(app): State<App>,
    _admin: Admin,
    Query(query): Query<ProbeQuery>,
) -> ApiResult<Json<ProbeReport>> {
    let (range, span) = match query.range.as_deref().unwrap_or("24h") {
        "1h" => ("1h", 3600),
        "7d" => ("7d", 7 * 86400),
        "30d" => ("30d", 30 * 86400),
        _ => ("24h", 86400),
    };
    let since = now() - span;
    let peer = query.peer.filter(|peer| !peer.is_empty());
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM probes WHERE ts >= ? AND (? IS NULL OR peer = ?)")
            .bind(since)
            .bind(&peer)
            .bind(&peer)
            .fetch_one(&app.db)
            .await?;
    let peers: Vec<(String, i64, i64, i64, String)> = sqlx::query_as("SELECT peer, COUNT(*), MIN(ts), MAX(ts), GROUP_CONCAT(DISTINCT reason) FROM probes WHERE ts >= ? AND (? IS NULL OR peer = ?) GROUP BY peer ORDER BY COUNT(*) DESC, MAX(ts) DESC LIMIT 200")
        .bind(since)
        .bind(&peer)
        .bind(&peer)
        .fetch_all(&app.db)
        .await?;
    let rows: Vec<ProbeRow> = sqlx::query_as("SELECT probes.id, probes.ts, nodes.name, probes.peer, probes.reason, probes.sni, probes.size, probes.head FROM probes LEFT JOIN nodes ON nodes.id = probes.node_id WHERE probes.ts >= ? AND (? IS NULL OR probes.peer = ?) ORDER BY probes.id DESC LIMIT 200")
        .bind(since)
        .bind(&peer)
        .bind(&peer)
        .fetch_all(&app.db)
        .await?;
    Ok(Json(ProbeReport {
        range: range.into(),
        total,
        peers: peers
            .into_iter()
            .map(|(peer, count, first, last, reasons)| ProbePeer {
                peer,
                count,
                first,
                last,
                reasons: reasons.split(',').map(str::to_owned).collect(),
            })
            .collect(),
        probes: rows
            .into_iter()
            .map(|(id, ts, node, peer, reason, sni, size, head)| ProbeView {
                id,
                ts,
                node: node.unwrap_or_default(),
                peer,
                reason,
                sni,
                size,
                head: hex::encode(head),
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
struct AuditQuery {
    limit: Option<i64>,
}

async fn audit(
    State(app): State<App>,
    _admin: Admin,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<Vec<AuditView>>> {
    let rows: Vec<(i64, i64, Option<String>, String, String, String)> = sqlx::query_as("SELECT audit.id, audit.ts, admins.name, audit.action, audit.target, audit.detail FROM audit LEFT JOIN admins ON admins.id = audit.admin_id ORDER BY audit.id DESC LIMIT ?")
        .bind(query.limit.unwrap_or(200).clamp(1, 1000))
        .fetch_all(&app.db)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, ts, admin, action, target, detail)| AuditView {
                id,
                ts,
                admin,
                action,
                target,
                detail,
            })
            .collect(),
    ))
}
