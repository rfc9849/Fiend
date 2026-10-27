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
const RECORD_TYPE_APPDATA: u8 = 0x17;
const MIN_RECORD_BODY: usize = 1 + AEAD_TAG_SIZE;
const MAX_RECORD_BODY: usize = DEFAULT_RECORD_MAX + 1 + AEAD_TAG_SIZE;

pub struct Session<S> {
    stream: S,
    write_cipher: RecordCipher,
    read_cipher: RecordCipher,
    record_max: usize,
    read_header: [u8; RECORD_HEADER_SIZE],
    read_header_pos: usize,
    read_body: Vec<u8>,
    read_body_pos: usize,
    read_body_len: usize,
    read_plain: Vec<u8>,
    read_plain_pos: usize,
    write_buf: Vec<u8>,
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
        let body_capacity = record_max + 1 + AEAD_TAG_SIZE;
        Self {
            stream,
            write_cipher,
            read_cipher,
            record_max,
            read_header: [0u8; RECORD_HEADER_SIZE],
            read_header_pos: 0,
            read_body: Vec::with_capacity(body_capacity),
            read_body_pos: 0,
            read_body_len: 0,
            read_plain: Vec::with_capacity(body_capacity),
            read_plain_pos: 0,
            write_buf: Vec::with_capacity(body_capacity + RECORD_HEADER_SIZE),
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
        let mut record = Vec::with_capacity(body_len);
        self.write_cipher
            .seal_into(&self.write_buf[RECORD_HEADER_SIZE..], &mut record)
            .map_err(crypto_error)?;
        record.splice(0..0, self.write_buf[..RECORD_HEADER_SIZE].iter().copied());
        self.write_buf.clear();
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
            if this.read_plain_pos < this.read_plain.len() {
                let available = &this.read_plain[this.read_plain_pos..];
                let count = available.len().min(buf.remaining());
                if count == 0 {
                    return Poll::Ready(Ok(()));
                }
                buf.put_slice(&available[..count]);
                this.read_plain_pos += count;
                return Poll::Ready(Ok(()));
            }

            if this.read_body_pos < this.read_body_len {
                let mut read_buf =
                    ReadBuf::new(&mut this.read_body[this.read_body_pos..this.read_body_len]);
                match Pin::new(&mut this.stream).poll_read(cx, &mut read_buf) {
                    Poll::Ready(Ok(())) => {
                        let count = read_buf.filled().len();
                        if count == 0 {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "truncated TLS record",
                            )));
                        }
                        this.read_body_pos += count;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Pending => return Poll::Pending,
                }
                if this.read_body_pos < this.read_body_len {
                    continue;
                }

                this.read_cipher
                    .open_in_place(&mut this.read_body)
                    .map_err(crypto_error)?;
                if this.read_body.last() != Some(&INNER_CONTENT_TYPE) {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected inner TLS content type",
                    )));
                }
                this.read_body.pop();
                std::mem::swap(&mut this.read_body, &mut this.read_plain);
                this.read_plain_pos = 0;
                this.read_body_pos = 0;
                this.read_body_len = 0;
                continue;
            }

            let mut read_buf = ReadBuf::new(&mut this.read_header[this.read_header_pos..]);
            match Pin::new(&mut this.stream).poll_read(cx, &mut read_buf) {
                Poll::Ready(Ok(())) => {
                    let count = read_buf.filled().len();
                    if count == 0 {
                        if this.read_header_pos == 0 {
                            return Poll::Ready(Ok(()));
                        }
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated TLS record header",
                        )));
                    }
                    this.read_header_pos += count;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
            if this.read_header_pos < RECORD_HEADER_SIZE {
                continue;
            }

            if this.read_header[0] != REC_APPDATA {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected TLS record type",
                )));
            }
            let length = u16::from_be_bytes([this.read_header[3], this.read_header[4]]) as usize;
            if !(MIN_RECORD_BODY..=MAX_RECORD_BODY).contains(&length) {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid TLS record length",
                )));
            }
            this.read_body.resize(length, 0);
            this.read_body_pos = 0;
            this.read_body_len = length;
            this.read_header_pos = 0;
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
        session.sealed.pop_front();
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
