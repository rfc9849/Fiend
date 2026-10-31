use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::client::{self, Handle};
use russh::keys::PublicKeyOrCertificate;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey};
use russh::{ChannelMsg, Disconnect};
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

use crate::jobs::Progress;
use crate::types::SshInstall;
use crate::{App, now};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct Scan {
    pub host: String,
    pub port: u16,
    pub key: String,
    pub fingerprint: String,
    pub created: Instant,
}

struct Checker {
    expected: Option<String>,
    seen: Arc<Mutex<Option<PublicKey>>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };
        *self.seen.lock().unwrap_or_else(|error| error.into_inner()) = Some(key.clone());
        Ok(match &self.expected {
            Some(expected) => key
                .to_openssh()
                .map(|text| text == *expected)
                .unwrap_or(false),
            None => true,
        })
    }
}

fn config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    })
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

pub async fn scan(host: &str, port: u16) -> Result<Scan, String> {
    let seen = Arc::new(Mutex::new(None));
    let handler = Checker {
        expected: None,
        seen: seen.clone(),
    };
    let connection = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client::connect(config(), (host, port), handler),
    )
    .await
    .map_err(|_| "The SSH connection timed out".to_owned())?
    .map_err(|error| format!("Cannot connect over SSH: {error}"))?;
    let _ = connection
        .disconnect(Disconnect::ByApplication, "", "en")
        .await;
    let key = seen
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .ok_or_else(|| "The server sent no host key".to_owned())?;
    Ok(Scan {
        host: host.into(),
        port,
        key: key.to_openssh().map_err(|error| error.to_string())?,
        fingerprint: fingerprint(&key),
        created: Instant::now(),
    })
}

pub struct Session {
    handle: Handle<Checker>,
    sudo: Option<Zeroizing<String>>,
    root: bool,
}

pub struct Output {
    pub status: u32,
    pub stdout: String,
    pub stderr: String,
}

impl Session {
    pub async fn connect(scan: &Scan, input: &SshInstall) -> Result<Self, String> {
        let handler = Checker {
            expected: Some(scan.key.clone()),
            seen: Arc::new(Mutex::new(None)),
        };
        let mut handle = tokio::time::timeout(
            CONNECT_TIMEOUT,
            client::connect(config(), (scan.host.as_str(), scan.port), handler),
        )
        .await
        .map_err(|_| "The SSH connection timed out".to_owned())?
        .map_err(|error| match error {
            russh::Error::UnknownKey => {
                "The host key changed after you confirmed it. The panel stopped.".to_owned()
            }
            other => format!("Cannot connect over SSH: {other}"),
        })?;
        let user = input.user.clone();
        let authenticated = if let Some(key) = input
            .private_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
        {
            let key = russh::keys::decode_secret_key(
                key,
                input.passphrase.as_deref().filter(|text| !text.is_empty()),
            )
            .map_err(|error| format!("The private key cannot be read: {error}"))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| error.to_string())?
                .flatten();
            handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(|error| error.to_string())?
                .success()
        } else if let Some(password) = input.password.as_deref().filter(|text| !text.is_empty()) {
            handle
                .authenticate_password(user, password)
                .await
                .map_err(|error| error.to_string())?
                .success()
        } else {
            return Err("Enter a private key or a password".into());
        };
        if !authenticated {
            return Err("The SSH server refused the user name, key, or password".into());
        }
        let sudo = if input.user == "root" {
            None
        } else {
            Some(Zeroizing::new(
                input
                    .sudo_password
                    .clone()
                    .or_else(|| input.password.clone())
                    .unwrap_or_default(),
            ))
        };
        Ok(Self {
            handle,
            sudo,
            root: input.user == "root",
        })
    }

    pub async fn run(&self, command: &str, stdin: Option<&[u8]>) -> Result<Output, String> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|error| format!("Cannot open an SSH channel: {error}"))?;
        channel
            .exec(true, command)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(data) = stdin {
            channel
                .data(data)
                .await
                .map_err(|error| error.to_string())?;
        }
        channel.eof().await.map_err(|error| error.to_string())?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut status = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } if stdout.len() < 1 << 20 => {
                    stdout.extend_from_slice(&data)
                }
                ChannelMsg::ExtendedData { data, .. } if stderr.len() < 1 << 20 => {
                    stderr.extend_from_slice(&data)
                }
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                _ => {}
            }
        }
        Ok(Output {
            status: status.unwrap_or(255),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    pub async fn upload(&self, path: &str, file: &std::path::Path) -> Result<(), String> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|error| format!("Cannot open an SSH channel: {error}"))?;
        channel
            .exec(true, format!("umask 077 && cat > {path}"))
            .await
            .map_err(|error| error.to_string())?;
        let mut source = tokio::fs::File::open(file)
            .await
            .map_err(|error| error.to_string())?;
        let mut buffer = vec![0u8; 32 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            channel
                .data(&buffer[..count])
                .await
                .map_err(|error| error.to_string())?;
        }
        channel.eof().await.map_err(|error| error.to_string())?;
        let mut status = None;
        while let Some(message) = channel.wait().await {
            if let ChannelMsg::ExitStatus { exit_status } = message {
                status = Some(exit_status);
            }
        }
        if status != Some(0) {
            return Err("The bundle upload failed".into());
        }
        Ok(())
    }

    pub async fn prepare_sudo(&mut self) -> Result<(), String> {
        if self.root {
            return Ok(());
        }
        if self.run("sudo -n true", None).await?.status == 0 {
            self.sudo = None;
            self.root = false;
            return Ok(());
        }
        let (check, stdin) = self.privileged("true");
        if self.run(&check, Some(&stdin)).await?.status != 0 {
            return Err("The user cannot run commands as root. Check the sudo password.".into());
        }
        Ok(())
    }

    pub fn privileged(&self, command: &str) -> (String, Vec<u8>) {
        match &self.sudo {
            None if self.root => (command.to_owned(), Vec::new()),
            None => (format!("sudo -n {command}"), Vec::new()),
            Some(password) => {
                let mut input = password.as_bytes().to_vec();
                input.push(b'\n');
                (format!("sudo -S -p '' {command}"), input)
            }
        }
    }

    pub async fn close(self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }
}

