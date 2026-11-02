use super::*;
use crate::crypto::RecordCipher;
use crate::mux::ServerConnection;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

fn session_pair(config: Config) -> Session<DuplexStream> {
    let (client, server) = tokio::io::duplex(64 << 10);
    let session = |io| {
        Session::new(
            io,
            RecordCipher::new(&[1; 32], [2; 12]),
            RecordCipher::new(&[1; 32], [2; 12]),
            16384,
        )
    };
    let server = session(server);
    tokio::spawn(async move {
        let mut connection = ServerConnection::handshake(server, &config).await.unwrap();
        while let Ok(Some(incoming)) = connection.accept().await {
            tokio::spawn(async move {
                let mut lane = incoming.accept().await.unwrap();
                let mut buffer = [0; 4096];
                while let Ok(count) = lane.read(&mut buffer).await {
                    if count == 0 {
                        break;
                    }
                    if lane.write_all(&buffer[..count]).await.is_err() {
                        return;
                    }
                }
                let _ = lane.shutdown().await;
            });
        }
    });
    session(client)
}

fn pool(config: Config) -> LinkPool<DuplexStream> {
    let server_config = config.clone();
    LinkPool::new(config, move || {
        let session = session_pair(server_config.clone());
        async { Ok(session) }
    })
}

async fn add_link(pool: &LinkPool<DuplexStream>) -> Arc<ClientLink> {
    let id = {
        let mut state = pool.inner.state();
        let id = state.next_id;
        state.next_id += 1;
        id
    };
    let session = session_pair(pool.config().clone());
    let link = Arc::new(
        ClientLink::connect(session, pool.config(), id, pool.inner.wake.clone())
            .await
            .unwrap(),
    );
    pool.inner.state().links.push(link.clone());
    link
}

