//! Talking to the tray app (if it's running): reload after saving (with the hotkeys it
//! couldn't register, for the settings window), and the on-screen preview.

use gz_core::ipc::{self, Preview, MAIN_WINDOW_CLASS, RELOAD_MESSAGE};
use gz_win::{inspect, ipc as win_ipc, process, window, Hwnd};

fn core_window() -> Option<Hwnd> {
    win_ipc::find_window(MAIN_WINDOW_CLASS)
}

/// Asks the tray app to re-read its files. `false` if it isn't running.
pub fn reload() -> bool {
    let Some(core) = core_window() else { return false };
    let msg = window::register_window_message(RELOAD_MESSAGE);
    msg != 0 && window::post_message(core, msg, 0, 0).is_ok()
}

/// Asks the tray app to re-read its files after a settings change, and returns the hotkey
/// settings it couldn't register ([`gz_core::HotkeyName::bit`]s). `None` if it isn't
/// running or didn't answer.
pub fn reload_settings() -> Option<u32> {
    let core = core_window()?;
    let msg = window::register_window_message(RELOAD_MESSAGE);
    if msg == 0 {
        return None;
    }
    let reply = win_ipc::send_message(core, msg, ipc::RELOAD_QUIET, 0, 5000).ok()?;
    // An older tray app (before Phase 4) answers 0.
    (reply & !0xFFFF == ipc::RELOAD_DONE).then_some((reply & 0xFFFF) as u32)
}

/// Shows the preview (`Some`) or hides it (`None`). `false` if the tray app isn't running
/// or didn't take the message.
pub fn preview(editor: Hwnd, preview: Option<&Preview>) -> bool {
    let Some(core) = core_window() else { return false };
    let sent = match preview {
        Some(p) => {
            let json = serde_json::to_vec(p).expect("preview always serialises");
            win_ipc::send_data(core, Some(editor), ipc::PREVIEW_SHOW, &json)
        }
        None => win_ipc::send_data(core, Some(editor), ipc::PREVIEW_HIDE, &[]),
    };
    sent.is_ok()
}

/// This process's main (visible, top-level) window.
pub fn own_window() -> Option<Hwnd> {
    let me = std::process::id();
    inspect::top_level_windows().into_iter().find(|&w| process::window_pid(w) == me && inspect::is_visible(w))
}

/// The window titled `title` of this exe in another process (the editor or the settings
/// window already open).
pub fn other_window(title: &str) -> Option<Hwnd> {
    let me = std::process::id();
    let exe = std::env::current_exe().ok()?;
    let exe = exe.to_string_lossy().to_lowercase();
    inspect::top_level_windows().into_iter().find(|&w| {
        let pid = process::window_pid(w);
        pid != me
            && inspect::is_visible(w)
            && window::text(w) == title
            && process::exe_path(pid).is_ok_and(|p| p.to_lowercase() == exe)
    })
}
