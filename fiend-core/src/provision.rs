use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

use fiend_core::config;

struct Options {
    port: u16,
    listen: Option<SocketAddr>,
    proxy_protocol: bool,
    fallbacks: Vec<(String, String)>,
    fallback_proxy_protocol: bool,
}

pub fn prepare(args: &[String]) -> Result<(), String> {
    let Some(directory) = args.first() else {
        return Err(usage());
    };
    let options = parse(&args[1..])?;
    let raw = render(&options);
    let directory = Path::new(directory);
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join("node.toml");
    let mut file_options = OpenOptions::new();
    file_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        file_options.mode(0o600);
    }
    let mut file = file_options
        .open(&path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    let written = file
        .write_all(raw.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
        .and_then(|()| {
            config::load_server(&path)
                .map(drop)
                .map_err(|e| e.to_string())
        });
    if let Err(error) = written {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    println!("node config saved to {}", path.display());
    Ok(())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        port: 443,
        listen: None,
        proxy_protocol: false,
        fallbacks: Vec::new(),
        fallback_proxy_protocol: false,
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(usage)?.trim();
        match flag.as_str() {
            "--port" => {
                options.port = value.parse::<u16>().map_err(|_| "invalid node port")?;
                if options.port == 0 {
                    return Err("node port must be nonzero".into());
                }
            }
            "--listen" if value.is_empty() => {}
            "--listen" => {
                let listen = value
                    .parse::<SocketAddr>()
                    .map_err(|_| format!("invalid listen address {value}"))?;
                if listen.port() == 0 {
                    return Err("listen port must be nonzero".into());
                }
                options.listen = Some(listen);
            }
            "--proxy-protocol" => options.proxy_protocol = switch(flag, value)?,
            "--fallback" => {
                for entry in value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                {
                    let (host, address) = entry
                        .split_once('=')
                        .ok_or_else(|| format!("fallback {entry} must be <host>=<address>"))?;
                    options
                        .fallbacks
                        .push((host.trim().to_owned(), address.trim().to_owned()));
                }
            }
            "--fallback-proxy-protocol" => options.fallback_proxy_protocol = switch(flag, value)?,
            _ => return Err(usage()),
        }
    }
    if options.fallback_proxy_protocol && options.fallbacks.is_empty() {
        return Err("--fallback-proxy-protocol needs --fallback".into());
    }
    Ok(options)
}

fn switch(flag: &str, value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => Err(format!("{flag} must be true or false")),
    }
}

fn quote(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn render(options: &Options) -> String {
    let listen = options
        .listen
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], options.port)));
    let mut raw = format!("listen = {}\n", quote(&listen.to_string()));
    if listen.port() != options.port {
        raw.push_str(&format!("public_port = {}\n", options.port));
    }
    if options.proxy_protocol {
        raw.push_str("proxy_protocol = true\n");
    }
    raw.push_str("origin_host = \"www.wikipedia.org\"\norigin_port = 443\n");
    for (host, address) in &options.fallbacks {
        raw.push_str(&format!(
            "\n[[fallback]]\nhost = {}\naddress = {}\n",
            quote(host),
            quote(address)
        ));
        if options.fallback_proxy_protocol {
            raw.push_str("proxy_protocol = true\n");
        }
    }
    raw
}

fn usage() -> String {
    "Usage: fiend-core prepare <directory> [--port <port>] [--listen <address>] [--proxy-protocol <true|false>] [--fallback <host>=<address>[,...]] [--fallback-proxy-protocol <true|false>]".into()
}
