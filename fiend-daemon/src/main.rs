use std::path::PathBuf;

use clap::{Parser, Subcommand};
use fiend_daemon::peer::{PeerPolicy, SystemDirectory};
use fiend_daemon::platform::{NullPlatform, Platform};
use fiend_daemon::{Daemon, Store, bind_socket, install_logging, serve};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(version, about = "Fiend privileged daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon.
    Run(RunArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Run in the foreground for tests, for example under sudo. The user
    /// who started sudo may use the daemon.
    #[arg(long)]
    foreground: bool,
    /// Socket path.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Directory for profiles, settings, and rule lists.
    #[arg(long)]
    store: Option<PathBuf>,
    /// Also allow this user ID. Can repeat.
    #[arg(long = "allow-uid")]
    allow_uid: Vec<u32>,
    /// Engine only: no TUN device, no routes, no DNS change.
    #[arg(long)]
    no_network: bool,
}

fn default_store() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/Fiend")
    } else {
        PathBuf::from("/var/lib/fiend")
    }
}

fn main() {
    let Command::Run(args) = Cli::parse().command;
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("fiend-daemon")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => fail(format!("cannot start the runtime: {error}")),
    };
    let result = if args.no_network {
        runtime.block_on(run(NullPlatform, args))
    } else {
        runtime.block_on(run(fiend_daemon::platform::native(), args))
    };
    if let Err(error) = result {
        fail(error);
    }
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("fiend-daemon: {message}");
    std::process::exit(1);
}

async fn run<P: Platform>(platform: P, args: RunArgs) -> Result<(), String> {
    let logs = install_logging();
    let socket = args.socket.unwrap_or_else(fiend_ipc::path::default_socket);
    let store_dir = args.store.unwrap_or_else(default_store);
    let store = Store::open(&store_dir)
        .map_err(|error| format!("cannot open {}: {error}", store_dir.display()))?;
    let mut policy = PeerPolicy::system_default();
    policy.uids.extend(args.allow_uid);
    if args.foreground
        && let Some(uid) = std::env::var("SUDO_UID").ok().and_then(|v| v.parse().ok())
    {
        policy.uids.push(uid);
    }
    let listener = bind_socket(&socket, policy.group.as_deref(), &SystemDirectory)
        .map_err(|error| format!("cannot use {}: {error}", socket.display()))?;
    tracing::info!(socket = %socket.display(), "daemon started");
    let daemon = Daemon::new(platform, store, logs);
    let shutdown = CancellationToken::new();
    let server = tokio::spawn(serve(daemon.clone(), listener, policy, shutdown.clone()));
    daemon.start().await;
    wait_for_signal().await;
    tracing::info!("daemon stopping");
    shutdown.cancel();
    daemon.shutdown().await;
    let _ = server.await;
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}
