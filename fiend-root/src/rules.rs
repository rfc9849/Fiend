//! The iptables and policy routing commands of the root mode.
//!
//! Packets of the selected apps get a mark in `mangle OUTPUT`. The mark
//! routes them to `lo` through a local table, and `mangle PREROUTING` gives
//! them to the transparent sockets. The mark bit is in the range that netd
//! keeps free, so the network ID bits of Android stay the same.
//!
//! Android sets `tcp_fwmark_accept` and `fwmark_reflect`, so the packets that
//! the transparent sockets send back to an app also carry the mark. A TCP
//! packet that belongs to a socket already goes to that socket without
//! TPROXY, because TPROXY drops packets for sockets that are not transparent.
//! UDP replies have no mark, and a UDP packet from an app must not go to the
//! reply socket that is bound to its destination.
//!
//! netd does not apply two app rules to loopback: the background rule, and
//! Data Saver with "Background data" on metered networks. A BPF program does
//! the background check, and `fiend_metered` jumps to the Data Saver chain of
//! netd while the network is metered. Blocked packets are dropped, so they
//! never leave outside the tunnel.

use crate::protocol::Policy;

pub const MARK: u32 = 0x0100_0000;
pub const TABLE: u32 = 1097;
pub const PRIORITY: u32 = 9000;
const OUT: &str = "fiend_out";
const PRE: &str = "fiend_pre";
const GUARD: &str = "fiend_guard";
const METERED: &str = "fiend_metered";
pub const DATA_SAVER_CHAIN: &str = "bw_penalty_box";
pub const CLEANUP_REPEATS: usize = 8;

pub type Command = Vec<String>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    pub fn iptables(self) -> &'static str {
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

/// Rules that skip traffic that must never reach the transparent sockets:
/// loopback and the excluded UIDs, then `middle`, then the reserved and local
/// networks. DNS goes in `middle`, so that queries to a resolver on the local
/// network also go through Fiend.
fn skip_rules(
    family: Family,
    chain: &str,
    table: &str,
    policy: &Policy,
    first: Vec<Command>,
    middle: Vec<Command>,
) -> Vec<Command> {
    let mut rules = vec![table_rule(
        family,
        table,
        &["-A", chain, "-o", "lo", "-j", "RETURN"],
    )];
    rules.extend(first);
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
    rules.extend(middle);
    for network in family.reserved(policy.bypass_lan) {
        rules.push(table_rule(
            family,
            table,
            &["-A", chain, "-d", network, "-j", "RETURN"],
        ));
    }
    rules
}

fn dns_rules(
    family: Family,
    table: &str,
    chain: &str,
    policy: &Policy,
    target: &[&str],
) -> Vec<Command> {
    let mut rules = Vec::new();
    for range in ranges(policy) {
        for protocol in ["udp", "tcp"] {
            let mut parts = vec![
                "-A",
                chain,
                "-p",
                protocol,
                "--dport",
                "53",
                "-m",
                "owner",
                "--uid-owner",
                &range,
                "-j",
            ];
            parts.extend_from_slice(target);
            rules.push(table_rule(family, table, &parts));
        }
    }
    rules
}

