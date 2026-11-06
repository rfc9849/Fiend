#![forbid(unsafe_code)]

mod desync;
pub mod dns;
mod engine;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod gateway;
pub mod icmp;
pub mod packet;
mod resolve;
#[cfg(any(target_os = "linux", target_os = "android", test))]
mod sniff;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod transparent;
pub use engine::{PacketIo, TunEngine, TunOptions, TunnelStats, shared_fake_dns};
pub use fiend_rules as rules;
pub use fiend_rules::srs;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use gateway::{CAPTURED_TTL, CaptureSet, GatewayDns, Upstream};
pub use resolve::parse_server as parse_resolver;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use transparent::{ReplySockets, TransparentSockets};
