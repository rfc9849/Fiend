use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use totp_rs::{Builder, Secret, Totp};

use crate::types::{
    AdminView, Login, SessionInfo, SetupAdmin, SetupStatus, TotpSetup, WizardState,
};
use crate::{ApiError, ApiResult, App, now};

pub const COOKIE: &str = "fiend_session";
const IDLE: i64 = 12 * 3600;
const ABSOLUTE: i64 = 7 * 86400;
const FIRST_DELAY: Duration = Duration::from_secs(60);
const MAX_DELAY: Duration = Duration::from_secs(15 * 60);
const FORGET: Duration = Duration::from_secs(24 * 3600);
const MAX_KEYS: usize = 10_000;
pub const SOURCE_FREE: u32 = 5;
pub const ACCOUNT_FREE: u32 = 50;
pub const CODE_FREE: u32 = 5;
pub const PASSWORD_FREE: u32 = 5;
const MAX_NAME: usize = 64;

struct Entry {
    failures: u32,
    last: Instant,
    until: Instant,
}

#[derive(Default)]
pub struct Limiter {
    entries: Mutex<HashMap<String, Entry>>,
}

fn delay(failures: u32, free: u32) -> Duration {
    if failures < free {
        return Duration::ZERO;
    }
    FIRST_DELAY
        .saturating_mul(1 << (failures - free).min(16))
        .min(MAX_DELAY)
}

fn wait_message(wait: Duration) -> String {
    let seconds = wait.as_secs().max(1);
    if seconds <= 60 {
        format!("Too many attempts. Wait {seconds} seconds and try again.")
    } else {
        format!(
            "Too many attempts. Wait {} minutes and try again.",
            seconds.div_ceil(60)
        )
    }
}

impl Limiter {
    fn entries(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn check(&self, key: &str) -> ApiResult<()> {
        let now = Instant::now();
        match self.entries().get(key) {
            Some(entry) if entry.until > now => Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                wait_message(entry.until - now),
            )),
            _ => Ok(()),
        }
    }

    pub fn fail(&self, key: &str, free: u32) {
        let now = Instant::now();
        let mut entries = self.entries();
        if !entries.contains_key(key) && entries.len() >= MAX_KEYS {
            entries.retain(|_, entry| now.duration_since(entry.last) < FORGET);
            if entries.len() >= MAX_KEYS {
                entries.retain(|_, entry| entry.until > now);
            }
            if entries.len() >= MAX_KEYS {
                return;
            }
        }
        let entry = entries.entry(key.to_owned()).or_insert(Entry {
            failures: 0,
            last: now,
            until: now,
        });
        if now.duration_since(entry.last) >= FORGET {
            entry.failures = 0;
        }
        entry.failures = entry.failures.saturating_add(1);
        entry.last = now;
        entry.until = now + delay(entry.failures, free);
    }

    pub fn clear(&self, key: &str) {
        self.entries().remove(key);
    }
}

pub struct Client {
    pub source: String,
    pub https: bool,
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn source_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => {
            let [a, b, c, d, ..] = ip.segments();
            format!("{a:x}:{b:x}:{c:x}:{d:x}::/64")
        }
    }
}

pub fn client(headers: &HeaderMap, peer: Option<SocketAddr>) -> Client {
    let proxied = peer.is_some_and(|peer| peer.ip().to_canonical().is_loopback());
    let forwarded = proxied
        .then(|| header_text(headers, "x-real-ip").and_then(|ip| ip.trim().parse::<IpAddr>().ok()))
        .flatten();
    let source = forwarded
        .or(peer.map(|peer| peer.ip()))
        .map_or_else(|| "unknown".to_owned(), source_key);
    let https = header_text(headers, header::ORIGIN.as_str())
        .is_some_and(|origin| origin.starts_with("https://"))
        || (proxied
            && header_text(headers, "x-forwarded-proto")
                .is_some_and(|proto| proto.trim().eq_ignore_ascii_case("https")));
    Client { source, https }
}

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0);
        Ok(client(&parts.headers, peer))
    }
}

