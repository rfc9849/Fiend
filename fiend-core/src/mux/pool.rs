use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval, timeout, timeout_at};

use super::config::Config;
use super::error::{Error, Result};
use super::lane::{H2Lane, LaneKind};
use super::link::{ClientLink, LOAD_INTERVAL, LaneReservation};
use crate::transport::{self, Session};

pub type DialFuture<S> =
    Pin<Box<dyn Future<Output = std::result::Result<Session<S>, transport::Error>> + Send>>;
pub type DialFn<S> = Arc<dyn Fn() -> DialFuture<S> + Send + Sync>;
type LinkFuture = Pin<Box<dyn Future<Output = Result<ClientLink>> + Send>>;

const GROWTH_COOLDOWN: Duration = Duration::from_secs(5);
const SPREAD_WAIT: Duration = Duration::from_secs(1);
const SPREAD_LANES: usize = 2;

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;

pub struct LinkPool<S> {
    inner: Arc<PoolInner<S>>,
}

struct PoolInner<S> {
    config: Config,
    dial: DialFn<S>,
    state: Mutex<State>,
    worker: Mutex<Option<JoinHandle<()>>>,
    wake: Arc<Notify>,
    ready: Notify,
    shutdown: Arc<Notify>,
    waiters: AtomicUsize,
    wait_us: AtomicU64,
    closed: AtomicBool,
}

struct State {
    links: Vec<Arc<ClientLink>>,
    generation: u64,
    backoff: Backoff,
    next_id: u64,
    dialing: bool,
    offline: bool,
    dial_attempts: u64,
    dial_failures: u64,
    idle_closures: u64,
    failed_closures: u64,
    pressure_watermark: usize,
    speculative_attempts: usize,
    spread_request: bool,
    last_growth: Option<Instant>,
    quiet_since: Option<Instant>,
}

struct Backoff {
    delay: Duration,
    next: Instant,
}

impl Backoff {
    fn new() -> Self {
        Self {
            delay: Duration::ZERO,
            next: Instant::now(),
        }
    }

    fn fail(&mut self, config: &Config) {
        self.delay = if self.delay.is_zero() {
            config.backoff_floor
        } else {
            self.delay.saturating_mul(2).min(config.backoff_ceiling)
        };
        self.next = Instant::now() + self.delay;
    }

