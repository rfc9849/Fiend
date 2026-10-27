use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::serverhello::REC_APPDATA;
use crate::crypto::RecordCipher;
use crate::fingerprint::DEFAULT_RECORD_MAX;

const RECORD_HEADER_SIZE: usize = 5;
const AEAD_TAG_SIZE: usize = 16;
const MAX_RECORD_BODY: usize = DEFAULT_RECORD_MAX + AEAD_TAG_SIZE;

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
        let body_capacity = record_max + AEAD_TAG_SIZE;
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
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    pub async fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        use tokio::io::AsyncReadExt;
        AsyncReadExt::read_exact(self, buffer).await.map(|_| ())
    }

    pub async fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        AsyncWriteExt::write_all(self, buffer).await
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
                let mut read_buf = ReadBuf::new(&mut this.read_body[this.read_body_pos..]);
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
            if !(AEAD_TAG_SIZE..=MAX_RECORD_BODY).contains(&length) {
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
        let this = self.get_mut();
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        match flush_pending(this, cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => return Poll::Pending,
        }

        let count = buf.len().min(this.record_max);
        this.write_buf
            .extend_from_slice(&[REC_APPDATA, 0x03, 0x03, 0, 0]);
        this.write_cipher
            .seal_into(&buf[..count], &mut this.write_buf)
            .map_err(crypto_error)?;
        let body_len = this.write_buf.len() - RECORD_HEADER_SIZE;
        this.write_buf[3..5].copy_from_slice(&(body_len as u16).to_be_bytes());
        this.write_pos = 0;

        match flush_pending(this, cx) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            _ => Poll::Ready(Ok(count)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match flush_pending(this, cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.stream).poll_flush(cx),
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
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
    while session.write_pos < session.write_buf.len() {
        match Pin::new(&mut session.stream).poll_write(cx, &session.write_buf[session.write_pos..])
        {
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
    session.write_buf.clear();
    session.write_pos = 0;
    Poll::Ready(Ok(()))
}

fn crypto_error(error: crate::crypto::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
