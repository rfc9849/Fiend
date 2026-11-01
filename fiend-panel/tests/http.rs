use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use fiend_panel::{App, Config, router, valid_host};
use tower::ServiceExt;

fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "fiend-panel-{}-{name}-{}",
        std::process::id(),
        hex::encode(rand::random::<[u8; 4]>())
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn config(dir: &std::path::Path) -> Config {
    Config {
        listen: "127.0.0.1:8080".parse().unwrap(),
        setup_token: "ab".repeat(32),
        setup_expires_at: u64::MAX,
        database: dir.join("panel.db"),
        local_node: dir.join("node.toml"),
    }
}

#[test]
fn listen_address() {
    let dir = temp("listen");
    for address in ["0.0.0.0:0", "127.0.0.1:0", "[::]:0"] {
        let mut config = config(&dir);
        config.listen = address.parse::<SocketAddr>().unwrap();
        assert!(config.validate().is_err(), "{address}");
    }
    for address in [
        "0.0.0.0:8080",
        "192.168.1.1:8080",
        "[::]:8080",
        "[::1]:8080",
        "127.0.0.1:8080",
    ] {
        let mut config = config(&dir);
        config.listen = address.parse::<SocketAddr>().unwrap();
        assert!(config.validate().is_ok(), "{address}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn host_allowlist() {
    for host in [
        "127.0.0.1:8080",
        "localhost:8080",
        "127.0.0.1",
        "203.0.113.7:8080",
        "panel.example.com",
        "panel.example.com:8443",
        "[::1]:8080",
        "[2001:db8::1]",
    ] {
        assert!(valid_host(host), "{host}");
    }
    for host in [
        "",
        ":8080",
        "localhost:",
        "localhost:0",
        "localhost:65536",
        "localhost:+80",
        "localhost:80:90",
        "localhost@evil:8080",
        "panel example.com",
        "[::1",
        "[::1]8080",
        "[evil]:8080",
        "2001:db8::1",
    ] {
        assert!(!valid_host(host), "{host}");
    }
}

#[test]
fn init_is_private_and_keeps_existing_config() {
    let dir = temp("init");
    let path = dir.join("panel.toml");
    let first = Config::init(&path).unwrap();
    let raw = fs::read(&path).unwrap();
    let second = Config::init(&path).unwrap();
    assert_eq!(first.setup_token, second.setup_token);
    assert_eq!(raw, fs::read(&path).unwrap());
    first.validate().unwrap();
    assert!(first.listen.ip().is_unspecified());
    assert_eq!(first.listen.port(), fiend_panel::PANEL_PORT);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::write(
        &path,
        "listen = \"127.0.0.1:9\"\nsetup_token = \"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd\"\nsetup_expires_at = 1\n",
    )
    .unwrap();
    let old = Config::load(&path).unwrap();
    assert_eq!(old.database, PathBuf::from("/var/lib/fiend/panel.db"));
    let renewed = Config::renew_setup(&path).unwrap();
    assert_ne!(renewed.setup_token, old.setup_token);
    assert!(renewed.setup_expires_at > 1);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn http_security_and_assets() {
    let dir = temp("assets");
    let app = router(App::open(config(&dir)).await.unwrap());
    for (path, status, mime) in [
        ("/", StatusCode::OK, "text/html; charset=utf-8"),
        ("/users", StatusCode::OK, "text/html; charset=utf-8"),
        ("/health", StatusCode::OK, "text/plain; charset=utf-8"),
        ("/missing.js", StatusCode::NOT_FOUND, ""),
        ("/elsewhere", StatusCode::NOT_FOUND, ""),
        ("/setup", StatusCode::NOT_FOUND, ""),
        ("/setup/wrong", StatusCode::NOT_FOUND, ""),
        ("/api/missing", StatusCode::NOT_FOUND, ""),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(
            response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .starts_with("default-src 'self'")
        );
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        if !mime.is_empty() {
            assert_eq!(response.headers()["content-type"], mime);
        }
        if path == "/" {
            let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
            let text = String::from_utf8(body.to_vec()).unwrap();
            assert!(!text.contains("<script>"));
            assert!(text.contains("src=\"/bootstrap."));
            let script = text
                .split("src=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(script)
                        .header("host", "localhost:8080")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["content-type"],
                "text/javascript; charset=utf-8"
            );
        }
    }
    for (origin, status) in [
        ("http://panel.example.com:8443", StatusCode::NOT_FOUND),
        ("https://panel.example.com:8443", StatusCode::NOT_FOUND),
        ("http://evil.example", StatusCode::FORBIDDEN),
        ("https://panel.example.com", StatusCode::FORBIDDEN),
        ("ftp://panel.example.com:8443", StatusCode::FORBIDDEN),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("host", "panel.example.com:8443")
                    .header("origin", origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{origin}");
    }
    for host in [None, Some("bad host:8080")] {
        let mut builder = Request::builder().uri("/");
        if let Some(host) = host {
            builder = builder.header("host", host);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn setup_token_expires() {
    for expires in [0, u64::MAX] {
        let dir = temp("expires");
        let mut config = config(&dir);
        let path = format!("/setup/{}", config.setup_token);
        config.setup_expires_at = expires;
        let response = router(App::open(config).await.unwrap())
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "localhost:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if expires == 0 {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
