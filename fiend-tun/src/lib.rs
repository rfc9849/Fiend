#![forbid(unsafe_code)]

pub mod dns;
mod engine;
pub mod packet;
mod resolve;
pub use fiend_rules as rules;
pub use fiend_rules::srs;
pub use engine::{PacketIo, TunEngine, TunOptions, TunnelStats};
pub use resolve::parse_server as parse_resolver;
