mod provision;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use fiend_core::config;
use fiend_core::fingerprint::parse_client_hello;
use fiend_core::net;
use fiend_core::server::Server;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("template") => run_template(&args[1..]),
        Some("prepare") => provision::prepare(&args[1..]),
        Some("generate-secret") if args.len() == 1 => {
            println!("{}", hex::encode(rand::random::<[u8; 32]>()));
            Ok(())
        }
        Some("check") => run_check(&args[1..]),
        Some("node") => run_node(&args[1..]).await,
        Some("--version") => {
            println!("fiend-core {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("-h" | "--help" | "help") => {
            usage();
            Ok(())
        }
        Some(command) => Err(format!("unknown command: {command}")),
        None => {
            usage();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    println!("Usage: fiend-core <command> [args]");
    println!("  template <input.bin>");
    println!("  node <config.toml>");
    println!("  check <config.toml> [state.json]");
    println!("  generate-secret");
    println!(
        "  prepare <directory> [--port <port>] [--listen <address>] [--proxy-protocol <true|false>]"
    );
    println!(
        "          [--fallback <host>=<address>[,...]] [--fallback-proxy-protocol <true|false>]"
    );
}

fn run_template(args: &[String]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: fiend-core template <input.bin>".to_owned());
    }
    extract_bin(&args[0])
}

fn run_check(args: &[String]) -> Result<(), String> {
    let (path, state_path) = match args {
        [path] => (path, None),
        [path, state] => (path, Some(state)),
        _ => return Err("Usage: fiend-core check <config.toml> [state.json]".into()),
    };
    let settings = config::load_server(path).map_err(|error| format!("config: {error}"))?;
    if let Some(path) = state_path {
        let raw = fs::read(path).map_err(|_| "cannot read node state")?;
        if raw.len() > fiend_core::node::MAX_STATE_BYTES {
            return Err("node state is too large".into());
        }
        let state: fiend_core::node::DesiredState =
            serde_json::from_slice(&raw).map_err(|_| "invalid node state")?;
        let secret = fs::read_to_string(&settings.node.control_secret_file)
            .map_err(|_| "cannot read control secret")?;
        let secret = fiend_core::node::decode_secret(&secret).map_err(|error| error.to_string())?;
        state.validate(&secret).map_err(|error| error.to_string())?;
    }
    println!("node config is valid");
    Ok(())
}

#[cfg(unix)]
async fn run_node(args: &[String]) -> Result<(), String> {
    use fiend_core::node::{DesiredState, Node, control, decode_secret};
    let path = config_path(args, "node")?;
    let settings = config::load_server(path).map_err(|error| format!("config: {error}"))?;
    let secret = fs::read_to_string(&settings.node.control_secret_file)
        .map_err(|_| "cannot read control secret")?;
    let secret = decode_secret(&secret).map_err(|error| error.to_string())?;
    let initial = DesiredState::initial(&settings.config);
    let node = Node::load_with(
        &settings.node.state_file,
        secret.to_vec(),
        initial,
        settings.node.wireproxy.clone(),
    )
    .map_err(|error| format!("node state: {error}"))?;
    let listener =
        net::bind_listener(settings.listen).map_err(|error| format!("listen: {error}"))?;
    node.set_listen_port(settings.listen.port());
    node.set_public_port(settings.public_port);
    node.set_fallbacks(settings.fallbacks.clone());
    let proxy_protocol = settings.config.proxy_protocol;
    if proxy_protocol && !settings.listen.ip().is_loopback() {
        tracing::warn!(listen = %settings.listen, "PROXY protocol is on: make sure that only your proxy can reach the listen address");
    }
    for fallback in &settings.fallbacks {
        tracing::info!(host = %fallback.host, address = %fallback.origin, proxy_protocol = fallback.origin.proxy_protocol, "fallback");
    }
    {
        let node = node.clone();
        tokio::task::spawn_blocking(move || node.health());
    }
    let server = Server::managed(settings.config, node.clone());
    {
        let node = node.clone();
        tokio::spawn(async move { node.start_warp().await });
    }
    let warp = node.warp().clone();
    tracing::info!(listen = %listener.local_addr().map_err(|error| error.to_string())?, public_port = settings.public_port, proxy_protocol, "node started");
    tokio::select! {
        result = server.serve(listener) => result.map_err(|error| format!("node: {error}"))?,
        result = control::serve_unix(&settings.node.control_socket, node.clone()) => result.map_err(|error| format!("control socket: {error}"))?,
        _ = node.maintain_usage() => {},
        _ = warp.maintain() => {},
        _ = node.wait_for_restart() => tracing::info!("restart requested"),
        _ = shutdown_signal() => tracing::info!("shutdown signal received"),
    }
    node.persist_usage()
        .await
        .map_err(|error| format!("save usage: {error}"))?;
    tracing::info!("node stopped");
    Ok(())
}

#[cfg(not(unix))]
async fn run_node(_args: &[String]) -> Result<(), String> {
    Err("managed node mode needs Unix".into())
}

fn config_path<'a>(args: &'a [String], command: &str) -> Result<&'a str, String> {
    match args {
        [path] => Ok(path),
        _ => Err(format!("Usage: fiend-core {command} <config.toml>")),
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(fiend_core::node::logs::Writer::default)
        .try_init();
}

fn extract_bin(input: &str) -> Result<(), String> {
    let raw = fs::read(input).map_err(|error| format!("read {input}: {error}"))?;
    let template =
        parse_client_hello(&raw).map_err(|error| format!("parse ClientHello: {error}"))?;
    let output = output_path(input);
    template
        .save(&output)
        .map_err(|error| format!("write template: {error}"))?;
    template
        .verify(&raw)
        .map_err(|error| format!("verification FAILED: {error}"))?;
    eprintln!("domain: {}", template.domain);
    eprintln!("grease fields: {}", template.grease.len());
    eprintln!("record max: {}", template.record_max);
    eprintln!("verified: static bytes match original 1:1");
    eprintln!("written to {}", output.display());
    Ok(())
}

fn output_path(input: &str) -> PathBuf {
    match input.strip_suffix(".bin") {
        Some(stem) => PathBuf::from(format!("{stem}.json")),
        None => PathBuf::from(format!("{input}.json")),
    }
}
