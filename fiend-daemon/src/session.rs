use std::net::SocketAddr;
use std::time::Duration;

use fiend_core::client::Client;
use fiend_core::net::ProtectSocket;
use fiend_ipc::{Profile, Tuning};
use fiend_tun::rules::RuleFile;

use crate::machine::EngineSettings;

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn app_profile(profile: &Profile) -> fiend_app::ProfileConfig {
    fiend_app::ProfileConfig {
        name: profile.name.clone(),
        servers: profile.servers.clone(),
        secret: profile.secret.clone(),
        template_json: profile.template_json.clone(),
    }
}

/// Resolve the server names one time, before routes change. A later lookup
/// would go through the tunnel DNS and return a fake address.
pub(crate) async fn resolve_servers(servers: &[String]) -> Result<Vec<SocketAddr>, String> {
    let mut resolved = Vec::new();
    let mut last = "The profile has no server".to_owned();
    for server in servers {
        match resolve_server(server).await {
            Ok(address) if !resolved.contains(&address) => resolved.push(address),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%server, %error, "cannot resolve a profile server");
                last = error;
            }
        }
    }
    if resolved.is_empty() {
        return Err(last);
    }
    Ok(resolved)
}

async fn resolve_server(server: &str) -> Result<SocketAddr, String> {
    if let Ok(address) = server.parse::<SocketAddr>() {
        return Ok(address);
    }
    let lookup = tokio::net::lookup_host(server.to_owned());
    let mut addresses = tokio::time::timeout(RESOLVE_TIMEOUT, lookup)
        .await
        .map_err(|_| "Server name lookup timed out".to_owned())?
        .map_err(|error| format!("Cannot resolve the server name: {error}"))?;
    addresses
        .next()
        .ok_or_else(|| "The server name has no address".to_owned())
}

pub(crate) fn build_client(
    profile: &Profile,
    servers: &[SocketAddr],
    protect: ProtectSocket,
) -> Result<Client, String> {
    let mut config = fiend_app::client_config(&app_profile(profile)).map_err(|e| e.to_string())?;
    config.servers = servers.iter().map(ToString::to_string).collect();
    config.handshake_timeout = Duration::from_millis(profile.tuning.handshake_timeout_ms.into());
    config.protect_socket = protect;
    let config = fiend_core::config::build_client(config).map_err(|e| e.to_string())?;
    Ok(Client::new(config))
}

pub(crate) fn tun_options(
    tuning: &Tuning,
    engine: &EngineSettings,
    rule_files: Vec<RuleFile>,
    protect: ProtectSocket,
) -> fiend_tun::TunOptions {
    fiend_tun::TunOptions {
        mtu: engine.mtu,
        ipv6: engine.ipv6,
        bypass_lan: engine.bypass_lan,
        default_route: match engine.default_route {
            fiend_ipc::DefaultRoute::Tunnel => fiend_tun::rules::DefaultRoute::Tunnel,
            fiend_ipc::DefaultRoute::Direct => fiend_tun::rules::DefaultRoute::Direct,
        },
        direct_rules: engine.direct_rules.clone(),
        block_rules: engine.block_rules.clone(),
        proxy_rules: engine.proxy_rules.clone(),
        dpi_rules: engine.dpi_rules.clone(),
        rule_files,
        direct_resolver: engine.direct_resolver.clone(),
        max_tcp_flows: tuning.max_connections as usize,
        max_udp_flows: tuning.max_udp_flows as usize,
        udp_idle: Duration::from_millis(tuning.udp_idle_ms.into()),
        fake_dns: Some(fiend_tun::shared_fake_dns()),
        protect_socket: protect,
        ..fiend_tun::TunOptions::default()
    }
}
