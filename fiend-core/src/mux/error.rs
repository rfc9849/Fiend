use std::io;

use thiserror::Error;

use crate::addr::AddressError;
use crate::transport;

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn closes_link(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Closed | Self::Io(_) => true,
            Self::H2(error) => error.is_io() || error.is_go_away(),
            _ => false,
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("transport: {0}")]
    Transport(#[from] transport::Error),
    #[error("h2: {0}")]
    H2(#[from] h2::Error),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("address: {0}")]
    Address(#[from] AddressError),
    #[error("link pool is closed")]
    Closed,
    #[error("timed out")]
    Timeout,
    #[error("invalid CONNECT request")]
    InvalidConnect,
    #[error("server rejected CONNECT with status {0}")]
    Rejected(u16),
    #[error("link pool has no usable link")]
    Unavailable,
    #[error("lane was reset")]
    LaneReset,
}
