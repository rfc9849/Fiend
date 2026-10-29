use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_BYTES: usize = 24 << 20;
pub const HANDSHAKE_FRAME_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("frame ended before its full length")]
    Truncated,
    #[error("frame is not valid: {0}")]
    Invalid(String),
    #[error("transport failed: {0}")]
    Io(#[from] io::Error),
}

pub fn encode_frame<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(value).map_err(|error| FrameError::Invalid(error.to_string()))?;
    if body.len() > limit {
        return Err(FrameError::TooLarge { limit });
    }
    let length = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge { limit })?;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_frame<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, FrameError> {
    let (header, body) = bytes
        .split_first_chunk::<4>()
        .ok_or(FrameError::Truncated)?;
    let length = u32::from_be_bytes(*header) as usize;
    if length > limit {
        return Err(FrameError::TooLarge { limit });
    }
    if body.len() < length {
        return Err(FrameError::Truncated);
    }
    if body.len() > length {
        return Err(FrameError::Invalid("trailing data after frame".into()));
    }
    serde_json::from_slice(body).map_err(|error| FrameError::Invalid(error.to_string()))
}

pub async fn write_frame<W, T>(writer: &mut W, value: &T, limit: usize) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode_frame(value, limit)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R, T>(reader: &mut R, limit: usize) -> Result<Option<T>, FrameError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        let read = reader.read(&mut header[filled..]).await?;
        if read == 0 {
            return if filled == 0 {
                Ok(None)
            } else {
                Err(FrameError::Truncated)
            };
        }
        filled += read;
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > limit {
        return Err(FrameError::TooLarge { limit });
    }
    let mut body = Vec::new();
    let read = reader.take(length as u64).read_to_end(&mut body).await?;
    if read < length {
        return Err(FrameError::Truncated);
    }
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| FrameError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, Hello, Request, ServerMessage};

    #[test]
    fn round_trip_keeps_messages() {
        let message = ClientMessage::Request {
            id: 7,
            request: Request::Connect {
                profile_id: Some("a".into()),
            },
        };
        let frame = encode_frame(&message, MAX_FRAME_BYTES).unwrap();
        assert_eq!(
            decode_frame::<ClientMessage>(&frame, MAX_FRAME_BYTES).unwrap(),
            message
        );
    }

    #[test]
    fn wire_format_is_stable() {
        let json = serde_json::to_string(&ClientMessage::Request {
            id: 1,
            request: Request::SetRuleText {
                kind: crate::RuleKind::Block,
                text: "x".into(),
            },
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"request","id":1,"request":{"type":"set_rule_text","kind":"block","text":"x"}}"#
        );
        let json =
            serde_json::to_string(&ServerMessage::Event(crate::Event::ProfilesChanged)).unwrap();
        assert_eq!(json, r#"{"type":"event","event":"profiles_changed"}"#);
    }

    #[test]
    fn decoder_rejects_bad_input() {
        assert!(matches!(
            decode_frame::<ClientMessage>(&[0, 0], 10),
            Err(FrameError::Truncated)
        ));
        assert!(matches!(
            decode_frame::<ClientMessage>(&[0, 0, 0, 20, 1], 10),
            Err(FrameError::TooLarge { .. })
        ));
        assert!(matches!(
            decode_frame::<ClientMessage>(&[0, 0, 0, 5, b'{'], 10),
            Err(FrameError::Truncated)
        ));
        assert!(matches!(
            decode_frame::<ClientMessage>(&[0, 0, 0, 1, b'{'], 10),
            Err(FrameError::Invalid(_))
        ));
        let mut frame = encode_frame(&Hello::new("x"), 100).unwrap();
        frame.push(0);
        assert!(decode_frame::<Hello>(&frame, 100).is_err());
        assert!(encode_frame(&Hello::new("x".repeat(200)), 100).is_err());
    }

    #[test]
    fn decoder_survives_random_bytes() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..20_000 {
            let length = (next() % 64) as usize;
            let mut bytes: Vec<u8> = (0..length).map(|_| next() as u8).collect();
            if round % 2 == 0 && bytes.len() >= 4 {
                let body = (bytes.len() - 4) as u32;
                bytes[..4].copy_from_slice(&body.to_be_bytes());
            }
            let _ = decode_frame::<ClientMessage>(&bytes, 4096);
            let _ = decode_frame::<ServerMessage>(&bytes, 4096);
        }
    }

    #[tokio::test]
    async fn reader_handles_eof_and_oversize_headers() {
        let mut empty: &[u8] = &[];
        assert!(
            read_frame::<_, Hello>(&mut empty, 100)
                .await
                .unwrap()
                .is_none()
        );
        let mut partial: &[u8] = &[0, 0];
        assert!(matches!(
            read_frame::<_, Hello>(&mut partial, 100).await,
            Err(FrameError::Truncated)
        ));
        let mut huge: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF];
        assert!(matches!(
            read_frame::<_, Hello>(&mut huge, MAX_FRAME_BYTES).await,
            Err(FrameError::TooLarge { .. })
        ));
        let mut short: &[u8] = &[0, 0, 0, 9, b'{'];
        assert!(matches!(
            read_frame::<_, Hello>(&mut short, 100).await,
            Err(FrameError::Truncated)
        ));
    }
}
