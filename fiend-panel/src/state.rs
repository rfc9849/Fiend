use std::collections::HashMap;

use fiend_core::fingerprint::Template;
use fiend_core::node::outbound::{Outbound, OutboundKind, Rule, WarpSettings};
use fiend_core::node::{DesiredState, User};
use fiend_core::transport::OriginProfile;
use sqlx::FromRow;

use crate::{ApiError, ApiResult, App, now};

#[derive(Clone, Debug, FromRow)]
pub struct NodeRow {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub port: i64,
    pub local: bool,
    pub arch: Option<String>,
    pub control_secret: String,
    pub service_secret: String,
    pub version: Option<String>,
    pub applied_version: i64,
    pub status: String,
    pub last_error: Option<String>,
    pub last_seen: Option<i64>,
    pub started_at: Option<i64>,
    pub disguise_id: Option<i64>,
    pub created_at: i64,
}

#[derive(Clone, Debug, FromRow)]
pub struct UserRow {
    pub id: i64,
    pub name: String,
    pub secret: String,
    pub enabled: bool,
    pub note: String,
    pub traffic_limit: Option<i64>,
    pub used_up: i64,
    pub used_down: i64,
    pub expire_at: Option<i64>,
    pub hold_days: Option<i64>,
    pub reset_period: String,
    pub reset_at: Option<i64>,
    pub usage_epoch: i64,
    pub last_seen: Option<i64>,
    pub online: i64,
    pub created_at: i64,
}

impl UserRow {
    pub fn used(&self) -> i64 {
        self.used_up.saturating_add(self.used_down)
    }

    pub fn state(&self, at: i64) -> &'static str {
        if !self.enabled {
            "disabled"
        } else if self.expire_at.is_some_and(|expiry| expiry <= at) {
            "expired"
        } else if self.traffic_limit.is_some_and(|limit| self.used() >= limit) {
            "limited"
        } else if self.expire_at.is_none() && self.hold_days.is_some() {
            "on_hold"
        } else {
            "active"
        }
    }

    pub fn active(&self, at: i64) -> bool {
        matches!(self.state(at), "active" | "on_hold")
    }
}

#[derive(Clone, Debug, FromRow)]
pub struct DisguiseRow {
    pub id: i64,
    pub name: String,
    pub origin_host: String,
    pub origin_port: i64,
    pub template_raw: Vec<u8>,
    pub origin_profile: String,
    pub probed_at: Option<i64>,
    pub probe_error: Option<String>,
    pub created_at: i64,
}

impl DisguiseRow {
    pub fn template(&self) -> ApiResult<Template> {
        fiend_core::share::template_from_hello(&self.template_raw)
            .map_err(|error| ApiError::bad(error.to_string()))
    }

    pub fn profile(&self) -> OriginProfile {
        serde_json::from_str(&self.origin_profile).unwrap_or_default()
    }
}

#[derive(Clone, Debug, FromRow)]
pub struct OutboundRow {
    pub id: i64,
    pub node_id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub config: String,
    pub udp_fallback: Option<String>,
}

#[derive(Clone, Debug, FromRow)]
pub struct RuleRow {
    pub id: i64,
    pub node_id: Option<i64>,
    pub position: i64,
    pub pattern: String,
    pub outbound: String,
}

pub const NODE_COLUMNS: &str = "id, name, address, port, local, arch, control_secret, service_secret, version, applied_version, status, last_error, last_seen, started_at, disguise_id, created_at";
pub const USER_COLUMNS: &str = "id, name, secret, enabled, note, traffic_limit, used_up, used_down, expire_at, hold_days, reset_period, reset_at, usage_epoch, last_seen, online, created_at";

pub async fn node(app: &App, id: i64) -> ApiResult<NodeRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(ApiError::not_found)
}

pub async fn nodes(app: &App) -> ApiResult<Vec<NodeRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {NODE_COLUMNS} FROM nodes ORDER BY id"
    )))
    .fetch_all(&app.db)
    .await?)
}

pub async fn user(app: &App, id: i64) -> ApiResult<UserRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLUMNS} FROM users WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(ApiError::not_found)
}

pub async fn users(app: &App) -> ApiResult<Vec<UserRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLUMNS} FROM users ORDER BY name COLLATE NOCASE"
    )))
    .fetch_all(&app.db)
    .await?)
}

pub async fn disguise(app: &App, id: i64) -> ApiResult<DisguiseRow> {
    sqlx::query_as("SELECT id, name, origin_host, origin_port, template_raw, origin_profile, probed_at, probe_error, created_at FROM disguises WHERE id = ?")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(ApiError::not_found)
}

