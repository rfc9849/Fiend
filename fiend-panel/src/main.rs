use std::path::Path;
use std::process::ExitCode;

use fiend_panel::{Config, io_error, router, shutdown};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => println!("fiend-panel {}", env!("CARGO_PKG_VERSION")),
        [command, path] if command == "init" => {
            let config = Config::init(Path::new(path)).map_err(io_error)?;
            println!("ssh -N -L 8080:127.0.0.1:{} <host>", config.listen.port());
            println!("http://127.0.0.1:8080/setup/{}", config.setup_token);
        }
        [command, path] if command == "check" => {
            Config::load(Path::new(path)).map_err(io_error)?;
            println!("panel config is valid");
        }
        [command, path] if command == "serve" => {
            let config = Config::load(Path::new(path)).map_err(io_error)?;
            let listener = tokio::net::TcpListener::bind(config.listen).await?;
            eprintln!("panel listening on {}", listener.local_addr()?);
            axum::serve(listener, router(config))
                .with_graceful_shutdown(shutdown())
                .await?;
        }
        _ => return Err("Usage: fiend-panel <init|check|serve> <panel.toml> | --version".into()),
    }
    Ok(())
}
