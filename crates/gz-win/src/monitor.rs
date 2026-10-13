//! Monitor rects and work areas.

use gz_core::Rect;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use crate::{from_wide, rect_from, Hwnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    /// GDI device name, e.g. `\\.\DISPLAY1`. Not stable across reboots; fine for logs.
    pub device: String,
    pub rect: Rect,
    /// Monitor rect minus the taskbar and docked app bars.
    pub work: Rect,
    pub dpi: u32,
    pub primary: bool,
}

fn info(monitor: HMONITOR) -> Option<MonitorInfo> {
    unsafe {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(monitor, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return None;
        }
        let (mut dpi_x, mut dpi_y) = (0, 0);
        let dpi = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
            Ok(()) => dpi_x,
            Err(_) => 0,
        };
        Some(MonitorInfo {
            device: from_wide(&mi.szDevice),
            rect: rect_from(mi.monitorInfo.rcMonitor),
            work: rect_from(mi.monitorInfo.rcWork),
            dpi,
            primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        })
    }
}

/// The monitor a window is mostly on (nearest one if it is off-screen).
pub fn for_window(hwnd: Hwnd) -> Option<MonitorInfo> {
    info(unsafe { MonitorFromWindow(hwnd.raw(), MONITOR_DEFAULTTONEAREST) })
}
