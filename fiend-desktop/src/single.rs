use std::io::{BufRead, BufReader, Write};
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

#[cfg(not(unix))]
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn messages_round_trip_and_reject_other_schemes() {
        for message in [Message::Show, Message::Open("fiend://v1/abc".into())] {
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
        let link = Message::Open("fiend://v1/xyz".into());
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
