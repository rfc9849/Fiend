use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::types::BundleView;
use crate::{ApiError, ApiResult, App, now};

pub const MAX_BUNDLE: usize = 128 * 1024 * 1024;
const REQUIRED: &[&str] = &[
    "fiend-core",
    "fiend-panel",
    "wireproxy",
    "install.sh",
    "rollback.sh",
    "activate-update.sh",
    "node-entry.sh",
    "systemd/fiend-node.service",
    "systemd/fiend-panel.service",
    "systemd/fiend-node-rollback.service",
];

pub struct Parsed {
    pub version: String,
    pub arch: String,
    pub core_sha256: String,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub fn elf_arch(path: &Path) -> io::Result<String> {
    let mut header = [0u8; 20];
    fs::File::open(path)?.read_exact(&mut header)?;
    if &header[..4] != b"\x7fELF" {
        return Err(invalid("fiend-core is not a Linux binary"));
    }
    match u16::from_le_bytes([header[18], header[19]]) {
        62 => Ok("amd64".into()),
        183 => Ok("arm64".into()),
        _ => Err(invalid("fiend-core has an unsupported architecture")),
    }
}

pub fn unpack(archive: &Path, into: &Path) -> io::Result<Parsed> {
    fs::create_dir_all(into)?;
    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file).take(4 * MAX_BUNDLE as u64));
    let mut total = 0u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
        {
            return Err(invalid("the bundle has an unsafe path"));
        }
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(invalid("the bundle has a link or a special file"));
        }
        total = total.saturating_add(entry.size());
        if total > 4 * MAX_BUNDLE as u64 {
            return Err(invalid("the bundle is too large"));
        }
        let target = into.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::File::create(&target)?;
        io::copy(&mut entry, &mut output)?;
    }
    let sums = fs::read_to_string(into.join("SHA256SUMS"))
        .map_err(|_| invalid("the bundle has no SHA256SUMS"))?;
    for required in REQUIRED {
        let line = sums
            .lines()
            .find(|line| line.split_whitespace().nth(1) == Some(required))
            .ok_or_else(|| invalid(format!("SHA256SUMS has no line for {required}")))?;
        let expected = line.split_whitespace().next().unwrap_or("");
        let actual = sha256_file(&into.join(required))
            .map_err(|_| invalid(format!("the bundle has no {required}")))?;
        if expected != actual {
            return Err(invalid(format!("checksum mismatch for {required}")));
        }
    }
    let arch = elf_arch(&into.join("fiend-core"))?;
    let mut version = "unknown".to_owned();
    if let Ok(text) = fs::read_to_string(into.join("VERSION")) {
        for line in text.lines() {
            match line.split_once('=') {
                Some(("version", value)) => version = value.trim().to_owned(),
                Some(("arch", value)) if value.trim() != arch => {
                    return Err(invalid("VERSION and fiend-core architectures differ"));
                }
                _ => {}
            }
        }
    }
    if version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+_".contains(&byte))
    {
        return Err(invalid("the bundle version is not valid"));
    }
    Ok(Parsed {
        version,
        arch,
        core_sha256: sha256_file(&into.join("fiend-core"))?,
    })
}

pub async fn import(app: &App, archive: PathBuf) -> ApiResult<BundleView> {
    let root = app.config.bundles.clone();
    let result =
        tokio::task::spawn_blocking(move || -> io::Result<(Parsed, String, PathBuf, i64)> {
            fs::create_dir_all(&root)?;
            let sha256 = sha256_file(&archive)?;
            let staging = root.join(format!(
                ".incoming-{}",
                hex::encode(rand::random::<[u8; 8]>())
            ));
            let parsed = match unpack(&archive, &staging.join("files")) {
                Ok(parsed) => parsed,
                Err(error) => {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(error);
                }
            };
            let size = fs::metadata(&archive)?.len() as i64;
            fs::copy(&archive, staging.join("bundle.tar.gz"))?;
            let target = root.join(format!(
                "{}-{}-{}",
                parsed.version,
                parsed.arch,
                &sha256[..12]
            ));
            if target.exists() {
                fs::remove_dir_all(&target)?;
            }
            fs::rename(&staging, &target)?;
            Ok((parsed, sha256, target, size))
        })
        .await
        .map_err(ApiError::internal)?;
    let (parsed, sha256, target, size) =
        result.map_err(|error| ApiError::bad(format!("The bundle is not valid: {error}")))?;
    let id: i64 = sqlx::query_scalar("INSERT INTO bundles (version, arch, sha256, core_sha256, path, uploaded_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(version, arch, sha256) DO UPDATE SET uploaded_at = excluded.uploaded_at, path = excluded.path RETURNING id")
        .bind(&parsed.version)
        .bind(&parsed.arch)
        .bind(&sha256)
        .bind(&parsed.core_sha256)
        .bind(target.to_string_lossy().to_string())
        .bind(now())
        .fetch_one(&app.db)
        .await?;
    let older: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, path FROM bundles WHERE version = ? AND arch = ? AND id != ?",
    )
    .bind(&parsed.version)
    .bind(&parsed.arch)
    .bind(id)
    .fetch_all(&app.db)
    .await?;
    for (old, path) in older {
        sqlx::query("DELETE FROM bundles WHERE id = ?")
            .bind(old)
            .execute(&app.db)
            .await?;
        let path = std::path::PathBuf::from(path);
        if path.starts_with(&app.config.bundles) && path != target {
            let _ = fs::remove_dir_all(&path);
        }
    }
    Ok(BundleView {
        id,
        version: parsed.version,
        arch: parsed.arch,
        sha256,
        core_sha256: parsed.core_sha256,
        uploaded_at: now(),
        size,
    })
}

pub async fn import_inbox(app: &App) {
    let inbox = app.config.bundles.join("inbox");
    let Ok(entries) = fs::read_dir(&inbox) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("gz") {
            continue;
        }
        match import(app, path.clone()).await {
            Ok(bundle) => {
                tracing::info!(version = %bundle.version, arch = %bundle.arch, "bundle imported")
            }
            Err(error) => tracing::warn!(error = %error.message, "cannot import bundle"),
        }
        let _ = fs::remove_file(&path);
    }
}

pub async fn find(app: &App, id: i64) -> ApiResult<(String, String, PathBuf, String)> {
    let row: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT version, arch, path, core_sha256 FROM bundles WHERE id = ?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?;
    let (version, arch, path, core) = row.ok_or_else(ApiError::not_found)?;
    Ok((version, arch, PathBuf::from(path), core))
}

pub async fn latest_for(app: &App, arch: &str) -> ApiResult<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM bundles WHERE arch = ? ORDER BY uploaded_at DESC, id DESC LIMIT 1",
    )
    .bind(arch)
    .fetch_optional(&app.db)
    .await?)
}
