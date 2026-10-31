use std::io;

use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf, split};

use crate::addr::Address;
use crate::mux::H2Lane;
use crate::udp::{self, Response};

pub struct UdpFlow {
    sender: UdpSender,
    receiver: UdpReceiver,
}

pub struct UdpSender {
    lane: WriteHalf<H2Lane>,
}

pub struct UdpReceiver {
    lane: ReadHalf<H2Lane>,
    destination: Address,
}

impl UdpFlow {
    pub(super) fn new(lane: H2Lane, destination: Address) -> Self {
        let (read, write) = split(lane);
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
        udp::write_frame(&mut self.lane, payload).await
    }

    pub async fn keepalive(&mut self) -> io::Result<()> {
        udp::write_keepalive(&mut self.lane).await
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
