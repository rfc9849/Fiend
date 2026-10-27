use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use h2::client::SendRequest;
use h2::{Ping, PingPong, Reason};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{sleep, timeout};

use super::config::Config;
use super::error::{Error, Result};
use super::lane::{H2Lane, LANE_HEADER, LaneKind, LaneOptions};
use crate::addr::Address;
use crate::transport::Session;

pub(crate) struct LinkShared {
    lanes: AtomicUsize,
    pending: AtomicUsize,
    bulk_lanes: AtomicUsize,
    closed: AtomicBool,
    last_inbound_ms: AtomicU64,
    epoch: Instant,
    pub(crate) bulk_notify: Notify,
    pub(crate) idle_notify: Notify,
    pub(crate) closed_notify: Notify,
}

impl LinkShared {
    fn new() -> Self {
        Self {
            lanes: AtomicUsize::new(0),
            pending: AtomicUsize::new(0),
            bulk_lanes: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            last_inbound_ms: AtomicU64::new(0),
            epoch: Instant::now(),
            bulk_notify: Notify::new(),
            idle_notify: Notify::new(),
            closed_notify: Notify::new(),
        }
    }

    pub(crate) fn note_io(&self, inbound: bool) {
        if inbound {
            self.last_inbound_ms
                .store(self.elapsed_ms(), Ordering::Relaxed);
        }
    }

    pub(crate) fn lane_opened(&self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
        self.lanes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn reserve_lane(&self) {
        self.pending.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn release_reservation(&self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }

    pub(crate) fn lane_closed(&self) {
        if self.lanes.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle_notify.notify_one();
        }
    }

    pub(crate) fn add_bulk_lane(&self) {
        if self.bulk_lanes.fetch_add(1, Ordering::AcqRel) == 0 {
            self.bulk_notify.notify_one();
        }
    }

    pub(crate) fn remove_bulk_lane(&self) {
        self.bulk_lanes.fetch_sub(1, Ordering::AcqRel);
    }

    pub(crate) fn mark_closed(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.closed_notify.notify_one();
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn is_bulk(&self) -> bool {
        self.bulk_lanes.load(Ordering::Acquire) > 0
    }

    pub(crate) fn lanes(&self) -> usize {
        self.lanes.load(Ordering::Acquire)
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
    shared: Arc<LinkShared>,
    driver: JoinHandle<()>,
    keepalive: Option<JoinHandle<()>>,
}

impl ClientLink {
    pub(crate) async fn connect<S>(session: Session<S>, config: &Config, id: u64) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let mut builder = h2::client::Builder::new();
        builder
            .initial_window_size(config.stream_window)
            .initial_connection_window_size(config.connection_window)
            .max_frame_size(config.max_frame_size)
            .max_concurrent_streams(config.max_concurrent_streams)
            .max_send_buffer_size(config.max_send_buffer_size)
            .enable_push(false);

        let (sender, mut connection) = timeout(
            config.settings_timeout,
            builder.handshake::<_, Bytes>(session),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        let ping_pong = connection.ping_pong();
        let shared = Arc::new(LinkShared::new());

        let driver_shared = shared.clone();
        let driver = tokio::spawn(async move {
            let _ = connection.await;
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

        Ok(Self {
            id,
            sender,
            shared,
            driver,
            keepalive,
        })
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

    pub(crate) fn lanes(&self) -> usize {
        self.shared.lanes()
    }

    pub(crate) async fn open_lane(
        &self,
        authority: &str,
        kind: LaneKind,
        config: &Config,
    ) -> Result<H2Lane> {
        if self.is_closed() {
            return Err(Error::Unavailable);
        }
        let mut reservation = LaneReservation::new(&self.shared);
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

struct LaneReservation<'a> {
    shared: &'a Arc<LinkShared>,
    armed: bool,
}

impl<'a> LaneReservation<'a> {
    fn new(shared: &'a Arc<LinkShared>) -> Self {
        shared.reserve_lane();
        Self {
            shared,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for LaneReservation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.shared.release_reservation();
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
            Ok(Ok(_)) => shared.note_io(true),
            _ => {
                shared.mark_closed();
                driver.abort();
                return;
            }
        }
    }
}
