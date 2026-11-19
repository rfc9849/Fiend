use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use bytes::Bytes;
use h2::client::ResponseFuture;
use h2::{Reason, RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use super::link::{LOAD_INTERVAL, LinkShared};

enum Recv {
    Pending(ResponseFuture),
    Open(RecvStream),
}

fn rejected(status: http::StatusCode) -> io::Error {
    let kind = match status {
        http::StatusCode::FORBIDDEN => io::ErrorKind::PermissionDenied,
        http::StatusCode::GATEWAY_TIMEOUT => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::ConnectionRefused,
    };
    io::Error::new(kind, format!("the server refused the lane: {status}"))
}

pub const LANE_HEADER: &str = "x-lane";
const H2_FRAME_HEADER_SIZE: usize = 9;
const MAX_ALIGNED_H2_DATA: usize = crate::fingerprint::DEFAULT_RECORD_MAX - H2_FRAME_HEADER_SIZE;
pub(crate) type IoMeter = Arc<dyn Fn(usize, bool) + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneKind {
    Tcp,
    Udp,
    Icmp,
}

impl LaneKind {
    pub fn is_datagram(self) -> bool {
        !matches!(self, Self::Tcp)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Icmp => "icmp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tcp" => Some(Self::Tcp),
            "udp" => Some(Self::Udp),
            "icmp" => Some(Self::Icmp),
            _ => None,
        }
    }
}

pub(crate) struct LaneOptions {
    pub(crate) link_id: u64,
    pub(crate) shared: Option<Arc<LinkShared>>,
    pub(crate) lane_share: usize,
    pub(crate) bulk_lane_bytes: u64,
    pub(crate) meter: Option<IoMeter>,
}

impl LaneOptions {
    pub(crate) fn client(
        link_id: u64,
        shared: Arc<LinkShared>,
        lane_share: usize,
        bulk_lane_bytes: u64,
    ) -> Self {
        Self {
            link_id,
            shared: Some(shared),
            lane_share,
            bulk_lane_bytes,
            meter: None,
        }
    }

    pub(crate) fn server(lane_share: usize, meter: Option<IoMeter>) -> Self {
        Self {
            link_id: 0,
            shared: None,
            lane_share,
            bulk_lane_bytes: 0,
            meter,
        }
    }
}

struct LaneShared {
    kind: LaneKind,
    shared: Option<Arc<LinkShared>>,
    bulk_lane_bytes: u64,
    recv_eof: AtomicBool,
    reset: AtomicBool,
    meter: Option<IoMeter>,
}

impl Drop for LaneShared {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.lane_closed(self.kind);
        }
    }
}

struct Traffic {
    carried: u64,
    sampled: Instant,
    bulk_until: Option<Instant>,
}

impl Traffic {
    fn new() -> Self {
        Self {
            carried: 0,
            sampled: Instant::now(),
            bulk_until: None,
        }
    }

    fn is_bulk(&self) -> bool {
        self.bulk_until.is_some_and(|until| Instant::now() < until)
    }

    fn track(&mut self, state: &LaneShared, bytes: usize, inbound: bool) {
        if let Some(meter) = &state.meter {
            meter(bytes, inbound);
        }
        let Some(shared) = &state.shared else {
            return;
        };
        if bytes == 0 {
            return;
        }
        shared.note_io(bytes, inbound);
        let now = Instant::now();
        if now.duration_since(self.sampled) >= LOAD_INTERVAL {
            self.sampled = now;
            self.carried = 0;
        }
        self.carried = self.carried.saturating_add(bytes as u64);
        if self.carried >= state.bulk_lane_bytes {
            self.bulk_until = Some(now + std::time::Duration::from_secs(1));
        }
    }
}

pub struct H2LaneRead {
    recv: Recv,
    read_buf: Option<Bytes>,
    recv_eof: bool,
    traffic: Traffic,
    state: Arc<LaneShared>,
}

pub struct H2LaneWrite {
    send: SendStream<Bytes>,
    send_eof: bool,
    reset: bool,
    lane_share: usize,
    blocked: bool,
    traffic: Traffic,
    state: Arc<LaneShared>,
}

