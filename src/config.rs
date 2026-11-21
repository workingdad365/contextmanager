//! 앱 설정 저장/로드 (%LOCALAPPDATA%\ContextManager\config.json)

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Language;

pub const APP_NAME: &str = "ContextManager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: Language,
    pub skipped_version: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: Language::En,
            skipped_version: None,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.join(APP_NAME).join("config.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("failed to create config dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize config: {e}"))?;
        fs::write(path, json).map_err(|e| format!("failed to write config: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_language_is_english_and_roundtrips() {
        let cfg = Config::default();
        assert_eq!(cfg.language, Language::En);
        assert_eq!(cfg.skipped_version, None);

        let tmp = std::env::temp_dir().join(format!("cm_cfg_test_{}.json", std::process::id()));
        let mut custom = Config {
            language: Language::Ko,
            skipped_version: Some("v1.0.1".into()),
        };
        custom.save_to(&tmp).unwrap();
        let loaded = Config::load_from(&tmp);
        assert_eq!(loaded, custom);

        custom.language = Language::Ja;
        custom.save_to(&tmp).unwrap();
        assert_eq!(Config::load_from(&tmp).language, Language::Ja);
        let _ = fs::remove_file(&tmp);
    }
}
