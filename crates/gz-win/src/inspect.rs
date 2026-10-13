//! Everything the debug hotkey and the ruler report about a window.

use gz_core::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetClassNameW, GetClientRect, GetForegroundWindow, GetWindow, GetWindowRect, IsIconic,
    IsWindow, IsWindowVisible, IsZoomed, GA_ROOT, GW_OWNER,
};

use crate::monitor::{self, MonitorInfo};
use crate::{dpi, from_wide, process, rect_from, window, Hwnd};

/// Every top-level window, front to back.
pub fn top_level_windows() -> Vec<Hwnd> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<Hwnd>) };
        list.extend(Hwnd::from_raw(hwnd));
        BOOL(1)
    }
    let mut list: Vec<Hwnd> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut Vec<Hwnd> as isize));
    }
    list
}

pub fn foreground_window() -> Option<Hwnd> {
    Hwnd::from_raw(unsafe { GetForegroundWindow() })
}

pub fn exists(hwnd: Hwnd) -> bool {
    unsafe { IsWindow(Some(hwnd.raw())) }.as_bool()
}

pub fn is_visible(hwnd: Hwnd) -> bool {
    unsafe { IsWindowVisible(hwnd.raw()) }.as_bool()
}

pub fn is_minimized(hwnd: Hwnd) -> bool {
    unsafe { IsIconic(hwnd.raw()) }.as_bool()
}

pub fn is_maximized(hwnd: Hwnd) -> bool {
    unsafe { IsZoomed(hwnd.raw()) }.as_bool()
}

/// The top-level window containing `hwnd` (itself, for a top-level window).
pub fn root(hwnd: Hwnd) -> Option<Hwnd> {
    Hwnd::from_raw(unsafe { GetAncestor(hwnd.raw(), GA_ROOT) })
}

/// The owner of a top-level window (dialogs are owned by their main window).
pub fn owner(hwnd: Hwnd) -> Option<Hwnd> {
    unsafe { GetWindow(hwnd.raw(), GW_OWNER) }.ok().and_then(Hwnd::from_raw)
}

/// `GetWindowRect`: includes the invisible resize borders on Windows 10/11.
pub fn window_rect(hwnd: Hwnd) -> crate::Result<Rect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd.raw(), &mut r)? };
    Ok(rect_from(r))
}

/// The client area in screen coordinates.
pub fn client_rect_screen(hwnd: Hwnd) -> crate::Result<Rect> {
    let mut r = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        GetClientRect(hwnd.raw(), &mut r)?;
        if !ClientToScreen(hwnd.raw(), &mut origin).as_bool() {
            return Err(crate::Error::from_thread());
        }
    }
    Ok(rect_from(r).offset(origin.x, origin.y))
}

/// `DWMWA_EXTENDED_FRAME_BOUNDS`: the visible frame, without the invisible borders.
/// Always physical pixels, whatever the caller's DPI awareness.
pub fn frame_bounds(hwnd: Hwnd) -> crate::Result<Rect> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd.raw(),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )?
    };
    Ok(rect_from(r))
}

/// Non-zero if DWM has cloaked the window (other virtual desktop, suspended UWP app...).
pub fn cloaked(hwnd: Hwnd) -> u32 {
    let mut value = 0u32;
    let _ = unsafe {
        DwmGetWindowAttribute(hwnd.raw(), DWMWA_CLOAKED, &mut value as *mut u32 as *mut _, size_of::<u32>() as u32)
    };
    value
}

pub fn class_name(hwnd: Hwnd) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd.raw(), &mut buf) }.max(0) as usize;
    from_wide(&buf[..n])
}

#[derive(Clone, Debug)]
pub enum Elevation {
    Elevated,
    NotElevated,
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: Hwnd,
    pub class: String,
    pub title: String,
    pub pid: u32,
    pub exe: Result<String, String>,
    pub elevation: Elevation,
    pub window_rect: Rect,
    pub client_rect: Rect,
    pub frame_bounds: Result<Rect, String>,
    pub dpi: u32,
    pub dpi_awareness: &'static str,
    pub monitor: Option<MonitorInfo>,
    pub style: u32,
    pub ex_style: u32,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub cloaked: u32,
}

/// Collects [`WindowInfo`]; fails only if the window itself is gone.
pub fn window_info(hwnd: Hwnd) -> crate::Result<WindowInfo> {
    let pid = process::window_pid(hwnd);
    let elevation = match process::is_elevated(pid) {
        Ok(true) => Elevation::Elevated,
        Ok(false) => Elevation::NotElevated,
        Err(e) => Elevation::Unknown(e.message()),
    };
    Ok(WindowInfo {
        hwnd,
        class: class_name(hwnd),
        title: window::text(hwnd),
        pid,
        exe: process::exe_path(pid).map_err(|e| e.message()),
        elevation,
        window_rect: window_rect(hwnd)?,
        client_rect: client_rect_screen(hwnd)?,
        frame_bounds: frame_bounds(hwnd).map_err(|e| e.message()),
        dpi: dpi::window_dpi(hwnd),
        dpi_awareness: dpi::window_awareness(hwnd),
        monitor: monitor::for_window(hwnd),
        style: window::style(hwnd),
        ex_style: window::ex_style(hwnd),
        visible: is_visible(hwnd),
        minimized: is_minimized(hwnd),
        maximized: is_maximized(hwnd),
        cloaked: cloaked(hwnd),
    })
}
