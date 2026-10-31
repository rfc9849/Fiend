use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fiend_core::addr::Address;
use fiend_core::client::{Client, ClientConfig};
use fiend_core::fingerprint::Template;
use fiend_core::node::control::{Call, Client as ControlClient};
use fiend_core::node::outbound::{Outbound, OutboundKind, Rule, WarpSettings};
use fiend_core::node::{DesiredState, Node, User};
use fiend_core::server::{Server, ServerConfig};
use fiend_core::socks5;
use fiend_core::target::TargetPolicy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::timeout;

const TEMPLATE: &str = include_str!("../testdata/template.json");
const CONTROL: [u8; 32] = [0x73; 32];
const USER: [u8; 32] = [0x71; 32];
const CHAIN: [u8; 32] = [0x72; 32];

type Seen = Arc<Mutex<Vec<String>>>;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fiend-route-{}",
            hex::encode(rand::random::<[u8; 12]>())
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn tcp_echo(tag: &'static [u8]) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                while let Ok(count) = stream.read(&mut buffer).await {
                    if count == 0 {
                        break;
                    }
                    let mut reply = tag.to_vec();
                    reply.extend_from_slice(&buffer[..count]);
                    if stream.write_all(&reply).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    address
}

async fn udp_echo() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buffer = [0u8; 2048];
        while let Ok((count, peer)) = socket.recv_from(&mut buffer).await {
            let _ = socket.send_to(&buffer[..count], peer).await;
        }
    });
    address
}

async fn send_reply(stream: &mut TcpStream, bound: &Address) {
    let mut reply = vec![socks5::VERSION, socks5::REP_SUCCESS, 0];
    bound.encode_into(&mut reply).unwrap();
    stream.write_all(&reply).await.unwrap();
}

async fn socks_proxy(tcp: SocketAddr, udp: SocketAddr, seen: Seen) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = [0u8; 2];
                stream.read_exact(&mut head).await.unwrap();
                let mut methods = vec![0u8; head[1] as usize];
                stream.read_exact(&mut methods).await.unwrap();
                assert!(methods.contains(&2));
                stream.write_all(&[5, 2]).await.unwrap();
                let mut version = [0u8; 2];
                stream.read_exact(&mut version).await.unwrap();
                let mut user = vec![0u8; version[1] as usize];
                stream.read_exact(&mut user).await.unwrap();
                let length = stream.read_u8().await.unwrap();
                let mut password = vec![0u8; length as usize];
                stream.read_exact(&mut password).await.unwrap();
                let good = user == b"proxy-user" && password == b"proxy-pass";
                stream
                    .write_all(&[1, if good { 0 } else { 1 }])
                    .await
                    .unwrap();
                if !good {
                    return;
                }
                let mut request = [0u8; 4];
                stream.read_exact(&mut request).await.unwrap();
                let target = socks5::read_address(&mut stream, request[3]).await.unwrap();
                seen.lock()
                    .unwrap()
                    .push(format!("{}:{target}", request[1]));
                if request[1] == socks5::CMD_UDP_ASSOCIATE {
                    let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                    let bound = Address::Ip(relay.local_addr().unwrap());
                    send_reply(&mut stream, &bound).await;
                    let mut buffer = [0u8; 2048];
                    let (count, client) = relay.recv_from(&mut buffer).await.unwrap();
                    let (destination, payload) =
                        socks5::parse_udp_request(&buffer[..count]).unwrap();
                    seen.lock().unwrap().push(format!("udp:{destination}"));
                    let upstream = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                    upstream.send_to(payload, udp).await.unwrap();
                    let (count, _) = upstream.recv_from(&mut buffer).await.unwrap();
                    let reply = socks5::encode_udp_request(&destination, &buffer[..count]).unwrap();
                    relay.send_to(&reply, client).await.unwrap();
                    let _ = stream.read_u8().await;
                    return;
                }
                let mut upstream = TcpStream::connect(tcp).await.unwrap();
                send_reply(
                    &mut stream,
                    &Address::Ip(SocketAddr::from(([0, 0, 0, 0], 0))),
                )
                .await;
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
            });
        }
    });
    address
}

