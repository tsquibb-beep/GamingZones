//! The layout selector (`Win+Shift+``): a popup in the middle of one monitor with a card
//! per layout, drawn with GDI. It exists only while it's open.
//!
//! Number keys (1..9, 0 = no zones), a click, or the arrow keys and Enter choose; Esc or
//! clicking anywhere else closes it. It posts `done_msg` to the main window with the chosen
//! card (or [`CANCELLED`]) and the main window then drops it.

use std::cell::RefCell;
use std::rc::Rc;

use gz_core::layout::{Layout, ResolvedZone};
use gz_core::{Color, DisabledZones, Rect};
use gz_win::controls::Font;
use gz_win::monitor::MonitorInfo;
use gz_win::window::{self, msg, Background, Message, WindowHandler, WindowSpec};
use gz_win::{dpi, paint, styles::ws, Hwnd};

/// `wParam` of `done_msg` when nothing was chosen.
pub const CANCELLED: usize = usize::MAX;

const BACKGROUND: Color = Color::rgb(0x20, 0x20, 0x20);
const WINDOW_BORDER: Color = Color::rgb(0x3A, 0x3A, 0x3A);
const CARD: Color = Color::rgb(0x2B, 0x2B, 0x2B);
const CARD_HOVER: Color = Color::rgb(0x33, 0x33, 0x33);
const THUMB: Color = Color::rgb(0x16, 0x16, 0x16);
const ZONE: Color = Color::rgb(0x3A, 0x46, 0x54);
const ZONE_BORDER: Color = Color::rgb(0x6B, 0x7B, 0x8C);
const DISABLED: Color = Color::rgb(0x30, 0x30, 0x30);
/// The logo's green, as in the editor.
const GAMING: Color = Color::rgb(0x27, 0xA0, 0x60);
const ACCENT: Color = Color::rgb(0x00, 0x8C, 0xFF);
const TEXT: Color = Color::rgb(0xE6, 0xE6, 0xE6);
const DIM: Color = Color::rgb(0x9A, 0x9A, 0x9A);
const FONT: &str = "Segoe UI";

/// One card: a layout, or "No zones".
pub struct Entry {
    /// `None` = no zones.
    pub layout: Option<String>,
    name: String,
    zones: Vec<ResolvedZone>,
    /// The number key that picks it.
    key: Option<u8>,
}

impl Entry {
    /// Cards for `layouts` (in list order, numbered 1..9) on `m`, and a last "No zones" card (0).
    pub fn all(layouts: &[Layout], m: &MonitorInfo, deadzone_px: i32) -> Vec<Entry> {
        let mut out: Vec<Entry> = layouts
            .iter()
            .enumerate()
            .map(|(i, l)| Entry {
                layout: Some(l.id.clone()),
                name: l.name.clone(),
                zones: l.resolve(m.work, m.rect, deadzone_px).unwrap_or_default(),
                key: (i < 9).then_some(i as u8 + 1),
            })
            .collect();
        out.push(Entry { layout: None, name: "No zones".into(), zones: Vec::new(), key: Some(0) });
        out
    }
}

/// Sizes in pixels at the monitor's DPI.
struct Metrics {
    pad: i32,
    gap: i32,
    title_h: i32,
    hint_h: i32,
    card_pad: i32,
    label_h: i32,
    thumb_w: i32,
    thumb_h: i32,
}

impl Metrics {
    fn new(work: Rect, dpi: u32) -> Metrics {
        let s = |v| dpi::scale(v, dpi);
        let thumb_w = s(240);
        // The thumbnail has the work area's shape, within limits (an ultrawide is very flat).
        let thumb_h = (thumb_w as i64 * work.height() as i64 / work.width().max(1) as i64) as i32;
        Metrics {
            pad: s(20),
            gap: s(12),
            title_h: s(32),
            hint_h: s(28),
            card_pad: s(10),
            label_h: s(26),
            thumb_w,
            thumb_h: thumb_h.clamp(s(60), s(200)),
        }
    }

