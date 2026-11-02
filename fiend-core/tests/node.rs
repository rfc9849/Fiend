use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use fiend_core::addr::Address;
use fiend_core::client::{Client, ClientConfig};
use fiend_core::fingerprint::Template;
use fiend_core::node::control::{Call, Client as ControlClient};
use fiend_core::node::{DesiredState, Node, User, unix_time};
use fiend_core::server::{Server, ServerConfig};
use fiend_core::target::TargetPolicy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const TEMPLATE: &str = include_str!("../testdata/template.json");
const CONTROL: [u8; 32] = [0x93; 32];
const ALICE: [u8; 32] = [0x91; 32];
const BOB: [u8; 32] = [0x92; 32];

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let path = root.join(format!(
            "fiend-node-{}",
            hex::encode(rand::random::<[u8; 16]>())
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn user(id: &str, secret: &[u8]) -> User {
    User {
        id: id.into(),
        secret: hex::encode(secret),
        enabled: true,
        allowed: true,
        expire_at: None,
        traffic_limit: None,
        used_traffic: 0,
        usage_epoch: 0,
    }
}

fn initial() -> (ServerConfig, DesiredState) {
    let mut config = ServerConfig::new("127.0.0.1", 1);
    config.origin.record_sizes = vec![35, 3400, 286, 74];
    config.target = TargetPolicy::allow_all();
    let mut state = DesiredState::initial(&config);
    state.users = vec![user("alice", &ALICE), user("bob", &BOB)];
    (config, state)
}

fn client_config(address: std::net::SocketAddr, secret: &[u8]) -> ClientConfig {
    let mut config = ClientConfig::new(
        address.to_string(),
        secret,
        Template::from_json(TEMPLATE).unwrap(),
    );
    config.mux.attempts = 1;
    config.mux.open_timeout = Duration::from_secs(2);
    config
}

#[tokio::test]
async fn two_users_control_state_and_counters() {
    let dir = Temp::new();
    let (config, state) = initial();
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::managed(config, node.clone());
    let task = tokio::spawn(async move { server.serve(listener).await });
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = Address::Ip(echo.local_addr().unwrap());
    let echo_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = echo.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                while let Ok(count) = stream.read(&mut buffer).await {
                    if count == 0 || stream.write_all(&buffer[..count]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let mut control = ControlClient::connect(&client_config(address, &CONTROL))
        .await
        .unwrap();
    assert_eq!(
        control.call(Call::Health).await.unwrap()["state_version"],
        1
    );
    let alice = Client::new(client_config(address, &ALICE));
    let bob = Client::new(client_config(address, &BOB));
    let mut alice_lane = alice.open_tcp(&destination).await.unwrap();
    let mut bob_lane = bob.open_tcp(&destination).await.unwrap();
    for lane in [&mut alice_lane, &mut bob_lane] {
        lane.write_all(b"hello").await.unwrap();
        let mut reply = [0u8; 5];
        lane.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"hello");
    }
    let stats = control.call(Call::Stats).await.unwrap();
    for id in ["alice", "bob"] {
        let stats = stats["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap();
        assert_eq!(stats["up"], 5);
        assert_eq!(stats["down"], 5);
        assert_eq!(stats["open_sessions"], 1);
        assert_eq!(stats["new_sessions"], 1);
    }
    let stats = control.call(Call::Stats).await.unwrap();
    assert!(
        stats["users"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["up"] == 0 && row["down"] == 0 && row["used_traffic"] == 10)
    );
    assert_eq!(
        control
            .call(Call::Sessions {
                user_id: "alice".into()
            })
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut next = node.desired_state();
    next.version = 2;
    next.users[0].enabled = false;
    assert_eq!(
        control.call(Call::ApplyState(next.clone())).await.unwrap()["applied_version"],
        2
    );
    let ended = timeout(Duration::from_secs(2), alice_lane.read_u8())
        .await
        .expect("disabled session closes");
    assert!(ended.is_err());
    bob_lane.write_all(b"works").await.unwrap();
    let mut reply = [0u8; 5];
    bob_lane.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"works");
    assert_eq!(
        control
            .call(Call::KillSessions {
                user_id: "bob".into()
            })
            .await
            .unwrap()["closed"],
        1
    );
    assert!(
        timeout(Duration::from_secs(2), bob_lane.read_u8())
            .await
            .unwrap()
            .is_err()
    );
    assert!(control.call(Call::ApplyState(next)).await.is_err());
    let proxy_with_control = Client::new(client_config(address, &CONTROL));
    let refused = match proxy_with_control.open_tcp(&destination).await {
        Err(_) => true,
        Ok(mut lane) => timeout(Duration::from_secs(5), lane.read_u8())
            .await
            .map_or(true, |read| read.is_err()),
    };
    assert!(refused, "a control session must not carry proxy traffic");
    let mut rpc_with_user = ControlClient::connect(&client_config(address, &BOB))
        .await
        .unwrap();
    assert!(rpc_with_user.call(Call::Health).await.is_err());
    alice.close();
    bob.close();
    proxy_with_control.close();
    task.abort();
    echo_task.abort();
    let restored = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), initial().1).unwrap();
    assert_eq!(restored.desired_state().version, 2);
    assert!(!restored.desired_state().users[0].enabled);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(dir.0.join("state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn disabled_expired_limited_and_unknown_users_reach_origin() {
    let dir = Temp::new();
    let (mut config, mut state) = initial();
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.origin_port = origin.local_addr().unwrap().port();
    state.origin_port = config.origin_port;
    state.users[0].enabled = false;
    state.users[1].expire_at = Some(unix_time());
    let mut limited = user("limited", &[0x94; 32]);
    limited.traffic_limit = Some(10);
    limited.used_traffic = 10;
    state.users.push(limited);
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::managed(config, node.clone());
    let task = tokio::spawn(async move { server.serve(listener).await });
    let template = Template::from_json(TEMPLATE).unwrap();
    for secret in [&ALICE, &BOB, &[0x94; 32], &[0x95; 32]] {
        let hello = template.build(secret).unwrap().record;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(&hello).await.unwrap();
        let (mut forwarded, _) = timeout(Duration::from_secs(2), origin.accept())
            .await
            .unwrap()
            .unwrap();
        let mut received = vec![0u8; hello.len()];
        forwarded.read_exact(&mut received).await.unwrap();
        assert_eq!(received, hello);
    }
    assert_eq!(node.stats().handshakes_origin, 4);
    task.abort();
}

#[tokio::test]
async fn invalid_and_failed_state_do_not_change_live_state_or_usage() {
    let dir = Temp::new();
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), initial().1).unwrap();
    let before = fs::read(dir.0.join("state.json")).unwrap();
    let mut next = node.desired_state();
    next.version += 1;
    next.users[1].secret = next.users[0].secret.clone();
    assert!(node.apply(next).await.is_err());
    assert_eq!(fs::read(dir.0.join("state.json")).unwrap(), before);
    let mut next = node.desired_state();
    next.version += 1;
    next.users[0].used_traffic = 100;
    fs::rename(dir.0.join("state.json"), dir.0.join("saved.json")).unwrap();
    fs::create_dir(dir.0.join("state.json")).unwrap();
    assert!(node.apply(next).await.is_err());
    assert_eq!(node.health().state_version, 1);
    assert_eq!(node.stats().users[0].used_traffic, 0);
}

#[tokio::test]
async fn idle_tcp_lanes_are_closed() {
    let dir = Temp::new();
    let (mut config, state) = initial();
    config.tcp_idle = Duration::from_millis(300);
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::managed(config, node.clone());
    let task = tokio::spawn(async move { server.serve(listener).await });
    let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = Address::Ip(silent.local_addr().unwrap());
    let silent_task = tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let (stream, _) = silent.accept().await.unwrap();
            held.push(stream);
        }
    });
    let alice = Client::new(client_config(address, &ALICE));
    let mut lane = alice.open_tcp(&destination).await.unwrap();
    let mut buffer = [0u8; 8];
    let read = timeout(Duration::from_secs(5), lane.read(&mut buffer))
        .await
        .expect("the idle lane closes");
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    task.abort();
    silent_task.abort();
}

