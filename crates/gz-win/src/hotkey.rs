//! Global hotkeys via `RegisterHotKey`. Windows delivers `WM_HOTKEY` to the window; nothing
//! runs between presses.

use gz_core::Hotkey;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MAPVK_VK_TO_CHAR, MOD_ALT, MOD_CONTROL,
    MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
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

/// `hotkey` as the user's keyboard prints it. The punctuation keys show the character
/// they type on this thread's keyboard layout: VK_OEM_3 is "`" on a US keyboard and "'" on
/// a UK one, though settings.json calls it "`" either way.
pub fn display(hotkey: &Hotkey) -> String {
    let key = match hotkey.vk {
        0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 => layout_char(hotkey.vk),
        _ => None,
    };
    match key {
        Some(c) if hotkey.modifiers.any() => format!("{}+{c}", hotkey.modifiers),
        Some(c) => c.to_string(),
        None => hotkey.to_string(),
    }
}

/// The character `vk` types on this thread's keyboard layout (unshifted).
fn layout_char(vk: u16) -> Option<char> {
    // The top bit marks a dead key (an accent); it still shows its character.
    let code = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) } & 0xFFFF;
    char::from_u32(code).filter(|c| !c.is_control() && !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_the_keyboard_layout_for_punctuation() {
        let h = |s: &str| s.parse::<Hotkey>().unwrap();
        assert_eq!(display(&h("Win+Ctrl+Alt+G")), "Win+Ctrl+Alt+G");
        assert_eq!(display(&h("Ctrl+F5")), "Ctrl+F5");
        // Whatever the layout, a punctuation key shows as one character.
        let shown = display(&h("Win+Shift+`"));
        let key = shown.strip_prefix("Win+Shift+").unwrap();
        assert_eq!(key.chars().count(), 1, "{shown}");
    }

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