async fn http_proxy(tcp: SocketAddr, seen: Seen) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(stream.read_u8().await.unwrap());
                }
                let text = String::from_utf8(head).unwrap();
                let line = text.lines().next().unwrap().to_owned();
                seen.lock().unwrap().push(line.clone());
                if !text.contains("Proxy-Authorization: Basic aHR0cC11c2VyOmh0dHAtcGFzcw==") {
                    stream
                        .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                        .await
                        .unwrap();
                    return;
                }
                let mut upstream = TcpStream::connect(tcp).await.unwrap();
                stream
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
            });
        }
    });
    address
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

async fn node(
    dir: &Temp,
    name: &str,
    policy: TargetPolicy,
    users: Vec<User>,
    outbounds: Vec<Outbound>,
    rules: Vec<Rule>,
) -> (SocketAddr, Node) {
    let mut config = ServerConfig::new("127.0.0.1", 1);
    config.origin.record_sizes = vec![35, 3400, 286, 74];
    config.target = policy;
    let mut state = DesiredState::initial(&config);
    state.users = users;
    state.outbounds = outbounds;
    state.rules = rules;
    let node = Node::load(dir.0.join(format!("{name}.json")), CONTROL.to_vec(), state).unwrap();
    node.set_resolver(Some(Arc::new(|host: String, port: u16| {
        Box::pin(async move {
            if host == "localhost" {
                Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no test address",
                ))
            }
        })
    })));
    let listener = fiend_core::net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::managed(config, node.clone());
    tokio::spawn(async move { server.serve(listener).await });
    (address, node)
}

fn client(address: SocketAddr, secret: &[u8]) -> Client {
    let mut config = ClientConfig::new(
        address.to_string(),
        secret,
        Template::from_json(TEMPLATE).unwrap(),
    );
    config.mux.attempts = 1;
    config.mux.open_timeout = Duration::from_secs(5);
    Client::new(config)
}

