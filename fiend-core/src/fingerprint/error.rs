use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("record_max must be between 1 and {max}, got {value}")]
    RecordMax { value: usize, max: usize },
    #[error("{name} offset {offset}+{size} exceeds record length {length}")]
    FieldRange {
        name: &'static str,
        offset: usize,
        size: usize,
        length: usize,
    },
    #[error("unsupported key_share group 0x{0:04x}; regenerate the template")]
    UnsupportedKeyShareGroup(u16),
    #[error(
        "key_share group 0x{group:04x} generated {generated} bytes, template has {expected}; generate with a real share"
    )]
    KeyShareSize {
        group: u16,
        generated: usize,
        expected: usize,
    },
    #[error("read template {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("write template {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse template JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("decode template hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error(
        "verification failed: {count} unexpected byte difference(s) outside dynamic fields{details}"
    )]
    Verify { count: usize, details: String },
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
