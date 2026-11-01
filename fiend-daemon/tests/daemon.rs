#![cfg(any(unix, windows))]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fiend_daemon::peer::{PeerPolicy, SystemDirectory};
use fiend_daemon::platform::NullPlatform;
use fiend_daemon::{Daemon, LogBuffer, Store, bind_socket, serve};
use fiend_ipc::{
    BackupData, Client, ClientError, ConnectStep, ErrorCode, Event, Profile, ProfileInput, Request,
    RuleKind, Settings, TunnelState,
};
use tokio_util::sync::CancellationToken;

const TEMPLATE: &str = include_str!("../../fiend-core/testdata/template.json");
const SECRET: [u8; 32] = [0x42; 32];

struct Fixture {
    dir: PathBuf,
    socket: PathBuf,
    shutdown: CancellationToken,
    daemon: Daemon<NullPlatform>,
}

impl Fixture {
    async fn new(name: &str, policy: PeerPolicy) -> Self {
        let dir = std::env::temp_dir().join(format!("fiend-it-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        let socket = dir.join("daemon.sock");
        #[cfg(windows)]
        let socket = PathBuf::from(format!(r"\\.\pipe\fiend-it-{name}-{}", std::process::id()));
        let store = Store::open(dir.join("store")).unwrap();
        let daemon = Daemon::new(NullPlatform, store, Arc::new(LogBuffer::new()));
        let listener = bind_socket(&socket, None, &SystemDirectory).unwrap();
        let shutdown = CancellationToken::new();
        tokio::spawn(serve(daemon.clone(), listener, policy, shutdown.clone()));
        Self {
            dir,
            socket,
            shutdown,
            daemon,
        }
    }

    async fn allowed(name: &str) -> Self {
        let fixture = Self::new(name, policy()).await;
        fixture.daemon.allow_local_subscription_urls();
        fixture
    }

    async fn client(&self) -> Client {
        Client::connect(&self.socket, "test").await.unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.cancel();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn policy() -> PeerPolicy {
    #[cfg(unix)]
    {
        PeerPolicy {
            uids: vec![current_uid()],
            ..PeerPolicy::default()
        }
    }
    #[cfg(windows)]
    {
        PeerPolicy::default()
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(std::env::temp_dir()).unwrap().uid()
}

#[cfg(unix)]
async fn raw_connect(path: &std::path::Path) -> tokio::net::UnixStream {
    tokio::net::UnixStream::connect(path).await.unwrap()
}

#[cfg(windows)]
async fn raw_connect(path: &std::path::Path) -> tokio::net::windows::named_pipe::NamedPipeClient {
    use tokio::net::windows::named_pipe::ClientOptions;

    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..20 {
        match ClientOptions::new().open(path.as_os_str()) {
            Ok(client) => return client,
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("cannot connect to the test pipe: {error}"),
        }
    }
    panic!("the test pipe stayed busy");
}

fn managed_server(
    secret: &[u8],
    config: fiend_core::server::ServerConfig,
) -> fiend_core::server::Server {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "fiend-daemon-test-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let mut state = fiend_core::node::DesiredState::initial(&config);
    state.users.push(fiend_core::node::User {
        id: "test".into(),
        secret: secret.iter().map(|byte| format!("{byte:02x}")).collect(),
        enabled: true,
        allowed: true,
        expire_at: None,
        traffic_limit: None,
        used_traffic: 0,
        usage_epoch: 0,
    });
    let node = fiend_core::node::Node::load(&path, vec![0x5c; 32], state).unwrap();
    fiend_core::server::Server::managed(config, node)
}

async fn fiend_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = managed_server(
        &SECRET,
        fiend_core::server::ServerConfig::new("127.0.0.1", 443),
    );
    tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    address
}

fn input(name: &str, server: &str) -> ProfileInput {
    ProfileInput {
        id: None,
        name: name.into(),
        servers: vec![server.into()],
        secret: "42".repeat(32),
        template_json: TEMPLATE.into(),
        tuning: Default::default(),
    }
}

async fn wait_for(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    wanted: impl Fn(&TunnelState) -> bool,
) -> fiend_ipc::Status {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Event::State(status) = events.recv().await.unwrap()
                && wanted(&status.state)
            {
                return status;
            }
        }
    })
    .await
    .expect("state did not arrive")
}

fn http_server(body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    format!("http://{address}/list")
}

#[tokio::test]
async fn connect_and_disconnect_follow_the_state_machine() {
    let fixture = Fixture::allowed("cycle").await;
    let server = fiend_server().await;
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Local", &server.to_string()))
        .await
        .unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    assert_eq!(
        client.status().await.unwrap().state,
        TunnelState::Disconnected
    );

    client
        .connect_profile(Some(profile.id.clone()))
        .await
        .unwrap();
    let connected = wait_for(&mut events, |s| matches!(s, TunnelState::Connected { .. })).await;
    assert_eq!(connected.profile_id.as_deref(), Some(profile.id.as_str()));
    assert!(!connected.restart_required);
    assert_eq!(
        client.settings().await.unwrap().active_profile.as_deref(),
        Some(profile.id.as_str())
    );

    client.disconnect().await.unwrap();
    let status = client.status().await.unwrap();
    assert_eq!(status.state, TunnelState::Disconnected);
    assert!(status.profile_id.is_none());
    client.disconnect().await.unwrap();
}

#[tokio::test]
async fn steps_are_reported_in_order() {
    let fixture = Fixture::allowed("steps").await;
    let server = fiend_server().await;
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Local", &server.to_string()))
        .await
        .unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    client.connect_profile(Some(profile.id)).await.unwrap();
    let mut seen = Vec::new();
    loop {
        let status = wait_for(&mut events, |_| true).await;
        match status.state {
            TunnelState::Connecting { step } => seen.push(step),
            TunnelState::Connected { .. } => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        seen,
        [ConnectStep::Server, ConnectStep::Routes, ConnectStep::Dns]
    );
}

#[tokio::test]
async fn failed_connect_reports_a_reason_and_allows_retry() {
    let fixture = Fixture::allowed("failed").await;
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Closed", &address.to_string()))
        .await
        .unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    client
        .connect_profile(Some(profile.id.clone()))
        .await
        .unwrap();
    let status = wait_for(&mut events, |s| matches!(s, TunnelState::Failed { .. })).await;
    assert!(matches!(status.state, TunnelState::Failed { reason } if !reason.is_empty()));
    client.connect_profile(Some(profile.id)).await.unwrap();
    wait_for(&mut events, |s| matches!(s, TunnelState::Failed { .. })).await;
    client.disconnect().await.unwrap();
    assert_eq!(
        client.status().await.unwrap().state,
        TunnelState::Disconnected
    );
}

#[tokio::test]
async fn choosing_another_profile_reconnects() {
    let fixture = Fixture::allowed("switch").await;
    let (a, b) = (fiend_server().await, fiend_server().await);
    let client = fixture.client().await;
    let first = client
        .save_profile(input("A", &a.to_string()))
        .await
        .unwrap();
    let second = client
        .save_profile(input("B", &b.to_string()))
        .await
        .unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    client
        .connect_profile(Some(first.id.clone()))
        .await
        .unwrap();
    wait_for(&mut events, |s| matches!(s, TunnelState::Connected { .. })).await;
    client
        .connect_profile(Some(second.id.clone()))
        .await
        .unwrap();
    wait_for(&mut events, |s| matches!(s, TunnelState::Disconnecting)).await;
    let status = wait_for(&mut events, |s| matches!(s, TunnelState::Connected { .. })).await;
    assert_eq!(status.profile_id.as_deref(), Some(second.id.as_str()));
    let error = client.delete_profile(second.id.clone()).await.unwrap_err();
    assert!(matches!(error, ClientError::Daemon(e) if e.code == ErrorCode::Busy));
    client.delete_profile(first.id).await.unwrap();
}

#[tokio::test]
async fn settings_change_marks_the_session_as_stale() {
    let fixture = Fixture::allowed("stale").await;
    let server = fiend_server().await;
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Local", &server.to_string()))
        .await
        .unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    client.connect_profile(Some(profile.id)).await.unwrap();
    wait_for(&mut events, |s| matches!(s, TunnelState::Connected { .. })).await;
    let mut settings = client.settings().await.unwrap();
    settings.mtu = 1400;
    client.set_settings(settings).await.unwrap();
    assert!(client.status().await.unwrap().restart_required);
    client
        .set_rule_text(RuleKind::Direct, "domain:example.com\n".into())
        .await
        .unwrap();
    client.disconnect().await.unwrap();
    assert!(!client.status().await.unwrap().restart_required);
}

#[tokio::test]
async fn backup_restore_replaces_profiles_and_settings() {
    let fixture = Fixture::allowed("restore").await;
    let client = fixture.client().await;
    let old = client
        .save_profile(input("Old", "127.0.0.1:1"))
        .await
        .unwrap();
    let id = "ab".repeat(16);
    let replacement = Profile {
        id: id.clone(),
        name: "Restored".into(),
        servers: vec!["127.0.0.1:8443".into()],
        secret: "cd".repeat(32),
        template_json: TEMPLATE.into(),
        tuning: Default::default(),
    };
    let settings = Settings {
        active_profile: Some(id.clone()),
        direct_rules: "domain:example.com\n".into(),
        mtu: 1400,
        ..Settings::default()
    };
    client
        .request(Request::RestoreBackup {
            data: BackupData {
                profiles: vec![replacement.clone()],
                settings: settings.clone(),
            },
        })
        .await
        .unwrap();
    assert_eq!(
        client.list_profiles().await.unwrap(),
        vec![replacement.clone()]
    );
    assert_eq!(client.settings().await.unwrap(), settings);
    assert_ne!(old.id, replacement.id);
}

#[tokio::test]
async fn backup_restore_refuses_invalid_ids_and_keeps_the_old_data() {
    let fixture = Fixture::allowed("restore-bad").await;
    let client = fixture.client().await;
    let old = client
        .save_profile(input("Old", "127.0.0.1:1"))
        .await
        .unwrap();
    let bad = Profile {
        id: "not-a-profile-id".into(),
        name: "Bad".into(),
        servers: vec!["127.0.0.1:8443".into()],
        secret: "cd".repeat(32),
        template_json: TEMPLATE.into(),
        tuning: Default::default(),
    };
    assert_eq!(
        code(
            client
                .request(Request::RestoreBackup {
                    data: BackupData {
                        profiles: vec![bad],
                        settings: Settings::default(),
                    },
                })
                .await
        ),
        ErrorCode::InvalidProfile
    );
    assert_eq!(client.list_profiles().await.unwrap(), vec![old]);
}

fn code<T>(result: Result<T, ClientError>) -> ErrorCode {
    match result {
        Err(ClientError::Daemon(error)) => error.code,
        Ok(_) => panic!("accepted"),
        Err(other) => panic!("{other}"),
    }
}

#[tokio::test]
async fn bad_input_is_refused_with_a_code() {
    let fixture = Fixture::allowed("bad").await;
    let client = fixture.client().await;
    assert_eq!(
        code(client.connect_profile(None).await),
        ErrorCode::InvalidRequest
    );
    assert!(client.list_profiles().await.unwrap().is_empty());
    assert_eq!(
        code(client.connect_profile(Some("none".into())).await),
        ErrorCode::NotFound
    );
    let mut bad = input("Bad", "127.0.0.1:1");
    bad.secret = "short".into();
    assert_eq!(
        code(client.save_profile(bad).await),
        ErrorCode::InvalidProfile
    );
    assert_eq!(
        code(client.import_link("fiend://!".into()).await),
        ErrorCode::InvalidProfile
    );
    let settings = Settings {
        mtu: 100,
        ..Settings::default()
    };
    assert_eq!(
        code(client.set_settings(settings).await),
        ErrorCode::InvalidRequest
    );
    let settings = Settings {
        direct_rules: "regexp:x".into(),
        ..Settings::default()
    };
    assert_eq!(
        code(client.set_settings(settings).await),
        ErrorCode::InvalidRules
    );
    assert_eq!(
        code(
            client
                .set_rule_text(RuleKind::Block, "ip:1.2.3.0/24".into())
                .await
        ),
        ErrorCode::InvalidRules
    );
    assert_eq!(
        code(
            client
                .set_rule_text(RuleKind::Dpi, "bad entry".into())
                .await
        ),
        ErrorCode::InvalidRules
    );
    let settings = Settings {
        dpi_rules: "regexp:x".into(),
        ..Settings::default()
    };
    assert_eq!(
        code(client.set_settings(settings).await),
        ErrorCode::InvalidRules
    );
    client
        .set_rule_text(RuleKind::Dpi, "domain:youtube.com\nip:1.2.3.0/24\n".into())
        .await
        .unwrap();
    assert_eq!(
        client.settings().await.unwrap().dpi_rules,
        "domain:youtube.com\nip:1.2.3.0/24\n"
    );
    assert_eq!(
        code(client.share_link("none".into()).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(client.measure_latency("none".into()).await),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn share_links_round_trip_and_latency_uses_the_server() {
    let fixture = Fixture::allowed("link").await;
    let server = fiend_server().await;
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Local", &server.to_string()))
        .await
        .unwrap();
    let link = client.share_link(profile.id.clone()).await.unwrap();
    let imported = client.import_link(link).await.unwrap();
    assert_ne!(imported.id, profile.id);
    assert_eq!(
        (&imported.name, &imported.servers, &imported.secret),
        (&profile.name, &profile.servers, &profile.secret)
    );
    assert_eq!(client.list_profiles().await.unwrap().len(), 2);
    assert!(client.measure_latency(profile.id).await.unwrap() < 10_000);
}

#[tokio::test]
async fn subscriptions_are_downloaded_checked_and_removed() {
    let fixture = Fixture::allowed("subs").await;
    let client = fixture.client().await;
    let url = http_server(b"domain:a.ru\nfull:b.ru\n");
    let subscription = client
        .add_subscription(url, RuleKind::Direct, String::new())
        .await
        .unwrap();
    assert_eq!(subscription.entries, 2);
    assert!(subscription.error.is_none() && subscription.updated_at_ms > 0);
    let bad = http_server(b"regexp:x\n");
    let broken = client
        .add_subscription(bad, RuleKind::Direct, String::new())
        .await
        .unwrap();
    assert!(broken.error.is_some() && broken.entries == 0);
    let settings = client.refresh_subscriptions().await.unwrap();
    assert_eq!(settings.subscriptions.len(), 2);
    let mut changed = settings.clone();
    changed.subscriptions.clear();
    let kept = client.set_settings(changed).await.unwrap();
    assert_eq!(kept.subscriptions.len(), 2);
    client
        .remove_subscription(subscription.id.clone())
        .await
        .unwrap();
    assert!(client.remove_subscription(subscription.id).await.is_err());
    assert!(
        client
            .add_subscription("ftp://x/y".into(), RuleKind::Direct, String::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn subscriptions_are_renamed_and_rechecked_for_a_new_type() {
    let fixture = Fixture::allowed("subs-edit").await;
    let client = fixture.client().await;
    let url = http_server(b"domain:a.ru\nip:1.2.3.0/24\n");
    let added = client
        .add_subscription(url, RuleKind::Proxy, " Mixed ".into())
        .await
        .unwrap();
    assert_eq!((added.name.as_str(), added.entries), ("Mixed", 2));
    let blocked = client
        .update_subscription(added.id.clone(), "Mixed".into(), RuleKind::Block)
        .await
        .unwrap();
    assert_eq!(blocked.kind, RuleKind::Block);
    assert!(blocked.error.is_some());
    let dpi = client
        .update_subscription(added.id.clone(), String::new(), RuleKind::Dpi)
        .await
        .unwrap();
    assert_eq!(
        (dpi.kind, dpi.entries, &dpi.error),
        (RuleKind::Dpi, 2, &None)
    );
    assert_eq!(dpi.title(), "list");
    let saved = client.settings().await.unwrap().subscriptions;
    assert_eq!(saved, vec![dpi]);
    assert!(
        client
            .update_subscription(added.id.clone(), "x".repeat(65), RuleKind::Dpi)
            .await
            .is_err()
    );
    assert!(
        client
            .update_subscription("none".into(), String::new(), RuleKind::Dpi)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_peer_without_permission_is_refused() {
    let fixture = Fixture::new("peer", PeerPolicy::default()).await;
    let error = match Client::connect(&fixture.socket, "test").await {
        Ok(_) => panic!("accepted"),
        Err(error) => error,
    };
    assert!(matches!(error, ClientError::Daemon(e) if e.code == ErrorCode::Unauthorized));
}

#[tokio::test]
async fn an_unknown_protocol_major_is_refused() {
    use fiend_ipc::{ClientMessage, Hello, ServerMessage, read_frame, write_frame};
    let fixture = Fixture::allowed("version").await;
    let mut stream = raw_connect(&fixture.socket).await;
    let mut hello = Hello::new("test");
    hello.major = 99;
    write_frame(&mut stream, &ClientMessage::Hello(hello), 4096)
        .await
        .unwrap();
    let reply = read_frame::<_, ServerMessage>(&mut stream, 4096)
        .await
        .unwrap();
    assert!(matches!(
        reply,
        Some(ServerMessage::Response { result: Err(e), .. }) if e.code == ErrorCode::UnsupportedVersion
    ));
}

#[tokio::test]
async fn garbage_and_oversize_frames_close_only_that_connection() {
    use tokio::io::AsyncWriteExt;
    let fixture = Fixture::allowed("garbage").await;
    let mut raw = raw_connect(&fixture.socket).await;
    raw.write_all(&[0xFF, 0xFF, 0xFF, 0xFF]).await.unwrap();
    let mut buffer = [0u8; 16];
    let read = tokio::io::AsyncReadExt::read(&mut raw, &mut buffer)
        .await
        .unwrap();
    assert_eq!(read, 0);
    let client = fixture.client().await;
    assert_eq!(
        client.status().await.unwrap().state,
        TunnelState::Disconnected
    );
    drop(fixture.daemon.clone());
}

#[tokio::test]
async fn logs_are_listed_and_cleared() {
    let fixture = Fixture::allowed("logs").await;
    let client = fixture.client().await;
    assert!(client.logs(10).await.unwrap().is_empty());
    client.clear_logs().await.unwrap();
    let reply = client.request(Request::Logs { limit: 5 }).await.unwrap();
    assert!(matches!(reply, fiend_ipc::Reply::Logs { entries } if entries.is_empty()));
}

#[tokio::test]
async fn connect_at_startup_connects_the_active_profile() {
    let fixture = Fixture::allowed("auto").await;
    let server = fiend_server().await;
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Local", &server.to_string()))
        .await
        .unwrap();
    let mut settings = client.settings().await.unwrap();
    settings.active_profile = Some(profile.id);
    settings.connect_at_startup = true;
    client.set_settings(settings).await.unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    fixture.daemon.start().await;
    wait_for(&mut events, |s| matches!(s, TunnelState::Connected { .. })).await;
    fixture.daemon.shutdown().await;
    assert_eq!(
        client.status().await.unwrap().state,
        TunnelState::Disconnected
    );
}

#[tokio::test]
async fn rule_lists_on_this_computer_are_refused() {
    let fixture = Fixture::new("strict", policy()).await;
    let client = fixture.client().await;
    let result = client
        .add_subscription(
            "http://127.0.0.1:9/list".into(),
            RuleKind::Direct,
            String::new(),
        )
        .await;
    assert_eq!(code(result), ErrorCode::InvalidRules);
    assert!(client.settings().await.unwrap().subscriptions.is_empty());
}

#[tokio::test]
async fn connect_without_a_profile_uses_the_first_one() {
    let fixture = Fixture::allowed("first").await;
    let server = fiend_server().await;
    let client = fixture.client().await;
    let profile = client
        .save_profile(input("Only", &server.to_string()))
        .await
        .unwrap();
    client.subscribe().await.unwrap();
    let mut events = client.events();
    client.connect_profile(None).await.unwrap();
    let status = wait_for(&mut events, |s| matches!(s, TunnelState::Connected { .. })).await;
    assert_eq!(status.profile_id.as_deref(), Some(profile.id.as_str()));
}
