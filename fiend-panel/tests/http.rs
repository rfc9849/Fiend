use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use fiend_panel::{Config, router, valid_host};
use tower::ServiceExt;

fn config() -> Config {
    Config {
        listen: "127.0.0.1:8080".parse().unwrap(),
        setup_token: "ab".repeat(32),
        setup_expires_at: u64::MAX,
    }
}

fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("fiend-panel-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn loopback_only() {
    for address in [
        "0.0.0.0:8080",
        "192.168.1.1:8080",
        "[::]:8080",
        "[::1]:8080",
        "127.0.0.2:8080",
        "127.0.0.1:0",
    ] {
        let mut config = config();
        config.listen = address.parse::<SocketAddr>().unwrap();
        assert!(config.validate().is_err(), "{address}");
    }
    assert!(config().validate().is_ok());
}

#[test]
fn host_allowlist() {
    for host in ["127.0.0.1:8080", "localhost:8080"] {
        assert!(valid_host(host));
    }
    for host in [
        "127.0.0.1",
        "localhost:",
        "localhost:0",
        "localhost:65536",
        "localhost:+80",
        "localhost:80:90",
        "127.0.0.1.evil:8080",
        "evil:8080",
        "localhost@evil:8080",
        "[::1]:8080",
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn http_security_and_assets() {
    let app = router(config());
    for (path, status, mime) in [
        ("/", StatusCode::OK, "text/html; charset=utf-8"),
        ("/health", StatusCode::OK, "text/plain; charset=utf-8"),
        ("/missing.js", StatusCode::NOT_FOUND, ""),
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
            let body = to_bytes(response.into_body(), 100_000).await.unwrap();
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
    for host in [None, Some("evil:8080")] {
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
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn setup_token_expires() {
    for expires in [0, u64::MAX] {
        let mut config = config();
        let path = format!("/setup/{}", config.setup_token);
        config.setup_expires_at = expires;
        let response = router(config)
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
    }
}