#[tokio::test]
async fn user_sessions_and_lanes_are_capped() {
    let dir = Temp::new();
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), initial().1).unwrap();
    let hello = Template::from_json(TEMPLATE)
        .unwrap()
        .build(&ALICE)
        .unwrap()
        .record;
    let (_, principal) = node.authenticate(&hello[5..], unix_time() as i64).unwrap();
    let fiend_core::node::Principal::User(entry) = principal else {
        panic!()
    };
    let first = node.begin_session_within(entry.clone(), 2).unwrap();
    let second = node.begin_session_within(entry.clone(), 2).unwrap();
    assert!(node.begin_session_within(entry.clone(), 2).is_none());
    assert_eq!(node.stats().users[0].open_sessions, 2);
    drop(first);
    assert!(node.begin_session_within(entry.clone(), 2).is_some());
    drop(second);
    let lane = entry.begin_lane(1).unwrap();
    assert!(entry.begin_lane(1).is_none());
    drop(lane);
    assert!(entry.begin_lane(1).is_some());
}

#[tokio::test]
async fn traffic_limit_revokes_sessions_and_reset_epoch_restores_access() {
    let dir = Temp::new();
    let mut state = initial().1;
    state.users[0].traffic_limit = Some(5);
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let template = Template::from_json(TEMPLATE).unwrap();
    let hello = template.build(&ALICE).unwrap().record;
    let (_, principal) = node.authenticate(&hello[5..], unix_time() as i64).unwrap();
    let fiend_core::node::Principal::User(entry) = principal else {
        panic!()
    };
    let mut guard = node.begin_session(entry.clone()).unwrap();
    entry.note_bytes(5, true);
    timeout(Duration::from_secs(1), guard.revoked())
        .await
        .unwrap();
    assert!(node.authenticate(&hello[5..], unix_time() as i64).is_none());
    let mut next = node.desired_state();
    next.version += 1;
    next.users[0].usage_epoch += 1;
    node.apply(next).await.unwrap();
    assert!(node.authenticate(&hello[5..], unix_time() as i64).is_some());
    drop(guard);
    assert_eq!(node.stats().users[0].open_sessions, 0);
}

