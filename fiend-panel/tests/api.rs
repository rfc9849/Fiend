#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use fiend_core::addr::Address;
use fiend_core::client::{Client, ClientConfig};
use fiend_core::node::{DesiredState, Node};
use fiend_core::server::Server;
use fiend_core::target::TargetPolicy;
use fiend_panel::{App, Config, router};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const TOKEN: &str = "abababababababababababababababababababababababababababababababab";
const CONTROL: [u8; 32] = [0x6b; 32];

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path =
            PathBuf::from("/tmp").join(format!("fpa-{}", hex::encode(rand::random::<[u8; 5]>())));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Panel {
    app: App,
    router: Router,
    cookie: Option<String>,
    csrf: Option<String>,
}

impl Panel {
    async fn new(dir: &Path) -> Self {
        let config = Config {
            listen: "127.0.0.1:8080".parse().unwrap(),
            setup_token: TOKEN.into(),
            setup_expires_at: u64::MAX,
            database: dir.join("panel.db"),
            local_node: dir.join("node.toml"),
        };
        let app = App::open(config).await.unwrap();
        Self {
            router: router(app.clone()),
            app,
            cookie: None,
            csrf: None,
        }
    }

    async fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        csrf: bool,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8080")
            .header("content-type", "application/json");
        if let Some(cookie) = &self.cookie {
            builder = builder.header("cookie", format!("fiend_session={cookie}"));
        }
        if csrf && let Some(token) = &self.csrf {
            builder = builder.header("x-csrf-token", token);
        }
        let body = body
            .map(|body| Body::from(body.to_string()))
            .unwrap_or_else(Body::empty);
        let response = self
            .router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        if let Some(cookie) = response.headers().get("set-cookie") {
            let text = cookie.to_str().unwrap();
            assert!(text.contains("HttpOnly") && text.contains("SameSite=Strict"));
            let value = text
                .split(';')
                .next()
                .unwrap()
                .split_once('=')
                .unwrap()
                .1
                .to_owned();
            self.cookie = (!value.is_empty()).then_some(value);
        }
        let bytes = to_bytes(response.into_body(), 10_000_000).await.unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn get(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("GET", path, None, false).await
    }

    async fn post(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", path, Some(body), true).await
    }

    async fn put(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", path, Some(body), true).await
    }

    async fn del(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("DELETE", path, None, true).await
    }

    async fn refresh(&mut self) {
        let (status, session) = self.get("/api/session").await;
        assert_eq!(status, StatusCode::OK);
        self.csrf = Some(session["csrf"].as_str().unwrap().to_owned());
    }

    async fn setup(&mut self) {
        let (status, _) = self
            .post("/api/setup", json!({"token": TOKEN, "name": "owner", "password": "a long test password", "totp_secret": null, "totp_code": null}))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        self.refresh().await;
    }
}

fn totp_code(secret: &str) -> String {
    let bytes = fiend_panel::auth::decode_base32(secret).unwrap();
    let totp = totp_rs::Builder::new()
        .with_secret(totp_rs::Secret::new(bytes.into_boxed_slice()))
        .with_account_name("test")
        .build()
        .unwrap();
    totp.generate_current().to_string()
}

