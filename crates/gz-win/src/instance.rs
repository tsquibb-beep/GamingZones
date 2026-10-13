//! Single-instance guard (named mutex).

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

use crate::wide;

/// Holds the mutex for the life of the process.
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// `Ok(None)` if another process already holds `name`.
    pub fn acquire(name: &str) -> crate::Result<Option<SingleInstance>> {
        let name = wide(name);
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(SingleInstance(handle)))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
