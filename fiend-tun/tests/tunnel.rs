use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use etherparse::{PacketBuilder, SlicedPacket, TcpOptionElement, TransportSlice};
use fiend_core::client::{Client, ClientConfig};
use fiend_core::fingerprint::Template;
use fiend_core::server::{Server, ServerConfig};
use fiend_core::target::TargetPolicy;
use fiend_tun::packet::{Packet, Reassembly, Transport, udp_packets};
use fiend_tun::{PacketIo, TunEngine, TunOptions};
use hickory_proto::op::{Message, Query};
use hickory_proto::rr::{Name, RData, RecordType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;

struct Pipe {
    input: Mutex<mpsc::Receiver<Vec<u8>>>,
    output: mpsc::Sender<Vec<u8>>,
}

impl PacketIo for Pipe {
    async fn recv(&self, packet: &mut [u8]) -> io::Result<usize> {
        let Some(raw) = self.input.lock().await.recv().await else {
            return Ok(0);
        };
        packet[..raw.len()].copy_from_slice(&raw);
        Ok(raw.len())
    }
    async fn send(&self, packet: &[u8]) -> io::Result<usize> {
        self.output
            .send(packet.to_vec())
            .await
            .map_err(io::Error::other)?;
        Ok(packet.len())
    }
}

struct Peer {
    input: mpsc::Sender<Vec<u8>>,
    output: mpsc::Receiver<Vec<u8>>,
}

impl Peer {
    async fn recv(&mut self) -> Vec<u8> {
        timeout(Duration::from_secs(5), self.output.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn send(&self, packet: Vec<u8>) {
        self.input.send(packet).await.unwrap();
    }
}

async fn stack(
    options: TunOptions,
) -> (
    Arc<TunEngine>,
    Client,
    Peer,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server_config = ServerConfig::new([42; 32], "127.0.0.1", 443);
    server_config.target = TargetPolicy::allow_all();
    let server = Server::new(server_config);
    tokio::spawn(async move {
        server.serve(listener).await.unwrap();
    });
    let client = Client::new(ClientConfig::new(
        address.to_string(),
        [42; 32],
        Template::from_json(include_str!("../../fiend-core/testdata/template.json")).unwrap(),
    ));
    let engine = Arc::new(TunEngine::new(client.clone(), options).unwrap());
    let (input, input_receiver) = mpsc::channel(64);
    let (output, output_receiver) = mpsc::channel(64);
    let runner = engine.clone();
    let task = tokio::spawn(async move {
        runner
            .run(Pipe {
                input: Mutex::new(input_receiver),
                output,
            })
            .await
    });
    (
        engine,
        client,
        Peer {
            input,
            output: output_receiver,
        },
        task,
    )
}

async fn udp_exchange(
    peer: &mut Peer,
    source: SocketAddr,
    target: SocketAddr,
    payload: &[u8],
) -> Vec<u8> {
    for packet in udp_packets(source, target, payload, 1500, 42).unwrap() {
        peer.send(packet).await;
    }
    let mut reassembly = Reassembly::default();
    loop {
        let raw = peer.recv().await;
        assert!(raw.len() <= 1500);
        if let Some(raw) = reassembly.ingest(&raw, Instant::now()).unwrap() {
            let packet = Packet::parse(&raw).unwrap();
            assert_eq!(packet.key.source, target);
            assert_eq!(packet.key.destination, source);
            let Transport::Udp(payload) = packet.transport else {
                panic!("UDP required")
            };
            return payload;
        }
    }
}

fn tcp_packet(
    source: SocketAddr,
    target: SocketAddr,
    sequence: u32,
    ack: Option<u32>,
    syn: bool,
    payload: &[u8],
) -> Vec<u8> {
    let builder = match (source.ip(), target.ip()) {
        (IpAddr::V4(src), IpAddr::V4(dst)) => PacketBuilder::ipv4(src.octets(), dst.octets(), 64),
        (IpAddr::V6(src), IpAddr::V6(dst)) => PacketBuilder::ipv6(src.octets(), dst.octets(), 64),
        _ => panic!("same address family required"),
    };
    let mut builder = builder.tcp(source.port(), target.port(), sequence, 65535);
    if syn {
        builder = builder
            .syn()
            .options(&[TcpOptionElement::MaximumSegmentSize(1400)])
            .unwrap();
    }
    if let Some(ack) = ack {
        builder = builder.ack(ack);
    }
    let mut packet = Vec::new();
    builder.write(&mut packet, payload).unwrap();
    packet
}

async fn tcp_handshake(peer: &mut Peer, source: SocketAddr, target: SocketAddr) -> u32 {
    let syn = tcp_packet(source, target, 1000, None, true, &[]);
    peer.send(syn.clone()).await;
    peer.send(syn).await;
    let response = peer.recv().await;
    let parsed = SlicedPacket::from_ip(&response).unwrap();
    let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
        panic!("TCP required")
    };
    assert!(tcp.syn() && tcp.ack());
    assert_eq!(tcp.acknowledgment_number(), 1001);
    let next = tcp.sequence_number().wrapping_add(1);
    peer.send(tcp_packet(source, target, 1001, Some(next), false, &[]))
        .await;
    next
}

#[tokio::test]
async fn udp_preserves_large_datagrams_ipv4_and_ipv6_and_recovers() {
    for (bind, source) in [
        ("127.0.0.1:0", "10.0.0.2:30000"),
        ("[::1]:0", "[fd00::2]:30000"),
    ] {
        let echo = UdpSocket::bind(bind).await.unwrap();
        let target = echo.local_addr().unwrap();
        tokio::spawn(async move {
            let mut packet = vec![0; 65535];
            loop {
                let (size, from) = echo.recv_from(&mut packet).await.unwrap();
                echo.send_to(&packet[..size], from).await.unwrap();
            }
        });
        let (engine, client, mut peer, task) = stack(TunOptions::default()).await;
        let largest = if cfg!(target_os = "macos") {
            8192
        } else {
            65507
        };
        for size in [8, 4096, largest] {
            let payload = vec![0xa5; size];
            assert_eq!(
                udp_exchange(&mut peer, source.parse().unwrap(), target, &payload).await,
                payload
            );
        }
        let before = client.stats().link_ids;
        engine.network_changed();
        assert_eq!(
            udp_exchange(&mut peer, source.parse().unwrap(), target, b"new network").await,
            b"new network"
        );
        assert_ne!(client.stats().link_ids, before);
        assert!(engine.stats().uploaded > largest as u64);
        assert!(engine.stats().downloaded > largest as u64);
        engine.stop();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn udp_full_duplex_keeps_response_framing() {
    let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = echo.local_addr().unwrap();
    tokio::spawn(async move {
        let mut packet = vec![0; 9000];
        for _ in 0..8 {
            let (size, source) = echo.recv_from(&mut packet).await.unwrap();
            echo.send_to(&packet[..size], source).await.unwrap();
        }
    });
    let (engine, _, mut peer, task) = stack(TunOptions::default()).await;
    let source = "10.0.0.2:30000".parse().unwrap();
    for index in 0..8u8 {
        for packet in
            udp_packets(source, target, &vec![index; 8192], 1500, u32::from(index)).unwrap()
        {
            peer.send(packet).await;
        }
    }
    let mut reassembly = Reassembly::default();
    let mut received = std::collections::HashSet::new();
    while received.len() < 8 {
        let packet = peer.recv().await;
        if let Some(raw) = reassembly.ingest(&packet, Instant::now()).unwrap() {
            let Transport::Udp(payload) = Packet::parse(&raw).unwrap().transport else {
                panic!("UDP required")
            };
            assert_eq!(payload.len(), 8192);
            assert!(payload.iter().all(|byte| *byte == payload[0]));
            assert!(received.insert(payload[0]));
        }
    }
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn dns_is_local_and_fake_ip_opens_a_domain_lane() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let (mut read, mut write) = stream.split();
        tokio::io::copy(&mut read, &mut write).await.unwrap();
    });
    let (engine, client, mut peer, task) = stack(TunOptions::default()).await;
    let mut query = Message::query();
    query.metadata.id = 123;
    query.add_query(Query::query(
        Name::from_ascii("localhost.").unwrap(),
        RecordType::A,
    ));
    let response = udp_exchange(
        &mut peer,
        "10.0.0.2:30000".parse().unwrap(),
        "8.8.8.8:53".parse().unwrap(),
        &query.to_vec().unwrap(),
    )
    .await;
    assert_eq!(client.stats().dial_attempts, 0);
    let message = Message::from_vec(&response).unwrap();
    let RData::A(ip) = message.answers[0].data else {
        panic!("A record required")
    };
    let target = SocketAddr::new(ip.0.into(), port);
    let local = Message::from_vec(&engine.answer_dns(&query.to_vec().unwrap()).unwrap()).unwrap();
    assert!(matches!(local.answers[0].data, RData::A(same) if same == ip));
    assert!(engine.answer_dns(&[1]).is_none());
    let source = "10.0.0.2:30001".parse().unwrap();
    let next = tcp_handshake(&mut peer, source, target).await;
    peer.send(tcp_packet(
        source,
        target,
        1001,
        Some(next),
        false,
        b"domain flow",
    ))
    .await;
    let mut echoed = Vec::new();
    while echoed.len() < 11 {
        let raw = peer.recv().await;
        let parsed = SlicedPacket::from_ip(&raw).unwrap();
        let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
            panic!("TCP required")
        };
        echoed.extend_from_slice(tcp.payload());
        peer.send(tcp_packet(
            source,
            target,
            1012,
            Some(
                tcp.sequence_number()
                    .wrapping_add(tcp.payload().len() as u32),
            ),
            false,
            &[],
        ))
        .await;
    }
    assert_eq!(echoed, b"domain flow");
    assert_eq!(client.stats().dial_attempts, 1);
    assert_eq!(engine.stats().flows, 1);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn disabled_ipv6_is_closed_and_does_not_dial() {
    let (engine, client, mut peer, task) = stack(TunOptions {
        ipv6: false,
        ..TunOptions::default()
    })
    .await;
    peer.send(tcp_packet(
        "[fd00::2]:30000".parse().unwrap(),
        "[2606:4700:4700::1111]:443".parse().unwrap(),
        1000,
        None,
        true,
        &[],
    ))
    .await;
    let response = peer.recv().await;
    let parsed = SlicedPacket::from_ip(&response).unwrap();
    let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
        panic!("TCP required")
    };
    assert!(tcp.rst());
    assert_eq!(tcp.acknowledgment_number(), 1001);
    assert_eq!(client.stats().dial_attempts, 0);
    assert_eq!(engine.stats().dropped_packets, 1);
    engine.stop();
    task.await.unwrap().unwrap();
}

async fn tcp_reply(
    peer: &mut Peer,
    source: SocketAddr,
    target: SocketAddr,
    payload: &[u8],
    wanted: usize,
) -> Vec<u8> {
    let next = tcp_handshake(peer, source, target).await;
    peer.send(tcp_packet(source, target, 1001, Some(next), false, payload))
        .await;
    let mut answer = Vec::new();
    while answer.len() < wanted {
        let raw = peer.recv().await;
        assert!(raw.len() <= 1500);
        let parsed = SlicedPacket::from_ip(&raw).unwrap();
        let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
            panic!("TCP required")
        };
        assert!(!tcp.rst());
        if !tcp.payload().is_empty() {
            assert_eq!(
                tcp.sequence_number(),
                next.wrapping_add(answer.len() as u32)
            );
            answer.extend_from_slice(tcp.payload());
            peer.send(tcp_packet(
                source,
                target,
                1001 + payload.len() as u32,
                Some(next.wrapping_add(answer.len() as u32)),
                false,
                &[],
            ))
            .await;
        }
    }
    answer
}

