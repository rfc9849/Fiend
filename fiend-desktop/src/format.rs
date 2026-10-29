use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub fn duration(seconds: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

pub fn elapsed(since_ms: u64, now_ms: u64) -> String {
    duration(now_ms.saturating_sub(since_ms) / 1000)
}

/// A value and a unit for a rate in bytes per second, shown as bits.
pub fn rate(bytes_per_second: u64) -> (String, &'static str) {
    let bits = bytes_per_second as f64 * 8.0;
    if bits >= 1e9 {
        (format!("{:.2}", bits / 1e9), "Gbps")
    } else if bits >= 1e6 {
        (format!("{:.1}", bits / 1e6), "Mbps")
    } else if bits >= 1e3 {
        (format!("{:.0}", bits / 1e3), "Kbps")
    } else {
        (format!("{bits:.0}"), "bps")
    }
}

pub fn size(bytes: u64) -> (String, &'static str) {
    let value = bytes as f64;
    if value >= 1e9 {
        (format!("{:.2}", value / 1e9), "GB")
    } else if value >= 1e6 {
        (format!("{:.1}", value / 1e6), "MB")
    } else if value >= 1e3 {
        (format!("{:.0}", value / 1e3), "KB")
    } else {
        (format!("{bytes}"), "B")
    }
}

pub fn clock(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds % 86400 / 3600,
        seconds % 3600 / 60,
        seconds % 60,
        timestamp_ms % 1000
    )
}

pub fn initials(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect();
    let text = if letters.chars().count() >= 2 {
        letters
    } else {
        name.chars().take(2).collect()
    };
    text.to_uppercase()
}

pub fn host_port(server: &str) -> (String, String) {
    match server.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            (host.trim_matches(['[', ']']).to_owned(), port.to_owned())
        }
        _ => (server.to_owned(), "443".to_owned()),
    }
}

pub fn join_host_port(host: &str, port: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_sizes_are_readable() {
        assert_eq!(duration(2534), "00:42:14");
        assert_eq!(elapsed(1_000, 4_000), "00:00:03");
        assert_eq!(rate(3_100_000), ("24.8".into(), "Mbps"));
        assert_eq!(rate(0), ("0".into(), "bps"));
        assert_eq!(size(1_240_000_000), ("1.24".into(), "GB"));
        assert_eq!(size(512), ("512".into(), "B"));
        assert_eq!(clock(34_862_018), "09:41:02.018");
    }

    #[test]
    fn names_and_addresses_split_cleanly() {
        assert_eq!(initials("Amsterdam"), "AM");
        assert_eq!(initials("home lab"), "HL");
        assert_eq!(
            host_port("nl.example:8443"),
            ("nl.example".into(), "8443".into())
        );
        assert_eq!(host_port("[::1]:443"), ("::1".into(), "443".into()));
        assert_eq!(host_port("nl.example"), ("nl.example".into(), "443".into()));
        assert_eq!(join_host_port("::1", "443"), "[::1]:443");
        assert_eq!(join_host_port("a.b", "1"), "a.b:1");
    }
}
