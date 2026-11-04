//! The iptables and policy routing commands of the root mode.
//!
//! Packets of the selected apps get a mark in `mangle OUTPUT`. The mark
//! routes them to `lo` through a local table, and `mangle PREROUTING` gives
//! them to the transparent sockets. The mark bit is in the range that netd
//! keeps free, so the network ID bits of Android stay the same.

use crate::protocol::Policy;

pub const MARK: u32 = 0x0100_0000;
pub const TABLE: u32 = 1097;
pub const PRIORITY: u32 = 9000;
const OUT: &str = "fiend_out";
const PRE: &str = "fiend_pre";
const GUARD: &str = "fiend_guard";
pub const CLEANUP_REPEATS: usize = 8;

pub type Command = Vec<String>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    V4,
    V6,
}

impl Family {
    fn iptables(self) -> &'static str {
        match self {
            Self::V4 => "iptables",
            Self::V6 => "ip6tables",
        }
    }

    fn ip(self) -> &'static str {
        match self {
            Self::V4 => "-4",
            Self::V6 => "-6",
        }
    }

    fn loopback(self) -> &'static str {
        match self {
            Self::V4 => "127.0.0.1",
            Self::V6 => "::1",
        }
    }

    fn any(self) -> &'static str {
        match self {
            Self::V4 => "0.0.0.0/0",
            Self::V6 => "::/0",
        }
    }

    fn reserved(self, bypass_lan: bool) -> Vec<&'static str> {
        let (always, lan): (&[&str], &[&str]) = match self {
            Self::V4 => (
                &[
                    "0.0.0.0/8",
                    "127.0.0.0/8",
                    "169.254.0.0/16",
                    "224.0.0.0/4",
                    "240.0.0.0/4",
                ],
                &[
                    "10.0.0.0/8",
                    "100.64.0.0/10",
                    "172.16.0.0/12",
                    "192.168.0.0/16",
                ],
            ),
            Self::V6 => (
                &["::/128", "::1/128", "fe80::/10", "ff00::/8"],
                &["fc00::/7"],
            ),
        };
        let mut list = always.to_vec();
        if bypass_lan {
            list.extend_from_slice(lan);
        }
        list
    }
}

