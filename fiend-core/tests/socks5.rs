use std::net::SocketAddr;
use std::time::{Duration, Instant};

use fiend_core::addr::Address;
use fiend_core::client::{Client, ClientConfig};
use fiend_core::fingerprint::Template;
use fiend_core::net;
use fiend_core::server::{Server, ServerConfig};
use fiend_core::socks5;
use fiend_core::target::TargetPolicy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::timeout;

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");
const SECRET: &[u8] = b"socks5 test secret";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

async fn start_stack() -> (SocketAddr, Client) {
    start_stack_with(|_| {}, |_| {}).await
}

async fn start_stack_with(
    configure_server: impl FnOnce(&mut ServerConfig),
    configure_client: impl FnOnce(&mut ClientConfig),
) -> (SocketAddr, Client) {
    let template = Template::from_json(TEMPLATE_JSON).expect("load sample template");

    let listener =
        net::bind_listener("127.0.0.1:0".parse().expect("carrier address")).expect("carrier bind");
    let carrier = listener.local_addr().expect("carrier address");
    let mut server_config = ServerConfig::new(SECRET, "127.0.0.1", 443);
    server_config.mux.target_timeout = Duration::from_secs(5);
    server_config.target = TargetPolicy::allow_all();
    configure_server(&mut server_config);
    let server = Server::new(server_config);
    tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });

    let mut client_config = ClientConfig::new(carrier.to_string(), SECRET, template);
    client_config.mux.target_timeout = Duration::from_secs(5);
    configure_client(&mut client_config);
    let client = Client::new(client_config);

    let listener =
        net::bind_listener("127.0.0.1:0".parse().expect("SOCKS address")).expect("SOCKS bind");
    let socks = listener.local_addr().expect("SOCKS address");
    let service = client.clone();
    tokio::spawn(async move {
        let _ = service.serve(listener).await;
    });

    (socks, client)
}

async fn tcp_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("echo bind");
    let address = listener.local_addr().expect("echo address");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 16 << 10];
                while let Ok(length) = stream.read(&mut buffer).await {
                    if length == 0 || stream.write_all(&buffer[..length]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    address
}

async fn udp_echo() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("echo bind");
    let address = socket.local_addr().expect("echo address");
    tokio::spawn(async move {
        let mut buffer = vec![0u8; u16::MAX as usize];
        loop {
            let Ok((length, from)) = socket.recv_from(&mut buffer).await else {
                break;
            };
            let _ = socket.send_to(&buffer[..length], from).await;
        }
    });
    address
}

async fn open_control(socks: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(socks).await.expect("SOCKS connect");
    stream.write_all(&[5, 1, 0]).await.expect("greeting");
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).await.expect("greeting reply");
    assert_eq!(reply, [5, 0]);
    stream
}

async fn connect_flow(socks: SocketAddr, target: SocketAddr) -> TcpStream {
    let mut stream = open_control(socks).await;
    let (reply, _bound) = request(&mut stream, 1, &Address::Ip(target)).await;
    assert_eq!(reply, 0);
    stream
}

async fn request(stream: &mut TcpStream, command: u8, address: &Address) -> (u8, Address) {
    let mut message = vec![5, command, 0];
    address.encode_into(&mut message).expect("encode address");
    stream.write_all(&message).await.expect("request");

    let mut header = [0u8; 3];
    stream.read_exact(&mut header).await.expect("reply header");
    assert_eq!(header[0], 5);
    let mut atyp = [0u8; 1];
    stream.read_exact(&mut atyp).await.expect("reply type");
    let bound = socks5::read_address(stream, atyp[0])
        .await
        .expect("reply address");
    (header[1], bound)
}

async fn udp_associate(socks: SocketAddr) -> (TcpStream, UdpSocket, SocketAddr) {
    let mut control = open_control(socks).await;
    let (reply, relay) = request(&mut control, 3, &socks5::zero_address()).await;
    assert_eq!(reply, 0);
    let Address::Ip(relay) = relay else {
        panic!("relay is not an IP address");
    };
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("UDP bind");
    (control, socket, relay)
}

