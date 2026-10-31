#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(any(unix, windows))]
use std::path::Path;

use futures::channel::mpsc::{UnboundedReceiver, unbounded};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Show,
    Open(String),
}

impl Message {
    pub fn encode(&self) -> String {
        match self {
            Self::Show => "show".to_owned(),
            Self::Open(url) => format!("open {url}"),
        }
    }

    pub fn decode(line: &str) -> Option<Self> {
        let line = line.trim();
        if line == "show" {
            Some(Self::Show)
        } else {
            line.strip_prefix("open ")
                .filter(|url| url.starts_with("fiend://") && url.len() <= 65_536)
                .map(|url| Self::Open(url.to_owned()))
        }
    }
}

pub enum Role {
    Primary(UnboundedReceiver<Message>),
    Secondary,
}

#[cfg(unix)]
pub fn acquire(path: &Path, message: &Message) -> Role {
    use std::os::unix::net::{UnixListener, UnixStream};
    if let Ok(mut stream) = UnixStream::connect(path) {
        let _ = writeln!(stream, "{}", message.encode());
        return Role::Secondary;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(path);
    let (sender, receiver) = unbounded();
    match UnixListener::bind(path) {
        Ok(listener) => {
            let _ = sender.unbounded_send(message.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut line = String::new();
                    let mut reader = BufReader::new(stream.take_limited());
                    if reader.read_line(&mut line).is_ok()
                        && let Some(message) = Message::decode(&line)
                        && sender.unbounded_send(message).is_err()
                    {
                        break;
                    }
                }
            });
            Role::Primary(receiver)
        }
        Err(_) => Role::Primary(receiver),
    }
}

#[cfg(windows)]
fn instance_id(path: &Path) -> std::io::Result<String> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the config path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let file = parent.join("instance.id");
    let valid = |text: &str| text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if let Ok(text) = std::fs::read_to_string(&file) {
        let text = text.trim();
        if valid(text) {
            return Ok(text.to_owned());
        }
    }
    let bytes: [u8; 16] = rand::random();
    let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
    {
        Ok(mut file) => file.write_all(id.as_bytes())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            for _ in 0..50 {
                if let Ok(text) = std::fs::read_to_string(&file) {
                    let text = text.trim();
                    if valid(text) {
                        return Ok(text.to_owned());
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            std::fs::write(&file, &id)?;
        }
        Err(error) => return Err(error),
    }
    Ok(id)
}

#[cfg(windows)]
fn pipe_name(path: &Path) -> std::ffi::OsString {
    let id = instance_id(path).unwrap_or_else(|_| "default".to_owned());
    format!(r"\\.\pipe\fiend-desktop-{id}").into()
}

#[cfg(windows)]
async fn forward_message(name: &std::ffi::OsStr, message: &Message) -> bool {
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::ClientOptions;

    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..80 {
        match ClientOptions::new().open(name) {
            Ok(mut client) => {
                let line = format!("{}\n", message.encode());
                return client.write_all(line.as_bytes()).await.is_ok()
                    && client.flush().await.is_ok();
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(_) => return false,
        }
    }
    false
}

#[cfg(windows)]
async fn serve_messages(
    name: std::ffi::OsString,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    sender: futures::channel::mpsc::UnboundedSender<Message>,
) {
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    loop {
        if server.connect().await.is_err() {
            return;
        }
        let Ok(next) = ServerOptions::new().max_instances(2).create(&name) else {
            return;
        };
        let mut line = String::new();
        let mut reader = BufReader::new(server.take(70_000));
        if tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .is_ok()
            && let Some(message) = Message::decode(&line)
            && sender.unbounded_send(message).is_err()
        {
            return;
        }
        server = next;
    }
}

#[cfg(windows)]
pub fn acquire(path: &Path, message: &Message) -> Role {
    use tokio::net::windows::named_pipe::ServerOptions;

    let name = pipe_name(path);
    let (sender, receiver) = unbounded();
    let _ = sender.unbounded_send(message.clone());
    let (status, status_receiver) = std::sync::mpsc::channel();
    let thread_name = name.clone();
    let thread_message = message.clone();
    let handle = std::thread::Builder::new()
        .name("fiend-single".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                let _ = status.send(false);
                return;
            };
            runtime.block_on(async move {
                match ServerOptions::new()
                    .first_pipe_instance(true)
                    .max_instances(2)
                    .create(&thread_name)
                {
                    Ok(server) => {
                        let _ = status.send(true);
                        serve_messages(thread_name, server, sender).await;
                    }
                    Err(_) => {
                        let _ = status.send(false);
                        let _ = forward_message(&thread_name, &thread_message).await;
                    }
                }
            });
        })
        .expect("single instance thread");
    match status_receiver.recv() {
        Ok(true) => {
            drop(handle);
            Role::Primary(receiver)
        }
        _ => {
            let _ = handle.join();
            Role::Secondary
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub fn acquire(_: &Path, message: &Message) -> Role {
    let (sender, receiver) = unbounded();
    let _ = sender.unbounded_send(message.clone());
    std::mem::forget(sender);
    Role::Primary(receiver)
}

#[cfg(unix)]
trait TakeLimited {
    fn take_limited(self) -> std::io::Take<Self>
    where
        Self: Sized;
}

#[cfg(unix)]
impl<T: std::io::Read> TakeLimited for T {
    fn take_limited(self) -> std::io::Take<Self> {
        self.take(70_000)
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn the_instance_id_is_stable_and_hex() {
        let dir = std::env::temp_dir().join(format!("fiend-single-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("desktop.sock");
        let first = instance_id(&path).unwrap();
        let second = instance_id(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn messages_round_trip_and_reject_other_schemes() {
        for message in [Message::Show, Message::Open("fiend://abc".into())] {
            assert_eq!(Message::decode(&message.encode()), Some(message));
        }
        assert_eq!(Message::decode("open https://example.com"), None);
        assert_eq!(Message::decode("quit"), None);
    }

    #[test]
    fn second_instance_forwards_its_link_to_the_first() {
        let dir = std::env::temp_dir().join(format!("fiend-single-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("desktop.sock");
        let Role::Primary(mut receiver) = acquire(&path, &Message::Show) else {
            panic!("first instance is not primary");
        };
        let link = Message::Open("fiend://xyz".into());
        assert!(matches!(acquire(&path, &link), Role::Secondary));
        let got = futures::executor::block_on(async {
            let first = receiver.next().await;
            let second = receiver.next().await;
            (first, second)
        });
        assert_eq!(got, (Some(Message::Show), Some(link)));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
