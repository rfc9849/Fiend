use std::path::Path;
use std::process::ExitCode;

use fiend_panel::{App, Config, io_error, router, shutdown};

#[tokio::main]
async fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn print_access(config: &Config) {
    let port = config.listen.port();
    if config.listen.ip().is_loopback() {
        println!("ssh -N -L 8080:127.0.0.1:{port} <host>");
        println!("http://127.0.0.1:8080/setup/{}", config.setup_token);
    } else {
        println!("http://<host>:{port}/setup/{}", config.setup_token);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => println!("fiend-panel {}", env!("CARGO_PKG_VERSION")),
        [command, path] if command == "init" => {
            let config = Config::init(Path::new(path)).map_err(io_error)?;
            print_access(&config);
        }
        [command, path] if command == "renew-setup" => {
            let config = Config::renew_setup(Path::new(path)).map_err(io_error)?;
            print_access(&config);
            println!("The setup link works for one hour and only until the first admin exists.");
        }
        [command, path] if command == "check" => {
            Config::load(Path::new(path)).map_err(io_error)?;
            println!("panel config is valid");
        }
        [command, path] if command == "export-types" => {
            fiend_panel::types::export(Path::new(path))?;
        }
        [command, path] if command == "serve" => {
            let config = Config::load(Path::new(path)).map_err(io_error)?;
            let listen = config.listen;
            let app = App::open(config).await.map_err(io_error)?;
            app.start();
            let listener = tokio::net::TcpListener::bind(listen).await?;
            eprintln!("panel listening on {}", listener.local_addr()?);
            axum::serve(
                listener,
                router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown())
            .await?;
        }
        _ => {
            return Err(
                "Usage: fiend-panel <init|renew-setup|check|serve> <panel.toml> | export-types <folder> | --version"
                    .into(),
            );
        }
    }
    Ok(())
}
