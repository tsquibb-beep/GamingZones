//! A low-level keyboard hook (`WH_KEYBOARD_LL`) for use **during a drag only**.
//!
//! PLAN §1 forbids persistent low-level hooks. This one is created when a drag starts and
//! dropped when it ends. It lets Shift/Ctrl/Esc take effect while the mouse is still.
//!
//! While a low-level hook is installed, every key press in the system waits for this
//! thread to run the callback, so the thread must keep pumping messages and never block.
//! The callback only posts a message; keys are always passed on, never swallowed.

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
