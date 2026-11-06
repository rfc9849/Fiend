use std::net::IpAddr;
use std::time::Duration;

const NLMSG_ERROR: u16 = 2;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_CREATE: u16 = 0x400;
const NLA_F_NESTED: u16 = 0x8000;
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFNL_MSG_BATCH_BEGIN: u16 = 0x10;
const NFNL_MSG_BATCH_END: u16 = 0x11;
const NFT_MSG_NEWSETELEM: u16 = 12;
const NFT_MSG_DESTROYSETELEM: u16 = 30;
const NFPROTO_INET: u8 = 1;
const NFTA_SET_ELEM_LIST_TABLE: u16 = 1;
const NFTA_SET_ELEM_LIST_SET: u16 = 2;
const NFTA_SET_ELEM_LIST_ELEMENTS: u16 = 3;
const NFTA_LIST_ELEM: u16 = 1;
const NFTA_SET_ELEM_KEY: u16 = 1;
const NFTA_SET_ELEM_TIMEOUT: u16 = 4;
const NFTA_DATA_VALUE: u16 = 1;

fn align(length: usize) -> usize {
    (length + 3) & !3
}

struct Attributes(Vec<u8>);

impl Attributes {
    fn put(&mut self, kind: u16, payload: &[u8]) {
        let length = 4 + payload.len();
        self.0.extend_from_slice(&(length as u16).to_ne_bytes());
        self.0.extend_from_slice(&kind.to_ne_bytes());
        self.0.extend_from_slice(payload);
        self.0.resize(align(self.0.len()), 0);
    }

    fn string(&mut self, kind: u16, value: &str) {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        self.put(kind, &bytes);
    }

    fn nest(&mut self, kind: u16, build: impl FnOnce(&mut Attributes)) {
        let mut inner = Attributes(Vec::new());
        build(&mut inner);
        self.put(kind | NLA_F_NESTED, &inner.0);
    }
}

fn message(kind: u16, flags: u16, seq: u32, family: u8, subsystem: u16, body: &[u8]) -> Vec<u8> {
    let length = 16 + 4 + body.len();
    let mut bytes = Vec::with_capacity(align(length));
    bytes.extend_from_slice(&(length as u32).to_ne_bytes());
    bytes.extend_from_slice(&kind.to_ne_bytes());
    bytes.extend_from_slice(&flags.to_ne_bytes());
    bytes.extend_from_slice(&seq.to_ne_bytes());
    bytes.extend_from_slice(&0u32.to_ne_bytes());
    bytes.push(family);
    bytes.push(0);
    bytes.extend_from_slice(&subsystem.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes.resize(align(bytes.len()), 0);
    bytes
}

fn key(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(ip) => ip.octets().to_vec(),
        IpAddr::V6(ip) => ip.octets().to_vec(),
    }
}

fn elements(table: &str, set: &str, addresses: &[IpAddr], timeout: Option<Duration>) -> Vec<u8> {
    let mut body = Attributes(Vec::new());
    body.string(NFTA_SET_ELEM_LIST_TABLE, table);
    body.string(NFTA_SET_ELEM_LIST_SET, set);
    body.nest(NFTA_SET_ELEM_LIST_ELEMENTS, |list| {
        for ip in addresses {
            list.nest(NFTA_LIST_ELEM, |element| {
                element.nest(NFTA_SET_ELEM_KEY, |data| {
                    data.put(NFTA_DATA_VALUE, &key(*ip))
                });
                if let Some(timeout) = timeout {
                    let millis = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
                    element.put(NFTA_SET_ELEM_TIMEOUT, &millis.to_be_bytes());
                }
            });
        }
    });
    body.0
}

pub struct Update<'a> {
    pub set: &'a str,
    pub addresses: &'a [IpAddr],
}

