//! Low-level keyboard hooks (`WH_KEYBOARD_LL`), never persistent (PLAN §1):
//! - [`KeyboardHook`], **during a drag only** (tray app). It's created when a drag starts and
//!   dropped when it ends, and lets Shift/Ctrl/Esc take effect while the mouse is still.
//! - [`CaptureHook`], **while a hotkey box in the settings window is capturing**, held by the
//!   tray app on a thread of its own. It takes every key press for itself, so Win+…
//!   combinations reach the box instead of Windows or the app that owns them.
//!
//! While a low-level hook is installed, every key press in the system waits for this
//! thread to run the callback, so the thread must keep pumping messages and never block.
//! `KeyboardHook`'s callback only posts a message and always passes keys on.

use std::cell::RefCell;
use std::sync::mpsc;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostMessageW, PostQuitMessage, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_UP, MSG, PM_NOREMOVE, WH_KEYBOARD_LL,
    WM_APP, WM_TIMER,
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

/// The capture hook's state, on its own thread.
struct CaptureState {
    /// Called with each key press it takes: (virtual key, down).
    on_key: Box<dyn Fn(u16, bool) + Send>,
    /// Keys that went down while capturing: only their ups are swallowed, so a key held
    /// before the hook existed is still released properly.
    down: Vec<u16>,
    /// The owner is done: take no new keys, only the rest of the ones in `down`.
    finishing: bool,
}

thread_local! {
    static CAPTURE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
}

/// Posted to the hook's thread when the [`CaptureHook`] is dropped.
const WM_FINISH: u32 = WM_APP + 1;
/// How long a finishing hook waits for the keys still held to be released (ms).
const FINISH_TIMEOUT_MS: u32 = 1000;

/// Takes all keyboard input while it exists, so Win+… combinations reach the hotkey box
/// instead of Windows or the app that owns them.
///
/// Windows doesn't call a low-level hook while a window of the hook's own process is in
/// front, so it lives in the tray app, not in the settings window (seen 2026-10-18: no key
/// at all until another window was in front).
///
/// The hook runs on a thread of its own that only pumps messages: Windows waits a limited
/// time for a low-level hook to answer, then passes the key on. Dropping it stops taking
/// keys, but the keys still held are swallowed until they're released (at most a second),
/// or Windows would see the rest of the combination.
pub struct CaptureHook {
    thread_id: u32,
}

impl CaptureHook {
    /// `on_key(virtual key, down)` is called on the hook's thread for each key press it
    /// takes, auto-repeats included. It must be quick (post a message, say).
    pub fn new(on_key: impl Fn(u16, bool) + Send + 'static) -> crate::Result<Self> {
        let (ready, started) = mpsc::channel();
        let state = CaptureState { on_key: Box::new(on_key), down: Vec::new(), finishing: false };
        std::thread::Builder::new()
            .name("capture hook".into())
            .spawn(move || capture_thread(state, ready))
            .map_err(|e| crate::Error::new(windows::core::HRESULT(0x8000_4005u32 as i32), e.to_string()))?;
        let thread_id = started.recv().map_err(|_| {
            crate::Error::new(windows::core::HRESULT(0x8000_4005u32 as i32), "capture hook thread ended")
        })??;
        Ok(Self { thread_id })
    }
}

impl Drop for CaptureHook {
    fn drop(&mut self) {
        // The thread unhooks itself once the keys still held are up.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_FINISH, WPARAM(0), LPARAM(0));
        }
    }
}

fn capture_thread(state: CaptureState, ready: mpsc::Sender<crate::Result<u32>>) {
    let mut msg = MSG::default();
    let handle = unsafe {
        // Make the message queue now, so WM_FINISH can't be posted before it exists.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        GetModuleHandleW(None).and_then(|m| SetWindowsHookExW(WH_KEYBOARD_LL, Some(capture_callback), Some(m.into()), 0))
    };
    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    CAPTURE.with(|s| *s.borrow_mut() = Some(state));
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        match msg.message {
            WM_FINISH => {
                let all_up = CAPTURE.with(|s| {
                    let mut s = s.borrow_mut();
                    let state = s.as_mut().expect("set above");
                    state.finishing = true;
                    state.down.is_empty()
                });
                if all_up {
                    break;
                }
                unsafe { SetTimer(None, 0, FINISH_TIMEOUT_MS, None) };
            }
            WM_TIMER => break,
            _ => {}
        }
    }
    unsafe {
        let _ = UnhookWindowsHookEx(handle);
    }
    CAPTURE.with(|s| *s.borrow_mut() = None);
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
            let held = state.down.contains(&key);
            if state.finishing {
                // Only the keys still held from the capture (their repeats and their ups).
                if held && !down {
                    state.down.retain(|&k| k != key);
                    if state.down.is_empty() {
                        unsafe { PostQuitMessage(0) };
                    }
                }
                return held;
            }
            if down {
                if !held {
                    state.down.push(key);
                }
            } else if held {
                state.down.retain(|&k| k != key);
            } else {
                return false; // went down before we started: let Windows see it go up
            }
            (state.on_key)(key, down);
            true
        });
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
