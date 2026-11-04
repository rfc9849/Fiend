//! Root helper for the Android root mode. The app keeps the engine; the
//! helper adds the TPROXY rules for the selected apps and makes the sockets
//! that need root, then gives them to the app over a Unix socket.

pub mod protocol;
pub mod rules;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod bpf;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod client;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod frame;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod sockets;
