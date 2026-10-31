use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use fiend_core::config;

pub fn prepare(args: &[String]) -> Result<(), String> {
    let Some(directory) = args.first() else {
        return Err(usage());
    };
    let port = match &args[1..] {
        [] => 443,
        [flag, value] if flag == "--port" => {
            let port = value.parse::<u16>().map_err(|_| "invalid node port")?;
            if port == 0 {
                return Err("node port must be nonzero".into());
            }
            port
        }
        _ => return Err(usage()),
    };
    let raw = format!(
        "listen = \"0.0.0.0:{port}\"\norigin_host = \"www.wikipedia.org\"\norigin_port = 443\n"
    );
    let directory = Path::new(directory);
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join("node.toml");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    file.write_all(raw.as_bytes()).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    config::load_server(&path).map_err(|e| e.to_string())?;
    println!("node config saved to {}", path.display());
    Ok(())
}

fn usage() -> String {
    "Usage: fiend-core prepare <directory> [--port <port>]".into()
}
