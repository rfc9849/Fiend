use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

use super::error::{Error, Result};
use super::freshness::FreshnessCheck;
use super::originprofile::OriginProfile;
use super::passthrough::bridge_passthrough;
use super::serverhello::{
    REC_APPDATA, REC_HANDSHAKE, TLS_VERSION, build_server_hello, choose_cipher_suite,
    choose_server_share, client_shared_secret, finished_record_wire_size, generate_server_share,
    parse_server_hello,
};
use super::session::Session;
use crate::crypto;
use crate::fingerprint::{BuiltClientHello, DEFAULT_RECORD_MAX, Template, parse_client_hello};

pub const HANDSHAKE_VERSION: u8 = 4;
pub const AUTH_WINDOW: Duration = Duration::from_secs(3 * 3600);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PRE_AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_FINISHED_TIMEOUT: Duration = Duration::from_secs(60);
const ALERT_WIRE_BODY: usize = 2 + 1 + 16;
const MAX_HANDSHAKE_RECORD: usize = DEFAULT_RECORD_MAX + 16 + 256;
const MAX_RECORD_WIRE: usize = 5 + DEFAULT_RECORD_MAX + 16;
const EARLY_DATA_MIN: usize = 200;
const EARLY_DATA_MAX: usize = 1400;

pub enum AcceptOutcome<S> {
    Session(Box<Session<S>>),
    Passthrough { stream: S, buffered: Vec<u8> },
}

pub async fn connect<S>(mut stream: S, secret: &[u8], template: &Template) -> Result<Session<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let BuiltClientHello { record, key_shares } = template.build(secret)?;
    let tls_random: [u8; 32] = record[template.tls_random.offset..template.tls_random.offset + 32]
        .try_into()
        .map_err(|_| Error::invalid("invalid TLS random offset"))?;
    let session_id: [u8; 32] = record[template.session_id.offset..template.session_id.offset + 32]
        .try_into()
        .map_err(|_| Error::invalid("invalid session_id offset"))?;

    let mut hello = record;
    if template.early_data {
        let body_len = random_range(EARLY_DATA_MIN, EARLY_DATA_MAX + 1);
        hello.extend_from_slice(&[REC_APPDATA, TLS_VERSION[0], TLS_VERSION[1]]);
        hello.extend_from_slice(&(body_len as u16).to_be_bytes());
        let start = hello.len();
        hello.resize(start + body_len, 0);
        rand::fill(&mut hello[start..]);
    }
    write_all_timeout(&mut stream, &hello).await?;

    let record = read_record_timeout(&mut stream).await?;
    if record[0] != REC_HANDSHAKE {
        return Err(Error::invalid("server did not send a ServerHello"));
    }
    let parsed = parse_server_hello(&record)?;
    if !crypto::server_auth_random_matches(secret, &tls_random, &parsed.random) {
        return Err(Error::invalid("server authentication failed"));
    }
    if parsed.session_id != session_id {
        return Err(Error::invalid("ServerHello session_id echo mismatch"));
    }
    if !template.cipher_suites.contains(&parsed.cipher_suite) {
        return Err(Error::invalid(
            "ServerHello selected an unoffered cipher suite",
        ));
    }
    let selected = key_shares
        .shares
        .into_iter()
        .find(|share| share.group == parsed.group)
        .ok_or_else(|| Error::invalid("ServerHello selected an unoffered key_share group"))?;
    let shared = Zeroizing::new(client_shared_secret(selected, &parsed.key_share)?);
    let pair = crypto::derive_session_keys(secret, &tls_random, &parsed.random, &shared)?;

    read_change_cipher_spec(&mut stream).await?;

    let mut found = false;
    for _ in 0..64 {
        let (record_type, body) = read_record_parts(&mut stream).await?;
        if record_type != REC_APPDATA {
            return Err(Error::invalid("invalid server handshake flight record"));
        }
        if body.len() >= 32
            && crypto::handshake_terminator_matches(
                secret,
                &tls_random,
                &parsed.random,
                HANDSHAKE_VERSION,
                &body[..32],
            )
        {
            if body.len() != finished_record_wire_size(parsed.cipher_suite) - 5 {
                return Err(Error::invalid("invalid server Finished size"));
            }
            found = true;
            break;
        }
    }
    if !found {
        return Err(Error::invalid("handshake terminator not found"));
    }

    write_client_finished(
        &mut stream,
        secret,
        &tls_random,
        &parsed.random,
        parsed.cipher_suite,
    )
    .await?;

    let (write_cipher, read_cipher) = pair.for_client();
    Ok(Session::new(
        stream,
        write_cipher,
        read_cipher,
        template.record_max,
    ))
}

