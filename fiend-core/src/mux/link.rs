use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use h2::client::SendRequest;
use h2::{Ping, PingPong, Reason};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, oneshot};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{Instant, sleep, timeout, timeout_at};

use super::config::Config;
use super::error::{Error, Result};
use super::lane::{H2Lane, LANE_HEADER, LaneKind, LaneOptions};
use crate::addr::Address;
use crate::transport::Session;

pub(crate) const LOAD_INTERVAL: Duration = Duration::from_millis(250);
const LOAD_COOLDOWN: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(crate) struct Work {
    pub(crate) lanes: usize,
    pub(crate) pending: usize,
    idle_since: Option<Instant>,
}

impl Work {
    pub(crate) fn total(self) -> usize {
        self.lanes + self.pending
    }

    fn became_idle(&mut self) {
        if self.total() == 0 {
            self.idle_since = Some(Instant::now());
        }
    }
}

struct Load {
    sampled: Instant,
    high_samples: usize,
    hot_until: Instant,
}

pub(crate) struct LinkShared {
    work: Mutex<Work>,
    load: Mutex<Load>,
    inbound: AtomicU64,
    outbound: AtomicU64,
    blocked_writers: AtomicUsize,
    bulk: AtomicBool,
    closed: AtomicBool,
    last_inbound_ms: AtomicU64,
    epoch: Instant,
    wake: Arc<Notify>,
}

impl LinkShared {
    fn new(wake: Arc<Notify>) -> Self {
        let now = Instant::now();
        Self {
            work: Mutex::new(Work {
                lanes: 0,
                pending: 0,
                idle_since: Some(now),
            }),
            load: Mutex::new(Load {
                sampled: now,
                high_samples: 0,
                hot_until: now,
            }),
            inbound: AtomicU64::new(0),
            outbound: AtomicU64::new(0),
            blocked_writers: AtomicUsize::new(0),
            bulk: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            last_inbound_ms: AtomicU64::new(0),
            epoch: now,
            wake,
        }
    }

    fn lock_work(&self) -> MutexGuard<'_, Work> {
        self.work.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn work(&self) -> Work {
        *self.lock_work()
    }

