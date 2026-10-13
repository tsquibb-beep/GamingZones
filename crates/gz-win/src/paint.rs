//! GDI painting for the zone overlay: filled rectangles, borders and centred text.

use gz_core::{Color, Rect};
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DrawTextW, EndPaint, ExcludeClipRect, FillRect, GetStockObject, SelectClipRgn, SelectObject, SetBkMode,
    SetDCBrushColor, SetTextColor, DC_BRUSH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HBRUSH, HDC,
    PAINTSTRUCT, TRANSPARENT,
};

use crate::controls::Font;
use crate::{rect_to, Hwnd};

/// GDI's 0x00BBGGRR.
pub const fn colorref(c: Color) -> u32 {
    c.r as u32 | (c.g as u32) << 8 | (c.b as u32) << 16
}

/// Draws into a window during WM_PAINT. Coordinates are client coordinates.
pub struct Painter {
    hdc: HDC,
}

/// Runs `f` between BeginPaint and EndPaint. Call only while handling WM_PAINT.
pub fn paint(hwnd: Hwnd, f: impl FnOnce(&mut Painter)) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd.raw(), &mut ps) };
    if !hdc.is_invalid() {
        f(&mut Painter { hdc });
    }
    unsafe {
        let _ = EndPaint(hwnd.raw(), &ps);
    }
}

impl Painter {
    pub fn fill(&mut self, rect: Rect, color: Color) {
        unsafe {
            SetDCBrushColor(self.hdc, COLORREF(colorref(color)));
            FillRect(self.hdc, &rect_to(rect), HBRUSH(GetStockObject(DC_BRUSH).0));
        }
    }

    /// Leaves `rect` untouched by later drawing, until [`Painter::reset_clip`].
    pub fn exclude(&mut self, rect: Rect) {
        unsafe {
            ExcludeClipRect(self.hdc, rect.left, rect.top, rect.right, rect.bottom);
        }
    }

    pub fn reset_clip(&mut self) {
        unsafe {
            SelectClipRgn(self.hdc, None);
        }
    }

    /// A border `width` px thick, inside `rect`.
    pub fn frame(&mut self, rect: Rect, color: Color, width: i32) {
        let w = width.min(rect.width() / 2).min(rect.height() / 2).max(0);
        if w == 0 {
            return;
        }
        let r = rect;
        self.fill(Rect::new(r.left, r.top, r.right, r.top + w), color);
        self.fill(Rect::new(r.left, r.bottom - w, r.right, r.bottom), color);
        self.fill(Rect::new(r.left, r.top + w, r.left + w, r.bottom - w), color);
        self.fill(Rect::new(r.right - w, r.top + w, r.right, r.bottom - w), color);
    }

    /// One line of text centred in `rect`.
    pub fn text_centered(&mut self, rect: Rect, text: &str, font: &Font, color: Color) {
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        let mut r = rect_to(rect);
        unsafe {
            let old = SelectObject(self.hdc, font.handle().into());
            SetTextColor(self.hdc, COLORREF(colorref(color)));
            SetBkMode(self.hdc, TRANSPARENT);
            DrawTextW(self.hdc, &mut buf, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(self.hdc, old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(colorref(Color::rgb(0x00, 0x8C, 0xFF)), 0x00FF_8C00);
    }
}