pub fn batch(
    table: &str,
    updates: &[Update<'_>],
    timeout: Duration,
    destroy: bool,
    first_seq: u32,
) -> (Vec<u8>, Vec<u32>) {
    let mut seq = first_seq;
    let mut next = || {
        let current = seq;
        seq = seq.wrapping_add(1);
        current
    };
    let mut bytes = message(
        NFNL_MSG_BATCH_BEGIN,
        NLM_F_REQUEST,
        next(),
        0,
        NFNL_SUBSYS_NFTABLES,
        &[],
    );
    let mut acks = Vec::new();
    for update in updates.iter().filter(|update| !update.addresses.is_empty()) {
        if destroy {
            let seq = next();
            acks.push(seq);
            bytes.extend(message(
                (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_DESTROYSETELEM,
                NLM_F_REQUEST | NLM_F_ACK,
                seq,
                NFPROTO_INET,
                0,
                &elements(table, update.set, update.addresses, None),
            ));
        }
        let seq = next();
        acks.push(seq);
        bytes.extend(message(
            (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_NEWSETELEM,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
            seq,
            NFPROTO_INET,
            0,
            &elements(table, update.set, update.addresses, Some(timeout)),
        ));
    }
    bytes.extend(message(
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        next(),
        0,
        NFNL_SUBSYS_NFTABLES,
        &[],
    ));
    (bytes, acks)
}

pub fn replies(bytes: &[u8]) -> Vec<(u32, i32)> {
    let mut found = Vec::new();
    let mut rest = bytes;
    while rest.len() >= 16 {
        let length = u32::from_ne_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if length < 16 || length > rest.len() {
            break;
        }
        let kind = u16::from_ne_bytes([rest[4], rest[5]]);
        if kind == NLMSG_ERROR && length >= 20 {
            let error = i32::from_ne_bytes([rest[16], rest[17], rest[18], rest[19]]);
            let seq = if length >= 36 {
                u32::from_ne_bytes([rest[28], rest[29], rest[30], rest[31]])
            } else {
                u32::from_ne_bytes([rest[8], rest[9], rest[10], rest[11]])
            };
            found.push((seq, -error));
        }
        rest = &rest[align(length).min(rest.len())..];
    }
    found
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub use socket::SetWriter;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod socket {
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use nix::sys::socket::{
        AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, send,
        setsockopt, socket, sockopt,
    };
    use nix::sys::time::{TimeVal, TimeValLike};

    use super::{Update, batch, replies};

    const WAIT: Duration = Duration::from_secs(2);

    struct Channel {
        socket: OwnedFd,
        seq: u32,
    }

    pub struct SetWriter {
        table: String,
        channel: Mutex<Channel>,
        destroy: AtomicBool,
    }

    impl SetWriter {
        pub fn open(table: &str) -> io::Result<Self> {
            let socket = socket(
                AddressFamily::Netlink,
                SockType::Raw,
                SockFlag::SOCK_CLOEXEC,
                SockProtocol::NetlinkNetFilter,
            )?;
            bind(socket.as_raw_fd(), &NetlinkAddr::new(0, 0))?;
            setsockopt(
                &socket,
                sockopt::ReceiveTimeout,
                &TimeVal::seconds(WAIT.as_secs() as i64),
            )?;
            setsockopt(&socket, sockopt::RcvBuf, &(256 << 10))?;
            Ok(Self {
                table: table.to_owned(),
                channel: Mutex::new(Channel { socket, seq: 1 }),
                destroy: AtomicBool::new(true),
            })
        }

        pub fn add(&self, updates: &[Update<'_>], timeout: Duration) -> io::Result<()> {
            if updates.iter().all(|update| update.addresses.is_empty()) {
                return Ok(());
            }
            let destroy = self.destroy.load(Ordering::Relaxed);
            match self.send(updates, timeout, destroy) {
                Err(error)
                    if destroy
                        && matches!(
                            error.raw_os_error(),
                            Some(nix::libc::EOPNOTSUPP | nix::libc::EINVAL)
                        ) =>
                {
                    tracing::info!("this kernel cannot renew set elements, adding only");
                    self.destroy.store(false, Ordering::Relaxed);
                    self.send(updates, timeout, false)
                }
                result => result,
            }
        }

        fn send(&self, updates: &[Update<'_>], timeout: Duration, destroy: bool) -> io::Result<()> {
            let mut channel = self
                .channel
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let first = channel.seq;
            let (bytes, mut waiting) = batch(&self.table, updates, timeout, destroy, first);
            channel.seq = first.wrapping_add(waiting.len() as u32 + 2);
            let fd = channel.socket.as_raw_fd();
            send(fd, &bytes, MsgFlags::empty())?;
            let mut buffer = vec![0u8; 16 << 10];
            while !waiting.is_empty() {
                let size = recv(fd, &mut buffer, MsgFlags::empty())?;
                for (seq, errno) in replies(&buffer[..size]) {
                    if !waiting.contains(&seq) {
                        continue;
                    }
                    if errno != 0 {
                        return Err(io::Error::from_raw_os_error(errno));
                    }
                    waiting.retain(|item| *item != seq);
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_renews_then_adds_each_family() {
        let v4 = ["192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()];
        let v6: [IpAddr; 0] = [];
        let (bytes, acks) = batch(
            "fiend",
            &[
                Update {
                    set: "dns4",
                    addresses: &v4,
                },
                Update {
                    set: "dns6",
                    addresses: &v6,
                },
            ],
            Duration::from_secs(3600),
            true,
            7,
        );
        assert_eq!(acks, [8, 9]);
        assert_eq!(bytes.len() % 4, 0);
        let mut offsets = Vec::new();
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            let length = u32::from_ne_bytes(rest[..4].try_into().unwrap()) as usize;
            let kind = u16::from_ne_bytes(rest[4..6].try_into().unwrap());
            let seq = u32::from_ne_bytes(rest[8..12].try_into().unwrap());
            offsets.push((
                kind,
                seq,
                rest[16],
                u16::from_be_bytes([rest[18], rest[19]]),
            ));
            rest = &rest[align(length)..];
        }
        assert_eq!(
            offsets,
            [
                (0x10, 7, 0, 10),
                (0x0a00 | 30, 8, 1, 0),
                (0x0a00 | 12, 9, 1, 0),
                (0x11, 10, 0, 10),
            ]
        );
        let timeout = 3_600_000u64.to_be_bytes();
        assert!(bytes.windows(8).any(|window| window == timeout));
        assert!(bytes.windows(4).any(|window| window == [192, 0, 2, 2]));
        assert!(bytes.windows(6).any(|window| window == b"fiend\0"));
        let (plain, acks) = batch(
            "fiend",
            &[Update {
                set: "dns4",
                addresses: &v4,
            }],
            Duration::from_secs(1),
            false,
            1,
        );
        assert_eq!(acks, [2]);
        assert!(plain.len() < bytes.len());
    }

    #[test]
    fn error_messages_give_the_sequence_and_errno() {
        let mut bytes = Vec::new();
        for (seq, error) in [(5u32, 0i32), (6, -2)] {
            bytes.extend_from_slice(&36u32.to_ne_bytes());
            bytes.extend_from_slice(&NLMSG_ERROR.to_ne_bytes());
            bytes.extend_from_slice(&0u16.to_ne_bytes());
            bytes.extend_from_slice(&seq.to_ne_bytes());
            bytes.extend_from_slice(&0u32.to_ne_bytes());
            bytes.extend_from_slice(&error.to_ne_bytes());
            bytes.extend_from_slice(&[0; 8]);
            bytes.extend_from_slice(&seq.to_ne_bytes());
            bytes.extend_from_slice(&0u32.to_ne_bytes());
        }
        assert_eq!(replies(&bytes), [(5, 0), (6, 2)]);
        assert!(replies(&bytes[..10]).is_empty());
    }
}
