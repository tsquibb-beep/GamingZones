//! Window creation, a safe window-procedure trait, and the message loop.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt;

use gz_core::Rect;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{from_wide, wide};

/// A window handle. Plain data, so it is `Copy`, `Send` and hashable; whether the window
/// still exists is checked by each call that uses it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hwnd(isize);

impl Hwnd {
    pub(crate) fn raw(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }

    pub(crate) fn from_raw(h: HWND) -> Option<Hwnd> {
        (!h.is_invalid()).then_some(Hwnd(h.0 as isize))
    }

    /// The handle as a number, to pass in a message parameter.
    pub fn as_isize(self) -> isize {
        self.0
    }
}

impl fmt::Display for Hwnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#X}", self.0)
    }
}

/// Raw message parameters, as the window procedure received them.
#[derive(Clone, Copy, Debug)]
pub struct Message {
    pub msg: u32,
    pub wparam: usize,
    pub lparam: isize,
}

impl Message {
    pub const fn loword_wparam(&self) -> u16 {
        (self.wparam & 0xFFFF) as u16
    }

    pub const fn hiword_wparam(&self) -> u16 {
        ((self.wparam >> 16) & 0xFFFF) as u16
    }

    pub const fn loword_lparam(&self) -> u32 {
        (self.lparam as usize & 0xFFFF) as u32
    }
}

/// Message numbers the apps match on.
pub mod msg {
    use windows::Win32::UI::WindowsAndMessaging as wm;
    pub const CREATE: u32 = wm::WM_CREATE;
    pub const DESTROY: u32 = wm::WM_DESTROY;
    pub const CLOSE: u32 = wm::WM_CLOSE;
    pub const SIZE: u32 = wm::WM_SIZE;
    pub const MOVE: u32 = wm::WM_MOVE;
    pub const EXITSIZEMOVE: u32 = wm::WM_EXITSIZEMOVE;
    pub const COMMAND: u32 = wm::WM_COMMAND;
    pub const HOTKEY: u32 = wm::WM_HOTKEY;
    pub const DPICHANGED: u32 = wm::WM_DPICHANGED;
    pub const DISPLAYCHANGE: u32 = wm::WM_DISPLAYCHANGE;
    pub const SETTINGCHANGE: u32 = wm::WM_SETTINGCHANGE;
    pub const APP: u32 = wm::WM_APP;
    pub const LBUTTONUP: u32 = wm::WM_LBUTTONUP;
    pub const RBUTTONUP: u32 = wm::WM_RBUTTONUP;
    pub const CONTEXTMENU: u32 = wm::WM_CONTEXTMENU;
    pub const PAINT: u32 = wm::WM_PAINT;
    pub const ERASEBKGND: u32 = wm::WM_ERASEBKGND;
    pub const NCHITTEST: u32 = wm::WM_NCHITTEST;
    pub const MOUSEACTIVATE: u32 = wm::WM_MOUSEACTIVATE;
    pub const COPYDATA: u32 = wm::WM_COPYDATA;
    pub const TIMER: u32 = wm::WM_TIMER;
    pub const KEYDOWN: u32 = wm::WM_KEYDOWN;
    pub const SYSKEYDOWN: u32 = wm::WM_SYSKEYDOWN;
    pub const LBUTTONDOWN: u32 = wm::WM_LBUTTONDOWN;
    pub const MOUSEMOVE: u32 = wm::WM_MOUSEMOVE;
    pub const ACTIVATE: u32 = wm::WM_ACTIVATE;
    pub const GETDLGCODE: u32 = wm::WM_GETDLGCODE;
    /// `WM_NCHITTEST` result: let clicks fall through to the window below.
    pub const HTTRANSPARENT: isize = wm::HTTRANSPARENT as isize;
    /// `WM_MOUSEACTIVATE` result: don't activate.
    pub const MA_NOACTIVATE: isize = wm::MA_NOACTIVATE as isize;
    /// `wParam` of `WM_SETTINGCHANGE` when the work area changed.
    pub const SPI_SETWORKAREA: usize = wm::SPI_SETWORKAREA.0 as usize;
}

/// The safe side of a window procedure.
pub trait WindowHandler {
    /// Return `Some(result)` if the message was handled, `None` for default processing.
    ///
    /// Re-entrancy: if something called from inside `handle` makes Windows send another
    /// message to this window synchronously (SetWindowPos → WM_SIZE, a modal menu loop...),
    /// that nested message gets default processing and is **not** passed to `handle`.
    /// Re-read state after such calls instead of relying on the nested message.
    fn handle(&mut self, hwnd: Hwnd, msg: &Message) -> Option<isize>;
}