pub async fn accept<S>(
    mut stream: S,
    secret: &[u8],
    replay: &FreshnessCheck,
    origin: &OriginProfile,
) -> Result<AcceptOutcome<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let deadline = Instant::now() + PRE_AUTH_TIMEOUT;
    let mut buffered = Vec::with_capacity(2048);
    if !read_prefix(&mut stream, &mut buffered, 1, deadline).await? || buffered[0] != REC_HANDSHAKE
    {
        return passthrough(stream, buffered);
    }
    if !read_prefix(&mut stream, &mut buffered, 5, deadline).await? {
        return passthrough(stream, buffered);
    }
    let length = u16::from_be_bytes([buffered[3], buffered[4]]) as usize;
    if !(4..=MAX_HANDSHAKE_RECORD).contains(&length) {
        return passthrough(stream, buffered);
    }
    if !read_prefix(&mut stream, &mut buffered, 5 + length, deadline).await? {
        return passthrough(stream, buffered);
    }

    let body = &buffered[5..];
    if !verify_auth_token(body, secret, unix_time()) {
        return passthrough(stream, buffered);
    }
    let tls_random: [u8; 32] = body[6..38].try_into().expect("32-byte slice");
    let session_id: [u8; 32] = body[39..71].try_into().expect("32-byte slice");
    if !replay.check(&tls_random) {
        return passthrough(stream, buffered);
    }

    let template = match parse_client_hello(&buffered) {
        Ok(template) => template,
        Err(_) => return passthrough(stream, buffered),
    };

    if template.early_data {
        let deadline = Instant::now() + PRE_AUTH_TIMEOUT;
        let start = buffered.len();
        if !read_prefix(&mut stream, &mut buffered, start + 5, deadline).await? {
            return passthrough(stream, buffered);
        }
        let early_header = &buffered[start..start + 5];
        let early_len = u16::from_be_bytes([early_header[3], early_header[4]]) as usize;
        if early_header[0] != REC_APPDATA
            || early_header[1..3] != TLS_VERSION
            || early_len == 0
            || early_len > MAX_HANDSHAKE_RECORD
        {
            return passthrough(stream, buffered);
        }
        if !read_prefix(&mut stream, &mut buffered, start + 5 + early_len, deadline).await? {
            return passthrough(stream, buffered);
        }
    }

    let chosen = match choose_server_share(&template, origin.key_share_group) {
        Ok(share) => share,
        Err(_) => return passthrough(stream, buffered),
    };
    let server_share = match generate_server_share(&chosen) {
        Ok(share) => share,
        Err(_) => return passthrough(stream, buffered),
    };
    let cipher_suite = choose_cipher_suite(&template, origin.cipher_suite);
    let mut nonce = [0u8; crypto::SERVER_NONCE_SIZE];
    rand::fill(&mut nonce);
    let server_random = crypto::server_auth_random(secret, &tls_random, &nonce);

    write_all_timeout(
        &mut stream,
        &build_server_flight(ServerFlight {
            server_random,
            session_id: &session_id,
            cipher_suite,
            group: server_share.group,
            key: &server_share.public,
            secret,
            client_random: &tls_random,
            profile_sizes: &origin.record_sizes,
            extension_order: &origin.extension_order,
        })?,
    )
    .await?;

    let finished = tokio::time::timeout(
        CLIENT_FINISHED_TIMEOUT,
        read_client_finished(
            &mut stream,
            secret,
            &tls_random,
            &server_random,
            cipher_suite,
        ),
    )
    .await
    .map_err(|_| Error::Timeout)?;
    if let Err(error) = finished {
        if matches!(error, Error::Invalid(_)) {
            write_fake_alert(&mut stream).await;
        }
        return Err(error);
    }

    let shared = Zeroizing::new(server_share.shared);
    let pair = crypto::derive_session_keys(secret, &tls_random, &server_random, &shared)?;
    let (write_cipher, read_cipher) = pair.for_server();
    Ok(AcceptOutcome::Session(Box::new(Session::new(
        stream,
        write_cipher,
        read_cipher,
        template.record_max,
    ))))
}