#[tokio::test]
async fn setup_is_one_time_and_validates_input() {
    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    assert_eq!(
        panel.get(&format!("/api/setup/{TOKEN}")).await.0,
        StatusCode::OK
    );
    assert_eq!(panel.get("/api/setup/00").await.0, StatusCode::NOT_FOUND);
    let (status, body) = panel
        .post("/api/setup", json!({"token": TOKEN, "name": "owner", "password": "short", "totp_secret": null, "totp_code": null}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["field"], "password");
    let (status, totp) = panel.get(&format!("/api/setup/{TOKEN}/totp")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(totp["uri"].as_str().unwrap().starts_with("otpauth://totp/"));
    assert!(totp["qr_svg"].as_str().unwrap().starts_with("<svg"));
    let (status, body) = panel
        .post("/api/setup", json!({"token": TOKEN, "name": "owner", "password": "a long test password", "totp_secret": totp["secret"], "totp_code": "000000"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "totp_code");
    let (status, _) = panel
        .post("/api/setup", json!({"token": "cd".repeat(32), "name": "owner", "password": "a long test password", "totp_secret": null, "totp_code": null}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let code = totp_code(totp["secret"].as_str().unwrap());
    let (status, _) = panel
        .post("/api/setup", json!({"token": TOKEN, "name": "owner", "password": "a long test password", "totp_secret": totp["secret"], "totp_code": code}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    panel.refresh().await;
    let (_, session) = panel.get("/api/session").await;
    assert_eq!(session["admin"]["totp"], true);
    assert_eq!(session["wizard"]["local_node"], false);
    assert_eq!(
        panel.get(&format!("/api/setup/{TOKEN}")).await.0,
        StatusCode::NOT_FOUND
    );
    let page = panel
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/setup/{TOKEN}"))
                .header("host", "localhost:1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    let (status, _) = panel
        .post("/api/setup", json!({"token": TOKEN, "name": "second", "password": "a long test password", "totp_secret": null, "totp_code": null}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    panel.cookie = None;
    let (status, _) = panel
        .post(
            "/api/login",
            json!({"name": "owner", "password": "wrong password value"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = panel
        .post(
            "/api/login",
            json!({"name": "owner", "password": "a long test password"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let challenge = body["challenge"].as_str().unwrap();
    assert!(panel.cookie.is_none());
    assert_eq!(panel.get("/api/session").await.0, StatusCode::UNAUTHORIZED);

    let (status, body) = panel
        .post(
            "/api/login/totp",
            json!({"challenge": challenge, "code": "not a code"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["field"], "code");
    assert!(panel.cookie.is_none());

    let code = totp_code(totp["secret"].as_str().unwrap());
    let verification = json!({"challenge": challenge, "code": code});
    let (status, _) = panel.post("/api/login/totp", verification.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(panel.get("/api/session").await.0, StatusCode::OK);

    panel.cookie = None;
    let (status, _) = panel.post("/api/login/totp", verification).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(panel.cookie.is_none());

    let (status, _) = panel
        .post(
            "/api/login",
            json!({"name": "owner", "password": "a long test password", "totp_code": code}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a TOTP code works one time"
    );
}

#[tokio::test]
async fn sessions_csrf_origin_and_rate_limits() {
    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    assert_eq!(panel.get("/api/session").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(panel.get("/api/users").await.0, StatusCode::UNAUTHORIZED);
    panel.setup().await;
    panel.cookie = None;
    let (status, _) = panel
        .post(
            "/api/login",
            json!({"name": "owner", "password": "a long test password"}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    panel.refresh().await;

    let user = json!({"name": "alice", "enabled": true, "note": "", "traffic_limit": null, "expire_at": null, "hold_days": null, "reset_period": "none", "nodes": []});
    let (status, _) = panel
        .send("POST", "/api/users", Some(user.clone()), false)
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a change needs the CSRF token"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("host", "127.0.0.1:8080")
        .header("origin", "http://evil.example")
        .header("content-type", "application/json")
        .header(
            "cookie",
            format!("fiend_session={}", panel.cookie.clone().unwrap()),
        )
        .header("x-csrf-token", panel.csrf.clone().unwrap())
        .body(Body::from(user.to_string()))
        .unwrap();
    assert_eq!(
        panel
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let (status, created) = panel.post("/api/users", user.clone()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert!(
        created.get("secret").is_none(),
        "user lists do not hold secrets"
    );
    let (status, body) = panel.post("/api/users", user).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["field"], "name");
    let (status, users) = panel.get("/api/users").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!users.to_string().contains(&"5a".repeat(8)));
    let id = created["id"].as_i64().unwrap();
    let (status, secret) = panel.get(&format!("/api/users/{id}/secret")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(secret["secret"].as_str().unwrap().len(), 64);
    let (_, audit) = panel.get("/api/audit").await;
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "secret shown")
    );
    assert!(
        !audit
            .to_string()
            .contains(secret["secret"].as_str().unwrap())
    );
    let (status, body) = panel.post("/api/admins", json!({"name": "second", "password": "another long password", "current_code": "123456"})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = panel.post("/api/logout", json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(panel.get("/api/session").await.0, StatusCode::UNAUTHORIZED);
    for _ in 0..5 {
        let (status, _) = panel
            .post(
                "/api/login",
                json!({"name": "owner", "password": "wrong password value", "totp_code": null}),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = panel
        .post(
            "/api/login",
            json!({"name": "owner", "password": "a long test password", "totp_code": null}),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let (status, body) = panel.post("/api/users", json!({"name": "", "enabled": true, "note": "", "traffic_limit": -1, "expire_at": null, "hold_days": null, "reset_period": "none", "nodes": []})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

fn write_node_files(dir: &Path, port: u16) {
    fs::write(
        dir.join("control.secret"),
        format!("{}\n", hex::encode(CONTROL)),
    )
    .unwrap();
    fs::write(
        dir.join("node.toml"),
        format!(
            "listen = \"127.0.0.1:{port}\"\norigin_host = \"127.0.0.1\"\norigin_port = 9\n\n[node]\nstate_file = \"state.json\"\ncontrol_secret_file = \"control.secret\"\ncontrol_socket = \"control.sock\"\n",
        ),
    )
    .unwrap();
}

async fn start_node(dir: &Path) -> (std::net::SocketAddr, Node) {
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    write_node_files(dir, address.port());
    let settings = fiend_core::config::load_server(dir.join("node.toml")).unwrap();
    let mut config = settings.config.clone();
    config.origin.record_sizes = vec![35, 3400, 286, 74];
    config.target = TargetPolicy::allow_all();
    let node = Node::load(
        &settings.node.state_file,
        CONTROL.to_vec(),
        DesiredState::initial(&config),
    )
    .unwrap();
    node.set_listen_port(address.port());
    let server = Server::managed(config, node.clone());
    tokio::spawn(async move { server.serve(listener).await });
    let control = node.clone();
    let socket = settings.node.control_socket.clone();
    tokio::spawn(async move { fiend_core::node::control::serve_unix(&socket, control).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    (address, node)
}

async fn echo() -> Address {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 65536];
                while let Ok(count) = stream.read(&mut buffer).await {
                    if count == 0 || stream.write_all(&buffer[..count]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    Address::Ip(address)
}

async fn wait_for(mut check: impl AsyncFnMut() -> bool) -> bool {
    for _ in 0..80 {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[tokio::test]
async fn local_node_sync_links_statistics_and_limits() {
    let dir = Temp::new();
    let (node_address, node) = start_node(&dir.0).await;
    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    let (status, defaults) = panel.get("/api/wizard/local").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(defaults["available"], true, "{defaults}");
    assert_eq!(defaults["port"], node_address.port());
    let (status, body) = panel
        .post(
            "/api/wizard/local",
            json!({"name": "main", "address": "bad address"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["field"], "address");
    let (status, created) = panel
        .post(
            "/api/wizard/local",
            json!({"name": "main", "address": "127.0.0.1"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let node_id = created["id"].as_i64().unwrap();
    let (_, users) = panel.get("/api/users").await;
    assert_eq!(users, json!([]));

    let upload = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        include_bytes!("../../fiend-core/testdata/template.json"),
    );
    let (status, uploaded) = panel
        .post(
            "/api/disguises",
            json!({"preset": null, "template_base64": upload}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    let (_, listed) = panel.get("/api/disguises").await;
    let item = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == uploaded["id"])
        .unwrap()
        .clone();
    assert_eq!(
        (
            item["name"].as_str(),
            item["origin_host"].as_str(),
            item["origin_port"].as_i64()
        ),
        (
            Some("www.wikipedia.org"),
            Some("www.wikipedia.org"),
            Some(443)
        )
    );
    assert!(
        item["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["name"] != "Domain" || check["ok"] == true),
        "{item}"
    );
    let (status, _) = panel
        .del(&format!("/api/disguises/{}", uploaded["id"]))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, preset) = panel
        .post(
            "/api/disguises",
            json!({"preset": "wikipedia", "template_base64": null}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let disguise = preset["id"].as_i64().unwrap();
    let (_, listed) = panel.get("/api/disguises").await;
    let listed: Vec<_> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["name"].as_str().unwrap().to_owned(),
                item["origin_host"].as_str().unwrap().to_owned(),
                item["origin_port"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        vec![("Wikipedia".to_owned(), "www.wikipedia.org".to_owned(), 443)]
    );
    for body in [
        json!({"preset": "wikipedia", "template_base64": null}),
        json!({"preset": null, "template_base64": upload}),
    ] {
        let (status, again) = panel.post("/api/disguises", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{again}");
        assert_eq!(
            again["error"],
            "This template is already added as Wikipedia."
        );
    }
    let (status, body) = panel
        .post(
            "/api/disguises",
            json!({"preset": null, "template_base64": "bm90IGEgdGVtcGxhdGU="}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["field"], "template");
    let (status, _) = panel.put(&format!("/api/nodes/{node_id}"), json!({"name": "main", "address": "127.0.0.1", "port": node_address.port(), "disguise_id": disguise})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = panel.put(&format!("/api/nodes/{node_id}"), json!({"name": "main", "address": "127.0.0.1", "port": node_address.port(), "disguise_id": null})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, user) = panel.post("/api/users", json!({"name": "alice", "enabled": true, "note": "test", "traffic_limit": 200000, "expire_at": null, "hold_days": null, "reset_period": "month", "nodes": []})).await;
    assert_eq!(status, StatusCode::OK, "{user}");
    let alice = user["id"].as_i64().unwrap();
    assert!(user["reset_at"].as_i64().unwrap() > fiend_panel::now());
    sqlx::query("UPDATE nodes SET disguise_version = 1000000 WHERE id = ?")
        .bind(node_id)
        .execute(&panel.app.db)
        .await
        .unwrap();
    assert_eq!(
        panel.get(&format!("/api/users/{alice}/link")).await.0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE nodes SET disguise_version = 0 WHERE id = ?")
        .bind(node_id)
        .execute(&panel.app.db)
        .await
        .unwrap();
    let (status, link) = panel.get(&format!("/api/users/{alice}/link")).await;
    assert_eq!(status, StatusCode::OK, "{link}");
    let text = link["link"].as_str().unwrap();
    assert!(
        text.starts_with("fiend://") && text.len() < 1200,
        "{}",
        text.len()
    );
    assert!(fiend_panel::qr::version(text).unwrap() <= 25);
    let share = fiend_core::share::decode(text).unwrap();
    assert_eq!(share.servers, vec![node_address.to_string()]);

    panel.app.start();
    panel.app.sync.ensure(&panel.app, node_id);
    assert!(
        wait_for(async || node
            .desired_state()
            .users
            .iter()
            .any(|user| user.id == format!("u{alice}")))
        .await,
        "the panel applies users"
    );
    assert_eq!(node.desired_state().origin_host, "www.wikipedia.org");

    let mut config =
        ClientConfig::new(String::new(), share.secret.to_vec(), share.template.clone());
    config.servers = share.servers.clone();
    config.mux.attempts = 1;
    let client = Client::new(config);
    let target = echo().await;
    let mut lane = client.open_tcp(&target).await.unwrap();
    let payload = vec![7u8; 50_000];
    lane.write_all(&payload).await.unwrap();
    let mut reply = vec![0u8; payload.len()];
    lane.read_exact(&mut reply).await.unwrap();
    assert!(
        wait_for(async || {
            let (_, user) = panel.get(&format!("/api/users/{alice}")).await;
            user["user"]["used"].as_i64().unwrap_or(0) >= 100_000
        })
        .await,
        "the panel stores usage"
    );
    let (_, traffic) = panel
        .get(&format!("/api/traffic?range=24h&user={alice}"))
        .await;
    let total: i64 = traffic["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| point["up"].as_i64().unwrap() + point["down"].as_i64().unwrap())
        .sum();
    assert!(total >= 100_000, "{total}");
    let (_, dashboard) = panel.get("/api/dashboard").await;
    assert_eq!(dashboard["nodes_online"], 1, "{dashboard}");
    assert_eq!(dashboard["nodes"][0]["status"], "online");
    let mut prober = tokio::net::TcpStream::connect(node_address).await.unwrap();
    prober
        .write_all(b"GET /probe HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    assert!(
        wait_for(async || {
            let (_, report) = panel.get("/api/probes?range=1h").await;
            report["probes"].as_array().is_some_and(|probes| {
                probes.iter().any(|probe| {
                    probe["peer"] == "127.0.0.1"
                        && probe["reason"] == "not_tls"
                        && probe["head"].as_str().is_some_and(|head| {
                            head.starts_with(&hex::encode("GET /probe HTTP/1.1"))
                        })
                })
            }) && report["peers"][0]["peer"] == "127.0.0.1"
        })
        .await,
        "the panel stores probes"
    );
    drop(prober);
    lane.write_all(&vec![1u8; 150_000]).await.unwrap();
    let mut rest = vec![0u8; 150_000];
    let _ = tokio::time::timeout(Duration::from_secs(5), lane.read_exact(&mut rest)).await;
    assert!(
        wait_for(async || {
            let (_, user) = panel.get(&format!("/api/users/{alice}")).await;
            user["user"]["state"] == "limited"
        })
        .await,
        "the user reaches the limit"
    );
    assert!(
        wait_for(async || node
            .desired_state()
            .users
            .iter()
            .any(|user| user.id == format!("u{alice}") && !user.enabled))
        .await
    );
    assert!(
        client.open_tcp(&target).await.is_err(),
        "a limited user goes to the origin"
    );
    let (status, reset) = panel
        .post(&format!("/api/users/{alice}/reset"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reset["used"], 0);
    assert!(
        wait_for(async || node
            .desired_state()
            .users
            .iter()
            .any(|user| user.id == format!("u{alice}") && user.enabled))
        .await
    );
    client.close();

    let (status, body) = panel.post("/api/outbounds", json!({"node_id": null, "name": "proxy", "kind": "socks5", "server": "bad", "username": null, "password": null, "udp": false, "link_node_id": null, "udp_fallback": null})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["field"], "server");
    let (status, _) = panel.post("/api/outbounds", json!({"node_id": null, "name": "proxy", "kind": "http", "server": "127.0.0.1:3128", "username": "user", "password": "pass", "udp": false, "link_node_id": null, "udp_fallback": null})).await;
    assert_eq!(status, StatusCode::OK);
    let (_, routing) = panel.get("/api/routing").await;
    assert_eq!(routing["outbounds"][0]["has_password"], true);
    assert!(!routing.to_string().contains("\"pass\""));
    let (status, body) = panel
        .post(
            "/api/rules",
            json!({"node_id": null, "pattern": "regexp:x", "outbound": "proxy"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _) = panel
        .post(
            "/api/rules",
            json!({"node_id": null, "pattern": "domain:example.com", "outbound": "proxy"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        wait_for(async || node
            .desired_state()
            .rules
            .iter()
            .any(|rule| rule.outbound == "proxy"))
        .await
    );
    let (status, _) = panel
        .send(
            "DELETE",
            &format!("/api/outbounds/{}", routing["outbounds"][0]["id"]),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "an outbound in use stays");

    let (status, logs) = panel.get(&format!("/api/nodes/{node_id}/logs")).await;
    assert_eq!(status, StatusCode::OK, "{logs}");
    let (status, sessions) = panel.get(&format!("/api/users/{alice}/sessions")).await;
    assert_eq!(status, StatusCode::OK, "{sessions}");
    let (status, closed) = panel.del(&format!("/api/users/{alice}/sessions")).await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    assert!(closed[0]["closed"].is_u64() && closed[0]["error"].is_null());
}

#[tokio::test]
async fn warp_rules_enable_the_tunnel_and_fail_closed() {
    use fiend_core::node::outbound::{Context, OutboundKind, Router};

    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    let id: i64 = sqlx::query_scalar("INSERT INTO nodes (name, address, port, local, control_secret, service_secret, created_at) VALUES ('main', '127.0.0.1', 1, 0, '', '', 0) RETURNING id")
        .fetch_one(&panel.app.db)
        .await
        .unwrap();

    let mut ids = Vec::new();
    for (domain, action, udp) in [
        ("block.example", "@block", None),
        ("warp.example", "@warp", None),
        ("udp.example", "@warp", Some("direct")),
    ] {
        let (status, body) = panel
            .post(
                "/api/rules",
                json!({"node_id": null, "pattern": format!("domain:{domain}"), "outbound": action, "udp_fallback": udp}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        ids.push(body["id"].as_i64().unwrap());
    }
    let (status, warp) = panel.get(&format!("/api/nodes/{id}/warp")).await;
    assert_eq!(status, StatusCode::OK, "{warp}");
    assert_eq!(warp["rule_count"], 2);
    let (outbounds, rules, warp) = fiend_panel::state::routing_for(&panel.app, id)
        .await
        .unwrap();
    assert!(warp.enabled);
    fiend_core::node::outbound::validate(&outbounds, &rules, &warp).unwrap();
    let router = Router::build(&outbounds, &rules, None).unwrap();
    let target = |host: &str| Address::Domain {
        host: host.into(),
        port: 443,
    };
    assert!(matches!(
        router.route(&target("block.example")).config.kind,
        OutboundKind::Block
    ));
    assert!(router.udp_route(&target("warp.example")).is_none());
    assert!(matches!(
        router
            .udp_route(&target("udp.example"))
            .unwrap()
            .config
            .kind,
        OutboundKind::Direct
    ));
    let context = Context {
        policy: TargetPolicy::default(),
        timeout: Duration::from_secs(1),
        warp: None,
        resolver: None,
    };
    let dest = target("warp.example");
    let runtime = router.route(&dest);
    assert!(matches!(runtime.config.kind, OutboundKind::Warp));
    let error = runtime.connect_tcp(&dest, &context).await.err().unwrap();
    assert_eq!(error.to_string(), "WARP is not running");

    for id in &ids[1..] {
        assert_eq!(
            panel.del(&format!("/api/rules/{id}")).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let (_, rules, warp) = fiend_panel::state::routing_for(&panel.app, id)
        .await
        .unwrap();
    assert_eq!(rules.len(), 1);
    assert!(!warp.enabled);
}

#[tokio::test]
async fn routing_migration_preserves_existing_actions_and_udp() {
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

    let dir = Temp::new();
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(dir.0.join("panel.db"))
            .create_if_missing(true),
    )
    .await
    .unwrap();
    fiend_panel::db::MIGRATOR
        .run_direct(Some(2), &mut connection, false)
        .await
        .unwrap();
    sqlx::raw_sql(
        "INSERT INTO outbounds (name, kind, config, udp_fallback) VALUES ('old-warp', 'warp', '{}', 'direct'), ('old-block', 'block', '{}', NULL), ('warp', 'socks5', '{}', NULL);
         INSERT INTO rules (position, pattern, outbound) VALUES (1, 'domain:a.example', 'old-warp'), (2, 'domain:b.example', 'old-block'), (3, 'domain:c.example', 'warp');"
    ).execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();

    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    let (status, routing) = panel.get("/api/routing").await;
    assert_eq!(status, StatusCode::OK, "{routing}");
    assert_eq!(routing["rules"][0]["outbound"], "@warp");
    assert_eq!(routing["rules"][0]["udp_fallback"], "direct");
    assert_eq!(routing["rules"][1]["outbound"], "@block");
    assert_eq!(routing["rules"][2]["outbound"], "warp");
    assert_eq!(routing["outbounds"][0]["name"], "warp");
    assert_eq!(routing["outbounds"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unreachable_nodes_report_errors() {
    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    let id: i64 = sqlx::query_scalar("INSERT INTO nodes (name, address, port, local, control_secret, service_secret, created_at) VALUES ('far', '127.0.0.1', 1, 0, ?, ?, 0) RETURNING id")
        .bind(hex::encode(CONTROL))
        .bind(hex::encode([1u8; 32]))
        .fetch_one(&panel.app.db)
        .await
        .unwrap();
    panel.app.sync.ensure(&panel.app, id);
    assert!(
        wait_for(async || {
            let (_, node) = panel.get(&format!("/api/nodes/{id}")).await;
            node["status"] == "offline" && node["last_error"].is_string()
        })
        .await
    );
    let (status, body) = panel
        .post(&format!("/api/nodes/{id}/restart"), json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let disguise: i64 = sqlx::query_scalar("INSERT INTO disguises (name, origin_host, origin_port, template_raw, created_at) VALUES ('test', 'localhost', 443, X'00', 0) RETURNING id")
        .fetch_one(&panel.app.db).await.unwrap();
    let candidate = json!({"disguise_id": disguise});
    let (status, checks) = panel
        .post(
            &format!("/api/nodes/{id}/disguise/check"),
            candidate.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{checks}");
    assert_eq!(checks["ready"], false);
    let (status, body) = panel
        .put(&format!("/api/nodes/{id}/disguise"), candidate)
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let (_, node) = panel.get(&format!("/api/nodes/{id}")).await;
    assert!(node["disguise_id"].is_null());
    let (status, body) = panel.get("/api/users/999").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, user) = panel
        .post("/api/users", json!({"name": "alice", "enabled": true, "note": "", "traffic_limit": null, "expire_at": null, "hold_days": null, "reset_period": "none", "nodes": []}))
        .await;
    assert_eq!(status, StatusCode::OK, "{user}");
    let (status, closed) = panel
        .del(&format!("/api/users/{}/sessions", user["id"]))
        .await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    assert_eq!(closed[0]["node_id"], id);
    assert!(closed[0]["closed"].is_null() && closed[0]["error"].is_string());
    let (status, audit) = panel.get("/api/audit").await;
    assert_eq!(status, StatusCode::OK, "{audit}");
    assert_eq!(audit[0]["action"], "session close failed");
}

async fn raw(
    router: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    peer: Option<&str>,
    body: Body,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(body).unwrap();
    if let Some(peer) = peer {
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            peer.parse::<std::net::SocketAddr>().unwrap(),
        ));
    }
    router.clone().oneshot(request).await.unwrap()
}

async fn login_from(
    panel: &Panel,
    peer: &str,
    real_ip: Option<&str>,
    password: &str,
) -> StatusCode {
    let mut headers = Vec::new();
    if let Some(ip) = real_ip {
        headers.push(("x-real-ip", ip));
    }
    let body = json!({"name": "owner", "password": password, "totp_code": null}).to_string();
    raw(
        &panel.router,
        "POST",
        "/api/login",
        &headers,
        Some(peer),
        Body::from(body),
    )
    .await
    .status()
}

#[tokio::test]
async fn login_limits_use_the_source_address() {
    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    const GOOD: &str = "a long test password";
    const BAD: &str = "wrong password value";
    for _ in 0..5 {
        assert_eq!(
            login_from(&panel, "203.0.113.1:1000", None, BAD).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_from(&panel, "203.0.113.1:1001", None, GOOD).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login_from(&panel, "203.0.113.2:1000", None, GOOD).await,
        StatusCode::NO_CONTENT,
        "failures from one address do not block other addresses"
    );
    for _ in 0..5 {
        assert_eq!(
            login_from(&panel, "[2001:db8:1:2::1]:1", None, BAD).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_from(&panel, "[2001:db8:1:2::9]:1", None, GOOD).await,
        StatusCode::TOO_MANY_REQUESTS,
        "an IPv6 /64 is one source"
    );
    for index in 0..5 {
        let ip = format!("198.51.100.{index}");
        assert_eq!(
            login_from(&panel, "203.0.113.3:1", Some(&ip), BAD).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_from(&panel, "203.0.113.3:1", Some("198.51.100.99"), GOOD).await,
        StatusCode::TOO_MANY_REQUESTS,
        "a remote peer cannot set X-Real-IP"
    );
    for _ in 0..5 {
        assert_eq!(
            login_from(&panel, "127.0.0.1:1", Some("198.51.100.1"), BAD).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_from(&panel, "127.0.0.1:2", Some("198.51.100.1"), GOOD).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login_from(&panel, "127.0.0.1:2", Some("198.51.100.2"), GOOD).await,
        StatusCode::NO_CONTENT,
        "a local proxy sets the source with X-Real-IP"
    );
}

#[tokio::test]
async fn https_requests_get_a_secure_cookie_and_hsts() {
    let dir = Temp::new();
    let panel = Panel::new(&dir.0).await;
    let body = json!({"token": TOKEN, "name": "owner", "password": "a long test password", "totp_secret": null, "totp_code": null}).to_string();
    let response = raw(
        &panel.router,
        "POST",
        "/api/setup",
        &[("origin", "https://127.0.0.1:8080")],
        None,
        Body::from(body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("; Secure")
    );
    assert!(response.headers().contains_key("strict-transport-security"));
    let login =
        json!({"name": "owner", "password": "a long test password", "totp_code": null}).to_string();
    let response = raw(
        &panel.router,
        "POST",
        "/api/login",
        &[],
        Some("203.0.113.1:1"),
        Body::from(login.clone()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        !response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    assert!(!response.headers().contains_key("strict-transport-security"));
    let response = raw(
        &panel.router,
        "POST",
        "/api/login",
        &[("x-forwarded-proto", "https")],
        Some("203.0.113.1:1"),
        Body::from(login.clone()),
    )
    .await;
    assert!(
        !response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Secure"),
        "a remote peer cannot set the scheme"
    );
    let response = raw(
        &panel.router,
        "POST",
        "/api/login",
        &[("x-forwarded-proto", "https")],
        Some("127.0.0.1:1"),
        Body::from(login),
    )
    .await;
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("; Secure")
    );
    assert!(response.headers().contains_key("strict-transport-security"));
}

#[tokio::test]
async fn restore_needs_the_password_and_codes_are_rate_limited() {
    use base64::Engine;
    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    let (_, totp) = panel.get(&format!("/api/setup/{TOKEN}/totp")).await;
    let secret = totp["secret"].as_str().unwrap().to_owned();
    let (status, _) = panel
        .post("/api/setup", json!({"token": TOKEN, "name": "owner", "password": "a long test password", "totp_secret": secret, "totp_code": totp_code(&secret)}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    panel.refresh().await;
    let cookie = format!("fiend_session={}", panel.cookie.clone().unwrap());
    let csrf = panel.csrf.clone().unwrap();
    let good = base64::engine::general_purpose::STANDARD.encode("a long test password");
    let bad = base64::engine::general_purpose::STANDARD.encode("wrong password value");
    let router = panel.router.clone();
    let restore = async |password: Option<&str>, code: Option<&str>| {
        let mut headers = vec![("cookie", cookie.as_str()), ("x-csrf-token", csrf.as_str())];
        if let Some(password) = password {
            headers.push(("x-current-password", password));
        }
        if let Some(code) = code {
            headers.push(("x-current-code", code));
        }
        let response = raw(
            &router,
            "POST",
            "/api/restore",
            &headers,
            None,
            Body::from(&b"SQLite format 3\0x"[..]),
        )
        .await;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
        (
            status,
            serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
        )
    };
    let (status, body) = restore(None, None).await;
    assert_eq!(
        (status, body["field"].as_str()),
        (StatusCode::BAD_REQUEST, Some("current")),
        "{body}"
    );
    let (status, body) = restore(Some(&bad), Some("123456")).await;
    assert_eq!(
        (status, body["field"].as_str()),
        (StatusCode::BAD_REQUEST, Some("current")),
        "{body}"
    );
    let (status, body) = restore(Some(&good), None).await;
    assert_eq!(
        (status, body["field"].as_str()),
        (StatusCode::BAD_REQUEST, Some("totp_code")),
        "{body}"
    );
    for _ in 0..4 {
        let (status, body) = panel.post("/api/backup", json!({"code": "x"})).await;
        assert_eq!(
            (status, body["field"].as_str()),
            (StatusCode::BAD_REQUEST, Some("code")),
            "{body}"
        );
    }
    let (status, _) = panel
        .post("/api/backup", json!({"code": totp_code(&secret)}))
        .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "five wrong codes block the next check"
    );
    let (status, _) = panel.del("/api/account/totp").await;
    assert_ne!(status, StatusCode::NO_CONTENT);
    let (status, _) = restore(Some(&good), Some(&totp_code(&secret))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    panel.cookie = None;
    let (status, _) = panel
        .post("/api/login", json!({"name": "owner", "password": "a long test password", "totp_code": totp_code(&secret)}))
        .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "sign-in uses the same code limit"
    );
}

#[tokio::test]
async fn restore_rejects_an_incompatible_database() {
    use base64::Engine;
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

    let dir = Temp::new();
    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    let backup = dir.0.join("incompatible.db");
    sqlx::query("VACUUM INTO ?")
        .bind(backup.to_string_lossy().as_ref())
        .execute(&panel.app.db)
        .await
        .unwrap();
    let mut connection =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&backup))
            .await
            .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    let cookie = format!("fiend_session={}", panel.cookie.as_ref().unwrap());
    let csrf = panel.csrf.as_deref().unwrap();
    let password = base64::engine::general_purpose::STANDARD.encode("a long test password");
    let response = raw(
        &panel.router,
        "POST",
        "/api/restore",
        &[
            ("cookie", &cookie),
            ("x-csrf-token", csrf),
            ("x-current-password", &password),
        ],
        None,
        Body::from(fs::read(backup).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        !panel
            .app
            .config
            .database
            .with_extension("db.restore")
            .exists()
    );
    let (status, body) = panel.get("/api/session").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["admin"]["name"], "owner");
}

#[tokio::test]
async fn the_local_node_link_port_follows_the_node_public_port() {
    let dir = Temp::new();
    let (node_address, node) = start_node(&dir.0).await;
    let mut panel = Panel::new(&dir.0).await;
    panel.setup().await;
    let (status, created) = panel
        .post(
            "/api/wizard/local",
            json!({"name": "main", "address": "127.0.0.1"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let node_id = created["id"].as_i64().unwrap();
    let (_, view) = panel.get(&format!("/api/nodes/{node_id}")).await;
    assert_eq!(view["port"], node_address.port(), "{view}");
    node.set_public_port(443);
    assert!(
        wait_for(async || {
            let (_, view) = panel.get(&format!("/api/nodes/{node_id}")).await;
            view["port"] == 443
        })
        .await,
        "the panel takes the public port of the local node"
    );
}
