use std::fs;
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fiend_core::target::TargetPolicy;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use crate::{AppError, RuleKind, RuleStats, inspect_rule_file};

pub const MAX_DOWNLOAD_BYTES: usize = 32 << 20;
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_URL: usize = 2048;
const MAX_REDIRECTS: u32 = 5;
const METADATA: [IpAddr; 2] = [
    IpAddr::V4(Ipv4Addr::new(100, 100, 100, 200)),
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hosts {
    Public,
    Any,
}

pub fn check_address(address: SocketAddr) -> io::Result<()> {
    let policy = TargetPolicy {
        allow_private: true,
        allow_cgnat: true,
        ..TargetPolicy::default()
    };
    policy.check(address)?;
    let ip = address.ip().to_canonical();
    if METADATA.contains(&ip) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("target {address} is not allowed"),
        ));
    }
    Ok(())
}

type Check = Arc<dyn Fn(SocketAddr) -> io::Result<()> + Send + Sync>;

struct CheckedResolver {
    inner: DefaultResolver,
    check: Check,
}

impl std::fmt::Debug for CheckedResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CheckedResolver")
    }
}

impl Resolver for CheckedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let resolved = self.inner.resolve(uri, config, timeout)?;
        let mut allowed = self.empty();
        let mut denied = None;
        for address in &resolved {
            match (self.check)(*address) {
                Ok(()) => allowed.push(*address),
                Err(error) => denied = Some(error),
            }
        }
        if allowed.is_empty() {
            return Err(denied.map_or(ureq::Error::HostNotFound, ureq::Error::Io));
        }
        Ok(allowed)
    }
}

pub fn validate_url(url: &str) -> Result<String, AppError> {
    let trimmed = url.trim();
    let invalid = || AppError::rules("The link is not valid");
    if trimmed.is_empty() || trimmed.len() > MAX_URL {
        return Err(invalid());
    }
    let uri: http::Uri = trimmed.parse().map_err(|_| invalid())?;
    match uri.scheme_str() {
        Some("https" | "http") => {}
        _ => return Err(AppError::rules("The link must use HTTP or HTTPS")),
    }
    if uri.host().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    Ok(trimmed.to_owned())
}

fn agent(check: Option<Check>) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(60)))
        .max_redirects(MAX_REDIRECTS)
        .http_status_as_error(false);
    match check {
        None => config.build().into(),
        Some(check) => ureq::Agent::with_parts(
            config.proxy(None).build(),
            DefaultConnector::default(),
            CheckedResolver {
                inner: DefaultResolver::default(),
                check,
            },
        ),
    }
}

pub fn download(url: &str, hosts: Hosts) -> Result<Vec<u8>, AppError> {
    let check: Option<Check> = match hosts {
        Hosts::Any => None,
        Hosts::Public => Some(Arc::new(check_address)),
    };
    fetch(url, &agent(check))
}

fn fetch(url: &str, agent: &ureq::Agent) -> Result<Vec<u8>, AppError> {
    let url = validate_url(url)?;
    let response = agent
        .get(&url)
        .header("User-Agent", "fiend")
        .header("Accept", "*/*")
        .call()
        .map_err(|error| AppError::rules(format!("Cannot download the list: {error}")))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(AppError::rules(format!("HTTP {status}")));
    }
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_DOWNLOAD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::rules(format!("Cannot download the list: {error}")))?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(AppError::rules("The list is larger than 32 MiB"));
    }
    if bytes.is_empty() {
        return Err(AppError::rules("The list is empty"));
    }
    Ok(bytes)
}

pub fn store_rule_file(
    bytes: &[u8],
    kind: RuleKind,
    destination: &Path,
) -> Result<RuleStats, AppError> {
    let temporary = destination.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(AppError::engine)?;
    let stats = match inspect_rule_file(&temporary.to_string_lossy(), kind) {
        Ok(stats) => stats,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(AppError::engine(error));
    }
    Ok(stats)
}

