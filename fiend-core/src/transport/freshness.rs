use std::collections::hash_map::RandomState;
use std::collections::{HashSet, VecDeque};
use std::hash::BuildHasher;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SHARDS: usize = 64;
pub const DEFAULT_MAX_ENTRIES: usize = 1 << 20;

#[derive(Default)]
struct Shard {
    seen: HashSet<[u8; 32]>,
    order: VecDeque<([u8; 32], Instant)>,
}

impl Shard {
    fn expire(&mut self, now: Instant, ttl: Duration) {
        while let Some((random, seen_at)) = self.order.front() {
            if now.duration_since(*seen_at) <= ttl {
                break;
            }
            self.seen.remove(random);
            self.order.pop_front();
        }
    }

    fn evict_oldest(&mut self) {
        if let Some((random, _)) = self.order.pop_front() {
            self.seen.remove(&random);
        }
    }
}

pub struct FreshnessCheck {
    ttl: Duration,
    shard_limit: usize,
    shards: Vec<Mutex<Shard>>,
    salt: RandomState,
    started: Instant,
    last_purge: AtomicU64,
    purge_interval: Duration,
}

impl FreshnessCheck {
    pub fn new(ttl: Duration) -> Self {
        Self::with_limit(ttl, DEFAULT_MAX_ENTRIES)
    }

    pub fn with_limit(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            shard_limit: max_entries.div_ceil(SHARDS).max(1),
            shards: (0..SHARDS).map(|_| Mutex::new(Shard::default())).collect(),
            salt: RandomState::new(),
            started: Instant::now(),
            last_purge: AtomicU64::new(0),
            purge_interval: (ttl / 4).clamp(Duration::from_secs(1), Duration::from_secs(60)),
        }
    }

    pub fn check(&self, random: &[u8; 32]) -> bool {
        self.maybe_purge();

        let shard = &self.shards[(self.salt.hash_one(random) % SHARDS as u64) as usize];
        let mut shard = shard.lock().unwrap_or_else(|error| error.into_inner());
        if shard.seen.contains(random) {
            return false;
        }
        let now = Instant::now();
        if shard.order.len() >= self.shard_limit {
            shard.expire(now, self.ttl);
        }
        while shard.order.len() >= self.shard_limit {
            shard.evict_oldest();
        }
        shard.seen.insert(*random);
        shard.order.push_back((*random, now));
        true
    }

    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| {
                shard
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .order
                    .len()
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
            shard
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .expire(now, self.ttl);
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
        std::thread::sleep(Duration::from_millis(2));
        guard.purge();
        assert!(guard.is_empty());
        assert!(guard.check(&random));
    }

    #[test]
    fn size_is_capped_by_evicting_the_oldest_entry() {
        let guard = FreshnessCheck::with_limit(Duration::from_secs(3600), SHARDS * 2);
        let random = |index: u16| {
            let mut random = [0u8; 32];
            random[..2].copy_from_slice(&index.to_be_bytes());
            random
        };
        let shard = |index: u16| guard.salt.hash_one(random(index)) % SHARDS as u64;
        let same: Vec<u16> = (0..u16::MAX)
            .filter(|index| shard(*index) == shard(0))
            .take(10)
            .collect();
        for index in &same {
            assert!(guard.check(&random(*index)));
        }
        assert_eq!(guard.len(), 2);
        assert!(!guard.check(&random(same[9])));
        assert!(!guard.check(&random(same[8])));
        assert!(
            guard.check(&random(same[0])),
            "the oldest entry was evicted"
        );
    }

    #[test]
    fn a_shared_first_byte_does_not_pick_one_shard() {
        let guard = FreshnessCheck::with_limit(Duration::from_secs(3600), SHARDS * 4);
        for index in 0..=255u8 {
            let mut random = [0u8; 32];
            random[1] = index;
            assert!(guard.check(&random));
        }
        assert!(guard.len() > 4 * 4, "entries spread over several shards");
    }
}
