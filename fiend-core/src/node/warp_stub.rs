use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::outbound::WarpSettings;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Status {
    pub enabled: bool,
    pub registered: bool,
    pub running: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rotation {
    pub before: Status,
    pub after: Status,
}

#[derive(Clone)]
pub struct Manager;

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "WARP needs Linux")
}

impl Manager {
    pub fn new(_directory: PathBuf, _binary: PathBuf) -> Self {
        Self
    }

    pub fn socks(&self) -> Option<SocketAddr> {
        None
    }

    pub async fn apply(&self, _settings: WarpSettings) {}

    pub async fn register(&self) -> io::Result<Status> {
        Err(unsupported())
    }

    pub async fn rotate(&self) -> io::Result<Rotation> {
        Err(unsupported())
    }

    pub async fn delete(&self) -> io::Result<()> {
        Err(unsupported())
    }

    pub async fn status(&self) -> Status {
        Status::default()
    }

    pub async fn maintain(&self) {
        std::future::pending::<()>().await
    }
}
