//! Talking to the tray app (if it's running): reload after saving (with the hotkeys it
//! couldn't register, for the settings window), hotkey capture, and the on-screen preview.

use std::cell::RefCell;
use std::rc::Rc;

use gz_core::ipc::{self, Preview, CAPTURE_KEY_MESSAGE, CAPTURE_MESSAGE, MAIN_WINDOW_CLASS, RELOAD_MESSAGE};
use gz_win::window::{Background, Message, WindowHandler, WindowSpec};
use gz_win::{inspect, ipc as win_ipc, process, styles::ws, window, Hwnd};

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

/// A hidden window that receives the keys the tray app captures for a hotkey box. It's
/// on the UI thread, so eframe's message loop dispatches its messages.
pub struct KeyReceiver {
    hwnd: Hwnd,
    keys: Rc<RefCell<Vec<(u16, bool)>>>,
}

struct KeyHandler {
    key_msg: u32,
    keys: Rc<RefCell<Vec<(u16, bool)>>>,
    /// Wakes the UI up.
    wake: Box<dyn Fn()>,
}

impl WindowHandler for KeyHandler {
    fn handle(&mut self, _hwnd: Hwnd, m: &Message) -> Option<isize> {
        if m.msg == 0 || m.msg != self.key_msg {
            return None;
        }
        self.keys.borrow_mut().push((m.wparam as u16, m.lparam != 0));
        (self.wake)();
        Some(0)
    }
}

impl KeyReceiver {
    pub fn new(wake: impl Fn() + 'static) -> gz_win::Result<KeyReceiver> {
        let keys = Rc::new(RefCell::new(Vec::new()));
        let key_msg = window::register_window_message(CAPTURE_KEY_MESSAGE);
        let handler = KeyHandler { key_msg, keys: keys.clone(), wake: Box::new(wake) };
        let spec = WindowSpec {
            class: "GamingZones.SettingsKeys",
            title: "",
            style: ws::POPUP,
            ex_style: ws::EX_TOOLWINDOW,
            rect: Some(gz_core::Rect::default()),
            icon: false,
            background: Background::None,
        };
        let hwnd = window::create_window(&spec, Box::new(handler))?;
        // The tray app may run elevated: let its key messages through.
        let _ = win_ipc::allow_message(hwnd, key_msg);
        Ok(KeyReceiver { hwnd, keys })
    }

    /// The keys received since the last call: (virtual key, down).
    pub fn take(&self) -> Vec<(u16, bool)> {
        std::mem::take(&mut self.keys.borrow_mut())
    }
}

impl Drop for KeyReceiver {
    fn drop(&mut self) {
        let _ = window::destroy_window(self.hwnd);
    }
}

/// A hotkey capture running in the tray app; stopped when dropped.
pub struct TrayCapture {
    core: Hwnd,
    msg: u32,
}

/// Asks the tray app to capture keys for `receiver`. `None` if it isn't running (or is
/// too old to know how).
pub fn start_capture(receiver: &KeyReceiver) -> Option<TrayCapture> {
    let core = core_window()?;
    let msg = window::register_window_message(CAPTURE_MESSAGE);
    if msg == 0 {
        return None;
    }
    let reply = win_ipc::send_message(core, msg, ipc::CAPTURE_START, receiver.hwnd.as_isize(), 2000).ok()?;
    (reply == ipc::CAPTURE_STARTED).then_some(TrayCapture { core, msg })
}

impl Drop for TrayCapture {
    fn drop(&mut self) {
        let _ = window::post_message(self.core, self.msg, ipc::CAPTURE_STOP, 0);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the tray app running (so it's ignored by default): it holds the keyboard for
    /// a moment, then lets go.
    #[test]
    #[ignore]
    fn the_tray_app_captures_for_us() {
        let receiver = KeyReceiver::new(|| ()).expect("receiver window");
        let capture = start_capture(&receiver).expect("the tray app started capturing");
        drop(capture);
        // A second one replaces nothing and stops cleanly too.
        drop(start_capture(&receiver).expect("again"));
    }
}
