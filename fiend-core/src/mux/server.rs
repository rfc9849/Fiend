use std::time::Duration;

use bytes::Bytes;
use h2::RecvStream;
use h2::server::{Connection, SendResponse};
use tokio::io::{AsyncRead, AsyncWrite, copy_bidirectional};
use tokio::time::timeout;

use super::config::Config;
use super::error::{Error, Result};
use super::lane::{H2Lane, LANE_HEADER, LaneKind, LaneOptions};
use crate::addr::Address;
use crate::target::{self, TargetPolicy};

pub struct ServerConnection<S> {
    connection: Connection<S, Bytes>,
    lane_share: usize,
}

impl<S> ServerConnection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn handshake(session: S, config: &Config) -> Result<Self> {
        let mut builder = h2::server::Builder::new();
        builder
            .initial_window_size(config.stream_window)
            .initial_connection_window_size(config.connection_window)
            .max_frame_size(config.max_frame_size)
            .max_concurrent_streams(config.max_concurrent_streams)
            .max_send_buffer_size(config.max_send_buffer_size);

        let connection = timeout(config.settings_timeout, builder.handshake(session))
            .await
            .map_err(|_| Error::Timeout)??;
        Ok(Self {
            connection,
            lane_share: config.lane_share,
        })
    }

    pub async fn accept(&mut self) -> Result<Option<Incoming>> {
        loop {
            match self.connection.accept().await {
                Some(Ok((request, respond))) => {
                    match Incoming::parse(request, respond, self.lane_share) {
                        Ok(incoming) => return Ok(Some(incoming)),
                        Err(Error::InvalidConnect) => continue,
                        Err(error) => return Err(error),
                    }
                }
                Some(Err(error)) => return Err(error.into()),
                None => return Ok(None),
            }
        }
    }
}

pub struct Incoming {
    pub kind: LaneKind,
    pub authority: String,
    pub address: Address,
    body: RecvStream,
    respond: SendResponse<Bytes>,
    lane_share: usize,
}

impl Incoming {
    fn parse(
        request: http::Request<RecvStream>,
        mut respond: SendResponse<Bytes>,
        lane_share: usize,
    ) -> Result<Self> {
        let (parts, body) = request.into_parts();
        let authority = parts
            .uri
            .authority()
            .map(|authority| authority.as_str().to_owned())
            .or_else(|| {
                parts
                    .headers
                    .get(http::header::HOST)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            });
        let kind = parts
            .headers
            .get(LANE_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(LaneKind::parse);

        if parts.method != http::Method::CONNECT
            || parts.version != http::Version::HTTP_2
            || authority.is_none()
            || kind.is_none()
        {
            reject(&mut respond, http::StatusCode::BAD_REQUEST)?;
            return Err(Error::InvalidConnect);
        }

        let authority = authority.expect("checked authority");
        let address = match Address::from_authority(&authority) {
            Ok(address) => address,
            Err(_) => {
                reject(&mut respond, http::StatusCode::BAD_REQUEST)?;
                return Err(Error::InvalidConnect);
            }
        };

        Ok(Self {
            kind: kind.expect("checked lane kind"),
            authority,
            address,
            body,
            respond,
            lane_share,
        })
    }

    pub async fn accept(mut self) -> Result<H2Lane> {
        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .version(http::Version::HTTP_2)
            .body(())
            .map_err(|_| Error::InvalidConnect)?;
        let send = self.respond.send_response(response, false)?;
        Ok(H2Lane::new(
            self.kind,
            self.authority,
            send,
            self.body,
            LaneOptions::server(self.lane_share),
        ))
    }

    pub fn reject(mut self, status: http::StatusCode) {
        let _ = reject(&mut self.respond, status);
    }
}

fn reject(respond: &mut SendResponse<Bytes>, status: http::StatusCode) -> Result<()> {
    let response = http::Response::builder()
        .status(status)
        .version(http::Version::HTTP_2)
        .body(())
        .map_err(|_| Error::InvalidConnect)?;
    respond.send_response(response, true)?;
    Ok(())
}

pub async fn serve_tcp<S>(session: S, config: Config) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    serve_tcp_with_policy(session, config, TargetPolicy::default()).await
}

pub async fn serve_tcp_with_policy<S>(
    session: S,
    config: Config,
    policy: TargetPolicy,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let target_timeout = config.target_timeout;
    let mut server = ServerConnection::handshake(session, &config).await?;
    while let Some(incoming) = server.accept().await? {
        tokio::spawn(async move {
            let _ = handle_tcp(incoming, target_timeout, policy).await;
        });
    }
    Ok(())
}

pub(crate) async fn handle_tcp(
    incoming: Incoming,
    target_timeout: Duration,
    policy: TargetPolicy,
) -> Result<()> {
    if incoming.kind != LaneKind::Tcp {
        incoming.reject(http::StatusCode::BAD_REQUEST);
        return Ok(());
    }
    let mut upstream = match target::connect_tcp(&incoming.address, &policy, target_timeout).await {
        Ok(upstream) => upstream,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            incoming.reject(http::StatusCode::FORBIDDEN);
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            incoming.reject(http::StatusCode::GATEWAY_TIMEOUT);
            return Ok(());
        }
        Err(_) => {
            incoming.reject(http::StatusCode::BAD_GATEWAY);
            return Ok(());
        }
    };
    let mut lane = incoming.accept().await?;
    match copy_bidirectional(&mut lane, &mut upstream).await {
        Ok(_) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