impl H2LaneWrite {
    fn set_blocked(&mut self, blocked: bool) {
        if self.blocked != blocked {
            if let Some(shared) = &self.state.shared {
                shared.blocked(blocked);
            }
            self.blocked = blocked;
        }
    }

    pub fn reset(&mut self) {
        self.set_blocked(false);
        self.send.send_reset(Reason::CANCEL);
        self.reset = true;
        self.send_eof = true;
        self.state.reset.store(true, Ordering::Release);
    }

    fn poll_write_bytes(
        &mut self,
        cx: &mut Context<'_>,
        data: &mut Bytes,
    ) -> Poll<io::Result<usize>> {
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if self.send_eof || self.reset || self.state.reset.load(Ordering::Acquire) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if let Some(shared) = &self.state.shared
            && shared.is_stale()
        {
            shared.request_check();
        }

        self.set_blocked(false);
        let limit = self.lane_share.min(MAX_ALIGNED_H2_DATA);
        loop {
            let capacity = self.send.capacity();
            if capacity == 0 {
                let wanted = data.len().min(limit);
                self.send.reserve_capacity(wanted);
                match self.send.poll_capacity(cx) {
                    Poll::Ready(Some(Ok(_))) => continue,
                    Poll::Ready(Some(Err(error))) => {
                        return Poll::Ready(Err(io::Error::other(error)));
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                    }
                    Poll::Pending => {
                        self.set_blocked(true);
                        return Poll::Pending;
                    }
                }
            }

            let count = data.len().min(capacity).min(limit);
            let chunk = data.split_to(count);
            if let Err(error) = self.send.send_data(chunk, false) {
                return Poll::Ready(Err(io::Error::other(error)));
            }
            self.traffic.track(&self.state, count, false);
            return Poll::Ready(Ok(count));
        }
    }

    pub async fn write_all_bytes(&mut self, mut data: Bytes) -> io::Result<()> {
        while !data.is_empty() {
            std::future::poll_fn(|cx| self.poll_write_bytes(cx, &mut data)).await?;
        }
        Ok(())
    }
}

impl Drop for H2LaneWrite {
    fn drop(&mut self) {
        self.set_blocked(false);
        if !(self.reset || (self.send_eof && self.state.recv_eof.load(Ordering::Acquire))) {
            self.send.send_reset(Reason::CANCEL);
        }
    }
}

