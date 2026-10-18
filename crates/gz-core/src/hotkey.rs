//! Hotkeys as written in settings.json, e.g. `"Win+Ctrl+Alt+I"`.
//!
//! The key is stored as a Windows virtual-key code. These are plain numbers from the
//! Win32 docs; no Win32 API is called here.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Written as e.g. `"Win+Ctrl+Alt"` when stored on its own (the quick-switch keys).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Modifiers {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const fn any(&self) -> bool {
        self.win || self.ctrl || self.alt || self.shift
    }

    /// These modifiers with `vk`.
    pub const fn with(self, vk: u16) -> Hotkey {
        Hotkey { modifiers: self, vk }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hotkey {
    pub modifiers: Modifiers,
    /// Windows virtual-key code (`VK_*`).
    pub vk: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    Empty,
    UnknownKey(String),
    NoKey,
    TwoKeys(String, String),
    NoModifier,
    /// A key where only modifiers belong (the quick-switch modifiers).
    UnexpectedKey(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "hotkey is empty"),
            Self::UnknownKey(k) => write!(f, "unknown key \"{k}\""),
            Self::NoKey => write!(f, "hotkey has modifiers but no key"),
            Self::TwoKeys(a, b) => write!(f, "hotkey has two keys (\"{a}\" and \"{b}\")"),
            Self::NoModifier => write!(f, "hotkey needs at least one of Win, Ctrl, Alt, Shift"),
            Self::UnexpectedKey(k) => write!(f, "only Win, Ctrl, Alt and Shift belong here, not \"{k}\""),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// Named keys: (canonical name, VK code, other accepted spellings).
const NAMED_KEYS: &[(&str, u16, &[&str])] = &[
    ("`", 0xC0, &["Backtick", "Grave", "Tilde", "OEM3"]),
    ("-", 0xBD, &["Minus"]),
    ("=", 0xBB, &["Equals", "Plus"]),
    ("[", 0xDB, &["LBracket"]),
    ("]", 0xDD, &["RBracket"]),
    ("\\", 0xDC, &["Backslash"]),
    (";", 0xBA, &["Semicolon"]),
    ("'", 0xDE, &["Quote"]),
    (",", 0xBC, &["Comma"]),
    (".", 0xBE, &["Period"]),
    ("/", 0xBF, &["Slash"]),
    ("Space", 0x20, &[]),
    ("Tab", 0x09, &[]),
    ("Enter", 0x0D, &["Return"]),
    ("Esc", 0x1B, &["Escape"]),
    ("Backspace", 0x08, &[]),
    ("Pause", 0x13, &[]),
    ("Insert", 0x2D, &["Ins"]),
    ("Delete", 0x2E, &["Del"]),
    ("Home", 0x24, &[]),
    ("End", 0x23, &[]),
    ("PageUp", 0x21, &["PgUp"]),
    ("PageDown", 0x22, &["PgDn"]),
    ("Left", 0x25, &[]),
    ("Up", 0x26, &[]),
    ("Right", 0x27, &[]),
    ("Down", 0x28, &[]),
];

fn key_from_name(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    // A-Z and 0-9 map straight to their ASCII codes.
    if bytes.len() == 1 && (bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit()) {
        return Some(bytes[0] as u16);
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    if let Some(n) = upper.strip_prefix("NUMPAD").and_then(|n| n.parse::<u16>().ok()) {
        if n <= 9 {
            return Some(0x60 + n);
        }
    }
    NAMED_KEYS
        .iter()
        .find(|(canon, _, alts)| canon.eq_ignore_ascii_case(name) || alts.iter().any(|a| a.eq_ignore_ascii_case(name)))
        .map(|(_, vk, _)| *vk)
}

fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        _ => NAMED_KEYS
            .iter()
            .find(|(_, code, _)| *code == vk)
            .map(|(canon, _, _)| canon.to_string())
            .unwrap_or_else(|| format!("VK{vk:#04X}")),
    }
}

impl FromStr for Hotkey {
    type Err = HotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(HotkeyError::Empty);
        }
        let mut modifiers = Modifiers::default();
        let mut key: Option<(String, u16)> = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "win" | "windows" | "super" => modifiers.win = true,
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                _ => {
                    let vk = key_from_name(part).ok_or_else(|| HotkeyError::UnknownKey(part.into()))?;
                    if let Some((prev, _)) = key {
                        return Err(HotkeyError::TwoKeys(prev, part.into()));
                    }
                    key = Some((part.into(), vk));
                }
            }
        }
        let (_, vk) = key.ok_or(HotkeyError::NoKey)?;
        if !modifiers.any() {
            return Err(HotkeyError::NoModifier);
        }
        Ok(Hotkey { modifiers, vk })
    }
}

