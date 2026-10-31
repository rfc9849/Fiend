use std::sync::Arc;
use std::time::Duration;

mod flow;
mod select;
pub use flow::{UdpFlow, UdpReceiver, UdpSender};
pub use select::ServerStatus;
use tokio::net::TcpStream;

use crate::addr::Address;
use crate::mux::{self, H2Lane, LaneKind, LinkPool, PoolStats};

#[derive(Clone)]
pub struct ClientConfig {
    pub servers: Vec<String>,
    pub secret: Vec<u8>,
    pub template: crate::fingerprint::Template,
    pub mux: mux::Config,
    pub handshake_timeout: Duration,
    pub tcp_notsent_lowat: u32,
    pub protect_socket: crate::net::ProtectSocket,
}

impl ClientConfig {
    pub fn new(
        server: impl Into<String>,
        secret: impl Into<Vec<u8>>,
        template: crate::fingerprint::Template,
    ) -> Self {
        Self {
            servers: vec![server.into()],
            secret: secret.into(),
            template,
            mux: mux::Config::default(),
            handshake_timeout: Duration::from_secs(10),
            tcp_notsent_lowat: 0,
            protect_socket: Arc::new(|_| Ok(())),
        }
    }
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    pool: LinkPool<TcpStream>,
    selector: Arc<select::Selector>,
}

impl Client {
    pub fn new(config: ClientConfig) -> Self {
        let selector = Arc::new(select::Selector::new(
            config.servers.clone(),
            config.secret.clone(),
            config.template.clone(),
            config.protect_socket.clone(),
            config.tcp_notsent_lowat,
            config.handshake_timeout,
        ));
        let dial = selector.clone();
        let pool = LinkPool::new(config.mux.clone(), move || dial.clone().dial());

        Self {
            inner: Arc::new(ClientInner { pool, selector }),
        }
    }

    pub async fn ready(&self) -> mux::Result<()> {
        self.inner.pool.ready().await
    }

    pub fn stats(&self) -> PoolStats {
        self.inner.pool.stats()
    }

    pub fn rtt(&self) -> Option<Duration> {
        self.inner.pool.rtt()
    }

    pub fn close(&self) {
        self.inner.pool.close();
    }

    pub fn servers(&self) -> Vec<ServerStatus> {
        self.inner.selector.status()
    }

    pub fn network_changed(&self) {
        self.inner.selector.reset();
        self.inner.pool.network_changed();
    }

    pub async fn open_tcp(&self, address: &Address) -> mux::Result<H2Lane> {
        self.inner
            .pool
            .open_lane(&address.to_string(), LaneKind::Tcp)
            .await
    }

    pub async fn open_udp(&self, address: &Address) -> mux::Result<UdpFlow> {
        let lane = self
            .inner
            .pool
            .open_lane(&address.to_string(), LaneKind::Udp)
            .await?;
        Ok(UdpFlow::new(lane, address.clone()))
    }
}
