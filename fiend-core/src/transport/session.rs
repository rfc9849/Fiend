use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use std::collections::VecDeque;
use std::io::IoSlice;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::serverhello::REC_APPDATA;
use crate::crypto::RecordCipher;
use crate::fingerprint::DEFAULT_RECORD_MAX;

const RECORD_HEADER_SIZE: usize = 5;
const AEAD_TAG_SIZE: usize = 16;
const INNER_CONTENT_TYPE: u8 = 0x17;
const INNER_HANDSHAKE: u8 = 0x16;
const RECORD_TYPE_APPDATA: u8 = 0x17;
const MIN_RECORD_BODY: usize = 1 + AEAD_TAG_SIZE;
const MAX_RECORD_BODY: usize = DEFAULT_RECORD_MAX + 1 + AEAD_TAG_SIZE;
const READ_BUFFER_SIZE: usize = 32 << 10;

pub struct Session<S> {
    stream: S,
    write_cipher: RecordCipher,
    read_cipher: RecordCipher,
    record_max: usize,
    read_raw: Vec<u8>,
    read_raw_pos: usize,
    read_raw_len: usize,
    read_plain_pos: usize,
    read_plain_end: usize,
    write_buf: Vec<u8>,
    recycle: Option<Vec<u8>>,
    sealed: VecDeque<Vec<u8>>,
    write_pos: usize,
}

impl<S> Session<S> {
    pub fn new(
        stream: S,
        write_cipher: RecordCipher,
        read_cipher: RecordCipher,
        record_max: usize,
    ) -> Self {
        let record_max = if record_max == 0 || record_max > DEFAULT_RECORD_MAX {
            DEFAULT_RECORD_MAX
        } else {
            record_max
        };
        let record_capacity = RECORD_HEADER_SIZE + record_max + 1 + AEAD_TAG_SIZE;
        Self {
            stream,
            write_cipher,
            read_cipher,
            record_max,
            read_raw: vec![0u8; READ_BUFFER_SIZE],
            read_raw_pos: 0,
            read_raw_len: 0,
            read_plain_pos: 0,
            read_plain_end: 0,
            write_buf: Vec::with_capacity(record_capacity),
            recycle: Some(Vec::with_capacity(record_capacity)),
            sealed: VecDeque::with_capacity(2),
            write_pos: 0,
        }
    }

    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    pub fn into_inner(self) -> S {
        self.stream
    }

    pub fn record_max(&self) -> usize {
        self.record_max
    }

    fn plain_len(&self) -> usize {
        self.write_buf.len().saturating_sub(RECORD_HEADER_SIZE)
    }

    fn seal(&mut self) -> io::Result<()> {
        if self.plain_len() == 0 {
            return Ok(());
        }
        self.write_buf.push(INNER_CONTENT_TYPE);
        let plaintext_len = self.write_buf.len() - RECORD_HEADER_SIZE;
        let body_len = plaintext_len + AEAD_TAG_SIZE;
        self.write_buf[3..5].copy_from_slice(&(body_len as u16).to_be_bytes());
        self.write_cipher
            .seal_in_place(&mut self.write_buf, RECORD_HEADER_SIZE)
            .map_err(crypto_error)?;
        let next = self.recycle.take().unwrap_or_else(|| {
            Vec::with_capacity(RECORD_HEADER_SIZE + self.record_max + 1 + AEAD_TAG_SIZE)
        });
        let record = std::mem::replace(&mut self.write_buf, next);
        self.sealed.push_back(record);
        Ok(())
    }

    fn start_record(&mut self) {
        self.write_buf.clear();
        self.write_buf
            .extend_from_slice(&[RECORD_TYPE_APPDATA, 0x03, 0x03, 0, 0]);
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    pub async fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        use tokio::io::AsyncReadExt;
        AsyncReadExt::read_exact(self, buffer).await.map(|_| ())
    }