#[tokio::test]
async fn tcp_ipv6_uses_real_lanes_and_preserves_payload() {
    let echo = TcpListener::bind("[::1]:0").await.unwrap();
    let target = echo.local_addr().unwrap();
    let payload = vec![0x5a; 65536];
    let response = payload.clone();
    tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut request = [0; 2];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"go");
        stream.write_all(&response).await.unwrap();
    });
    let (engine, client, mut peer, task) = stack(TunOptions::default()).await;
    let answer = tcp_reply(
        &mut peer,
        "[fd00::2]:30000".parse().unwrap(),
        target,
        b"go",
        payload.len(),
    )
    .await;
    assert_eq!(answer, payload);
    assert_eq!(client.stats().dial_attempts, 1);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn tcp_dns_is_local_and_does_not_open_a_lane() {
    let (engine, client, mut peer, task) = stack(TunOptions::default()).await;
    let mut query = Message::query();
    query.add_query(Query::query(
        Name::from_ascii("example.com").unwrap(),
        RecordType::AAAA,
    ));
    let wire = query.to_vec().unwrap();
    let mut request = (wire.len() as u16).to_be_bytes().to_vec();
    request.extend_from_slice(&wire);
    let answer = tcp_reply(
        &mut peer,
        "10.0.0.2:30000".parse().unwrap(),
        "9.9.9.9:53".parse().unwrap(),
        &request,
        2,
    )
    .await;
    let length = u16::from_be_bytes([answer[0], answer[1]]) as usize;
    assert_eq!(answer.len(), length + 2);
    let answer = Message::from_vec(&answer[2..]).unwrap();
    assert!(answer.answers.is_empty());
    assert_eq!(client.stats().dial_attempts, 0);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn lan_udp_bypass_uses_protect_and_no_server_link() {
    let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buffer = [0; 1024];
        let (size, source) = socket.recv_from(&mut buffer).await.unwrap();
        socket.send_to(&buffer[..size], source).await.unwrap();
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = calls.clone();
    let options = TunOptions {
        bypass_lan: true,
        protect_socket: Arc::new(move |socket| {
            assert!(socket.peer_addr().is_err());
            hook_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        ..TunOptions::default()
    };
    let (engine, client, mut peer, task) = stack(options).await;
    let target = SocketAddr::new("127.0.0.1".parse().unwrap(), port);
    assert_eq!(
        udp_exchange(
            &mut peer,
            "10.0.0.2:30000".parse().unwrap(),
            target,
            b"LAN bypass"
        )
        .await,
        b"LAN bypass"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.stats().dial_attempts, 0);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn desktop_tcp_throughput() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = echo.local_addr().unwrap();
    let size = 8 << 20;
    tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut request = [0; 2];
        stream.read_exact(&mut request).await.unwrap();
        stream.write_all(&vec![0x5a; size]).await.unwrap();
    });
    let (engine, _, mut peer, task) = stack(TunOptions::default()).await;
    let start = Instant::now();
    let answer = tcp_reply(
        &mut peer,
        "10.0.0.2:30000".parse().unwrap(),
        target,
        b"go",
        size,
    )
    .await;
    let elapsed = start.elapsed().as_secs_f64();
    assert_eq!(answer, vec![0x5a; size]);
    eprintln!(
        "desktop fake TUN TCP: {size} bytes in {elapsed:.3} s, {:.2} Mbit/s",
        size as f64 * 8.0 / elapsed / 1_000_000.0
    );
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn tcp_limit_rejects_new_tuple_and_stop_cancels_waiting_reads() {
    let (engine, _, mut peer, task) = stack(TunOptions {
        max_tcp_flows: 1,
        ..TunOptions::default()
    })
    .await;
    let target = "192.0.2.1:443".parse().unwrap();
    peer.send(tcp_packet(
        "10.0.0.2:30000".parse().unwrap(),
        target,
        1000,
        None,
        true,
        &[],
    ))
    .await;
    peer.send(tcp_packet(
        "10.0.0.2:30001".parse().unwrap(),
        target,
        1000,
        None,
        true,
        &[],
    ))
    .await;
    loop {
        let response = peer.recv().await;
        let parsed = SlicedPacket::from_ip(&response).unwrap();
        let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
            panic!("TCP required")
        };
        if tcp.destination_port() == 30001 {
            assert!(tcp.rst());
            break;
        }
    }
    engine.stop();
    timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

async fn dns_server(
    code: hickory_proto::op::ResponseCode,
    address: std::net::Ipv4Addr,
) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buffer = [0u8; 2048];
        loop {
            let (size, from) = socket.recv_from(&mut buffer).await.unwrap();
            let request = Message::from_vec(&buffer[..size]).unwrap();
            let mut response = Message::response(request.metadata.id, request.metadata.op_code);
            response.metadata.response_code = code;
            let query = request.queries[0].clone();
            response.add_query(query.clone());
            if code == hickory_proto::op::ResponseCode::NoError && query.query_type == RecordType::A
            {
                response.add_answer(hickory_proto::rr::Record::from_rdata(
                    query.name,
                    60,
                    RData::A(hickory_proto::rr::rdata::A(address)),
                ));
            }
            socket
                .send_to(&response.to_vec().unwrap(), from)
                .await
                .unwrap();
        }
    });
    local
}