/// Per-window state behind `GWLP_USERDATA`.
struct Slot {
    handler: RefCell<Box<dyn WindowHandler>>,
    /// Set when WM_NCDESTROY arrives while the handler is borrowed further up the stack;
    /// that outer frame then frees the slot.
    dead: Cell<bool>,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Background {
    /// The dialog colour, matching STATIC/BUTTON controls.
    #[default]
    Dialog,
    /// No background brush (the window paints everything or is never shown).
    None,
}

/// Parameters for [`create_window`]. `None` position/size means `CW_USEDEFAULT`.
pub struct WindowSpec<'a> {
    pub class: &'a str,
    pub title: &'a str,
    pub style: u32,
    pub ex_style: u32,
    pub rect: Option<Rect>,
    /// Use icon resource 1 of the exe for the class.
    pub icon: bool,
    pub background: Background,
}

/// Registers the class (once) and creates a top-level window driven by `handler`.
pub fn create_window(spec: &WindowSpec, handler: Box<dyn WindowHandler>) -> crate::Result<Hwnd> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = wide(spec.class);
        let icon = if spec.icon {
            LoadIconW(Some(instance.into()), crate::int_resource(1)).unwrap_or_default()
        } else {
            HICON::default()
        };
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: match spec.background {
                Background::Dialog => HBRUSH((COLOR_BTNFACE.0 + 1) as usize as *mut c_void),
                Background::None => HBRUSH::default(),
            },
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // Fails harmlessly with ERROR_CLASS_ALREADY_EXISTS on a second window of the class.
        RegisterClassExW(&wc);

        let (x, y, w, h) = match spec.rect {
            Some(r) => (r.left, r.top, r.width(), r.height()),
            None => (CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT),
        };
        let slot = Box::into_raw(Box::new(Slot { handler: RefCell::new(handler), dead: Cell::new(false) }));
        let title = wide(spec.title);
        ATTACHED.with(|a| a.set(false));
        let result = CreateWindowExW(
            WINDOW_EX_STYLE(spec.ex_style),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WINDOW_STYLE(spec.style),
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            Some(slot as *const c_void),
        );
        match result {
            Ok(hwnd) => Ok(Hwnd::from_raw(hwnd).expect("CreateWindowExW returned a valid handle")),
            Err(e) => {
                // WM_NCCREATE may have attached the slot before creation failed; WM_NCDESTROY
                // then freed it. If it never got attached, free it here.
                if !ATTACHED.with(Cell::get) {
                    drop(Box::from_raw(slot));
                }
                Err(e)
            }
        }
    }
}

thread_local! {
    /// Whether the last WM_NCCREATE on this thread took ownership of its slot.
    static ATTACHED: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            ATTACHED.with(|a| a.set(true));
        }
        let slot = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Slot;
        if slot.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if (*slot).handler.try_borrow_mut().is_ok() {
                drop(Box::from_raw(slot as *mut Slot));
            } else {
                (*slot).dead.set(true);
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        let message = Message { msg, wparam: wparam.0, lparam: lparam.0 };
        let handled = match (*slot).handler.try_borrow_mut() {
            Ok(mut handler) => {
                let this = Hwnd::from_raw(hwnd).expect("wndproc gets a valid hwnd");
                let result = handler.handle(this, &message);
                drop(handler);
                if (*slot).dead.get() {
                    // The window was destroyed during `handle`; this frame owned the borrow.
                    drop(Box::from_raw(slot as *mut Slot));
                }
                result
            }
            Err(_) => None, // nested message while `handle` runs: default processing
        };
        match handled {
            Some(result) => LRESULT(result),
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Runs the message loop until `WM_QUIT`; returns its exit code. With `dialog`, keyboard
/// navigation (Tab, Enter = IDOK) works for that window's child controls.
pub fn run_message_loop(dialog: Option<Hwnd>) -> i32 {
    unsafe {
        let mut msg = MSG::default();
        loop {
            match GetMessageW(&mut msg, None, 0, 0).0 {
                0 => return msg.wParam.0 as i32,
                -1 => return -1,
                _ => {}
            }
            if let Some(d) = dialog {
                if IsDialogMessageW(d.raw(), &msg).as_bool() {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn post_quit(code: i32) {
    unsafe { PostQuitMessage(code) }
}

pub fn post_message(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> crate::Result<()> {
    unsafe { PostMessageW(Some(hwnd.raw()), msg, WPARAM(wparam), LPARAM(lparam)) }
}

/// `RegisterWindowMessageW`: a system-wide message number for `name` (0 on failure).
pub fn register_window_message(name: &str) -> u32 {
    let name = wide(name);
    unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
}

pub fn destroy_window(hwnd: Hwnd) -> crate::Result<()> {
    unsafe { DestroyWindow(hwnd.raw()) }
}

pub fn show_window(hwnd: Hwnd) {
    unsafe {
        let _ = ShowWindow(hwnd.raw(), SW_SHOWNORMAL);
    }
}

pub fn set_text(hwnd: Hwnd, text: &str) -> crate::Result<()> {
    let text = wide(text);
    unsafe { SetWindowTextW(hwnd.raw(), PCWSTR(text.as_ptr())) }
}

pub fn text(hwnd: Hwnd) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd.raw()).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(hwnd.raw(), &mut buf).max(0) as usize;
        from_wide(&buf[..n])
    }
}

pub fn style(hwnd: Hwnd) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd.raw(), GWL_STYLE) as u32 }
}

pub fn ex_style(hwnd: Hwnd) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd.raw(), GWL_EXSTYLE) as u32 }
}

