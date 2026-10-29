use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use crate::{AppError, RuleKind, RuleStats, inspect_rule_file};

pub const MAX_DOWNLOAD_BYTES: usize = 32 << 20;
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_URL: usize = 2048;

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

pub fn download(url: &str) -> Result<Vec<u8>, AppError> {
    let url = validate_url(url)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .into();
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
) -> Result<RuleStats, AppError> {
    store_rule_file(&download(url)?, kind, destination)
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
        let stats = refresh_rule_file(&url, RuleKind::Direct, &destination).unwrap();
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
        assert!(refresh_rule_file(&url, RuleKind::Direct, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"domain:old.ru\n");
        assert!(!destination.with_extension("tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn http_errors_and_empty_bodies_fail() {
        let error = download(&serve("404 Not Found", b"x".to_vec())).unwrap_err();
        assert!(error.to_string().contains("404"));
        let error = download(&serve("200 OK", Vec::new())).unwrap_err();
        assert!(error.to_string().contains("empty"));
    }
}
