mod config;
mod error;
mod lane;
mod link;
mod pool;
mod server;

use std::future::Future;

pub use config::Config;
pub use error::{Error, Result};
pub use lane::{H2Lane, LANE_HEADER, Lane, LaneKind};
pub use pool::{DialFn, DialFuture, LinkPool, PoolStats};
pub(crate) use server::handle_tcp;
pub use server::{Incoming, ServerConnection, serve_tcp, serve_tcp_with_policy};

pub trait Mux {
    type Lane: Lane;

    fn open(
        &self,
        authority: &str,
        kind: LaneKind,
    ) -> impl Future<Output = Result<Self::Lane>> + Send;
}

impl<S> Mux for LinkPool<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    type Lane = H2Lane;

    fn open(
        &self,
        authority: &str,
        kind: LaneKind,
    ) -> impl Future<Output = Result<Self::Lane>> + Send {
        self.open_lane(authority, kind)
    }
}
