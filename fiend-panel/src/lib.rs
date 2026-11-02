#![forbid(unsafe_code)]

pub mod address;
pub mod api;
pub mod auth;
pub mod db;
pub mod qr;
pub mod state;
pub mod sync;
pub mod types;

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

#[derive(RustEmbed)]
#[folder = "web/build/"]
struct Assets;

pub const PANEL_PORT: u16 = 38029;

fn initial_listen(value: Option<&str>) -> Result<SocketAddr, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, PANEL_PORT))),
        Some(value) => value
            .parse()
            .map_err(|_| format!("FIEND_PANEL_LISTEN is not an address and port: {value}")),
    }
}

fn default_database() -> PathBuf {
    PathBuf::from("/var/lib/fiend/panel.db")
}

fn default_local_node() -> PathBuf {
    PathBuf::from("/etc/fiend/node.toml")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub listen: SocketAddr,
    pub setup_token: String,
    pub setup_expires_at: u64,
    #[serde(default = "default_database")]
    pub database: PathBuf,
    #[serde(default = "default_local_node")]
    pub local_node: PathBuf,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let config: Self = toml::from_str(&raw).map_err(|error| error.to_string())?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.listen.port() == 0 {
            return Err("panel listen must have a nonzero port".into());
        }
        if self.setup_token.len() != 64 || !self.setup_token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("setup token must have 64 hex characters".into());
        }
        Ok(())
    }

    pub fn init(path: &Path) -> Result<Self, String> {
        if path.exists() {
            return Self::load(path);
        }
        let config = Self {
            listen: initial_listen(std::env::var("FIEND_PANEL_LISTEN").ok().as_deref())?,
            setup_token: hex::encode(rand::random::<[u8; 32]>()),
            setup_expires_at: now() as u64 + 3600,
            database: default_database(),
            local_node: default_local_node(),
        };
        let raw = toml::to_string(&config).map_err(|e| e.to_string())?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|e| e.to_string())?;
        file.write_all(raw.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(config)
    }

    pub fn renew_setup(path: &Path) -> Result<Self, String> {
        let mut config = Self::load(path)?;
        config.setup_token = hex::encode(rand::random::<[u8; 32]>());
        config.setup_expires_at = now() as u64 + 3600;
        let raw = toml::to_string(&config).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        let owner = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(path).map(|meta| (meta.uid(), meta.gid())).ok()
        };
        fiend_core::node::save_private(path, raw.as_bytes()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some((uid, gid)) = owner {
            std::os::unix::fs::chown(path, Some(uid), Some(gid)).map_err(|e| e.to_string())?;
        }
        Ok(config)
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub field: Option<&'static str>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            field: None,
        }
    }

    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn field(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            field: Some(field),
        }
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "The item is not available")
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "internal panel error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal panel error")
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database) = &error
            && database.is_unique_violation()
        {
            return Self::conflict("This name is already in use");
        }
        Self::internal(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(types::ErrorBody {
                error: self.message,
                field: self.field.map(str::to_owned),
            }),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
pub struct App(Arc<Inner>);

pub struct Inner {
    pub config: Config,
    pub db: SqlitePool,
    pub events: broadcast::Sender<types::Event>,
    pub limiter: auth::Limiter,
    pub sync: sync::Sync,
    pub totp_steps: Mutex<HashMap<i64, u64>>,
}

impl std::ops::Deref for App {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl App {
    pub async fn open(config: Config) -> Result<Self, String> {
        let db = db::open(&config.database).await?;
        let (events, _) = broadcast::channel(256);
        Ok(Self(Arc::new(Inner {
            config,
            db,
            events,
            limiter: auth::Limiter::default(),
            sync: sync::Sync::default(),
            totp_steps: Mutex::new(HashMap::new()),
        })))
    }

    pub fn start(&self) {
        sync::start(self.clone());
    }

    pub fn emit(&self, kind: &str, id: Option<String>) {
        let _ = self.events.send(types::Event {
            kind: kind.into(),
            id,
        });
    }

    pub async fn audit(&self, admin: Option<i64>, action: &str, target: &str, detail: &str) {
        let result = sqlx::query(
            "INSERT INTO audit (ts, admin_id, action, target, detail) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(now())
        .bind(admin)
        .bind(action)
        .bind(target)
        .bind(detail)
        .execute(&self.db)
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, "cannot write audit record");
        }
    }

    pub async fn setup_open(&self) -> bool {
        let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
            .fetch_one(&self.db)
            .await
            .unwrap_or(1);
        admins == 0 && (now() as u64) < self.config.setup_expires_at
    }
}

pub fn router(app: App) -> Router {
    Router::new()
        .route(
            "/health",
            axum::routing::get(|| async { env!("CARGO_PKG_VERSION") }),
        )
        .nest("/api", api::routes())
        .fallback(fallback)
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn(security))
        .with_state(app)
}

