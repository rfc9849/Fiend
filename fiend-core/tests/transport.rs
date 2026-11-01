use std::time::Duration;

use fiend_core::fingerprint::{
    DEFAULT_RECORD_MAX, FieldOffset, KeyShareSpec, P256, P256_SIZE, Template, X25519,
    X25519_MLKEM768, X25519_MLKEM768_SIZE, X25519_SIZE, parse_client_hello,
};
use fiend_core::transport::{
    AcceptOutcome, FreshnessCheck, OriginProfile, PassthroughReason, ServerFlight, Session, accept,
    bridge_passthrough, build_server_flight, connect, parse_server_hello, probe_origin_at,
    server_hello_extension_order,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");

fn sample() -> Template {
    Template::from_json(TEMPLATE_JSON).expect("load sample template")
}

fn synthetic_template(group: u16) -> Template {
    let size = match group {
        X25519 => X25519_SIZE,
        P256 => P256_SIZE,
        X25519_MLKEM768 => X25519_MLKEM768_SIZE,
        _ => panic!("unsupported test group"),
    };
    let cipher_suites = [0x1301u16, 0x1302, 0x1303];
    let mut raw = vec![0u8; 5];
    raw[..3].copy_from_slice(&[0x16, 0x03, 0x01]);
    raw.extend_from_slice(&[0x01, 0, 0, 0]);
    raw.extend_from_slice(&[0x03, 0x03]);

    let tls_random = FieldOffset {
        offset: raw.len(),
        size: 32,
    };
    raw.extend_from_slice(&[0u8; 32]);
    raw.push(32);
    let session_id = FieldOffset {
        offset: raw.len(),
        size: 32,
    };
    raw.extend_from_slice(&[0u8; 32]);

    raw.extend_from_slice(&((cipher_suites.len() * 2) as u16).to_be_bytes());
    for suite in cipher_suites {
        raw.extend_from_slice(&suite.to_be_bytes());
    }
    raw.extend_from_slice(&[1, 0]);

    let extensions_length = raw.len();
    raw.extend_from_slice(&[0, 0]);
    let extensions_start = raw.len();

    let domain = b"example.com";
    raw.extend_from_slice(&0u16.to_be_bytes());
    raw.extend_from_slice(&((domain.len() + 5) as u16).to_be_bytes());
    raw.extend_from_slice(&((domain.len() + 3) as u16).to_be_bytes());
    raw.push(0);
    raw.extend_from_slice(&(domain.len() as u16).to_be_bytes());
    raw.extend_from_slice(domain);

    raw.extend_from_slice(&[0x00, 0x2b, 0x00, 0x03, 0x02, 0x03, 0x04]);

    raw.extend_from_slice(&0x0033u16.to_be_bytes());
    raw.extend_from_slice(&((2 + 4 + size) as u16).to_be_bytes());
    raw.extend_from_slice(&((4 + size) as u16).to_be_bytes());
    raw.extend_from_slice(&group.to_be_bytes());
    raw.extend_from_slice(&(size as u16).to_be_bytes());
    let key_share = FieldOffset {
        offset: raw.len(),
        size,
    };
    raw.resize(raw.len() + size, 0);

    let extensions_len = (raw.len() - extensions_start) as u16;
    raw[extensions_length..extensions_length + 2].copy_from_slice(&extensions_len.to_be_bytes());
    let handshake_len = raw.len() - 9;
    raw[6..9].copy_from_slice(&(handshake_len as u32).to_be_bytes()[1..]);
    let record_len = raw.len() - 5;
    raw[3..5].copy_from_slice(&(record_len as u16).to_be_bytes());

    let template = Template {
        domain: "example.com".to_string(),
        raw,
        record_max: DEFAULT_RECORD_MAX,
        tls_random,
        session_id,
        key_share,
        psk_identity: None,
        ticket_age: None,
        psk_binder: None,
        grease: Vec::new(),
        key_shares: vec![KeyShareSpec {
            group,
            offset: key_share.offset,
            size,
            public: vec![0u8; size],
        }],
        cipher_suites: cipher_suites.to_vec(),
        early_data: false,
        ech: None,
        extensions_offset: extensions_length,
    };
    template
        .verify(&template.raw)
        .expect("verify synthetic template");
    template
}

async fn connect_pair(template: &Template) -> (Session<DuplexStream>, Session<DuplexStream>) {
    let secret = [0x42u8; 32];
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();
    let (accepted, connected) = tokio::join!(
        accept(server_io, &secret, &replay, &profile),
        connect(client_io, &secret, template)
    );
    let server = match accepted {
        Ok(AcceptOutcome::Session(session)) => *session,
        Ok(AcceptOutcome::Passthrough { .. }) => panic!(
            "authenticated connection was passed through; client error: {:?}",
            connected.as_ref().err()
        ),
        Err(error) => panic!(
            "accept handshake: {error}; client error: {:?}",
            connected.as_ref().err()
        ),
    };
    let client = connected.expect("client handshake");
    (client, server)
}

#[test]
fn default_flight_sizes_are_stable_across_connections() {
    let build = || {
        build_server_flight(ServerFlight {
            server_random: [1u8; 32],
            session_id: &[2u8; 32],
            cipher_suite: 0x1301,
            group: X25519,
            key: &[3u8; 32],
            secret: &[4u8; 32],
            client_random: &[5u8; 32],
            profile_sizes: &[],
            extension_order: &[],
        })
        .expect("build flight")
    };
    let first = build();
    let second = build();
    let sizes = |flight: &[u8]| {
        let mut sizes = Vec::new();
        let mut position = 0;
        while position + 5 <= flight.len() {
            let length = u16::from_be_bytes([flight[position + 3], flight[position + 4]]) as usize;
            sizes.push(length);
            position += 5 + length;
        }
        assert_eq!(position, flight.len());
        sizes
    };
    assert_eq!(first.len(), second.len());
    assert_eq!(sizes(&first), sizes(&second));
    assert_eq!(sizes(&first).len(), 6);
}

#[test]
fn server_hello_round_trips_with_extension_order() {
    let record = fiend_core::transport::build_server_hello(
        [1u8; 32],
        &[2u8; 32],
        0x1301,
        X25519,
        &[3u8; 32],
        &[0x0033, 0x002b],
    )
    .expect("build ServerHello");
    let parsed = parse_server_hello(&record).expect("parse ServerHello");
    assert_eq!(parsed.random, [1u8; 32]);
    assert_eq!(parsed.session_id, vec![2u8; 32]);
    assert_eq!(parsed.cipher_suite, 0x1301);
    assert_eq!(parsed.group, X25519);
    assert_eq!(parsed.key_share, vec![3u8; 32]);
    assert_eq!(
        server_hello_extension_order(&record),
        Some(vec![0x0033, 0x002b])
    );

    let mut truncated = record.clone();
    truncated[3] ^= 1;
    assert!(parse_server_hello(&truncated).is_err());
}

#[test]
fn synthetic_client_hello_parses() {
    let template = synthetic_template(X25519);
    let built = template.build(&[0x42u8; 32]).expect("build");
    let parsed = parse_client_hello(&built.record).expect("parse synthetic");
    assert_eq!(parsed.cipher_suites, vec![0x1301, 0x1302, 0x1303]);
    assert_eq!(parsed.key_shares.len(), 1);
}

#[tokio::test]
async fn handshake_succeeds_for_all_key_share_groups() {
    for group in [X25519, P256, X25519_MLKEM768] {
        eprintln!("testing key share group 0x{group:04x}");
        let template = synthetic_template(group);
        let (mut client, mut server) = connect_pair(&template).await;

        client.write_all(b"ping").await.expect("client write");
        let mut request = [0u8; 4];
        server.read_exact(&mut request).await.expect("server read");
        assert_eq!(&request, b"ping");

        server.write_all(b"pong").await.expect("server write");
        let mut response = [0u8; 4];
        client.read_exact(&mut response).await.expect("client read");
        assert_eq!(&response, b"pong");
    }
}

#[tokio::test]
async fn sample_template_handshake_carries_large_records() {
    let template = sample();
    let (mut client, mut server) = connect_pair(&template).await;
    let payload: Vec<u8> = (0..100_000).map(|index| (index % 251) as u8).collect();

    let sender = async {
        client.write_all(&payload).await.expect("write payload");
        client.shutdown().await.expect("shutdown client");
    };
    let receiver = async {
        let mut received = vec![0u8; payload.len()];
        server
            .read_exact(&mut received)
            .await
            .expect("read payload");
        assert_eq!(received, payload);
        let mut eof = [0u8; 1];
        assert_eq!(server.read(&mut eof).await.expect("read EOF"), 0);
    };
    tokio::join!(sender, receiver);
}

#[tokio::test]
async fn invalid_auth_is_passed_through_unchanged() {
    let template = sample();
    let built = template.build(&[0x11u8; 32]).expect("build hello");
    let secret = [0x42u8; 32];
    let (mut client_io, server_io) = tokio::io::duplex(64 * 1024);
    client_io
        .write_all(&built.record)
        .await
        .expect("write hello");

    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let outcome = accept(server_io, &secret, &replay, &OriginProfile::default())
        .await
        .expect("accept");
    match outcome {
        AcceptOutcome::Passthrough { buffered, .. } => {
            assert_eq!(buffered, built.record);
        }
        AcceptOutcome::Session(_) => panic!("invalid authentication was accepted"),
    }
}

#[tokio::test]
async fn replayed_client_hello_is_passed_through() {
    let template = sample();
    let secret = [0x42u8; 32];
    let hello = template.build(&secret).expect("build hello").record;
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();

    let (mut first_client, first_server) = tokio::io::duplex(64 * 1024);
    first_client
        .write_all(&hello)
        .await
        .expect("write first hello");
    drop(first_client);
    assert!(
        accept(first_server, &secret, &replay, &profile)
            .await
            .is_err()
    );

    let (mut second_client, second_server) = tokio::io::duplex(64 * 1024);
    second_client
        .write_all(&hello)
        .await
        .expect("write replayed hello");
    let outcome = accept(second_server, &secret, &replay, &profile)
        .await
        .expect("accept replay");
    match outcome {
        AcceptOutcome::Passthrough {
            buffered, reason, ..
        } => {
            assert_eq!(buffered, hello);
            assert_eq!(reason, PassthroughReason::Replay);
        }
        AcceptOutcome::Session(_) => panic!("replayed ClientHello was accepted"),
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_probes_are_passed_through() {
    let secret = [0x42u8; 32];
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();
    for (probe, expected) in [
        (&[][..], PassthroughReason::NotTls),
        (&[0x16, 0x03][..], PassthroughReason::Truncated),
        (
            &[0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00][..],
            PassthroughReason::Truncated,
        ),
        (&[0x47][..], PassthroughReason::NotTls),
    ] {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        client.write_all(probe).await.expect("write probe");
        let outcome = accept(server, &secret, &replay, &profile)
            .await
            .expect("stalled probe must not fail");
        match outcome {
            AcceptOutcome::Passthrough {
                buffered, reason, ..
            } => {
                assert_eq!(buffered, probe);
                assert_eq!(reason, expected);
            }
            AcceptOutcome::Session(_) => panic!("probe was accepted"),
        }
        drop(client);
    }
}

async fn probe_replay(hello: &[u8], secret: &[u8]) -> ([u8; 32], Vec<u8>) {
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    client.write_all(hello).await.expect("write hello");
    let probe = async {
        let mut header = [0u8; 5];
        client
            .read_exact(&mut header)
            .await
            .expect("ServerHello header");
        let length = u16::from_be_bytes([header[3], header[4]]) as usize;
        let mut record = header.to_vec();
        record.resize(5 + length, 0);
        client
            .read_exact(&mut record[5..])
            .await
            .expect("ServerHello body");
        let random = parse_server_hello(&record).expect("parse").random;
        client
            .write_all(&[0x14, 0x03, 0x03, 0, 1, 1, 0x17, 0x03, 0x03, 0, 2, 0, 0])
            .await
            .expect("write garbage Finished");
        let mut tail = Vec::new();
        client.read_to_end(&mut tail).await.expect("read tail");
        (random, tail)
    };
    let (accepted, (random, tail)) = tokio::join!(accept(server, secret, &replay, &profile), probe);
    assert!(accepted.is_err());
    (random, tail)
}

#[tokio::test]
async fn replay_after_restart_gets_fresh_server_random_and_alert() {
    let template = sample();
    let secret = [0x42u8; 32];
    let hello = template.build(&secret).expect("build hello").record;

    let (first, first_tail) = probe_replay(&hello, &secret).await;
    let (second, second_tail) = probe_replay(&hello, &secret).await;
    assert_ne!(first, second);
    for tail in [first_tail, second_tail] {
        let alert = &tail[tail.len() - 24..];
        assert_eq!(&alert[..5], &[0x17, 0x03, 0x03, 0, 19]);
    }
}

#[tokio::test]
async fn active_probe_is_forwarded_to_origin() {
    const PROBE: &[u8] = b"GET / HTTP/1.1\r\nHost: origin\r\n\r\n";
    const RESPONSE: &[u8] = b"HTTP/1.1 204 No Content\r\n\r\n";

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind origin");
    let port = listener.local_addr().expect("origin address").port();
    let origin = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept origin");
        let mut received = vec![0u8; PROBE.len()];
        stream.read_exact(&mut received).await.expect("origin read");
        assert_eq!(received, PROBE);
        stream.write_all(RESPONSE).await.expect("origin write");
        stream.shutdown().await.expect("origin shutdown");
    });

    let (mut client_io, server_io) = tokio::io::duplex(64 * 1024);
    client_io.write_all(PROBE).await.expect("write probe");
    let secret = [0x42u8; 32];
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();
    let serve = tokio::spawn(async move {
        match accept(server_io, &secret, &replay, &profile).await? {
            AcceptOutcome::Session(session) => Ok(Some(*session)),
            AcceptOutcome::Passthrough {
                stream, buffered, ..
            } => {
                bridge_passthrough(stream, buffered, "127.0.0.1", port).await?;
                Ok::<_, fiend_core::transport::Error>(None)
            }
        }
    });

    let mut response = vec![0u8; RESPONSE.len()];
    client_io
        .read_exact(&mut response)
        .await
        .expect("read response");
    assert_eq!(response, RESPONSE);
    drop(client_io);
    let session = serve.await.expect("serve task").expect("serve probe");
    assert!(session.is_none());
    origin.await.expect("origin task");
}

#[tokio::test]
async fn unauthenticated_client_hello_reaches_origin_unchanged() {
    let template = sample();
    let hello = template.build(&[0x11u8; 32]).expect("build hello").record;
    let origin_reply = fiend_core::transport::build_server_hello(
        [0x55u8; 32],
        &[0x66u8; 32],
        0x1301,
        X25519,
        &[0x77u8; 32],
        &[0x002b, 0x0033],
    )
    .expect("build origin reply");

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind origin");
    let port = listener.local_addr().expect("origin address").port();
    let expected_hello = hello.clone();
    let expected_reply = origin_reply.clone();
    let origin = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept origin");
        let mut received = vec![0u8; expected_hello.len()];
        stream
            .read_exact(&mut received)
            .await
            .expect("origin read hello");
        assert_eq!(received, expected_hello);
        stream
            .write_all(&expected_reply)
            .await
            .expect("origin write reply");
        stream.shutdown().await.expect("origin shutdown");
    });

    let (mut client_io, server_io) = tokio::io::duplex(64 * 1024);
    client_io.write_all(&hello).await.expect("write hello");
    let secret = [0x42u8; 32];
    let replay = FreshnessCheck::new(Duration::from_secs(3600));
    let profile = OriginProfile::default();
    let serve = tokio::spawn(async move {
        match accept(server_io, &secret, &replay, &profile).await? {
            AcceptOutcome::Session(session) => Ok(Some(*session)),
            AcceptOutcome::Passthrough {
                stream, buffered, ..
            } => {
                bridge_passthrough(stream, buffered, "127.0.0.1", port).await?;
                Ok::<_, fiend_core::transport::Error>(None)
            }
        }
    });

    let mut received = vec![0u8; origin_reply.len()];
    client_io
        .read_exact(&mut received)
        .await
        .expect("read origin reply");
    assert_eq!(received, origin_reply);
    drop(client_io);
    assert!(serve.await.expect("serve task").expect("serve").is_none());
    origin.await.expect("origin task");
}

