//! `settings.json`. Unknown fields are ignored and missing fields take their defaults,
//! so older and newer files both load.

use crate::hotkey::Hotkey;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub log_level: LogLevel,
    pub hotkeys: Hotkeys,
}

/// `None` (JSON `null`) disables a hotkey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Logs geometry and process details of the foreground window.
    pub debug_dump: Option<Hotkey>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self { debug_dump: Some(builtin("Win+Ctrl+Alt+I")) }
    }
}

/// For built-in defaults only; panics on a typo, which the tests catch.
fn builtin(s: &str) -> Hotkey {
    s.parse().expect("built-in hotkey must parse")
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Json(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// What [`Settings::load_or_create`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    Loaded,
    /// The file was missing; defaults were written to it.
    Created,
}

impl Settings {
    pub fn from_json(json: &str) -> Result<Settings, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings always serialise")
    }

    /// Loads `path`, or writes the defaults there if it doesn't exist.
    /// A file that exists but fails to parse is an error and is left untouched.
    pub fn load_or_create(path: &Path) -> Result<(Settings, LoadOutcome), ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok((Settings::from_json(&text).map_err(ConfigError::Json)?, LoadOutcome::Loaded)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let settings = Settings::default();
                std::fs::write(path, settings.to_json() + "\n").map_err(ConfigError::Io)?;
                Ok((settings, LoadOutcome::Created))
            }
            Err(e) => Err(ConfigError::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let s = Settings::default();
        assert_eq!(Settings::from_json(&s.to_json()).unwrap(), s);
        assert_eq!(s.hotkeys.debug_dump.unwrap().to_string(), "Win+Ctrl+Alt+I");
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_are_ignored() {
        let s = Settings::from_json(r#"{ "log_level": "debug", "future_thing": 1 }"#).unwrap();
        assert_eq!(s.log_level, LogLevel::Debug);
        assert_eq!(s.hotkeys, Hotkeys::default());
    }

    #[test]
    fn null_disables_hotkey() {
        let s = Settings::from_json(r#"{ "hotkeys": { "debug_dump": null } }"#).unwrap();
        assert_eq!(s.hotkeys.debug_dump, None);
    }

    #[test]
    fn bad_hotkey_or_level_is_an_error() {
        assert!(Settings::from_json(r#"{ "hotkeys": { "debug_dump": "Win+Nope" } }"#).is_err());
        assert!(Settings::from_json(r#"{ "log_level": "loud" }"#).is_err());
    }

    #[test]
    fn load_or_create_writes_defaults_once() {
        let dir = std::env::temp_dir().join(format!("gz-core-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS_FILE);
        let _ = std::fs::remove_file(&path);

        let (s, outcome) = Settings::load_or_create(&path).unwrap();
        assert_eq!((s, outcome), (Settings::default(), LoadOutcome::Created));

        std::fs::write(&path, r#"{ "log_level": "warn" }"#).unwrap();
        let (s, outcome) = Settings::load_or_create(&path).unwrap();
        assert_eq!((s.log_level, outcome), (LogLevel::Warn, LoadOutcome::Loaded));

        std::fs::write(&path, "{ broken").unwrap();
        assert!(matches!(Settings::load_or_create(&path), Err(ConfigError::Json(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