    fn card_size(&self) -> (i32, i32) {
        (self.thumb_w + 2 * self.card_pad, self.card_pad + self.thumb_h + self.label_h + self.card_pad / 2)
    }
}

/// Lays out `count` cards of `card` size in rows of at most as many as fit in `max_width`.
/// Returns the window size and each card's rect (client coordinates).
fn grid(count: usize, card: (i32, i32), max_width: i32, m: &Metrics) -> ((i32, i32), Vec<Rect>) {
    let fit = ((max_width - 2 * m.pad + m.gap) / (card.0 + m.gap)).max(1) as usize;
    let cols = count.clamp(1, fit);
    let rows = count.div_ceil(cols).max(1);
    let width = 2 * m.pad + cols as i32 * card.0 + (cols as i32 - 1) * m.gap;
    let height = 2 * m.pad + m.title_h + rows as i32 * card.1 + (rows as i32 - 1) * m.gap + m.hint_h;
    let cards = (0..count)
        .map(|i| {
            let (col, row) = ((i % cols) as i32, (i / cols) as i32);
            let left = m.pad + col * (card.0 + m.gap);
            let top = m.pad + m.title_h + row * (card.1 + m.gap);
            Rect::new(left, top, left + card.0, top + card.1)
        })
        .collect();
    ((width, height), cards)
}

/// `r` (in `from`) scaled into `to`.
fn scale_rect(r: Rect, from: Rect, to: Rect) -> Rect {
    let sx = |x: i32| to.left + ((x - from.left) as i64 * to.width() as i64 / from.width().max(1) as i64) as i32;
    let sy = |y: i32| to.top + ((y - from.top) as i64 * to.height() as i64 / from.height().max(1) as i64) as i32;
    Rect::new(sx(r.left), sy(r.top), sx(r.right), sy(r.bottom))
}

/// The card a number key picks (`vk` 0-9 on the main keys or the numpad).
fn card_for_key(entries: &[Entry], vk: u16) -> Option<usize> {
    let digit = match vk {
        0x30..=0x39 => vk - 0x30,
        0x60..=0x69 => vk - 0x60,
        _ => return None,
    } as u8;
    entries.iter().position(|e| e.key == Some(digit))
}

/// Where the arrow keys move the selection in a grid `cols` wide.
fn arrow_move(selected: usize, count: usize, cols: usize, vk: u16) -> usize {
    let last = count.saturating_sub(1);
    match vk {
        0x25 => selected.saturating_sub(1),                 // Left
        0x27 => (selected + 1).min(last),                   // Right
        0x26 if selected >= cols => selected - cols,        // Up
        0x28 if selected + cols <= last => selected + cols, // Down
        0x24 => 0,                                          // Home
        0x23 => last,                                       // End
        _ => selected,
    }
}

struct Fonts {
    title: Option<Font>,
    label: Option<Font>,
    number: Option<Font>,
    small: Option<Font>,
}

struct View {
    entries: Vec<Entry>,
    cards: Vec<Rect>,
    cols: usize,
    size: (i32, i32),
    metrics: Metrics,
    work: Rect,
    title: String,
    /// The card in use on the monitor.
    current: Option<usize>,
    selected: usize,
    disabled_zones: DisabledZones,
    fonts: Fonts,
    dpi: u32,
    main: Hwnd,
    done_msg: u32,
    finished: bool,
}

pub struct Selector {
    hwnd: Hwnd,
    view: Rc<RefCell<View>>,
    pub monitor: MonitorInfo,
    /// The window that had the focus before; it gets it back when the selector closes.
    pub previous: Option<Hwnd>,
}

