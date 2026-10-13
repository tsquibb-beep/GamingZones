//! Local wall-clock time for log lines.

use windows::Win32::System::SystemInformation::GetLocalTime;

/// `YYYY-MM-DD HH:MM:SS.mmm` in local time.
pub fn local_timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}
