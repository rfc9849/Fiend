use std::fmt::Write as _;
use std::net::IpAddr;

use crate::rules::{MARK, PRIORITY, TABLE};

pub const NFT_TABLE: &str = "fiend";
pub const DNS_SET_V4: &str = "dns4";
pub const DNS_SET_V6: &str = "dns6";
pub const DNS_SET_TIMEOUT_SECONDS: u32 = 3600;

const RESERVED_V4: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "224.0.0.0/4",
    "240.0.0.0/4",
];
const RESERVED_V6: &[&str] = &["::/127", "fc00::/7", "fe80::/10", "ff00::/8"];

pub type Range = (IpAddr, IpAddr);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gateway {
    pub interfaces: Vec<String>,
    pub port: u16,
    pub ipv6: bool,
    pub capture_all: bool,
    pub capture: Vec<Range>,
    pub bypass: Vec<Range>,
    pub dns_redirect: bool,
}

fn quote(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
        .collect();
    format!("\"{clean}\"")
}

fn range(from: IpAddr, to: IpAddr) -> String {
    if from == to {
        from.to_string()
    } else {
        format!("{from}-{to}")
    }
}

fn elements(ranges: impl Iterator<Item = String>) -> String {
    let list: Vec<String> = ranges.collect();
    if list.is_empty() {
        String::new()
    } else {
        format!(" elements = {{ {} }};", list.join(", "))
    }
}

fn family_ranges(ranges: &[Range], v6: bool) -> impl Iterator<Item = String> + '_ {
    ranges
        .iter()
        .filter(move |(from, _)| from.is_ipv6() == v6)
        .map(|(from, to)| range(*from, *to))
}

pub fn ruleset(config: &Gateway) -> String {
    let mark = format!("{MARK:#x}");
    let port = config.port;
    let timeout = DNS_SET_TIMEOUT_SECONDS;
    let mut families = vec![("ip", "4", "ipv4_addr", "127.0.0.1", RESERVED_V4)];
    if config.ipv6 {
        families.push(("ip6", "6", "ipv6_addr", "[::1]", RESERVED_V6));
    }
    let interfaces = elements(config.interfaces.iter().map(|name| quote(name)));
    let mut text = String::new();
    let _ = writeln!(text, "table inet {NFT_TABLE} {{");
    let _ = writeln!(text, "\tset interfaces {{ type ifname;{interfaces} }}");
    for (_, suffix, kind, _, reserved) in &families {
        let v6 = *suffix == "6";
        let _ = writeln!(
            text,
            "\tset reserved{suffix} {{ type {kind}; flags interval;{} }}",
            elements(reserved.iter().map(|item| (*item).to_owned()))
        );
        let _ = writeln!(
            text,
            "\tset dns{suffix} {{ type {kind}; flags dynamic, timeout; timeout {timeout}s; }}"
        );
        let _ = writeln!(
            text,
            "\tset lists{suffix} {{ type {kind}; flags interval; auto-merge;{} }}",
            elements(family_ranges(&config.capture, v6))
        );
        if config.capture_all {
            let _ = writeln!(
                text,
                "\tset direct{suffix} {{ type {kind}; flags interval; auto-merge;{} }}",
                elements(family_ranges(&config.bypass, v6))
            );
        }
    }
    let _ = writeln!(text, "\tchain prerouting {{");
    let _ = writeln!(
        text,
        "\t\ttype filter hook prerouting priority mangle; policy accept;"
    );
    let _ = writeln!(text, "\t\tiifname != @interfaces return");
    let _ = writeln!(text, "\t\tmeta l4proto != {{ tcp, udp }} return");
    if !config.ipv6 {
        let _ = writeln!(text, "\t\tmeta nfproto ipv6 return");
    }
    let _ = writeln!(text, "\t\tfib daddr type local return");
    for (ip, suffix, ..) in &families {
        let _ = writeln!(text, "\t\t{ip} daddr @reserved{suffix} return");
    }
    if config.dns_redirect {
        let _ = writeln!(text, "\t\tth dport 53 return");
    }
    let _ = writeln!(
        text,
        "\t\tmeta l4proto tcp socket transparent 1 meta mark set {mark} accept"
    );
    for (ip, suffix, ..) in &families {
        let _ = writeln!(
            text,
            "\t\t{ip} daddr @dns{suffix} update @dns{suffix} {{ {ip} daddr timeout {timeout}s }} goto capture"
        );
        let _ = writeln!(text, "\t\t{ip} daddr @lists{suffix} goto capture");
    }
    if config.capture_all {
        for (ip, suffix, ..) in &families {
            let _ = writeln!(text, "\t\t{ip} daddr @direct{suffix} return");
        }
        let _ = writeln!(text, "\t\tgoto capture");
    }
    let _ = writeln!(text, "\t}}");
    let _ = writeln!(text, "\tchain capture {{");
    for (ip, _, _, loopback, _) in &families {
        let _ = writeln!(
            text,
            "\t\tmeta nfproto {} meta l4proto {{ tcp, udp }} tproxy {ip} to {loopback}:{port} meta mark set {mark} accept",
            if *ip == "ip" { "ipv4" } else { "ipv6" }
        );
    }
    let _ = writeln!(text, "\t}}");
    if config.dns_redirect {
        let _ = writeln!(text, "\tchain dns {{");
        let _ = writeln!(
            text,
            "\t\ttype nat hook prerouting priority dstnat - 5; policy accept;"
        );
        let _ = writeln!(
            text,
            "\t\tiifname @interfaces meta l4proto {{ tcp, udp }} th dport 53 fib daddr type != local redirect to :53"
        );
        let _ = writeln!(text, "\t}}");
    }
    let _ = writeln!(text, "}}");
    text
}