#[tokio::test]
async fn saved_usage_survives_restart_and_removed_users_keep_pending_counters() {
    let dir = Temp::new();
    let mut state = initial().1;
    state.users[0].traffic_limit = Some(5);
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let hello = Template::from_json(TEMPLATE)
        .unwrap()
        .build(&ALICE)
        .unwrap()
        .record;
    let (_, principal) = node.authenticate(&hello[5..], unix_time() as i64).unwrap();
    let fiend_core::node::Principal::User(entry) = principal else {
        panic!()
    };
    entry.note_bytes(5, true);
    node.persist_usage().await.unwrap();
    let restored = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), initial().1).unwrap();
    assert!(
        restored
            .authenticate(&hello[5..], unix_time() as i64)
            .is_none()
    );
    let mut next = node.desired_state();
    next.version += 1;
    next.users.remove(0);
    node.apply(next).await.unwrap();
    let stats = node.stats();
    assert_eq!(
        stats.users.iter().find(|row| row.id == "alice").unwrap().up,
        5
    );
    assert!(node.stats().users.iter().all(|row| row.id != "alice"));
}

#[tokio::test]
async fn expiry_closes_an_open_session_without_panel_access() {
    let dir = Temp::new();
    let mut state = initial().1;
    state.users[0].expire_at = Some(unix_time() + 3);
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let hello = Template::from_json(TEMPLATE)
        .unwrap()
        .build(&ALICE)
        .unwrap()
        .record;
    let (_, principal) = node.authenticate(&hello[5..], unix_time() as i64).unwrap();
    let fiend_core::node::Principal::User(entry) = principal else {
        panic!()
    };
    let mut guard = node.begin_session(entry).unwrap();
    timeout(Duration::from_secs(5), guard.revoked())
        .await
        .unwrap();
}

