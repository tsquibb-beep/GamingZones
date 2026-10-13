//! Process queries. Only `PROCESS_QUERY_LIMITED_INFORMATION` is ever requested (PLAN §1).

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::{from_wide, Hwnd};

/// Closes a handle on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open(pid: u32) -> crate::Result<Owned> {
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map(Owned) }
}

/// Process id of the window's owner (0 if the window is gone).
pub fn window_pid(hwnd: Hwnd) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd.raw(), Some(&mut pid)) };
    pid
}

/// Full path of the process's executable.
pub fn exe_path(pid: u32) -> crate::Result<String> {
    let process = open(pid)?;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)? };
    Ok(from_wide(&buf[..len as usize]))
}

fn token_elevated(process: HANDLE) -> crate::Result<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)?;
        let token = Owned(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )?;
        Ok(elevation.TokenIsElevated != 0)
    }
}

/// Whether the process runs elevated (UAC "as administrator").
pub fn is_elevated(pid: u32) -> crate::Result<bool> {
    token_elevated(open(pid)?.0)
}

pub fn current_is_elevated() -> crate::Result<bool> {
    // GetCurrentProcess is a pseudo-handle and needs no closing.
    token_elevated(unsafe { GetCurrentProcess() })
}

/// Just the file name of a full path (`C:\x\game.exe` → `game.exe`).
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_of_path() {
        assert_eq!(file_name(r"C:\Games\Thing\game.exe"), "game.exe");
        assert_eq!(file_name("game.exe"), "game.exe");
    }

    #[test]
    fn own_process_is_queryable() {
        let path = exe_path(std::process::id()).unwrap();
        assert!(path.to_ascii_lowercase().ends_with(".exe"), "{path}");
        current_is_elevated().unwrap();
        is_elevated(std::process::id()).unwrap();
    }
}
