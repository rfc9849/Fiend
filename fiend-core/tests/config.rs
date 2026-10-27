use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fiend_core::config;
use fiend_core::fingerprint::Template;
use fiend_core::target::TargetPolicy;

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");
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

fn write_template(dir: &Path) {
    fs::write(dir.join("template.json"), TEMPLATE_JSON).expect("write template");
}

fn client_file(dir: &Path, name: &str, body: &str) -> PathBuf {
    write_template(dir);
    write(dir, name, body)
}

#[test]
fn client_loads_defaults() {
    let dir = temp_dir("client-defaults");
    let path = client_file(
        &dir,
        "client.toml",
        &format!(
            r#"
server = "example.com:443"
secret = "{SECRET_HEX}"
template = "template.json"
"#
        ),
    );

    let settings = config::load_client(&path).expect("load client");
    assert_eq!(settings.listen.to_string(), "127.0.0.1:1080");
    assert_eq!(settings.config.server, "example.com:443");
    assert_eq!(settings.config.secret, vec![0x42u8; 32]);
    assert!(!settings.config.template.domain.is_empty());
    assert_eq!(settings.config.udp_idle, Duration::from_secs(60));
    assert_eq!(settings.config.udp_keepalive, Duration::from_secs(20));
    assert_eq!(settings.config.max_udp_flows, 64);
    assert_eq!(settings.config.max_connections, 1024);
    assert_eq!(settings.config.handshake_timeout, Duration::from_secs(10));
    assert_eq!(settings.config.tcp_notsent_lowat, 0);
    assert_eq!(settings.config.mux.max_links, 8);
    assert_eq!(settings.config.mux.open_timeout, Duration::from_secs(45));
    assert_eq!(settings.config.mux.max_frame_size, 16 << 10);
}

#[test]
fn client_loads_overrides_and_relative_files() {
    let dir = temp_dir("client-overrides");
    write_template(&dir);
    write(&dir, "secret.txt", &format!("{SECRET_HEX}\n"));
    let path = write(
        &dir,
        "client.toml",
        r#"
listen = "0.0.0.0:9050"
server = "example.com:8443"
secret_file = "secret.txt"
template = "template.json"
handshake_timeout_ms = 1500
max_connections = 2048
tcp_notsent_lowat = 65536

[udp]
idle_ms = 7000
keepalive_ms = 3000
max_flows = 12

[mux]
max_links = 3
open_timeout_ms = 1000
ping_interval_ms = 500
max_frame_size = 32768
"#,
    );

    let settings = config::load_client(&path).expect("load client");
    assert_eq!(settings.listen.to_string(), "0.0.0.0:9050");
    assert_eq!(settings.config.server, "example.com:8443");
    assert_eq!(settings.config.secret, vec![0x42u8; 32]);
    assert_eq!(
        settings.config.handshake_timeout,
        Duration::from_millis(1500)
    );
    assert_eq!(settings.config.udp_idle, Duration::from_millis(7000));
    assert_eq!(settings.config.udp_keepalive, Duration::from_millis(3000));
    assert_eq!(settings.config.max_udp_flows, 12);
    assert_eq!(settings.config.max_connections, 2048);
    assert_eq!(settings.config.tcp_notsent_lowat, 65536);
    assert_eq!(settings.config.mux.max_links, 3);
    assert_eq!(
        settings.config.mux.open_timeout,
        Duration::from_millis(1000)
    );
    assert_eq!(
        settings.config.mux.ping_interval,
        Duration::from_millis(500)
    );
    assert_eq!(settings.config.mux.max_frame_size, 32 << 10);
}