pub fn token_matches(given: &str, expected: &str) -> bool {
    let given = Sha256::digest(given.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    given
        .iter()
        .zip(expected.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

pub fn hash_password(password: &str) -> ApiResult<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(ApiError::internal)
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn check_password_rules(password: &str) -> ApiResult<()> {
    if password.chars().count() < 12 || password.len() > 256 {
        return Err(ApiError::field("password", "Use 12 to 256 characters."));
    }
    Ok(())
}

pub fn check_name(name: &str, field: &'static str) -> ApiResult<()> {
    if name.trim() != name
        || name.is_empty()
        || name.len() > 64
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::field(
            field,
            "Use 1 to 64 characters with no spaces at the ends.",
        ));
    }
    Ok(())
}

fn totp(secret: &[u8], account: &str) -> ApiResult<Totp> {
    Builder::new()
        .with_secret(Secret::new(secret.to_vec().into_boxed_slice()))
        .with_account_name(account.replace(':', "_"))
        .with_issuer(Some("fiend panel"))
        .build()
        .map_err(|_| ApiError::field("totp_code", "The authenticator secret is not valid."))
}

pub fn new_totp(account: &str) -> ApiResult<TotpSetup> {
    let secret: [u8; 20] = rand::random();
    let totp = totp(&secret, account)?;
    let uri = totp.to_url().map_err(ApiError::internal)?;
    Ok(TotpSetup {
        secret: base32(&secret),
        qr_svg: crate::qr::svg(&uri).map_err(ApiError::internal)?,
        uri,
    })
}

pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            output.push(ALPHABET[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        output.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    output
}

pub fn decode_base32(text: &str) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for character in text.trim().trim_end_matches('=').bytes() {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a',
            b'2'..=b'7' => character - b'2' + 26,
            b' ' | b'-' => continue,
            _ => return None,
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            output.push((buffer >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(output)
}

fn code_key(admin_id: i64) -> String {
    format!("code:{admin_id}")
}

impl App {
    pub fn verify_code(&self, admin_id: i64, secret_hex: &str, code: &str) -> ApiResult<bool> {
        let key = code_key(admin_id);
        self.limiter.check(&key)?;
        if self.totp_matches(admin_id, secret_hex, code) {
            self.limiter.clear(&key);
            return Ok(true);
        }
        self.limiter.fail(&key, CODE_FREE);
        Ok(false)
    }

    fn totp_matches(&self, admin_id: i64, secret_hex: &str, code: &str) -> bool {
        let Ok(secret) = hex::decode(secret_hex) else {
            return false;
        };
        let Ok(totp) = totp(&secret, "admin") else {
            return false;
        };
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let Some(step) = totp.check_current(&code) else {
            return false;
        };
        let mut used = self
            .totp_steps
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if used.get(&admin_id).is_some_and(|last| *last >= step) {
            return false;
        }
        used.insert(admin_id, step);
        true
    }
}

pub fn totp_secret_from_input(secret: &str, code: &str) -> ApiResult<String> {
    let bytes = decode_base32(secret)
        .filter(|bytes| bytes.len() >= 16)
        .ok_or_else(|| ApiError::field("totp_code", "The authenticator secret is not valid."))?;
    let totp = totp(&bytes, "admin")?;
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if totp.check_current(&code).is_none() {
        return Err(ApiError::field(
            "totp_code",
            "The code is not correct. Check the time on your device.",
        ));
    }
    Ok(hex::encode(bytes))
}

#[derive(Clone)]
pub struct Admin {
    pub id: i64,
    pub name: String,
    pub totp_secret: Option<String>,
    pub csrf: String,
    pub session_hash: String,
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_owned())
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

impl FromRequestParts<App> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, Self::Rejection> {
        let unauthorized = || ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to continue.");
        let token = cookie_value(&parts.headers).ok_or_else(unauthorized)?;
        if token.len() != 64 {
            return Err(unauthorized());
        }
        let hash = hash_token(&token);
        let row: Option<(i64, String, Option<String>, String, i64, i64)> = sqlx::query_as(
            "SELECT admins.id, admins.name, admins.totp_secret, sessions.csrf, sessions.created_at, sessions.last_seen FROM sessions JOIN admins ON admins.id = sessions.admin_id WHERE sessions.id_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(&app.db)
        .await?;
        let (id, name, totp_secret, csrf, created_at, last_seen) = row.ok_or_else(unauthorized)?;
        let current = now();
        if current - last_seen > IDLE || current - created_at > ABSOLUTE {
            sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
                .bind(&hash)
                .execute(&app.db)
                .await?;
            return Err(unauthorized());
        }
        if !matches!(parts.method, Method::GET | Method::HEAD) {
            let given = parts
                .headers
                .get("x-csrf-token")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("");
            if !token_matches(given, &csrf) {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "The request token is not valid. Reload the page.",
                ));
            }
        }
        if current - last_seen > 30 {
            sqlx::query("UPDATE sessions SET last_seen = ? WHERE id_hash = ?")
                .bind(current)
                .bind(&hash)
                .execute(&app.db)
                .await?;
        }
        Ok(Self {
            id,
            name,
            totp_secret,
            csrf,
            session_hash: hash,
        })
    }
}

async fn create_session(app: &App, admin_id: i64) -> ApiResult<String> {
    let token = hex::encode(rand::random::<[u8; 32]>());
    let csrf = hex::encode(rand::random::<[u8; 32]>());
    sqlx::query("INSERT INTO sessions (id_hash, admin_id, csrf, created_at, last_seen) VALUES (?, ?, ?, ?, ?)")
        .bind(hash_token(&token))
        .bind(admin_id)
        .bind(csrf)
        .bind(now())
        .bind(now())
        .execute(&app.db)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE last_seen < ? OR created_at < ?")
        .bind(now() - IDLE)
        .bind(now() - ABSOLUTE)
        .execute(&app.db)
        .await?;
    Ok(token)
}

fn session_cookie(token: &str, max_age: i64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}"
    ))
    .expect("cookie is ASCII")
}