async fn pressure(link: &ClientLink, inbound: bool) {
    for _ in 0..3 {
        link.shared().note_io(256 << 10, inbound);
        tokio::time::advance(LOAD_INTERVAL).await;
        link.shared().sample_load(256 << 10);
    }
    assert!(link.is_bulk());
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

#[tokio::test(start_paused = true)]
async fn network_change_cancels_old_dial_and_keeps_waiters() {
    let calls = Arc::new(AtomicUsize::new(0));
    let dial_calls = calls.clone();
    let pool = Arc::new(LinkPool::new(Config::default(), move || {
        let first = dial_calls.fetch_add(1, Ordering::SeqCst) == 0;
        async move {
            if first {
                pending::<()>().await;
            }
            Ok(session_pair(Config::default()))
        }
    }));
    let waiter_pool = pool.clone();
    let waiter = tokio::spawn(async move {
        waiter_pool
            .open_lane("example.com:443", LaneKind::Tcp)
            .await
    });
    settle_until(|| calls.load(Ordering::SeqCst) == 1).await;
    pool.network_changed();
    let mut lane = waiter.await.unwrap().unwrap();
    lane.write_all(b"new network").await.unwrap();
    let mut result = [0; 11];
    lane.read_exact(&mut result).await.unwrap();
    assert_eq!(&result, b"new network");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn network_change_discards_a_late_dial_result() {
    let pool = pool(Config::default());
    pool.inner.waiters.store(1, Ordering::SeqCst);
    let generation = pool.inner.state().generation;
    let dial = pool.inner.maintain_generation(generation).unwrap();
    let link = dial.await.unwrap();
    pool.inner.waiters.store(0, Ordering::SeqCst);
    pool.network_changed();
    pool.inner.dial_finished_generation(generation, Ok(link));
    assert_eq!(pool.stats().links, 0);
    assert!(!pool.stats().dialing);
    assert_eq!(pool.stats().dial_failures, 0);
}

#[tokio::test(start_paused = true)]
async fn selection_reserves_before_reaping_and_drop_releases() {
    let pool = pool(Config {
        surplus_idle: Duration::from_secs(5),
        ..Config::default()
    });
    let first = add_link(&pool).await;
    add_link(&pool).await;
    tokio::time::advance(Duration::from_secs(6)).await;
    let reserved = pool
        .inner
        .acquire(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(reserved.link.id(), first.id());
    assert_eq!(pool.stats().pending_lanes, 1);
    assert!(pool.inner.maintain().is_none());
    assert!(!first.is_closed());
    assert_eq!(pool.stats().links, 1);
    drop(reserved);
    assert_eq!(pool.stats().pending_lanes, 0);
    assert_eq!(first.shared().idle_for_work(), Some(Duration::ZERO));
}

#[tokio::test(start_paused = true)]
async fn continuous_idle_and_unused_spare_expiry_retain_one_link() {
    let pool = pool(Config {
        surplus_idle: Duration::from_secs(5),
        ..Config::default()
    });
    let first = add_link(&pool).await;
    let held = first.reserve();
    let spare = add_link(&pool).await;
    tokio::time::advance(Duration::from_secs(4)).await;
    let reused = pool
        .inner
        .acquire(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(reused.link.id(), spare.id());
    drop(reused);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(pool.inner.maintain().is_none());
    assert_eq!(pool.stats().links, 2);
    tokio::time::advance(Duration::from_secs(3)).await;
    assert!(pool.inner.maintain().is_none());
    assert!(spare.is_closed());
    let unused = add_link(&pool).await;
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(pool.inner.maintain().is_none());
    assert!(unused.is_closed());
    drop(held);
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(pool.inner.maintain().is_none());
    assert_eq!(pool.stats().link_ids, vec![first.id()]);
    assert_eq!(pool.stats().idle_closures, 2);
}

#[tokio::test(start_paused = true)]
async fn cancellation_and_timeout_release_reservations() {
    let pool = pool(Config::default());
    add_link(&pool).await;
    let reservation = pool
        .inner
        .acquire(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    let future = async move {
        let _reservation = reservation;
        pending::<()>().await;
    };
    let mut future = Box::pin(future);
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(pool.stats().pending_lanes, 1);
    drop(future);
    assert_eq!(pool.stats().pending_lanes, 0);
    let reservation = pool
        .inner
        .acquire(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_secs(1), async move {
            let _reservation = reservation;
            pending::<()>().await;
        })
        .await
        .is_err()
    );
    assert_eq!(pool.stats().pending_lanes, 0);
}

#[tokio::test(start_paused = true)]
async fn recent_load_recovers_and_brief_bursts_do_not_expand() {
    let pool = pool(Config::default());
    let link = add_link(&pool).await;
    let held = link.reserve();
    for _ in 0..4 {
        link.shared().note_io(1 << 20, false);
        tokio::time::advance(LOAD_INTERVAL).await;
        assert!(pool.inner.maintain().is_none());
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(pool.inner.maintain().is_none());
    }
    pressure(&link, true).await;
    let dial = pool
        .inner
        .maintain()
        .expect("sustained load permits a spare");
    pool.inner.dial_finished(dial.await);
    assert_eq!(pool.stats().links, 2);
    assert_eq!(pool.stats().dial_attempts, 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    link.shared().sample_load(256 << 10);
    assert!(!link.is_bulk());
    drop(held);
    let reservation = pool
        .inner
        .acquire(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(reservation.link.id(), link.id());
}

#[tokio::test(start_paused = true)]
async fn optional_dial_never_blocks_existing_capacity_or_moves_work() {
    let pool: LinkPool<DuplexStream> = LinkPool::new(Config::default(), pending);
    let link = add_link(&pool).await;
    let held = link.reserve();
    pressure(&link, false).await;
    let dial = pool.inner.maintain().unwrap();
    let before = Instant::now();
    let reservation = pool
        .inner
        .acquire(before + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(Instant::now(), before);
    assert_eq!(reservation.link.id(), held.link.id());
    pool.inner.dial_finished(Err(Error::Timeout));
    drop(dial);
    assert!(!link.is_closed());
    assert_eq!(held.link.id(), link.id());
    for _ in 0..20 {
        let reservation = pool
            .inner
            .acquire(Instant::now() + Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(reservation.link.id(), link.id());
        assert!(pool.inner.maintain().is_none());
    }
    assert_eq!(pool.stats().dial_attempts, 1);
    assert_eq!(pool.stats().dial_failures, 1);
}

#[tokio::test(start_paused = true)]
async fn sustained_pressure_does_not_replace_an_unused_spare() {
    let pool = pool(Config {
        surplus_idle: Duration::from_secs(1),
        ..Config::default()
    });
    let link = add_link(&pool).await;
    let _held = link.reserve();
    pressure(&link, false).await;
    let dial = pool.inner.maintain().unwrap();
    pool.inner.dial_finished(dial.await);
    for _ in 0..80 {
        link.shared().note_io(256 << 10, false);
        tokio::time::advance(LOAD_INTERVAL).await;
        assert!(pool.inner.maintain().is_none());
    }
    assert_eq!(pool.stats().links, 1);
    assert_eq!(pool.stats().idle_closures, 1);
    assert_eq!(pool.stats().dial_attempts, 1);
}

#[tokio::test(start_paused = true)]
async fn stream_capacity_selection_uses_other_links() {
    let pool = pool(Config {
        max_concurrent_streams: 1,
        ..Config::default()
    });
    let first = add_link(&pool).await;
    let held = first.reserve();
    let second = add_link(&pool).await;
    let reservation = pool
        .inner
        .acquire(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(reservation.link.id(), second.id());
    assert_eq!(held.link.id(), first.id());
    assert_eq!(pool.stats().dial_attempts, 0);
}

#[tokio::test(start_paused = true)]
async fn speculative_failures_obey_backoff_and_attempt_limit() {
    let pool: LinkPool<DuplexStream> = LinkPool::new(Config::default(), || async {
        Err(transport::Error::invalid("test dial failure"))
    });
    let link = add_link(&pool).await;
    let _held = link.reserve();
    link.shared().blocked(true);
    pressure(&link, false).await;
    for attempt in 1..=3 {
        let dial = pool.inner.maintain().expect("bounded speculative retry");
        pool.inner.dial_finished(dial.await);
        assert_eq!(pool.stats().dial_attempts, attempt);
        let delay = pool.inner.state().backoff.delay;
        tokio::time::advance(delay / 2).await;
        for _ in 0..20 {
            let reservation = pool
                .inner
                .acquire(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            assert_eq!(reservation.link.id(), link.id());
            assert!(pool.inner.maintain().is_none());
        }
        tokio::time::advance(delay - delay / 2).await;
    }
    for _ in 0..20 {
        tokio::time::advance(LOAD_INTERVAL).await;
        assert!(pool.inner.maintain().is_none());
    }
    assert_eq!(pool.stats().dial_attempts, 3);
    assert_eq!(pool.stats().dial_failures, 3);
    assert!(!link.is_closed());
    link.shared().blocked(false);
}

#[tokio::test(start_paused = true)]
async fn peers_that_close_before_connect_response_obey_retry_backoff() {
    let times = Arc::new(Mutex::new(Vec::new()));
    let dial_times = times.clone();
    let pool = LinkPool::new(
        Config {
            open_timeout: Duration::from_secs(5),
            ..Config::default()
        },
        move || {
            dial_times.lock().unwrap().push(Instant::now());
            async move {
                let (client, server) = tokio::io::duplex(64 << 10);
                let session = |io| {
                    Session::new(
                        io,
                        RecordCipher::new(&[1; 32], [2; 12]),
                        RecordCipher::new(&[1; 32], [2; 12]),
                        16384,
                    )
                };
                let server = session(server);
                tokio::spawn(async move {
                    let connection = ServerConnection::handshake(server, &Config::default())
                        .await
                        .unwrap();
                    drop(connection);
                });
                Ok(session(client))
            }
        },
    );
    assert!(
        pool.open_lane("close.test:80", LaneKind::Tcp)
            .await
            .is_err()
    );
    let times = times.lock().unwrap();
    assert_eq!(times.len(), 3);
    assert!(times[1].duration_since(times[0]) >= Duration::from_millis(250));
    assert!(times[2].duration_since(times[1]) >= Duration::from_millis(500));
    assert_eq!(pool.stats().pending_lanes, 0);
}

#[tokio::test(start_paused = true)]
async fn peer_stream_limit_can_require_another_link() {
    let pool = pool(Config {
        max_links: 2,
        ..Config::default()
    });
    let session = session_pair(Config {
        max_concurrent_streams: 1,
        ..Config::default()
    });
    let link = Arc::new(
        ClientLink::connect(session, pool.config(), 1, pool.inner.wake.clone())
            .await
            .unwrap(),
    );
    pool.inner.state().links.push(link.clone());
    pool.inner.state().next_id = 2;
    let mut first = pool
        .open_lane("first.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    assert!(!link.has_capacity());
    let second = pool
        .open_lane("second.test:80", LaneKind::Tcp)
        .await
        .unwrap();
    assert_ne!(first.link_id(), second.link_id());
    first.write_all(b"same link").await.unwrap();
    let mut response = [0; 9];
    first.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"same link");
    assert_eq!(first.link_id(), link.id());
    assert_eq!(pool.stats().links, 2);
    assert_eq!(pool.stats().dial_attempts, 1);
}

struct DialGuard(Arc<AtomicUsize>);

impl Drop for DialGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[tokio::test(start_paused = true)]
async fn concurrent_opens_obey_cap_and_single_dial() {
    let config = Config {
        max_links: 3,
        ..Config::default()
    };
    let active = Arc::new(AtomicUsize::new(0));
    let count = active.clone();
    let server_config = Config {
        max_concurrent_streams: 2,
        ..config.clone()
    };
    let pool = Arc::new(LinkPool::new(config, move || {
        let count = count.clone();
        let config = server_config.clone();
        async move {
            assert_eq!(count.fetch_add(1, Ordering::AcqRel), 0);
            let _guard = DialGuard(count);
            tokio::task::yield_now().await;
            Ok(session_pair(config))
        }
    }));
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            pool.open_lane("echo.test:80", LaneKind::Tcp).await.unwrap()
        }));
    }
    let mut lanes = Vec::new();
    for task in tasks {
        lanes.push(task.await.unwrap());
    }
    assert_eq!(pool.stats().links, 3);
    assert_eq!(pool.stats().dial_attempts, 3);
    assert_eq!(pool.stats().lanes, 6);
    assert_eq!(active.load(Ordering::Acquire), 0);
    let extra = {
        let pool = pool.clone();
        tokio::spawn(async move { pool.open_lane("extra.test:80", LaneKind::Tcp).await })
    };
    settle_until(|| pool.stats().waiting_opens == 1).await;
    assert_eq!(pool.stats().links, 3);
    drop(lanes.pop());
    let lane = extra.await.unwrap().unwrap();
    assert!(pool.stats().link_ids.contains(&lane.link_id()));
    assert_eq!(pool.stats().dial_attempts, 3);
}

#[tokio::test(start_paused = true)]
async fn caller_cancellation_does_not_restart_an_owned_dial() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let dial_started = started.clone();
    let dial_release = release.clone();
    let pool = Arc::new(LinkPool::new(Config::default(), move || {
        let started = dial_started.clone();
        let release = dial_release.clone();
        async move {
            started.notify_one();
            release.notified().await;
            Ok(session_pair(Config::default()))
        }
    }));
    let first = {
        let pool = pool.clone();
        tokio::spawn(async move { pool.open_lane("first.test:80", LaneKind::Tcp).await })
    };
    started.notified().await;
    first.abort();
    assert!(matches!(first.await, Err(error) if error.is_cancelled()));
    assert_eq!(pool.stats().waiting_opens, 0);
    let second = {
        let pool = pool.clone();
        tokio::spawn(async move { pool.open_lane("second.test:80", LaneKind::Tcp).await })
    };
    settle_until(|| pool.stats().waiting_opens == 1).await;
    assert_eq!(pool.stats().dial_attempts, 1);
    release.notify_one();
    let lane = second.await.unwrap().unwrap();
    assert_eq!(pool.stats().links, 1);
    assert_eq!(pool.stats().dial_attempts, 1);
    assert_eq!(lane.link_id(), 1);
}

#[tokio::test(start_paused = true)]
async fn close_wakes_blocked_opens_and_cancels_dial() {
    let active = Arc::new(AtomicUsize::new(0));
    let count = active.clone();
    let pool: Arc<LinkPool<DuplexStream>> = Arc::new(LinkPool::new(Config::default(), move || {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::AcqRel);
            let _guard = DialGuard(count);
            pending().await
        }
    }));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            pool.open_lane("wait.test:80", LaneKind::Tcp).await
        }));
    }
    settle_until(|| active.load(Ordering::Acquire) == 1).await;
    pool.close();
    for task in tasks {
        assert!(matches!(task.await.unwrap(), Err(Error::Closed)));
    }
    settle_until(|| active.load(Ordering::Acquire) == 0).await;
    assert_eq!(pool.stats().waiting_opens, 0);
    assert!(!pool.stats().dialing);
}

