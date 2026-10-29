use std::fs;
use std::io::Write;
use std::path::Path;

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};

use fiend_ipc::BackupData;

use crate::prefs::Prefs;

pub const FORMAT: &str = "fiend-backup";
pub const VERSION: u32 = 1;
pub const MAX_FILE_BYTES: usize = 24 << 20;
const MAX_PLAIN_BYTES: usize = 16 << 20;
const MIN_PASSWORD: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub version: u32,
    pub created_at_ms: u64,
    pub data: BackupData,
    pub prefs: Prefs,
}

pub fn new_backup(data: BackupData, prefs: Prefs) -> BackupFile {
    BackupFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        created_at_ms: crate::format::now_ms(),
        data,
        prefs,
    }
}

pub fn check_password(password: &str) -> Result<(), String> {
    if password.len() < MIN_PASSWORD {
        return Err(format!("Use at least {MIN_PASSWORD} characters."));
    }
    Ok(())
}

pub fn encode(file: &BackupFile, password: &str) -> Result<Vec<u8>, String> {
    check_password(password)?;
    let plain = serde_json::to_vec(file).map_err(|_| "Cannot create the backup.".to_owned())?;
    if plain.len() > MAX_PLAIN_BYTES {
        return Err("The backup is too large.".to_owned());
    }
    let passphrase = SecretString::from(password.to_owned());
    let recipient = age::scrypt::Recipient::new(passphrase);
    age::encrypt(&recipient, &plain).map_err(|_| "Cannot create the backup.".to_owned())
}

pub fn decode(bytes: &[u8], password: &str) -> Result<BackupFile, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("The backup file is too large.".to_owned());
    }
    let passphrase = SecretString::from(password.to_owned());
    let identity = age::scrypt::Identity::new(passphrase);
    let plain = age::decrypt(&identity, bytes)
        .map_err(|_| "The password is incorrect or the backup is damaged.".to_owned())?;
    if plain.len() > MAX_PLAIN_BYTES {
        return Err("The backup file is too large.".to_owned());
    }
    let file: BackupFile =
        serde_json::from_slice(&plain).map_err(|_| "The backup file is damaged.".to_owned())?;
    if file.format != FORMAT {
        return Err("This is not a Fiend backup.".to_owned());
    }
    if file.version != VERSION {
        return Err(format!(
            "This backup uses version {}, but this Fiend supports version {VERSION}.",
            file.version
        ));
    }
    Ok(file)
}

pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("Cannot write the backup: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("Cannot write the backup: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Cannot write the backup: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Cannot protect the backup: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiend_ipc::{Profile, Settings};

    fn backup() -> BackupFile {
        new_backup(
            BackupData {
                profiles: vec![Profile {
                    id: "ab".repeat(16),
                    name: "Home".into(),
                    server: "127.0.0.1:443".into(),
                    secret: "0123456789abcdef".repeat(4),
                    template_json: "{}".into(),
                    tuning: Default::default(),
                }],
                settings: Settings {
                    active_profile: Some("ab".repeat(16)),
                    direct_rules: "domain:example.com\n".into(),
                    ..Settings::default()
                },
            },
            Prefs::default(),
        )
    }

    #[test]
    fn round_trip_keeps_profiles_settings_and_prefs() {
        let file = backup();
        let encrypted = encode(&file, "correct horse").unwrap();
        assert!(!String::from_utf8_lossy(&encrypted).contains("0123456789abcdef"));
        let decoded = decode(&encrypted, "correct horse").unwrap();
        assert_eq!(decoded, file);
    }

    #[test]
    fn wrong_password_is_refused() {
        let encrypted = encode(&backup(), "correct horse").unwrap();
        assert!(decode(&encrypted, "wrong password").is_err());
    }

    #[test]
    fn damaged_file_is_refused() {
        let mut encrypted = encode(&backup(), "correct horse").unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 1;
        assert!(decode(&encrypted, "correct horse").is_err());
    }

    #[test]
    fn plain_json_is_not_a_backup() {
        let plain = serde_json::to_vec(&backup()).unwrap();
        assert!(decode(&plain, "correct horse").is_err());
    }

    #[test]
    fn short_passwords_are_refused() {
        assert!(check_password("short").is_err());
        assert!(check_password("long enough").is_ok());
    }
}
