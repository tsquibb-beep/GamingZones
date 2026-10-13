//! WinEvent hooks (`SetWinEventHook`, out of context).
//!
//! Out-of-context callbacks run on the thread that installed the hook, while it waits in
//! GetMessage, and only when a matching event happens: nothing runs while nothing moves.
//! The callback does no work itself. It filters the event and posts a message to a window,
//! because a callback can be re-entered and must not call back into the app.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, CHILDID_SELF, OBJID_CURSOR, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::Hwnd;

/// Event ids (WinUser.h).
pub mod event {
    use windows::Win32::UI::WindowsAndMessaging as wm;
    pub const MOVESIZESTART: u32 = wm::EVENT_SYSTEM_MOVESIZESTART;
    pub const MOVESIZEEND: u32 = wm::EVENT_SYSTEM_MOVESIZEEND;
    pub const OBJECT_DESTROY: u32 = wm::EVENT_OBJECT_DESTROY;
    pub const OBJECT_LOCATIONCHANGE: u32 = wm::EVENT_OBJECT_LOCATIONCHANGE;
}

/// Which events are passed on.
#[derive(Clone, Copy, Debug)]
pub struct Filter {
    /// Window events (`OBJID_WINDOW`, the window itself) for this window only, or for any
    /// window if `None`.
    pub window: Option<Hwnd>,
    /// Also pass `OBJID_CURSOR` events (the mouse pointer moved), from anywhere.
    pub cursor: bool,
}

/// Where a hook delivers: `PostMessage(target, msg, event, hwnd)`.
#[derive(Clone, Copy, Debug)]
pub struct Delivery {
    pub target: Hwnd,
    pub msg: u32,
    /// Post at most one message until [`WinEventHook::rearm`] is called. Use this for
    /// high-rate events where only "something changed" matters.
    pub coalesce: bool,
}

struct Entry {
    hook: isize,
    filter: Filter,
    delivery: Delivery,
    /// A coalesced message is waiting in the queue.
    pending: Cell<bool>,
}

thread_local! {
    static HOOKS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

/// An installed hook; removed on drop. Must be dropped on the thread that created it.
pub struct WinEventHook {
    handle: HWINEVENTHOOK,
}

impl WinEventHook {
    /// Hooks `first..=last` events. `process` = 0 for all processes; events from our own
    /// process are always skipped.
    pub fn new(first: u32, last: u32, process: u32, filter: Filter, delivery: Delivery) -> crate::Result<Self> {
        let handle = unsafe {
            SetWinEventHook(
                first,
                last,
                None,
                Some(callback),
                process,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if handle.is_invalid() {
            return Err(crate::Error::from_thread());
        }
        HOOKS.with(|h| {
            h.borrow_mut().push(Entry { hook: handle.0 as isize, filter, delivery, pending: Cell::new(false) })
        });
        Ok(Self { handle })
    }

    /// Allows the next coalesced message to be posted. Call when handling the message.
    pub fn rearm(&self) {
        let key = self.handle.0 as isize;
        HOOKS.with(|h| {
            if let Some(e) = h.borrow().iter().find(|e| e.hook == key) {
                e.pending.set(false);
            }
        });
    }

    /// How many hooks this thread has installed (for the "nothing left after a drag" log).
    pub fn live_count() -> usize {
        HOOKS.with(|h| h.borrow().len())
    }
}

impl Drop for WinEventHook {
    fn drop(&mut self) {
        let key = self.handle.0 as isize;
        HOOKS.with(|h| h.borrow_mut().retain(|e| e.hook != key));
        unsafe {
            let _ = UnhookWinEvent(self.handle);
        }
    }
}

unsafe extern "system" fn callback(
    hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let key = hook.0 as isize;
    // try_borrow: if somehow re-entered while the list is being changed, drop the event.
    HOOKS.with(|h| {
        let Ok(hooks) = h.try_borrow() else { return };
        let Some(e) = hooks.iter().find(|e| e.hook == key) else { return };
        let is_window = id_object == OBJID_WINDOW.0 && id_child == CHILDID_SELF as i32;
        let wanted = (is_window && e.filter.window.is_none_or(|w| w.raw() == hwnd))
            || (e.filter.cursor && id_object == OBJID_CURSOR.0);
        if !wanted {
            return;
        }
        if e.delivery.coalesce {
            if e.pending.get() {
                return;
            }
            e.pending.set(true);
        }
        unsafe {
            let _ = PostMessageW(
                Some(e.delivery.target.raw()),
                e.delivery.msg,
                WPARAM(event as usize),
                LPARAM(hwnd.0 as isize),
            );
        }
    });
}

/// The window carried in a hook message's `lParam`.
pub fn message_window(lparam: isize) -> Option<Hwnd> {
    Hwnd::from_raw(HWND(lparam as *mut _))
}
