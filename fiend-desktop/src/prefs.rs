use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::design::ThemeMode;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeMode,
    pub launch_at_login: bool,
    pub connect_on_open: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            launch_at_login: false,
            connect_on_open: false,
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Fiend"))
}

impl Prefs {
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join("desktop.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_missing_fields_use_defaults() {
        let prefs: Prefs = serde_json::from_str(r#"{"theme":"dark","extra":1}"#).unwrap();
        assert_eq!(prefs.theme, ThemeMode::Dark);
        assert!(!prefs.launch_at_login && !prefs.connect_on_open);
    }
}
