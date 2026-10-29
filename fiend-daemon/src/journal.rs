use std::fs;
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEntry {
    pub destination: IpAddr,
    pub prefix: u8,
    pub if_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsEntry {
    pub key: String,
    pub original: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    pub routes: Vec<RouteEntry>,
    #[serde(default)]
    pub dns: Vec<DnsEntry>,
}

impl Journal {
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty() && self.dns.is_empty()
    }

    pub fn add_route(&mut self, entry: RouteEntry) {
        if !self.routes.contains(&entry) {
            self.routes.push(entry);
        }
    }

    /// The first original of a key wins. A later entry would hold our own
    /// change.
    pub fn add_dns(&mut self, key: &str, original: Option<String>) -> bool {
        if self.dns.iter().any(|entry| entry.key == key) {
            return false;
        }
        self.dns.push(DnsEntry {
            key: key.to_owned(),
            original,
        });
        true
    }
}

/// Write-ahead record of network changes. The daemon writes an entry before
/// it makes the change, so a crash at any point leaves enough data to undo it.
#[derive(Clone, Debug)]
pub struct JournalFile {
    path: PathBuf,
}

impl JournalFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<Journal> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {error}", self.path.display()),
                )
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Journal::default()),
            Err(error) => Err(error),
        }
    }

    /// Set aside a journal that cannot be read, so it does not block every start.
    pub fn quarantine(&self) {
        let _ = fs::rename(&self.path, self.path.with_extension("bad"));
    }

    pub fn save(&self, journal: &Journal) -> io::Result<()> {
        use std::io::Write;
        let temporary = self.path.with_extension("tmp");
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(journal)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &self.path)
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> JournalFile {
        let dir = std::env::temp_dir().join(format!("fiend-journal-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        JournalFile::new(dir.join("journal.json"))
    }

    #[test]
    fn missing_file_is_an_empty_journal() {
        let file = file("missing");
        assert!(file.load().unwrap().is_empty());
        file.clear().unwrap();
    }

    #[test]
    fn journal_round_trips_and_clears() {
        let file = file("round");
        let mut journal = Journal::default();
        journal.add_route(RouteEntry {
            destination: "0.0.0.0".parse().unwrap(),
            prefix: 1,
            if_index: 17,
        });
        assert!(journal.add_dns("State:/Network/Service/A/DNS", Some("<plist/>".into())));
        file.save(&journal).unwrap();
        assert_eq!(file.load().unwrap(), journal);
        file.clear().unwrap();
        assert!(file.load().unwrap().is_empty());
    }

    #[test]
    fn first_dns_original_wins_and_routes_do_not_repeat() {
        let mut journal = Journal::default();
        assert!(journal.add_dns("k", Some("old".into())));
        assert!(!journal.add_dns("k", Some("ours".into())));
        assert_eq!(journal.dns[0].original.as_deref(), Some("old"));
        let route = RouteEntry {
            destination: "::".parse().unwrap(),
            prefix: 1,
            if_index: 3,
        };
        journal.add_route(route.clone());
        journal.add_route(route);
        assert_eq!(journal.routes.len(), 1);
    }

    #[test]
    fn corrupt_journal_is_quarantined() {
        let file = file("corrupt");
        fs::write(file.path(), b"{nope").unwrap();
        assert!(file.load().is_err());
        file.quarantine();
        assert!(file.load().unwrap().is_empty());
        assert!(file.path().with_extension("bad").exists());
    }

    #[cfg(unix)]
    #[test]
    fn journal_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let file = file("mode");
        file.save(&Journal::default()).unwrap();
        let mode = fs::metadata(file.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