use std::task::Poll;

#[tokio::test(start_paused = true)]
async fn spread_request_dials_a_second_link() {
    let pool = pool(Config::default());
    let link = add_link(&pool).await;
    let _held = link.reserve();
    pool.inner.state().spread_request = true;
    let dial = pool
        .inner
        .maintain()
        .expect("a spread request opens another link");
    pool.inner.dial_finished(dial.await);
    assert_eq!(pool.stats().links, 2);
    assert_eq!(pool.stats().dial_attempts, 1);
    assert!(!pool.inner.state().spread_request);
}

#[tokio::test(start_paused = true)]
async fn a_loaded_single_link_spreads_a_new_lane() {
    let pool = pool(Config::default());
    let link = add_link(&pool).await;
    let _a = link.reserve();
    let _b = link.reserve();
    let inner = pool.inner.clone();
    let task = tokio::spawn(async move {
        inner
            .acquire(Instant::now() + Duration::from_secs(5))
            .await
            .map(|reservation| reservation.link.id())
    });
    settle_until(|| pool.inner.state().spread_request).await;
    let dial = pool.inner.maintain().expect("spread dial");
    pool.inner.dial_finished(dial.await);
    let id = task.await.unwrap().unwrap();
    assert_ne!(id, link.id());
    assert_eq!(pool.stats().links, 2);
    assert_eq!(pool.stats().dial_attempts, 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_dial_without_links_is_offline_until_a_dial_succeeds() {
    let pool = pool(Config::default());
    assert!(!pool.is_offline());
    pool.inner.dial_finished(Err(Error::Timeout));
    assert!(pool.is_offline());
    let link = add_link(&pool).await;
    assert!(!pool.is_offline(), "a live link is not offline");
    link.close();
    pool.inner.state().links.clear();
    assert!(pool.is_offline());
    let session = session_pair(pool.config().clone());
    let link = ClientLink::connect(session, pool.config(), 99, pool.inner.wake.clone())
        .await
        .unwrap();
    pool.inner.dial_finished(Ok(link));
    assert!(!pool.is_offline());
}
