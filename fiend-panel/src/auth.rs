use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use axum::Json;
use axum::extract::{FromRequestParts, State};
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
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const LOGIN_LIMIT: usize = 5;

#[derive(Default)]
pub struct Limiter {
    attempts: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Limiter {
    pub fn check(&self, key: &str) -> ApiResult<()> {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        attempts.retain(|_, list| {
            list.retain(|at| now.duration_since(*at) < LOGIN_WINDOW);
            !list.is_empty()
        });
        let used = attempts.get(key).map_or(0, Vec::len);
        if used >= LOGIN_LIMIT {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many attempts. Wait one minute and try again.",
            ));
        }
        Ok(())
    }

    pub fn fail(&self, key: &str) {
        self.attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(key.to_owned())
            .or_default()
            .push(Instant::now());
    }

    pub fn clear(&self, key: &str) {
        self.attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(key);
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

impl App {
    pub fn check_totp(&self, admin_id: i64, secret_hex: &str, code: &str) -> bool {
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

fn session_cookie(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ABSOLUTE}"
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
    Json(input): Json<SetupAdmin>,
) -> ApiResult<Response> {
    app.limiter.check("setup")?;
    if !app.setup_open().await || !token_matches(&input.token, &app.config.setup_token) {
        app.limiter.fail("setup");
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
    response
        .headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&token));
    Ok(response)
}

pub async fn login(State(app): State<App>, Json(input): Json<Login>) -> ApiResult<Response> {
    let key = format!("login:{}", input.name);
    app.limiter.check(&key)?;
    app.limiter.check("login")?;
    let row: Option<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash, totp_secret FROM admins WHERE name = ?")
            .bind(&input.name)
            .fetch_optional(&app.db)
            .await?;
    let fail = |app: &App| {
        app.limiter.fail(&key);
        app.limiter.fail("login");
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "The name, password, or code is not correct.",
        )
    };
    let Some((id, hash, totp_secret)) = row else {
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let dummy = DUMMY.get_or_init(|| hash_password("fiend dummy password").unwrap_or_default());
        let _ = verify_password(&input.password, dummy);
        return Err(fail(&app));
    };
    if !verify_password(&input.password, &hash) {
        return Err(fail(&app));
    }
    if let Some(secret) = &totp_secret {
        let code = input.totp_code.as_deref().unwrap_or("");
        if !app.check_totp(id, secret, code) {
            return Err(fail(&app));
        }
    }
    app.limiter.clear(&key);
    let token = create_session(&app, id).await?;
    app.audit(Some(id), "login", &format!("admin {}", input.name), "")
        .await;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&token));
    Ok(response)
}

pub async fn logout(State(app): State<App>, admin: Admin) -> ApiResult<Response> {
    sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
        .bind(&admin.session_hash)
        .execute(&app.db)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("fiend_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
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
    if !app.check_totp(admin.id, secret, code) {
        return Err(ApiError::field("current_code", "The code is not correct."));
    }
    Ok(())
}
