use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, timeout};

use super::config::Config;
use super::error::{Error, Result};
use super::lane::{H2Lane, LaneKind};
use super::link::ClientLink;
use crate::transport::{self, Session};

pub type DialFuture<S> =
    Pin<Box<dyn Future<Output = std::result::Result<Session<S>, transport::Error>> + Send>>;
pub type DialFn<S> = Arc<dyn Fn() -> DialFuture<S> + Send + Sync>;

pub struct LinkPool<S> {
    inner: Arc<PoolInner<S>>,
}

struct PoolInner<S> {
    config: Config,
    dial: DialFn<S>,
    links: Mutex<Vec<Arc<ClientLink>>>,
    dial_lock: AsyncMutex<()>,
    backoff: Mutex<Backoff>,
    closed: AtomicBool,
    next_id: AtomicU64,
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

    fn wait(&self) -> Duration {
        self.next.saturating_duration_since(Instant::now())
    }

    fn fail(&mut self, config: &Config) {
        let delay = if self.delay.is_zero() {
            config.backoff_floor
        } else {
            (self.delay * 2).min(config.backoff_ceiling)
        };
        self.delay = delay;
        self.next = Instant::now() + delay;
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
                links: Mutex::new(Vec::new()),
                dial_lock: AsyncMutex::new(()),
                backoff: Mutex::new(Backoff::new()),
                closed: AtomicBool::new(false),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub async fn open_lane(&self, authority: &str, kind: LaneKind) -> Result<H2Lane> {
        if self.inner.closed() {
            return Err(Error::Closed);
        }
        let deadline = Instant::now() + self.inner.config.open_timeout;
        let mut last = Error::Unavailable;

        for _ in 0..self.inner.config.attempts {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let link = match self.inner.acquire(deadline).await {
                Ok(link) => link,
                Err(Error::Closed) => return Err(Error::Closed),
                Err(error) => {
                    last = error;
                    continue;
                }
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }

            match timeout(
                remaining,
                link.open_lane(authority, kind, &self.inner.config),
            )
            .await
            {
                Ok(Ok(lane)) => return Ok(lane),
                Ok(Err(Error::Rejected(status))) => return Err(Error::Rejected(status)),
                Ok(Err(error)) => {
                    self.inner.discard(&link);
                    last = error;
                }
                Err(_) => {
                    self.inner.discard(&link);
                    last = Error::Timeout;
                }
            }
        }
        Err(last)
    }

    pub fn stats(&self) -> PoolStats {
        self.inner.stats()
    }

    pub fn close(&self) {
        self.inner.close();
    }
}

impl<S> Drop for LinkPool<S> {
    fn drop(&mut self) {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let links = std::mem::take(
            &mut *self
                .inner
                .links
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for link in links {
            link.close();
        }
    }
}

impl<S> PoolInner<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn lock_links(&self) -> MutexGuard<'_, Vec<Arc<ClientLink>>> {
        self.links
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_backoff(&self) -> MutexGuard<'_, Backoff> {
        self.backoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn snapshot(&self) -> (Option<Arc<ClientLink>>, Option<Arc<ClientLink>>, usize) {
        let mut links = self.lock_links();
        links.retain(|link| !link.is_closed());
        let mut light = None;
        let mut light_lanes = usize::MAX;
        let mut fallback = None;
        let mut fallback_lanes = usize::MAX;

        for link in links.iter() {
            let lanes = link.lanes();
            if link.is_bulk() {
                if lanes < fallback_lanes {
                    fallback = Some(link.clone());
                    fallback_lanes = lanes;
                }
            } else if lanes < light_lanes {
                light = Some(link.clone());
                light_lanes = lanes;
            }
        }
        let count = links.len();
        (light, fallback, count)
    }

    fn least_loaded(&self) -> (Option<Arc<ClientLink>>, usize) {
        let mut links = self.lock_links();
        links.retain(|link| !link.is_closed());
        let mut best = None;
        let mut best_load = usize::MAX;
        for link in links.iter() {
            let load = link.load();
            if load < best_load {
                best = Some(link.clone());
                best_load = load;
            }
        }
        let count = links.len();
        (best, count)
    }

    fn stats(&self) -> PoolStats {
        let mut links = self.lock_links();
        links.retain(|link| !link.is_closed());
        PoolStats {
            links: links.len(),
            bulk_links: links.iter().filter(|link| link.is_bulk()).count(),
            lanes: links.iter().map(|link| link.lanes()).sum(),
            link_ids: links.iter().map(|link| link.id()).collect(),
        }
    }

