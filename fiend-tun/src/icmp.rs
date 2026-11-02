use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const ICMP: u8 = 1;
const ICMPV6: u8 = 58;
const ECHO_REQUEST_V4: u8 = 8;
const ECHO_REPLY_V4: u8 = 0;
const ECHO_REQUEST_V6: u8 = 128;
const ECHO_REPLY_V6: u8 = 129;
const TTL: u8 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Echo {
    pub source: IpAddr,
    pub destination: IpAddr,
    pub ident: u16,
    pub sequence: u16,
    pub data: Vec<u8>,
}

fn sum(bytes: &[u8], mut total: u32) -> u32 {
    let (pairs, rest) = bytes.as_chunks::<2>();
    for pair in pairs {
        total += u32::from(u16::from_be_bytes(*pair));
    }
    if let [last] = rest {
        total += u32::from(*last) << 8;
    }
    total
}

fn fold(mut total: u32) -> u16 {
    while total > 0xffff {
        total = (total & 0xffff) + (total >> 16);
    }
    !(total as u16)
}

fn echo_body(kind: u8, ident: u16, sequence: u16, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + data.len());
    body.extend_from_slice(&[kind, 0, 0, 0]);
    body.extend_from_slice(&ident.to_be_bytes());
    body.extend_from_slice(&sequence.to_be_bytes());
    body.extend_from_slice(data);
    body
}

fn parse_body(body: &[u8], request: u8) -> Option<(u16, u16, Vec<u8>)> {
    if body.len() < 8 || body[0] != request || body[1] != 0 {
        return None;
    }
    Some((
        u16::from_be_bytes([body[4], body[5]]),
        u16::from_be_bytes([body[6], body[7]]),
        body[8..].to_vec(),
    ))
}

pub fn parse_echo_request(raw: &[u8]) -> Option<Echo> {
    match raw.first()? >> 4 {
        4 => {
            let header = usize::from(raw[0] & 0x0f) * 4;
            let total = usize::from(u16::from_be_bytes([*raw.get(2)?, *raw.get(3)?]));
            if header < 20 || total < header || raw.len() < total || raw[9] != ICMP {
                return None;
            }
            let fragment = u16::from_be_bytes([raw[6], raw[7]]);
            if fragment & 0x3fff != 0 {
                return None;
            }
            let (ident, sequence, data) = parse_body(&raw[header..total], ECHO_REQUEST_V4)?;
            Some(Echo {
                source: IpAddr::V4(Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15])),
                destination: IpAddr::V4(Ipv4Addr::new(raw[16], raw[17], raw[18], raw[19])),
                ident,
                sequence,
                data,
            })
        }
        6 => {
            if raw.len() < 40 || raw[6] != ICMPV6 {
                return None;
            }
            let length = usize::from(u16::from_be_bytes([raw[4], raw[5]]));
            if raw.len() < 40 + length {
                return None;
            }
            let (ident, sequence, data) = parse_body(&raw[40..40 + length], ECHO_REQUEST_V6)?;
            let source: [u8; 16] = raw[8..24].try_into().ok()?;
            let destination: [u8; 16] = raw[24..40].try_into().ok()?;
            Some(Echo {
                source: IpAddr::V6(Ipv6Addr::from(source)),
                destination: IpAddr::V6(Ipv6Addr::from(destination)),
                ident,
                sequence,
                data,
            })
        }
        _ => None,
    }
}

pub fn build_echo_reply(request: &Echo, sequence: u16, data: &[u8]) -> Option<Vec<u8>> {
    match (request.destination, request.source) {
        (IpAddr::V4(from), IpAddr::V4(to)) => {
            let mut body = echo_body(ECHO_REPLY_V4, request.ident, sequence, data);
            let checksum = fold(sum(&body, 0));
            body[2..4].copy_from_slice(&checksum.to_be_bytes());
            let total = u16::try_from(20 + body.len()).ok()?;
            let mut packet = Vec::with_capacity(usize::from(total));
            packet.extend_from_slice(&[0x45, 0]);
            packet.extend_from_slice(&total.to_be_bytes());
            packet.extend_from_slice(&[0, 0, 0x40, 0, TTL, ICMP, 0, 0]);
            packet.extend_from_slice(&from.octets());
            packet.extend_from_slice(&to.octets());
            let checksum = fold(sum(&packet, 0));
            packet[10..12].copy_from_slice(&checksum.to_be_bytes());
            packet.extend_from_slice(&body);
            Some(packet)
        }
        (IpAddr::V6(from), IpAddr::V6(to)) => {
            let mut body = echo_body(ECHO_REPLY_V6, request.ident, sequence, data);
            let length = u16::try_from(body.len()).ok()?;
            let mut pseudo = Vec::with_capacity(40);
            pseudo.extend_from_slice(&from.octets());
            pseudo.extend_from_slice(&to.octets());
            pseudo.extend_from_slice(&u32::from(length).to_be_bytes());
            pseudo.extend_from_slice(&[0, 0, 0, ICMPV6]);
            let checksum = fold(sum(&body, sum(&pseudo, 0)));
            body[2..4].copy_from_slice(&checksum.to_be_bytes());
            let mut packet = Vec::with_capacity(40 + body.len());
            packet.extend_from_slice(&[0x60, 0, 0, 0]);
            packet.extend_from_slice(&length.to_be_bytes());
            packet.extend_from_slice(&[ICMPV6, TTL]);
            packet.extend_from_slice(&from.octets());
            packet.extend_from_slice(&to.octets());
            packet.extend_from_slice(&body);
            Some(packet)
        }
        _ => None,
    }
}

