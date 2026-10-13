//! DPI queries.

use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetAwarenessFromDpiAwarenessContext, GetDpiForWindow, GetThreadDpiAwarenessContext,
    GetWindowDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_PER_MONITOR_AWARE, DPI_AWARENESS_SYSTEM_AWARE, DPI_AWARENESS_UNAWARE,
};

use crate::Hwnd;

/// The DPI Windows uses for `hwnd` (96 = 100%, 120 = 125%). 0 if the window is gone.
pub fn window_dpi(hwnd: Hwnd) -> u32 {
    unsafe { GetDpiForWindow(hwnd.raw()) }
}

fn describe(ctx: DPI_AWARENESS_CONTEXT) -> &'static str {
    unsafe {
        if AreDpiAwarenessContextsEqual(ctx, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).as_bool() {
            return "per-monitor v2";
        }
        match GetAwarenessFromDpiAwarenessContext(ctx) {
            DPI_AWARENESS_PER_MONITOR_AWARE => "per-monitor v1",
            DPI_AWARENESS_SYSTEM_AWARE => "system aware",
            DPI_AWARENESS_UNAWARE => "unaware (bitmap-stretched by Windows)",
            _ => "unknown",
        }
    }
}

/// DPI awareness of another window, e.g. "per-monitor v2" or "unaware".
pub fn window_awareness(hwnd: Hwnd) -> &'static str {
    describe(unsafe { GetWindowDpiAwarenessContext(hwnd.raw()) })
}

/// Whether the calling thread runs Per-Monitor-V2 (i.e. the manifest took effect).
pub fn thread_is_per_monitor_v2() -> bool {
    unsafe {
        AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(), DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            .as_bool()
    }
}

/// Scales a 96-DPI design value to `dpi`, rounding to nearest.
pub const fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

/// `dpi` as a scaling percentage (120 → 125).
pub const fn percent(dpi: u32) -> u32 {
    (dpi * 100 + 48) / 96
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling() {
        assert_eq!(scale(12, 96), 12);
        assert_eq!(scale(12, 120), 15);
        assert_eq!(scale(10, 144), 15);
        assert_eq!(percent(120), 125);
        assert_eq!(percent(144), 150);
    }
}