    fn reset(&mut self) {
        self.delay = Duration::ZERO;
        self.next = Instant::now();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolStats {
    pub links: usize,
    pub bulk_links: usize,
    pub lanes: usize,
    pub pending_lanes: usize,
    pub blocked_writers: usize,
    pub waiting_opens: usize,
    pub dialing: bool,
    pub offline: bool,
    pub dial_attempts: u64,
    pub dial_failures: u64,
    pub idle_closures: u64,
    pub failed_closures: u64,
    pub link_wait_us: u64,
    pub link_ids: Vec<u64>,
}

impl<S> LinkPool<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    pub fn new<F, Fut>(config: Config, dial: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Session<S>, transport::Error>> + Send + 'static,
    {
        let dial: DialFn<S> = Arc::new(move || Box::pin(dial()));
        Self {
            inner: Arc::new(PoolInner {
                config,
                dial,
                state: Mutex::new(State {
                    links: Vec::new(),
                    generation: 0,
                    backoff: Backoff::new(),
                    next_id: 1,
                    dialing: false,
                    offline: false,
                    dial_attempts: 0,
                    dial_failures: 0,
                    idle_closures: 0,
                    failed_closures: 0,
                    pressure_watermark: 0,
                    speculative_attempts: 0,
                    spread_request: false,
                    last_growth: None,
                    quiet_since: None,
                }),
                worker: Mutex::new(None),
                wake: Arc::new(Notify::new()),
                ready: Notify::new(),
                shutdown: Arc::new(Notify::new()),
                waiters: AtomicUsize::new(0),
                wait_us: AtomicU64::new(0),
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub async fn open_lane(&self, authority: &str, kind: LaneKind) -> Result<H2Lane> {
        let shutdown = self.inner.shutdown.notified();
        tokio::pin!(shutdown);
        shutdown.as_mut().enable();
        if self.inner.closed() {
            return Err(Error::Closed);
        }
        self.inner.start_worker();
        let deadline = Instant::now() + self.inner.config.open_timeout;
        tokio::select! {
            biased;
            _ = shutdown => Err(Error::Closed),
            result = timeout_at(deadline, self.open_until(authority, kind, deadline)) => {
                result.unwrap_or(Err(Error::Timeout))
            }
        }
    }

    async fn open_until(
        &self,
        authority: &str,
        kind: LaneKind,
        deadline: Instant,
    ) -> Result<H2Lane> {
        let mut last = Error::Unavailable;
        for _ in 0..self.inner.config.attempts {
            let reservation = self.inner.acquire(deadline).await?;
            let link = reservation.link.clone();
            match link
                .open_lane(authority, kind, &self.inner.config, reservation)
                .await
            {
                Ok(lane) => {
                    if link.first_use() {
                        self.inner.state().backoff.reset();
                    }
                    return Ok(lane);
                }
                Err(Error::Rejected(status)) => {
                    if link.first_use() {
                        self.inner.state().backoff.reset();
                    }
                    return Err(Error::Rejected(status));
                }
                Err(error) if error.closes_link() => {
                    self.inner.discard(&link);
                    last = error;
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    pub async fn ready(&self) -> Result<()> {
        self.inner.start_worker();
        let deadline = Instant::now() + self.inner.config.open_timeout;
        self.inner.acquire(deadline).await.map(drop)
    }

    pub fn stats(&self) -> PoolStats {
        self.inner.stats()
    }

    pub fn rtt(&self) -> Option<Duration> {
        self.inner.rtt()
    }

    pub fn is_offline(&self) -> bool {
        self.inner.stats().offline
    }

    pub fn close(&self) {
        self.inner.close();
    }

    pub fn network_changed(&self) {
        {
            let mut worker = self.inner.worker.lock().unwrap_or_else(|e| e.into_inner());
            let mut state = self.inner.state();
            state.generation = state.generation.wrapping_add(1);
            if let Some(worker) = worker.take() {
                worker.abort();
            }
            for link in state.links.drain(..) {
                link.close();
            }
            state.dialing = false;
            state.backoff.reset();
            state.pressure_watermark = 0;
            state.speculative_attempts = 0;
            state.spread_request = false;
            state.last_growth = None;
            state.quiet_since = None;
        }
        self.inner.start_worker();
        self.inner.wake.notify_one();
        self.inner.ready.notify_waiters();
    }
}

impl<S> Drop for LinkPool<S> {
    fn drop(&mut self) {
        self.inner.close();
    }
}

impl<S> PoolInner<S> {
    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.shutdown.notify_waiters();
        self.ready.notify_waiters();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            worker.abort();
        }
        let mut state = self.state();
        state.dialing = false;
        for link in state.links.drain(..) {
            link.close();
        }
    }

    fn discard(&self, link: &Arc<ClientLink>) {
        let mut state = self.state();
        link.close();
        let before = state.links.len();
        state
            .links
            .retain(|candidate| !Arc::ptr_eq(candidate, link));
        state.failed_closures += (before - state.links.len()) as u64;
        if before != state.links.len() {
            state.backoff.fail(&self.config);
        }
        self.wake.notify_one();
        self.ready.notify_waiters();
    }

    fn rtt(&self) -> Option<Duration> {
        let max_age = self.config.rtt_max_age();
        self.state()
            .links
            .iter()
            .filter(|link| !link.is_closed())
            .filter_map(|link| link.shared().rtt())
            .filter(|(_, age)| *age <= max_age)
            .min_by_key(|(_, age)| *age)
            .map(|(rtt, _)| rtt)
    }

    fn stats(&self) -> PoolStats {
        let state = self.state();
        let links: Vec<_> = state
            .links
            .iter()
            .filter(|link| !link.is_closed())
            .collect();
        PoolStats {
            links: links.len(),
            bulk_links: links.iter().filter(|link| link.is_bulk()).count(),
            lanes: links.iter().map(|link| link.shared().work().lanes).sum(),
            pending_lanes: links.iter().map(|link| link.shared().work().pending).sum(),
            blocked_writers: links
                .iter()
                .map(|link| link.shared().blocked_writers())
                .sum(),
            waiting_opens: self.waiters.load(Ordering::Acquire),
            dialing: state.dialing,
            offline: state.offline && links.is_empty(),
            dial_attempts: state.dial_attempts,
            dial_failures: state.dial_failures,
            idle_closures: state.idle_closures,
            failed_closures: state.failed_closures,
            link_wait_us: self.wait_us.load(Ordering::Relaxed),
            link_ids: links.iter().map(|link| link.id()).collect(),
        }
    }
}

struct Demand<'a, S> {
    pool: &'a PoolInner<S>,
    started: Instant,
}

impl<S> Drop for Demand<'_, S> {
    fn drop(&mut self) {
        self.pool.waiters.fetch_sub(1, Ordering::AcqRel);
        self.pool.wait_us.fetch_add(
            self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
        self.pool.wake.notify_one();
    }
}

impl<S> PoolInner<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn start_worker(self: &Arc<Self>) {
        let mut worker = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if worker.is_none() && !self.closed() {
            *worker = Some(tokio::spawn(Self::run(
                Arc::downgrade(self),
                self.wake.clone(),
                self.state().generation,
            )));
        }
    }

    async fn acquire(&self, deadline: Instant) -> Result<LaneReservation> {
        self.waiters.fetch_add(1, Ordering::AcqRel);
        let _demand = Demand {
            pool: self,
            started: Instant::now(),
        };
        let failures = {
            let state = self.state();
            state.dial_failures + state.failed_closures
        };
        self.wake.notify_one();
        let mut spread_until: Option<Instant> = None;
        loop {
            let ready = self.ready.notified();
            tokio::pin!(ready);
            ready.as_mut().enable();
            {
                let mut state = self.state();
                if self.closed() {
                    return Err(Error::Closed);
                }
                let now = Instant::now();
                let loaded = !state.links.is_empty()
                    && state.links.len() < self.config.max_links
                    && state.links.iter().all(|link| {
                        link.is_closed() || link.shared().work().total() >= SPREAD_LANES
                    });
                let spread = loaded
                    && state.backoff.delay.is_zero()
                    && spread_until.is_none_or(|until| now < until);
                if spread {
                    state.spread_request = true;
                    spread_until.get_or_insert(now + SPREAD_WAIT);
                    self.wake.notify_one();
                } else if let Some(link) = state
                    .links
                    .iter()
                    .filter(|link| !link.is_closed() && link.has_capacity())
                    .min_by_key(|link| (link.is_bulk(), link.shared().work().total()))
                {
                    return Ok(link.reserve());
                }
                if (state.dial_failures + state.failed_closures).saturating_sub(failures)
                    >= self.config.attempts as u64
                {
                    return Err(Error::Unavailable);
                }
            }
            timeout_at(deadline, ready)
                .await
                .map_err(|_| Error::Timeout)?;
        }
    }

    #[cfg(test)]
    fn maintain(&self) -> Option<LinkFuture> {
        let generation = self.state().generation;
        self.maintain_generation(generation)
    }

    fn maintain_generation(&self, generation: u64) -> Option<LinkFuture> {
        let mut state = self.state();
        if self.closed() || state.generation != generation {
            return None;
        }
        let before = state.links.len();
        state.links.retain(|link| !link.is_closed());
        state.failed_closures += (before - state.links.len()) as u64;
        if before != state.links.len() {
            state.backoff.fail(&self.config);
        }
        for link in &state.links {
            link.shared().sample_load(self.config.bulk_lane_bytes);
        }
        let mut index = 0;
        while state.links.len() > 1 && index < state.links.len() {
            if state.links[index]
                .shared()
                .idle_for_work()
                .is_some_and(|idle| idle >= self.config.surplus_idle)
            {
                let link = state.links.remove(index);
                link.close();
                state.idle_closures += 1;
                tracing::debug!(link_id = link.id(), reason = "idle", "mux link closed");
            } else {
                index += 1;
            }
        }
        let now = Instant::now();
        let pressure = state.links.iter().filter(|link| link.is_bulk()).count();
        if pressure == 0 {
            let quiet = *state.quiet_since.get_or_insert(now);
            if now.duration_since(quiet) >= GROWTH_COOLDOWN {
                state.pressure_watermark = 0;
                state.speculative_attempts = 0;
            }
        } else {
            state.quiet_since = None;
        }
        let capacity = state.links.iter().any(|link| link.has_capacity());
        let waiters = self.waiters.load(Ordering::Acquire);
        let required = waiters > 0 && !capacity;
        let spread = state.spread_request && state.links.len() < self.config.max_links;
        let speculative = pressure > state.pressure_watermark
            && state
                .links
                .iter()
                .all(|link| link.is_bulk() || !link.has_capacity())
            && state.speculative_attempts < self.config.attempts
            && state
                .last_growth
                .is_none_or(|last| last.elapsed() >= GROWTH_COOLDOWN);
        if state.dialing
            || state.links.len() >= self.config.max_links
            || now < state.backoff.next
            || !(required || speculative || spread)
        {
            return None;
        }
        state.dialing = true;
        state.dial_attempts += 1;
        state.spread_request = false;
        if !required && !spread {
            state.speculative_attempts += 1;
        }
        let id = state.next_id;
        state.next_id += 1;
        let dial = self.dial.clone();
        let config = self.config.clone();
        let wake = self.wake.clone();
        tracing::debug!(link_id = id, required, spread, "mux link dial");
        Some(Box::pin(async move {
            timeout(config.open_timeout, async {
                let session = dial().await?;
                ClientLink::connect(session, &config, id, wake).await
            })
            .await
            .map_err(|_| Error::Timeout)?
        }))
    }

    #[cfg(test)]
    fn dial_finished(&self, result: Result<ClientLink>) {
        let generation = self.state().generation;
        self.dial_finished_generation(generation, result);
    }

    fn dial_finished_generation(&self, generation: u64, result: Result<ClientLink>) {
        let mut state = self.state();
        if state.generation != generation {
            if let Ok(link) = result {
                link.close();
            }
            return;
        }
        state.dialing = false;
        match result {
            Ok(link) if !self.closed() => {
                if !state.links.is_empty() {
                    state.last_growth = Some(Instant::now());
                    state.pressure_watermark =
                        state.links.iter().filter(|link| link.is_bulk()).count();
                }
                state.offline = false;
                tracing::debug!(link_id = link.id(), "mux link ready");
                state.links.push(Arc::new(link));
            }
            Ok(link) => link.close(),
            Err(error) => {
                state.backoff.fail(&self.config);
                state.dial_failures += 1;
                state.offline = true;
                tracing::debug!(%error, "mux link dial failed");
            }
        }
        self.ready.notify_waiters();
    }

    async fn run(weak: Weak<Self>, wake: Arc<Notify>, generation: u64) {
        let mut tick = interval(LOAD_INTERVAL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut dial: Option<LinkFuture> = None;
        loop {
            let Some(pool) = weak.upgrade() else { return };
            if pool.closed() || pool.state().generation != generation {
                return;
            }
            if let Some(next) = pool.maintain_generation(generation) {
                dial = Some(next);
            }
            pool.ready.notify_waiters();
            drop(pool);
            tokio::select! {
                result = async {
                    match &mut dial {
                        Some(future) => future.await,
                        None => pending().await,
                    }
                } => {
                    dial = None;
                    if let Some(pool) = weak.upgrade() {
                        pool.dial_finished_generation(generation, result);
                    }
                }
                _ = tick.tick() => {}
                _ = wake.notified() => {}
            }
        }
    }
}
