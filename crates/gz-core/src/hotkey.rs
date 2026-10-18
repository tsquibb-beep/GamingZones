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

    /// Win, Ctrl or Alt held. Shift alone doesn't make a hotkey: Shift+A types an "A".
    pub const fn usable(&self) -> bool {
        self.win || self.ctrl || self.alt
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
///
/// The punctuation names are what the key prints on a US keyboard; other layouts print
/// something else on the same key (VK_OEM_3 is ` in the US, ' in the UK). The settings
/// window shows the user's own layout (`gz_win::hotkey::display`). OEM8 and OEM102 have no
/// US key: on a UK keyboard they're ` and the \ next to left Shift.
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
    ("OEM8", 0xDF, &[]),
    ("OEM102", 0xE2, &[]),
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

/// Shortcuts Windows keeps for itself or that every app relies on, with what they do.
/// `RegisterHotKey` accepts some of them (Alt+Tab), and Win+L locks the screen whatever
/// an app does, so they're refused by name.
const WINDOWS_SHORTCUTS: [(Modifiers, u16, &str); 11] = {
    const fn m(win: bool, ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers { win, ctrl, alt, shift }
    }
    const TAB: u16 = 0x09;
    const ESC: u16 = 0x1B;
    [
        (m(true, false, false, false), 0x4C, "locks the screen"),
        (m(false, true, true, false), 0x2E, "the security screen"),
        (m(false, true, false, true), ESC, "opens Task Manager"),
        (m(false, true, false, false), ESC, "opens Start"),
        (m(false, false, true, false), TAB, "switches windows"),
        (m(false, false, true, true), TAB, "switches windows"),
        (m(false, true, true, false), TAB, "switches windows"),
        (m(true, false, false, false), TAB, "Task View"),
        (m(false, false, true, false), ESC, "switches windows"),
        (m(false, false, true, false), 0x73, "closes windows"),
        (m(false, false, true, false), 0x20, "opens the window menu"),
    ]
};

impl Hotkey {
    /// What Windows does with these keys, if they're one of its own shortcuts (see
    /// [`WINDOWS_SHORTCUTS`]). Taken shortcuts that `RegisterHotKey` reports aren't listed.
    pub fn windows_shortcut(&self) -> Option<&'static str> {
        WINDOWS_SHORTCUTS.iter().find(|(m, vk, _)| *m == self.modifiers && *vk == self.vk).map(|&(_, _, what)| what)
    }

    /// Whether it can be written in settings.json and read back (keys without a name, e.g.
    /// media keys, can't).
    pub fn has_name(&self) -> bool {
        self.to_string().parse::<Hotkey>().is_ok_and(|h| h == *self)
    }
}

/// What a key press during hotkey capture led to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    /// Still waiting (a modifier went down or up).
    Waiting,
    Done(Hotkey),
    /// Esc on its own: keep the old hotkey.
    Cancelled,
    /// A key without Win, Ctrl or Alt.
    NeedsModifier,
    /// One of Windows' own shortcuts: what it does (see [`Hotkey::windows_shortcut`]).
    WindowsShortcut(Hotkey, &'static str),
    /// A key that has no name (see [`Hotkey::has_name`]).
    Unusable,
}

/// Turns raw key presses (virtual keys, left/right modifiers as the low-level keyboard hook
/// reports them) into a hotkey: the modifiers held when the first other key goes down.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    /// Modifier keys (left/right variants) currently down.
    down: Vec<u16>,
}

impl Capture {
    /// The modifier a virtual key is, if any.
    fn modifier(vk: u16) -> Option<fn(&mut Modifiers)> {
        match vk {
            0x5B | 0x5C => Some(|m| m.win = true),          // LWIN, RWIN
            0x11 | 0xA2 | 0xA3 => Some(|m| m.ctrl = true),  // CONTROL, LCONTROL, RCONTROL
            0x12 | 0xA4 | 0xA5 => Some(|m| m.alt = true),   // MENU, LMENU, RMENU
            0x10 | 0xA0 | 0xA1 => Some(|m| m.shift = true), // SHIFT, LSHIFT, RSHIFT
            _ => None,
        }
    }

    /// The modifiers held right now.
    pub fn modifiers(&self) -> Modifiers {
        let mut m = Modifiers::default();
        for &vk in &self.down {
            if let Some(set) = Self::modifier(vk) {
                set(&mut m);
            }
        }
        m
    }

    pub fn key(&mut self, vk: u16, down: bool) -> Captured {
        if Self::modifier(vk).is_some() {
            self.down.retain(|&k| k != vk);
            if down {
                self.down.push(vk);
            }
            return Captured::Waiting;
        }
        if !down {
            return Captured::Waiting;
        }
        let modifiers = self.modifiers();
        if vk == 0x1B && !modifiers.any() {
            return Captured::Cancelled;
        }
        if !modifiers.usable() {
            return Captured::NeedsModifier;
        }
        let hotkey = modifiers.with(vk);
        if let Some(what) = hotkey.windows_shortcut() {
            Captured::WindowsShortcut(hotkey, what)
        } else if hotkey.has_name() {
            Captured::Done(hotkey)
        } else {
            Captured::Unusable
        }
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
        // Keys a US keyboard doesn't have (UK ` and \\).
        assert_eq!(hk("Win+OEM8").vk, 0xDF);
        assert_eq!(hk("Ctrl+OEM102").to_string(), "Ctrl+OEM102");
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
    fn capture() {
        let mut c = Capture::default();
        // Win down, Shift down, ` down: Win+Shift+`.
        assert_eq!(c.key(0x5B, true), Captured::Waiting);
        assert_eq!(c.key(0xA1, true), Captured::Waiting);
        assert_eq!(c.modifiers(), Modifiers { win: true, shift: true, ..Default::default() });
        assert_eq!(c.key(0xC0, true), Captured::Done(hk("Win+Shift+`")));

        // Left and right Ctrl: letting go of one keeps Ctrl held.
        let mut c = Capture::default();
        c.key(0xA2, true);
        c.key(0xA3, true);
        c.key(0xA2, false);
        assert_eq!(c.key(0x4B, true), Captured::Done(hk("Ctrl+K")));
        c.key(0xA3, false);
        assert_eq!(c.key(0x4B, true), Captured::NeedsModifier);
        assert_eq!(c.key(0x4B, false), Captured::Waiting);

        // Esc alone cancels; with modifiers it's a hotkey like any other.
        assert_eq!(c.key(0x1B, true), Captured::Cancelled);
        c.key(0xA4, true);
        c.key(0xA2, true);
        assert_eq!(c.key(0x1B, true), Captured::Done(hk("Ctrl+Alt+Esc")));
        c.key(0xA2, false);

        // Windows' own shortcuts are refused.
        assert_eq!(c.key(0x09, true), Captured::WindowsShortcut(hk("Alt+Tab"), "switches windows"));
        assert_eq!(c.key(0x73, true), Captured::WindowsShortcut(hk("Alt+F4"), "closes windows"));
        c.key(0xA4, false);
        c.key(0x5B, true);
        assert_eq!(c.key(0x4C, true), Captured::WindowsShortcut(hk("Win+L"), "locks the screen"));
        c.key(0x5B, false);

        // Shift on its own isn't enough.
        c.key(0xA0, true);
        assert_eq!(c.key(0x41, true), Captured::NeedsModifier);
        c.key(0xA0, false);

        // A key with no name (VK_VOLUME_UP) can't be used.
        c.key(0xA2, true);
        assert_eq!(c.key(0xAF, true), Captured::Unusable);
        assert!(hk("Win+Ctrl+Alt+F13").has_name());
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
