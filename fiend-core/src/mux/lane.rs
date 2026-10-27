use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use h2::{Reason, RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::link::LinkShared;

pub const LANE_HEADER: &str = "x-lane";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneKind {
    Tcp,
    Udp,
}

impl LaneKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tcp" => Some(Self::Tcp),
            "udp" => Some(Self::Udp),
            _ => None,
        }
    }
}

pub trait Lane: AsyncRead + AsyncWrite + Unpin + Send + 'static {
    fn kind(&self) -> LaneKind;
    fn authority(&self) -> &str;
    fn reset(&mut self);
}

pub(crate) struct LaneOptions {
    pub(crate) link_id: u64,
    pub(crate) shared: Option<Arc<LinkShared>>,
    pub(crate) lane_share: usize,
    pub(crate) bulk_lane_bytes: u64,
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
        }
    }

    pub(crate) fn server() -> Self {
        Self {
            link_id: 0,
            shared: None,
            lane_share: 32 << 10,
            bulk_lane_bytes: 0,
        }
    }
}

pub struct H2Lane {
    kind: LaneKind,
    authority: String,
    send: SendStream<Bytes>,
    recv: RecvStream,
    read_buf: Option<Bytes>,
    recv_eof: bool,
    send_eof: bool,
    reset: bool,
    link_id: u64,
    shared: Option<Arc<LinkShared>>,
    lane_share: usize,
    bulk_lane_bytes: u64,
    carried: u64,
    bulk: bool,
}

impl H2Lane {
    pub(crate) fn new(
        kind: LaneKind,
        authority: String,
        send: SendStream<Bytes>,
        recv: RecvStream,
        options: LaneOptions,
    ) -> Self {
        let LaneOptions {
            link_id,
            shared,
            lane_share,
            bulk_lane_bytes,
        } = options;
        if let Some(shared) = &shared {
            shared.lane_opened();
        }
        Self {
            kind,
            authority,
            send,
            recv,
            read_buf: None,
            recv_eof: false,
            send_eof: false,
            reset: false,
            link_id,
            shared,
            lane_share: lane_share.max(1),
            bulk_lane_bytes,
            carried: 0,
            bulk: false,
        }
    }

    pub fn link_id(&self) -> u64 {
        self.link_id
    }

    pub fn is_bulk(&self) -> bool {
        self.bulk
    }

    fn track(&mut self, bytes: usize, inbound: bool) {
        let Some(shared) = &self.shared else {
            return;
        };
        if bytes == 0 {
            return;
        }
        shared.note_io(inbound);
        self.carried = self.carried.saturating_add(bytes as u64);
        if !self.bulk && self.carried >= self.bulk_lane_bytes {
            self.bulk = true;
            shared.add_bulk_lane();
        }
    }
}

impl Lane for H2Lane {
    fn kind(&self) -> LaneKind {
        self.kind
    }

    fn authority(&self) -> &str {
        &self.authority
    }

    fn reset(&mut self) {
        self.send.send_reset(Reason::CANCEL);
        self.reset = true;
        self.send_eof = true;
    }
}

impl AsyncRead for H2Lane {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
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
                this.recv
                    .flow_control()
                    .release_capacity(count)
                    .map_err(io::Error::other)?;
                this.track(count, true);
                if count < chunk.len() {
                    this.read_buf = Some(chunk.slice(count..));
                }
                return Poll::Ready(Ok(()));
            }

            match this.recv.poll_data(cx) {
                Poll::Ready(Some(Ok(chunk))) => this.read_buf = Some(chunk),
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(io::Error::other(error))),
                Poll::Ready(None) => {
                    this.recv_eof = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for H2Lane {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if this.send_eof || this.reset {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }

        loop {
            let capacity = this.send.capacity();
            if capacity == 0 {
                let wanted = buf.len().min(this.lane_share);
                this.send.reserve_capacity(wanted);
                match this.send.poll_capacity(cx) {
                    Poll::Ready(Some(Ok(_))) => continue,
                    Poll::Ready(Some(Err(error))) => {
                        return Poll::Ready(Err(io::Error::other(error)));
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            let count = buf.len().min(capacity).min(this.lane_share);
            let data = Bytes::copy_from_slice(&buf[..count]);
            if let Err(error) = this.send.send_data(data, false) {
                return Poll::Ready(Err(io::Error::other(error)));
            }
            this.track(count, false);
            return Poll::Ready(Ok(count));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
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

impl Drop for H2Lane {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            if self.bulk {
                shared.remove_bulk_lane();
            }
            shared.lane_closed();
        }
        if !self.reset && !(self.send_eof && self.recv_eof) {
            self.send.send_reset(Reason::CANCEL);
        }
    }
}
