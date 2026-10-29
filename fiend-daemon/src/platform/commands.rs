//! Network setup as a list of commands, each with its undo command.
//!
//! The daemon writes the undo command to the journal before it runs the
//! command. Linux uses this with `ip` and `resolvectl`.

use std::io;
use std::path::Path;

use crate::journal::{Journal, JournalFile};

pub const WRITE_RESOLV: &str = "@write-resolv";
pub const RESTORE_FILE: &str = "@restore-file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub apply: Vec<String>,
    pub undo: Option<Vec<String>>,
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn pair(apply: &str, undo: &str) -> Step {
    Step {
        apply: words(apply),
        undo: Some(words(undo)),
    }
}

/// Policy routing as in wg-quick. Sockets with the mark use the main table.
/// All other traffic uses a table with a default route to the tunnel.
pub fn route_steps(iface: &str, table: u32) -> Vec<Step> {
    let mut steps = Vec::new();
    for family in ["-4", "-6"] {
        steps.push(pair(
            &format!("ip {family} route add default dev {iface} table {table}"),
            &format!("ip {family} route del default dev {iface} table {table}"),
        ));
        steps.push(pair(
            &format!("ip {family} rule add not fwmark {table} table {table}"),
            &format!("ip {family} rule del not fwmark {table} table {table}"),
        ));
        steps.push(pair(
            &format!("ip {family} rule add table main suppress_prefixlength 0"),
            &format!("ip {family} rule del table main suppress_prefixlength 0"),
        ));
    }
    steps
}

pub fn dns_steps(
    iface: &str,
    dns: &str,
    resolved: bool,
    backup: &Path,
    resolv: &Path,
) -> Vec<Step> {
    if resolved {
        vec![
            Step {
                apply: words(&format!("resolvectl dns {iface} {dns}")),
                undo: Some(words(&format!("resolvectl revert {iface}"))),
            },
            Step {
                apply: vec![
                    "resolvectl".into(),
                    "domain".into(),
                    iface.into(),
                    "~.".into(),
                ],
                undo: None,
            },
            Step {
                apply: words(&format!("resolvectl default-route {iface} yes")),
                undo: None,
            },
        ]
    } else {
        vec![Step {
            apply: vec![
                WRITE_RESOLV.into(),
                dns.into(),
                backup.to_string_lossy().into_owned(),
                resolv.to_string_lossy().into_owned(),
            ],
            undo: Some(vec![
                RESTORE_FILE.into(),
                backup.to_string_lossy().into_owned(),
                resolv.to_string_lossy().into_owned(),
            ]),
        }]
    }
}

pub trait Runner {
    fn run(&mut self, argv: &[String]) -> io::Result<()>;
}

pub fn apply_steps(
    steps: &[Step],
    journal: &mut Journal,
    file: &JournalFile,
    runner: &mut dyn Runner,
) -> io::Result<()> {
    for step in steps {
        if let Some(undo) = &step.undo {
            journal.undo.push(undo.clone());
            file.save(journal)?;
        }
        runner.run(&step.apply)?;
    }
    Ok(())
}

/// Run every undo command, newest first. A failing command does not stop the rest.
pub fn undo_all(journal: &Journal, runner: &mut dyn Runner) -> usize {
    let mut failed = 0;
    for argv in journal.undo.iter().rev() {
        if let Err(error) = runner.run(argv) {
            failed += 1;
            tracing::warn!(command = %argv.join(" "), %error, "undo command failed");
        }
    }
    failed
}

pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, argv: &[String]) -> io::Result<()> {
        let Some((program, arguments)) = argv.split_first() else {
            return Ok(());
        };
        match program.as_str() {
            WRITE_RESOLV => {
                let [dns, backup, resolv] = arguments else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "bad resolver command",
                    ));
                };
                if !Path::new(backup).exists() {
                    std::fs::copy(resolv, backup)?;
                }
                std::fs::write(resolv, format!("# Written by Fiend\nnameserver {dns}\n"))
            }
            RESTORE_FILE => {
                let [backup, target] = arguments else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "bad restore command",
                    ));
                };
                if Path::new(backup).exists() {
                    std::fs::copy(backup, target)?;
                    std::fs::remove_file(backup)?;
                }
                Ok(())
            }
            _ => {
                let output = std::process::Command::new(program)
                    .args(arguments)
                    .output()?;
                if output.status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!(
                        "{}: {}",
                        argv.join(" "),
                        String::from_utf8_lossy(&output.stderr).trim()
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        seen: Vec<String>,
        fail_on: Option<String>,
    }

    impl Runner for Recorder {
        fn run(&mut self, argv: &[String]) -> io::Result<()> {
            let line = argv.join(" ");
            self.seen.push(line.clone());
            if self
                .fail_on
                .as_deref()
                .is_some_and(|text| line.contains(text))
            {
                Err(io::Error::other("failed"))
            } else {
                Ok(())
            }
        }
    }

    fn file(name: &str) -> JournalFile {
        let dir =
            std::env::temp_dir().join(format!("fiend-commands-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        JournalFile::new(dir.join("journal.json"))
    }

    #[test]
    fn route_steps_follow_wg_quick_for_both_families() {
        let steps = route_steps("fiend0", 51820);
        let text: Vec<String> = steps.iter().map(|s| s.apply.join(" ")).collect();
        assert!(text.contains(&"ip -4 route add default dev fiend0 table 51820".to_owned()));
        assert!(text.contains(&"ip -4 rule add not fwmark 51820 table 51820".to_owned()));
        assert!(text.contains(&"ip -4 rule add table main suppress_prefixlength 0".to_owned()));
        assert!(text.contains(&"ip -6 rule add not fwmark 51820 table 51820".to_owned()));
        assert_eq!(steps.len(), 6);
        assert!(
            steps
                .iter()
                .all(|s| s.undo.as_ref().unwrap()[3] == "del"
                    || s.undo.as_ref().unwrap()[4] == "del")
        );
    }

    #[test]
    fn dns_steps_use_resolved_or_a_backed_up_file() {
        let steps = dns_steps(
            "fiend0",
            "198.18.0.1",
            true,
            Path::new("/b"),
            Path::new("/r"),
        );
        assert_eq!(steps[0].apply.join(" "), "resolvectl dns fiend0 198.18.0.1");
        assert_eq!(steps[1].apply.last().unwrap(), "~.");
        assert_eq!(
            steps[0].undo.as_ref().unwrap().join(" "),
            "resolvectl revert fiend0"
        );
        let steps = dns_steps(
            "fiend0",
            "198.18.0.1",
            false,
            Path::new("/b"),
            Path::new("/r"),
        );
        assert_eq!(steps[0].apply[0], WRITE_RESOLV);
        assert_eq!(steps[0].undo.as_ref().unwrap()[0], RESTORE_FILE);
    }

    #[test]
    fn undo_is_saved_before_each_command_and_runs_newest_first() {
        let file = file("order");
        let mut journal = Journal::default();
        let mut runner = Recorder::default();
        let steps = route_steps("fiend0", 7);
        apply_steps(&steps, &mut journal, &file, &mut runner).unwrap();
        assert_eq!(journal.undo.len(), 6);
        assert_eq!(file.load().unwrap(), journal);
        runner.seen.clear();
        assert_eq!(undo_all(&journal, &mut runner), 0);
        assert_eq!(
            runner.seen.first().unwrap(),
            "ip -6 rule del table main suppress_prefixlength 0"
        );
        assert_eq!(
            runner.seen.last().unwrap(),
            "ip -4 route del default dev fiend0 table 7"
        );
    }

    #[test]
    fn a_failed_command_keeps_its_undo_so_a_crash_can_clean_up() {
        let file = file("fail");
        let mut journal = Journal::default();
        let mut runner = Recorder {
            fail_on: Some("-6 route add".into()),
            ..Recorder::default()
        };
        assert!(apply_steps(&route_steps("fiend0", 7), &mut journal, &file, &mut runner).is_err());
        assert_eq!(journal.undo.len(), 4);
        assert_eq!(file.load().unwrap().undo.len(), 4);
    }

    #[test]
    fn undo_continues_after_a_failure() {
        let journal = Journal {
            undo: vec![vec!["a".into()], vec!["b".into()], vec!["c".into()]],
            ..Journal::default()
        };
        let mut runner = Recorder {
            fail_on: Some("b".into()),
            ..Recorder::default()
        };
        assert_eq!(undo_all(&journal, &mut runner), 1);
        assert_eq!(runner.seen, ["c", "b", "a"]);
    }

    #[test]
    fn resolver_file_is_backed_up_written_and_restored() {
        let dir = std::env::temp_dir().join(format!("fiend-resolv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (resolv, backup) = (dir.join("resolv.conf"), dir.join("backup"));
        std::fs::write(&resolv, "nameserver 192.168.1.1\n").unwrap();
        let steps = dns_steps("fiend0", "198.18.0.1", false, &backup, &resolv);
        let file = JournalFile::new(dir.join("j.json"));
        let mut journal = Journal::default();
        apply_steps(&steps, &mut journal, &file, &mut SystemRunner).unwrap();
        assert!(
            std::fs::read_to_string(&resolv)
                .unwrap()
                .contains("198.18.0.1")
        );
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "nameserver 192.168.1.1\n"
        );
        assert_eq!(undo_all(&journal, &mut SystemRunner), 0);
        assert_eq!(
            std::fs::read_to_string(&resolv).unwrap(),
            "nameserver 192.168.1.1\n"
        );
        assert!(!backup.exists());
    }
}
