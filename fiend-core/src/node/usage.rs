use std::fs;
use std::io;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};

use super::{MAX_STATE_BYTES, Node, invalid, save_private};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedUsage {
    users: Vec<UserUsage>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserUsage {
    id: String,
    usage_epoch: u64,
    used_traffic: u64,
}

impl Node {
    pub(super) fn restore_usage(&self) -> io::Result<()> {
        let path = self.inner.state_path.with_extension("usage.json");
        let raw = match fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if raw.len() > MAX_STATE_BYTES {
            return Err(invalid("saved usage is too large"));
        }
        let saved: SavedUsage =
            serde_json::from_slice(&raw).map_err(|_| invalid("invalid saved node usage"))?;
        let snapshot = self.snapshot();
        for row in saved.users {
            if let Some(entry) = snapshot
                .users
                .iter()
                .find(|entry| entry.user.id == row.id && entry.user.usage_epoch == row.usage_epoch)
            {
                entry
                    .counters
                    .total
                    .fetch_max(row.used_traffic, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    pub async fn persist_usage(&self) -> io::Result<()> {
        let node = self.clone();
        tokio::spawn(async move {
            let _lock = node.inner.applying.lock().await;
            let users = node
                .snapshot()
                .users
                .iter()
                .map(|entry| UserUsage {
                    id: entry.user.id.clone(),
                    usage_epoch: entry.user.usage_epoch,
                    used_traffic: entry.counters.total.load(Ordering::Relaxed),
                })
                .collect();
            let bytes = serde_json::to_vec(&SavedUsage { users }).map_err(io::Error::other)?;
            let path = node.inner.state_path.with_extension("usage.json");
            tokio::task::spawn_blocking(move || save_private(&path, &bytes))
                .await
                .map_err(io::Error::other)?
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn maintain_usage(&self) {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            if let Err(error) = self.persist_usage().await {
                tracing::warn!(%error, "cannot save node usage");
            }
        }
    }
}
