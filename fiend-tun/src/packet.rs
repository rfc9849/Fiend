use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::ops::Range;
use std::time::{Duration, Instant};

use etherparse::{
    IpFragOffset, IpNumber, Ipv4HeaderSlice, Ipv6FragmentHeader, Ipv6HeaderSlice, NetSlice,
    PacketBuilder, SlicedPacket, TransportSlice,
};
use thiserror::Error;

const FRAGMENT_TTL: Duration = Duration::from_secs(30);
const MAX_FRAGMENTS: usize = 128;
const MAX_ASSEMBLIES: usize = 64;
const MAX_FRAGMENT_BYTES: usize = 4 << 20;

#[derive(Debug, Error)]
#[error("packet is not valid or is not supported")]
pub struct PacketError;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FlowKey {
    pub source: SocketAddr,
    pub destination: SocketAddr,
}

pub enum Transport {
    Tcp {
        syn: bool,
        rst: bool,
        sequence: u32,
        acknowledgment: Option<u32>,
        sequence_len: u32,
    },
    Udp(Vec<u8>),
}

pub struct Packet {
    pub key: FlowKey,
    pub transport: Transport,
}

impl Packet {
    pub fn parse(raw: &[u8]) -> Result<Self, PacketError> {
        let sliced = SlicedPacket::from_ip(raw).map_err(|_| PacketError)?;
        let (source, destination) = match &sliced.net {
            Some(NetSlice::Ipv4(ip)) if !ip.payload().fragmented => {
                let header = ip.header();
                if header.header_checksum() != header.to_header().calc_header_checksum() {
                    return Err(PacketError);
                }
                (
                    IpAddr::V4(header.source_addr()),
                    IpAddr::V4(header.destination_addr()),
                )
            }
            Some(NetSlice::Ipv6(ip)) if !ip.payload().fragmented => {
                if !matches!(ip.header().next_header(), IpNumber::TCP | IpNumber::UDP) {
                    return Err(PacketError);
                }
                (
                    IpAddr::V6(ip.header().source_addr()),
                    IpAddr::V6(ip.header().destination_addr()),
                )
            }
            _ => return Err(PacketError),
        };
        let (source_port, destination_port, transport) = match sliced.transport {
            Some(TransportSlice::Tcp(tcp)) => {
                let header = tcp.to_header();
                let expected = match (source, destination) {
                    (IpAddr::V4(src), IpAddr::V4(dst)) => {
                        header.calc_checksum_ipv4_raw(src.octets(), dst.octets(), tcp.payload())
                    }
                    (IpAddr::V6(src), IpAddr::V6(dst)) => {
                        header.calc_checksum_ipv6_raw(src.octets(), dst.octets(), tcp.payload())
                    }
                    _ => return Err(PacketError),
                }
                .map_err(|_| PacketError)?;
                if expected != tcp.checksum() {
                    return Err(PacketError);
                }
                (
                    tcp.source_port(),
                    tcp.destination_port(),
                    Transport::Tcp {
                        syn: tcp.syn() && !tcp.ack(),
                        rst: tcp.rst(),
                        sequence: tcp.sequence_number(),
                        acknowledgment: tcp.ack().then(|| tcp.acknowledgment_number()),
                        sequence_len: tcp.payload().len() as u32
                            + u32::from(tcp.syn())
                            + u32::from(tcp.fin()),
                    },
                )
            }
            Some(TransportSlice::Udp(udp)) => {
                let expected =
                    match (source, destination) {
                        (IpAddr::V4(src), IpAddr::V4(dst)) => udp
                            .to_header()
                            .calc_checksum_ipv4_raw(src.octets(), dst.octets(), udp.payload()),
                        (IpAddr::V6(src), IpAddr::V6(dst)) => udp
                            .to_header()
                            .calc_checksum_ipv6_raw(src.octets(), dst.octets(), udp.payload()),
                        _ => return Err(PacketError),
                    }
                    .map_err(|_| PacketError)?;
                if udp.checksum() != expected && !(source.is_ipv4() && udp.checksum() == 0) {
                    return Err(PacketError);
                }
                (
                    udp.source_port(),
                    udp.destination_port(),
                    Transport::Udp(udp.payload().to_vec()),
                )
            }
            _ => return Err(PacketError),
        };
        Ok(Self {
            key: FlowKey {
                source: SocketAddr::new(source, source_port),
                destination: SocketAddr::new(destination, destination_port),
            },
            transport,
        })
    }