impl Selector {
    /// Opens the selector in the middle of `m`'s work area and gives it the focus.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        main: Hwnd,
        done_msg: u32,
        m: MonitorInfo,
        monitor_name: &str,
        entries: Vec<Entry>,
        current: Option<usize>,
        disabled_zones: DisabledZones,
        previous: Option<Hwnd>,
    ) -> gz_win::Result<Selector> {
        let metrics = Metrics::new(m.work, m.dpi);
        let max_width = m.work.width() * 9 / 10;
        let (size, cards) = grid(entries.len(), metrics.card_size(), max_width, &metrics);
        let cols = cards.iter().take_while(|c| c.top == cards[0].top).count().max(1);
        let s = |v| dpi::scale(v, m.dpi);
        let fonts = Fonts {
            title: Font::sized(FONT, s(17), 600).ok(),
            label: Font::sized(FONT, s(14), 400).ok(),
            number: Font::sized(FONT, s(15), 700).ok(),
            small: Font::sized(FONT, s(12), 400).ok(),
        };
        let title = if monitor_name.is_empty() {
            format!("Layout for {}", m.device.trim_start_matches(r"\\.\"))
        } else {
            format!("Layout for {monitor_name}")
        };
        let view = Rc::new(RefCell::new(View {
            entries,
            cards,
            cols,
            size,
            metrics,
            work: m.work,
            title,
            current,
            selected: current.unwrap_or(0),
            disabled_zones,
            fonts,
            dpi: m.dpi,
            main,
            done_msg,
            finished: false,
        }));
        let (w, h) = size;
        let (left, top) = (m.work.left + (m.work.width() - w) / 2, m.work.top + (m.work.height() - h) / 2);
        let spec = WindowSpec {
            class: "GamingZones.Selector",
            title: "GamingZones layouts",
            style: ws::POPUP,
            ex_style: ws::EX_TOOLWINDOW | ws::EX_TOPMOST,
            rect: Some(Rect::new(left, top, left + w, top + h)),
            icon: false,
            background: Background::None,
        };
        let hwnd = window::create_window(&spec, Box::new(SelectorWindow { view: view.clone() }))?;
        window::round_corners(hwnd);
        window::show_window(hwnd);
        window::activate(hwnd);
        Ok(Selector { hwnd, view, monitor: m, previous })
    }

    /// The layout of card `i` (`Some(None)` = no zones), or `None` for no such card.
    pub fn layout_of(&self, i: usize) -> Option<Option<String>> {
        self.view.borrow().entries.get(i).map(|e| e.layout.clone())
    }
}

impl Drop for Selector {
    fn drop(&mut self) {
        if let Err(e) = window::destroy_window(self.hwnd) {
            log::warn!("Destroying the layout selector failed: {}", e.message());
        }
    }
}

impl View {
    fn card_at(&self, x: i32, y: i32) -> Option<usize> {
        self.cards.iter().position(|c| c.contains(x, y))
    }

    /// Reports the choice once; the main window then drops the selector.
    fn finish(&mut self, choice: Option<usize>) {
        if !self.finished {
            self.finished = true;
            let _ = window::post_message(self.main, self.done_msg, choice.unwrap_or(CANCELLED), 0);
        }
    }

    fn paint(&self, hwnd: Hwnd) {
        let (w, h) = self.size;
        let m = &self.metrics;
        let one = dpi::scale(1, self.dpi).max(1);
        paint::paint_buffered(hwnd, w, h, |p| {
            p.fill(Rect::new(0, 0, w, h), BACKGROUND);
            p.frame(Rect::new(0, 0, w, h), WINDOW_BORDER, one);
            if let Some(font) = &self.fonts.title {
                p.text_left(Rect::new(m.pad, m.pad, w - m.pad, m.pad + m.title_h - m.gap / 2), &self.title, font, TEXT);
            }
            for (i, (entry, &card)) in self.entries.iter().zip(&self.cards).enumerate() {
                self.paint_card(p, entry, card, i);
            }
            if let Some(font) = &self.fonts.small {
                let hint = "Press a number or click a layout. Esc closes. Windows don't move.";
                p.text_left(Rect::new(m.pad, h - m.pad - m.hint_h, w - m.pad, h - m.pad), hint, font, DIM);
            }
        });
    }

    fn paint_card(&self, p: &mut paint::Painter, entry: &Entry, card: Rect, i: usize) {
        let m = &self.metrics;
        let one = dpi::scale(1, self.dpi).max(1);
        let selected = i == self.selected;
        p.fill(card, if selected { CARD_HOVER } else { CARD });
        if selected {
            p.frame(card, ACCENT, 2 * one);
        }
        let thumb = Rect::new(
            card.left + m.card_pad,
            card.top + m.card_pad,
            card.right - m.card_pad,
            card.top + m.card_pad + m.thumb_h,
        );
        p.fill(thumb, THUMB);
        let greyed = self.disabled_zones == DisabledZones::Greyed;
        for z in &entry.zones {
            if z.rect.is_empty() || (z.disabled && !greyed) {
                continue;
            }
            let visible = z.rect.intersect(&self.work);
            if visible.is_empty() {
                continue;
            }
            let r = scale_rect(visible, self.work, thumb);
            let fill = if z.disabled {
                DISABLED
            } else if z.gaming {
                GAMING
            } else {
                ZONE
            };
            p.fill(r, ZONE_BORDER);
            p.fill(Rect::new(r.left + one, r.top + one, r.right - one, r.bottom - one), fill);
            if z.gaming && !z.disabled {
                if let Some(font) = &self.fonts.small {
                    p.text_centered(r, &format!("{}×{}", z.rect.width(), z.rect.height()), font, TEXT);
                }
            }
        }
        if entry.layout.is_none() {
            if let Some(font) = &self.fonts.small {
                p.text_centered(thumb, "Zones off on this monitor", font, DIM);
            }
        }
        let label = Rect::new(thumb.left, thumb.bottom, thumb.right, thumb.bottom + m.label_h);
        let mut name_left = label.left;
        if let (Some(key), Some(font)) = (entry.key, &self.fonts.number) {
            p.text_left(label, &key.to_string(), font, ACCENT);
            name_left += dpi::scale(18, self.dpi);
        }
        let in_use = Some(i) == self.current;
        let tag_w = if in_use { dpi::scale(48, self.dpi) } else { 0 };
        if let Some(font) = &self.fonts.label {
            p.text_left(Rect::new(name_left, label.top, label.right - tag_w, label.bottom), &entry.name, font, TEXT);
        }
        if let (true, Some(font)) = (in_use, &self.fonts.small) {
            p.text_right(label, "in use", font, DIM);
        }
    }
}

struct SelectorWindow {
    view: Rc<RefCell<View>>,
}

/// Client coordinates from a mouse message's `lParam` (signed 16-bit each).
fn mouse_pos(m: &Message) -> (i32, i32) {
    ((m.lparam & 0xFFFF) as u16 as i16 as i32, ((m.lparam >> 16) & 0xFFFF) as u16 as i16 as i32)
}

impl WindowHandler for SelectorWindow {
    fn handle(&mut self, hwnd: Hwnd, m: &Message) -> Option<isize> {
        let Ok(mut view) = self.view.try_borrow_mut() else {
            // Being read right now (Selector::layout_of); try painting again later.
            if m.msg == msg::PAINT {
                window::invalidate(hwnd);
            }
            return None;
        };
        match m.msg {
            msg::PAINT => view.paint(hwnd),
            msg::ERASEBKGND => return Some(1), // WM_PAINT covers every pixel
            msg::KEYDOWN | msg::SYSKEYDOWN => {
                let vk = m.wparam as u16;
                match vk {
                    0x1B => view.finish(None), // Esc
                    0x0D => {
                        // Enter
                        let i = view.selected;
                        view.finish(Some(i));
                    }
                    _ => {
                        if let Some(i) = card_for_key(&view.entries, vk) {
                            view.finish(Some(i));
                        } else {
                            let moved = arrow_move(view.selected, view.entries.len(), view.cols, vk);
                            if moved != view.selected {
                                view.selected = moved;
                                window::invalidate(hwnd);
                            }
                        }
                    }
                }
            }
            msg::MOUSEMOVE => {
                let (x, y) = mouse_pos(m);
                if let Some(i) = view.card_at(x, y).filter(|&i| i != view.selected) {
                    view.selected = i;
                    window::invalidate(hwnd);
                }
            }
            msg::LBUTTONDOWN => {
                let (x, y) = mouse_pos(m);
                if let Some(i) = view.card_at(x, y) {
                    view.finish(Some(i));
                }
            }
            // Clicking another window or Alt+Tab closes it.
            msg::ACTIVATE if m.loword_wparam() == 0 => view.finish(None),
            _ => return None,
        }
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics::new(Rect::new(0, 0, 6144, 2500), 120)
    }

    fn entries(n: usize) -> Vec<Entry> {
        (0..n)
            .map(|i| Entry {
                layout: Some(format!("l{i}")),
                name: format!("L{i}"),
                zones: Vec::new(),
                key: (i < 9).then_some(i as u8 + 1),
            })
            .chain([Entry { layout: None, name: "No zones".into(), zones: Vec::new(), key: Some(0) }])
            .collect()
    }

    #[test]
    fn thumbnail_has_the_work_areas_shape_within_limits() {
        let m = metrics();
        assert_eq!(m.thumb_w, 300);
        assert_eq!(m.thumb_h, 122); // 300 × 2500 / 6144
        let tall = Metrics::new(Rect::new(0, 0, 1000, 4000), 96);
        assert_eq!(tall.thumb_h, 200);
    }

    #[test]
    fn cards_wrap_into_rows() {
        let m = metrics();
        let card = m.card_size();
        let ((w, _), cards) = grid(6, card, 10_000, &m);
        assert!(cards.iter().all(|c| c.top == cards[0].top));
        assert_eq!(w, 2 * m.pad + 6 * card.0 + 5 * m.gap);
        // Room for three per row: 6 cards make 2 rows of 3.
        let narrow = 2 * m.pad + 3 * card.0 + 2 * m.gap;
        let ((w, h), cards) = grid(6, card, narrow, &m);
        assert_eq!(w, narrow);
        assert_eq!(cards[3].left, cards[0].left);
        assert_eq!(cards[3].top, cards[0].bottom + m.gap);
        assert_eq!(h, cards[5].bottom + m.hint_h + m.pad);
        // Never zero columns.
        let ((_, _), cards) = grid(2, card, 10, &m);
        assert_eq!(cards[1].top, cards[0].bottom + m.gap);
    }

    #[test]
    fn number_keys_and_arrows() {
        let e = entries(10);
        assert_eq!(card_for_key(&e, 0x31), Some(0));
        assert_eq!(card_for_key(&e, 0x69), Some(8)); // Numpad9
        assert_eq!(card_for_key(&e, 0x30), Some(10)); // 0 = No zones
        assert_eq!(card_for_key(&entries(2), 0x35), None);
        assert_eq!(card_for_key(&e, 0x41), None);

        // 7 cards, 3 per row.
        assert_eq!(arrow_move(0, 7, 3, 0x25), 0);
        assert_eq!(arrow_move(0, 7, 3, 0x27), 1);
        assert_eq!(arrow_move(6, 7, 3, 0x27), 6);
        assert_eq!(arrow_move(1, 7, 3, 0x28), 4);
        assert_eq!(arrow_move(4, 7, 3, 0x28), 4); // nothing below
        assert_eq!(arrow_move(4, 7, 3, 0x26), 1);
        assert_eq!(arrow_move(4, 7, 3, 0x23), 6);
    }

    #[test]
    fn thumbnails_scale_zones() {
        let work = Rect::new(-100, 0, 1900, 1000);
        let thumb = Rect::new(10, 10, 210, 110);
        assert_eq!(scale_rect(Rect::new(-100, 0, 900, 500), work, thumb), Rect::new(10, 10, 110, 60));
        assert_eq!(scale_rect(work, work, thumb), thumb);
    }
}