impl fmt::Display for Hotkey {
    /// Canonical order: Win+Ctrl+Alt+Shift+Key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.any() {
            write!(f, "{}+", self.modifiers)?;
        }
        f.write_str(&key_name(self.vk))
    }
}

impl FromStr for Modifiers {
    type Err = HotkeyError;

    /// Modifiers only, e.g. `"Win+Ctrl+Alt"`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(HotkeyError::Empty);
        }
        // Parse as a hotkey with a stand-in key, so a real key in `s` is reported as a second key.
        match format!("{s}+F1").parse::<Hotkey>() {
            Ok(h) => Ok(h.modifiers),
            Err(HotkeyError::TwoKeys(key, _)) => Err(HotkeyError::UnexpectedKey(key)),
            Err(e) => Err(e),
        }
    }
}

impl fmt::Display for Modifiers {
    /// Canonical order: Win+Ctrl+Alt+Shift.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [(self.win, "Win"), (self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift")];
        let on: Vec<&str> = names.iter().filter(|(on, _)| *on).map(|(_, name)| *name).collect();
        f.write_str(&on.join("+"))
    }
}

impl TryFrom<String> for Modifiers {
    type Error = HotkeyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Modifiers> for String {
    fn from(m: Modifiers) -> String {
        m.to_string()
    }
}

impl TryFrom<String> for Hotkey {
    type Error = HotkeyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Hotkey> for String {
    fn from(h: Hotkey) -> String {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        s.parse().unwrap()
    }

    #[test]
    fn parses_modifiers_and_letter() {
        let h = hk("Win+Ctrl+Alt+I");
        assert_eq!(h.modifiers, Modifiers { win: true, ctrl: true, alt: true, shift: false });
        assert_eq!(h.vk, 0x49);
    }

    #[test]
    fn case_whitespace_and_order_insensitive() {
        assert_eq!(hk(" alt + win+ ctrl +i "), hk("Win+Ctrl+Alt+I"));
    }

    #[test]
    fn canonical_round_trip() {
        for s in ["Win+Shift+`", "Win+Ctrl+Alt+1", "Ctrl+Alt+F12", "Win+Numpad5", "Alt+PageUp", "Win+Ctrl+Alt+B"] {
            assert_eq!(hk(s).to_string(), s);
        }
        assert_eq!(hk("win+shift+backtick").to_string(), "Win+Shift+`");
    }

    #[test]
    fn vk_codes() {
        assert_eq!(hk("Win+Shift+`").vk, 0xC0);
        assert_eq!(hk("Ctrl+F1").vk, 0x70);
        assert_eq!(hk("Ctrl+F24").vk, 0x87);
        assert_eq!(hk("Ctrl+9").vk, 0x39);
        assert_eq!(hk("Ctrl+Numpad0").vk, 0x60);
    }

    #[test]
    fn errors() {
        assert_eq!("".parse::<Hotkey>(), Err(HotkeyError::Empty));
        assert_eq!("Win+Alt".parse::<Hotkey>(), Err(HotkeyError::NoKey));
        assert_eq!("D".parse::<Hotkey>(), Err(HotkeyError::NoModifier));
        assert_eq!("Win+Banana".parse::<Hotkey>(), Err(HotkeyError::UnknownKey("Banana".into())));
        assert_eq!("Win+A+B".parse::<Hotkey>(), Err(HotkeyError::TwoKeys("A".into(), "B".into())));
        assert!("Ctrl+F25".parse::<Hotkey>().is_err());
    }

    #[test]
    fn modifiers_on_their_own() {
        let m: Modifiers = " ctrl+WIN + alt".parse().unwrap();
        assert_eq!(m, Modifiers { win: true, ctrl: true, alt: true, shift: false });
        assert_eq!(m.to_string(), "Win+Ctrl+Alt");
        assert_eq!(m.with(0x31).to_string(), "Win+Ctrl+Alt+1");
        assert_eq!("".parse::<Modifiers>(), Err(HotkeyError::Empty));
        assert_eq!("Win+A".parse::<Modifiers>(), Err(HotkeyError::UnexpectedKey("A".into())));
        assert_eq!("Win+Banana".parse::<Modifiers>(), Err(HotkeyError::UnknownKey("Banana".into())));
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"Win+Ctrl+Alt\"");
    }

    #[test]
    fn serde_as_string() {
        let h = hk("Win+Ctrl+Alt+I");
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, "\"Win+Ctrl+Alt+I\"");
        assert_eq!(serde_json::from_str::<Hotkey>(&json).unwrap(), h);
        assert!(serde_json::from_str::<Hotkey>("\"Win\"").is_err());
    }
}