pub async fn wizard(app: &App) -> ApiResult<WizardState> {
    let local: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE local = 1")
        .fetch_one(&app.db)
        .await?;
    let disguises: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nodes WHERE local = 1 AND disguise_id IS NOT NULL",
    )
    .fetch_one(&app.db)
    .await?;
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&app.db)
        .await?;
    let skipped = crate::db::setting(&app.db, "wizard_done").await.is_some();
    Ok(WizardState {
        local_node: local > 0,
        disguise: disguises > 0,
        first_user: users > 0,
        done: skipped || (local > 0 && disguises > 0 && users > 0),
    })
}

pub async fn session_info(app: &App, admin: &Admin) -> ApiResult<SessionInfo> {
    let created_at: i64 = sqlx::query_scalar("SELECT created_at FROM admins WHERE id = ?")
        .bind(admin.id)
        .fetch_one(&app.db)
        .await?;
    Ok(SessionInfo {
        admin: AdminView {
            id: admin.id,
            name: admin.name.clone(),
            totp: admin.totp_secret.is_some(),
            created_at,
        },
        csrf: admin.csrf.clone(),
        wizard: wizard(app).await?,
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

pub async fn setup_status(
    State(app): State<App>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> ApiResult<Json<SetupStatus>> {
    let open = app.setup_open().await && token_matches(&token, &app.config.setup_token);
    if !open {
        return Err(ApiError::not_found());
    }
    Ok(Json(SetupStatus { open }))
}

pub async fn setup_totp(
    State(app): State<App>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> ApiResult<Json<TotpSetup>> {
    if !app.setup_open().await || !token_matches(&token, &app.config.setup_token) {
        return Err(ApiError::not_found());
    }
    Ok(Json(new_totp("admin")?))
}

pub async fn setup_admin(
    State(app): State<App>,
    client: Client,
    Json(input): Json<SetupAdmin>,
) -> ApiResult<Response> {
    let key = format!("setup:{}", client.source);
    app.limiter.check(&key)?;
    if !app.setup_open().await || !token_matches(&input.token, &app.config.setup_token) {
        app.limiter.fail(&key, SOURCE_FREE);
        return Err(ApiError::not_found());
    }
    check_name(&input.name, "name")?;
    check_password_rules(&input.password)?;
    let totp_secret = match (&input.totp_secret, &input.totp_code) {
        (Some(secret), Some(code)) if !secret.is_empty() => {
            Some(totp_secret_from_input(secret, code)?)
        }
        _ => None,
    };
    let hash = hash_password(&input.password)?;
    let mut transaction = app.db.begin().await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
        .fetch_one(&mut *transaction)
        .await?;
    if count != 0 {
        return Err(ApiError::not_found());
    }
    let id: i64 = sqlx::query_scalar("INSERT INTO admins (name, password_hash, totp_secret, created_at) VALUES (?, ?, ?, ?) RETURNING id")
        .bind(&input.name)
        .bind(hash)
        .bind(totp_secret)
        .bind(now())
        .fetch_one(&mut *transaction)
        .await?;
    transaction.commit().await?;
    app.audit(
        Some(id),
        "setup",
        &format!("admin {}", input.name),
        "first admin created",
    )
    .await;
    let token = create_session(&app, id).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&token, ABSOLUTE, client.https),
    );
    Ok(response)
}

fn login_failed() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "The name, password, or code is not correct.",
    )
}

pub async fn login(
    State(app): State<App>,
    client: Client,
    Json(input): Json<Login>,
) -> ApiResult<Response> {
    let source_key = format!("login-source:{}", client.source);
    app.limiter.check(&source_key)?;
    if input.name.len() > MAX_NAME {
        app.limiter.fail(&source_key, SOURCE_FREE);
        return Err(login_failed());
    }
    let account_key = format!("login-name:{}", input.name);
    app.limiter.check(&account_key)?;
    let row: Option<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash, totp_secret FROM admins WHERE name = ?")
            .bind(&input.name)
            .fetch_optional(&app.db)
            .await?;
    let fail = |app: &App| {
        app.limiter.fail(&source_key, SOURCE_FREE);
        app.limiter.fail(&account_key, ACCOUNT_FREE);
        login_failed()
    };
    let Some((id, hash, totp_secret)) = row else {
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let dummy = DUMMY.get_or_init(|| hash_password("fiend dummy password").unwrap_or_default());
        let _ = verify_password(&input.password, dummy);
        return Err(fail(&app));
    };
    if totp_secret.is_some() {
        app.limiter.check(&code_key(id))?;
    }
    if !verify_password(&input.password, &hash) {
        return Err(fail(&app));
    }
    if let Some(secret) = &totp_secret {
        let code = input.totp_code.as_deref().unwrap_or("");
        if !app.verify_code(id, secret, code)? {
            return Err(fail(&app));
        }
    }
    app.limiter.clear(&source_key);
    app.limiter.clear(&account_key);
    let token = create_session(&app, id).await?;
    app.audit(Some(id), "login", &format!("admin {}", input.name), "")
        .await;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&token, ABSOLUTE, client.https),
    );
    Ok(response)
}

