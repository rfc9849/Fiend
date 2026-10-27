use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::addr::Address;

const FLAG_ADDRESS: u8 = 0x01;
const MAX_FRAME: usize = u16::MAX as usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub source: Option<Address>,
    pub payload: Vec<u8>,
}

pub async fn read_frame<R>(reader: &mut R) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut length = [0u8; 2];
    reader.read_exact(&mut length).await?;
    let length = u16::from_be_bytes(length) as usize;
    if length == 0 {
        return Ok(None);
    }

    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    Ok(Some(payload))
}

pub fn encode_frame(payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_FRAME {
        return Err(invalid("UDP frame is too large"));
    }
    let mut frame = Vec::with_capacity(2 + payload.len());
    frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let frame = encode_frame(payload)?;
    writer.write_all(&frame).await
}

pub async fn write_keepalive<W>(writer: &mut W) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(&[0, 0]).await
}

pub async fn read_response<R>(reader: &mut R) -> io::Result<Option<Response>>
where
    R: AsyncRead + Unpin,
{
    let mut length = [0u8; 2];
    reader.read_exact(&mut length).await?;
    let length = u16::from_be_bytes(length) as usize;
    if length == 0 {
        return Ok(None);
    }

    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    let flags = body[0];
    if flags & FLAG_ADDRESS == 0 {
        return Ok(Some(Response {
            source: None,
            payload: body[1..].to_vec(),
        }));
    }

    let (source, used) = Address::decode(&body[1..])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let payload_start = 1 + used;
    if payload_start > body.len() {
        return Err(invalid("UDP response address is too short"));
    }
    Ok(Some(Response {
        source: Some(source),
        payload: body[payload_start..].to_vec(),
    }))
}

pub async fn write_response<W>(writer: &mut W, source: &Address, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut frame = Vec::with_capacity(3 + 19 + payload.len());
    frame.extend_from_slice(&[0, 0, FLAG_ADDRESS]);
    source
        .encode_into(&mut frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    frame.extend_from_slice(payload);
    let length = frame.len() - 2;
    if length > MAX_FRAME {
        return Err(invalid("UDP response is too large"));
    }
    frame[..2].copy_from_slice(&(length as u16).to_be_bytes());
    writer.write_all(&frame).await
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let mut stream = Vec::new();
        write_frame(&mut stream, b"payload").await.expect("write");
        let mut reader = stream.as_slice();
        assert_eq!(
            read_frame(&mut reader).await.expect("read"),
            Some(b"payload".to_vec())
        );
    }

    #[tokio::test]
    async fn keepalives_are_empty_frames() {
        let mut stream = Vec::new();
        write_keepalive(&mut stream).await.expect("write");
        assert_eq!(stream, vec![0, 0]);
        let mut reader = stream.as_slice();
        assert_eq!(read_frame(&mut reader).await.expect("read"), None);
    }

    #[tokio::test]
    async fn responses_carry_sources() {
        let source = Address::domain("dns.example", 53);
        let payload = vec![0x55u8; 2048];
        let mut stream = Vec::new();
        write_response(&mut stream, &source, &payload)
            .await
            .expect("write");
        let mut reader = stream.as_slice();
        let response = read_response(&mut reader)
            .await
            .expect("read")
            .expect("response");
        assert_eq!(response.source, Some(source));
        assert_eq!(response.payload, payload);
    }

    #[tokio::test]
    async fn oversized_frames_are_rejected() {
        let mut stream = Vec::new();
        assert_eq!(
            write_frame(&mut stream, &vec![0u8; MAX_FRAME + 1])
                .await
                .expect_err("oversize")
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
