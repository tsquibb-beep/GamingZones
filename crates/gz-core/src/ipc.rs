//! What the editor and the tray app say to each other, and where their files live.
//!
//! The editor saves the JSON files and sends [`RELOAD_MESSAGE`] to the tray app's window
//! (class [`MAIN_WINDOW_CLASS`]). For "Preview on screen" it sends a [`Preview`] as JSON in
//! a `WM_COPYDATA` message of kind [`PREVIEW_SHOW`], and [`PREVIEW_HIDE`] to take it away.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::layout::Layout;

/// Window class of the tray app's hidden window.
pub const MAIN_WINDOW_CLASS: &str = "GamingZones.Main";
/// Registered window message: re-read settings.json, layouts.json and applied.json.
pub const RELOAD_MESSAGE: &str = "GamingZones.Reload";
/// `wParam` of [`RELOAD_MESSAGE`] from the settings window: hotkeys that can't be
/// registered are reported in the reply, not as a tray notification.
pub const RELOAD_QUIET: usize = 1;
/// The tray app's reply to [`RELOAD_MESSAGE`]: this, plus [`crate::HotkeyName::bit`] for
/// each hotkey setting that couldn't be registered.
pub const RELOAD_DONE: isize = 0x475A_0000;
/// `WM_COPYDATA` kinds ("GZ" in the high bytes, so a stray message isn't mistaken for one).
pub const PREVIEW_SHOW: usize = 0x475A_0001;
pub const PREVIEW_HIDE: usize = 0x475A_0002;

/// Show `layout` on the monitor with stable id `monitor`, with `selected` zones highlighted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub monitor: String,
    pub layout: Layout,
    #[serde(default)]
    pub selected: Vec<usize>,
}

/// `%APPDATA%\GamingZones`, where settings and layouts live.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("GamingZones"))
}