    pub fn reset(&self) -> Result<Vec<u8>, PacketError> {
        let Transport::Tcp {
            rst,
            sequence,
            acknowledgment,
            sequence_len,
            ..
        } = self.transport
        else {
            return Err(PacketError);
        };
        if rst {
            return Err(PacketError);
        }
        let builder = match (self.key.destination.ip(), self.key.source.ip()) {
            (IpAddr::V4(src), IpAddr::V4(dst)) => {
                PacketBuilder::ipv4(src.octets(), dst.octets(), 64)
            }
            (IpAddr::V6(src), IpAddr::V6(dst)) => {
                PacketBuilder::ipv6(src.octets(), dst.octets(), 64)
            }
            _ => return Err(PacketError),
        };
        let builder = builder
            .tcp(
                self.key.destination.port(),
                self.key.source.port(),
                acknowledgment.unwrap_or(0),
                0,
            )
            .rst();
        let builder = if acknowledgment.is_none() {
            builder.ack(sequence.wrapping_add(sequence_len))
        } else {
            builder
        };
        let mut raw = Vec::new();
        builder.write(&mut raw, &[]).map_err(|_| PacketError)?;
        Ok(raw)
    }
}

pub fn abort_fin(raw: &[u8]) -> Option<(FlowKey, Vec<Vec<u8>>)> {
    let sliced = SlicedPacket::from_ip(raw).ok()?;
    let (source, destination) = match &sliced.net {
        Some(NetSlice::Ipv4(ip)) => (
            IpAddr::V4(ip.header().source_addr()),
            IpAddr::V4(ip.header().destination_addr()),
        ),
        Some(NetSlice::Ipv6(ip)) => (
            IpAddr::V6(ip.header().source_addr()),
            IpAddr::V6(ip.header().destination_addr()),
        ),
        _ => return None,
    };
    let Some(TransportSlice::Tcp(tcp)) = &sliced.transport else {
        return None;
    };
    if !tcp.fin() || tcp.rst() {
        return None;
    }
    let key = FlowKey {
        source: SocketAddr::new(destination, tcp.destination_port()),
        destination: SocketAddr::new(source, tcp.source_port()),
    };
    let payload = tcp.payload();
    let mut packets = Vec::with_capacity(2);
    if !payload.is_empty() {
        let offset = raw.len() - payload.len() - tcp.header_len();
        let mut data = raw.to_vec();
        data[offset + 13] &= !0x01;
        let header = etherparse::TcpHeaderSlice::from_slice(&data[offset..])
            .ok()?
            .to_header();
        let checksum = match (source, destination) {
            (IpAddr::V4(src), IpAddr::V4(dst)) => {
                header.calc_checksum_ipv4_raw(src.octets(), dst.octets(), payload)
            }
            (IpAddr::V6(src), IpAddr::V6(dst)) => {
                header.calc_checksum_ipv6_raw(src.octets(), dst.octets(), payload)
            }
            _ => return None,
        }
        .ok()?;
        data[offset + 16..offset + 18].copy_from_slice(&checksum.to_be_bytes());
        packets.push(data);
    }
    let builder = match (source, destination) {
        (IpAddr::V4(src), IpAddr::V4(dst)) => PacketBuilder::ipv4(src.octets(), dst.octets(), 64),
        (IpAddr::V6(src), IpAddr::V6(dst)) => PacketBuilder::ipv6(src.octets(), dst.octets(), 64),
        _ => return None,
    };
    let sequence = tcp.sequence_number().wrapping_add(payload.len() as u32);
    let builder = builder
        .tcp(tcp.source_port(), tcp.destination_port(), sequence, 0)
        .rst();
    let builder = if tcp.ack() {
        builder.ack(tcp.acknowledgment_number())
    } else {
        builder
    };
    let mut reset = Vec::new();
    builder.write(&mut reset, &[]).ok()?;
    packets.push(reset);
    Some((key, packets))
}