    pub(crate) fn note_io(&self, bytes: usize, inbound: bool) {
        if inbound {
            self.last_inbound_ms
                .store(self.elapsed_ms(), Ordering::Relaxed);
            self.inbound.fetch_add(bytes as u64, Ordering::Relaxed);
        } else {
            self.outbound.fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }

    pub(crate) fn blocked(&self, blocked: bool) {
        if blocked {
            self.blocked_writers.fetch_add(1, Ordering::Relaxed);
        } else {
            self.blocked_writers.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn blocked_writers(&self) -> usize {
        self.blocked_writers.load(Ordering::Relaxed)
    }

    pub(crate) fn sample_load(&self, threshold: u64) {
        let now = Instant::now();
        let mut load = self.load.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = now.duration_since(load.sampled);
        if elapsed < LOAD_INTERVAL {
            return;
        }
        let inbound = self.inbound.swap(0, Ordering::Relaxed);
        let outbound = self.outbound.swap(0, Ordering::Relaxed);
        let high = u128::from(inbound.max(outbound)) * LOAD_INTERVAL.as_nanos()
            >= u128::from(threshold) * elapsed.as_nanos()
            || self.blocked_writers() > 0;
        load.sampled = now;
        if high {
            load.high_samples = load.high_samples.saturating_add(1);
            if load.high_samples >= 3 {
                load.hot_until = now + LOAD_COOLDOWN;
            }
        } else {
            load.high_samples = 0;
        }
        self.bulk.store(
            now < load.hot_until && self.work().total() > 0,
            Ordering::Release,
        );
    }

    pub(crate) fn lane_opened(&self) {
        let mut work = self.lock_work();
        work.pending -= 1;
        work.lanes += 1;
    }

    fn reserve_lane(&self) {
        let mut work = self.lock_work();
        work.pending += 1;
        work.idle_since = None;
    }

    fn release_reservation(&self) {
        let mut work = self.lock_work();
        work.pending -= 1;
        work.became_idle();
        self.wake.notify_one();
    }

    pub(crate) fn lane_closed(&self) {
        let mut work = self.lock_work();
        work.lanes -= 1;
        work.became_idle();
        self.wake.notify_one();
    }

    pub(crate) fn idle_for_work(&self) -> Option<Duration> {
        self.lock_work().idle_since.map(|since| since.elapsed())
    }

    pub(crate) fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
        self.wake.notify_one();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn is_bulk(&self) -> bool {
        self.bulk.load(Ordering::Acquire) && self.work().total() > 0
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn idle_for(&self) -> Duration {
        let last = self.last_inbound_ms.load(Ordering::Acquire);
        Duration::from_millis(self.elapsed_ms().saturating_sub(last))
    }
}

pub(crate) struct ClientLink {
    id: u64,
    sender: SendRequest<Bytes>,
    max_lanes: usize,
    used: AtomicBool,
    shared: Arc<LinkShared>,
    driver: JoinHandle<()>,
    keepalive: Option<JoinHandle<()>>,
}

impl ClientLink {
    pub(crate) async fn connect<S>(
        session: Session<S>,
        config: &Config,
        id: u64,
        wake: Arc<Notify>,
    ) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let deadline = Instant::now() + config.settings_timeout;
        let mut builder = h2::client::Builder::new();
        builder
            .initial_window_size(config.stream_window)
            .initial_connection_window_size(config.connection_window)
            .max_frame_size(config.max_frame_size)
            .max_concurrent_streams(config.max_concurrent_streams)
            .max_send_buffer_size(config.max_send_buffer_size)
            .initial_max_send_streams(0)
            .enable_push(false);

        let (sender, mut connection) = timeout_at(deadline, builder.handshake::<_, Bytes>(session))
            .await
            .map_err(|_| Error::Timeout)??;
        let ping_pong = connection.ping_pong();
        let shared = Arc::new(LinkShared::new(wake));

        let driver_shared = shared.clone();
        let driver_sender = sender.clone();
        let (ready, settings) = oneshot::channel();
        let driver = tokio::spawn(async move {
            let mut ready = Some(ready);
            let _ = poll_fn(|cx| {
                let result = Pin::new(&mut connection).poll(cx);
                if driver_sender.current_max_send_streams() > 0
                    && let Some(ready) = ready.take()
                {
                    let _ = ready.send(());
                }
                result
            })
            .await;
            driver_shared.mark_closed();
        });
        let driver_abort = driver.abort_handle();

        let keepalive = ping_pong.map(|ping_pong| {
            let shared = shared.clone();
            let interval = config.ping_interval;
            let probe = config.probe_timeout;
            tokio::spawn(async move {
                keepalive_loop(shared, ping_pong, interval, probe, driver_abort).await;
            })
        });

        let link = Self {
            id,
            sender,
            max_lanes: config.max_concurrent_streams as usize,
            used: AtomicBool::new(false),
            shared,
            driver,
            keepalive,
        };
        timeout_at(deadline, settings)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Unavailable)?;
        if link.is_closed() {
            return Err(Error::Unavailable);
        }
        Ok(link)
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn shared(&self) -> &Arc<LinkShared> {
        &self.shared
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.shared.is_closed()
    }

    pub(crate) fn is_bulk(&self) -> bool {
        self.shared.is_bulk()
    }

    pub(crate) fn first_use(&self) -> bool {
        !self.used.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn has_capacity(&self) -> bool {
        self.shared.work().total() < self.sender.current_max_send_streams().min(self.max_lanes)
    }

    pub(crate) fn reserve(self: &Arc<Self>) -> LaneReservation {
        self.shared.reserve_lane();
        LaneReservation {
            link: self.clone(),
            armed: true,
        }
    }

    pub(crate) async fn open_lane(
        &self,
        authority: &str,
        kind: LaneKind,
        config: &Config,
        mut reservation: LaneReservation,
    ) -> Result<H2Lane> {
        if self.is_closed() {
            return Err(Error::Unavailable);
        }
        Address::from_authority(authority)?;
        let authority_value: http::uri::Authority =
            authority.parse().map_err(|_| Error::InvalidConnect)?;
        let uri = http::Uri::builder()
            .authority(authority_value)
            .build()
            .map_err(|_| Error::InvalidConnect)?;
        let request = http::Request::builder()
            .method(http::Method::CONNECT)
            .version(http::Version::HTTP_2)
            .uri(uri)
            .header(LANE_HEADER, kind.as_str())
            .body(())
            .map_err(|_| Error::InvalidConnect)?;

        let sender = self.sender.clone();
        let mut sender = timeout(config.open_timeout, sender.ready())
            .await
            .map_err(|_| Error::Timeout)??;
        let (response, mut send) = sender.send_request(request, false)?;
        let response = timeout(config.open_timeout, response)
            .await
            .map_err(|_| Error::Timeout)??;
        let status = response.status();
        if status != http::StatusCode::OK {
            send.send_reset(Reason::CANCEL);
            return Err(Error::Rejected(status.as_u16()));
        }

        reservation.disarm();
        Ok(H2Lane::new(
            kind,
            authority.to_owned(),
            send,
            response.into_body(),
            LaneOptions::client(
                self.id,
                self.shared.clone(),
                config.lane_share,
                config.bulk_lane_bytes,
            ),
        ))
    }

    pub(crate) fn close(&self) {
        self.shared.mark_closed();
        self.driver.abort();
        if let Some(keepalive) = &self.keepalive {
            keepalive.abort();
        }
    }
}

impl Drop for ClientLink {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) struct LaneReservation {
    pub(crate) link: Arc<ClientLink>,
    armed: bool,
}

impl LaneReservation {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for LaneReservation {
    fn drop(&mut self) {
        if self.armed {
            self.link.shared.release_reservation();
        }
    }
}

async fn keepalive_loop(
    shared: Arc<LinkShared>,
    mut ping_pong: PingPong,
    interval: Duration,
    probe: Duration,
    driver: AbortHandle,
) {
    loop {
        sleep(interval).await;
        if shared.is_closed() {
            return;
        }
        if shared.last_inbound_ms.load(Ordering::Acquire) != 0 && shared.idle_for() < interval {
            continue;
        }
        match timeout(probe, ping_pong.ping(Ping::opaque())).await {
            Ok(Ok(_)) => shared.note_io(0, true),
            _ => {
                shared.mark_closed();
                driver.abort();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn pressure_needs_sustained_load_and_expires() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        shared.reserve_lane();
        for inbound in [true, false] {
            shared.note_io(256 << 10, inbound);
            tokio::time::advance(LOAD_INTERVAL).await;
            shared.sample_load(256 << 10);
            assert!(!shared.is_bulk());
            tokio::time::advance(LOAD_INTERVAL).await;
            shared.sample_load(256 << 10);
        }
        for _ in 0..3 {
            shared.note_io(256 << 10, false);
            tokio::time::advance(LOAD_INTERVAL).await;
            shared.sample_load(256 << 10);
        }
        assert!(shared.is_bulk());
        tokio::time::advance(LOAD_COOLDOWN).await;
        shared.sample_load(256 << 10);
        assert!(!shared.is_bulk());
        shared.release_reservation();
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_link_closes_after_the_keepalive_probe() {
        use crate::crypto::RecordCipher;
        use crate::mux::ServerConnection;
        use std::future::pending;

        let (client, server) = tokio::io::duplex(64 << 10);
        let session = |io| {
            Session::new(
                io,
                RecordCipher::new(&[1; 32], [2; 12]),
                RecordCipher::new(&[1; 32], [2; 12]),
                16384,
            )
        };
        let config = Config::default();
        let server_config = config.clone();
        let held = tokio::spawn(async move {
            let connection = ServerConnection::handshake(session(server), &server_config)
                .await
                .expect("server handshake");
            pending::<()>().await;
            drop(connection);
        });
        let link = ClientLink::connect(session(client), &config, 1, Arc::new(Notify::new()))
            .await
            .expect("client handshake");
        assert!(!link.is_closed());
        tokio::time::advance(config.ping_interval + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(!link.is_closed());
        tokio::time::advance(config.probe_timeout + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(link.is_closed());
        held.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn pending_work_resets_continuous_idle() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(shared.idle_for_work(), Some(Duration::from_secs(10)));
        shared.reserve_lane();
        assert_eq!(shared.work().pending, 1);
        assert_eq!(shared.idle_for_work(), None);
        shared.lane_opened();
        assert_eq!(shared.work().total(), 1);
        assert_eq!(shared.work().pending, 0);
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(shared.idle_for_work(), None);
        shared.lane_closed();
        assert_eq!(shared.idle_for_work(), Some(Duration::ZERO));
        tokio::time::advance(Duration::from_secs(2)).await;
        shared.reserve_lane();
        shared.release_reservation();
        assert_eq!(shared.idle_for_work(), Some(Duration::ZERO));
    }
}
