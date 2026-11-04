//! Frames on the Unix socket between the app and the helper: a 4-byte
//! length, JSON, and the file descriptors of the message.

use std::io::{self, IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

use nix::errno::Errno;
use nix::sys::socket::{ControlMessage, ControlMessageOwned, MsgFlags, UnixAddr, recvmsg, sendmsg};
use serde::Serialize;
use serde::de::DeserializeOwned;

const MAX_FRAME: usize = 64 << 10;
const MAX_FDS: usize = 8;

pub fn send<T: Serialize>(stream: &UnixStream, message: &T, fds: &[RawFd]) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    if body.len() > MAX_FRAME || fds.len() > MAX_FDS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame is too large",
        ));
    }
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    let rights = [ControlMessage::ScmRights(fds)];
    let controls: &[ControlMessage] = if fds.is_empty() { &[] } else { &rights };
    let mut sent = loop {
        match sendmsg::<UnixAddr>(
            stream.as_raw_fd(),
            &[IoSlice::new(&frame)],
            controls,
            MsgFlags::MSG_NOSIGNAL,
            None,
        ) {
            Err(Errno::EINTR) => continue,
            result => break result?,
        }
    };
    while sent < frame.len() {
        sent += loop {
            match sendmsg::<UnixAddr>(
                stream.as_raw_fd(),
                &[IoSlice::new(&frame[sent..])],
                &[],
                MsgFlags::MSG_NOSIGNAL,
                None,
            ) {
                Err(Errno::EINTR) => continue,
                result => break result?,
            }
        };
    }
    Ok(())
}

/// Reads one frame. `Ok(None)` is a clean end of stream before a frame.
/// An interrupted read returns `ErrorKind::Interrupted` so that the caller
/// can check for a stop signal.
pub fn receive<T: DeserializeOwned>(stream: &UnixStream) -> io::Result<Option<(T, Vec<OwnedFd>)>> {
    let mut fds = Vec::new();
    let mut header = [0; 4];
    if !read(stream, &mut header, &mut fds)? {
        return Ok(None);
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame is too large",
        ));
    }
    let mut body = vec![0; length];
    if !read(stream, &mut body, &mut fds)? {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let message = serde_json::from_slice(&body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some((message, fds)))
}

fn read(stream: &UnixStream, buffer: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buffer.len() {
        let mut space = nix::cmsg_space!([RawFd; MAX_FDS]);
        let mut iov = [IoSliceMut::new(&mut buffer[filled..])];
        let message = match recvmsg::<UnixAddr>(
            stream.as_raw_fd(),
            &mut iov,
            Some(&mut space),
            MsgFlags::MSG_CMSG_CLOEXEC,
        ) {
            Ok(message) => message,
            Err(Errno::EINTR) => return Err(io::ErrorKind::Interrupted.into()),
            Err(error) => return Err(error.into()),
        };
        for control in message.cmsgs()? {
            if let ControlMessageOwned::ScmRights(received) = control {
                for fd in received {
                    // SAFETY: the kernel gave this process a new descriptor
                    // in SCM_RIGHTS, and nothing else owns it.
                    fds.push(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        }
        if message.bytes == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        filled += message.bytes;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Response;
    use std::io::Read;

    #[test]
    fn frames_carry_descriptors() {
        let (left, right) = UnixStream::pair().unwrap();
        let (pipe_read, mut pipe_write) = UnixStream::pair().unwrap();
        send(&left, &Response::Socket, &[pipe_read.as_raw_fd()]).unwrap();
        drop(pipe_read);
        let (message, fds): (Response, _) = receive(&right).unwrap().unwrap();
        assert_eq!(message, Response::Socket);
        assert_eq!(fds.len(), 1);
        let mut received = UnixStream::from(fds.into_iter().next().unwrap());
        std::io::Write::write_all(&mut pipe_write, b"ok").unwrap();
        let mut text = [0; 2];
        received.read_exact(&mut text).unwrap();
        assert_eq!(&text, b"ok");
        send(
            &left,
            &Response::Failed {
                message: "x".into(),
            },
            &[],
        )
        .unwrap();
        let (message, fds): (Response, _) = receive(&right).unwrap().unwrap();
        assert_eq!(
            message,
            Response::Failed {
                message: "x".into()
            }
        );
        assert!(fds.is_empty());
        drop(left);
        assert!(receive::<Response>(&right).unwrap().is_none());
    }
}
