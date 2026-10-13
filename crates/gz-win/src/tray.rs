//! Notification-area icon, balloon messages and the popup menu.

use windows::core::PCWSTR;
use windows::Win32::Foundation::POINT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{LoadIconMetric, LIM_SMALL};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{wide, window, Hwnd};

fn copy_into(dst: &mut [u16], text: &str) {
    let room = dst.len() - 1; // keep the terminating NUL
    for (d, s) in dst.iter_mut().zip(text.encode_utf16().take(room)) {
        *d = s;
    }
}

/// The app's tray icon. Removed from the tray when dropped.
pub struct TrayIcon {
    hwnd: Hwnd,
    icon: HICON,
    callback_msg: u32,
    tip: String,
}

impl TrayIcon {
    /// Adds the icon (exe icon resource 1). Clicks arrive at `hwnd` as `callback_msg`
    /// with the mouse message (e.g. `WM_RBUTTONUP`) in the low word of lParam.
    pub fn add(hwnd: Hwnd, callback_msg: u32, tip: &str) -> crate::Result<TrayIcon> {
        let icon = unsafe {
            let instance = GetModuleHandleW(None)?;
            // LoadIconMetric picks the right size for the current DPI.
            LoadIconMetric(Some(instance.into()), crate::int_resource(1), LIM_SMALL)?
        };
        let tray = TrayIcon { hwnd, icon, callback_msg, tip: tip.to_string() };
        tray.notify(NIM_ADD, tray.data())?;
        Ok(tray)
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd.raw(),
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: self.callback_msg,
            hIcon: self.icon,
            ..Default::default()
        };
        copy_into(&mut nid.szTip, &self.tip);
        nid
    }

    fn notify(&self, op: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE, nid: NOTIFYICONDATAW) -> crate::Result<()> {
        if unsafe { Shell_NotifyIconW(op, &nid) }.as_bool() {
            Ok(())
        } else {
            Err(crate::Error::from_thread())
        }
    }

    /// Adds the icon again after Explorer restarts (`TaskbarCreated`).
    pub fn re_add(&self) -> crate::Result<()> {
        self.notify(NIM_ADD, self.data())
    }

    /// Shows a Windows notification from the tray icon.
    pub fn balloon(&self, title: &str, text: &str, warning: bool) -> crate::Result<()> {
        let mut nid = self.data();
        nid.uFlags |= NIF_INFO;
        nid.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        copy_into(&mut nid.szInfoTitle, title);
        copy_into(&mut nid.szInfo, text);
        self.notify(NIM_MODIFY, nid)
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let _ = self.notify(NIM_DELETE, self.data());
        unsafe {
            let _ = DestroyIcon(self.icon);
        }
    }
}

/// Message number Explorer broadcasts when the taskbar is (re)created.
pub fn taskbar_created_message() -> u32 {
    window::register_window_message("TaskbarCreated")
}

pub enum MenuItem<'a> {
    Item { id: u32, label: &'a str, enabled: bool },
    Separator,
}

/// Shows a popup menu at the cursor; returns the chosen id, or `None` if dismissed.
pub fn show_menu(hwnd: Hwnd, items: &[MenuItem]) -> Option<u32> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for item in items {
            match item {
                MenuItem::Item { id, label, enabled } => {
                    let text = wide(label);
                    let flags = if *enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                    let _ = AppendMenuW(menu, flags, *id as usize, PCWSTR(text.as_ptr()));
                }
                MenuItem::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
            }
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Without this the menu doesn't close when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd.raw());
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd.raw(),
            None,
        );
        let _ = PostMessageW(Some(hwnd.raw()), WM_NULL, Default::default(), Default::default());
        let _ = DestroyMenu(menu);
        (cmd.0 != 0).then_some(cmd.0 as u32)
    }
}
