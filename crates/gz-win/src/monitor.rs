//! Monitor rects and work areas.

use gz_core::Rect;
use windows::core::BOOL;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
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

/// The monitor containing a point, if any.
pub fn from_point(x: i32, y: i32) -> Option<MonitorInfo> {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) };
    if monitor.is_invalid() {
        return None;
    }
    info(monitor)
}

/// All monitors, in the order Windows enumerates them.
pub fn all() -> Vec<MonitorInfo> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut Vec` passed to EnumDisplayMonitors below, alive for the call.
        let list = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
        if let Some(i) = info(monitor) {
            list.push(i);
        }
        BOOL(1)
    }
    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut Vec<MonitorInfo> as isize));
    }
    list
}

/// A monitor's identity for `applied.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorId {
    /// Stable across reboots: the DisplayConfig target device path, e.g.
    /// `DISPLAY#DEL4321#5&2c3d4e&0&UID4352`. Changes if the monitor moves to another port.
    /// Falls back to `\\.\DISPLAY1@2560x1440` if DisplayConfig has nothing for it.
    pub id: String,
    /// Friendly name from the EDID, e.g. `DELL U3423WE` (may be empty).
    pub name: String,
}

/// Looks up stable ids for monitors. Query once and reuse for several monitors.
pub struct MonitorIds {
    /// (GDI device name, id).
    targets: Vec<(String, MonitorId)>,
}

impl MonitorIds {
    pub fn query() -> MonitorIds {
        MonitorIds { targets: display_targets().unwrap_or_default() }
    }

    pub fn id(&self, monitor: &MonitorInfo) -> MonitorId {
        self.targets.iter().find(|(device, _)| *device == monitor.device).map(|(_, id)| id.clone()).unwrap_or_else(
            || MonitorId {
                id: format!("{}@{}x{}", monitor.device, monitor.rect.width(), monitor.rect.height()),
                name: String::new(),
            },
        )
    }
}

/// Active display paths: GDI device name → target device path and friendly name.
fn display_targets() -> Option<Vec<(String, MonitorId)>> {
    unsafe {
        let (mut n_paths, mut n_modes) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) != ERROR_SUCCESS {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut n_paths,
            paths.as_mut_ptr(),
            &mut n_modes,
            modes.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        paths.truncate(n_paths as usize);

        let mut out: Vec<(String, MonitorId)> = Vec::new();
        for path in &paths {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
                continue;
            }
            let device = from_wide(&source.viewGdiDeviceName);
            let device_path = tidy_device_path(&from_wide(&target.monitorDevicePath));
            // A mirrored source has several targets; the first one names it.
            if device_path.is_empty() || out.iter().any(|(d, _)| *d == device) {
                continue;
            }
            out.push((device, MonitorId { id: device_path, name: from_wide(&target.monitorFriendlyDeviceName) }));
        }
        Some(out)
    }
}

/// `\\?\DISPLAY#DEL4321#5&2c3d4e&0&UID4352#{e6f07b5f-...}` → `DISPLAY#DEL4321#5&2c3d4e&0&UID4352`:
/// drops the `\\?\` prefix and the interface-class GUID, which are the same for every monitor.
fn tidy_device_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = match path.rfind("#{") {
        Some(i) if path.ends_with('}') => &path[..i],
        _ => path,
    };
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidies_device_paths() {
        assert_eq!(
            tidy_device_path(r"\\?\DISPLAY#DEL4321#5&2c3d4e&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"),
            "DISPLAY#DEL4321#5&2c3d4e&0&UID4352"
        );
        assert_eq!(tidy_device_path("odd"), "odd");
    }

    #[test]
    fn every_monitor_gets_an_id() {
        let monitors = all();
        assert!(!monitors.is_empty());
        let ids = MonitorIds::query();
        for m in &monitors {
            let id = ids.id(m);
            assert!(!id.id.is_empty());
            assert_eq!(from_point(m.rect.left, m.rect.top).map(|f| f.device), Some(m.device.clone()));
        }
    }
}