async fn exchange(
    socket: &UdpSocket,
    relay: SocketAddr,
    target: Address,
    payload: &[u8],
) -> (Address, Vec<u8>) {
    let datagram = socks5::encode_udp_request(&target, payload).expect("encode request");
    socket.send_to(&datagram, relay).await.expect("send");

    let mut buffer = vec![0u8; u16::MAX as usize];
    let (length, from) = timeout(CALL_TIMEOUT, socket.recv_from(&mut buffer))
        .await
        .expect("UDP response timeout")
        .expect("UDP response");
    assert_eq!(from, relay);
    let (source, body) = socks5::parse_udp_request(&buffer[..length]).expect("parse response");
    (source, body.to_vec())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_relays_tcp() {
    let echo = tcp_echo().await;
    let (socks, _client) = start_stack().await;
    let mut stream = open_control(socks).await;

    let (reply, _bound) = request(&mut stream, 1, &Address::Ip(echo)).await;
    assert_eq!(reply, 0);

    let payload = vec![0xa5u8; 64 << 10];
    timeout(CALL_TIMEOUT, stream.write_all(&payload))
        .await
        .expect("write timeout")
        .expect("write");
    let mut echoed = vec![0u8; payload.len()];
    timeout(CALL_TIMEOUT, stream.read_exact(&mut echoed))
        .await
        .expect("read timeout")
        .expect("read");
    assert_eq!(echoed, payload);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_associate_relays_datagrams() {
    let echo = udp_echo().await;
    let (socks, _client) = start_stack().await;
    let (_control, socket, relay) = udp_associate(socks).await;

    let (source, body) = exchange(&socket, relay, Address::Ip(echo), b"datagram").await;
    assert_eq!(source, Address::Ip(echo));
    assert_eq!(body, b"datagram");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_associate_relays_large_datagrams() {
    let echo = udp_echo().await;
    let (socks, _client) = start_stack().await;
    let (_control, socket, relay) = udp_associate(socks).await;

    let payload = vec![0x5au8; 8 << 10];
    let (source, body) = exchange(&socket, relay, Address::Ip(echo), &payload).await;
    assert_eq!(source, Address::Ip(echo));
    assert_eq!(body, payload);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_associate_demuxes_destinations() {
    let first = udp_echo().await;
    let second = udp_echo().await;
    let (socks, _client) = start_stack().await;
    let (_control, socket, relay) = udp_associate(socks).await;

    let (source, body) = exchange(&socket, relay, Address::Ip(first), b"first").await;
    assert_eq!(source, Address::Ip(first));
    assert_eq!(body, b"first");

    let (source, body) = exchange(&socket, relay, Address::Ip(second), b"second").await;
    assert_eq!(source, Address::Ip(second));
    assert_eq!(body, b"second");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_keepalive_keeps_lane_open() {
    let echo = udp_echo().await;
    let (socks, _client) = start_stack_with(
        |config| config.udp_idle = Duration::from_millis(400),
        |config| config.udp_keepalive = Duration::from_millis(100),
    )
    .await;
    let (_control, socket, relay) = udp_associate(socks).await;

    let (_, body) = exchange(&socket, relay, Address::Ip(echo), b"before").await;
    assert_eq!(body, b"before");

    tokio::time::sleep(Duration::from_millis(1200)).await;

    let (source, body) = exchange(&socket, relay, Address::Ip(echo), b"after").await;
    assert_eq!(source, Address::Ip(echo));
    assert_eq!(body, b"after");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_unsupported_authentication() {
    let (socks, _client) = start_stack().await;
    let mut stream = TcpStream::connect(socks).await.expect("SOCKS connect");
    stream.write_all(&[5, 1, 2]).await.expect("greeting");

    let mut reply = [0u8; 2];
    timeout(CALL_TIMEOUT, stream.read_exact(&mut reply))
        .await
        .expect("read timeout")
        .expect("read");
    assert_eq!(reply, [5, 0xff]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_unsupported_commands() {
    let (socks, _client) = start_stack().await;
    let mut stream = open_control(socks).await;

    let (reply, _bound) = request(&mut stream, 2, &socks5::zero_address()).await;
    assert_eq!(reply, 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocks_loopback_targets_by_default() {
    let echo = tcp_echo().await;
    let (socks, _client) =
        start_stack_with(|config| config.target = TargetPolicy::default(), |_| {}).await;
    let mut stream = open_control(socks).await;

    let (reply, _bound) = request(&mut stream, 1, &Address::Ip(echo)).await;
    assert_eq!(reply, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocks_loopback_udp_targets_by_default() {
    let echo = udp_echo().await;
    let (socks, _client) =
        start_stack_with(|config| config.target = TargetPolicy::default(), |_| {}).await;
    let (_control, socket, relay) = udp_associate(socks).await;

    let datagram = socks5::encode_udp_request(&Address::Ip(echo), b"blocked").expect("encode");
    socket.send_to(&datagram, relay).await.expect("send");
    let mut buffer = [0u8; 256];
    let response = timeout(Duration::from_millis(800), socket.recv_from(&mut buffer)).await;
    assert!(response.is_err(), "blocked UDP target answered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handles_two_hundred_concurrent_tcp_flows() {
    let echo = tcp_echo().await;
    let (socks, client) = start_stack().await;

    let mut tasks = Vec::new();
    for index in 0..200u32 {
        tasks.push(tokio::spawn(async move {
            let mut stream = open_control(socks).await;
            let (reply, _bound) = request(&mut stream, 1, &Address::Ip(echo)).await;
            assert_eq!(reply, 0);

            let payload = index.to_be_bytes();
            stream.write_all(&payload).await.expect("write");
            let mut echoed = [0u8; 4];
            stream.read_exact(&mut echoed).await.expect("read");
            assert_eq!(echoed, payload);
        }));
    }

    for task in tasks {
        timeout(Duration::from_secs(60), task)
            .await
            .expect("flow timeout")
            .expect("flow task");
    }

    let stats = client.stats();
    assert!(stats.links >= 1);
    eprintln!("RSS after 200 flows: {:?} KiB", rss_kib());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latency_stays_low_under_bulk() {
    let echo = tcp_echo().await;
    let (socks, client) = start_stack().await;
    let bulk = connect_flow(socks, echo).await;
    let (mut bulk_read, mut bulk_write) = bulk.into_split();

    let total: u64 = 32 * 1024 * 1024;
    let writer = tokio::spawn(async move {
        let chunk = vec![0x33u8; 16 << 10];
        let mut sent = 0u64;
        while sent < total {
            let size = ((total - sent) as usize).min(chunk.len());
            bulk_write
                .write_all(&chunk[..size])
                .await
                .expect("bulk write");
            sent += size as u64;
        }
        let _ = bulk_write.shutdown().await;
        sent
    });
    let reader = tokio::spawn(async move {
        let mut buffer = vec![0u8; 64 << 10];
        let mut received = 0u64;
        while received < total {
            let length = bulk_read.read(&mut buffer).await.expect("bulk read");
            if length == 0 {
                break;
            }
            received += length as u64;
        }
        received
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while client.stats().links < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(client.stats().links >= 2, "bulk link did not split");

    let mut samples = Vec::with_capacity(20);
    for _ in 0..20 {
        let started = Instant::now();
        let mut probe = connect_flow(socks, echo).await;
        probe.write_all(b"p").await.expect("probe write");
        let mut response = [0u8; 1];
        probe.read_exact(&mut response).await.expect("probe read");
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    eprintln!("SOCKS5 interactive p95 under bulk: {p95:?}");
    eprintln!("RSS during bulk: {:?} KiB", rss_kib());
    assert!(p95 < Duration::from_secs(2), "p95 was {p95:?}");

    assert_eq!(writer.await.expect("bulk writer"), total);
    assert_eq!(reader.await.expect("bulk reader"), total);
}

fn rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}