pub async fn serve_connection<S>(
    stream: S,
    secret: &[u8],
    replay: &FreshnessCheck,
    profile: &OriginProfile,
    origin_host: &str,
    origin_port: u16,
) -> Result<Option<Session<S>>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match accept(stream, secret, replay, profile).await? {
        AcceptOutcome::Session(session) => Ok(Some(*session)),
        AcceptOutcome::Passthrough { stream, buffered } => {
            bridge_passthrough(stream, buffered, origin_host, origin_port).await?;
            Ok(None)
        }
    }
}

fn passthrough<S>(stream: S, buffered: Vec<u8>) -> Result<AcceptOutcome<S>> {
    Ok(AcceptOutcome::Passthrough { stream, buffered })
}

pub struct ServerFlight<'a> {
    pub server_random: [u8; 32],
    pub session_id: &'a [u8],
    pub cipher_suite: u16,
    pub group: u16,
    pub key: &'a [u8],
    pub secret: &'a [u8],
    pub client_random: &'a [u8],
    pub profile_sizes: &'a [usize],
    pub extension_order: &'a [u16],
}

pub fn build_server_flight(flight: ServerFlight<'_>) -> Result<Vec<u8>> {
    let ServerFlight {
        server_random,
        session_id,
        cipher_suite,
        group,
        key,
        secret,
        client_random,
        profile_sizes,
        extension_order,
    } = flight;
    let mut flight = build_server_hello(
        server_random,
        session_id,
        cipher_suite,
        group,
        key,
        extension_order,
    )?;
    flight.extend_from_slice(&[0x14, 0x03, 0x03, 0, 1, 1]);

    let mut sizes = profile_sizes.to_vec();
    if sizes.is_empty() {
        sizes = default_flight_sizes(cipher_suite);
    }
    let last = sizes.len() - 1;
    sizes[last] = finished_record_wire_size(cipher_suite);

    let terminator =
        crypto::handshake_terminator(secret, client_random, &server_random, HANDSHAKE_VERSION);
    for (index, wire_size) in sizes.iter().enumerate() {
        let wire_size = (*wire_size).clamp(5, MAX_RECORD_WIRE);
        let body_len = wire_size - 5;
        flight.extend_from_slice(&[REC_APPDATA, TLS_VERSION[0], TLS_VERSION[1]]);
        flight.extend_from_slice(&(body_len as u16).to_be_bytes());
        let start = flight.len();
        flight.resize(start + body_len, 0);
        if index == last {
            flight[start..start + 32].copy_from_slice(&terminator);
            if body_len > 32 {
                rand::fill(&mut flight[start + 32..]);
            }
        } else {
            rand::fill(&mut flight[start..]);
        }
    }
    Ok(flight)
}

pub fn default_flight_sizes(cipher_suite: u16) -> Vec<usize> {
    let certificate_verify = if cipher_suite == 0x1303 { 97 } else { 286 };
    vec![
        35,
        3400,
        certificate_verify,
        finished_record_wire_size(cipher_suite),
    ]
}

async fn read_client_finished<S>(
    stream: &mut S,
    secret: &[u8],
    client_random: &[u8],
    server_random: &[u8; 32],
    cipher_suite: u16,
) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0u8; 5];
    stream.read_exact(&mut header).await?;
    if header != [0x14, 0x03, 0x03, 0, 1] {
        return Err(Error::invalid("invalid ChangeCipherSpec"));
    }
    let mut value = [0u8; 1];
    stream.read_exact(&mut value).await?;
    if value[0] != 1 {
        return Err(Error::invalid("invalid ChangeCipherSpec value"));
    }

    stream.read_exact(&mut header).await?;
    let record_type = header[0];
    let length = u16::from_be_bytes([header[3], header[4]]) as usize;
    if length > MAX_HANDSHAKE_RECORD {
        return Err(Error::invalid("TLS record too large"));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await?;
    if record_type != REC_APPDATA {
        return Err(Error::invalid("invalid client Finished record"));
    }
    if body.len() != finished_record_wire_size(cipher_suite) - 5 {
        return Err(Error::invalid("invalid client Finished size"));
    }
    if !crypto::client_finished_matches(secret, client_random, server_random, &body[..32]) {
        return Err(Error::invalid("client Finished authentication failed"));
    }
    Ok(())
}

