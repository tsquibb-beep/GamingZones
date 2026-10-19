//! The borderless toggle's style maths and its record file. Window style bits are plain
//! numbers here (the values from WinUser.h); the tray app makes the Win32 calls.

use serde::{Deserialize, Serialize};

use crate::config::without_bom;
use crate::Rect;

pub const BORDERLESS_FILE: &str = "borderless.json";

/// `WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`.
pub const BORDER_STYLE: u32 = 0x00C0_0000 | 0x0004_0000 | 0x0008_0000 | 0x0002_0000 | 0x0001_0000;
/// `WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE`.
pub const BORDER_EX_STYLE: u32 = 0x0000_0001 | 0x0000_0100 | 0x0000_0200 | 0x0002_0000;
const EX_TOPMOST: u32 = 0x0000_0008;

/// A window's `GWL_STYLE` and `GWL_EXSTYLE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Styles {
    pub style: u32,
    pub ex_style: u32,
}

impl Styles {
    /// Any of the bits the toggle removes is set.
    pub fn has_border(self) -> bool {
        self.style & BORDER_STYLE != 0 || self.ex_style & BORDER_EX_STYLE != 0
    }

    /// The same styles without the border bits.
    pub fn without_border(self) -> Styles {
        Styles { style: self.style & !BORDER_STYLE, ex_style: self.ex_style & !BORDER_EX_STYLE }
    }

    /// `self` (saved before the border was removed) to put back now. `WS_EX_TOPMOST` stays
    /// as it is in `current`: it only changes through SetWindowPos, and keep-on-top owns it.
    pub fn restored_over(self, current: Styles) -> Styles {
        Styles { style: self.style, ex_style: (self.ex_style & !EX_TOPMOST) | (current.ex_style & EX_TOPMOST) }
    }
}

/// A window whose border GamingZones removed, kept in borderless.json so the border can be
/// put back after the tray app restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The window handle as a number.
    pub hwnd: isize,
    pub pid: u32,
    pub exe: String,
    /// The styles before the border was removed.
    pub saved: Styles,
    /// The window rect and client rect just before, so the border goes back the same size.
    pub window: Rect,
    pub client: Rect,
    /// Where the borderless window belongs: its rect when the border was removed, or when
    /// it was last snapped. A border the game puts back is removed again around this.
    pub place: Rect,
}

pub fn records_from_json(json: &str) -> Result<Vec<Record>, serde_json::Error> {
    serde_json::from_str(without_bom(json))
}

pub fn records_to_json(records: &[Record]) -> String {
    serde_json::to_string_pretty(records).expect("records always serialise")
}

/// Borderless Gaming's failsafe: a game that keeps putting its border back is left alone
/// rather than fought. At most [`Reapplies::MAX`] re-applies within [`Reapplies::WINDOW_MS`].
#[derive(Clone, Debug, Default)]
pub struct Reapplies {
    times: Vec<u64>,
}

impl Reapplies {
    pub const MAX: usize = 3;
    pub const WINDOW_MS: u64 = 10_000;

    /// Counts a re-apply at `now_ms` (any monotonic clock). `false`: there were already
    /// [`Self::MAX`] in the last [`Self::WINDOW_MS`], so give up instead.
    pub fn allow(&mut self, now_ms: u64) -> bool {
        self.times.retain(|&t| now_ms.saturating_sub(t) < Self::WINDOW_MS);
        if self.times.len() >= Self::MAX {
            return false;
        }
        self.times.push(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPSIBLINGS, WS_EX_WINDOWEDGE | WS_EX_APPWINDOW.
    const NORMAL: Styles = Styles { style: 0x16CF_0000, ex_style: 0x0004_0100 };

    #[test]
    fn strips_only_the_border() {
        let s = NORMAL.without_border();
        assert_eq!(s, Styles { style: 0x1600_0000, ex_style: 0x0004_0000 });
        assert!(NORMAL.has_border() && !s.has_border());
        // A popup with a thin border (WS_POPUP | WS_BORDER | WS_VISIBLE) counts as bordered.
        let popup = Styles { style: 0x9080_0000, ex_style: 0 };
        assert!(popup.has_border());
        assert_eq!(popup.without_border().style, 0x9000_0000);
        // Only an edge ex-style left over still counts.
        assert!(Styles { style: 0x1600_0000, ex_style: 0x200 }.has_border());
    }

    #[test]
    fn restore_keeps_the_current_topmost_bit() {
        let now = Styles { style: 0x1600_0000, ex_style: 0x0004_0008 };
        assert_eq!(NORMAL.restored_over(now), Styles { style: NORMAL.style, ex_style: 0x0004_0108 });
        let saved_topmost = Styles { ex_style: NORMAL.ex_style | EX_TOPMOST, ..NORMAL };
        assert_eq!(saved_topmost.restored_over(NORMAL.without_border()).ex_style, NORMAL.ex_style);
    }

    #[test]
    fn failsafe_allows_three_in_ten_seconds() {
        let mut r = Reapplies::default();
        assert!(r.allow(0) && r.allow(1_000) && r.allow(2_000));
        assert!(!r.allow(3_000));
        assert!(!r.allow(9_999));
        // The first one is more than 10 s old now.
        assert!(r.allow(10_000));
    }

    #[test]
    fn records_round_trip() {
        let client = Rect::new(108, 138, 2668, 1578);
        let records = vec![Record {
            hwnd: 0x1234,
            pid: 42,
            exe: "game.exe".into(),
            saved: NORMAL,
            window: Rect::new(100, 100, 2676, 1586),
            client,
            place: client,
        }];
        let json = records_to_json(&records);
        assert_eq!(records_from_json(&json).unwrap(), records);
        assert_eq!(records_from_json(&format!("\u{feff}{json}")).unwrap(), records);
        assert!(records_from_json("{}").is_err());
    }
}
