//! Cursor and key state, read on demand (nothing here runs in the background).

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetPhysicalCursorPos, LoadCursorW, CURSORINFO, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE,
};

/// Cursor position in physical screen pixels.
pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetPhysicalCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
}

/// Whether a key is down right now (`GetAsyncKeyState`).
pub fn key_down(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

/// Whether the cursor currently shows one of the resize arrows. `EVENT_SYSTEM_MOVESIZESTART`
/// fires for resizes as well as moves; FancyZones tells them apart this way
/// (`WindowUtils.cpp`, `IsCursorTypeIndicatingSizeEvent`).
pub fn cursor_is_resize() -> bool {
    let mut info = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
    if unsafe { GetCursorInfo(&mut info) }.is_err() {
        return false;
    }
    [IDC_SIZENS, IDC_SIZEWE, IDC_SIZENESW, IDC_SIZENWSE]
        .into_iter()
        .filter_map(|id| unsafe { LoadCursorW(None, id) }.ok())
        .any(|c| c == info.hCursor)
}