pub fn routes(ipv6: bool) -> Vec<(Vec<String>, Vec<String>)> {
    let mark = format!("{MARK:#x}/{MARK:#x}");
    let table = TABLE.to_string();
    let priority = PRIORITY.to_string();
    let mut families = vec![("-4", "0.0.0.0/0")];
    if ipv6 {
        families.push(("-6", "::/0"));
    }
    let pair = |text: &[&str]| {
        let apply: Vec<String> = text.iter().map(|w| (*w).to_owned()).collect();
        let undo = apply
            .iter()
            .map(|w| {
                if w == "add" {
                    "del".to_owned()
                } else {
                    w.clone()
                }
            })
            .collect();
        (apply, undo)
    };
    let mut steps = Vec::new();
    for (family, any) in families {
        steps.push(pair(&[
            "ip", family, "rule", "add", "fwmark", &mark, "lookup", &table, "pref", &priority,
        ]));
        steps.push(pair(&[
            "ip", family, "route", "add", "local", any, "dev", "lo", "table", &table,
        ]));
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(capture_all: bool, ipv6: bool) -> Gateway {
        Gateway {
            interfaces: vec!["br-lan".into(), "wg\"0;".into()],
            port: 34567,
            ipv6,
            capture_all,
            capture: vec![
                (
                    "203.0.113.0".parse().unwrap(),
                    "203.0.113.255".parse().unwrap(),
                ),
                (
                    "2001:db8::".parse().unwrap(),
                    "2001:db8::ffff".parse().unwrap(),
                ),
                (
                    "198.51.100.7".parse().unwrap(),
                    "198.51.100.7".parse().unwrap(),
                ),
            ],
            bypass: vec![("192.0.2.0".parse().unwrap(), "192.0.2.255".parse().unwrap())],
            dns_redirect: true,
        }
    }

    #[test]
    fn listed_traffic_goes_to_fiend_and_the_rest_stays_in_the_kernel() {
        let text = ruleset(&config(false, true));
        assert!(
            text.contains("set interfaces { type ifname; elements = { \"br-lan\", \"wg0\" }; }")
        );
        assert!(text.contains("elements = { 203.0.113.0-203.0.113.255, 198.51.100.7 };"));
        assert!(text.contains("elements = { 2001:db8::-2001:db8::ffff };"));
        assert!(
            text.contains("ip daddr @dns4 update @dns4 { ip daddr timeout 3600s } goto capture")
        );
        assert!(text.contains(
            "meta nfproto ipv6 meta l4proto { tcp, udp } tproxy ip6 to [::1]:34567 meta mark set 0x1000000 accept"
        ));
        assert!(!text.contains("\t\tgoto capture\n"));
        assert!(!text.contains("direct4"));
        let socket = text.find("socket transparent 1").unwrap();
        let reserved = text.find("ip daddr @reserved4 return").unwrap();
        let dns = text.find("th dport 53 return").unwrap();
        let capture = text.find("goto capture").unwrap();
        assert!(reserved < dns && dns < socket && socket < capture);
        assert!(text.contains("redirect to :53"));
    }

    #[test]
    fn capture_all_skips_only_the_direct_list() {
        let text = ruleset(&config(true, false));
        assert!(text.contains("set direct4 { type ipv4_addr; flags interval; auto-merge; elements = { 192.0.2.0-192.0.2.255 }; }"));
        assert!(text.contains("\t\tip daddr @direct4 return\n\t\tgoto capture\n\t}"));
        assert!(text.contains("meta nfproto ipv6 return"));
        assert!(!text.contains("ip6 "));
        assert!(!text.contains("set dns6"));
        let mut quiet = config(false, false);
        quiet.dns_redirect = false;
        let text = ruleset(&quiet);
        assert!(!text.contains("dport 53"));
        assert!(!text.contains("chain dns"));
    }

    #[test]
    fn routes_have_their_undo() {
        let steps = routes(true);
        assert_eq!(steps.len(), 4);
        assert_eq!(
            steps[0].0.join(" "),
            "ip -4 rule add fwmark 0x1000000/0x1000000 lookup 1097 pref 9000"
        );
        assert_eq!(
            steps[3].1.join(" "),
            "ip -6 route del local ::/0 dev lo table 1097"
        );
        assert_eq!(routes(false).len(), 2);
    }
}
