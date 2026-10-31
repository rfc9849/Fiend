use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use fiend_core::addr::Address;
use fiend_core::client::{Client, ClientConfig};
use fiend_core::config::{build_client, build_server};
use fiend_core::fingerprint::{Template, parse_client_hello};
use fiend_core::net;
use fiend_core::server::{Server, ServerConfig};
use fiend_core::socks5;
use fiend_core::target::TargetPolicy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::timeout;

const TEMPLATE: &str = include_str!("../testdata/template.json");
const SECRET: [u8; 32] = [42; 32];
const LIMIT: Duration = Duration::from_secs(5);

async fn stack() -> (Client, std::net::SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = ServerConfig::new(SECRET, "127.0.0.1", 443);
    config.target = TargetPolicy::allow_all();
    let server = Server::new(build_server(config).unwrap());
    tokio::spawn(async move { server.serve(listener).await.unwrap() });
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = calls.clone();
    let mut config = ClientConfig::new(
        address.to_string(),
        SECRET,
        Template::from_json(TEMPLATE).unwrap(),
    );
    config.protect_socket = Arc::new(move |socket| {
        assert!(socket.peer_addr().is_err());
        hook_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let client = Client::new(build_client(config).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socks = listener.local_addr().unwrap();
    let service = client.clone();
    tokio::spawn(async move { service.serve(listener).await.unwrap() });
    (client, socks, calls)
}

async fn echo_tcp() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                tokio::io::copy(&mut read, &mut write).await.unwrap();
            });
        }
    });
    address
}

async fn socks_request(
    socks: std::net::SocketAddr,
    command: u8,
    target: &Address,
) -> (TcpStream, Address) {
    let mut stream = TcpStream::connect(socks).await.unwrap();
    stream.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);
    let mut request = vec![5, command, 0];
    target.encode_into(&mut request).unwrap();
    stream.write_all(&request).await.unwrap();
    let mut header = [0; 4];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(&header[..3], &[5, 0, 0]);
    let bound = socks5::read_address(&mut stream, header[3]).await.unwrap();
    (stream, bound)
}

#[tokio::test]
async fn tcp_flow_matches_socks_and_recovers_after_network_change() {
    timeout(LIMIT, async {
        let target = Address::Ip(echo_tcp().await);
        let (client, socks, calls) = stack().await;
        client.ready().await.unwrap();
        assert_eq!(client.stats().lanes, 0);
        let mut direct = client.open_tcp(&target).await.unwrap();
        let (mut proxy, _) = socks_request(socks, 1, &target).await;
        let payload = vec![0xa5; 32 << 10];
        for stream in [&mut direct as &mut dyn AsyncStream, &mut proxy] {
            stream.write_all(&payload).await.unwrap();
            let mut result = vec![0; payload.len()];
            stream.read_exact(&mut result).await.unwrap();
            assert_eq!(result, payload);
        }
        let old_ids = client.stats().link_ids;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        client.network_changed();
        assert_eq!(client.stats().links, 0);
        let mut next = client.open_tcp(&target).await.unwrap();
        next.write_all(b"new network").await.unwrap();
        let mut result = [0; 11];
        next.read_exact(&mut result).await.unwrap();
        assert_eq!(&result, b"new network");
        assert_ne!(client.stats().link_ids, old_ids);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        client.close();
        client.network_changed();
        assert!(client.open_tcp(&target).await.is_err());
        assert!(client.ready().await.is_err());
    })
    .await
    .unwrap();
}

trait AsyncStream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> AsyncStream for T {}

#[tokio::test]
async fn udp_flow_matches_socks_datagrams() {
    timeout(LIMIT, async {
        let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = Address::Ip(echo.local_addr().unwrap());
        tokio::spawn(async move {
            let mut buffer = [0; 4096];
            loop {
                let (size, from) = echo.recv_from(&mut buffer).await.unwrap();
                echo.send_to(&buffer[..size], from).await.unwrap();
            }
        });
        let (client, socks, _) = stack().await;
        let mut direct = client.open_udp(&target).await.unwrap();
        direct.send(b"datagram").await.unwrap();
        let response = direct.recv().await.unwrap().unwrap();
        assert_eq!(response.source, Some(target.clone()));
        let (_control, relay) = socks_request(socks, 3, &socks5::zero_address()).await;
        let Address::Ip(relay) = relay else {
            panic!("IP relay required")
        };
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let request = socks5::encode_udp_request(&target, b"datagram").unwrap();
        socket.send_to(&request, relay).await.unwrap();
        let mut buffer = [0; 4096];
        let (size, _) = socket.recv_from(&mut buffer).await.unwrap();
        let (source, payload) = socks5::parse_udp_request(&buffer[..size]).unwrap();
        assert_eq!(Some(source), response.source);
        assert_eq!(payload, response.payload);
        client.close();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn protect_failure_never_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let protect: net::ProtectSocket = Arc::new(|socket| {
        assert!(socket.peer_addr().is_err());
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "protect failed",
        ))
    });
    let error = net::dial(&listener.local_addr().unwrap().to_string(), &protect)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[test]
fn domain_metadata_does_not_change_captured_sni() {
    let mut template = Template::from_json(TEMPLATE).unwrap();
    let captured = template.domain.clone();
    template.domain = "different.example".into();
    let hello = template.build(&SECRET).unwrap();
    assert_eq!(parse_client_hello(hello.as_ref()).unwrap().domain, captured);
}

#[test]
fn value_config_uses_file_validation_rules() {
    let valid = || {
        ClientConfig::new(
            " example.com:443 ",
            SECRET,
            Template::from_json(TEMPLATE).unwrap(),
        )
    };
    assert_eq!(
        build_client(valid()).unwrap().servers,
        vec!["example.com:443"]
    );
    let cases: [fn(&mut ClientConfig); 8] = [
        |config| config.secret.clear(),
        |config| config.servers = vec!["no-port".into()],
        |config| config.udp_keepalive = config.udp_idle,
        |config| config.max_udp_flows = 0,
        |config| config.max_connections = 0,
        |config| config.handshake_timeout = Duration::ZERO,
        |config| config.mux.max_links = 0,
        |config| config.tcp_notsent_lowat = u32::MAX,
    ];
    for change in cases {
        let mut config = valid();
        change(&mut config);
        assert!(build_client(config).is_err());
    }
}
