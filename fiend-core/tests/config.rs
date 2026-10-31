use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fiend_core::config;
use fiend_core::target::TargetPolicy;

const SECRET_HEX: &str = "4242424242424242424242424242424242424242424242424242424242424242";

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fiend-core-config-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).expect("write file");
    path
}

#[test]
fn server_loads_defaults() {
    let dir = temp_dir("server-defaults");
    let path = write(
        &dir,
        "server.toml",
        r#"
origin_host = "origin.example"
"#,
    );

    let settings = config::load_server(&path).expect("load server");
    assert_eq!(settings.listen.to_string(), "0.0.0.0:443");
    assert_eq!(settings.config.origin_host, "origin.example");
    assert_eq!(settings.config.origin_port, 443);
    assert_eq!(settings.config.origin, Default::default());
    assert_eq!(
        settings.config.replay_ttl,
        Duration::from_secs(3 * 3600 + 300)
    );
    assert_eq!(settings.config.udp_idle, Duration::from_secs(60));
    assert_eq!(settings.config.max_connections, 1024);
    assert_eq!(settings.config.tcp_notsent_lowat, 0);
    assert_eq!(settings.config.target, TargetPolicy::default());
    assert_eq!(settings.config.mux.max_links, 8);
}

#[test]
fn server_loads_overrides() {
    let dir = temp_dir("server-overrides");
    let path = write(
        &dir,
        "server.toml",
        r#"
listen = "127.0.0.1:8443"
origin_host = "origin.example"
origin_port = 9443
replay_ttl_ms = 14400000
max_connections = 2048
tcp_notsent_lowat = 131072

[target]
allow_loopback = true
allow_private = true

[origin]
cipher_suite = 4865
key_share_group = 29
extension_order = [43, 51]
record_sizes = [35, 4000, 300]

[udp]
idle_ms = 2500

[mux]
max_links = 3
"#,
    );

    let settings = config::load_server(&path).expect("load server");
    assert_eq!(settings.listen.to_string(), "127.0.0.1:8443");
    assert_eq!(settings.config.origin_host, "origin.example");
    assert_eq!(settings.config.origin_port, 9443);
    assert_eq!(settings.config.replay_ttl, Duration::from_secs(4 * 3600));
    assert_eq!(settings.config.udp_idle, Duration::from_millis(2500));
    assert_eq!(settings.config.max_connections, 2048);
    assert_eq!(settings.config.tcp_notsent_lowat, 131072);
    assert_eq!(
        settings.config.target,
        TargetPolicy {
            allow_loopback: true,
            allow_private: true,
            ..TargetPolicy::default()
        }
    );
    assert_eq!(settings.config.mux.max_links, 3);
    assert_eq!(settings.config.origin.cipher_suite, 4865);
    assert_eq!(settings.config.origin.key_share_group, 29);
    assert_eq!(settings.config.origin.extension_order, vec![43, 51]);
    assert_eq!(settings.config.origin.record_sizes, vec![35, 4000, 300]);
}

#[test]
fn mux_values_are_validated() {
    let cases = [
        ("frame", "[mux]\nmax_frame_size = 16000\n", "max_frame_size"),
        (
            "backoff",
            "[mux]\nbackoff_floor_ms = 100\nbackoff_ceiling_ms = 50\n",
            "backoff_ceiling_ms",
        ),
        ("attempts", "[mux]\nattempts = 0\n", "attempts"),
        (
            "send buffer",
            "[mux]\nmax_send_buffer_size = 4294967296\n",
            "max_send_buffer_size",
        ),
    ];

    for (name, extra, needle) in cases {
        let dir = temp_dir(&format!("mux-error-{name}"));
        let body = format!("origin_host = \"origin.example\"\n\n{extra}");
        let path = write(&dir, "server.toml", &body);
        let error = config::load_server(&path).err().expect(name);
        assert!(
            error.to_string().contains(needle),
            "{name}: {error} does not contain {needle}"
        );
    }
}

#[test]
fn server_errors_are_specific() {
    let cases: [(&str, &str, &str); 8] = [
        (
            "secret field",
            r#"
secret = "SECRET"
origin_host = "origin.example"
"#,
            "unknown field",
        ),
        (
            "short replay ttl",
            r#"
origin_host = "origin.example"
replay_ttl_ms = 3600000
"#,
            "replay_ttl_ms must be at least",
        ),
        (
            "missing origin",
            r#"
"#,
            "origin_host is required",
        ),
        (
            "zero origin port",
            r#"
origin_host = "origin.example"
origin_port = 0
"#,
            "origin_port",
        ),
        (
            "zero record size",
            r#"
origin_host = "origin.example"

[origin]
record_sizes = [0]
"#,
            "record_sizes",
        ),
        (
            "unknown origin field",
            r#"
origin_host = "origin.example"

[origin]
nope = 1
"#,
            "unknown field",
        ),
        (
            "server udp field",
            r#"
origin_host = "origin.example"

[udp]
keepalive_ms = 100
"#,
            "unknown field",
        ),
        (
            "zero connections",
            r#"
origin_host = "origin.example"
max_connections = 0
"#,
            "max_connections",
        ),
    ];

    for (name, body, needle) in cases {
        let dir = temp_dir(&format!("server-error-{name}"));
        let body = body.replace("SECRET", SECRET_HEX);
        let path = write(&dir, "server.toml", &body);
        let error = config::load_server(&path).err().expect(name);
        assert!(
            error.to_string().contains(needle),
            "{name}: {error} does not contain {needle}"
        );
    }
}

#[test]
fn parse_errors_do_not_echo_secrets() {
    let dir = temp_dir("parse-redaction");
    let path = write(
        &dir,
        "server.toml",
        r#"
origin_host = "do-not-print-this-value"
broken = [
"#,
    );
    let error = config::load_server(&path).err().expect("parse error");
    let message = error.to_string();
    assert!(message.contains("parse"), "{message}");
    assert!(!message.contains("do-not-print-this-value"), "{message}");
}

#[test]
fn missing_files_are_reported() {
    let dir = temp_dir("missing-files");
    let path = dir.join("nope.toml");
    let error = config::load_server(&path).err().expect("missing config");
    assert!(error.to_string().contains("read"));
}
