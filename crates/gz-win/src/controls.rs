//! Standard child controls (STATIC, EDIT, BUTTON) and fonts, for the ruler window.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT, HGDIOBJ, LOGFONTW};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{wide, Hwnd};

/// A GDI font, deleted on drop. Keep it alive as long as any control uses it.
pub struct Font(HFONT);

impl Font {
    fn from_logfont(lf: &LOGFONTW) -> crate::Result<Font> {
        let font = unsafe { CreateFontIndirectW(lf) };
        if font.is_invalid() {
            Err(crate::Error::from_thread())
        } else {
            Ok(Font(font))
        }
    }

    /// The system UI ("message box") font at `dpi`.
    pub fn message_font(dpi: u32) -> crate::Result<Font> {
        let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                ncm.cbSize,
                Some(&mut ncm as *mut NONCLIENTMETRICSW as *mut _),
                0,
                dpi,
            )?;
        }
        Font::from_logfont(&ncm.lfMessageFont)
    }

    /// Consolas at `points` size for `dpi`.
    pub fn monospace(points: i32, dpi: u32) -> crate::Result<Font> {
        let mut lf = LOGFONTW { lfHeight: -(points * dpi as i32 + 36) / 72, lfWeight: 400, ..Default::default() };
        for (d, s) in lf.lfFaceName.iter_mut().zip("Consolas".encode_utf16()) {
            *d = s;
        }
        Font::from_logfont(&lf)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Control {
    Label,
    /// Digits only.
    NumberEdit,
    /// The default push button: Enter in the window clicks it. Give it id 1 (IDOK).
    DefaultButton,
    CheckBox,
}

/// Creates a visible child control with command id `id`.
pub fn create(parent: Hwnd, kind: Control, text: &str, id: u16) -> crate::Result<Hwnd> {
    // Style values from WinUser.h; the windows crate types them inconsistently.
    const SS_NOPREFIX: u32 = 0x80;
    const ES_AUTOHSCROLL: u32 = 0x80;
    const ES_NUMBER: u32 = 0x2000;
    const BS_DEFPUSHBUTTON: u32 = 0x1;
    const BS_AUTOCHECKBOX: u32 = 0x3;

    let base = WS_CHILD.0 | WS_VISIBLE.0;
    let (class, style, ex_style) = match kind {
        Control::Label => ("STATIC", base | SS_NOPREFIX, 0),
        Control::NumberEdit => ("EDIT", base | WS_TABSTOP.0 | ES_NUMBER | ES_AUTOHSCROLL, WS_EX_CLIENTEDGE.0),
        Control::DefaultButton => ("BUTTON", base | WS_TABSTOP.0 | BS_DEFPUSHBUTTON, 0),
        Control::CheckBox => ("BUTTON", base | WS_TABSTOP.0 | BS_AUTOCHECKBOX, 0),
    };
    let class = wide(class);
    let text = wide(text);
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(ex_style),
            PCWSTR(class.as_ptr()),
            PCWSTR(text.as_ptr()),
            WINDOW_STYLE(style),
            0,
            0,
            0,
            0,
            Some(parent.raw()),
            Some(HMENU(id as usize as *mut _)),
            Some(instance.into()),
            None,
        )?;
        Ok(Hwnd::from_raw(hwnd).expect("CreateWindowExW returned a valid handle"))
    }
}

fn send(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd.raw(), msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

pub fn set_font(control: Hwnd, font: &Font) {
    send(control, WM_SETFONT, font.0 .0 as usize, 1);
}

pub fn is_checked(checkbox: Hwnd) -> bool {
    send(checkbox, BM_GETCHECK, 0, 0) == 1
}

pub fn set_checked(checkbox: Hwnd, checked: bool) {
    send(checkbox, BM_SETCHECK, checked as usize, 0);
}

pub fn focus(control: Hwnd) {
    unsafe {
        let _ = SetFocus(Some(HWND(control.raw().0)));
    }
}

/// Selects all text in an edit control.
pub fn select_all(edit: Hwnd) {
    const EM_SETSEL: u32 = 0x00B1;
    send(edit, EM_SETSEL, 0, -1);
}
