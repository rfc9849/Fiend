use std::path::PathBuf;

pub const SOCKET_ENV: &str = "FIEND_SOCKET";

#[cfg(windows)]
pub const PIPE_NAME: &str = r"\\.\pipe\fiend-daemon";

pub fn default_socket() -> PathBuf {
    if let Some(path) = std::env::var_os(SOCKET_ENV).filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    PathBuf::from("/var/run/fiend/daemon.sock")
}