/// DNS rules by socket UID, for queries that the Android resolver sends for
/// the apps. They come before the UID exceptions, because the resolver
/// socket has the file owner of netd.
fn resolver_rules(
    family: Family,
    table: &str,
    chain: &str,
    uids: &str,
    background: Option<&str>,
    target: &[&str],
) -> Vec<Command> {
    let mut rules = Vec::new();
    for protocol in ["udp", "tcp"] {
        let base = [
            "-A",
            chain,
            "-p",
            protocol,
            "--dport",
            "53",
            "-m",
            "bpf",
            "--object-pinned",
            uids,
        ];
        if let Some(program) = background {
            let mut parts = base.to_vec();
            parts.extend_from_slice(&["-m", "bpf", "--object-pinned", program, "-j", "DROP"]);
            rules.push(table_rule(family, table, &parts));
        }
        let mut parts = base.to_vec();
        parts.push("-j");
        parts.extend_from_slice(target);
        rules.push(table_rule(family, table, &parts));
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

/// What the helper found on this system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Support {
    /// The path of the pinned background program.
    pub background: Option<String>,
    /// The path of the pinned program that matches the captured socket UIDs.
    pub uids: Option<String>,
}

fn capture(family: Family, policy: &Policy, port: u16, support: &Support) -> Vec<Command> {
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
        table_rule(family, "filter", &["-N", METERED]),
    ];
    let mut middle = Vec::new();
    if let Some(program) = &support.background {
        for range in ranges(policy) {
            middle.push(table_rule(
                family,
                "mangle",
                &[
                    "-A",
                    OUT,
                    "-m",
                    "owner",
                    "--uid-owner",
                    &range,
                    "-m",
                    "bpf",
                    "--object-pinned",
                    program,
                    "-j",
                    "DROP",
                ],
            ));
        }
    }
    let mut first = Vec::new();
    if policy.dns {
        let target = ["MARK", "--set-xmark", &mark];
        match &support.uids {
            Some(uids) => first.extend(resolver_rules(
                family,
                "mangle",
                OUT,
                uids,
                support.background.as_deref(),
                &target,
            )),
            None => middle.extend(dns_rules(family, "mangle", OUT, policy, &target)),
        }
    }
    rules.extend(skip_rules(family, OUT, "mangle", policy, first, middle));
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
    rules.push(table_rule(
        family,
        "mangle",
        &[
            "-A",
            PRE,
            "-i",
            "lo",
            "-p",
            "tcp",
            "-m",
            "mark",
            "--mark",
            &mark,
            "-m",
            "socket",
            "--nowildcard",
            "-j",
            "RETURN",
        ],
    ));
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
    rules.push(table_rule(
        family,
        "filter",
        &["-A", GUARD, "-m", "mark", "--mark", &mark, "-j", METERED],
    ));
    rules.extend(jumps(family, true));
    rules
}

/// Commands that make `fiend_metered` follow the metered state. With no
/// Data Saver chain on the system the chain stays empty.
pub fn metered(family: Family, on: bool, chain_present: bool) -> Vec<Command> {
    let mut rules = vec![table_rule(family, "filter", &["-F", METERED])];
    if on && chain_present {
        rules.push(table_rule(
            family,
            "filter",
            &["-A", METERED, "-j", DATA_SAVER_CHAIN],
        ));
    }
    rules
}

/// The families that capture traffic for this policy.
pub fn families(policy: &Policy) -> Vec<Family> {
    if policy.ipv6 {
        vec![Family::V4, Family::V6]
    } else {
        vec![Family::V4]
    }
}

