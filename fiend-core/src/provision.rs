use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use fiend_core::config;

pub fn prepare(args: &[String]) -> Result<(), String> {
    let Some(directory) = args.first() else {
        return Err(usage());
    };
    let mut import = None;
    let mut port = 443;
    match &args[1..] {
        [] => {}
        [flag, value] if flag == "--import" => import = Some(value),
        [flag, value] if flag == "--port" => {
            port = value.parse::<u16>().map_err(|_| "invalid node port")?;
            if port == 0 {
                return Err("node port must be nonzero".into());
            }
        }
        _ => return Err(usage()),
    }
    let raw = if let Some(path) = import {
        let settings = config::load_server(path).map_err(|e| format!("import: {e}"))?;
        let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut value: toml::Value = toml::from_str(&raw).map_err(|e| e.to_string())?;
        let table = value
            .as_table_mut()
            .ok_or("server config must be a table")?;
        table.remove("secret_file");
        table.insert("secret".into(), hex::encode(&settings.config.secret).into());
        toml::to_string(&value).map_err(|e| e.to_string())?
    } else {
        let secret = hex::encode(rand::random::<[u8; 32]>());
        format!(
            "listen = \"0.0.0.0:{port}\"\nsecret = \"{secret}\"\norigin_host = \"www.wikipedia.org\"\norigin_port = 443\n"
        )
    };
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
    "Usage: fiend-core prepare <directory> [--import <server.toml> | --port <port>]".into()
}
