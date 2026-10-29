#![forbid(unsafe_code)]

mod client;
mod frame;
mod message;
pub mod path;
mod types;

pub use client::{Client, ClientError};
pub use frame::{
    FrameError, HANDSHAKE_FRAME_BYTES, MAX_FRAME_BYTES, decode_frame, encode_frame, read_frame,
    write_frame,
};
pub use message::{
    ClientMessage, Event, Hello, HelloAck, PROTOCOL_MAJOR, PROTOCOL_MINOR, Reply, Request,
    ServerMessage,
};
pub use types::{
    BackupData, ConnectStep, DEFAULT_MTU, ErrorCode, IpcError, LogEntry, LogLevel, MAX_MTU,
    MIN_MTU, Profile, ProfileInput, RuleKind, RuleStats, Settings, Stats, Status, Subscription,
    Tuning, TunnelState,
};
