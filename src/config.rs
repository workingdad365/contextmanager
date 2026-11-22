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
    pub only_orphaned: bool,
    pub only_disabled: bool,
    pub hide_disabled: bool,
    pub hide_system: bool,
    pub flat_view: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: Language::En,
            skipped_version: None,
            only_orphaned: false,
            only_disabled: false,
            hide_disabled: false,
            hide_system: true,
            flat_view: false,
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
        let mut cfg: Self = serde_json::from_str(&text).unwrap_or_default();
        // 상호 배타 필터 충돌 방지
        if cfg.only_disabled && cfg.hide_disabled {
            cfg.only_disabled = false;
        }
        cfg
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
            only_orphaned: true,
            only_disabled: false,
            hide_disabled: true,
            hide_system: false,
            flat_view: true,
        };
        custom.save_to(&tmp).unwrap();
        let loaded = Config::load_from(&tmp);
        assert_eq!(loaded, custom);

        custom.language = Language::Ja;
        custom.save_to(&tmp).unwrap();
        assert_eq!(Config::load_from(&tmp).language, Language::Ja);

        // 상호 배타 옵션(only_disabled + hide_disabled) 동시 true 일 때 정리 검증
        fs::write(&tmp, r#"{"only_disabled":true,"hide_disabled":true}"#).unwrap();
        let norm = Config::load_from(&tmp);
        assert!(!norm.only_disabled);
        assert!(norm.hide_disabled);
        assert!(norm.hide_system);

        let _ = fs::remove_file(&tmp);
    }
}