fn command(parts: &[&str]) -> Command {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn table_rule(family: Family, table: &str, rest: &[&str]) -> Command {
    let mut parts = vec![family.iptables(), "-w", "-t", table];
    parts.extend_from_slice(rest);
    command(&parts)
}

fn mark() -> String {
    format!("{MARK:#x}/{MARK:#x}")
}

/// Rules that skip traffic that must never reach the transparent sockets.
fn skip_rules(family: Family, chain: &str, table: &str, policy: &Policy) -> Vec<Command> {
    let mut rules = vec![table_rule(
        family,
        table,
        &["-A", chain, "-o", "lo", "-j", "RETURN"],
    )];
    for network in family.reserved(policy.bypass_lan) {
        rules.push(table_rule(
            family,
            table,
            &["-A", chain, "-d", network, "-j", "RETURN"],
        ));
    }
    for protocol in ["udp", "tcp"] {
        rules.push(table_rule(
            family,
            table,
            &["-A", chain, "-p", protocol, "--dport", "53", "-j", "RETURN"],
        ));
    }
    let mut exclude = vec![0];
    exclude.extend(policy.exclude.iter().copied());
    for uid in exclude {
        let uid = uid.to_string();
        rules.push(table_rule(
            family,
            table,
            &[
                "-A",
                chain,
                "-m",
                "owner",
                "--uid-owner",
                &uid,
                "-j",
                "RETURN",
            ],
        ));
    }
    rules
}

fn ranges(policy: &Policy) -> Vec<String> {
    policy
        .include
        .iter()
        .map(|range| {
            if range.first == range.last {
                range.first.to_string()
            } else {
                format!("{}-{}", range.first, range.last)
            }
        })
        .collect()
}

fn capture(family: Family, policy: &Policy, port: u16) -> Vec<Command> {
    let mark = mark();
    let port = port.to_string();
    let mut rules = vec![
        command(&[
            "ip",
            family.ip(),
            "rule",
            "add",
            "fwmark",
            &mark,
            "lookup",
            &TABLE.to_string(),
            "pref",
            &PRIORITY.to_string(),
        ]),
        command(&[
            "ip",
            family.ip(),
            "route",
            "add",
            "local",
            family.any(),
            "dev",
            "lo",
            "table",
            &TABLE.to_string(),
        ]),
        table_rule(family, "mangle", &["-N", OUT]),
        table_rule(family, "mangle", &["-N", PRE]),
        table_rule(family, "filter", &["-N", GUARD]),
    ];
    rules.extend(skip_rules(family, OUT, "mangle", policy));
    for range in ranges(policy) {
        for protocol in ["tcp", "udp"] {
            rules.push(table_rule(
                family,
                "mangle",
                &[
                    "-A",
                    OUT,
                    "-p",
                    protocol,
                    "-m",
                    "owner",
                    "--uid-owner",
                    &range,
                    "-j",
                    "MARK",
                    "--set-xmark",
                    &mark,
                ],
            ));
        }
    }
    for protocol in ["tcp", "udp"] {
        rules.push(table_rule(
            family,
            "mangle",
            &[
                "-A",
                PRE,
                "-i",
                "lo",
                "-p",
                protocol,
                "-m",
                "mark",
                "--mark",
                &mark,
                "-j",
                "TPROXY",
                "--on-ip",
                family.loopback(),
                "--on-port",
                &port,
                "--tproxy-mark",
                &mark,
            ],
        ));
    }
    for (protocol, reject) in [
        ("tcp", "tcp-reset"),
        (
            "udp",
            match family {
                Family::V4 => "icmp-port-unreachable",
                Family::V6 => "icmp6-port-unreachable",
            },
        ),
    ] {
        rules.push(table_rule(
            family,
            "filter",
            &[
                "-A",
                GUARD,
                "-d",
                family.loopback(),
                "-p",
                protocol,
                "--dport",
                &port,
                "-j",
                "REJECT",
                "--reject-with",
                reject,
            ],
        ));
    }
    rules.extend(jumps(family, true));
    rules
}

/// IPv6 of the selected apps is refused, so that the apps use IPv4 and no
/// IPv6 traffic goes around the tunnel.
fn refuse(family: Family, policy: &Policy) -> Vec<Command> {
    let mut rules = vec![table_rule(family, "filter", &["-N", GUARD])];
    rules.extend(skip_rules(family, GUARD, "filter", policy));
    for range in ranges(policy) {
        rules.push(table_rule(
            family,
            "filter",
            &[
                "-A",
                GUARD,
                "-p",
                "tcp",
                "-m",
                "owner",
                "--uid-owner",
                &range,
                "-j",
                "REJECT",
                "--reject-with",
                "tcp-reset",
            ],
        ));
        rules.push(table_rule(
            family,
            "filter",
            &[
                "-A",
                GUARD,
                "-m",
                "owner",
                "--uid-owner",
                &range,
                "-j",
                "REJECT",
            ],
        ));
    }
    rules.push(table_rule(family, "filter", &["-I", "OUTPUT", "-j", GUARD]));
    rules
}

/// The jumps from the built-in chains.
fn jumps(family: Family, insert: bool) -> Vec<Command> {
    let action = if insert { "-I" } else { "-C" };
    vec![
        table_rule(family, "mangle", &[action, "OUTPUT", "-j", OUT]),
        table_rule(family, "mangle", &[action, "PREROUTING", "-j", PRE]),
        table_rule(family, "filter", &[action, "OUTPUT", "-j", GUARD]),
    ]
}

/// Commands that set up the rules. Run them in order and stop at the first
/// error.
pub fn apply(policy: &Policy, port: u16) -> Vec<Command> {
    let mut rules = capture(Family::V4, policy, port);
    if policy.ipv6 {
        rules.extend(capture(Family::V6, policy, port));
    } else {
        rules.extend(refuse(Family::V6, policy));
    }
    rules
}

/// Commands that must succeed while the rules are in place.
pub fn check(policy: &Policy) -> Vec<Command> {
    let mut rules = jumps(Family::V4, false);
    if policy.ipv6 {
        rules.extend(jumps(Family::V6, false));
    } else {
        rules.push(table_rule(
            Family::V6,
            "filter",
            &["-C", "OUTPUT", "-j", GUARD],
        ));
    }
    rules
}

/// Commands that remove every rule. Each one can fail when the rule is
/// absent; a command marked with `true` repeats until it fails.
pub fn cleanup() -> Vec<(bool, Command)> {
    let mut rules = Vec::new();
    for family in [Family::V4, Family::V6] {
        for (table, chain, parent) in [
            ("mangle", OUT, "OUTPUT"),
            ("mangle", PRE, "PREROUTING"),
            ("filter", GUARD, "OUTPUT"),
        ] {
            rules.push((
                true,
                table_rule(family, table, &["-D", parent, "-j", chain]),
            ));
            rules.push((false, table_rule(family, table, &["-F", chain])));
            rules.push((false, table_rule(family, table, &["-X", chain])));
        }
        rules.push((
            true,
            command(&[
                "ip",
                family.ip(),
                "rule",
                "del",
                "fwmark",
                &mark(),
                "lookup",
                &TABLE.to_string(),
                "pref",
                &PRIORITY.to_string(),
            ]),
        ));
        rules.push((
            false,
            command(&[
                "ip",
                family.ip(),
                "route",
                "flush",
                "table",
                &TABLE.to_string(),
            ]),
        ));
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::UidRange;

    fn policy(ipv6: bool) -> Policy {
        Policy {
            include: vec![
                UidRange {
                    first: 10000,
                    last: 19999,
                },
                UidRange {
                    first: 10123,
                    last: 10123,
                },
            ],
            exclude: vec![10200],
            ipv6,
            bypass_lan: true,
        }
    }

    fn lines(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|command| command.join(" ")).collect()
    }

    #[test]
    fn capture_marks_selected_apps_and_skips_the_rest() {
        let lines = lines(&apply(&policy(true), 34567));
        let has = |line: &str| lines.iter().any(|item| item == line);
        assert!(has(
            "ip -4 rule add fwmark 0x1000000/0x1000000 lookup 1097 pref 9000"
        ));
        assert!(has("ip -6 route add local ::/0 dev lo table 1097"));
        assert!(has(
            "iptables -w -t mangle -A fiend_out -p tcp -m owner --uid-owner 10000-19999 -j MARK --set-xmark 0x1000000/0x1000000"
        ));
        assert!(has(
            "iptables -w -t mangle -A fiend_out -p udp -m owner --uid-owner 10123 -j MARK --set-xmark 0x1000000/0x1000000"
        ));
        assert!(has(
            "ip6tables -w -t mangle -A fiend_pre -i lo -p udp -m mark --mark 0x1000000/0x1000000 -j TPROXY --on-ip ::1 --on-port 34567 --tproxy-mark 0x1000000/0x1000000"
        ));
        assert!(has(
            "iptables -w -t filter -A fiend_guard -d 127.0.0.1 -p tcp --dport 34567 -j REJECT --reject-with tcp-reset"
        ));
        let position = |line: &str| lines.iter().position(|item| item == line).unwrap();
        let skip_root =
            position("iptables -w -t mangle -A fiend_out -m owner --uid-owner 0 -j RETURN");
        let skip_app =
            position("iptables -w -t mangle -A fiend_out -m owner --uid-owner 10200 -j RETURN");
        let skip_dns = position("iptables -w -t mangle -A fiend_out -p udp --dport 53 -j RETURN");
        let first_mark = lines
            .iter()
            .position(|line| line.contains("-j MARK"))
            .unwrap();
        assert!(skip_root < first_mark && skip_app < first_mark && skip_dns < first_mark);
        let jump = position("iptables -w -t mangle -I OUTPUT -j fiend_out");
        assert!(lines[..jump].iter().any(|line| line.contains("TPROXY")));
        assert!(has(
            "iptables -w -t mangle -A fiend_out -d 192.168.0.0/16 -j RETURN"
        ));
    }

    #[test]
    fn ipv6_off_refuses_ipv6_of_selected_apps() {
        let lines = lines(&apply(&policy(false), 34567));
        assert!(!lines.iter().any(|line| line.starts_with("ip -6")));
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("ip6tables -w -t mangle"))
        );
        assert!(lines.iter().any(|line| line
            == "ip6tables -w -t filter -A fiend_guard -p tcp -m owner --uid-owner 10000-19999 -j REJECT --reject-with tcp-reset"));
        assert_eq!(
            lines.last().unwrap(),
            "ip6tables -w -t filter -I OUTPUT -j fiend_guard"
        );
        assert!(
            lines.iter().position(|line| line
                == "ip6tables -w -t filter -A fiend_guard -m owner --uid-owner 10200 -j RETURN")
                < lines
                    .iter()
                    .position(|line| line.contains("REJECT --reject-with tcp-reset")
                        && line.starts_with("ip6tables"))
        );
    }

    #[test]
    fn cleanup_removes_jumps_before_chains() {
        let lines: Vec<(bool, String)> = cleanup()
            .into_iter()
            .map(|(repeat, command)| (repeat, command.join(" ")))
            .collect();
        let jump = lines
            .iter()
            .position(|(_, line)| line == "iptables -w -t mangle -D OUTPUT -j fiend_out")
            .unwrap();
        let delete = lines
            .iter()
            .position(|(_, line)| line == "iptables -w -t mangle -X fiend_out")
            .unwrap();
        assert!(jump < delete && lines[jump].0);
        assert!(lines.iter().any(|(repeat, line)| *repeat
            && line == "ip -6 rule del fwmark 0x1000000/0x1000000 lookup 1097 pref 9000"));
        assert_eq!(check(&policy(true)).len(), 6);
        assert_eq!(check(&policy(false)).len(), 4);
    }
}
