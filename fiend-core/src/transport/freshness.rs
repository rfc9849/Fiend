use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SHARDS: usize = 64;

pub struct FreshnessCheck {
    ttl: Duration,
    shards: Vec<Mutex<HashMap<[u8; 32], Instant>>>,
    started: Instant,
    last_purge: AtomicU64,
    purge_interval: Duration,
}

impl FreshnessCheck {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            started: Instant::now(),
            last_purge: AtomicU64::new(0),
            purge_interval: (ttl / 4).clamp(Duration::from_secs(1), Duration::from_secs(60)),
        }
    }

    pub fn check(&self, random: &[u8; 32]) -> bool {
        self.maybe_purge();

        let shard = &self.shards[random[0] as usize % SHARDS];
        let mut seen = shard.lock().expect("freshness shard lock");
        if seen.contains_key(random) {
            return false;
        }
        seen.insert(*random, Instant::now());
        true
    }

    fn maybe_purge(&self) {
        let elapsed = self.started.elapsed().as_millis() as u64;
        let last = self.last_purge.load(Ordering::Relaxed);
        if elapsed.saturating_sub(last) < self.purge_interval.as_millis() as u64 {
            return;
        }
        if self
            .last_purge
            .compare_exchange(last, elapsed, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.purge();
        }
    }

    pub fn purge(&self) {
        let now = Instant::now();
        for shard in &self.shards {
            let mut seen = shard.lock().expect("freshness shard lock");
            seen.retain(|_, seen_at| now.duration_since(*seen_at) <= self.ttl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_random_is_rejected() {
        let guard = FreshnessCheck::new(Duration::from_secs(3600));
        let random = [7u8; 32];
        assert!(guard.check(&random));
        assert!(!guard.check(&random));
        assert!(guard.check(&[8u8; 32]));
    }

    #[test]
    fn purge_releases_expired_entries() {
        let guard = FreshnessCheck::new(Duration::ZERO);
        let random = [3u8; 32];
        assert!(guard.check(&random));
        guard.purge();
        assert!(guard.check(&random));
    }
}