    pub async fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        AsyncWriteExt::write_all(self, buffer).await?;
        AsyncWriteExt::flush(self).await
    }

    pub async fn flush(&mut self) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        AsyncWriteExt::flush(self).await
    }

    pub async fn write_handshake_records(&mut self, sizes: &[usize]) -> io::Result<()> {
        for &size in sizes {
            let body = size
                .saturating_sub(RECORD_HEADER_SIZE)
                .clamp(MIN_RECORD_BODY, MAX_RECORD_BODY);
            let mut plain = vec![0u8; body - AEAD_TAG_SIZE];
            plain[0] = INNER_HANDSHAKE;
            plain.reverse();
            let mut record = vec![RECORD_TYPE_APPDATA, 0x03, 0x03];
            record.extend_from_slice(&(body as u16).to_be_bytes());
            self.write_cipher
                .seal_into(&plain, &mut record)
                .map_err(crypto_error)?;
            self.sealed.push_back(record);
        }
        self.flush().await
    }

    pub async fn shutdown(&mut self) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        AsyncWriteExt::shutdown(self).await
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Session<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.read_plain_pos < this.read_plain_end {
                let available = &this.read_raw[this.read_plain_pos..this.read_plain_end];
                let count = available.len().min(buf.remaining());
                if count == 0 {
                    return Poll::Ready(Ok(()));
                }
                buf.put_slice(&available[..count]);
                this.read_plain_pos += count;
                return Poll::Ready(Ok(()));
            }

            let available = this.read_raw_len - this.read_raw_pos;
            if available >= RECORD_HEADER_SIZE {
                let header = &this.read_raw[this.read_raw_pos..this.read_raw_pos + RECORD_HEADER_SIZE];
                if header[0] != REC_APPDATA {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected TLS record type",
                    )));
                }
                let length = u16::from_be_bytes([header[3], header[4]]) as usize;
                if !(MIN_RECORD_BODY..=MAX_RECORD_BODY).contains(&length) {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid TLS record length",
                    )));
                }
                let frame_len = RECORD_HEADER_SIZE + length;
                if available >= frame_len {
                    let body_start = this.read_raw_pos + RECORD_HEADER_SIZE;
                    let body_end = this.read_raw_pos + frame_len;
                    this.read_raw_pos = body_end;
                    let plain_len = this
                        .read_cipher
                        .open_in_place_slice(&mut this.read_raw[body_start..body_end])
                        .map_err(crypto_error)?;
                    if plain_len == 0 {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected inner TLS content type",
                        )));
                    }
                    match this.read_raw[body_start + plain_len - 1] {
                        INNER_CONTENT_TYPE => {
                            this.read_plain_pos = body_start;
                            this.read_plain_end = body_start + plain_len - 1;
                            continue;
                        }
                        INNER_HANDSHAKE => {
                            this.read_plain_pos = 0;
                            this.read_plain_end = 0;
                            continue;
                        }
                        _ => {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "unexpected inner TLS content type",
                            )));
                        }
                    }
                }
            }

            this.read_plain_pos = 0;
            this.read_plain_end = 0;
            if this.read_raw_pos == this.read_raw_len {
                this.read_raw_pos = 0;
                this.read_raw_len = 0;
            } else if this.read_raw_pos > 0 {
                this.read_raw
                    .copy_within(this.read_raw_pos..this.read_raw_len, 0);
                this.read_raw_len -= this.read_raw_pos;
                this.read_raw_pos = 0;
            }

            let mut read_buf = ReadBuf::new(&mut this.read_raw[this.read_raw_len..]);
            match Pin::new(&mut this.stream).poll_read(cx, &mut read_buf) {
                Poll::Ready(Ok(())) => {
                    let count = read_buf.filled().len();
                    if count == 0 {
                        let remaining = this.read_raw_len - this.read_raw_pos;
                        if remaining == 0 {
                            return Poll::Ready(Ok(()));
                        }
                        let message = if remaining < RECORD_HEADER_SIZE {
                            "truncated TLS record header"
                        } else {
                            "truncated TLS record"
                        };
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            message,
                        )));
                    }
                    this.read_raw_len += count;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Session<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.poll_write_vectored(cx, &[IoSlice::new(buf)])
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let total: usize = bufs.iter().map(|buf| buf.len()).sum();
        if total == 0 {
            return Poll::Ready(Ok(0));
        }
        match flush_pending(this, cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => return Poll::Pending,
        }

        let mut written = 0;
        for buf in bufs {
            let mut remaining: &[u8] = buf.as_ref();
            while !remaining.is_empty() {
                if this.write_buf.is_empty() {
                    this.start_record();
                }
                let room = this.record_max - this.plain_len();
                let count = remaining.len().min(room);
                this.write_buf.extend_from_slice(&remaining[..count]);
                remaining = &remaining[count..];
                written += count;
                if this.plain_len() == this.record_max {
                    this.seal()?;
                    match flush_pending(this, cx) {
                        Poll::Ready(Ok(())) => {}
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Pending => return Poll::Ready(Ok(written)),
                    }
                }
            }
        }

        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.seal()?;
        match flush_pending(this, cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.stream).poll_flush(cx),
            other => other,
        }
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.seal()?;
        match flush_pending(this, cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.stream).poll_shutdown(cx),
            other => other,
        }
    }
}

