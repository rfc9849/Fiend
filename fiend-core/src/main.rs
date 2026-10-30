mod provision;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use fiend_core::client::Client;
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
        Some("check") => config_path(&args[1..], "check").and_then(|path| {
            config::load_server(path).map_err(|error| format!("config: {error}"))?;
            println!("node config is valid");
            Ok(())
        }),
        Some("--version") => {
            println!("fiend-core {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("client") => run_client(&args[1..]).await,
        Some("server") => run_server(&args[1..]).await,
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
    println!("  client <config.toml>");
    println!("  server <config.toml>");
    println!("  check <config.toml>");
    println!("  generate-secret");
    println!("  prepare <directory> [--import <server.toml> | --port <port>]");
}

fn run_template(args: &[String]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: fiend-core template <input.bin>".to_owned());
    }
    extract_bin(&args[0])
}

async fn run_client(args: &[String]) -> Result<(), String> {
    let path = config_path(args, "client")?;
    let settings = config::load_client(path).map_err(|error| format!("config: {error}"))?;
    let listener = net::bind_listener(settings.listen)
        .map_err(|error| format!("listen {}: {error}", settings.listen))?;
    let listen = listener
        .local_addr()
        .map_err(|error| format!("listen address: {error}"))?;
    tracing::info!(listen = %listen, "client started");

    let client = Client::new(settings.config);
    tokio::select! {
        result = client.serve(listener) => {
            result.map_err(|error| format!("client: {error}"))?;
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received");
        }
    }
    client.close();
    tracing::info!("client stopped");
    Ok(())
}

async fn run_server(args: &[String]) -> Result<(), String> {
    let path = config_path(args, "server")?;
    let settings = config::load_server(path).map_err(|error| format!("config: {error}"))?;
    let listener = net::bind_listener(settings.listen)
        .map_err(|error| format!("listen {}: {error}", settings.listen))?;
    let listen = listener
        .local_addr()
        .map_err(|error| format!("listen address: {error}"))?;
    tracing::info!(listen = %listen, "server started");

    let server = Server::new(settings.config);
    tokio::select! {
        result = server.serve(listener) => {
            result.map_err(|error| format!("server: {error}"))?;
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received");
        }
    }
    tracing::info!("server stopped");
    Ok(())
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
        .with_writer(std::io::stderr)
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
