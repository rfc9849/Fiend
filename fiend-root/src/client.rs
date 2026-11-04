//! App side: start the helper through `su` and receive the sockets.

use std::io::{self, BufRead, BufReader};
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};

use crate::frame;
use crate::protocol::{Policy, Request, Response};

const ACCEPT_POLL: Duration = Duration::from_millis(50);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const EXIT_WAIT: Duration = Duration::from_secs(3);

pub struct Helper {
    protocol: u32,
    stream: Mutex<UnixStream>,
    child: Mutex<Child>,
    path: PathBuf,
}

pub struct Started {
    pub helper: Helper,
    pub tcp: Vec<TcpListener>,
    pub udp: Vec<UdpSocket>,
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn failed(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

impl Helper {
    /// Starts `helper attach <socket>` as root and asks it to capture the
    /// apps of `policy`. `wait` covers the time to answer a root prompt.
    pub fn launch(
        helper: &str,
        socket: &Path,
        policy: &Policy,
        wait: Duration,
    ) -> io::Result<Started> {
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket)?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let script = format!(
            "{} attach {}",
            quote(helper),
            quote(&socket.to_string_lossy())
        );
        let mut child = Command::new("su")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| failed(format!("Cannot run su: {error}. Root access is required")))?;
        if let Some(stderr) = child.stderr.take() {
            std::thread::Builder::new()
                .name("fiend-root-log".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        tracing::info!(target: "fiend_root", "{line}");
                    }
                })?;
        }
        let deadline = Instant::now() + wait;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(stop(child, error)),
            }
            if let Some(status) = child.try_wait()? {
                let _ = std::fs::remove_file(socket);
                return Err(failed(format!(
                    "The root helper stopped ({status}). Give root access to Fiend and install the Fiend module"
                )));
            }
            if Instant::now() >= deadline {
                let _ = std::fs::remove_file(socket);
                return Err(stop(child, failed("Root access was not given in time")));
            }
            std::thread::sleep(ACCEPT_POLL);
        };
        drop(listener);
        let _ = std::fs::remove_file(socket);
        stream.set_nonblocking(false)?;
        let credentials = getsockopt(&stream, PeerCredentials)?;
        if credentials.uid() != 0 {
            return Err(stop(child, failed("The root helper does not run as root")));
        }
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        let mut helper = Helper {
            protocol: 0,
            stream: Mutex::new(stream),
            child: Mutex::new(child),
            path: socket.to_owned(),
        };
        let (response, fds) = helper.request(&Request::Start {
            policy: policy.clone(),
        })?;
        let Response::Ready { tcp, udp, protocol } = response else {
            return Err(failed("The root helper sent an unexpected answer"));
        };
        helper.protocol = protocol;
        if fds.len() != tcp + udp {
            return Err(failed("The root helper sent the wrong number of sockets"));
        }
        let mut fds = fds.into_iter();
        let tcp = fds.by_ref().take(tcp).map(TcpListener::from).collect();
        let udp = fds.map(UdpSocket::from).collect();
        Ok(Started { helper, tcp, udp })
    }

    fn request(&self, request: &Request) -> io::Result<(Response, Vec<OwnedFd>)> {
        let stream = self
            .stream
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        frame::send(&stream, request, &[])?;
        let (response, fds) = loop {
            match frame::receive(&stream) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        }
        .ok_or_else(|| failed("The root helper stopped"))?;
        if let Response::Failed { message } = response {
            return Err(failed(message));
        }
        Ok((response, fds))
    }

    /// True when the helper process is gone. Its rules are gone too, so the
    /// apps no longer go through the tunnel.
    pub fn exited(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|error| error.into_inner());
        !matches!(child.try_wait(), Ok(None))
    }

    /// Tells the helper whether the network is metered, so that the Data
    /// Saver rules of Android apply to the captured apps.
    /// A helper from an older module does not know the request and does
    /// nothing.
    pub fn set_metered(&self, metered: bool) -> io::Result<()> {
        if self.protocol < 1 {
            return Ok(());
        }
        match self.request(&Request::Metered { metered })?.0 {
            Response::Done => Ok(()),
            _ => Err(failed("The root helper sent an unexpected answer")),
        }
    }

    pub fn reply_socket(&self, address: SocketAddr) -> io::Result<UdpSocket> {
        let (response, fds) = self.request(&Request::Reply { address })?;
        match (response, fds.into_iter().next()) {
            (Response::Socket, Some(fd)) => Ok(UdpSocket::from(fd)),
            _ => Err(failed("The root helper sent no socket")),
        }
    }
}

fn stop(mut child: Child, error: io::Error) -> io::Error {
    let _ = child.kill();
    let _ = child.wait();
    error
}

impl Drop for Helper {
    fn drop(&mut self) {
        let stream = self
            .stream
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let child = self
            .child
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let deadline = Instant::now() + EXIT_WAIT;
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                let _ = std::fs::remove_file(&self.path);
                return;
            }
            std::thread::sleep(ACCEPT_POLL);
        }
        tracing::warn!("the root helper did not stop in time");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn shell_words_are_quoted() {
        assert_eq!(
            quote("/data/adb/modules/fiend/fiend-root"),
            "'/data/adb/modules/fiend/fiend-root'"
        );
        assert_eq!(quote("a'b"), r"'a'\''b'");
    }
}