fn flush_pending<S: AsyncWrite + Unpin>(
    session: &mut Session<S>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    while let Some(front) = session.sealed.front() {
        while session.write_pos < front.len() {
            match Pin::new(&mut session.stream).poll_write(cx, &front[session.write_pos..]) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "TLS stream accepted no bytes",
                    )));
                }
                Poll::Ready(Ok(count)) => session.write_pos += count,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
        }
        if let Some(mut done) = session.sealed.pop_front()
            && session.recycle.is_none()
        {
            done.clear();
            session.recycle = Some(done);
        }
        session.write_pos = 0;
    }
    Poll::Ready(Ok(()))
}

fn crypto_error(error: crate::crypto::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::crypto::RecordCipher;
    use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

    #[tokio::test]
    async fn handshake_records_have_the_given_sizes_and_are_skipped() {
        let (client, mut wire) = tokio::io::duplex(1 << 16);
        let cipher = || RecordCipher::new(&[4; 32], [5; 12]);
        let mut server = Session::new(client, cipher(), cipher(), DEFAULT_RECORD_MAX);
        server
            .write_handshake_records(&[255, 265])
            .await
            .expect("tickets");
        server.write_all(b"h2").await.expect("data");
        let mut raw = vec![0u8; 255 + 265];
        wire.read_exact(&mut raw).await.expect("ticket bytes");
        assert_eq!(&raw[..5], &[0x17, 0x03, 0x03, 0x00, 250]);
        assert_eq!(&raw[255..260], &[0x17, 0x03, 0x03, 0x01, 4]);
        let mut rest = vec![0u8; 5 + 2 + 1 + AEAD_TAG_SIZE];
        wire.read_exact(&mut rest).await.expect("data bytes");
        raw.extend_from_slice(&rest);
        let (reader, mut feed) = tokio::io::duplex(1 << 16);
        feed.write_all(&raw).await.expect("feed");
        let mut client = Session::new(reader, cipher(), cipher(), DEFAULT_RECORD_MAX);
        let mut data = [0u8; 2];
        client
            .read_exact(&mut data)
            .await
            .expect("data after tickets");
        assert_eq!(&data, b"h2");
    }

    fn session<S>(stream: S, record_max: usize) -> Session<S> {
        Session::new(
            stream,
            RecordCipher::new(&[1u8; 32], [2u8; 12]),
            RecordCipher::new(&[3u8; 32], [4u8; 12]),
            record_max,
        )
    }

    async fn read_record<R: AsyncRead + Unpin>(stream: &mut R) -> Vec<u8> {
        let mut header = [0u8; 5];
        stream.read_exact(&mut header).await.expect("record header");
        let length = u16::from_be_bytes([header[3], header[4]]) as usize;
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).await.expect("record body");
        body
    }

    #[tokio::test]
    async fn writes_full_records_with_tls13_overhead() {
        let (writer, mut wire) = tokio::io::duplex(1 << 20);
        let mut session = session(writer, DEFAULT_RECORD_MAX);
        session
            .write_all(&vec![0x55u8; DEFAULT_RECORD_MAX])
            .await
            .expect("write");
        let body = read_record(&mut wire).await;
        assert_eq!(
            body.len(),
            DEFAULT_RECORD_MAX + 1 + AEAD_TAG_SIZE,
            "record must carry the inner content type byte and the AEAD tag"
        );
    }

    #[tokio::test]
    async fn vectored_slices_share_one_record() {
        let (writer, mut wire) = tokio::io::duplex(1 << 20);
        let mut session = session(writer, DEFAULT_RECORD_MAX);
        let slices = [
            IoSlice::new(b"header"),
            IoSlice::new(b"payload"),
            IoSlice::new(b"trailer"),
        ];
        let written =
            std::future::poll_fn(|cx| Pin::new(&mut session).poll_write_vectored(cx, &slices))
                .await
                .expect("write");
        assert_eq!(written, 20);
        session.flush().await.expect("flush");
        let body = read_record(&mut wire).await;
        assert_eq!(body.len(), 20 + 1 + AEAD_TAG_SIZE);
        let mut extra = [0u8; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(50), wire.read(&mut extra))
                .await
                .is_err(),
            "one record only"
        );
    }

    #[tokio::test]
    async fn reads_records_split_across_polls() {
        let (mut wire, reader) = tokio::io::duplex(1 << 20);
        let mut session = session(reader, DEFAULT_RECORD_MAX);
        let mut payload = session_bytes(b"split payload");
        for byte in payload.drain(..) {
            wire.write_all(&[byte]).await.expect("write");
            wire.flush().await.expect("flush");
        }
        let mut out = vec![0u8; 13];
        session.read_exact(&mut out).await.expect("read");
        assert_eq!(&out, b"split payload");
    }

    #[test]
    fn reports_vectored_write_support() {
        let (writer, _) = tokio::io::duplex(1024);
        let session = session(writer, DEFAULT_RECORD_MAX);
        assert!(session.is_write_vectored());
    }

    #[tokio::test]
    async fn reads_back_to_back_records_from_one_buffer() {
        let (mut wire, reader) = tokio::io::duplex(1 << 20);
        let mut reader = session(reader, DEFAULT_RECORD_MAX);
        let mut cipher = RecordCipher::new(&[3u8; 32], [4u8; 12]);
        let mut combined = Vec::new();
        for payload in [&b"first record"[..], &b"second record"[..]] {
            let mut record = vec![RECORD_TYPE_APPDATA, 0x03, 0x03, 0, 0];
            let mut plain = payload.to_vec();
            plain.push(INNER_CONTENT_TYPE);
            cipher.seal_into(&plain, &mut record).expect("seal");
            let length = (record.len() - RECORD_HEADER_SIZE) as u16;
            record[3..5].copy_from_slice(&length.to_be_bytes());
            combined.extend_from_slice(&record);
        }
        wire.write_all(&combined).await.expect("write");
        wire.shutdown().await.expect("shutdown");
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.expect("read");
        assert_eq!(&out, b"first recordsecond record");
    }

    fn session_bytes(payload: &[u8]) -> Vec<u8> {
        let mut cipher = RecordCipher::new(&[3u8; 32], [4u8; 12]);
        let mut record = vec![RECORD_TYPE_APPDATA, 0x03, 0x03, 0, 0];
        let mut sealed = Vec::new();
        let mut plain = payload.to_vec();
        plain.push(INNER_CONTENT_TYPE);
        cipher
            .seal_into(&plain, &mut sealed)
            .map_err(|_| ())
            .expect("seal");
        record.extend_from_slice(&sealed);
        let length = (record.len() - RECORD_HEADER_SIZE) as u16;
        record[3..5].copy_from_slice(&length.to_be_bytes());
        record
    }
}
