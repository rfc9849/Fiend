use bytes::Bytes;
use h2::RecvStream;
use h2::server::{Connection, SendResponse};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::timeout;

use super::config::Config;
use super::error::{Error, Result};
use super::lane::{H2Lane, LANE_HEADER, LaneKind, LaneOptions};
use crate::addr::Address;

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
    meter: Option<super::lane::IoMeter>,
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
            meter: None,
        })
    }

    pub(crate) fn with_meter(mut self, meter: super::lane::IoMeter) -> Self {
        self.meter = Some(meter);
        self
    }

    pub async fn accept(mut self) -> Result<H2Lane> {
        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .version(http::Version::HTTP_2)
            .body(())
            .map_err(|_| Error::InvalidConnect)?;
        let send = self.respond.send_response(response, false)?;
        let mut lane = H2Lane::new(
            self.kind,
            self.authority,
            send,
            self.body,
            LaneOptions::server(self.lane_share),
        );
        lane.set_meter(self.meter);
        Ok(lane)
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
