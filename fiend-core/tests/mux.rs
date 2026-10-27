use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fiend_core::fingerprint::Template;
use fiend_core::mux::{
    Config, Incoming, LANE_HEADER, Lane, LaneKind, LinkPool, Mux, ServerConnection,
    serve_tcp_with_policy,
};
use fiend_core::target::TargetPolicy;
use fiend_core::transport::{
    AcceptOutcome, FreshnessCheck, OriginProfile, Session, accept, connect,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");
const SECRET: [u8; 32] = [0x42; 32];

type Handler = Arc<dyn Fn(Incoming) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

fn handler<F, Fut>(function: F) -> Handler
where
    F: Fn(Incoming) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    Arc::new(move |incoming| Box::pin(function(incoming)))
}

async fn connect_pair(
    template: &Template,
) -> fiend_core::transport::Result<(Session<DuplexStream>, Session<DuplexStream>)> {
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();
    let (accepted, connected) = tokio::join!(
        accept(server_io, &SECRET, &replay, &profile),
        connect(client_io, &SECRET, template)
    );
    let server = match accepted? {
        AcceptOutcome::Session(session) => *session,
        AcceptOutcome::Passthrough { .. } => {
            return Err(fiend_core::transport::Error::invalid(
                "mux test handshake was passed through",
            ));
        }
    };
    let client = connected?;
    Ok((client, server))
}

fn start_server(
    mut receiver: mpsc::UnboundedReceiver<Session<DuplexStream>>,
    config: Config,
    handler: Handler,
) {
    tokio::spawn(async move {
        while let Some(session) = receiver.recv().await {
            let Ok(mut connection) = ServerConnection::handshake(session, &config).await else {
                continue;
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                while let Ok(Some(incoming)) = connection.accept().await {
                    let handler = handler.clone();
                    tokio::spawn(async move { handler(incoming).await });
                }
            });
        }
    });
}

fn test_pool(handler: Handler) -> LinkPool<DuplexStream> {
    test_pool_with_config(Config::default(), handler)
}

fn test_pool_with_config(config: Config, handler: Handler) -> LinkPool<DuplexStream> {
    let template = Arc::new(Template::from_json(TEMPLATE_JSON).expect("load sample template"));
    let (sender, receiver) = mpsc::unbounded_channel();
    start_server(receiver, config.clone(), handler);
    LinkPool::new(config, move || {
        let template = template.clone();
        let sender = sender.clone();
        Box::pin(async move {
            let (client, server) = connect_pair(&template).await?;
            sender
                .send(server)
                .map_err(|_| fiend_core::transport::Error::invalid("server task stopped"))?;
            Ok(client)
        })
    })
}

async fn echo_lane(incoming: Incoming) {
    if incoming.kind != LaneKind::Tcp {
        incoming.reject(http::StatusCode::BAD_REQUEST);
        return;
    }
    let Ok(mut lane) = incoming.accept().await else {
        return;
    };
    let mut buffer = vec![0u8; 32 << 10];
    loop {
        match lane.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if lane.write_all(&buffer[..count]).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = lane.shutdown().await;
}

async fn drain_lane(incoming: Incoming) {
    let Ok(mut lane) = incoming.accept().await else {
        return;
    };
    let mut buffer = vec![0u8; 64 << 10];
    let mut total = 0u64;
    loop {
        match lane.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => total += count as u64,
            Err(_) => return,
        }
    }
    let _ = lane.write_all(&total.to_be_bytes()).await;
    let _ = lane.shutdown().await;
}

fn echo_handler() -> Handler {
    handler(|incoming| async move { echo_lane(incoming).await })
}

fn mixed_handler() -> Handler {
    handler(|incoming| async move {
        if incoming.authority.starts_with("bulk.test") {
            drain_lane(incoming).await;
        } else {
            echo_lane(incoming).await;
        }
    })
}

