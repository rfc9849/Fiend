use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::{RData, RecordType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::dns::valid_domain;
use crate::engine::Context;
use crate::rules::{Host, Route, is_global};
use crate::transparent::{
    DNS_TCP_IDLE, MAX_DNS_MESSAGE, MAX_DNS_QUERIES, Names, exchange_direct, exchange_tunnel,
    invalid_dns,
};

pub const CAPTURED_TTL: u32 = 300;

pub trait CaptureSet: Send + Sync + 'static {
    fn add(&self, addresses: &[IpAddr]) -> io::Result<()>;
}

pub type Upstream = Arc<dyn Fn() -> SocketAddr + Send + Sync>;

pub struct GatewayDns {
    pub udp: Vec<std::net::UdpSocket>,
    pub tcp: Vec<std::net::TcpListener>,
    pub upstream: Upstream,
    pub capture: Arc<dyn CaptureSet>,
    pub capture_all: bool,
}

pub(crate) struct Server {
    context: Arc<Context>,
    names: Arc<Names>,
    upstream: Upstream,
    capture: Arc<dyn CaptureSet>,
    capture_all: bool,
    queries: Arc<Semaphore>,
}

impl Server {
    pub(crate) fn new(context: Arc<Context>, names: Arc<Names>, gateway: &GatewayDns) -> Arc<Self> {
        Arc::new(Self {
            context,
            names,
            upstream: gateway.upstream.clone(),
            capture: gateway.capture.clone(),
            capture_all: gateway.capture_all,
            queries: Arc::new(Semaphore::new(MAX_DNS_QUERIES)),
        })
    }

    pub(crate) async fn serve_udp(self: Arc<Self>, socket: UdpSocket) -> io::Result<()> {
        let socket = Arc::new(socket);
        let mut buffer = vec![0; MAX_DNS_MESSAGE];
        loop {
            let (size, client) = socket.recv_from(&mut buffer).await?;
            let Ok(permit) = self.queries.clone().try_acquire_owned() else {
                self.context.drop_packet();
                continue;
            };
            let query = buffer[..size].to_vec();
            let server = self.clone();
            let socket = socket.clone();
            tokio::spawn(async move {
                let _permit = permit;
                match server.answer(&query).await {
                    Ok(answer) => {
                        if let Err(error) = socket.send_to(&answer, client).await {
                            tracing::debug!(%error, "cannot send a DNS answer");
                        }
                    }
                    Err(error) => tracing::debug!(%error, "DNS query failed"),
                }
            });
        }
    }

    pub(crate) async fn serve_tcp(self: Arc<Self>, listener: TcpListener) -> io::Result<()> {
        loop {
            let (stream, _) = fiend_core::net::accept(&listener).await;
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(error) = server.stream(stream).await {
                    tracing::debug!(%error, "DNS stream closed");
                }
            });
        }
    }

    async fn stream(&self, mut stream: TcpStream) -> io::Result<()> {
        loop {
            let length = match timeout(DNS_TCP_IDLE, stream.read_u16()).await {
                Err(_) => return Ok(()),
                Ok(Err(error)) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Ok(result) => usize::from(result?),
            };
            if length > MAX_DNS_MESSAGE {
                return Err(invalid_dns());
            }
            let mut query = vec![0; length];
            timeout(DNS_TCP_IDLE, stream.read_exact(&mut query)).await??;
            let answer = self.answer(&query).await?;
            let length = u16::try_from(answer.len()).map_err(|_| invalid_dns())?;
            stream.write_u16(length).await?;
            stream.write_all(&answer).await?;
        }
    }

    async fn answer(&self, query: &[u8]) -> io::Result<Vec<u8>> {
        let request = Message::from_vec(query).map_err(|_| invalid_dns())?;
        let id = request.metadata.id;
        let Some(question) = request.queries.first() else {
            return Err(invalid_dns());
        };
        let name = question
            .name
            .to_ascii()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if !valid_domain(&name) {
            return exchange_direct(&self.context, (self.upstream)(), query, id).await;
        }
        let route = self.context.rules.route(Host::Name(&name), false);
        let listed = !self.capture_all && matches!(route, Route::Proxy | Route::Dpi);
        let empty = match question.query_type {
            RecordType::HTTPS | RecordType::SVCB => listed,
            RecordType::AAAA => !self.context.options.ipv6 && route != Route::Direct,
            _ => false,
        };
        if route == Route::Block || empty {
            let mut response = Message::response(id, request.metadata.op_code);
            response.metadata.recursion_desired = request.metadata.recursion_desired;
            response.metadata.recursion_available = true;
            if route == Route::Block {
                response.metadata.response_code = ResponseCode::NXDomain;
            }
            response.add_query(question.clone());
            return response.to_vec().map_err(|_| invalid_dns());
        }
        let answer = match route {
            Route::Proxy => exchange_tunnel(&self.context, query, id).await?,
            _ => exchange_direct(&self.context, (self.upstream)(), query, id).await?,
        };
        self.names.remember(&name, &answer);
        if !listed {
            return Ok(answer);
        }
        let Ok(mut message) = Message::from_vec(&answer) else {
            return Ok(answer);
        };
        let addresses: Vec<IpAddr> = message
            .answers
            .iter()
            .filter_map(|record| match &record.data {
                RData::A(address) => Some(IpAddr::V4(address.0)),
                RData::AAAA(address) if self.context.options.ipv6 => Some(IpAddr::V6(address.0)),
                _ => None,
            })
            .filter(|ip| is_global(*ip))
            .collect();
        if !addresses.is_empty() {
            let capture = self.capture.clone();
            let list = addresses.clone();
            let added = tokio::task::spawn_blocking(move || capture.add(&list))
                .await
                .map_err(io::Error::other)
                .and_then(|result| result);
            match added {
                Ok(()) => {
                    tracing::debug!(%name, ?route, count = addresses.len(), "site addresses go to Fiend")
                }
                Err(error) => tracing::warn!(%name, %error, "cannot add the site addresses"),
            }
        }
        for record in message
            .answers
            .iter_mut()
            .chain(message.authorities.iter_mut())
        {
            record.ttl = record.ttl.min(CAPTURED_TTL);
        }
        Ok(message.to_vec().unwrap_or(answer))
    }
}
