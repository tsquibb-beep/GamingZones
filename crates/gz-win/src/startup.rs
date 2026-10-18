//! Starting GamingZones when Tom signs in: a value under HKCU `...\CurrentVersion\Run`, or
//! (to run it as administrator) a Scheduled Task with "Run with highest privileges".
//!
//! Creating or deleting the task needs administrator rights, so `schtasks.exe` is started
//! with the "runas" verb and Windows shows its UAC prompt.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use crate::wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The Run value's name and the task's name.
pub const NAME: &str = "GamingZones";
/// `CREATE_NO_WINDOW`: no console flashing up for schtasks.
const NO_WINDOW: u32 = 0x0800_0000;

/// The command line in the Run value, if there is one.
pub fn run_value() -> Option<String> {
    let (key, name) = (wide(RUN_KEY), wide(NAME));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let e = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (e == ERROR_SUCCESS).then(|| crate::from_wide(&buf))
}

/// Starts `exe` at sign-in (HKCU Run).
pub fn set_run_value(exe: &Path) -> crate::Result<()> {
    let (key, name) = (wide(RUN_KEY), wide(NAME));
    let data = wide(&format!("\"{}\"", exe.display()));
    let e = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    e.ok()
}

/// Removes the Run value (fine if there is none).
pub fn delete_run_value() -> crate::Result<()> {
    let (key, name) = (wide(RUN_KEY), wide(NAME));
    let e = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr())) };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    e.ok()
}

/// Whether the scheduled task exists (reading it needs no elevation).
pub fn task_exists() -> bool {
    Command::new("schtasks.exe")
        .args(["/Query", "/TN", NAME])
        .creation_flags(NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The task definition: at Tom's sign-in, run `exe` with highest privileges, no time limit,
/// also on battery, at normal priority (tasks default to below normal).
pub fn task_xml(exe: &Path, user: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let (exe, user) = (esc(&exe.display().to_string()), esc(user));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts GamingZones as administrator when you sign in.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// What an elevated schtasks run came to.
#[derive(Debug)]
pub enum Elevated {
    Done,
    /// Tom said No to the UAC prompt.
    Declined,
    Failed(String),
}

/// Runs `schtasks.exe args` as administrator (UAC prompt) and waits for it.
fn schtasks_elevated(args: &str) -> Elevated {
    let (verb, file, params) = (wide("runas"), wide("schtasks.exe"), wide(args));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        // ERROR_CANCELLED: the UAC prompt was declined.
        return if e.code().0 as u32 == 0x8007_04C7 { Elevated::Declined } else { Elevated::Failed(e.message()) };
    }
    if info.hProcess.is_invalid() {
        return Elevated::Failed("schtasks didn't start".into());
    }
    let mut code = 1u32;
    unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0 {
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
        }
        let _ = CloseHandle(info.hProcess);
    }
    if code == 0 {
        Elevated::Done
    } else {
        Elevated::Failed(format!("schtasks exited with code {code}"))
    }
}

/// Creates (or replaces) the task from `xml`, which is written to `xml_path` first (as
/// UTF-16, as schtasks wants). Shows the UAC prompt and waits; run it off the UI thread.
pub fn create_task(xml: &str, xml_path: &Path) -> Elevated {
    let bytes: Vec<u8> = [0xFEFFu16].into_iter().chain(xml.encode_utf16()).flat_map(u16::to_le_bytes).collect();
    if let Err(e) = std::fs::write(xml_path, bytes) {
        return Elevated::Failed(format!("{}: {e}", xml_path.display()));
    }
    let result = schtasks_elevated(&format!("/Create /TN {NAME} /XML \"{}\" /F", xml_path.display()));
    let _ = std::fs::remove_file(xml_path);
    result
}

/// Deletes the task. Shows the UAC prompt and waits; run it off the UI thread.
pub fn delete_task() -> Elevated {
    schtasks_elevated(&format!("/Delete /TN {NAME} /F"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_escapes_and_says_what_it_should() {
        let xml = task_xml(Path::new(r"C:\Tools & Games\gamingzones.exe"), r"PC\tom");
        assert!(xml.contains(r"<Command>C:\Tools &amp; Games\gamingzones.exe</Command>"));
        assert!(xml.contains(r"<UserId>PC\tom</UserId>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
    }

    #[test]
    fn run_value_round_trip() {
        // A made-up value name would be cleaner, but the functions use NAME. Only run this
        // when there's no real Run value, and put it back the way it was.
        if run_value().is_some() {
            return;
        }
        set_run_value(Path::new(r"C:\x\gamingzones.exe")).unwrap();
        assert_eq!(run_value().as_deref(), Some(r#""C:\x\gamingzones.exe""#));
        delete_run_value().unwrap();
        assert_eq!(run_value(), None);
        delete_run_value().unwrap();
    }
}
