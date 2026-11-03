use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use h2::client::SendRequest;
use h2::{Ping, PingPong, Reason};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
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
pub(crate) const STALE: Duration = Duration::from_secs(5);
const QUIET_PROBE: Duration = Duration::from_secs(2);
const SUSPECT_FLOOR: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(crate) struct Work {
    pub(crate) lanes: usize,
    pub(crate) pending: usize,
    pub(crate) datagrams: usize,
    idle_since: Option<Instant>,
}

impl Work {
    pub(crate) fn total(self) -> usize {
        self.lanes + self.pending
    }

    pub(crate) fn streams(self) -> usize {
        self.total() - self.datagrams
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
    armed: AtomicBool,
    suspect: AtomicBool,
    last_inbound_ms: AtomicU64,
    rtt_us: AtomicU64,
    rtt_at_ms: AtomicU64,
    check_requested: AtomicBool,
    check: Notify,
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
                datagrams: 0,
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
            armed: AtomicBool::new(false),
            suspect: AtomicBool::new(false),
            last_inbound_ms: AtomicU64::new(0),
            rtt_us: AtomicU64::new(u64::MAX),
            rtt_at_ms: AtomicU64::new(0),
            check_requested: AtomicBool::new(false),
            check: Notify::new(),
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
        if bytes > 0 && !self.armed.swap(true, Ordering::AcqRel) {
            self.check.notify_one();
        }
    }

    pub(crate) fn is_suspect(&self) -> bool {
        self.suspect.load(Ordering::Acquire)
    }

    pub(crate) fn set_suspect(&self, suspect: bool) {
        if self.suspect.swap(suspect, Ordering::AcqRel) != suspect {
            self.wake.notify_one();
        }
    }

    fn quiet_probe_due(&self) -> Option<Duration> {
        if !self.armed.load(Ordering::Acquire) || self.work().lanes == 0 {
            return None;
        }
        Some(QUIET_PROBE.saturating_sub(self.idle_for()))
    }

