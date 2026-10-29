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
    /// Install the system service. Run it as root.
    Install(InstallArgs),
    /// Remove the system service and restore the network. Run it as root.
    Uninstall(UninstallArgs),
    /// Undo network changes that a stopped daemon left. Run it as root.
    Restore {
        /// Directory of the daemon state.
        #[arg(long)]
        store: Option<PathBuf>,
    },
}

#[derive(clap::Args)]
struct InstallArgs {
    /// SID of the user who may use the service.
    #[cfg(windows)]
    #[arg(long, hide = true)]
    user_sid: Option<String>,
    /// File for the error text.
    #[cfg(windows)]
    #[arg(long, hide = true)]
    error_file: Option<PathBuf>,
}

#[derive(clap::Args)]
struct UninstallArgs {
    /// Also remove profiles, settings, and rule lists.
    #[arg(long)]
    purge: bool,
    /// File for the error text.
    #[cfg(windows)]
    #[arg(long, hide = true)]
    error_file: Option<PathBuf>,
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
    /// Run as a Windows service.
    #[arg(long, hide = true)]
    service: bool,
}

impl RunArgs {
    #[cfg(windows)]
    fn service() -> Self {
        Self {
            foreground: false,
            socket: None,
            store: None,
            allow_uid: Vec::new(),
            no_network: false,
            service: true,
        }
    }
}

fn default_store() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Application Support/Fiend")
    }
    #[cfg(windows)]
    {
        fiend_daemon::windows::store_dir()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        PathBuf::from("/var/lib/fiend")
    }
}

fn store_dir_for(args: &RunArgs) -> PathBuf {
    args.store.clone().unwrap_or_else(default_store)
}

#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, daemon_service_main);

fn main() {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--service")) {
        if let Err(error) = windows_service::service_dispatcher::start(
            fiend_daemon::windows::SERVICE_NAME,
            ffi_service_main,
        ) {
            fail(format!("cannot start the service dispatcher: {error}"));
        }
        return;
    }

    let args = match Cli::parse().command {
        Command::Run(args) => args,
        Command::Install(args) => {
            #[cfg(windows)]
            let result = fiend_daemon::install::install(args.user_sid.as_deref());
            #[cfg(not(windows))]
            let result = {
                let _ = args;
                fiend_daemon::install::install()
            };
            if let Err(error) = result {
                #[cfg(windows)]
                fiend_daemon::install::write_error(args.error_file.as_deref(), &error);
                fail(error);
            }
            return;
        }
        Command::Uninstall(args) => {
            if let Err(error) = fiend_daemon::install::uninstall(args.purge) {
                #[cfg(windows)]
                fiend_daemon::install::write_error(args.error_file.as_deref(), &error);
                fail(error);
            }
            return;
        }
        Command::Restore { store } => {
            let dir = store.unwrap_or_else(default_store);
            match fiend_daemon::platform::native(&dir).recover() {
                Ok(()) => println!("network settings are restored"),
                Err(error) => fail(format!("cannot restore: {error}")),
            }
            return;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("fiend-daemon")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => fail(format!("cannot start the runtime: {error}")),
    };
    let result = if args.no_network {
        runtime.block_on(run_foreground(NullPlatform, args))
    } else {
        runtime.block_on(run_foreground(
            fiend_daemon::platform::native(&store_dir_for(&args)),
            args,
        ))
    };
    if let Err(error) = result {
        fail(error);
    }
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("fiend-daemon: {message}");
    std::process::exit(1);
}

async fn run_foreground<P: Platform>(platform: P, args: RunArgs) -> Result<(), String> {
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    let signal = tokio::spawn(async move {
        wait_for_signal().await;
        stop.cancel();
    });
    let result = run(platform, args, shutdown, || {}).await;
    signal.abort();
    result
}

async fn run<P, F>(
    platform: P,
    args: RunArgs,
    shutdown: CancellationToken,
    ready: F,
) -> Result<(), String>
where
    P: Platform,
    F: FnOnce(),
{
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
    let server = tokio::spawn(serve(daemon.clone(), listener, policy, shutdown.clone()));
    ready();
    daemon.start().await;
    shutdown.cancelled().await;
    tracing::info!("daemon stopping");
    daemon.shutdown().await;
    let _ = server.await;
    #[cfg(unix)]
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

#[cfg(unix)]
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

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(windows)]
fn daemon_service_main(_: Vec<std::ffi::OsString>) {
    use std::time::Duration;

    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
            stop.cancel();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status =
        match service_control_handler::register(fiend_daemon::windows::SERVICE_NAME, handler) {
            Ok(status) => status,
            Err(_) => return,
        };
    let pending = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::NO_ERROR,
        checkpoint: 1,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    };
    let _ = status.set_service_status(pending);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("fiend-daemon")
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = status.set_service_status(stopped(ServiceExitCode::Win32(1)));
            return;
        }
    };
    let args = RunArgs::service();
    let platform = fiend_daemon::platform::native(&store_dir_for(&args));
    let stopping = shutdown.clone();
    let stop_report = runtime.spawn(async move {
        stopping.cancelled().await;
        let mut checkpoint = 1;
        loop {
            let _ = status.set_service_status(stop_pending(checkpoint));
            checkpoint += 1;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    let ready_status = status;
    let result = runtime.block_on(run(platform, args, shutdown, move || {
        let running = ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP
                | ServiceControlAccept::SHUTDOWN
                | ServiceControlAccept::PRESHUTDOWN,
            exit_code: ServiceExitCode::NO_ERROR,
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        };
        let _ = ready_status.set_service_status(running);
    }));
    stop_report.abort();
    let exit = match result {
        Ok(()) => ServiceExitCode::NO_ERROR,
        Err(error) => {
            tracing::error!(%error, "service stopped with an error");
            ServiceExitCode::Win32(1)
        }
    };
    let _ = status.set_service_status(stopped(exit));
}

#[cfg(windows)]
fn stop_pending(checkpoint: u32) -> windows_service::service::ServiceStatus {
    use std::time::Duration;

    use windows_service::service::{
        ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
    };

    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::NO_ERROR,
        checkpoint,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    }
}

#[cfg(windows)]
fn stopped(
    exit_code: windows_service::service::ServiceExitCode,
) -> windows_service::service::ServiceStatus {
    use std::time::Duration;

    use windows_service::service::{
        ServiceControlAccept, ServiceState, ServiceStatus, ServiceType,
    };

    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}
