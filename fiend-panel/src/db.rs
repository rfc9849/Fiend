use std::path::Path;
use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

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
        std::fs::rename(&restore, path)
            .map_err(|error| format!("cannot restore the database: {error}"))?;
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
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
    let current: i64 = setting(pool, "state_version")
        .await
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let next = current + 1;
    set_setting(pool, "state_version", &next.to_string()).await?;
    Ok(next)
}

pub async fn version(pool: &SqlitePool) -> i64 {
    setting(pool, "state_version")
        .await
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

pub async fn raise_version(pool: &SqlitePool, at_least: i64) -> Result<i64, sqlx::Error> {
    let current = version(pool).await;
    if current >= at_least {
        return Ok(current);
    }
    set_setting(pool, "state_version", &at_least.to_string()).await?;
    Ok(at_least)
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