pub const STEPS: &[&str] = &[
    "Connect and check the host key",
    "Check the server",
    "Upload the bundle",
    "Install the node",
    "Close SSH",
    "Connect to the node",
    "Send settings and probe the origin",
];

fn short(output: &Output) -> String {
    let text = if output.stderr.trim().is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    let text = text.trim();
    let tail: String = text
        .chars()
        .rev()
        .take(600)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("exit {}: {}", output.status, tail)
}

pub async fn install(
    app: App,
    job: String,
    scan: Scan,
    input: SshInstall,
    bundle: std::path::PathBuf,
    arch_expected: String,
) {
    let progress = Progress::new(&app, &job);
    let result = install_inner(&app, &progress, &scan, &input, &bundle, &arch_expected).await;
    progress.finish(result.err());
}

async fn install_inner(
    app: &App,
    progress: &Progress,
    scan: &Scan,
    input: &SshInstall,
    bundle: &std::path::Path,
    arch_expected: &str,
) -> Result<(), String> {
    let mut session = progress.run(0, Session::connect(scan, input)).await?;
    let remote = progress
        .run(1, async {
            let arch = session.run("uname -m", None).await?;
            let arch = match arch.stdout.trim() {
                "x86_64" => "amd64",
                "aarch64" | "arm64" => "arm64",
                other => return Err(format!("Unsupported architecture: {other}")),
            };
            if arch != arch_expected {
                return Err(format!("The server is {arch}, but the bundle is {arch_expected}. Upload a bundle for {arch}."));
            }
            let systemd = session.run("command -v systemctl >/dev/null && test -d /run/systemd/system", None).await?;
            if systemd.status != 0 {
                return Err("The server does not run systemd".into());
            }
            let port = session
                .run(&format!("ss -H -ltn 'sport = :{}'", input.node_port), None)
                .await?;
            if port.status != 0 {
                return Err("Cannot check ports: the ss command failed".into());
            }
            if !port.stdout.trim().is_empty() {
                return Err(format!("Port {} is in use on the server", input.node_port));
            }
            let disk = session.run("df -Pk /opt /var/lib /tmp | awk 'NR>1 {print $4}' | sort -n | head -1", None).await?;
            let free: u64 = disk.stdout.trim().parse().unwrap_or(0);
            if free < 200 * 1024 {
                return Err("The server needs at least 200 MiB of free disk space".into());
            }
            session.prepare_sudo().await?;
            let folder = session.run("mktemp -d /tmp/fiend-install.XXXXXXXX", None).await?;
            let folder = folder.stdout.trim().to_owned();
            if !folder.starts_with("/tmp/fiend-install.") || !folder.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte)) {
                return Err("Cannot make a work folder on the server".into());
            }
            Ok(folder)
        })
        .await;
    let remote = match remote {
        Ok(folder) => folder,
        Err(error) => {
            session.close().await;
            return Err(error);
        }
    };
    let control = Zeroizing::new(crate::state::new_secret());
    let installed = async {
        progress
            .run(2, async {
                session
                    .upload(&format!("{remote}/bundle.tar.gz"), bundle)
                    .await?;
                let unpack = session
                    .run(
                        &format!("tar -xzf {remote}/bundle.tar.gz -C {remote}"),
                        None,
                    )
                    .await?;
                if unpack.status != 0 {
                    return Err(format!("Cannot unpack the bundle: {}", short(&unpack)));
                }
                Ok(())
            })
            .await?;
        progress
            .run(3, async {
                let (command, mut stdin) = session.privileged(&format!(
                    "sh {remote}/install.sh node --control-secret-stdin --port {}",
                    input.node_port
                ));
                stdin.extend_from_slice(control.as_bytes());
                stdin.push(b'\n');
                let output = session.run(&command, Some(&stdin)).await?;
                if output.status != 0 {
                    return Err(format!("The installer failed: {}", short(&output)));
                }
                Ok(())
            })
            .await
    }
    .await;
    let _ = session.run(&format!("rm -rf -- {remote}"), None).await;
    progress.step(4, "running", None);
    session.close().await;
    progress.step(
        4,
        "done",
        Some("The panel does not keep the SSH credentials.".into()),
    );
    installed?;
    let node_id: i64 = sqlx::query_scalar("INSERT INTO nodes (name, address, port, local, control_secret, service_secret, ssh_host_key, disguise_id, status, created_at) VALUES (?, ?, ?, 0, ?, ?, ?, ?, 'unknown', ?) RETURNING id")
        .bind(&input.name)
        .bind(&input.address)
        .bind(i64::from(input.node_port))
        .bind(control.as_str())
        .bind(crate::state::new_secret())
        .bind(&scan.key)
        .bind(input.disguise_id)
        .bind(now())
        .fetch_one(&app.db)
        .await
        .map_err(|error| format!("Cannot save the node: {error}"))?;
    progress.node(node_id);
    app.audit(
        None,
        "node installed",
        &format!("node {}", input.name),
        &format!("{} ssh {}", input.address, scan.fingerprint),
    )
    .await;
    app.emit("nodes", Some(node_id.to_string()));
    progress
        .run(5, async {
            let mut last = String::new();
            for _ in 0..20 {
                match crate::sync::call(app, node_id, fiend_core::node::control::Call::Health).await
                {
                    Ok(_) => return Ok(()),
                    Err(error) => last = error.to_string(),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(format!("The node does not answer: {last}"))
        })
        .await?;
    app.sync.ensure(app, node_id);
    progress
        .run(6, async {
            crate::db::bump_version(&app.db)
                .await
                .map_err(|error| error.to_string())?;
            app.sync.wake(node_id);
            if let Some(disguise) = input.disguise_id {
                crate::api::probe_disguise(app, disguise, node_id)
                    .await
                    .map_err(|error| error.message)?;
            }
            for _ in 0..15 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let node = crate::state::node(app, node_id)
                    .await
                    .map_err(|error| error.message)?;
                if node.status == "online"
                    || (node.status == "setup" && input.disguise_id.is_none())
                {
                    return Ok(());
                }
            }
            Err("The node did not report the new settings".into())
        })
        .await
}