async fn fallback(State(app): State<App>, method: Method, request: Request) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = request.uri().path();
    if path == "/setup" || path.starts_with("/setup/") {
        let token = path.strip_prefix("/setup/").unwrap_or("");
        if !app.setup_open().await || !auth::token_matches(token, &app.config.setup_token) {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    asset(path)
}

const SPA_ROUTES: &[&str] = &[
    "",
    "login",
    "setup",
    "welcome",
    "nodes",
    "users",
    "disguises",
    "routing",
    "settings",
];

fn asset(path: &str) -> Response {
    let name = path.trim_start_matches('/');
    if name.split('/').any(|part| part == ".." || part == ".") || name.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (file, mime) = if let Some(file) = Assets::get(name).filter(|_| !name.is_empty()) {
        let mime = match name.rsplit('.').next().unwrap_or("") {
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "svg" => "image/svg+xml",
            "html" => "text/html; charset=utf-8",
            "png" => "image/png",
            "json" => "application/json",
            _ => "application/octet-stream",
        };
        (file, mime)
    } else if SPA_ROUTES.contains(&name.split('/').next().unwrap_or("")) {
        let Some(file) = Assets::get("index.html") else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        (file, "text/html; charset=utf-8")
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Response::new(Body::from(file.data.into_owned()));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
}

pub fn valid_host(host: &str) -> bool {
    let (name_ok, port) = if let Some(rest) = host.strip_prefix('[') {
        let Some((address, after)) = rest.split_once(']') else {
            return false;
        };
        let port = match after {
            "" => None,
            _ => match after.strip_prefix(':') {
                Some(port) => Some(port),
                None => return false,
            },
        };
        (address.parse::<std::net::Ipv6Addr>().is_ok(), port)
    } else {
        let (name, port) = match host.split_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host, None),
        };
        let name_ok = !name.is_empty()
            && name.len() <= 253
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
        (name_ok, port)
    };
    name_ok
        && port.is_none_or(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        })
}

fn valid_origin(origin: &str, host: &str) -> bool {
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(|rest| rest == host)
}

async fn security(request: Request, next: Next) -> Response {
    let hosts: Vec<_> = request.headers().get_all(header::HOST).iter().collect();
    let host = hosts
        .first()
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let host_ok = hosts.len() == 1 && host.as_deref().is_some_and(valid_host);
    let origin_ok = request.headers().get(header::ORIGIN).is_none_or(|origin| {
        origin
            .to_str()
            .ok()
            .zip(host.as_deref())
            .is_some_and(|(origin, host)| valid_origin(origin, host))
    });
    let event_stream = request.uri().path() == "/api/events";
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let https = auth::client(request.headers(), peer).https;
    let mut response = if !host_ok {
        StatusCode::BAD_REQUEST.into_response()
    } else if !origin_ok {
        StatusCode::FORBIDDEN.into_response()
    } else {
        next.run(request).await
    };
    for (name, value) in [
        (
            "content-security-policy",
            "default-src 'self'; style-src 'self' 'unsafe-hashes' 'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo='; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    if https {
        response.headers_mut().insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    if !event_stream {
        response
            .headers_mut()
            .insert("cache-control", HeaderValue::from_static("no-store"));
    }
    response
}

pub async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = terminate.recv() => {},
                _ = tokio::signal::ctrl_c() => {},
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

pub fn io_error(message: String) -> io::Error {
    io::Error::other(message)
}

#[cfg(test)]
mod listen_tests {
    use super::*;

    #[test]
    fn the_panel_listens_on_loopback_unless_told_otherwise() {
        let local = initial_listen(None).unwrap();
        assert!(local.ip().is_loopback());
        assert_eq!(local.port(), PANEL_PORT);
        assert_eq!(initial_listen(Some(" ")).unwrap(), local);
        assert_eq!(
            initial_listen(Some("0.0.0.0:8080")).unwrap(),
            "0.0.0.0:8080".parse().unwrap()
        );
        assert!(initial_listen(Some("anywhere")).is_err());
    }
}
