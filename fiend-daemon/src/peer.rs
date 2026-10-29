#[cfg(unix)]
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub gid: u32,
    pub members: Vec<String>,
}

pub trait Directory {
    fn console_uid(&self) -> Option<u32>;
    fn group(&self, name: &str) -> Option<Group>;
    fn username(&self, uid: u32) -> Option<String>;
}

pub fn parse_group(text: &str, name: &str) -> Option<Group> {
    text.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != name {
            return None;
        }
        let _password = fields.next()?;
        let gid = fields.next()?.parse().ok()?;
        let members = fields
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|member| !member.is_empty())
            .map(str::to_owned)
            .collect();
        Some(Group { gid, members })
    })
}

pub fn parse_username(text: &str, uid: u32) -> Option<String> {
    text.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        (fields.next()?.parse::<u32>().ok()? == uid).then(|| name.to_owned())
    })
}

pub struct SystemDirectory;

#[cfg(unix)]
impl Directory for SystemDirectory {
    fn console_uid(&self) -> Option<u32> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(Path::new("/dev/console"))
            .ok()
            .map(|metadata| metadata.uid())
    }

    fn group(&self, name: &str) -> Option<Group> {
        parse_group(&std::fs::read_to_string("/etc/group").ok()?, name)
    }

    fn username(&self, uid: u32) -> Option<String> {
        parse_username(&std::fs::read_to_string("/etc/passwd").ok()?, uid)
    }
}

#[cfg(not(unix))]
impl Directory for SystemDirectory {
    fn console_uid(&self) -> Option<u32> {
        None
    }

    fn group(&self, _: &str) -> Option<Group> {
        None
    }

    fn username(&self, _: u32) -> Option<String> {
        None
    }
}

/// Who may use the daemon. Root always may. The console user is the user
/// at the screen on macOS. A group is the `fiend` group on Linux.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerPolicy {
    pub uids: Vec<u32>,
    pub console: bool,
    pub group: Option<String>,
}

impl PeerPolicy {
    pub fn system_default() -> Self {
        Self {
            uids: Vec::new(),
            console: cfg!(target_os = "macos"),
            group: cfg!(not(target_os = "macos")).then(|| "fiend".to_owned()),
        }
    }

    pub fn allows(&self, uid: u32, gid: u32, directory: &dyn Directory) -> bool {
        if uid == 0 || self.uids.contains(&uid) {
            return true;
        }
        if self.console && directory.console_uid() == Some(uid) {
            return true;
        }
        if let Some(group) = self.group.as_deref().and_then(|name| directory.group(name)) {
            if group.gid == gid {
                return true;
            }
            if let Some(name) = directory.username(uid) {
                return group.members.contains(&name);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Directory for Fake {
        fn console_uid(&self) -> Option<u32> {
            Some(501)
        }
        fn group(&self, name: &str) -> Option<Group> {
            (name == "fiend").then(|| Group {
                gid: 900,
                members: vec!["alice".into()],
            })
        }
        fn username(&self, uid: u32) -> Option<String> {
            match uid {
                1000 => Some("alice".into()),
                1001 => Some("bob".into()),
                _ => None,
            }
        }
    }

    #[test]
    fn root_and_listed_uids_are_allowed() {
        let policy = PeerPolicy {
            uids: vec![1234],
            ..PeerPolicy::default()
        };
        assert!(policy.allows(0, 0, &Fake));
        assert!(policy.allows(1234, 5, &Fake));
        assert!(!policy.allows(1235, 5, &Fake));
    }

    #[test]
    fn console_user_is_allowed_only_when_enabled() {
        let on = PeerPolicy {
            console: true,
            ..PeerPolicy::default()
        };
        assert!(on.allows(501, 20, &Fake));
        assert!(!on.allows(502, 20, &Fake));
        assert!(!PeerPolicy::default().allows(501, 20, &Fake));
    }

    #[test]
    fn group_members_and_primary_group_are_allowed() {
        let policy = PeerPolicy {
            group: Some("fiend".into()),
            ..PeerPolicy::default()
        };
        assert!(policy.allows(1000, 1000, &Fake));
        assert!(policy.allows(4000, 900, &Fake));
        assert!(!policy.allows(1001, 1001, &Fake));
        assert!(!policy.allows(4000, 4000, &Fake));
    }

    #[test]
    fn missing_group_allows_nobody_extra() {
        let policy = PeerPolicy {
            group: Some("missing".into()),
            ..PeerPolicy::default()
        };
        assert!(!policy.allows(1000, 900, &Fake));
    }

    #[test]
    fn system_files_are_parsed() {
        let group = "root:x:0:\nfiend:x:900:alice,bob\n";
        assert_eq!(
            parse_group(group, "fiend"),
            Some(Group {
                gid: 900,
                members: vec!["alice".into(), "bob".into()]
            })
        );
        assert_eq!(parse_group(group, "root").unwrap().members.len(), 0);
        assert!(parse_group(group, "none").is_none());
        assert!(parse_group("bad line", "bad line").is_none());
        let passwd = "root:x:0:0::/root:/bin/sh\nalice:x:1000:1000::/h:/bin/sh\n";
        assert_eq!(parse_username(passwd, 1000).as_deref(), Some("alice"));
        assert_eq!(parse_username(passwd, 7), None);
    }
}