#[test]
fn prepared_hmac_keys_match_auth_tokens() {
    use fiend_core::crypto::{AuthKey, auth_token};
    let key = AuthKey::new(&ALICE);
    let random = [0x59; 32];
    for bucket in [-1, 0, 100, 400_000] {
        let token = auth_token(&ALICE, &random, bucket);
        assert!(key.matches(&random, bucket, &token));
        assert!(!key.matches(&random, bucket + 1, &token));
        assert!(!key.matches(&random, bucket, &token[..31]));
    }
}

#[tokio::test]
async fn usage_sync_keeps_active_sessions_but_quota_sync_revokes_them() {
    use fiend_core::node::Principal;
    let dir = Temp::new();
    let mut state = initial().1;
    state.users[0].traffic_limit = Some(10);
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    let hello = Template::from_json(TEMPLATE)
        .unwrap()
        .build(&ALICE)
        .unwrap()
        .record;
    let (_, Principal::User(entry)) = node.authenticate(&hello[5..], unix_time() as i64).unwrap()
    else {
        panic!()
    };
    let mut guard = node.begin_session(entry.clone()).unwrap();
    let mut next = node.desired_state();
    next.version += 1;
    next.users[0].used_traffic = 5;
    node.apply(next).await.unwrap();
    let (_, Principal::User(current)) = node.authenticate(&hello[5..], unix_time() as i64).unwrap()
    else {
        panic!()
    };
    assert!(std::sync::Arc::ptr_eq(&entry, &current));
    let mut next = node.desired_state();
    next.version += 1;
    next.users[0].used_traffic = 10;
    node.apply(next).await.unwrap();
    timeout(Duration::from_secs(1), guard.revoked())
        .await
        .unwrap();
    assert!(node.authenticate(&hello[5..], unix_time() as i64).is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn unix_control_uses_private_socket_and_rejects_live_or_non_socket_paths() {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixStream;
    let dir = Temp::new();
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), initial().1).unwrap();
    let path = dir.0.join("control.sock");
    let service_path = path.clone();
    let service_node = node.clone();
    let task = tokio::spawn(async move {
        fiend_core::node::control::serve_unix(&service_path, service_node).await
    });
    let stream = timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(stream) = UnixStream::connect(&path).await {
                break stream;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut client = ControlClient::handshake(stream).await.unwrap();
    assert_eq!(client.call(Call::Health).await.unwrap()["state_version"], 1);
    assert!(
        fiend_core::node::control::serve_unix(&path, node.clone())
            .await
            .is_err()
    );
    let regular = dir.0.join("not-a-socket");
    fs::write(&regular, b"keep").unwrap();
    assert!(
        fiend_core::node::control::serve_unix(&regular, node)
            .await
            .is_err()
    );
    assert_eq!(fs::read(regular).unwrap(), b"keep");
    task.abort();
}

async fn proxy_relay(
    server: std::net::SocketAddr,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = relay.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut inbound, _) = relay.accept().await.unwrap();
            tokio::spawn(async move {
                let mut outbound = TcpStream::connect(server).await.unwrap();
                outbound
                    .write_all(b"PROXY TCP4 203.0.113.9 198.51.100.1 52000 443\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    (address, task)
}

#[tokio::test]
async fn a_node_behind_a_proxy_reads_and_forwards_client_addresses() {
    let dir = Temp::new();
    let (mut config, mut state) = initial();
    config.proxy_protocol = true;
    state.origin_host = "site.example".into();
    state.origin_port = 443;
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    node.set_fallbacks(vec![fiend_core::transport::Fallback {
        host: "site.example".into(),
        origin: fiend_core::transport::OriginTarget {
            host: "127.0.0.1".into(),
            port: origin.local_addr().unwrap().port(),
            proxy_protocol: true,
        },
    }]);
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::managed(config, node.clone());
    let task = tokio::spawn(async move { server.serve(listener).await });
    let template = Template::from_json(TEMPLATE).unwrap();

    let hello = template.build(&[0x95; 32]).unwrap().record;
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"PROXY TCP4 203.0.113.7 198.51.100.1 51000 443\r\n")
        .await
        .unwrap();
    stream.write_all(&hello).await.unwrap();
    let (mut forwarded, _) = timeout(Duration::from_secs(2), origin.accept())
        .await
        .unwrap()
        .unwrap();
    let client = fiend_core::proxy::read_header(&mut forwarded)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(client.source, "203.0.113.7:51000".parse().unwrap());
    assert_eq!(client.destination, "198.51.100.1:443".parse().unwrap());
    let mut received = vec![0u8; hello.len()];
    forwarded.read_exact(&mut received).await.unwrap();
    assert_eq!(received, hello);
    let stats = node.stats();
    assert_eq!(stats.handshakes_origin, 1);
    assert_eq!(stats.probes[0].peer, "203.0.113.7");

    let mut bare = TcpStream::connect(address).await.unwrap();
    bare.write_all(&hello).await.unwrap();
    let mut byte = [0u8; 1];
    assert!(matches!(
        timeout(Duration::from_secs(2), bare.read(&mut byte))
            .await
            .unwrap(),
        Ok(0) | Err(_)
    ));

    let mut looped = TcpStream::connect(address).await.unwrap();
    looped
        .write_all(b"PROXY TCP4 198.51.100.1 198.51.100.1 40000 443\r\n")
        .await
        .unwrap();
    looped.write_all(&hello).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), looped.read(&mut byte))
            .await
            .unwrap(),
        Ok(0) | Err(_)
    ));
    assert!(
        timeout(Duration::from_millis(500), origin.accept())
            .await
            .is_err(),
        "no connection without a header or from a loop may reach the origin"
    );
    assert_eq!(node.stats().handshakes_origin, 0);

    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = Address::Ip(echo.local_addr().unwrap());
    let echo_task = tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut buffer = [0u8; 5];
        stream.read_exact(&mut buffer).await.unwrap();
        stream.write_all(&buffer).await.unwrap();
    });
    let (relay, relay_task) = proxy_relay(address).await;
    let alice = Client::new(client_config(relay, &ALICE));
    let mut lane = alice.open_tcp(&destination).await.unwrap();
    lane.write_all(b"hello").await.unwrap();
    let mut reply = [0u8; 5];
    lane.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"hello");
    alice.close();
    task.abort();
    echo_task.abort();
    relay_task.abort();
}

