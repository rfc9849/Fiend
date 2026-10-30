use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_BINARY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 128 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Begin {
    pub size: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub upload_id: String,
    pub offset: u64,
    pub data: String,
}

pub struct Manager {
    directory: PathBuf,
    config_path: PathBuf,
    state_path: PathBuf,
    current: Option<Upload>,
}

struct Upload {
    id: String,
    path: PathBuf,
    size: u64,
    received: u64,
    sha256: String,
    started: Instant,
}

impl Drop for Upload {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Manager {
    pub fn new(directory: PathBuf, config_path: PathBuf, state_path: PathBuf) -> Self {
        Self {
            directory,
            config_path,
            state_path,
            current: None,
        }
    }

    pub fn begin(&mut self, request: Begin) -> io::Result<String> {
        if request.size == 0
            || request.size > MAX_BINARY_BYTES
            || request.sha256.len() != 64
            || !request.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid("invalid update size or SHA-256"));
        }
        self.expire();
        if self.current.is_some() {
            return Err(invalid("a binary update is in progress"));
        }
        if self.directory.join("core.pending.sha256").exists() {
            return Err(invalid("a binary update is waiting for restart"));
        }
        let id = hex::encode(rand::random::<[u8; 16]>());
        let path = self.directory.join(format!("core.upload-{id}"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?;
        self.current = Some(Upload {
            id: id.clone(),
            path,
            size: request.size,
            received: 0,
            sha256: request.sha256.to_ascii_lowercase(),
            started: Instant::now(),
        });
        Ok(id)
    }

    fn expire(&mut self) {
        if self
            .current
            .as_ref()
            .is_some_and(|upload| upload.started.elapsed() > Duration::from_secs(600))
        {
            self.current = None;
        }
    }

    pub fn chunk(&mut self, request: Chunk) -> io::Result<u64> {
        self.expire();
        let upload = self
            .current
            .as_mut()
            .ok_or_else(|| invalid("no binary upload"))?;
        if request.upload_id != upload.id
            || request.offset != upload.received
            || request.data.len() > MAX_CHUNK_BYTES * 2
        {
            return Err(invalid("invalid binary chunk id, offset, or size"));
        }
        let data = hex::decode(request.data).map_err(|_| invalid("binary chunk must be hex"))?;
        if data.is_empty() || upload.received.saturating_add(data.len() as u64) > upload.size {
            return Err(invalid("binary chunk exceeds upload size"));
        }
        let mut file = OpenOptions::new().append(true).open(&upload.path)?;
        file.write_all(&data)?;
        upload.received += data.len() as u64;
        Ok(upload.received)
    }

    pub async fn finish(&mut self, id: &str) -> io::Result<()> {
        self.expire();
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| invalid("no binary upload"))?;
        if current.id != id || current.received != current.size {
            return Err(invalid("binary upload is incomplete or has a different id"));
        }
        let upload = self.current.take().expect("checked upload");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&upload.path)?;
        file.sync_all()?;
        drop(file);
        if sha256(&upload.path)? != upload.sha256 {
            return Err(invalid("binary SHA-256 does not match"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&upload.path, fs::Permissions::from_mode(0o700))?;
        }
        let check = tokio::process::Command::new(&upload.path)
            .arg("check")
            .arg(&self.config_path)
            .arg(&self.state_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status();
        let status = tokio::time::timeout(Duration::from_secs(10), check)
            .await
            .map_err(|_| invalid("binary self-check timed out"))??;
        if !status.success() {
            return Err(invalid("binary self-check failed"));
        }
        fs::rename(&upload.path, self.directory.join("core.pending"))?;
        let marker = self.directory.join("core.pending.sha256");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&marker)?;
        writeln!(file, "{}", upload.sha256)?;
        file.sync_all()?;
        fs::File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