async fn write_fake_alert<S>(stream: &mut S)
where
    S: AsyncWrite + Unpin,
{
    let mut alert = [0u8; 5 + ALERT_WIRE_BODY];
    alert[..5].copy_from_slice(&[
        REC_APPDATA,
        TLS_VERSION[0],
        TLS_VERSION[1],
        0,
        ALERT_WIRE_BODY as u8,
    ]);
    rand::fill(&mut alert[5..]);
    let _ = write_all_timeout(stream, &alert).await;
    let _ = tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.shutdown()).await;
}

async fn write_client_finished<S>(
    stream: &mut S,
    secret: &[u8],
    client_random: &[u8],
    server_random: &[u8; 32],
    cipher_suite: u16,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let wire_size = finished_record_wire_size(cipher_suite);
    let body_len = wire_size - 5;
    let mut flight = Vec::with_capacity(6 + wire_size);
    flight.extend_from_slice(&[0x14, 0x03, 0x03, 0, 1, 1]);
    flight.extend_from_slice(&[REC_APPDATA, TLS_VERSION[0], TLS_VERSION[1]]);
    flight.extend_from_slice(&(body_len as u16).to_be_bytes());
    let start = flight.len();
    flight.resize(start + body_len, 0);
    flight[start..start + 32].copy_from_slice(&crypto::client_finished(
        secret,
        client_random,
        server_random,
    ));
    rand::fill(&mut flight[start + 32..]);
    write_all_timeout(stream, &flight).await
}

async fn read_change_cipher_spec<S>(stream: &mut S) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0u8; 5];
    read_exact_timeout(stream, &mut header).await?;
    if header != [0x14, 0x03, 0x03, 0, 1] {
        return Err(Error::invalid("invalid ChangeCipherSpec"));
    }
    let mut value = [0u8; 1];
    read_exact_timeout(stream, &mut value).await?;
    if value[0] != 1 {
        return Err(Error::invalid("invalid ChangeCipherSpec value"));
    }
    Ok(())
}

async fn read_record_timeout<S>(stream: &mut S) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let (record_type, body) = read_record_parts(stream).await?;
    let mut record = Vec::with_capacity(5 + body.len());
    record.extend_from_slice(&[record_type, TLS_VERSION[0], TLS_VERSION[1]]);
    record.extend_from_slice(&(body.len() as u16).to_be_bytes());
    record.extend_from_slice(&body);
    Ok(record)
}

async fn read_record_parts<S>(stream: &mut S) -> Result<(u8, Vec<u8>)>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0u8; 5];
    read_exact_timeout(stream, &mut header).await?;
    let length = u16::from_be_bytes([header[3], header[4]]) as usize;
    if length > MAX_HANDSHAKE_RECORD {
        return Err(Error::invalid("TLS record too large"));
    }
    let mut body = vec![0u8; length];
    read_exact_timeout(stream, &mut body).await?;
    Ok((header[0], body))
}

async fn read_prefix<S>(
    stream: &mut S,
    buffered: &mut Vec<u8>,
    wanted: usize,
    deadline: Instant,
) -> Result<bool>
where
    S: AsyncRead + Unpin,
{
    while buffered.len() < wanted {
        let start = buffered.len();
        buffered.resize(wanted, 0);
        let read = tokio::time::timeout_at(deadline, stream.read(&mut buffered[start..])).await;
        let count = match read {
            Ok(Ok(count)) => count,
            Ok(Err(error)) => {
                buffered.truncate(start);
                return Err(error.into());
            }
            Err(_) => 0,
        };
        buffered.truncate(start + count);
        if count == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn read_exact_timeout<S>(stream: &mut S, buffer: &mut [u8]) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(buffer))
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(())
}

async fn write_all_timeout<S>(stream: &mut S, buffer: &[u8]) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.write_all(buffer))
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(())
}

fn verify_auth_token(body: &[u8], secret: &[u8], unix_time: i64) -> bool {
    if body.len() < 71 || body[0] != 0x01 || body[38] != 32 {
        return false;
    }
    let random = &body[6..38];
    let token = &body[39..71];
    let hour = unix_time / 3600;
    [hour - 1, hour, hour + 1]
        .into_iter()
        .any(|bucket| crypto::auth_token_matches(secret, random, bucket, token))
}

fn unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn random_range(lo: usize, hi: usize) -> usize {
    if hi <= lo {
        return lo;
    }
    let mut bytes = [0u8; 4];
    rand::fill(&mut bytes);
    lo + (u32::from_be_bytes(bytes) as usize % (hi - lo))
}