async fn exchange(client: &Client, target: Address) -> Result<Vec<u8>, String> {
    let mut lane = timeout(Duration::from_secs(10), client.open_tcp(&target))
        .await
        .map_err(|_| "open timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    lane.write_all(b"ping")
        .await
        .map_err(|error| error.to_string())?;
    let mut reply = vec![0u8; 64];
    let count = timeout(Duration::from_secs(5), lane.read(&mut reply))
        .await
        .map_err(|_| "read timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    reply.truncate(count);
    if reply.is_empty() {
        return Err("closed".into());
    }
    Ok(reply)
}

fn domain(host: &str) -> Address {
    Address::Domain {
        host: host.into(),
        port: 443,
    }
}

fn rule(pattern: &str, outbound: &str) -> Rule {
    Rule {
        pattern: pattern.into(),
        outbound: outbound.into(),
    }
}

#[tokio::test]
async fn rules_send_traffic_to_socks_http_block_and_a_chained_node() {
    let dir = Temp::new();
    let seen: Seen = Arc::default();
    let socks_echo = tcp_echo(b"S").await;
    let http_echo = tcp_echo(b"H").await;
    let chain_echo = tcp_echo(b"C").await;
    let direct_echo = tcp_echo(b"D").await;
    let udp = udp_echo().await;
    let socks = socks_proxy(socks_echo, udp, seen.clone()).await;
    let http = http_proxy(http_echo, seen.clone()).await;
    let chain_http = http_proxy(chain_echo, seen.clone()).await;

    let (chain_address, _) = node(
        &dir,
        "chain",
        TargetPolicy::allow_all(),
        vec![user("service", &CHAIN)],
        vec![Outbound {
            id: "upstream".into(),
            kind: OutboundKind::Http {
                server: chain_http.to_string(),
                username: Some("http-user".into()),
                password: Some("http-pass".into()),
            },
            udp_fallback: None,
        }],
        vec![rule("full:chain.invalid", "upstream")],
    )
    .await;
    let link = fiend_core::share::encode(&fiend_core::share::Share {
        name: "chain".into(),
        servers: vec![chain_address.to_string()],
        secret: CHAIN,
        template: Template::from_json(TEMPLATE).unwrap(),
    })
    .unwrap();

    let outbounds = vec![
        Outbound {
            id: "socks".into(),
            kind: OutboundKind::Socks5 {
                server: socks.to_string(),
                username: Some("proxy-user".into()),
                password: Some("proxy-pass".into()),
                udp: true,
            },
            udp_fallback: None,
        },
        Outbound {
            id: "http".into(),
            kind: OutboundKind::Http {
                server: http.to_string(),
                username: Some("http-user".into()),
                password: Some("http-pass".into()),
            },
            udp_fallback: None,
        },
        Outbound {
            id: "chain".into(),
            kind: OutboundKind::Fiend { link },
            udp_fallback: None,
        },
        Outbound {
            id: "deny".into(),
            kind: OutboundKind::Block,
            udp_fallback: None,
        },
    ];
    let rules = vec![
        rule("full:socks.invalid\nfull:udp.invalid", "socks"),
        rule("domain:http.invalid", "http"),
        rule("full:chain.invalid", "chain"),
        rule("keyword:blocked", "deny"),
        rule("domain:invalid", "direct"),
    ];
    let (address, managed) = node(
        &dir,
        "main",
        TargetPolicy::allow_all(),
        vec![user("alice", &USER)],
        outbounds,
        rules,
    )
    .await;
    let alice = client(address, &USER);

    assert_eq!(
        exchange(&alice, domain("socks.invalid")).await.unwrap(),
        b"Sping"
    );
    assert_eq!(
        exchange(&alice, domain("www.http.invalid")).await.unwrap(),
        b"Hping"
    );
    assert_eq!(
        exchange(&alice, domain("chain.invalid")).await.unwrap(),
        b"Cping"
    );
    assert_eq!(
        exchange(&alice, Address::Ip(direct_echo)).await.unwrap(),
        b"Dping"
    );
    assert!(
        exchange(&alice, domain("is-blocked.invalid"))
            .await
            .is_err()
    );
    assert!(exchange(&alice, domain("other.invalid")).await.is_err());

    let mut flow = alice.open_udp(&domain("udp.invalid")).await.unwrap();
    flow.send(b"datagram").await.unwrap();
    let response = timeout(Duration::from_secs(5), flow.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.payload, b"datagram");
    assert!(alice.open_udp(&domain("www.http.invalid")).await.is_err());

    let seen = seen.lock().unwrap().clone();
    assert!(seen.contains(&"1:socks.invalid:443".to_owned()), "{seen:?}");
    assert!(
        seen.contains(&"CONNECT www.http.invalid:443 HTTP/1.1".to_owned()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&"CONNECT chain.invalid:443 HTTP/1.1".to_owned()),
        "{seen:?}"
    );
    assert!(seen.contains(&"udp:udp.invalid:443".to_owned()), "{seen:?}");

    let mut control = ControlClient::connect(&{
        let mut config = ClientConfig::new(
            address.to_string(),
            CONTROL,
            Template::from_json(TEMPLATE).unwrap(),
        );
        config.mux.attempts = 1;
        config
    })
    .await
    .unwrap();
    let stats = control.call(Call::Stats).await.unwrap();
    let outbound = |id: &str| {
        stats["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap()
            .clone()
    };
    assert!(outbound("socks")["up"].as_u64().unwrap() >= 4);
    assert!(outbound("http")["down"].as_u64().unwrap() >= 5);
    assert!(outbound("deny")["errors"].as_u64().unwrap() >= 1);
    assert_eq!(managed.desired_state().rules.len(), 5);
    alice.close();
}

#[tokio::test]
async fn target_policy_applies_to_the_final_destination_of_every_outbound() {
    let dir = Temp::new();
    let seen: Seen = Arc::default();
    let echo = tcp_echo(b"S").await;
    let udp = udp_echo().await;
    let socks = socks_proxy(echo, udp, seen.clone()).await;
    let outbounds = vec![Outbound {
        id: "socks".into(),
        kind: OutboundKind::Socks5 {
            server: socks.to_string(),
            username: Some("proxy-user".into()),
            password: Some("proxy-pass".into()),
            udp: false,
        },
        udp_fallback: None,
    }];
    let rules = vec![rule(
        "ip:127.0.0.0/8\nfull:localhost\nfull:socks.invalid",
        "socks",
    )];
    let (address, _) = node(
        &dir,
        "policy",
        TargetPolicy::default(),
        vec![user("alice", &USER)],
        outbounds,
        rules,
    )
    .await;
    let alice = client(address, &USER);
    assert_eq!(
        exchange(&alice, domain("socks.invalid")).await.unwrap(),
        b"Sping"
    );
    assert!(exchange(&alice, Address::Ip(echo)).await.is_err());
    assert!(
        exchange(
            &alice,
            Address::Domain {
                host: "localhost".into(),
                port: echo.port()
            }
        )
        .await
        .is_err()
    );
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen, vec!["1:socks.invalid:443".to_owned()]);
    alice.close();
}

#[test]
fn invalid_routing_state_is_rejected() {
    let base = |outbounds: Vec<Outbound>, rules: Vec<Rule>, warp: WarpSettings| {
        let mut state = DesiredState::initial(&ServerConfig::new("127.0.0.1", 1));
        state.version = 2;
        state.outbounds = outbounds;
        state.rules = rules;
        state.warp = warp;
        state.validate(&CONTROL)
    };
    let http = |id: &str| Outbound {
        id: id.into(),
        kind: OutboundKind::Http {
            server: "127.0.0.1:8080".into(),
            username: None,
            password: None,
        },
        udp_fallback: None,
    };
    assert!(
        base(
            vec![http("a")],
            vec![rule("domain:example.com", "a")],
            WarpSettings::default()
        )
        .is_ok()
    );
    assert!(base(vec![http("a"), http("a")], vec![], WarpSettings::default()).is_err());
    assert!(base(vec![http("direct")], vec![], WarpSettings::default()).is_err());
    assert!(
        base(
            vec![http("a")],
            vec![rule("domain:example.com", "missing")],
            WarpSettings::default()
        )
        .is_err()
    );
    assert!(
        base(
            vec![http("a")],
            vec![rule("regexp:x", "a")],
            WarpSettings::default()
        )
        .is_err()
    );
    assert!(
        base(
            vec![Outbound {
                id: "w".into(),
                kind: OutboundKind::Warp,
                udp_fallback: None
            }],
            vec![],
            WarpSettings::default()
        )
        .is_err()
    );
    assert!(
        base(
            vec![Outbound {
                id: "w".into(),
                kind: OutboundKind::Warp,
                udp_fallback: None
            }],
            vec![],
            WarpSettings {
                enabled: true,
                rotate_every: Some(21600)
            }
        )
        .is_ok()
    );
    assert!(
        base(
            vec![],
            vec![],
            WarpSettings {
                enabled: true,
                rotate_every: Some(60)
            }
        )
        .is_err()
    );
    let mut fallback = http("a");
    fallback.udp_fallback = Some("a".into());
    assert!(base(vec![fallback.clone()], vec![], WarpSettings::default()).is_err());
    fallback.udp_fallback = Some("direct".into());
    assert!(base(vec![fallback], vec![], WarpSettings::default()).is_ok());
    assert!(
        base(
            vec![Outbound {
                id: "f".into(),
                kind: OutboundKind::Fiend {
                    link: "fiend://bad".into()
                },
                udp_fallback: None
            }],
            vec![],
            WarpSettings::default()
        )
        .is_err()
    );
}
