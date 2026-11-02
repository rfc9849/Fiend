use std::io;
use std::time::Duration;

use fiend_core::fingerprint::server_name;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, timeout_at};

const HEADER: usize = 5;
const MAX_RECORD: usize = 16 << 10;
const HANDSHAKE: u8 = 0x16;
const CLIENT_HELLO: u8 = 0x01;

pub const FIRST_DATA_WAIT: Duration = Duration::from_millis(500);

/// Reads the first client data until a full TLS record is in the buffer.
/// Other data, end of stream, or the deadline stops the read early.
pub async fn read_first<R: AsyncRead + Unpin>(
    stream: &mut R,
    wait: Duration,
) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + wait;
    let mut buffer = Vec::with_capacity(2048);
    loop {
        match timeout_at(deadline, stream.read_buf(&mut buffer)).await {
            Err(_) | Ok(Ok(0)) => return Ok(buffer),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => return Err(error),
        }
        match record_end(&buffer) {
            Some(end) if buffer.len() < end => {}
            _ if buffer.len() < HEADER && looks_like_record(&buffer) => {}
            _ => return Ok(buffer),
        }
    }
}

/// Sends the first client data. A TLS ClientHello is sent as two TLS
/// records and three TCP segments. The two cuts are inside the server name.
pub async fn write_first<W: AsyncWrite + Unpin>(stream: &mut W, data: &[u8]) -> io::Result<()> {
    for segment in segments(data) {
        stream.write_all(&segment).await?;
        stream.flush().await?;
    }
    Ok(())
}

fn looks_like_record(data: &[u8]) -> bool {
    data.first().is_none_or(|byte| *byte == HANDSHAKE) && data.get(1).is_none_or(|byte| *byte == 3)
}

fn record_end(data: &[u8]) -> Option<usize> {
    if data.len() < HEADER || !looks_like_record(data) {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([data[3], data[4]]));
    (length <= MAX_RECORD).then_some(HEADER + length)
}

pub fn segments(data: &[u8]) -> Vec<Vec<u8>> {
    let Some(end) = record_end(data) else {
        return vec![data.to_vec()];
    };
    if data.len() < end || data.get(HEADER) != Some(&CLIENT_HELLO) {
        return vec![data.to_vec()];
    }
    let Some(name) = server_name(&data[..end]) else {
        return first_byte(data);
    };
    let cut = name.start + name.len() / 2;
    let header = |length: usize| {
        let mut record = data[..3].to_vec();
        record.extend_from_slice(&(length as u16).to_be_bytes());
        record
    };
    let mut first = header(cut - HEADER);
    first.extend_from_slice(&data[HEADER..cut]);
    let mut second = header(end - cut);
    second.extend_from_slice(&data[cut..]);
    let tail = first.split_off(1);
    vec![first, tail, second]
}