#[tokio::test]
async fn origin_probe_reads_server_shape_metadata() {
    let template = synthetic_template(X25519);
    let hello = template.build(&[0x31u8; 32]).expect("build hello").record;
    let expected = hello.clone();

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind origin");
    let port = listener.local_addr().expect("origin address").port();
    let origin = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept origin");
        let mut received = vec![0u8; expected.len()];
        stream
            .read_exact(&mut received)
            .await
            .expect("origin read hello");
        assert_eq!(received, expected);

        let mut response = fiend_core::transport::build_server_hello(
            [9u8; 32],
            &[3u8; 32],
            0x1301,
            X25519,
            &[7u8; 32],
            &[0x002b, 0x0033],
        )
        .expect("build origin ServerHello");
        for size in [100usize, 200] {
            response.extend_from_slice(&[0x17, 0x03, 0x03]);
            response.extend_from_slice(&(size as u16).to_be_bytes());
            response.resize(response.len() + size, 0x5a);
        }
        stream
            .write_all(&response)
            .await
            .expect("origin write flight");
        stream.shutdown().await.expect("origin shutdown");
    });

    let profile = probe_origin_at("127.0.0.1", port, &hello)
        .await
        .expect("probe origin");
    assert_eq!(profile.cipher_suite, 0x1301);
    assert_eq!(profile.key_share_group, X25519);
    assert_eq!(profile.extension_order, vec![0x002b, 0x0033]);
    assert_eq!(profile.record_sizes, vec![105, 205]);
    origin.await.expect("origin task");
}

#[tokio::test]
async fn session_records_may_coalesce_writes() {
    let template = sample();
    let (mut client, mut server) = connect_pair(&template).await;
    for size in [1usize, 100, 16_384, 20_000] {
        let payload: Vec<u8> = (0..size).map(|index| index as u8).collect();
        client.write_all(&payload).await.expect("client write");
        client.flush().await.expect("client flush");
        let mut read = vec![0u8; size];
        server.read_exact(&mut read).await.expect("server read");
        assert_eq!(read, payload);
    }
    server.write_all(b"back").await.expect("server write");
    let mut read = [0u8; 4];
    client.read_exact(&mut read).await.expect("client read");
    assert_eq!(&read, b"back");
}