#[test]
fn server_loads_defaults() {
    let dir = temp_dir("server-defaults");
    let path = write(
        &dir,
        "server.toml",
        &format!(
            r#"
secret = "{SECRET_HEX}"
origin_host = "origin.example"
"#
        ),
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
fn server_loads_overrides_and_binary_secret() {
    let dir = temp_dir("server-overrides");
    fs::write(dir.join("secret.bin"), [0x01u8; 32]).expect("write secret");
    let path = write(
        &dir,
        "server.toml",
        r#"
listen = "127.0.0.1:8443"
secret_file = "secret.bin"
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
    assert_eq!(settings.config.secret, vec![0x01u8; 32]);
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
fn ambiguous_secret_file_is_rejected() {
    let dir = temp_dir("ambiguous-secret");
    fs::write(dir.join("secret.txt"), "42424242424242424242424242424242").expect("write secret");
    let path = write(
        &dir,
        "server.toml",
        r#"
secret_file = "secret.txt"
origin_host = "origin.example"
"#,
    );
    let error = config::load_server(&path).err().expect("ambiguous secret");
    assert!(
        error
            .to_string()
            .contains("write 32 raw bytes or 64 hex characters"),
        "{error}"
    );
}

#[test]
fn client_errors_are_specific() {
    let cases: [(&str, &str, String); 12] = [
        (
            "unknown field",
            r#"
server = "example.com:443"
secret = "SECRET"
template = "template.json"
surprise = true
"#,
            "unknown field".to_owned(),
        ),
        (
            "missing server",
            r#"secret = "SECRET"
template = "template.json"
"#,
            "client server is required".to_owned(),
        ),
        (
            "missing template",
            r#"
server = "example.com:443"
secret = "SECRET"
"#,
            "client template is required".to_owned(),
        ),
        (
            "missing secret",
            r#"
server = "example.com:443"
template = "template.json"
"#,
            "secret or secret_file is required".to_owned(),
        ),
        (
            "two secrets",
            r#"
server = "example.com:443"
secret = "SECRET"
secret_file = "secret.txt"
template = "template.json"
"#,
            "set either secret or secret_file".to_owned(),
        ),
        (
            "bad listen",
            r#"
listen = "not-an-address"
server = "example.com:443"
secret = "SECRET"
template = "template.json"
"#,
            "client listen".to_owned(),
        ),
        (
            "bad server",
            r#"
server = "example.com"
secret = "SECRET"
template = "template.json"
"#,
            "client server".to_owned(),
        ),
        (
            "short secret",
            r#"
server = "example.com:443"
secret = "4242"
template = "template.json"
"#,
            "secret must be exactly 32 bytes".to_owned(),
        ),
        (
            "short udp keepalive",
            r#"
server = "example.com:443"
secret = "SECRET"
template = "template.json"

[udp]
idle_ms = 1000
keepalive_ms = 1000
"#,
            "keepalive_ms must be less than idle_ms".to_owned(),
        ),
        (
            "zero udp flows",
            r#"
server = "example.com:443"
secret = "SECRET"
template = "template.json"

[udp]
max_flows = 0
"#,
            "max_flows".to_owned(),
        ),
        (
            "zero links",
            r#"
server = "example.com:443"
secret = "SECRET"
template = "template.json"

[mux]
max_links = 0
"#,
            "max_links".to_owned(),
        ),
        (
            "zero connections",
            r#"
server = "example.com:443"
secret = "SECRET"
template = "template.json"
max_connections = 0
"#,
            "max_connections".to_owned(),
        ),
    ];

    for (name, body, needle) in cases {
        let dir = temp_dir(&format!("client-error-{name}"));
        let body = body.replace("SECRET", SECRET_HEX);
        let path = client_file(&dir, "client.toml", &body);
        let error = config::load_client(&path).err().expect(name);
        assert!(
            error.to_string().contains(&needle),
            "{name}: {error} does not contain {needle}"
        );
    }
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
        let body = format!(
            r#"
server = "example.com:443"
secret = "{SECRET_HEX}"
template = "template.json"

{extra}"#
        );
        let path = client_file(&dir, "client.toml", &body);
        let error = config::load_client(&path).err().expect(name);
        assert!(
            error.to_string().contains(needle),
            "{name}: {error} does not contain {needle}"
        );
    }
}

#[test]
fn server_errors_are_specific() {
    let cases: [(&str, &str, &str); 7] = [
        (
            "short replay ttl",
            r#"
secret = "SECRET"
origin_host = "origin.example"
replay_ttl_ms = 3600000
"#,
            "replay_ttl_ms must be at least",
        ),
        (
            "missing origin",
            r#"
secret = "SECRET"
"#,
            "origin_host is required",
        ),
        (
            "zero origin port",
            r#"
secret = "SECRET"
origin_host = "origin.example"
origin_port = 0
"#,
            "origin_port",
        ),
        (
            "zero record size",
            r#"
secret = "SECRET"
origin_host = "origin.example"

[origin]
record_sizes = [0]
"#,
            "record_sizes",
        ),
        (
            "unknown origin field",
            r#"
secret = "SECRET"
origin_host = "origin.example"

[origin]
nope = 1
"#,
            "unknown field",
        ),
        (
            "server udp field",
            r#"
secret = "SECRET"
origin_host = "origin.example"

[udp]
keepalive_ms = 100
"#,
            "unknown field",
        ),
        (
            "zero connections",
            r#"
secret = "SECRET"
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
        "client.toml",
        r#"
server = "example.com:443"
secret = "do-not-print-this-value"
template = "template.json"
broken = [
"#,
    );
    let error = config::load_client(&path).err().expect("parse error");
    let message = error.to_string();
    assert!(message.contains("parse"), "{message}");
    assert!(!message.contains("do-not-print-this-value"), "{message}");
}

#[test]
fn missing_files_are_reported() {
    let dir = temp_dir("missing-files");
    let path = dir.join("nope.toml");
    let error = config::load_client(&path).err().expect("missing config");
    assert!(error.to_string().contains("read"));

    let path = client_file(
        &dir,
        "client.toml",
        &format!(
            r#"
server = "example.com:443"
secret = "{SECRET_HEX}"
template = "missing.json"
"#
        ),
    );
    let error = config::load_client(&path).err().expect("missing template");
    assert!(error.to_string().contains("client template"));
    assert!(error.to_string().contains("missing.json"));
}

#[test]
fn invalid_template_is_reported() {
    let dir = temp_dir("invalid-template");
    write(&dir, "template.json", "not a template");
    let path = write(
        &dir,
        "client.toml",
        &format!(
            r#"
server = "example.com:443"
secret = "{SECRET_HEX}"
template = "template.json"
"#
        ),
    );
    let error = config::load_client(&path).err().expect("invalid template");
    assert!(error.to_string().contains("client template"));
}

#[test]
fn loaded_template_matches_reference() {
    let dir = temp_dir("template-reference");
    let path = client_file(
        &dir,
        "client.toml",
        &format!(
            r#"
server = "example.com:443"
secret = "{SECRET_HEX}"
template = "template.json"
"#
        ),
    );
    let settings = config::load_client(&path).expect("load client");
    let expected = Template::from_json(TEMPLATE_JSON).expect("reference template");
    assert_eq!(settings.config.template.domain, expected.domain);
    assert_eq!(settings.config.template.record_max, expected.record_max);
}