pub fn udp_packets(
    source: SocketAddr,
    destination: SocketAddr,
    payload: &[u8],
    mtu: u16,
    id: u32,
) -> Result<Vec<Vec<u8>>, PacketError> {
    if payload.len() > 65507 || mtu < 1280 {
        return Err(PacketError);
    }
    let builder = match (source.ip(), destination.ip()) {
        (IpAddr::V4(src), IpAddr::V4(dst)) => PacketBuilder::ipv4(src.octets(), dst.octets(), 64),
        (IpAddr::V6(src), IpAddr::V6(dst)) => PacketBuilder::ipv6(src.octets(), dst.octets(), 64),
        _ => return Err(PacketError),
    };
    let mut raw = Vec::new();
    builder
        .udp(source.port(), destination.port())
        .write(&mut raw, payload)
        .map_err(|_| PacketError)?;
    if raw.len() <= usize::from(mtu) {
        return Ok(vec![raw]);
    }
    let v4 = source.is_ipv4();
    let header_size = if v4 { 20 } else { 40 };
    let chunk_size = (usize::from(mtu) - if v4 { 20 } else { 48 }) & !7;
    let body = &raw[header_size..];
    let mut packets = Vec::new();
    for (index, chunk) in body.chunks(chunk_size).enumerate() {
        let offset = index * chunk_size;
        let more = offset + chunk.len() < body.len();
        let mut packet = if v4 {
            let mut header = Ipv4HeaderSlice::from_slice(&raw)
                .map_err(|_| PacketError)?
                .to_header();
            header.identification = id as u16;
            header.dont_fragment = false;
            header.more_fragments = more;
            header.fragment_offset =
                IpFragOffset::try_from((offset / 8) as u16).map_err(|_| PacketError)?;
            header
                .set_payload_len(chunk.len())
                .map_err(|_| PacketError)?;
            let mut packet = Vec::new();
            header.write(&mut packet).map_err(|_| PacketError)?;
            packet
        } else {
            let mut header = Ipv6HeaderSlice::from_slice(&raw)
                .map_err(|_| PacketError)?
                .to_header();
            header.next_header = IpNumber::IPV6_FRAGMENTATION_HEADER;
            header.payload_length = (8 + chunk.len()) as u16;
            let mut packet = Vec::new();
            header.write(&mut packet).map_err(|_| PacketError)?;
            Ipv6FragmentHeader::new(
                IpNumber::UDP,
                IpFragOffset::try_from((offset / 8) as u16).map_err(|_| PacketError)?,
                more,
                id,
            )
            .write(&mut packet)
            .map_err(|_| PacketError)?;
            packet
        };
        packet.extend_from_slice(chunk);
        packets.push(packet);
    }
    Ok(packets)
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct FragmentKey {
    source: IpAddr,
    destination: IpAddr,
    protocol: u8,
    id: u32,
}

struct Assembly {
    expires: Instant,
    header: Option<Vec<u8>>,
    parts: Vec<(Range<usize>, Vec<u8>)>,
    end: Option<usize>,
    bytes: usize,
    rejected: bool,
}

#[derive(Default)]
pub struct Reassembly {
    entries: HashMap<FragmentKey, Assembly>,
    bytes: usize,
}

impl Reassembly {
    pub fn ingest(&mut self, raw: &[u8], now: Instant) -> Result<Option<Vec<u8>>, PacketError> {
        self.expire(now);
        let (key, offset, more, payload, mut header) = match raw.first().map(|byte| byte >> 4) {
            Some(4) => {
                let header = Ipv4HeaderSlice::from_slice(raw).map_err(|_| PacketError)?;
                if header.header_checksum() != header.to_header().calc_header_checksum() {
                    return Err(PacketError);
                }
                let length = usize::from(header.total_len());
                if length < header.slice().len() || length > raw.len() {
                    return Err(PacketError);
                }
                if !header.more_fragments() && header.fragments_offset().value() == 0 {
                    return Ok(Some(raw[..length].to_vec()));
                }
                if header.dont_fragment() {
                    return Err(PacketError);
                }
                let key = FragmentKey {
                    source: header.source_addr().into(),
                    destination: header.destination_addr().into(),
                    protocol: header.protocol().0,
                    id: u32::from(header.identification()),
                };
                (
                    key,
                    usize::from(header.fragments_offset().byte_offset()),
                    header.more_fragments(),
                    &raw[header.slice().len()..length],
                    header.slice().to_vec(),
                )
            }
            Some(6) => {
                let header = Ipv6HeaderSlice::from_slice(raw).map_err(|_| PacketError)?;
                let length = 40 + usize::from(header.payload_length());
                if length > raw.len() {
                    return Err(PacketError);
                }
                if header.next_header() != IpNumber::IPV6_FRAGMENTATION_HEADER {
                    return Ok(Some(raw[..length].to_vec()));
                }
                let (fragment, payload) =
                    Ipv6FragmentHeader::from_slice(&raw[40..length]).map_err(|_| PacketError)?;
                let key = FragmentKey {
                    source: header.source_addr().into(),
                    destination: header.destination_addr().into(),
                    protocol: fragment.next_header.0,
                    id: fragment.identification,
                };
                let mut base = raw[..40].to_vec();
                base[6] = fragment.next_header.0;
                (
                    key,
                    usize::from(fragment.fragment_offset.byte_offset()),
                    fragment.more_fragments,
                    payload,
                    base,
                )
            }
            _ => return Err(PacketError),
        };
        if !matches!(key.protocol, 6 | 17) || payload.is_empty() || more && payload.len() % 8 != 0 {
            return Err(PacketError);
        }
        let end = offset
            .checked_add(payload.len())
            .filter(|end| {
                *end <= if key.source.is_ipv4() {
                    65535 - header.len()
                } else {
                    65535
                }
            })
            .ok_or(PacketError)?;
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= MAX_ASSEMBLIES {
                return Err(PacketError);
            }
            self.entries.insert(
                key,
                Assembly {
                    expires: now + FRAGMENT_TTL,
                    header: None,
                    parts: Vec::new(),
                    end: None,
                    bytes: 0,
                    rejected: false,
                },
            );
        }
        let entry = self.entries.get_mut(&key).ok_or(PacketError)?;
        if entry.rejected {
            return Err(PacketError);
        }
        let reject = entry.parts.len() >= MAX_FRAGMENTS
            || self.bytes + payload.len() > MAX_FRAGMENT_BYTES
            || entry
                .parts
                .iter()
                .any(|(range, _)| offset < range.end && end > range.start)
            || entry
                .end
                .is_some_and(|length| end > length || !more && length != end)
            || !more && entry.parts.iter().any(|(range, _)| range.end > end);
        if reject {
            self.bytes -= entry.bytes;
            entry.bytes = 0;
            entry.parts.clear();
            entry.header = None;
            entry.rejected = true;
            return Err(PacketError);
        }
        if offset == 0 {
            entry.header = Some(std::mem::take(&mut header));
        }
        if !more {
            entry.end = Some(end);
        }
        entry.bytes += payload.len();
        self.bytes += payload.len();
        entry.parts.push((offset..end, payload.to_vec()));
        if entry.end != Some(entry.bytes) || entry.header.is_none() {
            return Ok(None);
        }
        let entry = self.entries.remove(&key).ok_or(PacketError)?;
        self.bytes -= entry.bytes;
        let mut packet = entry.header.ok_or(PacketError)?;
        let header_size = packet.len();
        packet.resize(header_size + entry.bytes, 0);
        for (range, payload) in entry.parts {
            packet[header_size + range.start..header_size + range.end].copy_from_slice(&payload);
        }
        if key.source.is_ipv4() {
            let mut header = Ipv4HeaderSlice::from_slice(&packet)
                .map_err(|_| PacketError)?
                .to_header();
            header.fragment_offset = IpFragOffset::ZERO;
            header.more_fragments = false;
            header
                .set_payload_len(entry.bytes)
                .map_err(|_| PacketError)?;
            header
                .write(&mut &mut packet[..header_size])
                .map_err(|_| PacketError)?;
        } else {
            packet[4..6].copy_from_slice(&(entry.bytes as u16).to_be_bytes());
        }
        Ok(Some(packet))
    }

    fn expire(&mut self, now: Instant) {
        self.entries.retain(|_, entry| {
            if entry.expires <= now {
                self.bytes -= entry.bytes;
                false
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_fragments_preserve_one_message_in_both_ip_versions() {
        for (source, destination) in [
            ("1.1.1.1:53", "10.0.0.2:30000"),
            ("[2606:4700:4700::1111]:53", "[fd00::2]:30000"),
        ] {
            for size in [0, 4096, 65507] {
                let payload = vec![0xa5; size];
                let mut fragments = udp_packets(
                    source.parse().unwrap(),
                    destination.parse().unwrap(),
                    &payload,
                    1500,
                    123,
                )
                .unwrap();
                let mut reassembly = Reassembly::default();
                let mut complete = Vec::new();
                fragments.reverse();
                for fragment in fragments {
                    assert!(fragment.len() <= 1500);
                    if let Some(packet) = reassembly.ingest(&fragment, Instant::now()).unwrap() {
                        complete.push(packet);
                    }
                }
                assert_eq!(complete.len(), 1);
                let packet = Packet::parse(&complete[0]).unwrap();
                let Transport::Udp(restored) = packet.transport else {
                    panic!("UDP required")
                };
                assert_eq!(restored, payload);
                assert_eq!(reassembly.bytes, 0);
            }
        }
    }

    #[test]
    fn overlap_timeout_and_fragment_limits_are_bounded() {
        let packets = udp_packets(
            "1.1.1.1:53".parse().unwrap(),
            "10.0.0.2:30000".parse().unwrap(),
            &vec![1; 4096],
            1500,
            123,
        )
        .unwrap();
        let now = Instant::now();
        let mut table = Reassembly::default();
        assert!(table.ingest(&packets[0], now).unwrap().is_none());
        assert!(table.ingest(&packets[0], now).is_err());
        assert_eq!(table.bytes, 0);
        assert!(table.ingest(&packets[1], now).is_err());
        assert!(
            table
                .ingest(&packets[0], now + FRAGMENT_TTL)
                .unwrap()
                .is_none()
        );
        for id in 124..124 + MAX_ASSEMBLIES as u32 {
            let more = udp_packets(
                "1.1.1.1:53".parse().unwrap(),
                "10.0.0.2:30000".parse().unwrap(),
                &vec![1; 4096],
                1500,
                id,
            )
            .unwrap();
            let _ = table.ingest(&more[0], now + FRAGMENT_TTL);
        }
        assert_eq!(table.entries.len(), MAX_ASSEMBLIES);
        assert!(table.bytes <= MAX_FRAGMENT_BYTES);
    }
}
