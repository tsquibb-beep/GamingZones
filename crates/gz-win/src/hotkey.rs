//! Global hotkeys via `RegisterHotKey`. Windows delivers `WM_HOTKEY` to the window; nothing
//! runs between presses.

use gz_core::Hotkey;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};

use crate::Hwnd;

fn modifiers(hotkey: &Hotkey) -> HOT_KEY_MODIFIERS {
    let m = &hotkey.modifiers;
    let mut flags = MOD_NOREPEAT;
    for (on, flag) in [(m.win, MOD_WIN), (m.ctrl, MOD_CONTROL), (m.alt, MOD_ALT), (m.shift, MOD_SHIFT)] {
        if on {
            flags |= flag;
        }
    }
    flags
}

/// Registers `hotkey` as `id` for `hwnd`. Fails (ERROR_HOTKEY_ALREADY_REGISTERED) if another
/// app or Windows itself owns the combination.
pub fn register(hwnd: Hwnd, id: i32, hotkey: &Hotkey) -> crate::Result<()> {
    unsafe { RegisterHotKey(Some(hwnd.raw()), id, modifiers(hotkey), hotkey.vk as u32) }
}

pub fn unregister(hwnd: Hwnd, id: i32) -> crate::Result<()> {
    unsafe { UnregisterHotKey(Some(hwnd.raw()), id) }
}

/// Whether `hotkey` is free right now: registers it for this thread and lets it go again.
/// `false` if another app or Windows holds it (our own tray app counts as another app).
pub fn is_free(hotkey: &Hotkey) -> bool {
    const PROBE_ID: i32 = 0xBFFF;
    let free = unsafe { RegisterHotKey(None, PROBE_ID, modifiers(hotkey), hotkey.vk as u32) }.is_ok();
    if free {
        unsafe {
            let _ = UnregisterHotKey(None, PROBE_ID);
        }
    }
    free
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hotkey_held_elsewhere_is_not_free() {
        let hotkey: Hotkey = "Win+Ctrl+Alt+Shift+F22".parse().unwrap();
        assert!(is_free(&hotkey), "something else holds {hotkey}");
        // Held by another thread, as another app would.
        let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
        let thread = std::thread::spawn(move || {
            let ok = unsafe { RegisterHotKey(None, 1, modifiers(&hotkey), hotkey.vk as u32) }.is_ok();
            held.0.send(ok).unwrap();
            release.1.recv().unwrap();
            unsafe { UnregisterHotKey(None, 1) }.unwrap();
        });
        assert!(held.1.recv().unwrap());
        assert!(!is_free(&hotkey));
        release.0.send(()).unwrap();
        thread.join().unwrap();
        assert!(is_free(&hotkey));
    }
}