pub fn refresh_rule_file(
    url: &str,
    kind: RuleKind,
    destination: &Path,
    hosts: Hosts,
) -> Result<RuleStats, AppError> {
    store_rule_file(&download(url, hosts)?, kind, destination)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;

    use super::*;

    fn serve(status: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });
        format!("http://{address}/list")
    }

    fn directory(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("fiend-app-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn urls_are_checked() {
        assert_eq!(
            validate_url("  https://example.com/a.srs ").unwrap(),
            "https://example.com/a.srs"
        );
        for url in [
            "",
            "ftp://example.com/a",
            "file:///etc/passwd",
            "/a",
            "https://",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(validate_url(&format!("https://e.com/{}", "a".repeat(2048))).is_err());
    }

    #[test]
    fn list_is_downloaded_checked_and_stored() {
        let dir = directory("ok");
        let destination = dir.join("one.bin");
        let url = serve("200 OK", b"domain:a.ru\nfull:b.ru\n".to_vec());
        let stats = refresh_rule_file(&url, RuleKind::Direct, &destination, Hosts::Any).unwrap();
        assert_eq!((stats.domains, stats.exact), (1, 1));
        assert!(destination.is_file());
        assert!(!destination.with_extension("tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_list_keeps_the_old_file() {
        let dir = directory("bad");
        let destination = dir.join("one.bin");
        fs::write(&destination, b"domain:old.ru\n").unwrap();
        let url = serve("200 OK", b"regexp:x\n".to_vec());
        assert!(refresh_rule_file(&url, RuleKind::Direct, &destination, Hosts::Any).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"domain:old.ru\n");
        assert!(!destination.with_extension("tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn http_errors_and_empty_bodies_fail() {
        let error = download(&serve("404 Not Found", b"x".to_vec()), Hosts::Any).unwrap_err();
        assert!(error.to_string().contains("404"));
        let error = download(&serve("200 OK", Vec::new()), Hosts::Any).unwrap_err();
        assert!(error.to_string().contains("empty"));
    }

    fn redirect(location: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(head.as_bytes());
        });
        format!("http://{address}/list")
    }

    #[test]
    fn local_and_metadata_addresses_are_refused_after_resolution() {
        for address in [
            "127.0.0.1:80",
            "[::1]:80",
            "[::ffff:127.0.0.1]:80",
            "169.254.169.254:80",
            "[fe80::1]:80",
            "0.0.0.0:80",
            "100.100.100.200:80",
            "[fd00:ec2::254]:80",
            "240.0.0.1:80",
        ] {
            assert!(
                check_address(address.parse().unwrap()).is_err(),
                "{address}"
            );
        }
        for address in [
            "93.184.215.14:443",
            "192.168.1.5:80",
            "10.0.0.2:80",
            "100.64.0.1:80",
            "[fd12::1]:80",
        ] {
            assert!(check_address(address.parse().unwrap()).is_ok(), "{address}");
        }
        let error =
            download(&serve("200 OK", b"domain:a.ru\n".to_vec()), Hosts::Public).unwrap_err();
        assert!(error.to_string().contains("not allowed"), "{error}");
        let error = download("http://localhost:9/list", Hosts::Public).unwrap_err();
        assert!(error.to_string().contains("not allowed"), "{error}");
    }

    #[test]
    fn redirects_are_checked_again() {
        let target = serve("200 OK", b"domain:a.ru\n".to_vec());
        let bytes = download(&redirect(target), Hosts::Any).unwrap();
        assert_eq!(bytes, b"domain:a.ru\n");
        let first = redirect("http://127.0.0.1:9/list".into());
        let port: u16 = first
            .rsplit(':')
            .next()
            .unwrap()
            .trim_end_matches("/list")
            .parse()
            .unwrap();
        let check: Check = Arc::new(move |address: SocketAddr| {
            if address.port() == port {
                Ok(())
            } else {
                check_address(address)
            }
        });
        let error = fetch(&first, &agent(Some(check))).unwrap_err();
        assert!(error.to_string().contains("not allowed"), "{error}");
    }
}
