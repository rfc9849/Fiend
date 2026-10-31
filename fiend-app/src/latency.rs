use std::io;
use std::time::{Duration, Instant};

use fiend_core::net::ProtectSocket;

use crate::{AppError, ProfileConfig, profile};

pub async fn measure_latency(
    profile: &ProfileConfig,
    protect: &ProtectSocket,
) -> Result<u32, AppError> {
    let config = profile::client_config(profile)?;
    let mut probes = tokio::task::JoinSet::new();
    for server in config.servers.clone() {
        let config = config.clone();
        let protect = protect.clone();
        probes.spawn(async move {
            tokio::time::timeout(
                config.handshake_timeout,
                measure_server(&server, &config, &protect),
            )
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "server handshake timed out"))?
        });
    }
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "server has no addresses");
    while let Some(result) = probes.join_next().await {
        match result.map_err(io::Error::other) {
            Ok(Ok(rtt)) => return Ok(millis(rtt)),
            Ok(Err(error)) | Err(error) => last = error,
        }
    }
    Err(AppError::engine(last))
}

async fn measure_server(
    server: &str,
    config: &fiend_core::client::ClientConfig,
    protect: &ProtectSocket,
) -> io::Result<Duration> {
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "server has no addresses");
    for address in tokio::net::lookup_host(server).await? {
        let start = Instant::now();
        let stream = match fiend_core::net::dial(&address.to_string(), protect).await {
            Ok(stream) => stream,
            Err(error) => {
                last = error;
                continue;
            }
        };
        let rtt = start.elapsed();
        fiend_core::transport::connect(stream, &config.secret, &config.template)
            .await
            .map_err(io::Error::other)?;
        return Ok(rtt);
    }
    Err(last)
}

fn millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros().div_ceil(1000)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn latency_uses_a_real_fiend_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = fiend_core::server::Server::new(fiend_core::server::ServerConfig::new(
            [0x42; 32],
            "127.0.0.1",
            443,
        ));
        tokio::spawn(async move {
            server.serve(listener).await.unwrap();
        });
        let mut profile = profile::tests::profile();
        profile.servers = vec!["127.0.0.1:1".into(), address.to_string()];
        let hook: ProtectSocket = Arc::new(|_| Ok(()));
        assert!(measure_latency(&profile, &hook).await.unwrap() < 10000);
    }
}