    async fn acquire(self: &Arc<Self>, deadline: Instant) -> Result<Arc<ClientLink>> {
        if self.closed() {
            return Err(Error::Closed);
        }
        let (best, count) = self.least_loaded();
        if let Some(link) = &best
            && link.load() == 0
        {
            return Ok(link.clone());
        }
        if count < self.config.max_links {
            return self.dial_fresh(deadline).await;
        }
        best.ok_or(Error::Unavailable)
    }

    async fn dial(self: &Arc<Self>, deadline: Instant) -> Result<Arc<ClientLink>> {
        self.dial_inner(deadline, false).await
    }

    async fn dial_fresh(self: &Arc<Self>, deadline: Instant) -> Result<Arc<ClientLink>> {
        self.dial_inner(deadline, true).await
    }

    async fn dial_inner(
        self: &Arc<Self>,
        deadline: Instant,
        require_idle: bool,
    ) -> Result<Arc<ClientLink>> {
        let _guard = self.dial_lock.lock().await;
        if self.closed() {
            return Err(Error::Closed);
        }

        if require_idle {
            let (best, count) = self.least_loaded();
            if let Some(link) = &best
                && link.load() == 0
            {
                return Ok(link.clone());
            }
            if count >= self.config.max_links {
                return self.least_loaded().0.ok_or(Error::Unavailable);
            }
        } else {
            let (light, fallback, count) = self.snapshot();
            if let Some(link) = light {
                return Ok(link);
            }
            if count >= self.config.max_links {
                return fallback.ok_or(Error::Unavailable);
            }
        }

        let delay = self.lock_backoff().wait();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || delay >= remaining {
            return Err(Error::Timeout);
        }
        if !delay.is_zero() {
            timeout(remaining, sleep(delay))
                .await
                .map_err(|_| Error::Timeout)?;
        }
        if self.closed() {
            return Err(Error::Closed);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::Timeout);
        }
        let session = match timeout(remaining, (self.dial)()).await {
            Ok(Ok(session)) => session,
            Ok(Err(error)) => {
                self.lock_backoff().fail(&self.config);
                return Err(Error::Transport(error));
            }
            Err(_) => {
                self.lock_backoff().fail(&self.config);
                return Err(Error::Timeout);
            }
        };

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.lock_backoff().fail(&self.config);
            return Err(Error::Timeout);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let link = match timeout(remaining, ClientLink::connect(session, &self.config, id)).await {
            Ok(Ok(link)) => link,
            Ok(Err(error)) => {
                self.lock_backoff().fail(&self.config);
                return Err(error);
            }
            Err(_) => {
                self.lock_backoff().fail(&self.config);
                return Err(Error::Timeout);
            }
        };

        if self.closed() {
            link.close();
            return Err(Error::Closed);
        }
        self.lock_backoff().reset();
        let link = Arc::new(link);
        self.lock_links().push(link.clone());
        self.clone().spawn_monitor(link.clone());
        Ok(link)
    }

    fn spawn_monitor(self: Arc<Self>, link: Arc<ClientLink>) {
        let weak = Arc::downgrade(&self);
        let shared = link.shared().clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shared.bulk_notify.notified() => {
                        let Some(inner) = weak.upgrade() else {
                            return;
                        };
                        inner.ensure_light_link().await;
                    }
                    _ = shared.idle_notify.notified() => {
                        let Some(inner) = weak.upgrade() else {
                            return;
                        };
                        let pause = inner.config.surplus_idle;
                        sleep(pause).await;
                        inner.reap_if_idle(&link);
                    }
                    _ = shared.closed_notify.notified() => return,
                }
            }
        });
    }

    async fn ensure_light_link(self: &Arc<Self>) {
        if self.closed() {
            return;
        }
        let (light, _, count) = self.snapshot();
        if light.is_some() || count >= self.config.max_links {
            return;
        }
        let deadline = Instant::now() + self.config.open_timeout;
        let _ = self.dial(deadline).await;
    }

    fn reap_if_idle(&self, link: &Arc<ClientLink>) {
        if link.lanes() != 0 || link.is_closed() {
            return;
        }
        let mut links = self.lock_links();
        links.retain(|candidate| !candidate.is_closed());
        if links.len() <= 1 {
            return;
        }
        let before = links.len();
        links.retain(|candidate| !Arc::ptr_eq(candidate, link));
        if links.len() < before {
            drop(links);
            link.close();
        }
    }

    fn discard(&self, link: &Arc<ClientLink>) {
        link.close();
        self.lock_links()
            .retain(|candidate| !Arc::ptr_eq(candidate, link));
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let links = std::mem::take(&mut *self.lock_links());
        for link in links {
            link.close();
        }
    }
}
