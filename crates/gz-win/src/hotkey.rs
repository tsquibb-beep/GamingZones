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
