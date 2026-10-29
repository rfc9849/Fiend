use std::time::Instant;

use fiend_core::net::ProtectSocket;

use crate::{AppError, ProfileConfig, profile};

pub async fn measure_latency(
    profile: &ProfileConfig,
    protect: &ProtectSocket,
) -> Result<u32, AppError> {
    let config = profile::client_config(profile)?;
    tokio::time::timeout(config.handshake_timeout, async {
        let start = Instant::now();
        let stream = fiend_core::net::dial(&config.server, protect)
            .await
            .map_err(AppError::engine)?;
        let _session = fiend_core::transport::connect(stream, &config.secret, &config.template)
            .await
            .map_err(AppError::engine)?;
        Ok(u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX))
    })
    .await
    .map_err(AppError::engine)?
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
        profile.server = address.to_string();
        let hook: ProtectSocket = Arc::new(|_| Ok(()));
        assert!(measure_latency(&profile, &hook).await.unwrap() < 10000);
    }
}
