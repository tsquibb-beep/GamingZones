//! Thin, safe wrappers over the Win32 calls GamingZones needs.
//!
//! Every `unsafe` block in the project lives in this crate. The apps (`gamingzones`, `ruler`)
//! forbid `unsafe` and talk to Windows only through these functions. Coordinates are physical
//! pixels: both apps are Per-Monitor-V2 DPI aware via their manifests, so Windows does not
//! virtualise anything we read or write.

pub mod controls;
pub mod dpi;
pub mod hotkey;
pub mod inspect;
pub mod instance;
pub mod monitor;
pub mod process;
pub mod styles;
pub mod time;
pub mod tray;
pub mod window;

pub use window::Hwnd;
pub use windows::core::{Error, Result};

/// UTF-16, NUL-terminated copy of `s` for passing to W APIs.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Converts a NUL-terminated (or full) UTF-16 buffer to a String.
pub(crate) fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// `MAKEINTRESOURCEW(id)`: a resource id passed where a name pointer is expected.
pub(crate) fn int_resource(id: u16) -> windows::core::PCWSTR {
    windows::core::PCWSTR(std::ptr::without_provenance(id as usize))
}

pub(crate) fn rect_from(r: windows::Win32::Foundation::RECT) -> gz_core::Rect {
    gz_core::Rect::new(r.left, r.top, r.right, r.bottom)
}