/// IPv6 of the selected apps is refused, so that the apps use IPv4 and no
/// IPv6 traffic goes around the tunnel.
fn refuse(family: Family, policy: &Policy, support: &Support) -> Vec<Command> {
    let mut rules = vec![table_rule(family, "filter", &["-N", GUARD])];
    let mut first = Vec::new();
    let mut middle = Vec::new();
    if policy.dns {
        match &support.uids {
            Some(uids) => first.extend(resolver_rules(
                family,
                "filter",
                GUARD,
                uids,
                None,
                &["REJECT"],
            )),
            None => middle.extend(dns_rules(family, "filter", GUARD, policy, &["REJECT"])),
        }
    }
    rules.extend(skip_rules(family, GUARD, "filter", policy, first, middle));
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
pub fn apply(policy: &Policy, port: u16, support: &Support) -> Vec<Command> {
    let mut rules = capture(Family::V4, policy, port, support);
    if policy.ipv6 {
        rules.extend(capture(Family::V6, policy, port, support));
    } else {
        rules.extend(refuse(Family::V6, policy, support));
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
        rules.push((false, table_rule(family, "filter", &["-F", METERED])));
        rules.push((false, table_rule(family, "filter", &["-X", METERED])));
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

/// The desktop Linux setup: every packet goes through Fiend, except packets
/// of the daemon itself, which carry `protect_mark`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desktop {
    pub ipv6: bool,
    pub bypass_lan: bool,
    pub protect_mark: u32,
    pub port: u16,
}

const FORWARD: &str = "fiend_fwd";
const PROBE: &str = "fiend_probe";

fn undo_of(apply: &Command) -> Command {
    let mut undo = apply.clone();
    for word in undo.iter_mut() {
        let swapped = match word.as_str() {
            "-A" | "-I" => "-D",
            "-N" => "-X",
            "add" => "del",
            _ => continue,
        };
        *word = swapped.to_owned();
        break;
    }
    undo
}

fn desktop_capture(family: Family, config: &Desktop) -> Vec<Command> {
    let mark = mark();
    let port = config.port.to_string();
    let protect = config.protect_mark.to_string();
    let table = TABLE.to_string();
    let priority = PRIORITY.to_string();
    let mut rules = vec![
        command(&[
            "ip",
            family.ip(),
            "rule",
            "add",
            "fwmark",
            &mark,
            "lookup",
            &table,
            "pref",
            &priority,
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
            &table,
        ]),
    ];
    for (name, chain) in [
        ("mangle", OUT),
        ("mangle", PRE),
        ("mangle", FORWARD),
        ("filter", GUARD),
    ] {
        rules.push(table_rule(family, name, &["-N", chain]));
    }
    let out = |parts: &[&str]| {
        let mut all = vec!["-A", OUT];
        all.extend_from_slice(parts);
        table_rule(family, "mangle", &all)
    };
    rules.push(out(&["-m", "mark", "--mark", &protect, "-j", "RETURN"]));
    rules.push(out(&["-o", "lo", "-j", "RETURN"]));
    for protocol in ["udp", "tcp"] {
        rules.push(out(&[
            "-p",
            protocol,
            "--dport",
            "53",
            "-j",
            "MARK",
            "--set-xmark",
            &mark,
        ]));
    }
    for network in family.reserved(config.bypass_lan) {
        rules.push(out(&["-d", network, "-j", "RETURN"]));
    }
    for protocol in ["tcp", "udp"] {
        rules.push(out(&["-p", protocol, "-j", "MARK", "--set-xmark", &mark]));
    }
    let tproxy = |chain: &str, prefix: &[&str], protocol: &str| {
        let mut all = vec!["-A", chain];
        all.extend_from_slice(prefix);
        all.extend_from_slice(&[
            "-p",
            protocol,
            "-j",
            "TPROXY",
            "--on-ip",
            family.loopback(),
            "--on-port",
            &port,
            "--tproxy-mark",
            &mark,
        ]);
        table_rule(family, "mangle", &all)
    };
    rules.push(table_rule(
        family,
        "mangle",
        &[
            "-A",
            PRE,
            "-i",
            "lo",
            "-p",
            "tcp",
            "-m",
            "mark",
            "--mark",
            &mark,
            "-m",
            "socket",
            "--nowildcard",
            "-j",
            "RETURN",
        ],
    ));
    for protocol in ["tcp", "udp"] {
        rules.push(tproxy(
            PRE,
            &["-i", "lo", "-m", "mark", "--mark", &mark],
            protocol,
        ));
    }
    rules.push(table_rule(
        family,
        "mangle",
        &["-A", PRE, "!", "-i", "lo", "-j", FORWARD],
    ));
    rules.push(table_rule(
        family,
        "mangle",
        &[
            "-A",
            FORWARD,
            "-m",
            "addrtype",
            "--dst-type",
            "LOCAL",
            "-j",
            "RETURN",
        ],
    ));
    for network in family.reserved(config.bypass_lan) {
        rules.push(table_rule(
            family,
            "mangle",
            &["-A", FORWARD, "-d", network, "-j", "RETURN"],
        ));
    }
    for protocol in ["tcp", "udp"] {
        rules.push(tproxy(FORWARD, &[], protocol));
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

/// IPv6 off: the IPv6 traffic of this system is refused, except loopback,
/// local networks and the daemon.
fn desktop_refuse(family: Family, config: &Desktop) -> Vec<Command> {
    let protect = config.protect_mark.to_string();
    let mut rules = vec![table_rule(family, "filter", &["-N", GUARD])];
    let guard = |parts: &[&str]| {
        let mut all = vec!["-A", GUARD];
        all.extend_from_slice(parts);
        table_rule(family, "filter", &all)
    };
    rules.push(guard(&["-m", "mark", "--mark", &protect, "-j", "RETURN"]));
    rules.push(guard(&["-o", "lo", "-j", "RETURN"]));
    for network in family.reserved(config.bypass_lan) {
        rules.push(guard(&["-d", network, "-j", "RETURN"]));
    }
    rules.push(guard(&[
        "-p",
        "tcp",
        "-j",
        "REJECT",
        "--reject-with",
        "tcp-reset",
    ]));
    rules.push(guard(&["-j", "REJECT"]));
    rules.push(table_rule(family, "filter", &["-I", "OUTPUT", "-j", GUARD]));
    rules
}

/// The desktop rules as (apply, undo) pairs, in apply order. Undo them in
/// reverse order.
pub fn desktop(config: &Desktop) -> Vec<(Command, Command)> {
    let mut rules = desktop_capture(Family::V4, config);
    if config.ipv6 {
        rules.extend(desktop_capture(Family::V6, config));
    } else {
        rules.extend(desktop_refuse(Family::V6, config));
    }
    rules
        .into_iter()
        .map(|apply| {
            let undo = undo_of(&apply);
            (apply, undo)
        })
        .collect()
}

/// Commands that must succeed while the desktop rules are in place.
pub fn desktop_check(config: &Desktop) -> Vec<Command> {
    let mut rules = jumps(Family::V4, false);
    if config.ipv6 {
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

/// Commands that try every match and target of the desktop rules in a
/// chain of their own, and the commands that remove the chain again.
pub fn probe(ipv6: bool) -> (Vec<Command>, Vec<Command>) {
    let mut setup = Vec::new();
    let mut cleanup = Vec::new();
    let families: &[Family] = if ipv6 {
        &[Family::V4, Family::V6]
    } else {
        &[Family::V4]
    };
    let mark = mark();
    for family in families {
        let family = *family;
        cleanup.push(table_rule(family, "mangle", &["-F", PROBE]));
        cleanup.push(table_rule(family, "mangle", &["-X", PROBE]));
        setup.push(table_rule(family, "mangle", &["-N", PROBE]));
        setup.push(table_rule(
            family,
            "mangle",
            &[
                "-A",
                PROBE,
                "-p",
                "tcp",
                "-m",
                "socket",
                "--nowildcard",
                "-j",
                "RETURN",
            ],
        ));
        setup.push(table_rule(
            family,
            "mangle",
            &[
                "-A",
                PROBE,
                "-m",
                "addrtype",
                "--dst-type",
                "LOCAL",
                "-j",
                "RETURN",
            ],
        ));
        setup.push(table_rule(
            family,
            "mangle",
            &[
                "-A",
                PROBE,
                "-p",
                "udp",
                "-j",
                "TPROXY",
                "--on-ip",
                family.loopback(),
                "--on-port",
                "9",
                "--tproxy-mark",
                &mark,
            ],
        ));
    }
    (setup, cleanup)
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
            metered: false,
            dns: true,
        }
    }

    fn lines(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|command| command.join(" ")).collect()
    }

    fn lines_of(commands: &[Command]) -> Vec<String> {
        lines(commands)
    }

    fn desktop_config(ipv6: bool) -> Desktop {
        Desktop {
            ipv6,
            bypass_lan: true,
            protect_mark: 51820,
            port: 34567,
        }
    }

    #[test]
    fn desktop_captures_everything_but_the_daemon() {
        let rules = desktop(&desktop_config(true));
        let apply: Vec<String> = rules.iter().map(|(apply, _)| apply.join(" ")).collect();
        let position = |line: &str| {
            apply
                .iter()
                .position(|item| item == line)
                .unwrap_or_else(|| panic!("missing {line}"))
        };
        let protect = position("iptables -w -t mangle -A fiend_out -m mark --mark 51820 -j RETURN");
        let dns = position(
            "iptables -w -t mangle -A fiend_out -p udp --dport 53 -j MARK --set-xmark 0x1000000/0x1000000",
        );
        let lan = position("iptables -w -t mangle -A fiend_out -d 192.168.0.0/16 -j RETURN");
        let all = position(
            "iptables -w -t mangle -A fiend_out -p tcp -j MARK --set-xmark 0x1000000/0x1000000",
        );
        assert!(protect < dns && dns < lan && lan < all);
        assert!(!apply.iter().any(|line| line.contains("--uid-owner")));
        position("ip6tables -w -t mangle -A fiend_pre ! -i lo -j fiend_fwd");
        position("ip6tables -w -t mangle -A fiend_fwd -m addrtype --dst-type LOCAL -j RETURN");
        position(
            "iptables -w -t mangle -A fiend_fwd -p udp -j TPROXY --on-ip 127.0.0.1 --on-port 34567 --tproxy-mark 0x1000000/0x1000000",
        );
        let jump = position("iptables -w -t mangle -I OUTPUT -j fiend_out");
        assert!(
            apply[..jump]
                .iter()
                .any(|line| line.contains("fiend_fwd -p tcp -j TPROXY"))
        );
        for (apply, undo) in &rules {
            let changed: Vec<_> = apply.iter().zip(undo).filter(|(a, b)| a != b).collect();
            assert_eq!(changed.len(), 1, "one word changes in {}", apply.join(" "));
        }
        let (_, undo) =
            &rules[position("ip -4 rule add fwmark 0x1000000/0x1000000 lookup 1097 pref 9000")];
        assert_eq!(
            undo.join(" "),
            "ip -4 rule del fwmark 0x1000000/0x1000000 lookup 1097 pref 9000"
        );
        let (_, undo) = &rules[position("iptables -w -t mangle -N fiend_fwd")];
        assert_eq!(undo.join(" "), "iptables -w -t mangle -X fiend_fwd");
        assert_eq!(desktop_check(&desktop_config(true)).len(), 6);
    }

    #[test]
    fn desktop_without_ipv6_refuses_it() {
        let apply: Vec<String> = desktop(&desktop_config(false))
            .iter()
            .map(|(apply, _)| apply.join(" "))
            .collect();
        assert!(!apply.iter().any(|line| line.starts_with("ip -6")));
        assert!(apply.contains(
            &"ip6tables -w -t filter -A fiend_guard -m mark --mark 51820 -j RETURN".to_owned()
        ));
        assert_eq!(
            apply.last().unwrap(),
            "ip6tables -w -t filter -I OUTPUT -j fiend_guard"
        );
        let (setup, cleanup) = probe(false);
        assert_eq!(setup.len(), 4);
        assert_eq!(cleanup.len(), 2);
        assert_eq!(probe(true).0.len(), 8);
    }

    #[test]
    fn metered_state_jumps_to_the_data_saver_chain() {
        assert_eq!(
            lines(&metered(Family::V4, true, true)),
            [
                "iptables -w -t filter -F fiend_metered",
                "iptables -w -t filter -A fiend_metered -j bw_penalty_box"
            ]
        );
        assert_eq!(
            lines(&metered(Family::V6, false, true)),
            ["ip6tables -w -t filter -F fiend_metered"]
        );
        assert_eq!(lines(&metered(Family::V4, true, false)).len(), 1);
        assert_eq!(families(&policy(false)), [Family::V4]);
    }

    #[test]
    fn capture_marks_selected_apps_and_skips_the_rest() {
        let support = Support {
            background: Some("/sys/fs/bpf/fiend_background".into()),
            uids: None,
        };
        let lines = lines(&apply(&policy(true), 34567, &support));
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
        let dns = position(
            "iptables -w -t mangle -A fiend_out -p udp --dport 53 -m owner --uid-owner 10000-19999 -j MARK --set-xmark 0x1000000/0x1000000",
        );
        let lan = position("iptables -w -t mangle -A fiend_out -d 192.168.0.0/16 -j RETURN");
        assert!(skip_app < dns && dns < lan);
        let first_mark = lines
            .iter()
            .position(|line| line.contains("-j MARK"))
            .unwrap();
        let background = position(
            "iptables -w -t mangle -A fiend_out -m owner --uid-owner 10000-19999 -m bpf --object-pinned /sys/fs/bpf/fiend_background -j DROP",
        );
        assert!(skip_root < background && skip_app < background && background < first_mark);
        assert!(has(
            "ip6tables -w -t filter -A fiend_guard -m mark --mark 0x1000000/0x1000000 -j fiend_metered"
        ));
        let plain = lines_of(&apply(&policy(true), 34567, &Support::default()));
        assert!(!plain.iter().any(|line| line.contains("-m bpf")));
        let existing = position(
            "iptables -w -t mangle -A fiend_pre -i lo -p tcp -m mark --mark 0x1000000/0x1000000 -m socket --nowildcard -j RETURN",
        );
        let tproxy = lines
            .iter()
            .position(|line| line.contains("-j TPROXY"))
            .unwrap();
        assert!(existing < tproxy);
        let jump = position("iptables -w -t mangle -I OUTPUT -j fiend_out");
        assert!(lines[..jump].iter().any(|line| line.contains("TPROXY")));
        assert!(has(
            "iptables -w -t mangle -A fiend_out -d 192.168.0.0/16 -j RETURN"
        ));
    }

    #[test]
    fn resolver_queries_match_by_socket_uid_before_the_root_exception() {
        let support = Support {
            background: Some("/sys/fs/bpf/fiend_background".into()),
            uids: Some("/sys/fs/bpf/fiend_uids".into()),
        };
        let lines = lines(&apply(&policy(true), 34567, &support));
        let position = |line: &str| lines.iter().position(|item| item == line).unwrap();
        let drop = position(
            "iptables -w -t mangle -A fiend_out -p udp --dport 53 -m bpf --object-pinned /sys/fs/bpf/fiend_uids -m bpf --object-pinned /sys/fs/bpf/fiend_background -j DROP",
        );
        let mark = position(
            "iptables -w -t mangle -A fiend_out -p tcp --dport 53 -m bpf --object-pinned /sys/fs/bpf/fiend_uids -j MARK --set-xmark 0x1000000/0x1000000",
        );
        let root = position("iptables -w -t mangle -A fiend_out -m owner --uid-owner 0 -j RETURN");
        assert!(drop < mark && mark < root);
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("--dport 53 -m owner"))
        );
        let off = lines_of(&apply(&policy(false), 34567, &support));
        let reject = off
            .iter()
            .position(|line| {
                line == "ip6tables -w -t filter -A fiend_guard -p udp --dport 53 -m bpf --object-pinned /sys/fs/bpf/fiend_uids -j REJECT"
            })
            .unwrap();
        let root = off
            .iter()
            .position(|line| {
                line == "ip6tables -w -t filter -A fiend_guard -m owner --uid-owner 0 -j RETURN"
            })
            .unwrap();
        assert!(reject < root);
    }

    #[test]
    fn dns_stays_direct_for_older_apps() {
        let mut old = policy(true);
        old.dns = false;
        let lines = lines(&apply(&old, 34567, &Support::default()));
        assert!(!lines.iter().any(|line| line.contains("--dport 53")));
    }

    #[test]
    fn ipv6_off_refuses_ipv6_of_selected_apps() {
        let lines = lines(&apply(&policy(false), 34567, &Support::default()));
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
        let guard = lines
            .iter()
            .position(|(_, line)| line == "iptables -w -t filter -F fiend_guard")
            .unwrap();
        let metered = lines
            .iter()
            .position(|(_, line)| line == "iptables -w -t filter -X fiend_metered")
            .unwrap();
        assert!(guard < metered);
        assert!(lines.iter().any(|(repeat, line)| *repeat
            && line == "ip -6 rule del fwmark 0x1000000/0x1000000 lookup 1097 pref 9000"));
        assert_eq!(check(&policy(true)).len(), 6);
        assert_eq!(check(&policy(false)).len(), 4);
    }
}
