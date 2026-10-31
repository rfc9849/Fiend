use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use zeroize::Zeroizing;

use super::outbound::WarpSettings;
use super::unix_time;

const API: &str = "https://api.cloudflareclient.com/v0a5641";
const CLIENT_VERSION: &str = "a-6.38.9-5641";
const USER_AGENT: &str = "okhttp/3.12.1";
const API_TIMEOUT: Duration = Duration::from_secs(20);
const START_TIMEOUT: Duration = Duration::from_secs(15);
const ENDPOINT_PORTS: [u16; 4] = [2408, 500, 1701, 4500];

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    id: String,
    token: String,
    private_key: String,
    peer_public_key: String,
    endpoint: String,
    address_v4: String,
    address_v6: String,
    created_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Exit {
    pub ip: Option<String>,
    pub colo: Option<String>,
    pub warp: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Status {
    pub enabled: bool,
    pub registered: bool,
    pub running: bool,
    pub account_id: Option<String>,
    pub created_at: Option<u64>,
    pub rotated_at: Option<u64>,
    pub rotate_every: Option<u64>,
    pub socks: Option<String>,
    pub v4: Option<Exit>,
    pub v6: Option<Exit>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rotation {
    pub before: Status,
    pub after: Status,
}

struct Running {
    child: Child,
    socks: SocketAddr,
    account_id: String,
}

struct State {
    settings: WarpSettings,
    running: Option<Running>,
    rotated_at: Option<u64>,
    last_error: Option<String>,
}

#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

struct Inner {
    directory: PathBuf,
    binary: PathBuf,
    api: String,
    state: Mutex<State>,
    work: tokio::sync::Mutex<()>,
}

fn other(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

impl Manager {
    pub fn new(directory: PathBuf, binary: PathBuf) -> Self {
        Self::with_api(directory, binary, API.into())
    }

    pub fn with_api(directory: PathBuf, binary: PathBuf, api: String) -> Self {
        let rotated_at = std::fs::read_to_string(directory.join("rotated_at"))
            .ok()
            .and_then(|text| text.trim().parse().ok());
        Self {
            inner: Arc::new(Inner {
                directory,
                binary,
                api,
                state: Mutex::new(State {
                    settings: WarpSettings::default(),
                    running: None,
                    rotated_at,
                    last_error: None,
                }),
                work: tokio::sync::Mutex::new(()),
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn socks(&self) -> Option<SocketAddr> {
        let mut state = self.state();
        let alive = state
            .running
            .as_mut()
            .is_some_and(|running| matches!(running.child.try_wait(), Ok(None)));
        if !alive {
            state.running = None;
        }
        state.running.as_ref().map(|running| running.socks)
    }

    fn account_path(&self) -> PathBuf {
        self.inner.directory.join("account.json")
    }

    fn load(&self) -> io::Result<Option<Account>> {
        match std::fs::read(self.account_path()) {
            Ok(raw) => serde_json::from_slice(&raw)
                .map(Some)
                .map_err(|_| other("saved WARP account is not valid")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn save(&self, account: &Account) -> io::Result<()> {
        prepare_directory(&self.inner.directory)?;
        let raw = Zeroizing::new(serde_json::to_vec(account).map_err(io::Error::other)?);
        super::save_private(&self.account_path(), &raw)
    }

    fn mark_rotated(&self) {
        let now = unix_time();
        self.state().rotated_at = Some(now);
        let _ = super::save_private(
            &self.inner.directory.join("rotated_at"),
            now.to_string().as_bytes(),
        );
    }

    pub async fn apply(&self, settings: WarpSettings) {
        let _work = self.inner.work.lock().await;
        self.state().settings = settings.clone();
        if !settings.enabled {
            self.stop();
            return;
        }
        if let Err(error) = self.ensure_running().await {
            tracing::warn!(%error, "cannot start WARP");
            self.state().last_error = Some(error.to_string());
        }
    }

    async fn ensure_running(&self) -> io::Result<()> {
        if self.socks().is_some() {
            return Ok(());
        }
        let Some(account) = self.load()? else {
            return Ok(());
        };
        let running = self.start(&account).await?;
        self.state().running = Some(running);
        Ok(())
    }

    fn stop(&self) {
        if let Some(mut running) = self.state().running.take() {
            let _ = running.child.start_kill();
        }
    }

    async fn start(&self, account: &Account) -> io::Result<Running> {
        prepare_directory(&self.inner.directory)?;
        let socks = free_port()?;
        let name = format!("wireproxy-{}.conf", &account.id[..account.id.len().min(8)]);
        let path = self.inner.directory.join(name);
        let config = Zeroizing::new(format!(
            "[Interface]\nPrivateKey = {}\nAddress = {}/32, {}/128\nDNS = 1.1.1.1, 2606:4700:4700::1111\nMTU = 1280\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = {}\nPersistentKeepalive = 25\n\n[Socks5]\nBindAddress = {}\n",
            account.private_key,
            account.address_v4,
            account.address_v6,
            account.peer_public_key,
            account.endpoint,
            socks
        ));
        super::save_private(&path, config.as_bytes())?;
        let mut child = Command::new(&self.inner.binary)
            .arg("--silent")
            .arg("--config")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| other(format!("cannot start wireproxy: {error}")))?;
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(other(format!("wireproxy stopped at start: {status}")));
            }
            if tokio::net::TcpStream::connect(socks).await.is_ok() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                return Err(other("wireproxy did not open its SOCKS5 port"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tracing::info!(%socks, "WARP proxy started");
        Ok(Running {
            child,
            socks,
            account_id: account.id.clone(),
        })
    }

    pub async fn register(&self) -> io::Result<Status> {
        {
            let _work = self.inner.work.lock().await;
            if !self.state().settings.enabled {
                return Err(other("enable WARP in the node settings first"));
            }
            if self.load()?.is_some() {
                return Err(other("this node already has a WARP account"));
            }
            let account = self.create_account().await?;
            self.save(&account)?;
            self.mark_rotated();
            let running = self.start(&account).await?;
            self.state().running = Some(running);
        }
        Ok(self.status().await)
    }

    pub async fn rotate(&self) -> io::Result<Rotation> {
        let before = self.status().await;
        {
            let _work = self.inner.work.lock().await;
            let old = self
                .load()?
                .ok_or_else(|| other("this node has no WARP account"))?;
            let mut account = self.create_account().await?;
            let old_v4 = before.v4.as_ref().and_then(|exit| exit.ip.clone());
            let host = account
                .endpoint
                .rsplit_once(':')
                .map_or(account.endpoint.clone(), |(host, _)| host.to_owned());
            let mut chosen = None;
            let mut last_error = None;
            for (attempt, port) in ENDPOINT_PORTS.iter().enumerate() {
                account.endpoint = format!("{host}:{port}");
                let mut running = match self.start(&account).await {
                    Ok(running) => running,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };
                let exit = trace(running.socks, "https://1.1.1.1/cdn-cgi/trace")
                    .await
                    .ok();
                let new_v4 = exit.and_then(|exit| exit.ip);
                let changed = new_v4.is_some() && new_v4 != old_v4;
                if changed || attempt + 1 == ENDPOINT_PORTS.len() {
                    chosen = Some(running);
                    break;
                }
                tracing::info!(
                    port,
                    "WARP IPv4 exit did not change, trying another endpoint port"
                );
                let _ = running.child.start_kill();
                let _ = running.child.wait().await;
            }
            let Some(running) = chosen else {
                let _ = self.delete_account(&account).await;
                return Err(
                    last_error.unwrap_or_else(|| other("cannot start the new WARP account"))
                );
            };
            self.save(&account)?;
            let previous = self.state().running.replace(running);
            if let Some(mut previous) = previous {
                let _ = previous.child.start_kill();
            }
            self.mark_rotated();
            if let Err(error) = self.delete_account(&old).await {
                tracing::warn!(%error, "cannot delete the old WARP account");
            }
            let _ = std::fs::remove_file(
                self.inner
                    .directory
                    .join(format!("wireproxy-{}.conf", &old.id[..old.id.len().min(8)])),
            );
        }
        let after = self.status().await;
        Ok(Rotation { before, after })
    }

    pub async fn delete(&self) -> io::Result<()> {
        let _work = self.inner.work.lock().await;
        self.stop();
        if let Some(account) = self.load()? {
            self.delete_account(&account).await?;
        }
        if self.inner.directory.exists() {
            for entry in std::fs::read_dir(&self.inner.directory)? {
                let _ = std::fs::remove_file(entry?.path());
            }
        }
        self.state().rotated_at = None;
        Ok(())
    }

    pub async fn rotation_due(&self) -> bool {
        let state = self.state();
        let (Some(every), Some(at)) = (state.settings.rotate_every, state.rotated_at) else {
            return false;
        };
        state.settings.enabled && state.running.is_some() && unix_time() >= at.saturating_add(every)
    }

    pub async fn maintain(&self) {
        let mut timer = tokio::time::interval(Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let enabled = self.state().settings.enabled;
            if !enabled {
                continue;
            }
            if self.socks().is_none() {
                let _work = self.inner.work.lock().await;
                if let Err(error) = self.ensure_running().await {
                    tracing::warn!(%error, "cannot restart WARP");
                    self.state().last_error = Some(error.to_string());
                }
            }
            if self.rotation_due().await {
                match self.rotate().await {
                    Ok(rotation) => tracing::info!(
                        before = ?rotation.before.v4.and_then(|exit| exit.ip),
                        after = ?rotation.after.v4.and_then(|exit| exit.ip),
                        "WARP rotated"
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "WARP rotation failed");
                        self.state().last_error = Some(error.to_string());
                    }
                }
            }
        }
    }

    pub async fn status(&self) -> Status {
        let account = self.load().ok().flatten();
        let socks = self.socks();
        let (enabled, rotated_at, rotate_every, mut error, running_id) = {
            let state = self.state();
            (
                state.settings.enabled,
                state.rotated_at,
                state.settings.rotate_every,
                state.last_error.clone(),
                state
                    .running
                    .as_ref()
                    .map(|running| running.account_id.clone()),
            )
        };
        let (v4, v6) = match socks {
            Some(socks) => {
                let v4 = trace(socks, "https://1.1.1.1/cdn-cgi/trace").await;
                let v6 = trace(socks, "https://[2606:4700:4700::1111]/cdn-cgi/trace").await;
                if let (Err(failure), Err(_)) = (&v4, &v6) {
                    error = Some(failure.to_string());
                }
                (v4.ok(), v6.ok())
            }
            None => (None, None),
        };
        Status {
            enabled,
            registered: account.is_some(),
            running: socks.is_some() && running_id.is_some(),
            account_id: account.as_ref().map(|account| account.id.clone()),
            created_at: account.as_ref().map(|account| account.created_at),
            rotated_at,
            rotate_every,
            socks: socks.map(|socks| socks.to_string()),
            v4,
            v6,
            error,
        }
    }

    async fn create_account(&self) -> io::Result<Account> {
        let mut private = Zeroizing::new(rand::random::<[u8; 32]>());
        private[0] &= 248;
        private[31] &= 127;
        private[31] |= 64;
        let public = x25519_dalek::x25519(*private, x25519_dalek::X25519_BASEPOINT_BYTES);
        let body = serde_json::json!({
            "key": STANDARD.encode(public),
            "install_id": "",
            "fcm_token": "",
            "tos": tos_time(),
            "model": "PC",
            "serial_number": "",
            "locale": "en_US",
            "key_type": "curve25519",
            "tunnel_type": "wireguard",
        })
        .to_string();
        let url = format!("{}/reg", self.inner.api);
        let text = tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(API_TIMEOUT))
                .build()
                .into();
            agent
                .post(&url)
                .header("User-Agent", USER_AGENT)
                .header("CF-Client-Version", CLIENT_VERSION)
                .header("Content-Type", "application/json; charset=UTF-8")
                .send(body)
                .map_err(|error| other(format!("WARP registration failed: {error}")))?
                .body_mut()
                .read_to_string()
                .map_err(|error| other(format!("WARP registration reply failed: {error}")))
        })
        .await
        .map_err(io::Error::other)??;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| other("WARP registration reply is not JSON"))?;
        let result = value.get("result").unwrap_or(&value);
        let text = |pointer: &str| {
            result
                .pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| other(format!("WARP registration reply has no {pointer}")))
        };
        let endpoint_v4 = text("/config/peers/0/endpoint/v4")?;
        let endpoint_ip = endpoint_v4
            .rsplit_once(':')
            .map_or(endpoint_v4.as_str(), |(host, _)| host)
            .to_owned();
        let endpoint_ip: Ipv4Addr = endpoint_ip
            .parse()
            .map_err(|_| other("WARP endpoint is not an IPv4 address"))?;
        Ok(Account {
            id: text("/id")?,
            token: text("/token")?,
            private_key: STANDARD.encode(*private),
            peer_public_key: text("/config/peers/0/public_key")?,
            endpoint: format!("{endpoint_ip}:2408"),
            address_v4: text("/config/interface/addresses/v4")?,
            address_v6: text("/config/interface/addresses/v6")?,
            created_at: unix_time(),
        })
    }

    async fn delete_account(&self, account: &Account) -> io::Result<()> {
        let url = format!("{}/reg/{}", self.inner.api, account.id);
        let token = Zeroizing::new(format!("Bearer {}", account.token));
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(API_TIMEOUT))
                .build()
                .into();
            agent
                .delete(&url)
                .header("User-Agent", USER_AGENT)
                .header("CF-Client-Version", CLIENT_VERSION)
                .header("Authorization", token.as_str())
                .call()
                .map(drop)
                .map_err(|error| other(format!("WARP account delete failed: {error}")))
        })
        .await
        .map_err(io::Error::other)?
    }
}

fn prepare_directory(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn free_port() -> io::Result<SocketAddr> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()
}

fn tos_time() -> String {
    let now = unix_time();
    let days = (now / 86400) as i64;
    let seconds = now % 86400;
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
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
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub async fn trace(socks: SocketAddr, url: &str) -> io::Result<Exit> {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let proxy = ureq::Proxy::new(&format!("socks5://{socks}"))
            .map_err(|error| other(format!("invalid WARP proxy: {error}")))?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(Some(proxy))
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .into();
        let text = agent
            .get(&url)
            .call()
            .map_err(|error| other(format!("WARP trace failed: {error}")))?
            .body_mut()
            .with_config()
            .limit(16 * 1024)
            .read_to_string()
            .map_err(|error| other(format!("WARP trace failed: {error}")))?;
        Ok(parse_trace(&text))
    })
    .await
    .map_err(io::Error::other)?
}

pub fn parse_trace(text: &str) -> Exit {
    let mut exit = Exit::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("ip", value)) => exit.ip = Some(value.to_owned()),
            Some(("colo", value)) => exit.colo = Some(value.to_owned()),
            Some(("warp", value)) => exit.warp = Some(value.to_owned()),
            _ => {}
        }
    }
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_and_dates_parse() {
        let exit = parse_trace("fl=1\nip=104.16.0.1\ncolo=AMS\nwarp=on\n");
        assert_eq!(exit.ip.as_deref(), Some("104.16.0.1"));
        assert_eq!(exit.colo.as_deref(), Some("AMS"));
        assert_eq!(exit.warp.as_deref(), Some("on"));
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_392), (2025, 10, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert!(tos_time().ends_with(".000Z"));
    }
}
