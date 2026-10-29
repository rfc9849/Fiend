use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fiend_ipc::{Profile, ProfileInput, Settings};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct ProfileFile {
    profiles: Vec<Profile>,
}

pub struct Store {
    dir: PathBuf,
    profiles: Vec<Profile>,
    settings: Settings,
}

pub fn new_id() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> io::Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", path.display()),
            )
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

pub fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        create_private_dir(&dir)?;
        create_private_dir(&dir.join("rules"))?;
        let profiles: ProfileFile = read_json(&dir.join("profiles.json"))?;
        let settings: Settings = read_json(&dir.join("settings.json"))?;
        Ok(Self {
            dir,
            profiles: profiles.profiles,
            settings,
        })
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn rule_path(&self, id: &str) -> PathBuf {
        self.dir.join("rules").join(format!("{id}.bin"))
    }

    fn save_profiles(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&ProfileFile {
            profiles: self.profiles.clone(),
        })?;
        write_private(&self.dir.join("profiles.json"), &bytes)
    }

    pub fn save_settings(&self) -> io::Result<()> {
        write_private(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&self.settings)?,
        )
    }

    pub fn upsert_profile(&mut self, input: ProfileInput) -> io::Result<Option<Profile>> {
        let id = match input.id {
            Some(id) => {
                if self.profile(&id).is_none() {
                    return Ok(None);
                }
                id
            }
            None => new_id(),
        };
        let profile = Profile {
            id: id.clone(),
            name: input.name,
            server: input.server.trim().to_owned(),
            secret: input.secret.to_ascii_lowercase(),
            template_json: input.template_json,
        };
        let previous = self.profiles.clone();
        match self.profiles.iter_mut().find(|item| item.id == id) {
            Some(item) => *item = profile.clone(),
            None => self.profiles.push(profile.clone()),
        }
        if let Err(error) = self.save_profiles() {
            self.profiles = previous;
            return Err(error);
        }
        Ok(Some(profile))
    }

    pub fn delete_profile(&mut self, id: &str) -> io::Result<bool> {
        let previous = self.profiles.clone();
        self.profiles.retain(|profile| profile.id != id);
        if self.profiles.len() == previous.len() {
            return Ok(false);
        }
        if let Err(error) = self.save_profiles() {
            self.profiles = previous;
            return Err(error);
        }
        Ok(true)
    }

    pub fn update_settings<R>(&mut self, change: impl FnOnce(&mut Settings) -> R) -> io::Result<R> {
        let previous = self.settings.clone();
        let result = change(&mut self.settings);
        if let Err(error) = self.save_settings() {
            self.settings = previous;
            return Err(error);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fiend-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn input(name: &str) -> ProfileInput {
        ProfileInput {
            id: None,
            name: name.into(),
            server: " 127.0.0.1:443 ".into(),
            secret: "AB".repeat(32),
            template_json: "{}".into(),
        }
    }

    #[test]
    fn data_survives_a_restart() {
        let dir = directory("restart");
        let mut store = Store::open(&dir).unwrap();
        let profile = store.upsert_profile(input("One")).unwrap().unwrap();
        assert_eq!(profile.server, "127.0.0.1:443");
        assert_eq!(profile.secret, "ab".repeat(32));
        store
            .update_settings(|settings| settings.active_profile = Some(profile.id.clone()))
            .unwrap();
        let reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.profiles(), std::slice::from_ref(&profile));
        assert_eq!(reopened.settings().active_profile, Some(profile.id));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_and_delete_work_by_id() {
        let dir = directory("update");
        let mut store = Store::open(&dir).unwrap();
        let first = store.upsert_profile(input("One")).unwrap().unwrap();
        let mut change = input("Renamed");
        change.id = Some(first.id.clone());
        let updated = store.upsert_profile(change).unwrap().unwrap();
        assert_eq!(updated.id, first.id);
        assert_eq!(store.profiles().len(), 1);
        assert_eq!(store.profiles()[0].name, "Renamed");
        let mut missing = input("Missing");
        missing.id = Some("nope".into());
        assert!(store.upsert_profile(missing).unwrap().is_none());
        assert!(store.delete_profile(&first.id).unwrap());
        assert!(!store.delete_profile(&first.id).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = directory("mode");
        let mut store = Store::open(&dir).unwrap();
        store.upsert_profile(input("One")).unwrap();
        store.save_settings().unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("profiles.json")), 0o600);
        assert_eq!(mode(&dir.join("settings.json")), 0o600);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_files_are_an_error_not_a_reset() {
        let dir = directory("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("profiles.json"), b"{broken").unwrap();
        assert!(Store::open(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ids_are_unique_hex() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
