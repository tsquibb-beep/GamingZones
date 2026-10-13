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
    pub snapping: Snapping,
    pub overlay: Overlay,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapping {
    /// Dropping within `edge_span_radius_px` of a border shared by two zones snaps to both.
    pub edge_span: bool,
    pub edge_span_radius_px: i32,
    /// Exe file names (e.g. `"game.exe"`, any case) whose windows are never snapped.
    pub excluded_apps: Vec<String>,
}

impl Default for Snapping {
    fn default() -> Self {
        Self { edge_span: false, edge_span_radius_px: 20, excluded_apps: Vec::new() }
    }
}

impl Snapping {
    /// The radius for [`crate::layout::zones_at`]: `None` when edge spanning is off.
    pub fn edge_radius(&self) -> Option<i32> {
        self.edge_span.then_some(self.edge_span_radius_px.max(0))
    }

    pub fn is_excluded(&self, exe_name: &str) -> bool {
        self.excluded_apps.iter().any(|e| e.trim().eq_ignore_ascii_case(exe_name))
    }
}

/// Zone overlay colours (FancyZones' defaults). The whole overlay is drawn at `opacity`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub zone_color: Color,
    pub border_color: Color,
    pub highlight_color: Color,
    pub number_color: Color,
    pub show_numbers: bool,
    /// 0-100 %.
    pub opacity: u8,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            zone_color: Color::rgb(0xF5, 0xFC, 0xFF),
            border_color: Color::rgb(0xFF, 0xFF, 0xFF),
            highlight_color: Color::rgb(0x00, 0x8C, 0xFF),
            number_color: Color::rgb(0x00, 0x00, 0x00),
            show_numbers: true,
            opacity: 50,
        }
    }
}

/// An RGB colour, written as `"#RRGGBB"` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
        let bad = || format!("\"{s}\" is not a colour like \"#008CFF\"");
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        Ok(Color::rgb(byte(0)?, byte(2)?, byte(4)?))
    }
}

impl From<Color> for String {
    fn from(c: Color) -> String {
        c.to_string()
    }
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
    /// The file lacked settings added in a newer version; it was rewritten with them
    /// (at their defaults) so they can be found and edited.
    Upgraded,
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
            Ok(text) => {
                let settings = Settings::from_json(&text).map_err(ConfigError::Json)?;
                let full = settings.to_json();
                let on_disk: serde_json::Value = serde_json::from_str(&text).map_err(ConfigError::Json)?;
                let complete: serde_json::Value = serde_json::from_str(&full).map_err(ConfigError::Json)?;
                if has_all_keys(&on_disk, &complete) {
                    return Ok((settings, LoadOutcome::Loaded));
                }
                std::fs::write(path, full + "\n").map_err(ConfigError::Io)?;
                Ok((settings, LoadOutcome::Upgraded))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let settings = Settings::default();
                std::fs::write(path, settings.to_json() + "\n").map_err(ConfigError::Io)?;
                Ok((settings, LoadOutcome::Created))
            }
            Err(e) => Err(ConfigError::Io(e)),
        }
    }
}

/// Whether every object key in `complete` also exists in `actual` (recursively).
fn has_all_keys(actual: &serde_json::Value, complete: &serde_json::Value) -> bool {
    match (actual, complete) {
        (serde_json::Value::Object(a), serde_json::Value::Object(c)) => {
            c.iter().all(|(k, cv)| a.get(k).is_some_and(|av| has_all_keys(av, cv)))
        }
        _ => true,
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
    fn colours() {
        let c: Color = serde_json::from_str("\"#008cff\"").unwrap();
        assert_eq!(c, Color::rgb(0, 0x8C, 0xFF));
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"#008CFF\"");
        for bad in ["\"#08CFF\"", "\"#GG0000\"", "\"red\"", "\"#ÿÿÿ\""] {
            assert!(serde_json::from_str::<Color>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn snapping_helpers() {
        let mut s = Snapping { excluded_apps: vec![" Game.EXE ".into()], ..Default::default() };
        assert!(s.is_excluded("game.exe"));
        assert!(!s.is_excluded("other.exe"));
        assert_eq!(s.edge_radius(), None);
        s.edge_span = true;
        assert_eq!(s.edge_radius(), Some(20));
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

        let (_, outcome) = Settings::load_or_create(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);

        // A file from an older version gets the new keys, keeping its values.
        std::fs::write(&path, r#"{ "log_level": "warn" }"#).unwrap();
        let (s, outcome) = Settings::load_or_create(&path).unwrap();
        assert_eq!((s.log_level, outcome), (LogLevel::Warn, LoadOutcome::Upgraded));
        let (s, outcome) = Settings::load_or_create(&path).unwrap();
        assert_eq!((s.log_level, outcome), (LogLevel::Warn, LoadOutcome::Loaded));
        assert!(std::fs::read_to_string(&path).unwrap().contains("edge_span_radius_px"));

        std::fs::write(&path, "{ broken").unwrap();
        assert!(matches!(Settings::load_or_create(&path), Err(ConfigError::Json(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
