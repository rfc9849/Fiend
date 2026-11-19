use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Connection, SqliteConnection, SqlitePool};

use crate::{ApiError, ApiResult};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn open(path: &Path) -> Result<SqlitePool, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let restore = path.with_extension("db.restore");
    if restore.exists() {
        apply_restore(path, &restore)
            .await
            .map_err(|error| format!("cannot restore the database: {error}"))?;
    }
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(|error| error.to_string())?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(10));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .map_err(|error| format!("cannot open the panel database: {error}"))?;
    MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| format!("cannot update the panel database: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [
            path.to_path_buf(),
            path.with_extension("db-wal"),
            path.with_extension("db-shm"),
        ] {
            if file.exists() {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(pool)
}

pub(crate) async fn stage_restore(path: &Path, bytes: &[u8]) -> ApiResult<()> {
    let staging = temporary_path(path, "restore");
    let mut file = private_file(&staging).await.map_err(ApiError::internal)?;
    let result = async {
        use tokio::io::AsyncWriteExt;
        file.write_all(bytes).await.map_err(ApiError::internal)?;
        drop(file);
        if let Err(error) = prepare_restore(&staging).await {
            tracing::warn!(%error, "backup rejected");
            return Err(ApiError::bad(
                "This backup is damaged or incompatible with this panel version.",
            ));
        }
        sync_file(&staging).await.map_err(ApiError::internal)?;
        tokio::fs::hard_link(&staging, path.with_extension("db.restore"))
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    ApiError::conflict("A restore is already pending.")
                } else {
                    ApiError::internal(error)
                }
            })
    }
    .await;
    let _ = tokio::fs::remove_file(&staging).await;
    result
}

async fn apply_restore(
    path: &Path,
    restore: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    prepare_restore(restore).await?;
    sync_file(restore).await?;
    if path.try_exists()? {
        let previous = temporary_path(path, "previous");
        private_file(&previous).await?;
        let result = async {
            let mut connection = restore_connection(path).await?;
            sqlx::query("VACUUM INTO ?")
                .bind(previous.to_string_lossy().as_ref())
                .execute(&mut connection)
                .await?;
            connection.close().await?;
            sync_file(&previous).await?;
            tokio::fs::rename(&previous, path.with_extension("db.previous")).await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&previous).await;
        }
        result?;
    }
    tokio::fs::rename(restore, path).await?;
    Ok(())
}

async fn prepare_restore(path: &Path) -> Result<(), sqlx::Error> {
    let mut connection = restore_connection(path).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
        .fetch_one(&mut connection)
        .await?;
    let migrations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&mut connection)
        .await?;
    if integrity != "ok" || admins == 0 || migrations == 0 {
        return Err(sqlx::Error::Protocol("invalid panel database".into()));
    }
    MIGRATOR.run_direct(None, &mut connection, false).await?;
    if sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut connection)
        .await?
        .is_some()
    {
        return Err(sqlx::Error::Protocol(
            "invalid panel database references".into(),
        ));
    }
    sqlx::query("DELETE FROM sessions")
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    Ok(())
}

async fn restore_connection(path: &Path) -> Result<SqliteConnection, sqlx::Error> {
    SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(SqliteJournalMode::Delete)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(10)),
    )
    .await
}

fn temporary_path(path: &Path, purpose: &str) -> std::path::PathBuf {
    path.with_extension(format!(
        "{purpose}-{}",
        hex::encode(rand::random::<[u8; 16]>())
    ))
}

async fn private_file(path: &Path) -> std::io::Result<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}

async fn sync_file(path: &Path) -> std::io::Result<()> {
    tokio::fs::File::open(path).await?.sync_all().await
}

pub async fn setting(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map(drop)
}

pub async fn bump_version(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(BUMP_VERSION).fetch_one(pool).await
}

const BUMP_VERSION: &str = "INSERT INTO settings (key, value) VALUES ('state_version', '2')
    ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1
    RETURNING CAST(value AS INTEGER)";

pub async fn bump_version_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(BUMP_VERSION)
        .fetch_one(&mut **transaction)
        .await
}

pub async fn version(pool: &SqlitePool) -> i64 {
    setting(pool, "state_version")
        .await
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

pub async fn raise_version(pool: &SqlitePool, at_least: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO settings (key, value) VALUES ('state_version', ?)
         ON CONFLICT(key) DO UPDATE SET value = MAX(CAST(value AS INTEGER), CAST(excluded.value AS INTEGER))
         RETURNING CAST(value AS INTEGER)",
    )
    .bind(at_least.max(1))
    .fetch_one(pool)
    .await
}

pub async fn set_setting_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(&mut **transaction)
        .await
        .map(drop)
}