pub async fn user_nodes(app: &App) -> ApiResult<HashMap<i64, Vec<i64>>> {
    let rows: Vec<(i64, i64)> =
        sqlx::query_as("SELECT user_id, node_id FROM user_nodes ORDER BY node_id")
            .fetch_all(&app.db)
            .await?;
    let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
    for (user, node) in rows {
        map.entry(user).or_default().push(node);
    }
    Ok(map)
}

pub fn authority(address: &str, port: i64) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

pub fn user_id(id: i64) -> String {
    format!("u{id}")
}

pub fn parse_user_id(id: &str) -> Option<i64> {
    id.strip_prefix('u')?.parse().ok()
}

pub async fn chain_link(app: &App, target: i64) -> ApiResult<String> {
    let node = node(app, target).await?;
    let disguise_id = node.disguise_id.ok_or_else(|| {
        ApiError::bad(format!(
            "Node {} needs a disguise before other nodes can chain to it.",
            node.name
        ))
    })?;
    let disguise = disguise(app, disguise_id).await?;
    let secret =
        fiend_core::share::decode_secret(&node.service_secret).map_err(ApiError::internal)?;
    fiend_core::share::encode(&fiend_core::share::Share {
        name: format!("chain {}", node.name),
        servers: vec![authority(&node.address, node.port)],
        secret,
        template: disguise.template()?,
    })
    .map_err(|error| ApiError::bad(error.to_string()))
}

pub async fn outbound_kind(app: &App, row: &OutboundRow) -> ApiResult<OutboundKind> {
    let config: serde_json::Value =
        serde_json::from_str(&row.config).map_err(ApiError::internal)?;
    let text = |key: &str| {
        config
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Ok(match row.kind.as_str() {
        "direct" => OutboundKind::Direct,
        "block" => OutboundKind::Block,
        "warp" => OutboundKind::Warp,
        "socks5" => OutboundKind::Socks5 {
            server: text("server").unwrap_or_default(),
            username: text("username"),
            password: text("password"),
            udp: config
                .get("udp")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        },
        "http" => OutboundKind::Http {
            server: text("server").unwrap_or_default(),
            username: text("username"),
            password: text("password"),
        },
        "fiend" => {
            let target = config
                .get("node_id")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| ApiError::bad("The chain outbound needs a node"))?;
            OutboundKind::Fiend {
                link: chain_link(app, target).await?,
            }
        }
        _ => return Err(ApiError::bad("Unknown outbound type")),
    })
}

pub async fn routing_for(
    app: &App,
    node_id: i64,
) -> ApiResult<(Vec<Outbound>, Vec<Rule>, WarpSettings)> {
    let rows: Vec<OutboundRow> = sqlx::query_as(
        "SELECT id, node_id, name, kind, config, udp_fallback FROM outbounds WHERE node_id IS NULL OR node_id = ? ORDER BY node_id IS NOT NULL, id",
    )
    .bind(node_id)
    .fetch_all(&app.db)
    .await?;
    let mut merged: Vec<OutboundRow> = Vec::new();
    for row in rows {
        if let Some(existing) = merged.iter_mut().find(|item| item.name == row.name) {
            *existing = row;
        } else {
            merged.push(row);
        }
    }
    if merged.iter().any(|row| row.kind == "warp") {
        let added = sqlx::query("INSERT INTO warp (node_id, enabled) VALUES (?, 1) ON CONFLICT(node_id) DO UPDATE SET enabled = 1 WHERE chosen = 0 AND enabled = 0")
            .bind(node_id)
            .execute(&app.db)
            .await?;
        if added.rows_affected() > 0 {
            crate::db::bump_version(&app.db).await?;
        }
    }
    let warp: Option<(bool, Option<i64>)> =
        sqlx::query_as("SELECT enabled, rotate_every FROM warp WHERE node_id = ?")
            .bind(node_id)
            .fetch_optional(&app.db)
            .await?;
    let warp = warp
        .map(|(enabled, rotate_every)| WarpSettings {
            enabled,
            rotate_every: rotate_every.map(|value| value as u64),
        })
        .unwrap_or_default();
    let mut outbounds = Vec::new();
    for row in &merged {
        if row.kind == "warp" && !warp.enabled {
            continue;
        }
        if row.kind == "fiend"
            && serde_json::from_str::<serde_json::Value>(&row.config)
                .ok()
                .and_then(|config| config.get("node_id").and_then(serde_json::Value::as_i64))
                == Some(node_id)
        {
            continue;
        }
        outbounds.push(Outbound {
            id: row.name.clone(),
            kind: outbound_kind(app, row).await?,
            udp_fallback: row.udp_fallback.clone(),
        });
    }
    let names: Vec<String> = outbounds
        .iter()
        .map(|outbound| outbound.id.clone())
        .collect();
    for outbound in &mut outbounds {
        if outbound
            .udp_fallback
            .as_ref()
            .is_some_and(|fallback| fallback != "direct" && !names.contains(fallback))
        {
            outbound.udp_fallback = None;
        }
    }
    let rules: Vec<RuleRow> = sqlx::query_as(
        "SELECT id, node_id, position, pattern, outbound FROM rules WHERE node_id IS NULL OR node_id = ? ORDER BY node_id IS NOT NULL, position, id",
    )
    .bind(node_id)
    .fetch_all(&app.db)
    .await?;
    let rules = rules
        .into_iter()
        .filter(|rule| rule.outbound == "direct" || names.contains(&rule.outbound))
        .map(|rule| Rule {
            pattern: rule.pattern,
            outbound: rule.outbound,
        })
        .collect();
    Ok((outbounds, rules, warp))
}

