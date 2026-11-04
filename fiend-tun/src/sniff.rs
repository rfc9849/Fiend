use std::net::IpAddr;

use fiend_core::fingerprint::server_name_str;

use crate::dns::valid_domain;

const HANDSHAKE: u8 = 0x16;
const MAX_HEAD: usize = 8192;
const METHODS: [&str; 9] = [
    "GET ", "POST ", "HEAD ", "PUT ", "DELETE ", "OPTIONS ", "PATCH ", "CONNECT ", "TRACE ",
];

/// Finds the site name in the first client data: the TLS server name or the
/// HTTP Host header. An IP address literal gives no name.
pub fn host(data: &[u8]) -> Option<String> {
    let name = if data.first() == Some(&HANDSHAKE) {
        server_name_str(data)?
    } else {
        http_host(data)?
    };
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    (name.parse::<IpAddr>().is_err() && valid_domain(&name)).then_some(name)
}

fn http_host(data: &[u8]) -> Option<String> {
    let head = &data[..data.len().min(MAX_HEAD)];
    let end = head
        .windows(2)
        .position(|pair| pair == b"\r\n")
        .unwrap_or(head.len());
    let request = std::str::from_utf8(&head[..end]).ok()?;
    if !METHODS.iter().any(|method| request.starts_with(method)) || !request.contains(" HTTP/1.") {
        return None;
    }
    let text = String::from_utf8_lossy(head);
    let value = text.split("\r\n").skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("host")
            .then(|| value.trim())
    })?;
    let host = if let Some(rest) = value.strip_prefix('[') {
        rest.split_once(']')?.0
    } else {
        value.rsplit_once(':').map_or(value, |(host, port)| {
            if port.bytes().all(|byte| byte.is_ascii_digit()) {
                host
            } else {
                value
            }
        })
    };
    Some(host.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_hello(host: &str) -> Vec<u8> {
        let name = host.as_bytes();
        let mut server_name = vec![0, 0];
        server_name.extend_from_slice(&((name.len() + 5) as u16).to_be_bytes());
        server_name.extend_from_slice(&((name.len() + 3) as u16).to_be_bytes());
        server_name.push(0);
        server_name.extend_from_slice(&(name.len() as u16).to_be_bytes());
        server_name.extend_from_slice(name);
        let mut body = vec![3, 3];
        body.extend_from_slice(&[7; 32]);
        body.push(0);
        body.extend_from_slice(&[0, 2, 0x13, 0x01, 1, 0]);
        body.extend_from_slice(&(server_name.len() as u16).to_be_bytes());
        body.extend_from_slice(&server_name);
        let mut handshake = vec![1];
        handshake.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        handshake.extend_from_slice(&body);
        let mut record = vec![HANDSHAKE, 3, 1];
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }

    #[test]
    fn tls_server_name_is_found() {
        assert_eq!(
            host(&client_hello("Video.Example.COM")).as_deref(),
            Some("video.example.com")
        );
        assert_eq!(host(&client_hello("192.0.2.1")), None);
        assert_eq!(host(&client_hello("bad name")), None);
        assert_eq!(host(&client_hello("example.com")[..40]), None);
    }

    #[test]
    fn http_host_is_found() {
        assert_eq!(
            host(b"GET / HTTP/1.1\r\nUser-Agent: x\r\nHOST: Example.org:8080\r\n\r\n").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            host(b"POST /a HTTP/1.0\r\nHost: example.net\r\n\r\n").as_deref(),
            Some("example.net")
        );
        assert_eq!(
            host(b"GET / HTTP/1.1\r\nHost: [2001:db8::1]:80\r\n\r\n"),
            None
        );
        assert_eq!(host(b"GET / HTTP/1.1\r\nHost: 192.0.2.1\r\n\r\n"), None);
        assert_eq!(host(b"SSH-2.0-OpenSSH_9.9\r\n"), None);
        assert_eq!(host(b"GET / HTTP/1.1\r\nAccept: */*\r\n\r\n"), None);
        assert_eq!(host(b""), None);
    }
}
