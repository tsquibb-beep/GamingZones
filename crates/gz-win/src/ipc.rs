//! Talking between the editor and the tray app: finding the tray app's window, sending it
//! data with `WM_COPYDATA`, and letting those messages through when it runs elevated.

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, FindWindowW, SendMessageTimeoutW, MSGFLT_ALLOW, SMTO_ABORTIFHUNG, WM_COPYDATA,
};

use crate::window::Message;
use crate::{wide, Hwnd};

/// The top-level window with this class name, if there is one.
pub fn find_window(class: &str) -> Option<Hwnd> {
    let class = wide(class);
    unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) }.ok().and_then(Hwnd::from_raw)
}

/// Sends `msg` to `target` and returns its reply. Waits at most `timeout_ms`, and not at
/// all if the target is hung (then `Err`).
pub fn send_message(target: Hwnd, msg: u32, wparam: usize, lparam: isize, timeout_ms: u32) -> crate::Result<isize> {
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            target.raw(),
            msg,
            WPARAM(wparam),
            LPARAM(lparam),
            SMTO_ABORTIFHUNG,
            timeout_ms,
            Some(&mut result),
        )
    };
    if ok.0 == 0 {
        return Err(crate::Error::from_thread());
    }
    Ok(result as isize)
}

/// Sends `data` to `target` with `WM_COPYDATA` (`kind` says what it is). Waits at most two
/// seconds, and not at all if the target is hung.
pub fn send_data(target: Hwnd, sender: Option<Hwnd>, kind: usize, data: &[u8]) -> crate::Result<()> {
    let cds = COPYDATASTRUCT { dwData: kind, cbData: data.len() as u32, lpData: data.as_ptr() as *mut c_void };
    let wparam = WPARAM(sender.map_or(0, |h| h.as_isize() as usize));
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            target.raw(),
            WM_COPYDATA,
            wparam,
            LPARAM(&cds as *const COPYDATASTRUCT as isize),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        )
    };
    if ok.0 == 0 {
        return Err(crate::Error::from_thread());
    }
    Ok(())
}

/// For a `WM_COPYDATA` message: what was sent (`kind`, bytes) and the sender's window.
pub fn received_data(m: &Message) -> Option<(usize, Vec<u8>, Option<Hwnd>)> {
    if m.msg != WM_COPYDATA || m.lparam == 0 {
        return None;
    }
    // Valid for the duration of the message, which is all we need: the bytes are copied.
    let cds = unsafe { &*(m.lparam as *const COPYDATASTRUCT) };
    let bytes = if cds.lpData.is_null() || cds.cbData == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) }.to_vec()
    };
    Some((cds.dwData, bytes, crate::winevent::message_window(m.wparam as isize)))
}

/// Lets `msg` reach `hwnd` from less privileged processes (an elevated tray app still
/// hears the non-elevated editor).
pub fn allow_message(hwnd: Hwnd, msg: u32) -> crate::Result<()> {
    unsafe { ChangeWindowMessageFilterEx(hwnd.raw(), msg, MSGFLT_ALLOW, None) }
}
