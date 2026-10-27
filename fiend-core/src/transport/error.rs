use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("crypto: {0}")]
    Crypto(#[from] crate::crypto::Error),
    #[error("fingerprint: {0}")]
    Fingerprint(#[from] crate::fingerprint::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("handshake timeout")]
    Timeout,
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
