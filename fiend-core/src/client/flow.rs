use std::io;

use bytes::Bytes;
use tokio::io::AsyncWriteExt;

use crate::addr::Address;
use crate::mux::{H2Lane, H2LaneRead, H2LaneWrite};
use crate::udp::{self, Response};

pub struct UdpFlow {
    sender: UdpSender,
    receiver: UdpReceiver,
}

pub struct UdpSender {
    lane: H2LaneWrite,
}

pub struct UdpReceiver {
    lane: H2LaneRead,
    destination: Address,
}

impl UdpFlow {
    pub(super) fn new(lane: H2Lane, destination: Address) -> Self {
        let (read, write) = lane.into_split();
        Self {
            sender: UdpSender { lane: write },
            receiver: UdpReceiver {
                lane: read,
                destination,
            },
        }
    }

    pub async fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        self.sender.send(payload).await
    }

    pub async fn recv(&mut self) -> io::Result<Option<Response>> {
        self.receiver.recv().await
    }

    pub fn into_split(self) -> (UdpSender, UdpReceiver) {
        (self.sender, self.receiver)
    }
}

impl UdpSender {
    pub async fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        let frame = udp::encode_frame_bytes(payload)?;
        self.lane.write_all_bytes(frame).await
    }

    pub async fn keepalive(&mut self) -> io::Result<()> {
        self.lane.write_all_bytes(Bytes::from_static(&[0, 0])).await
    }

    pub async fn shutdown(&mut self) -> io::Result<()> {
        self.lane.shutdown().await
    }
}

impl UdpReceiver {
    pub async fn recv(&mut self) -> io::Result<Option<Response>> {
        let response = udp::read_response(&mut self.lane).await?;
        Ok(response.map(|mut response| {
            if response.source.is_none() {
                response.source = Some(self.destination.clone());
            }
            response
        }))
    }
}