impl AsyncRead for H2LaneRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Recv::Pending(response) = &mut this.recv {
            match Pin::new(response).poll(cx) {
                Poll::Ready(Ok(response)) if response.status() == http::StatusCode::OK => {
                    this.recv = Recv::Open(response.into_body());
                }
                Poll::Ready(Ok(response)) => {
                    this.recv_eof = true;
                    this.state.recv_eof.store(true, Ordering::Release);
                    this.state.reset.store(true, Ordering::Release);
                    return Poll::Ready(Err(rejected(response.status())));
                }
                Poll::Ready(Err(error)) => {
                    this.recv_eof = true;
                    this.state.recv_eof.store(true, Ordering::Release);
                    this.state.reset.store(true, Ordering::Release);
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, error)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        let Recv::Open(recv) = &mut this.recv else {
            return Poll::Pending;
        };
        loop {
            if buf.remaining() == 0 || this.recv_eof {
                return Poll::Ready(Ok(()));
            }

            if let Some(chunk) = this.read_buf.take() {
                if chunk.is_empty() {
                    continue;
                }
                let count = chunk.len().min(buf.remaining());
                buf.put_slice(&chunk[..count]);
                recv.flow_control()
                    .release_capacity(count)
                    .map_err(io::Error::other)?;
                this.traffic.track(&this.state, count, true);
                if count < chunk.len() {
                    this.read_buf = Some(chunk.slice(count..));
                }
                return Poll::Ready(Ok(()));
            }

            match recv.poll_data(cx) {
                Poll::Ready(Some(Ok(chunk))) => this.read_buf = Some(chunk),
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(io::Error::other(error))),
                Poll::Ready(None) => {
                    this.recv_eof = true;
                    this.state.recv_eof.store(true, Ordering::Release);
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for H2LaneWrite {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if this.send_eof || this.reset || this.state.reset.load(Ordering::Acquire) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if let Some(shared) = &this.state.shared
            && shared.is_stale()
        {
            shared.request_check();
        }

        this.set_blocked(false);
        let limit = this.lane_share.min(MAX_ALIGNED_H2_DATA);
        loop {
            let capacity = this.send.capacity();
            if capacity == 0 {
                let wanted = buf.len().min(limit);
                this.send.reserve_capacity(wanted);
                match this.send.poll_capacity(cx) {
                    Poll::Ready(Some(Ok(_))) => continue,
                    Poll::Ready(Some(Err(error))) => {
                        return Poll::Ready(Err(io::Error::other(error)));
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                    }
                    Poll::Pending => {
                        this.set_blocked(true);
                        return Poll::Pending;
                    }
                }
            }

            let count = buf.len().min(capacity).min(limit);
            let data = Bytes::copy_from_slice(&buf[..count]);
            if let Err(error) = this.send.send_data(data, false) {
                return Poll::Ready(Err(io::Error::other(error)));
            }
            this.traffic.track(&this.state, count, false);
            return Poll::Ready(Ok(count));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.set_blocked(false);
        if this.send_eof {
            return Poll::Ready(Ok(()));
        }
        match this.send.send_data(Bytes::new(), true) {
            Ok(()) => {
                this.send_eof = true;
                Poll::Ready(Ok(()))
            }
            Err(error) => {
                this.send_eof = true;
                if h2::Error::is_reset(&error) || h2::Error::is_io(&error) {
                    Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, error)))
                } else {
                    Poll::Ready(Err(io::Error::other(error)))
                }
            }
        }
    }
}

pub struct H2Lane {
    kind: LaneKind,
    authority: String,
    link_id: u64,
    read: H2LaneRead,
    write: H2LaneWrite,
}

impl H2Lane {
    pub(crate) fn new(
        kind: LaneKind,
        authority: String,
        send: SendStream<Bytes>,
        recv: RecvStream,
        options: LaneOptions,
    ) -> Self {
        Self::build(kind, authority, send, Recv::Open(recv), options)
    }

    pub(crate) fn pending(
        kind: LaneKind,
        authority: String,
        send: SendStream<Bytes>,
        response: ResponseFuture,
        options: LaneOptions,
    ) -> Self {
        Self::build(kind, authority, send, Recv::Pending(response), options)
    }

    fn build(
        kind: LaneKind,
        authority: String,
        send: SendStream<Bytes>,
        recv: Recv,
        options: LaneOptions,
    ) -> Self {
        let LaneOptions {
            link_id,
            shared,
            lane_share,
            bulk_lane_bytes,
            meter,
        } = options;
        if let Some(shared) = &shared {
            shared.lane_opened();
        }
        let state = Arc::new(LaneShared {
            kind,
            shared,
            bulk_lane_bytes,
            recv_eof: AtomicBool::new(false),
            reset: AtomicBool::new(false),
            meter,
        });
        Self {
            kind,
            authority,
            link_id,
            read: H2LaneRead {
                recv,
                read_buf: None,
                recv_eof: false,
                traffic: Traffic::new(),
                state: state.clone(),
            },
            write: H2LaneWrite {
                send,
                send_eof: false,
                reset: false,
                lane_share: lane_share.max(1),
                blocked: false,
                traffic: Traffic::new(),
                state,
            },
        }
    }

    pub fn link_id(&self) -> u64 {
        self.link_id
    }

    pub fn is_bulk(&self) -> bool {
        self.read.traffic.is_bulk() || self.write.traffic.is_bulk()
    }



    pub fn kind(&self) -> LaneKind {
        self.kind
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn reset(&mut self) {
        self.write.reset();
    }

    pub fn into_split(self) -> (H2LaneRead, H2LaneWrite) {
        (self.read, self.write)
    }
}

impl AsyncRead for H2Lane {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.read).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Err(_)))
            && this.read.state.reset.load(Ordering::Acquire)
            && !this.write.reset
        {
            this.write.reset();
        }
        result
    }
}

impl AsyncWrite for H2Lane {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().write).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_shutdown(cx)
    }
}