    fn suspect_after(&self, probe: Duration) -> Duration {
        let rtt = self.rtt().map_or(Duration::ZERO, |(rtt, _)| rtt * 4);
        rtt.max(SUSPECT_FLOOR).min(probe)
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

    fn reserve_lane(&self, kind: LaneKind) {
        let mut work = self.lock_work();
        work.pending += 1;
        if kind.is_datagram() {
            work.datagrams += 1;
        }
        work.idle_since = None;
    }

    fn release_reservation(&self, kind: LaneKind) {
        let mut work = self.lock_work();
        work.pending -= 1;
        if kind.is_datagram() {
            work.datagrams -= 1;
        }
        work.became_idle();
        self.wake.notify_one();
    }

    pub(crate) fn lane_closed(&self, kind: LaneKind) {
        let mut work = self.lock_work();
        work.lanes -= 1;
        if kind.is_datagram() {
            work.datagrams -= 1;
        }
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

    fn record_rtt(&self, rtt: Duration) {
        let micros = u64::try_from(rtt.as_micros()).unwrap_or(u64::MAX - 1);
        self.rtt_at_ms.store(self.elapsed_ms(), Ordering::Release);
        self.rtt_us.store(micros, Ordering::Release);
    }

    pub(crate) fn rtt(&self) -> Option<(Duration, Duration)> {
        let micros = self.rtt_us.load(Ordering::Acquire);
        if micros == u64::MAX {
            return None;
        }
        let at = self.rtt_at_ms.load(Ordering::Acquire);
        let age = Duration::from_millis(self.elapsed_ms().saturating_sub(at));
        Some((Duration::from_micros(micros), age))
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn idle_for(&self) -> Duration {
        let last = self.last_inbound_ms.load(Ordering::Acquire);
        Duration::from_millis(self.elapsed_ms().saturating_sub(last))
    }

    fn heard(&self) {
        self.last_inbound_ms
            .store(self.elapsed_ms(), Ordering::Relaxed);
    }

    pub(crate) fn is_stale(&self) -> bool {
        self.idle_for() >= STALE
    }

    pub(crate) fn request_check(&self) {
        if !self.check_requested.swap(true, Ordering::AcqRel) {
            self.check.notify_one();
        }
    }

    fn take_check(&self) -> bool {
        self.check_requested.swap(false, Ordering::AcqRel)
    }
}

struct Heard<S> {
    inner: S,
    shared: Arc<LinkShared>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Heard<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            this.shared.heard();
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Heard<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
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

        let shared = Arc::new(LinkShared::new(wake));
        let session = Heard {
            inner: session,
            shared: shared.clone(),
        };
        let (sender, mut connection) = timeout_at(deadline, builder.handshake::<_, Bytes>(session))
            .await
            .map_err(|_| Error::Timeout)??;
        let ping_pong = connection.ping_pong();

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
            let timing = Timing::new(config);
            tokio::spawn(async move {
                keepalive_loop(shared, ping_pong, timing, driver_abort).await;
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
        link.shared.note_io(0, true);
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

    pub(crate) fn reserve(self: &Arc<Self>, kind: LaneKind) -> LaneReservation {
        self.shared.reserve_lane(kind);
        LaneReservation {
            link: self.clone(),
            kind,
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
        let options = LaneOptions::client(
            self.id,
            self.shared.clone(),
            config.lane_share,
            config.bulk_lane_bytes,
        );
        if self.shared.is_stale() {
            self.shared.request_check();
        } else if kind == LaneKind::Tcp {
            reservation.disarm();
            return Ok(H2Lane::pending(
                kind,
                authority.to_owned(),
                send,
                response,
                options,
            ));
        }
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
            options,
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
    kind: LaneKind,
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
            self.link.shared.release_reservation(self.kind);
        }
    }
}

#[derive(Clone, Copy)]
struct Timing {
    tick: Duration,
    interval: Duration,
    probe: Duration,
    measure: bool,
}

impl Timing {
    fn new(config: &Config) -> Self {
        Self {
            tick: config.ping_tick(),
            interval: config.ping_interval,
            probe: config.probe_timeout,
            measure: config.rtt_interval.is_some(),
        }
    }

    fn next_idle(&self) -> Duration {
        let low = self.interval / 2;
        let span = u64::try_from(self.interval.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        let mut bytes = [0u8; 8];
        rand::fill(&mut bytes);
        low + Duration::from_millis(u64::from_be_bytes(bytes) % span)
    }
}

async fn keepalive_loop(
    shared: Arc<LinkShared>,
    mut ping_pong: PingPong,
    timing: Timing,
    driver: AbortHandle,
) {
    let mut first = timing.measure;
    let mut idle_wait = timing.next_idle();
    loop {
        if !first {
            let idle = if timing.measure {
                timing.tick
            } else {
                idle_wait.saturating_sub(shared.idle_for())
            };
            let wait = shared
                .quiet_probe_due()
                .map_or(idle, |quiet| quiet.min(idle));
            let check = shared.check.notified();
            tokio::pin!(check);
            check.as_mut().enable();
            if !shared.check_requested.load(Ordering::Acquire) && !wait.is_zero() {
                tokio::select! {
                    _ = sleep(wait) => {}
                    _ = check => {}
                }
            }
        }
        first = false;
        if shared.is_closed() {
            return;
        }
        let checked = shared.take_check();
        let quiet = shared.idle_for();
        let quiet_probe = shared.quiet_probe_due().is_some_and(|left| left.is_zero());
        let due = timing.measure || quiet >= idle_wait || checked && quiet >= STALE || quiet_probe;
        if !due {
            continue;
        }
        shared.armed.store(false, Ordering::Release);
        let started = Instant::now();
        let ping = ping_pong.ping(Ping::opaque());
        tokio::pin!(ping);
        let suspect_after = shared.suspect_after(timing.probe);
        let mut step = suspect_after;
        let answered = loop {
            match timeout(step, &mut ping).await {
                Ok(result) => break result.is_ok(),
                Err(_) if shared.idle_for() < step => {}
                Err(_) if !shared.is_suspect() && step == suspect_after => {
                    tracing::debug!("mux link suspect: no reply to the quiet probe");
                    shared.set_suspect(true);
                    step = timing
                        .probe
                        .saturating_sub(suspect_after)
                        .max(SUSPECT_FLOOR);
                }
                Err(_) => break false,
            }
        };
        if !answered {
            shared.mark_closed();
            driver.abort();
            return;
        }
        shared.set_suspect(false);
        shared.record_rtt(started.elapsed());
        shared.heard();
        shared.take_check();
        idle_wait = timing.next_idle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn pressure_needs_sustained_load_and_expires() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        shared.reserve_lane(LaneKind::Tcp);
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
        shared.release_reservation(LaneKind::Tcp);
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
        tokio::time::advance(config.ping_interval / 2 - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(!link.is_closed());
        tokio::time::advance(config.ping_interval).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        run_for(config.probe_timeout + Duration::from_millis(1)).await;
        assert!(link.is_closed());
        held.abort();
    }

    async fn run_for(span: Duration) {
        let step = Duration::from_millis(100);
        let mut left = span;
        while !left.is_zero() {
            let now = step.min(left);
            tokio::time::advance(now).await;
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            left -= now;
        }
    }

    fn test_session<S>(io: S) -> Session<S> {
        use crate::crypto::RecordCipher;
        Session::new(
            io,
            RecordCipher::new(&[1; 32], [2; 12]),
            RecordCipher::new(&[1; 32], [2; 12]),
            16384,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_that_goes_silent_mid_transfer_is_suspected_then_closed() {
        use crate::mux::ServerConnection;
        use std::future::pending;

        let (client, server) = tokio::io::duplex(64 << 10);
        let config = Config::default();
        let server_config = config.clone();
        let held = tokio::spawn(async move {
            let connection = ServerConnection::handshake(test_session(server), &server_config)
                .await
                .expect("server handshake");
            pending::<()>().await;
            drop(connection);
        });
        let wake = Arc::new(Notify::new());
        let link = ClientLink::connect(test_session(client), &config, 1, wake.clone())
            .await
            .expect("client handshake");
        link.shared().reserve_lane(LaneKind::Tcp);
        link.shared().lane_opened();
        link.shared().note_io(64 << 10, true);
        run_for(QUIET_PROBE - Duration::from_millis(200)).await;
        assert!(!link.shared().is_suspect(), "data flowed 1.8 s ago");
        run_for(Duration::from_millis(200) + SUSPECT_FLOOR + Duration::from_millis(200)).await;
        assert!(link.shared().is_suspect(), "no reply to the quiet probe");
        assert!(!link.is_closed());
        run_for(config.probe_timeout).await;
        assert!(link.is_closed(), "closed within the probe timeout");
        held.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_live_link_gets_one_quiet_probe_per_silence() {
        use crate::mux::ServerConnection;

        let (client, server) = tokio::io::duplex(64 << 10);
        let config = Config::default();
        let server_config = config.clone();
        let served = tokio::spawn(async move {
            let mut connection = ServerConnection::handshake(test_session(server), &server_config)
                .await
                .expect("server handshake");
            while let Ok(Some(_)) = connection.accept().await {}
        });
        let link = ClientLink::connect(test_session(client), &config, 1, Arc::new(Notify::new()))
            .await
            .expect("client handshake");
        link.shared().reserve_lane(LaneKind::Tcp);
        link.shared().lane_opened();
        link.shared().note_io(512, false);
        run_for(QUIET_PROBE + Duration::from_millis(300)).await;
        let (_, age) = link.shared().rtt().expect("one quiet probe");
        assert!(age < Duration::from_millis(500));
        assert!(!link.shared().is_suspect());
        run_for(Duration::from_secs(10)).await;
        let (_, age) = link.shared().rtt().expect("sample");
        assert!(
            age >= Duration::from_secs(10),
            "no new probe without new data"
        );
        link.shared().note_io(512, true);
        run_for(QUIET_PROBE + Duration::from_millis(300)).await;
        let (_, age) = link.shared().rtt().expect("sample");
        assert!(
            age < Duration::from_millis(500),
            "new data arms one more probe"
        );
        assert!(!link.is_closed());
        served.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_link_without_lanes_gets_no_quiet_probe() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        shared.note_io(512, true);
        assert_eq!(shared.quiet_probe_due(), None);
        shared.reserve_lane(LaneKind::Tcp);
        shared.lane_opened();
        assert_eq!(shared.quiet_probe_due(), Some(QUIET_PROBE));
        tokio::time::advance(QUIET_PROBE * 2).await;
        assert_eq!(shared.quiet_probe_due(), Some(Duration::ZERO));
        shared.record_rtt(Duration::from_millis(400));
        assert_eq!(
            shared.suspect_after(Duration::from_secs(5)),
            Duration::from_millis(1600)
        );
        shared.record_rtt(Duration::from_millis(50));
        assert_eq!(shared.suspect_after(Duration::from_secs(5)), SUSPECT_FLOOR);
        shared.record_rtt(Duration::from_secs(3));
        assert_eq!(
            shared.suspect_after(Duration::from_secs(5)),
            Duration::from_secs(5)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_link_stays_quiet_and_a_stale_link_is_checked_on_use() {
        use crate::crypto::RecordCipher;
        use crate::mux::ServerConnection;

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
        let served = tokio::spawn(async move {
            let mut connection = ServerConnection::handshake(session(server), &server_config)
                .await
                .expect("server handshake");
            while let Ok(Some(_)) = connection.accept().await {}
        });
        let link = ClientLink::connect(session(client), &config, 1, Arc::new(Notify::new()))
            .await
            .expect("client handshake");
        tokio::time::advance(config.ping_interval / 2 - Duration::from_millis(1)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            link.shared().rtt().is_none(),
            "no ping before the idle wait"
        );
        assert!(link.shared().is_stale());
        link.shared().request_check();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let (_, age) = link.shared().rtt().expect("the check sends a ping");
        assert!(age < Duration::from_secs(1));
        assert!(!link.shared().is_stale());
        assert!(!link.is_closed());
        served.abort();
    }

    #[test]
    fn idle_waits_have_jitter_inside_the_range() {
        let timing = Timing::new(&Config::default());
        let waits: Vec<Duration> = (0..200).map(|_| timing.next_idle()).collect();
        assert!(waits.iter().all(|wait| *wait >= timing.interval / 2));
        assert!(waits.iter().all(|wait| *wait < timing.interval * 3 / 2));
        assert!(waits.iter().any(|wait| *wait != waits[0]));
    }

    #[tokio::test(start_paused = true)]
    async fn a_measured_link_pings_at_once_and_while_busy() {
        use crate::crypto::RecordCipher;
        use crate::mux::ServerConnection;

        let (client, server) = tokio::io::duplex(64 << 10);
        let session = |io| {
            Session::new(
                io,
                RecordCipher::new(&[1; 32], [2; 12]),
                RecordCipher::new(&[1; 32], [2; 12]),
                16384,
            )
        };
        let config = Config {
            rtt_interval: Some(Duration::from_secs(1)),
            ..Config::default()
        };
        let server_config = config.clone();
        let served = tokio::spawn(async move {
            let mut connection = ServerConnection::handshake(session(server), &server_config)
                .await
                .expect("server handshake");
            while let Ok(Some(_)) = connection.accept().await {}
        });
        let link = ClientLink::connect(session(client), &config, 1, Arc::new(Notify::new()))
            .await
            .expect("client handshake");
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let (_, age) = link.shared().rtt().expect("first sample");
        assert!(age < Duration::from_secs(1));

        tokio::time::advance(Duration::from_millis(500)).await;
        link.shared().note_io(1, true);
        tokio::time::advance(Duration::from_millis(600)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let (_, age) = link.shared().rtt().expect("busy sample");
        assert!(age < Duration::from_millis(200), "{age:?}");
        assert!(!link.is_closed());
        served.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn rtt_samples_age() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        assert!(shared.rtt().is_none());
        shared.record_rtt(Duration::from_millis(42));
        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(
            shared.rtt(),
            Some((Duration::from_millis(42), Duration::from_secs(3)))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pending_work_resets_continuous_idle() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(shared.idle_for_work(), Some(Duration::from_secs(10)));
        shared.reserve_lane(LaneKind::Tcp);
        assert_eq!(shared.work().pending, 1);
        assert_eq!(shared.idle_for_work(), None);
        shared.lane_opened();
        assert_eq!(shared.work().total(), 1);
        assert_eq!(shared.work().pending, 0);
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(shared.idle_for_work(), None);
        shared.lane_closed(LaneKind::Tcp);
        assert_eq!(shared.idle_for_work(), Some(Duration::ZERO));
        tokio::time::advance(Duration::from_secs(2)).await;
        shared.reserve_lane(LaneKind::Tcp);
        shared.release_reservation(LaneKind::Tcp);
        assert_eq!(shared.idle_for_work(), Some(Duration::ZERO));
    }

    #[test]
    fn counts_datagram_lanes_apart_from_streams() {
        let shared = LinkShared::new(Arc::new(Notify::new()));
        shared.reserve_lane(LaneKind::Udp);
        shared.reserve_lane(LaneKind::Icmp);
        shared.reserve_lane(LaneKind::Tcp);
        assert_eq!(shared.work().datagrams, 2);
        assert_eq!(shared.work().streams(), 1);
        shared.lane_opened();
        shared.lane_opened();
        assert_eq!(shared.work().datagrams, 2);
        shared.lane_closed(LaneKind::Udp);
        shared.release_reservation(LaneKind::Icmp);
        assert_eq!(shared.work().datagrams, 0);
        assert_eq!(shared.work().streams(), 1);
        shared.lane_closed(LaneKind::Tcp);
        assert_eq!(shared.work().total(), 0);
    }
}