pub async fn logout(State(app): State<App>, client: Client, admin: Admin) -> ApiResult<Response> {
    sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
        .bind(&admin.session_hash)
        .execute(&app.db)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, session_cookie("", 0, client.https));
    Ok(response)
}

pub async fn session(State(app): State<App>, admin: Admin) -> ApiResult<Json<SessionInfo>> {
    Ok(Json(session_info(&app, &admin).await?))
}

pub fn require_code(app: &App, admin: &Admin, code: &str) -> ApiResult<()> {
    let secret = admin.totp_secret.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "Turn on two-factor sign-in before you change admins.",
        )
    })?;
    if !app.verify_code(admin.id, secret, code)? {
        return Err(ApiError::field("current_code", "The code is not correct."));
    }
    Ok(())
}

pub async fn confirm_identity(
    app: &App,
    admin: &Admin,
    password: &str,
    code: Option<&str>,
) -> ApiResult<()> {
    let key = format!("password:{}", admin.id);
    app.limiter.check(&key)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM admins WHERE id = ?")
        .bind(admin.id)
        .fetch_one(&app.db)
        .await?;
    if !verify_password(password, &hash) {
        app.limiter.fail(&key, PASSWORD_FREE);
        return Err(ApiError::field(
            "current",
            "The current password is not correct.",
        ));
    }
    app.limiter.clear(&key);
    if let Some(secret) = &admin.totp_secret
        && !app.verify_code(admin.id, secret, code.unwrap_or(""))?
    {
        return Err(ApiError::field("totp_code", "The code is not correct."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_doubles_after_free_failures_and_stops_at_the_cap() {
        assert_eq!(delay(4, 5), Duration::ZERO);
        assert_eq!(delay(5, 5), FIRST_DELAY);
        assert_eq!(delay(6, 5), FIRST_DELAY * 2);
        assert_eq!(delay(8, 5), FIRST_DELAY * 8);
        assert_eq!(delay(9, 5), MAX_DELAY);
        assert_eq!(delay(u32::MAX, 5), MAX_DELAY);
    }

    #[test]
    fn limiter_blocks_and_clears() {
        let limiter = Limiter::default();
        for _ in 0..4 {
            limiter.fail("a", 5);
        }
        assert!(limiter.check("a").is_ok());
        limiter.fail("a", 5);
        assert_eq!(
            limiter.check("a").unwrap_err().status,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert!(limiter.check("b").is_ok());
        limiter.clear("a");
        assert!(limiter.check("a").is_ok());
    }
}
