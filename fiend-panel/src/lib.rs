use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};

#[derive(RustEmbed)]
#[folder = "web/build/"]
struct Assets;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub listen: SocketAddr,
    pub setup_token: String,
    pub setup_expires_at: u64,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let config: Self = toml::from_str(&raw).map_err(|error| error.to_string())?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.listen.ip() != Ipv4Addr::LOCALHOST || self.listen.port() == 0 {
            return Err("panel listen must be 127.0.0.1 with a nonzero port".into());
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
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        let config = Self {
            listen: listener.local_addr().map_err(|e| e.to_string())?,
            setup_token: hex::encode(rand::random::<[u8; 32]>()),
            setup_expires_at: now() + 3600,
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
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn router(config: Config) -> Router {
    let token = config.setup_token;
    let expires = config.setup_expires_at;
    Router::new()
        .route("/health", get(|| async { env!("CARGO_PKG_VERSION") }))
        .fallback(get(move |request: Request| {
            let token = token.clone();
            async move {
                let path = request.uri().path();
                if (path == "/setup" || path.starts_with("/setup/"))
                    && (path != format!("/setup/{token}") || now() >= expires)
                {
                    return StatusCode::NOT_FOUND.into_response();
                }
                asset(path)
            }
        }))
        .layer(middleware::from_fn(security))
}

fn asset(path: &str) -> Response {
    let name = path.trim_start_matches('/');
    if name.split('/').any(|part| part == ".." || part == ".") || name.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (file, mime) = if let Some(file) = Assets::get(name) {
        let mime = match name.rsplit('.').next().unwrap_or("") {
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "svg" => "image/svg+xml",
            "html" => "text/html; charset=utf-8",
            _ => "application/octet-stream",
        };
        (file, mime)
    } else if name.is_empty() || name.starts_with("setup/") {
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
    let Some((name, port)) = host.split_once(':') else {
        return false;
    };
    matches!(name, "127.0.0.1" | "localhost")
        && !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

async fn security(request: Request, next: Next) -> Response {
    let hosts: Vec<_> = request.headers().get_all(header::HOST).iter().collect();
    let mut response = if hosts.len() != 1 || !hosts[0].to_str().is_ok_and(valid_host) {
        StatusCode::BAD_REQUEST.into_response()
    } else {
        next.run(request).await
    };
    for (name, value) in [
        (
            "content-security-policy",
            "default-src 'self'; style-src 'self' 'unsafe-hashes' 'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo='; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        ("cache-control", "no-store"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
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
