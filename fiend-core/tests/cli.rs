use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");
const SECRET_HEX: &str = "4242424242424242424242424242424242424242424242424242424242424242";

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fiend-core")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fiend-core-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).expect("write file");
    path
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("free port address").port()
}

fn wait_for_port(port: u16, child: &mut Child, stderr: &mut Option<std::process::ChildStderr>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        if let Some(status) = child.try_wait().expect("check child") {
            let text = read_stderr(stderr);
            panic!("process exited before listening: {status}; stderr: {text}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let text = read_stderr(stderr);
            panic!("process did not listen on port {port}; stderr: {text}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_stderr(stderr: &mut Option<std::process::ChildStderr>) -> String {
    let Some(mut stderr) = stderr.take() else {
        return String::new();
    };
    let mut text = String::new();
    let _ = stderr.read_to_string(&mut text);
    text
}

fn wait_for_exit(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().expect("check child") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn assert_clean_shutdown(mode: &str, name: &str, config: &str) {
    let dir = temp_dir(name);
    fs::write(dir.join("template.json"), TEMPLATE_JSON).expect("write template");
    let port = free_port();
    let config = config.replace("PORT", &port.to_string());
    let path = write(&dir, "config.toml", &config);

    let mut child = Command::new(bin())
        .arg(mode)
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start process");
    let mut stderr = child.stderr.take();

    wait_for_port(port, &mut child, &mut stderr);
    thread::sleep(Duration::from_millis(50));

    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(killed.success(), "kill command failed");

    let Some(status) = wait_for_exit(&mut child, Duration::from_secs(5)) else {
        let _ = child.kill();
        let text = read_stderr(&mut stderr);
        panic!("process did not stop on SIGTERM; stderr: {text}");
    };
    let text = read_stderr(&mut stderr);
    assert!(status.success(), "process failed: {status}; stderr: {text}");
    assert!(
        text.contains("stopped"),
        "shutdown was not logged; stderr: {text}"
    );
}

#[cfg(unix)]
#[test]
fn client_starts_and_stops_cleanly() {
    assert_clean_shutdown(
        "client",
        "client-lifecycle",
        &format!(
            r#"
listen = "127.0.0.1:PORT"
server = "127.0.0.1:1"
secret = "{SECRET_HEX}"
template = "template.json"
"#
        ),
    );
}

#[cfg(unix)]
#[test]
fn server_starts_and_stops_cleanly() {
    assert_clean_shutdown(
        "server",
        "server-lifecycle",
        &format!(
            r#"
listen = "127.0.0.1:PORT"
secret = "{SECRET_HEX}"
origin_host = "127.0.0.1"
origin_port = 1
"#
        ),
    );
}

#[test]
fn prepare_import_keeps_settings_and_does_not_replace_config() {
    let dir = temp_dir("prepare-import");
    fs::write(dir.join("secret.bin"), [0x91; 32]).unwrap();
    let source = write(
        &dir,
        "server.toml",
        "listen = \"127.0.0.1:9443\"\nsecret_file = \"secret.bin\"\norigin_host = \"example.com\"\n[origin]\nrecord_sizes = [100, 200]\n",
    );
    let destination = dir.join("installed");
    let output = Command::new(bin())
        .arg("prepare")
        .arg(&destination)
        .arg("--import")
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = destination.join("node.toml");
    let settings = fiend_core::config::load_server(&path).unwrap();
    assert_eq!(settings.listen.to_string(), "127.0.0.1:9443");
    assert_eq!(settings.config.secret, vec![0x91; 32]);
    assert_eq!(settings.config.origin.record_sizes, vec![100, 200]);
    let before = fs::read(&path).unwrap();
    let output = Command::new(bin())
        .arg("prepare")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(before, fs::read(&path).unwrap());
    let output = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn prepare_new_node_and_reject_invalid_input() {
    let dir = temp_dir("prepare-new");
    let destination = dir.join("installed");
    let output = Command::new(bin())
        .arg("prepare")
        .arg(&destination)
        .args(["--port", "8443"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let settings = fiend_core::config::load_server(destination.join("node.toml")).unwrap();
    assert_eq!(settings.listen.port(), 8443);
    assert_eq!(settings.config.secret.len(), 32);
    for port in ["0", "65536", "abc"] {
        let output = Command::new(bin())
            .arg("prepare")
            .arg(dir.join(port))
            .args(["--port", port])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!dir.join(port).exists());
    }
    let output = Command::new(bin())
        .arg("check")
        .arg(dir.join("missing.toml"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output = Command::new(bin()).arg("generate-secret").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(hex::decode(text.trim()).unwrap().len(), 32);
}

#[test]
fn invalid_config_exits_with_an_error() {
    let dir = temp_dir("invalid-config");
    let path = write(
        &dir,
        "client.toml",
        &format!(
            r#"
server = "127.0.0.1:1"
secret = "{SECRET_HEX}"
"#
        ),
    );
    let output = Command::new(bin())
        .args(["client", path.to_str().expect("config path")])
        .output()
        .expect("run process");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("client template is required"),
        "stderr: {stderr}"
    );
}

#[test]
fn unknown_command_exits_with_an_error() {
    let output = Command::new(bin())
        .arg("unknown")
        .output()
        .expect("run process");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown command"), "stderr: {stderr}");
}

#[test]
fn no_command_prints_usage() {
    let output = Command::new(bin()).output().expect("run process");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: fiend-core"), "stdout: {stdout}");
}
