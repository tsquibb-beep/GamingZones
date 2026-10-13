//! The zone overlay: one window covering a monitor's work area, shown only during a
//! Shift-drag and destroyed when the drag ends.
//!
//! It is a layered window drawn at one constant opacity (`SetLayeredWindowAttributes`), with
//! a colour key making any area outside the zones fully transparent. It never takes focus
//! or clicks (`WS_EX_TRANSPARENT | WS_EX_NOACTIVATE`) and sits directly under the dragged
//! window, so the window being dragged stays visible on top of the zones.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gz_core::config::Overlay as OverlayStyle;
use gz_core::layout::{zone_label, ResolvedZone};
use gz_core::DisabledZones;
use gz_core::{Color, Rect};
use gz_win::controls::Font;
use gz_win::window::{self, msg, Background, Message, WindowHandler, WindowSpec};
use gz_win::{dpi, paint, styles::ws, Hwnd};

/// Fully transparent colour (colour key). Unlikely to be picked as a zone colour.
const TRANSPARENT: Color = Color::rgb(0xFE, 0x00, 0xFE);

/// What the overlay shows. Shared between the drag code (writes) and the overlay
/// window's WM_PAINT (reads).
struct View {
    /// The window's rect on screen (the work area, plus a gaming zone over the taskbar).
    area: Rect,
    zones: Vec<ResolvedZone>,
    selected: Vec<usize>,
    style: OverlayStyle,
    dpi: u32,
    font: Option<Font>,
}

pub struct Overlay {
    hwnd: Hwnd,
    view: Rc<RefCell<View>>,
    area: Rect,
    visible: Cell<bool>,
}

impl Overlay {
    /// Creates the overlay (hidden) covering `area` (screen coordinates).
    pub fn create(area: Rect, zones: Vec<ResolvedZone>, dpi: u32, style: &OverlayStyle) -> gz_win::Result<Overlay> {
        let view =
            Rc::new(RefCell::new(View { area, zones, selected: Vec::new(), style: style.clone(), dpi, font: None }));
        view.borrow_mut().make_font();
        let spec = WindowSpec {
            class: "GamingZones.Overlay",
            title: "GamingZones zones",
            style: ws::POPUP,
            ex_style: ws::EX_TOOLWINDOW | ws::EX_LAYERED | ws::EX_TRANSPARENT | ws::EX_NOACTIVATE,
            rect: Some(area),
            icon: false,
            background: Background::None,
        };
        let hwnd = window::create_window(&spec, Box::new(Painter { view: view.clone() }))?;
        let alpha = (style.opacity.min(100) as u32 * 255 / 100) as u8;
        if let Err(e) = window::set_layered(hwnd, paint::colorref(TRANSPARENT), alpha) {
            let _ = window::destroy_window(hwnd);
            return Err(e);
        }
        Ok(Overlay { hwnd, view, area, visible: Cell::new(false) })
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Shows the overlay directly under `dragged` (if it isn't showing already).
    pub fn show_below(&self, dragged: Hwnd) {
        if self.visible.get() {
            return;
        }
        match window::show_below(self.hwnd, dragged, self.area) {
            Ok(()) => self.visible.set(true),
            Err(e) => log::warn!("Showing the zone overlay failed: {}", e.message()),
        }
    }

    pub fn hide(&self) {
        if self.visible.replace(false) {
            window::hide(self.hwnd);
        }
    }

    pub fn set_selected(&self, selected: &[usize]) {
        let mut view = self.view.borrow_mut();
        if view.selected != selected {
            view.selected = selected.to_vec();
            drop(view);
            window::invalidate(self.hwnd);
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if let Err(e) = window::destroy_window(self.hwnd) {
            log::warn!("Destroying the zone overlay failed: {}", e.message());
        }
    }
}

impl View {
    /// Zone numbers sized to fit the smallest zone, within sensible limits.
    fn make_font(&mut self) {
        if !self.style.show_numbers {
            return;
        }
        let smallest = self.zones.iter().filter(|z| !z.disabled).map(|z| z.rect.width().min(z.rect.height())).min();
        let (lo, hi) = (dpi::scale(20, self.dpi), dpi::scale(140, self.dpi));
        let height = (smallest.unwrap_or(0) / 3).clamp(lo, hi);
        self.font = Font::sized("Segoe UI", height, 600).ok();
    }

    /// Zones that are drawn: enabled ones, and disabled ones unless they're hidden.
    /// Disabled zones with no area (e.g. squeezed to 0 px by the gaming zone) are skipped.
    fn drawn(&self) -> impl Iterator<Item = (usize, &ResolvedZone)> {
        let greyed = self.style.disabled_zones == DisabledZones::Greyed;
        self.zones.iter().enumerate().filter(move |(_, z)| !z.disabled || (greyed && !z.rect.is_empty()))
    }

    fn paint(&self, hwnd: Hwnd) {
        let to_client = |r: Rect| r.offset(-self.area.left, -self.area.top);
        let border = dpi::scale(2, self.dpi);
        paint::paint(hwnd, |p| {
            // Outside the zones: the transparent colour. Every pixel is painted once, so
            // there is no flicker when the highlight moves.
            for (_, z) in self.drawn() {
                p.exclude(to_client(z.rect));
            }
            p.fill(Rect::new(0, 0, self.area.width(), self.area.height()), TRANSPARENT);
            p.reset_clip();
            for (i, z) in self.drawn() {
                let r = to_client(z.rect);
                let fill = if z.disabled {
                    self.style.disabled_color
                } else if self.selected.contains(&i) {
                    self.style.highlight_color
                } else {
                    self.style.zone_color
                };
                p.fill(Rect::new(r.left + border, r.top + border, r.right - border, r.bottom - border), fill);
                p.frame(r, self.style.border_color, border);
                // Disabled zones get no number: they can't be snapped to (and may be tiny).
                if let (Some(font), false) = (&self.font, z.disabled) {
                    p.text_centered(r, &zone_label(&self.zones, i), font, self.style.number_color);
                }
            }
        });
    }
}

struct Painter {
    view: Rc<RefCell<View>>,
}

impl WindowHandler for Painter {
    fn handle(&mut self, hwnd: Hwnd, m: &Message) -> Option<isize> {
        match m.msg {
            msg::PAINT => {
                match self.view.try_borrow() {
                    Ok(view) => view.paint(hwnd),
                    // Being updated right now; paint again once that's done.
                    Err(_) => window::invalidate(hwnd),
                }
                Some(0)
            }
            msg::ERASEBKGND => Some(1), // WM_PAINT covers every pixel
            msg::NCHITTEST => Some(msg::HTTRANSPARENT),
            msg::MOUSEACTIVATE => Some(msg::MA_NOACTIVATE),
            _ => None,
        }
    }
}
