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
    assert!(proxy_with_control.open_tcp(&destination).await.is_err());
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
async fn accepted_binary_update_restarts_even_when_the_control_client_disconnects() {
    use fiend_core::node::update::{Begin, Chunk};
    use sha2::{Digest, Sha256};
    let dir = Temp::new();
    let node = Node::load(dir.0.join("state.json"), CONTROL.to_vec(), initial().1).unwrap();
    node.configure_updates(dir.0.clone(), "node.toml".into())
        .await;
    let (server, stream) = tokio::io::duplex(64 * 1024);
    let service_node = node.clone();
    let task =
        tokio::spawn(async move { fiend_core::node::control::serve(server, service_node).await });
    let mut client = ControlClient::handshake(stream).await.unwrap();
    let source = b"#!/bin/sh\nexit 0\n";
    let result = client
        .call(Call::UpdateBegin(Begin {
            size: source.len() as u64,
            sha256: hex::encode(Sha256::digest(source)),
        }))
        .await
        .unwrap();
    let id = result["upload_id"].as_str().unwrap().to_owned();
    client
        .call(Call::UpdateChunk(Chunk {
            upload_id: id.clone(),
            offset: 0,
            data: hex::encode(source),
        }))
        .await
        .unwrap();
    client
        .call(Call::UpdateFinish { upload_id: id })
        .await
        .unwrap();
    drop(client);
    timeout(Duration::from_secs(2), node.wait_for_restart())
        .await
        .unwrap();
    assert!(dir.0.join("core.pending.sha256").exists());
    task.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn binary_updates_check_offsets_hash_and_self_check_before_staging() {
    use fiend_core::node::update::{Begin, Chunk, Manager};
    use sha2::{Digest, Sha256};
    let dir = Temp::new();
    let source =
        b"#!/bin/sh\n[ \"$1\" = check ] && [ \"$2\" = node.toml ] && [ \"$3\" = state.json ]\n";
    let digest = hex::encode(Sha256::digest(source));
    let mut manager = Manager::new(dir.0.clone(), "node.toml".into(), "state.json".into());
    let id = manager
        .begin(Begin {
            size: source.len() as u64,
            sha256: digest.clone(),
        })
        .unwrap();
    assert!(
        manager
            .begin(Begin {
                size: source.len() as u64,
                sha256: digest.clone()
            })
            .is_err()
    );
    assert!(manager.finish(&id).await.is_err());
    assert!(
        manager
            .chunk(Chunk {
                upload_id: id.clone(),
                offset: 1,
                data: hex::encode(source)
            })
            .is_err()
    );
    assert_eq!(
        manager
            .chunk(Chunk {
                upload_id: id.clone(),
                offset: 0,
                data: hex::encode(source)
            })
            .unwrap(),
        source.len() as u64
    );
    manager.finish(&id).await.unwrap();
    assert_eq!(fs::read(dir.0.join("core.pending")).unwrap(), source);
    assert_eq!(
        fs::read_to_string(dir.0.join("core.pending.sha256"))
            .unwrap()
            .trim(),
        digest
    );
    assert!(
        manager
            .begin(Begin {
                size: source.len() as u64,
                sha256: digest.clone()
            })
            .is_err()
    );
    fs::remove_file(dir.0.join("core.pending.sha256")).unwrap();
    let id = manager
        .begin(Begin {
            size: source.len() as u64,
            sha256: "00".repeat(32),
        })
        .unwrap();
    manager
        .chunk(Chunk {
            upload_id: id.clone(),
            offset: 0,
            data: hex::encode(source),
        })
        .unwrap();
    assert!(manager.finish(&id).await.is_err());
    let invalid = b"#!/bin/sh\nexit 1\n";
    let id = manager
        .begin(Begin {
            size: invalid.len() as u64,
            sha256: hex::encode(Sha256::digest(invalid)),
        })
        .unwrap();
    manager
        .chunk(Chunk {
            upload_id: id.clone(),
            offset: 0,
            data: hex::encode(invalid),
        })
        .unwrap();
    assert!(manager.finish(&id).await.is_err());
    assert!(!dir.0.join("core.pending.sha256").exists());
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
