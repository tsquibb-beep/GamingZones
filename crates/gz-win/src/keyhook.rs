//! Low-level keyboard hooks (`WH_KEYBOARD_LL`), never persistent (PLAN §1):
//! - [`KeyboardHook`], **during a drag only** (tray app). It's created when a drag starts and
//!   dropped when it ends, and lets Shift/Ctrl/Esc take effect while the mouse is still.
//! - [`CaptureHook`], **while a hotkey box is capturing** (settings window). It takes every
//!   key press for itself, so Win+… combinations reach the box instead of Windows or the app
//!   that owns them.
//!
//! While a low-level hook is installed, every key press in the system waits for this
//! thread to run the callback, so the thread must keep pumping messages and never block.
//! `KeyboardHook`'s callback only posts a message and always passes keys on.

use std::cell::RefCell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_UP,
    WH_KEYBOARD_LL,
};

use crate::Hwnd;

/// Virtual-key codes the drag code watches. The hook reports left/right variants.
pub mod vk {
    pub const ESCAPE: u16 = 0x1B;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
}

struct State {
    target: Hwnd,
    msg: u32,
    keys: Vec<u16>,
    /// Watched keys currently down, so auto-repeat isn't reported.
    down: Vec<u16>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The installed hook; removed on drop. Only one can exist per thread.
pub struct KeyboardHook {
    handle: HHOOK,
}

impl KeyboardHook {
    /// Posts `msg` to `target` with `wParam` = virtual key and `lParam` = 1 (down) or 0 (up)
    /// whenever one of `keys` changes state.
    pub fn new(target: Hwnd, msg: u32, keys: &[u16]) -> crate::Result<Self> {
        if STATE.with(|s| s.borrow().is_some()) {
            return Err(crate::Error::new(
                windows::core::HRESULT(0x8000_000Eu32 as i32),
                "keyboard hook already installed",
            ));
        }
        let handle = unsafe {
            let module = GetModuleHandleW(None)?;
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), Some(module.into()), 0)?
        };
        STATE.with(|s| *s.borrow_mut() = Some(State { target, msg, keys: keys.to_vec(), down: Vec::new() }));
        Ok(Self { handle })
    }

    pub fn is_installed() -> bool {
        STATE.with(|s| s.borrow().is_some())
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.handle);
        }
        STATE.with(|s| *s.borrow_mut() = None);
    }
}

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lParam points to a KBDLLHOOKSTRUCT valid for this call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let key = info.vkCode as u16;
        let down = info.flags.0 & LLKHF_UP.0 == 0;
        STATE.with(|s| {
            let Ok(mut state) = s.try_borrow_mut() else { return };
            let Some(state) = state.as_mut() else { return };
            if !state.keys.contains(&key) {
                return;
            }
            let was_down = state.down.contains(&key);
            if down && was_down {
                return; // auto-repeat
            }
            // An up is always reported: the key may have been held before the hook existed.
            if down {
                state.down.push(key);
            } else {
                state.down.retain(|&k| k != key);
            }
            unsafe {
                let _ = PostMessageW(Some(state.target.raw()), state.msg, WPARAM(key as usize), LPARAM(down as isize));
            }
        });
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

struct CaptureState {
    /// Key presses (virtual key, down) not yet taken by [`CaptureHook::take`].
    events: Vec<(u16, bool)>,
    /// Keys that went down while capturing: only their ups are swallowed, so a key held
    /// before the hook existed is still released properly.
    down: Vec<u16>,
}

thread_local! {
    static CAPTURE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
}

/// Takes all keyboard input while it exists (one per thread); removed on drop. The thread
/// must keep pumping messages; read the keys with [`CaptureHook::take`].
pub struct CaptureHook {
    handle: HHOOK,
}

impl CaptureHook {
    pub fn new() -> crate::Result<Self> {
        if CAPTURE.with(|s| s.borrow().is_some()) {
            return Err(crate::Error::new(
                windows::core::HRESULT(0x8000_000Eu32 as i32),
                "capture hook already installed",
            ));
        }
        let handle = unsafe {
            let module = GetModuleHandleW(None)?;
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(capture_callback), Some(module.into()), 0)?
        };
        CAPTURE.with(|s| *s.borrow_mut() = Some(CaptureState { events: Vec::new(), down: Vec::new() }));
        Ok(Self { handle })
    }

    /// The key presses since the last call: (virtual key, down). Auto-repeats are included.
    pub fn take(&self) -> Vec<(u16, bool)> {
        CAPTURE.with(|s| s.borrow_mut().as_mut().map(|s| std::mem::take(&mut s.events)).unwrap_or_default())
    }
}

impl Drop for CaptureHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.handle);
        }
        CAPTURE.with(|s| *s.borrow_mut() = None);
    }
}

unsafe extern "system" fn capture_callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lParam points to a KBDLLHOOKSTRUCT valid for this call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let key = info.vkCode as u16;
        let down = info.flags.0 & LLKHF_UP.0 == 0;
        let swallow = CAPTURE.with(|s| {
            let Ok(mut state) = s.try_borrow_mut() else { return false };
            let Some(state) = state.as_mut() else { return false };
            if down {
                if !state.down.contains(&key) {
                    state.down.push(key);
                }
            } else if let Some(i) = state.down.iter().position(|&k| k == key) {
                state.down.remove(i);
            } else {
                return false; // went down before we started: let Windows see it go up
            }
            state.events.push((key, down));
            true
        });
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