pub async fn desired(app: &App, node: &NodeRow) -> ApiResult<Option<DesiredState>> {
    let Some(disguise_id) = node.disguise_id else {
        return Ok(None);
    };
    let disguise = disguise(app, disguise_id).await?;
    let at = now();
    let allowed = user_nodes(app).await?;
    let mut users = Vec::new();
    for user in self::users(app).await? {
        if allowed
            .get(&user.id)
            .is_some_and(|nodes| !nodes.contains(&node.id))
        {
            continue;
        }
        users.push(User {
            id: user_id(user.id),
            secret: user.secret.clone(),
            enabled: user.active(at),
            allowed: true,
            expire_at: user.expire_at.map(|value| value.max(0) as u64),
            traffic_limit: user.traffic_limit.map(|value| value.max(0) as u64),
            used_traffic: user.used().max(0) as u64,
            usage_epoch: user.usage_epoch.max(0) as u64,
        });
    }
    users.push(User {
        id: "service".into(),
        secret: node.service_secret.clone(),
        enabled: true,
        allowed: true,
        expire_at: None,
        traffic_limit: None,
        used_traffic: 0,
        usage_epoch: 0,
    });
    let (outbounds, rules, warp) = routing_for(app, node.id).await?;
    Ok(Some(DesiredState {
        version: crate::db::version(&app.db).await as u64,
        users,
        origin_host: disguise.origin_host.clone(),
        origin_port: disguise.origin_port as u16,
        origin_profile: disguise.profile(),
        outbounds,
        rules,
        warp,
    }))
}

pub struct Link {
    pub link: String,
    pub servers: Vec<String>,
    pub warnings: Vec<String>,
}

pub async fn user_link(app: &App, user: &UserRow) -> ApiResult<Link> {
    let allowed = user_nodes(app).await?;
    let allowed = allowed.get(&user.id);
    let mut servers = Vec::new();
    let mut warnings = Vec::new();
    let mut template = None;
    let mut first_disguise = None;
    for node in nodes(app).await? {
        if allowed.is_some_and(|nodes| !nodes.contains(&node.id)) {
            continue;
        }
        let Some(disguise_id) = node.disguise_id else {
            warnings.push(format!(
                "Node {} has no disguise and is not in the link.",
                node.name
            ));
            continue;
        };
        if first_disguise.is_none() {
            first_disguise = Some(disguise_id);
            template = Some(self::disguise(app, disguise_id).await?.template()?);
        } else if first_disguise != Some(disguise_id) {
            warnings.push(format!(
                "Node {} has a different disguise than this link.",
                node.name
            ));
        }
        servers.push(authority(&node.address, node.port));
    }
    let template = template.ok_or_else(|| ApiError::bad("No node with a disguise."))?;
    if servers.len() > fiend_core::share::MAX_SERVERS {
        servers.truncate(fiend_core::share::MAX_SERVERS);
        warnings.push("Only the first 16 nodes are in the link.".into());
    }
    let secret = fiend_core::share::decode_secret(&user.secret).map_err(ApiError::internal)?;
    let link = fiend_core::share::encode(&fiend_core::share::Share {
        name: user.name.clone(),
        servers: servers.clone(),
        secret,
        template,
    })
    .map_err(|error| ApiError::bad(error.to_string()))?;
    Ok(Link {
        link,
        servers,
        warnings,
    })
}

pub fn new_secret() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}
