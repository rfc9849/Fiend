use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use fiend_core::addr::Address;
use fiend_core::client::{Client, ClientConfig};
use fiend_core::fingerprint::Template;
use fiend_core::net;
use fiend_core::server::{Server, ServerConfig};
use fiend_core::target::TargetPolicy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::time::timeout;

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");
const SECRET: &[u8] = &[0x24; 32];

async fn server() -> SocketAddr {
    let listener = net::bind_listener("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = ServerConfig::new(SECRET, "127.0.0.1", 443);
    config.target = TargetPolicy::allow_all();
    let server = Server::new(config);
    tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    address
}

struct Gate {
    address: SocketAddr,
    stop: Arc<Notify>,
}

async fn gate(backend: SocketAddr, delay: Duration) -> Gate {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(Notify::new());
    let signal = stop.clone();
    tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((mut inbound, _)) = accepted else { break };
                    connections.spawn(async move {
                        tokio::time::sleep(delay).await;
                        if let Ok(mut outbound) = TcpStream::connect(backend).await {
                            let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                        }
                    });
                }
                _ = signal.notified() => break,
            }
        }
    });
    Gate { address, stop }
}

async fn echo() -> Address {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                while let Ok(length) = stream.read(&mut buffer).await {
                    if length == 0 || stream.write_all(&buffer[..length]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    Address::Ip(address)
}

fn client(servers: Vec<String>) -> Client {
    let template = Template::from_json(TEMPLATE_JSON).unwrap();
    let mut config = ClientConfig::new(String::new(), SECRET, template);
    config.servers = servers;
    config.handshake_timeout = Duration::from_secs(3);
    config.mux.open_timeout = Duration::from_secs(10);
    Client::new(fiend_core::config::build_client(config).unwrap())
}

async fn round_trip(client: &Client, target: &Address) {
    let mut lane = timeout(Duration::from_secs(10), client.open_tcp(target))
        .await
        .expect("lane open timed out")
        .expect("lane open failed");
    lane.write_all(b"ping").await.unwrap();
    let mut reply = [0u8; 4];
    timeout(Duration::from_secs(5), lane.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"ping");
}

fn preferred(client: &Client) -> String {
    client
        .servers()
        .into_iter()
        .find(|status| status.preferred)
        .map(|status| status.server)
        .unwrap_or_default()
}

#[tokio::test]
async fn client_picks_the_faster_server_and_fails_over_when_it_stops() {
    let target = echo().await;
    let slow = gate(server().await, Duration::from_millis(400)).await;
    let fast = gate(server().await, Duration::ZERO).await;
    let client = client(vec![slow.address.to_string(), fast.address.to_string()]);
    round_trip(&client, &target).await;
    assert_eq!(preferred(&client), fast.address.to_string());

    fast.stop.notify_one();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut recovered = false;
    for _ in 0..20 {
        if timeout(Duration::from_secs(10), round_trip(&client, &target))
            .await
            .is_ok()
        {
            recovered = true;
            break;
        }
    }
    assert!(recovered);
    round_trip(&client, &target).await;
    assert_eq!(preferred(&client), slow.address.to_string());
    let status = client.servers();
    assert_eq!(status.len(), 2);
    drop(slow);
}

#[tokio::test]
async fn a_dead_first_server_does_not_block_the_connection() {
    let target = echo().await;
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_address = dead.local_addr().unwrap();
    drop(dead);
    let live = server().await;
    let client = client(vec![dead_address.to_string(), live.to_string()]);
    round_trip(&client, &target).await;
    assert_eq!(preferred(&client), live.to_string());
}