#[tokio::test]
async fn a_probe_for_a_site_with_a_fallback_goes_to_that_site() {
    let dir = Temp::new();
    let (mut config, mut state) = initial();
    let disguise = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.origin_port = disguise.local_addr().unwrap().port();
    state.origin_port = config.origin_port;
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), state).unwrap();
    node.set_fallbacks(vec![fiend_core::transport::Fallback {
        host: "site.example".into(),
        origin: fiend_core::transport::OriginTarget {
            host: "127.0.0.1".into(),
            port: site.local_addr().unwrap().port(),
            proxy_protocol: true,
        },
    }]);
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::managed(config, node.clone());
    let task = tokio::spawn(async move { server.serve(listener).await });

    let hello = fiend_core::transport::build_probe_hello("Site.Example");
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(&hello).await.unwrap();
    let (mut forwarded, _) = timeout(Duration::from_secs(2), site.accept())
        .await
        .unwrap()
        .unwrap();
    let client = fiend_core::proxy::read_header(&mut forwarded)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(client.source, stream.local_addr().unwrap());
    let mut received = vec![0u8; hello.len()];
    forwarded.read_exact(&mut received).await.unwrap();
    assert_eq!(received, hello);

    let other = fiend_core::transport::build_probe_hello("other.example");
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(&other).await.unwrap();
    let (mut forwarded, _) = timeout(Duration::from_secs(2), disguise.accept())
        .await
        .unwrap()
        .unwrap();
    let mut received = vec![0u8; other.len()];
    forwarded.read_exact(&mut received).await.unwrap();
    assert_eq!(received, other);
    task.abort();
}