fn first_byte(data: &[u8]) -> Vec<Vec<u8>> {
    if data.len() < 2 {
        return vec![data.to_vec()];
    }
    vec![data[..1].to_vec(), data[1..].to_vec()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_hello(host: &str) -> Vec<u8> {
        let mut sni = Vec::new();
        sni.extend_from_slice(&((host.len() + 3) as u16).to_be_bytes());
        sni.push(0);
        sni.extend_from_slice(&(host.len() as u16).to_be_bytes());
        sni.extend_from_slice(host.as_bytes());
        let mut extensions = Vec::new();
        extensions.extend_from_slice(&[0x00, 0x0b, 0x00, 0x02, 0x01, 0x00]);
        extensions.extend_from_slice(&[0x00, 0x00]);
        extensions.extend_from_slice(&(sni.len() as u16).to_be_bytes());
        extensions.extend_from_slice(&sni);
        let mut body = vec![0x03, 0x03];
        body.extend_from_slice(&[7; 32]);
        body.push(32);
        body.extend_from_slice(&[9; 32]);
        body.extend_from_slice(&[0x00, 0x04, 0x13, 0x01, 0x13, 0x02]);
        body.extend_from_slice(&[0x01, 0x00]);
        body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
        body.extend_from_slice(&extensions);
        let mut handshake = vec![CLIENT_HELLO];
        handshake.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        handshake.extend_from_slice(&body);
        let mut record = vec![HANDSHAKE, 0x03, 0x01];
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }

    fn records(stream: &[u8]) -> (Vec<u8>, usize) {
        let mut payload = Vec::new();
        let mut count = 0;
        let mut rest = stream;
        while !rest.is_empty() {
            assert_eq!(&rest[..3], &[HANDSHAKE, 0x03, 0x01]);
            let length = usize::from(u16::from_be_bytes([rest[3], rest[4]]));
            payload.extend_from_slice(&rest[HEADER..HEADER + length]);
            rest = &rest[HEADER + length..];
            count += 1;
        }
        (payload, count)
    }

    #[test]
    fn server_name_is_found() {
        let hello = client_hello("www.youtube.com");
        let range = server_name(&hello).unwrap();
        assert_eq!(&hello[range], b"www.youtube.com");
        assert!(server_name(&hello[..60]).is_none());
        assert!(server_name(b"\x16\x03\x01\x00\x01\x02").is_none());
    }

    #[test]
    fn client_hello_is_cut_inside_the_server_name() {
        let hello = client_hello("www.youtube.com");
        let parts = segments(&hello);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], vec![HANDSHAKE]);
        let name = server_name(&hello).unwrap();
        let first = &parts[1];
        assert!(first.ends_with(&hello[name.start..name.start + 7]));
        assert!(parts[2][HEADER..].starts_with(&hello[name.start + 7..name.end]));
        let joined = parts.concat();
        assert_eq!(joined.len(), hello.len() + HEADER);
        let (payload, count) = records(&joined);
        assert_eq!(count, 2);
        assert_eq!(payload, hello[HEADER..]);
    }

    #[test]
    fn data_after_the_record_stays_in_order() {
        let mut data = client_hello("a.example");
        data.extend_from_slice(b"early");
        let joined = segments(&data).concat();
        assert!(joined.ends_with(b"early"));
        assert_eq!(joined.len(), data.len() + HEADER);
    }

    #[test]
    fn other_data_is_not_changed() {
        assert_eq!(segments(b"GET / HTTP/1.1\r\n\r\n").len(), 1);
        assert_eq!(segments(b"").len(), 1);
        let hello = client_hello("a.example");
        assert_eq!(segments(&hello[..40]), vec![hello[..40].to_vec()]);
        let mut server_hello = hello.clone();
        server_hello[HEADER] = 0x02;
        assert_eq!(segments(&server_hello), vec![server_hello.clone()]);
        let mut nameless = hello.clone();
        let name = server_name(&hello).unwrap();
        nameless[name.start - 9] = 0x00;
        nameless[name.start - 8] = 0x2a;
        assert_eq!(segments(&nameless).concat(), nameless);
        assert_eq!(segments(&nameless).len(), 2);
    }

    #[tokio::test]
    async fn first_read_waits_for_the_full_record() {
        let hello = client_hello("www.youtube.com");
        let (mut client, mut server) = tokio::io::duplex(4096);
        let sent = hello.clone();
        let writer = tokio::spawn(async move {
            for part in sent.chunks(20) {
                client.write_all(part).await.unwrap();
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            client
        });
        let first = read_first(&mut server, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(first, hello);
        let _client = writer.await.unwrap();
    }

    #[tokio::test]
    async fn first_read_stops_at_the_deadline_or_other_data() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let first = read_first(&mut server, Duration::from_millis(20))
            .await
            .unwrap();
        assert!(first.is_empty());
        client.write_all(b"SSH-2.0-x\r\n").await.unwrap();
        let first = read_first(&mut server, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(first, b"SSH-2.0-x\r\n");
        client
            .write_all(&client_hello("a.example")[..30])
            .await
            .unwrap();
        let first = read_first(&mut server, Duration::from_millis(50))
            .await
            .unwrap();
        assert_eq!(first.len(), 30);
    }
}
