use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fiend_core::fingerprint::Template;
use fiend_core::mux::{Config, Incoming, LANE_HEADER, LaneKind, LinkPool, ServerConnection};
use fiend_core::transport::{
    AcceptOutcome, FreshnessCheck, OriginProfile, Session, accept, connect,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
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

async fn settle_until(mut condition: impl FnMut() -> bool) {
    for _ in 0..10000 {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("state did not converge");
}

async fn sustain_load(lane: &mut fiend_core::mux::H2Lane) {
    for _ in 0..4 {
        lane.write_all(&vec![0x5a; 512 << 10]).await.expect("load");
        tokio::time::advance(Duration::from_millis(250)).await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }
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

#[tokio::test(start_paused = true)]
async fn pending_connect_cancellation_and_timeout_release_work() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let pool = Arc::new(test_pool_with_config(
        Config {
            open_timeout: Duration::from_secs(2),
            ..Config::default()
        },
        handler(move |incoming| {
            let sender = sender.clone();
            async move {
                sender.send(incoming).unwrap();
            }
        }),
    ));
    let open = {
        let pool = pool.clone();
        tokio::spawn(async move { pool.open_lane("pending.test:80", LaneKind::Udp).await })
    };
    let incoming = receiver.recv().await.unwrap();
    assert_eq!(pool.stats().pending_lanes, 1);
    open.abort();
    assert!(matches!(open.await, Err(error) if error.is_cancelled()));
    assert_eq!(pool.stats().pending_lanes, 0);
    drop(incoming);
    let open = {
        let pool = pool.clone();
        tokio::spawn(async move { pool.open_lane("timeout.test:80", LaneKind::Udp).await })
    };
    let incoming = receiver.recv().await.unwrap();
    assert_eq!(pool.stats().pending_lanes, 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        open.await.unwrap(),
        Err(fiend_core::mux::Error::Timeout)
    ));
    assert_eq!(pool.stats().pending_lanes, 0);
    drop(incoming);
    assert_eq!(pool.stats().links, 1);
}

#[tokio::test(start_paused = true)]
async fn peer_half_close_keeps_upload_open_and_ordered() {
    let expected: Arc<Vec<u8>> = Arc::new(
        (0..(1 << 20))
            .map(|index| ((index * 17 + index / 256) % 251) as u8)
            .collect(),
    );
    let server_expected = expected.clone();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let pool = test_pool(handler(move |incoming| {
        let expected = server_expected.clone();
        let sender = sender.clone();
        async move {
            let mut lane = incoming.accept().await.unwrap();
            lane.shutdown().await.unwrap();
            let mut received = Vec::new();
            lane.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, *expected);
            sender.send(()).unwrap();
        }
    }));
    let mut lane = pool
        .open_lane("half-close.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    assert_eq!(lane.read(&mut [0; 1]).await.unwrap(), 0);
    assert_eq!(pool.stats().lanes, 1);
    lane.write_all(&expected).await.unwrap();
    lane.shutdown().await.unwrap();
    receiver.recv().await.unwrap();
    assert_eq!(pool.stats().lanes, 1);
    drop(lane);
    assert_eq!(pool.stats().lanes, 0);
}

#[tokio::test(start_paused = true)]
async fn closed_lane_drop_releases_stream_capacity() {
    let pool = test_pool_with_config(
        Config {
            max_links: 1,
            max_concurrent_streams: 1,
            ..Config::default()
        },
        echo_handler(),
    );
    let mut first = pool
        .open_lane("first.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    first.shutdown().await.unwrap();
    assert_eq!(first.read(&mut [0; 1]).await.unwrap(), 0);
    let id = first.link_id();
    drop(first);
    assert_eq!(pool.stats().lanes, 0);
    let next = pool.open_lane("next.test:80", LaneKind::Tcp).await.unwrap();
    assert_eq!(next.link_id(), id);
    assert_eq!(pool.stats().dial_attempts, 1);
}

#[tokio::test(start_paused = true)]
async fn rejection_and_reset_leave_other_lanes_usable() {
    let pool = test_pool(handler(|incoming| async move {
        if incoming.authority.starts_with("reject") {
            incoming.reject(http::StatusCode::BAD_GATEWAY);
        } else {
            echo_lane(incoming).await;
        }
    }));
    let mut healthy = pool
        .open_lane("healthy.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    let id = healthy.link_id();
    let rejected = pool.open_lane("reject.test:80", LaneKind::Udp).await;
    assert!(matches!(
        rejected,
        Err(fiend_core::mux::Error::Rejected(502))
    ));
    let mut optimistic = pool
        .open_lane("reject-tcp.test:80", LaneKind::Tcp)
        .await
        .expect("a TCP lane opens without waiting for the server");
    let error = optimistic
        .read(&mut [0; 1])
        .await
        .expect_err("the rejection arrives on read");
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
    drop(optimistic);
    let mut reset = pool
        .open_lane("reset.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    reset.reset();
    drop(reset);
    healthy.write_all(b"ordered bytes").await.unwrap();
    healthy.shutdown().await.unwrap();
    let mut received = Vec::new();
    healthy.read_to_end(&mut received).await.unwrap();
    assert_eq!(received, b"ordered bytes");
    assert_eq!(healthy.link_id(), id);
    assert_eq!(pool.stats().dial_attempts, 1);
    assert_eq!(pool.stats().pending_lanes, 0);
}

#[tokio::test(start_paused = true)]
async fn slow_readers_keep_both_directions_bounded_and_allow_progress() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let config = Config {
        max_links: 1,
        stream_window: 32 << 10,
        connection_window: 128 << 10,
        max_send_buffer_size: 16 << 10,
        lane_share: 4096,
        ..Config::default()
    };
    let pool = test_pool_with_config(
        config.clone(),
        handler(move |incoming| {
            let sender = sender.clone();
            async move {
                if incoming.authority.starts_with("slow") {
                    sender.send(incoming.accept().await.unwrap()).unwrap();
                } else {
                    echo_lane(incoming).await;
                }
            }
        }),
    );
    let client = pool.open_lane("slow.test:80", LaneKind::Tcp).await.unwrap();
    let server = receiver.recv().await.unwrap();
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut sr, mut sw) = tokio::io::split(server);
    let sent = Arc::new(AtomicUsize::new(0));
    let sent_server = Arc::new(AtomicUsize::new(0));
    let client_count = sent.clone();
    let server_count = sent_server.clone();
    let writer = tokio::spawn(async move {
        for _ in 0..64 {
            cw.write_all(&[0x35; 4096]).await.unwrap();
            client_count.fetch_add(4096, Ordering::AcqRel);
        }
        cw.shutdown().await.unwrap();
    });
    let server_writer = tokio::spawn(async move {
        let first = sw.write(&[0x53; 16384]).await.unwrap();
        assert!(first <= 4096, "server lane_share was not applied");
        server_count.fetch_add(first, Ordering::AcqRel);
        let rest = vec![0x53; (256 << 10) - first];
        for chunk in rest.chunks(4096) {
            sw.write_all(chunk).await.unwrap();
            server_count.fetch_add(chunk.len(), Ordering::AcqRel);
        }
        sw.shutdown().await.unwrap();
    });
    settle_until(|| {
        pool.stats().blocked_writers == 1 && sent_server.load(Ordering::Acquire) >= 32 << 10
    })
    .await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(sent.load(Ordering::Acquire) <= (48 << 10));
    assert!(sent_server.load(Ordering::Acquire) <= (48 << 10));
    let mut click = pool
        .open_lane("click.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    click.write_all(b"click").await.unwrap();
    let mut reply = [0; 5];
    click.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"click");
    let client_read = async {
        let mut bytes = Vec::new();
        cr.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, vec![0x53; 256 << 10]);
    };
    let server_read = async {
        let mut bytes = Vec::new();
        sr.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, vec![0x35; 256 << 10]);
    };
    tokio::join!(client_read, server_read);
    writer.await.unwrap();
    server_writer.await.unwrap();
    assert_eq!(pool.stats().blocked_writers, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_lane_opens_do_not_serialize_dials() {
    let template = Arc::new(Template::from_json(TEMPLATE_JSON).expect("load sample template"));
    let (sender, receiver) = mpsc::unbounded_channel();
    start_server(receiver, Config::default(), echo_handler());
    let completed = Arc::new(AtomicUsize::new(0));
    let dials = completed.clone();
    let pool = Arc::new(LinkPool::new(Config::default(), move || {
        let template = template.clone();
        let sender = sender.clone();
        let completed = dials.clone();
        Box::pin(async move {
            sleep(Duration::from_millis(150)).await;
            let (client, server) = connect_pair(&template).await?;
            completed.fetch_add(1, Ordering::SeqCst);
            sender
                .send(server)
                .map_err(|_| fiend_core::transport::Error::invalid("server task stopped"))?;
            Ok(client)
        })
    }));

    let mut handles = Vec::new();
    for index in 0..6 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            pool.open_lane(&format!("echo.test:{index}"), LaneKind::Tcp)
                .await
        }));
    }
    for handle in handles {
        handle.await.expect("task").expect("open lane");
    }
    let completed = completed.load(Ordering::SeqCst);
    assert!(
        completed < 4,
        "lane opens serialized the dials: {completed} dials before all lanes opened"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_lane_round_trip_uses_connect_authority() {
    let pool = test_pool(echo_handler());
    let mut lane = pool
        .open_lane("echo.test:7", LaneKind::Tcp)
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

#[tokio::test(start_paused = true)]
async fn surplus_link_is_reaped_when_idle() {
    let config = Config {
        surplus_idle: Duration::from_secs(2),
        ..Config::default()
    };
    let pool = test_pool_with_config(config, echo_handler());
    let mut bulk = pool
        .open_lane("bulk.test:9001", LaneKind::Tcp)
        .await
        .expect("open bulk lane");
    sustain_load(&mut bulk).await;
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

#[tokio::test(start_paused = true)]
async fn bulk_lane_splits_the_link_pool() {
    let pool = test_pool(echo_handler());
    let mut bulk = pool
        .open_lane("bulk.test:9000", LaneKind::Tcp)
        .await
        .expect("open bulk lane");
    let bulk_link = bulk.link_id();

    sustain_load(&mut bulk).await;
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

#[tokio::test(start_paused = true)]
async fn interactive_latency_stays_low_under_bulk() {
    let pool = test_pool(mixed_handler());
    let mut bulk = pool
        .open_lane("bulk.test:9000", LaneKind::Tcp)
        .await
        .expect("open bulk lane");
    let bulk_link = bulk.link_id();
    sustain_load(&mut bulk).await;
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
        let started = tokio::time::Instant::now();
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
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    eprintln!("interactive p95 under bulk: {p95:?}");
    assert!(p95 < Duration::from_secs(2), "p95 was {p95:?}");
    assert!(
        !writer.is_finished(),
        "the clicks waited until the bulk transfer ended"
    );

    let total = writer.await.expect("bulk writer");
    assert_eq!(total, 64 * 1024 * 1024 + 4 * (512 << 10));
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