async fn wait_for_links(pool: &LinkPool<DuplexStream>, count: usize) {
    timeout(Duration::from_secs(10), async {
        loop {
            if pool.stats().links >= count {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("second link did not appear");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_lane_round_trip_uses_connect_authority() {
    let pool = test_pool(echo_handler());
    let mut lane = Mux::open(&pool, "echo.test:7", LaneKind::Tcp)
        .await
        .expect("open lane");
    assert_eq!(lane.kind(), LaneKind::Tcp);
    assert_eq!(lane.authority(), "echo.test:7");

    lane.write_all(b"ping").await.expect("write");
    let mut response = [0u8; 4];
    lane.read_exact(&mut response).await.expect("read");
    assert_eq!(&response, b"ping");
    lane.shutdown().await.expect("shutdown");
    let mut eof = [0u8; 1];
    assert_eq!(lane.read(&mut eof).await.expect("read EOF"), 0);
    drop(lane);

    let stats = pool.stats();
    assert_eq!(stats.links, 1);
    assert_eq!(stats.lanes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_hundred_streams_run_at_once() {
    let pool = test_pool(echo_handler());
    let mut lanes = Vec::with_capacity(200);
    for index in 0..200u16 {
        let lane = pool
            .open_lane(&format!("many.test:{index}"), LaneKind::Tcp)
            .await
            .unwrap_or_else(|error| panic!("open lane {index}: {error}"));
        lanes.push(lane);
    }
    assert_eq!(pool.stats().lanes, 200);

    for (index, lane) in lanes.iter_mut().enumerate() {
        let payload = format!("stream-{index:03}");
        lane.write_all(payload.as_bytes()).await.expect("write");
        let mut response = vec![0u8; payload.len()];
        lane.read_exact(&mut response).await.expect("read");
        assert_eq!(response, payload.as_bytes());
    }

    for lane in &mut lanes {
        lane.shutdown().await.expect("shutdown");
        let mut eof = [0u8; 1];
        assert_eq!(lane.read(&mut eof).await.expect("read EOF"), 0);
    }
    drop(lanes);
    assert_eq!(pool.stats().lanes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn surplus_link_is_reaped_when_idle() {
    let config = Config {
        surplus_idle: Duration::from_millis(50),
        ..Config::default()
    };
    let pool = test_pool_with_config(config, echo_handler());
    let mut bulk = pool
        .open_lane("bulk.test:9001", LaneKind::Tcp)
        .await
        .expect("open bulk lane");
    let payload = vec![0x5au8; 64 << 10];
    for _ in 0..8 {
        bulk.write_all(&payload).await.expect("write bulk");
    }
    wait_for_links(&pool, 2).await;
    drop(bulk);
    assert_eq!(pool.stats().bulk_links, 0);

    timeout(Duration::from_secs(5), async {
        loop {
            if pool.stats().links == 1 {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("surplus link was not reaped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_lane_splits_the_link_pool() {
    let pool = test_pool(echo_handler());
    let mut bulk = pool
        .open_lane("bulk.test:9000", LaneKind::Tcp)
        .await
        .expect("open bulk lane");
    let bulk_link = bulk.link_id();

    let payload = vec![0x5au8; 64 << 10];
    for _ in 0..8 {
        bulk.write_all(&payload).await.expect("write bulk");
    }
    assert!(bulk.is_bulk());

    wait_for_links(&pool, 2).await;
    let stats = pool.stats();
    assert_eq!(stats.bulk_links, 1);

    let mut click = pool
        .open_lane("click.test:80", LaneKind::Tcp)
        .await
        .expect("open click lane");
    assert_ne!(click.link_id(), bulk_link);
    click.write_all(b"click").await.expect("write click");
    let mut response = [0u8; 5];
    click.read_exact(&mut response).await.expect("read click");
    assert_eq!(&response, b"click");

    drop(click);
    drop(bulk);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transfer_moves_one_hundred_mib() {
    let pool = test_pool(handler(
        |incoming| async move { drain_lane(incoming).await },
    ));
    let mut lane = pool
        .open_lane("bulk.test:10000", LaneKind::Tcp)
        .await
        .expect("open lane");

    let chunk = vec![0xa5u8; 64 << 10];
    let total: u64 = 100 * 1024 * 1024;
    let started = Instant::now();
    let mut sent = 0u64;
    while sent < total {
        let count = (total - sent).min(chunk.len() as u64) as usize;
        lane.write_all(&chunk[..count]).await.expect("write");
        sent += count as u64;
    }
    lane.shutdown().await.expect("shutdown");
    let mut received = [0u8; 8];
    lane.read_exact(&mut received).await.expect("read count");
    assert_eq!(u64::from_be_bytes(received), total);
    let elapsed = started.elapsed();

    let mib = total as f64 / (1024.0 * 1024.0);
    eprintln!(
        "100 MiB in {elapsed:?}: {:.1} MiB/s",
        mib / elapsed.as_secs_f64()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interactive_latency_stays_low_under_bulk() {
    let pool = test_pool(mixed_handler());
    let bulk = pool
        .open_lane("bulk.test:9000", LaneKind::Tcp)
        .await
        .expect("open bulk lane");
    let bulk_link = bulk.link_id();
    let (mut bulk_read, mut bulk_write) = tokio::io::split(bulk);
    let writer = tokio::spawn(async move {
        let chunk = vec![0x33u8; 64 << 10];
        for _ in 0..1024 {
            bulk_write.write_all(&chunk).await.expect("bulk write");
        }
        bulk_write.shutdown().await.expect("bulk shutdown");
        let mut count = [0u8; 8];
        bulk_read.read_exact(&mut count).await.expect("bulk count");
        u64::from_be_bytes(count)
    });

    wait_for_links(&pool, 2).await;
    let mut samples = Vec::with_capacity(20);
    for index in 0..20u16 {
        let started = Instant::now();
        let mut lane = pool
            .open_lane(&format!("click.test:{index}"), LaneKind::Tcp)
            .await
            .expect("open click lane");
        assert_ne!(lane.link_id(), bulk_link);
        lane.write_all(b"x").await.expect("click write");
        let mut response = [0u8; 1];
        lane.read_exact(&mut response).await.expect("click read");
        samples.push(started.elapsed());
        drop(lane);
    }
    samples.sort_unstable();
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    eprintln!("interactive p95 under bulk: {p95:?}");
    assert!(p95 < Duration::from_secs(2), "p95 was {p95:?}");

    let total = writer.await.expect("bulk writer");
    assert_eq!(total, 64 * 1024 * 1024);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_link_survives_keepalive() {
    let config = Config {
        ping_interval: Duration::from_millis(30),
        probe_timeout: Duration::from_millis(200),
        ..Config::default()
    };
    let pool = test_pool_with_config(config, echo_handler());
    let mut lane = pool
        .open_lane("idle.test:80", LaneKind::Tcp)
        .await
        .expect("open lane");
    sleep(Duration::from_millis(150)).await;
    lane.write_all(b"still").await.expect("write");
    let mut response = [0u8; 5];
    lane.read_exact(&mut response).await.expect("read");
    assert_eq!(&response, b"still");
    assert_eq!(pool.stats().links, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_dial_is_retried() {
    let template = Arc::new(Template::from_json(TEMPLATE_JSON).expect("load sample template"));
    let (sender, receiver) = mpsc::unbounded_channel();
    let config = Config {
        backoff_floor: Duration::from_millis(1),
        backoff_ceiling: Duration::from_millis(2),
        open_timeout: Duration::from_secs(5),
        ..Config::default()
    };
    start_server(receiver, config.clone(), echo_handler());

    let attempts = Arc::new(AtomicUsize::new(0));
    let dial_attempts = attempts.clone();
    let pool = LinkPool::new(config, move || {
        let template = template.clone();
        let sender = sender.clone();
        let attempt = dial_attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if attempt == 0 {
                return Err(fiend_core::transport::Error::invalid(
                    "first dial fails on purpose",
                ));
            }
            let (client, server) = connect_pair(&template).await?;
            sender
                .send(server)
                .map_err(|_| fiend_core::transport::Error::invalid("server task stopped"))?;
            Ok(client)
        })
    });

    let mut lane = pool
        .open_lane("retry.test:80", LaneKind::Tcp)
        .await
        .expect("open lane after retry");
    lane.write_all(b"retry").await.expect("write");
    let mut response = [0u8; 5];
    lane.read_exact(&mut response).await.expect("read");
    assert_eq!(&response, b"retry");
    assert!(attempts.load(Ordering::SeqCst) >= 2);
    assert_eq!(pool.stats().links, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_rejects_non_connect_requests() {
    let template = Template::from_json(TEMPLATE_JSON).expect("load sample template");
    let (client, server) = connect_pair(&template).await.expect("connect pair");
    let config = Config::default();
    let server_task = tokio::spawn(async move {
        let mut connection = ServerConnection::handshake(server, &config)
            .await
            .expect("server handshake");
        connection.accept().await
    });

    let (mut sender, connection) = h2::client::handshake(client).await.expect("h2 client");
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    let request = http::Request::builder()
        .method(http::Method::GET)
        .version(http::Version::HTTP_2)
        .uri("https://example.com/")
        .header(LANE_HEADER, "tcp")
        .body(())
        .expect("request");
    let (response, _send) = sender.send_request(request, true).expect("send request");
    let response = response.await.expect("response");
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
    drop(sender);
    driver.abort();
    let accepted = server_task.await.expect("server task").expect("accept");
    assert!(accepted.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_rejects_invalid_connect_metadata() {
    let template = Template::from_json(TEMPLATE_JSON).expect("load sample template");
    let (client, server) = connect_pair(&template).await.expect("connect pair");
    let config = Config::default();
    let server_task = tokio::spawn(async move {
        let mut connection = ServerConnection::handshake(server, &config)
            .await
            .expect("server handshake");
        connection.accept().await
    });

    let (mut sender, connection) = h2::client::handshake(client).await.expect("h2 client");
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });

    let long_domain = format!("https://{}.com:443/", "a".repeat(300));
    let requests = [
        (long_domain.as_str(), "tcp"),
        ("https://example.com:443/", "unknown"),
    ];
    for (uri, lane) in requests {
        let request = http::Request::builder()
            .method(http::Method::CONNECT)
            .version(http::Version::HTTP_2)
            .uri(uri)
            .header(LANE_HEADER, lane)
            .body(())
            .expect("request");
        let (response, _send) = sender.send_request(request, true).expect("send request");
        let response = response.await.expect("response");
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
    }

    drop(sender);
    driver.abort();
    let accepted = server_task.await.expect("server task").expect("accept");
    assert!(accepted.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_tcp_reaches_a_real_target() {
    let target = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind target");
    let target_port = target.local_addr().expect("target address").port();
    tokio::spawn(async move {
        let (mut stream, _) = target.accept().await.expect("accept target");
        let mut buffer = [0u8; 4];
        stream.read_exact(&mut buffer).await.expect("target read");
        assert_eq!(&buffer, b"ping");
        stream.write_all(b"pong").await.expect("target write");
    });

    let template = Template::from_json(TEMPLATE_JSON).expect("load sample template");
    let (client, server) = connect_pair(&template).await.expect("connect pair");
    let server_task = tokio::spawn(async move {
        serve_tcp_with_policy(server, Config::default(), TargetPolicy::allow_all()).await
    });

    let client = Arc::new(tokio::sync::Mutex::new(Some(client)));
    let pool: LinkPool<DuplexStream> = LinkPool::new(Config::default(), move || {
        let client = client.clone();
        Box::pin(async move {
            client
                .lock()
                .await
                .take()
                .ok_or_else(|| fiend_core::transport::Error::invalid("client already used"))
        })
    });
    let mut lane = pool
        .open_lane(&format!("127.0.0.1:{target_port}"), LaneKind::Tcp)
        .await
        .expect("open lane");
    lane.write_all(b"ping").await.expect("lane write");
    let mut response = [0u8; 4];
    lane.read_exact(&mut response).await.expect("lane read");
    assert_eq!(&response, b"pong");
    drop(lane);
    drop(pool);
    timeout(Duration::from_secs(5), server_task)
        .await
        .expect("server task timeout")
        .expect("server task")
        .expect("serve tcp");
}