pub fn encode_payload(sequence: u16, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + data.len());
    payload.extend_from_slice(&sequence.to_be_bytes());
    payload.extend_from_slice(data);
    payload
}

pub fn decode_payload(payload: &[u8]) -> Option<(u16, &[u8])> {
    let sequence = u16::from_be_bytes([*payload.first()?, *payload.get(1)?]);
    Some((sequence, &payload[2..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_v4(data: &[u8]) -> Vec<u8> {
        let echo = Echo {
            source: "10.0.0.2".parse().unwrap(),
            destination: "1.1.1.1".parse().unwrap(),
            ident: 0x1234,
            sequence: 7,
            data: data.to_vec(),
        };
        let mut packet = build_echo_reply(
            &Echo {
                source: echo.destination,
                destination: echo.source,
                ..echo
            },
            7,
            data,
        )
        .unwrap();
        packet[20] = ECHO_REQUEST_V4;
        packet
    }

    #[test]
    fn ipv4_echo_round_trip_has_valid_checksums() {
        let raw = request_v4(b"abcdefgh");
        let echo = parse_echo_request(&raw).unwrap();
        assert_eq!(echo.ident, 0x1234);
        assert_eq!(echo.sequence, 7);
        assert_eq!(echo.data, b"abcdefgh");
        assert_eq!(echo.destination, "1.1.1.1".parse::<IpAddr>().unwrap());
        let reply = build_echo_reply(&echo, 7, b"abcdefgh").unwrap();
        assert_eq!(fold(sum(&reply[..20], 0)), 0, "IP header checksum");
        assert_eq!(fold(sum(&reply[20..], 0)), 0, "ICMP checksum");
        assert_eq!(&reply[12..16], &[1, 1, 1, 1]);
        assert_eq!(&reply[16..20], &[10, 0, 0, 2]);
        assert_eq!(reply[20], ECHO_REPLY_V4);
        assert!(
            parse_echo_request(&reply).is_none(),
            "a reply is not a request"
        );
    }

    #[test]
    fn ipv6_reply_checksum_covers_the_pseudo_header() {
        let echo = Echo {
            source: "fd42::2".parse().unwrap(),
            destination: "2001:db8::1".parse().unwrap(),
            ident: 9,
            sequence: 3,
            data: b"odd".to_vec(),
        };
        let reply = build_echo_reply(&echo, 3, b"odd").unwrap();
        let mut pseudo = Vec::new();
        pseudo.extend_from_slice(&reply[8..40]);
        pseudo
            .extend_from_slice(&u32::from(u16::from_be_bytes([reply[4], reply[5]])).to_be_bytes());
        pseudo.extend_from_slice(&[0, 0, 0, ICMPV6]);
        assert_eq!(fold(sum(&reply[40..], sum(&pseudo, 0))), 0);
        let mut request = reply.clone();
        request[8..24].copy_from_slice(&reply[24..40]);
        request[24..40].copy_from_slice(&reply[8..24]);
        request[40] = ECHO_REQUEST_V6;
        let parsed = parse_echo_request(&request).unwrap();
        assert_eq!(parsed.source, echo.source);
        assert_eq!(parsed.destination, echo.destination);
        assert_eq!((parsed.ident, parsed.sequence), (9, 3));
    }

    #[test]
    fn other_packets_are_not_echo_requests() {
        let mut raw = request_v4(b"x");
        raw[9] = 17;
        assert!(parse_echo_request(&raw).is_none());
        let mut fragment = request_v4(b"x");
        fragment[7] = 1;
        assert!(parse_echo_request(&fragment).is_none());
        assert!(parse_echo_request(&[0x45]).is_none());
        assert!(parse_echo_request(&[]).is_none());
    }

    #[test]
    fn payloads_carry_the_sequence() {
        let payload = encode_payload(513, b"data");
        assert_eq!(decode_payload(&payload), Some((513, &b"data"[..])));
        assert_eq!(decode_payload(&[1]), None);
    }
}