/// Sets `GWL_STYLE` and tells Windows the frame changed (`SWP_FRAMECHANGED`), without
/// moving, sizing, activating or reordering the window.
pub fn set_style(hwnd: Hwnd, style: u32) -> crate::Result<()> {
    unsafe {
        SetWindowLongPtrW(hwnd.raw(), GWL_STYLE, style as isize);
        SetWindowPos(
            hwnd.raw(),
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}

/// Sets `GWL_STYLE` and `GWL_EXSTYLE`, then moves and sizes the window to `rect` in the same
/// `SetWindowPos` that tells Windows the frame changed. With one call the client area goes
/// straight to its new size, instead of first growing into the old frame (a game would see
/// that as a resize). `WS_EX_TOPMOST` can't be changed this way; use [`set_topmost`].
pub fn set_frame(hwnd: Hwnd, style: u32, ex_style: u32, rect: Rect) -> crate::Result<()> {
    unsafe {
        SetWindowLongPtrW(hwnd.raw(), GWL_STYLE, style as isize);
        SetWindowLongPtrW(hwnd.raw(), GWL_EXSTYLE, ex_style as isize);
        SetWindowPos(
            hwnd.raw(),
            None,
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}

/// Puts a window into (or takes it out of) the topmost band, without moving or activating it.
pub fn set_topmost(hwnd: Hwnd, topmost: bool) -> crate::Result<()> {
    let after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe { SetWindowPos(hwnd.raw(), Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) }
}

/// Moves and sizes a window to `rect` (window rect, screen coordinates, physical px).
pub fn set_window_rect(hwnd: Hwnd, rect: Rect) -> crate::Result<()> {
    unsafe {
        SetWindowPos(hwnd.raw(), None, rect.left, rect.top, rect.width(), rect.height(), SWP_NOZORDER | SWP_NOACTIVATE)
    }
}

/// Shows `hwnd` without activating it, directly below `above` in the z-order (so `above`
/// stays visible over it), at `rect`.
pub fn show_below(hwnd: Hwnd, above: Hwnd, rect: Rect) -> crate::Result<()> {
    unsafe {
        SetWindowPos(
            hwnd.raw(),
            Some(above.raw()),
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

/// Shows `hwnd` above every non-topmost window, at `rect`, without activating it.
pub fn show_topmost(hwnd: Hwnd, rect: Rect) -> crate::Result<()> {
    unsafe {
        SetWindowPos(
            hwnd.raw(),
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

/// A one-off or repeating `WM_TIMER` (wParam = `id`) to `hwnd` every `ms` milliseconds,
/// until [`kill_timer`]. Setting the same id again restarts it.
pub fn set_timer(hwnd: Hwnd, id: usize, ms: u32) -> crate::Result<()> {
    match unsafe { SetTimer(Some(hwnd.raw()), id, ms, None) } {
        0 => Err(crate::Error::from_thread()),
        _ => Ok(()),
    }
}

pub fn kill_timer(hwnd: Hwnd, id: usize) {
    unsafe {
        let _ = KillTimer(Some(hwnd.raw()), id);
    }
}

/// Asks Windows 11 to round the corners of a popup window (ignored on Windows 10).
pub fn round_corners(hwnd: Hwnd) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    let value = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd.raw(),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &value as *const _ as *const c_void,
            size_of_val(&value) as u32,
        );
    }
}

/// Brings a window to the front (restoring it if minimised).
pub fn activate(hwnd: Hwnd) {
    unsafe {
        if IsIconic(hwnd.raw()).as_bool() {
            let _ = ShowWindow(hwnd.raw(), SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd.raw());
    }
}

pub fn hide(hwnd: Hwnd) {
    unsafe {
        let _ = ShowWindow(hwnd.raw(), SW_HIDE);
    }
}

/// Restores a maximised or minimised window, waiting until it's done (so the window can be
/// measured and moved straight after).
pub fn restore(hwnd: Hwnd) {
    unsafe {
        let _ = ShowWindow(hwnd.raw(), SW_RESTORE);
    }
}

/// Marks the whole window for repainting; WM_PAINT follows from the message loop.
pub fn invalidate(hwnd: Hwnd) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd.raw()), None, false);
    }
}

/// For a `WS_EX_LAYERED` window: draw everything at `alpha` (0-255), and make pixels of
/// `transparent` (0x00BBGGRR) fully see-through.
pub fn set_layered(hwnd: Hwnd, transparent: u32, alpha: u8) -> crate::Result<()> {
    use windows::Win32::Foundation::COLORREF;
    unsafe { SetLayeredWindowAttributes(hwnd.raw(), COLORREF(transparent), alpha, LWA_ALPHA | LWA_COLORKEY) }
}

/// Sizes a window without moving it.
pub fn set_window_size(hwnd: Hwnd, width: i32, height: i32) -> crate::Result<()> {
    unsafe { SetWindowPos(hwnd.raw(), None, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) }
}

/// Moves/sizes a child control (client coordinates of its parent).
pub fn move_child(hwnd: Hwnd, rect: Rect) {
    unsafe {
        let _ = MoveWindow(hwnd.raw(), rect.left, rect.top, rect.width(), rect.height(), true);
    }
}

/// The suggested new window rect carried by `WM_DPICHANGED`.
pub fn dpi_changed_rect(msg: &Message) -> Option<Rect> {
    if msg.msg != WM_DPICHANGED || msg.lparam == 0 {
        return None;
    }
    // SAFETY: for WM_DPICHANGED, lParam points to a RECT owned by the sender for the
    // duration of the message.
    let r = unsafe { *(msg.lparam as *const RECT) };
    Some(crate::rect_from(r))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageBoxKind {
    Info,
    Warning,
    Error,
}

pub fn message_box(owner: Option<Hwnd>, text: &str, caption: &str, kind: MessageBoxKind) {
    let text = wide(text);
    let caption = wide(caption);
    let icon = match kind {
        MessageBoxKind::Info => MB_ICONINFORMATION,
        MessageBoxKind::Warning => MB_ICONWARNING,
        MessageBoxKind::Error => MB_ICONERROR,
    };
    unsafe {
        MessageBoxW(owner.map(Hwnd::raw), PCWSTR(text.as_ptr()), PCWSTR(caption.as_ptr()), MB_OK | icon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_words() {
        let m = Message { msg: 0, wparam: 0x0001_0002, lparam: 0x0000_0205 };
        assert_eq!(m.loword_wparam(), 2);
        assert_eq!(m.hiword_wparam(), 1);
        assert_eq!(m.loword_lparam(), 0x205);
    }

    struct Plain;
    impl WindowHandler for Plain {
        fn handle(&mut self, _: Hwnd, _: &Message) -> Option<isize> {
            None
        }
    }

    /// A real (never shown) window loses its frame and gets it back, and its rects follow.
    #[test]
    fn set_frame_removes_and_restores_the_border() {
        use crate::inspect;
        use crate::styles::ws;
        let spec = WindowSpec {
            class: "GamingZones.Test.Frame",
            title: "frame test",
            style: ws::OVERLAPPEDWINDOW,
            ex_style: ws::EX_WINDOWEDGE,
            rect: Some(Rect::from_xywh(100, 100, 640, 480)),
            icon: false,
            background: Background::None,
        };
        let hwnd = create_window(&spec, Box::new(Plain)).unwrap();
        let (style, ex) = (super::style(hwnd), ex_style(hwnd));
        let client = inspect::client_rect_screen(hwnd).unwrap();
        assert_ne!(inspect::window_rect(hwnd).unwrap(), client);

        let border = ws::CAPTION | ws::THICKFRAME | ws::SYSMENU | ws::MINIMIZEBOX | ws::MAXIMIZEBOX;
        set_frame(hwnd, style & !border, ex & !ws::EX_WINDOWEDGE, client).unwrap();
        assert_eq!(super::style(hwnd) & border, 0);
        assert_eq!(inspect::window_rect(hwnd).unwrap(), client);
        assert_eq!(inspect::client_rect_screen(hwnd).unwrap(), client);

        let win = inspect::window_rect(hwnd).unwrap();
        set_frame(hwnd, style, ex, win).unwrap();
        assert_eq!((super::style(hwnd), ex_style(hwnd)), (style, ex));
        assert!(inspect::client_rect_screen(hwnd).unwrap().width() < client.width());

        set_topmost(hwnd, true).unwrap();
        assert_ne!(ex_style(hwnd) & ws::EX_TOPMOST, 0);
        set_topmost(hwnd, false).unwrap();
        assert_eq!(ex_style(hwnd) & ws::EX_TOPMOST, 0);
        destroy_window(hwnd).unwrap();
    }
}