async fn fake_ip(
    peer: &mut Peer,
    domain: &str,
) -> (std::net::Ipv4Addr, hickory_proto::op::ResponseCode) {
    let mut query = Message::query();
    query.metadata.id = 7;
    query.add_query(Query::query(
        Name::from_ascii(domain).unwrap(),
        RecordType::A,
    ));
    let response = udp_exchange(
        peer,
        "10.0.0.2:30000".parse().unwrap(),
        "8.8.8.8:53".parse().unwrap(),
        &query.to_vec().unwrap(),
    )
    .await;
    let message = Message::from_vec(&response).unwrap();
    let ip = match message.answers.first().map(|record| &record.data) {
        Some(RData::A(ip)) => ip.0,
        _ => std::net::Ipv4Addr::UNSPECIFIED,
    };
    (ip, message.metadata.response_code)
}

async fn tcp_echo_once(peer: &mut Peer, source: SocketAddr, target: SocketAddr, payload: &[u8]) {
    let answer = tcp_reply(peer, source, target, payload, payload.len()).await;
    assert_eq!(answer, payload);
}

#[tokio::test]
async fn direct_domain_connects_locally_without_a_lane() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut data = [0; 6];
        stream.read_exact(&mut data).await.unwrap();
        stream.write_all(&data).await.unwrap();
    });
    let resolver = dns_server(
        hickory_proto::op::ResponseCode::NoError,
        std::net::Ipv4Addr::LOCALHOST,
    )
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let hook = calls.clone();
    let (engine, client, mut peer, task) = stack(TunOptions {
        direct_rules: "full:direct.test\n".into(),
        direct_resolver: resolver.to_string(),
        protect_socket: Arc::new(move |_| {
            hook.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        ..TunOptions::default()
    })
    .await;
    let (ip, _) = fake_ip(&mut peer, "direct.test").await;
    assert!(fiend_tun::dns::is_fake_ip(ip));
    let target = SocketAddr::new(ip.into(), port);
    tcp_echo_once(
        &mut peer,
        "10.0.0.2:30001".parse().unwrap(),
        target,
        b"direct",
    )
    .await;
    assert_eq!(client.stats().dial_attempts, 0);
    assert!(calls.load(Ordering::SeqCst) >= 2);
    engine.stop();
    task.await.unwrap().unwrap();
}

fn client_hello(host: &str) -> Vec<u8> {
    let mut sni = ((host.len() + 3) as u16).to_be_bytes().to_vec();
    sni.push(0);
    sni.extend_from_slice(&(host.len() as u16).to_be_bytes());
    sni.extend_from_slice(host.as_bytes());
    let mut extensions = vec![0, 0];
    extensions.extend_from_slice(&(sni.len() as u16).to_be_bytes());
    extensions.extend_from_slice(&sni);
    let mut body = vec![3, 3];
    body.extend_from_slice(&[7; 32]);
    body.push(0);
    body.extend_from_slice(&[0, 2, 0x13, 0x01, 1, 0]);
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);
    let mut handshake = vec![1];
    handshake.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    handshake.extend_from_slice(&body);
    let mut record = vec![0x16, 3, 1];
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

#[tokio::test]
async fn dpi_domain_goes_direct_with_a_split_client_hello() {
    let hello = client_hello("www.dpi.test");
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    let expected = hello.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut data = vec![0; expected.len() + 5];
        stream.read_exact(&mut data).await.unwrap();
        let mut payload = Vec::new();
        let mut records = 0;
        let mut rest = &data[..];
        while !rest.is_empty() {
            assert_eq!(rest[0], 0x16);
            let length = usize::from(u16::from_be_bytes([rest[3], rest[4]]));
            payload.extend_from_slice(&rest[5..5 + length]);
            rest = &rest[5 + length..];
            records += 1;
        }
        assert_eq!(records, 2);
        assert_eq!(payload, expected[5..]);
        stream.write_all(b"ok").await.unwrap();
    });
    let resolver = dns_server(
        hickory_proto::op::ResponseCode::NoError,
        std::net::Ipv4Addr::LOCALHOST,
    )
    .await;
    let (engine, client, mut peer, task) = stack(TunOptions {
        dpi_rules: "domain:dpi.test\n".into(),
        direct_resolver: resolver.to_string(),
        ..TunOptions::default()
    })
    .await;
    let (ip, _) = fake_ip(&mut peer, "www.dpi.test").await;
    let target = SocketAddr::new(ip.into(), port);
    let answer = tcp_reply(
        &mut peer,
        "10.0.0.2:30001".parse().unwrap(),
        target,
        &hello,
        2,
    )
    .await;
    assert_eq!(answer, b"ok");
    server.await.unwrap();
    assert_eq!(client.stats().dial_attempts, 0);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn proxied_domain_still_uses_a_lane() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut data = [0; 5];
        stream.read_exact(&mut data).await.unwrap();
        stream.write_all(&data).await.unwrap();
    });
    let (engine, client, mut peer, task) = stack(TunOptions {
        direct_rules: "full:other.test\n".into(),
        ..TunOptions::default()
    })
    .await;
    let (ip, _) = fake_ip(&mut peer, "localhost").await;
    let target = SocketAddr::new(ip.into(), port);
    tcp_echo_once(
        &mut peer,
        "10.0.0.2:30001".parse().unwrap(),
        target,
        b"proxy",
    )
    .await;
    assert_eq!(client.stats().dial_attempts, 1);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn resolver_failure_falls_back_to_a_lane() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = echo.accept().await.unwrap();
        let mut data = [0; 8];
        stream.read_exact(&mut data).await.unwrap();
        stream.write_all(&data).await.unwrap();
    });
    let resolver = dns_server(
        hickory_proto::op::ResponseCode::ServFail,
        std::net::Ipv4Addr::LOCALHOST,
    )
    .await;
    let (engine, client, mut peer, task) = stack(TunOptions {
        direct_rules: "domain:localhost\n".into(),
        direct_resolver: resolver.to_string(),
        ..TunOptions::default()
    })
    .await;
    let (ip, _) = fake_ip(&mut peer, "localhost").await;
    let target = SocketAddr::new(ip.into(), port);
    tcp_echo_once(
        &mut peer,
        "10.0.0.2:30001".parse().unwrap(),
        target,
        b"fallback",
    )
    .await;
    assert_eq!(client.stats().dial_attempts, 1);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn direct_domain_udp_uses_a_protected_socket() {
    let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buffer = [0; 1024];
        let (size, from) = echo.recv_from(&mut buffer).await.unwrap();
        echo.send_to(&buffer[..size], from).await.unwrap();
    });
    let resolver = dns_server(
        hickory_proto::op::ResponseCode::NoError,
        std::net::Ipv4Addr::LOCALHOST,
    )
    .await;
    let (engine, client, mut peer, task) = stack(TunOptions {
        direct_rules: "domain:direct.test\n".into(),
        direct_resolver: resolver.to_string(),
        ..TunOptions::default()
    })
    .await;
    let (ip, _) = fake_ip(&mut peer, "udp.direct.test").await;
    let target = SocketAddr::new(ip.into(), port);
    assert_eq!(
        udp_exchange(
            &mut peer,
            "10.0.0.2:30002".parse().unwrap(),
            target,
            b"quic"
        )
        .await,
        b"quic"
    );
    assert_eq!(client.stats().dial_attempts, 0);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn blocked_domain_gets_nxdomain_and_other_names_do_not() {
    let (engine, _, mut peer, task) = stack(TunOptions {
        block_rules: "domain:ads.example\n".into(),
        ..TunOptions::default()
    })
    .await;
    let (_, code) = fake_ip(&mut peer, "tracker.ads.example").await;
    assert_eq!(code, hickory_proto::op::ResponseCode::NXDomain);
    let (ip, code) = fake_ip(&mut peer, "fine.example").await;
    assert_eq!(code, hickory_proto::op::ResponseCode::NoError);
    assert!(fiend_tun::dns::is_fake_ip(ip));
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn null_network_flows_and_netbios_udp_are_dropped() {
    let (engine, client, mut peer, task) = stack(TunOptions::default()).await;
    peer.send(tcp_packet(
        "10.0.0.2:30000".parse().unwrap(),
        "0.0.0.0:443".parse().unwrap(),
        1000,
        None,
        true,
        &[],
    ))
    .await;
    let response = peer.recv().await;
    let parsed = SlicedPacket::from_ip(&response).unwrap();
    let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
        panic!("TCP required")
    };
    assert!(tcp.rst());
    assert_eq!(engine.stats().dropped_packets, 1);
    for packet in udp_packets(
        "10.0.0.2:30001".parse().unwrap(),
        "203.0.113.5:137".parse().unwrap(),
        b"netbios",
        1500,
        9,
    )
    .unwrap()
    {
        peer.send(packet).await;
    }
    for _ in 0..50 {
        if engine.stats().dropped_packets == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(engine.stats().dropped_packets, 2);
    assert_eq!(client.stats().dial_attempts, 0);
    engine.stop();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn invalid_rules_or_resolver_are_rejected() {
    let client = Client::new(ClientConfig::new(
        "127.0.0.1:1".to_string(),
        [42; 32],
        Template::from_json(include_str!("../../fiend-core/testdata/template.json")).unwrap(),
    ));
    for options in [
        TunOptions {
            direct_rules: "bogus:x".into(),
            ..TunOptions::default()
        },
        TunOptions {
            block_rules: "ip:1.2.3.0/24".into(),
            ..TunOptions::default()
        },
        TunOptions {
            direct_resolver: "dns.example".into(),
            ..TunOptions::default()
        },
    ] {
        assert!(TunEngine::new(client.clone(), options).is_err());
    }
}

#[tokio::test]
async fn fake_ip_outlives_its_ttl_and_engine_restarts() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = echo.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    let pool = Arc::new(std::sync::Mutex::new(
        fiend_tun::dns::FakeDns::new(16, 1).unwrap(),
    ));
    let options = || TunOptions {
        fake_dns: Some(pool.clone()),
        ..TunOptions::default()
    };
    let (engine, _, mut peer, task) = stack(options()).await;
    let (ip, code) = fake_ip(&mut peer, "localhost.").await;
    assert_eq!(code, hickory_proto::op::ResponseCode::NoError);
    let target = SocketAddr::new(ip.into(), port);
    tcp_echo_once(
        &mut peer,
        "10.0.0.2:30001".parse().unwrap(),
        target,
        b"first",
    )
    .await;
    engine.stop();
    task.await.unwrap().unwrap();

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (engine, _, mut peer, task) = stack(options()).await;
    tcp_echo_once(
        &mut peer,
        "10.0.0.2:30002".parse().unwrap(),
        target,
        b"after TTL",
    )
    .await;
    let (other, _) = fake_ip(&mut peer, "other.example.").await;
    assert_ne!(other, ip);
    let (same, _) = fake_ip(&mut peer, "localhost.").await;
    assert_eq!(same, ip);
    engine.stop();
    task.await.unwrap().unwrap();
}
