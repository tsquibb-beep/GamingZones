//! GamingZones Ruler: a test window that shows its own geometry live, in physical pixels.
//!
//! - Type a client size and press Enter (or Apply) to get exactly that client area.
//! - "Fixed size" removes `WS_THICKFRAME` (and the maximise box), like a windowed game
//!   that can't be resized, while keeping the client size.
#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

use gz_core::Rect;
use gz_win::controls::{self, Control, Font};
use gz_win::window::{self, msg, Background, Message, MessageBoxKind, WindowHandler, WindowSpec};
use gz_win::{dpi, inspect, monitor, styles, styles::ws, Hwnd};

/// IDOK, so Enter in the window clicks Apply.
const ID_APPLY: u16 = 1;
const ID_FIXED: u16 = 2;
const ID_WIDTH: u16 = 3;
const ID_HEIGHT: u16 = 4;

/// Posted from WM_CREATE so the first sizing happens outside window creation.
const WM_INITIAL_SIZE: u32 = msg::APP + 1;
const INITIAL_CLIENT: (i32, i32) = (1280, 720);
const MAX_SIZE: i32 = 16384;

struct Controls {
    size_label: Hwnd,
    width: Hwnd,
    times: Hwnd,
    height: Hwnd,
    apply: Hwnd,
    fixed: Hwnd,
    info: Hwnd,
}

#[derive(Default)]
struct Ruler {
    controls: Option<Controls>,
    ui_font: Option<Font>,
    mono_font: Option<Font>,
    dpi: u32,
    status: String,
    shown_info: String,
}

impl Ruler {
    fn create_controls(&mut self, hwnd: Hwnd) -> gz_win::Result<()> {
        let c = Controls {
            size_label: controls::create(hwnd, Control::Label, "Client size:", 0)?,
            width: controls::create(hwnd, Control::NumberEdit, "", ID_WIDTH)?,
            times: controls::create(hwnd, Control::Label, "×", 0)?,
            height: controls::create(hwnd, Control::NumberEdit, "", ID_HEIGHT)?,
            apply: controls::create(hwnd, Control::DefaultButton, "Apply", ID_APPLY)?,
            fixed: controls::create(hwnd, Control::CheckBox, "Fixed size (remove WS_THICKFRAME)", ID_FIXED)?,
            info: controls::create(hwnd, Control::Label, "", 0)?,
        };
        self.controls = Some(c);
        self.dpi = dpi::window_dpi(hwnd);
        self.set_fonts();
        Ok(())
    }

    /// (Re)creates fonts for the current DPI. Old fonts are dropped only after the
    /// controls have switched to the new ones.
    fn set_fonts(&mut self) {
        let (Ok(ui), Ok(mono)) = (Font::message_font(self.dpi), Font::monospace(10, self.dpi)) else {
            return;
        };
        if let Some(c) = &self.controls {
            for ctl in [c.size_label, c.width, c.times, c.height, c.apply, c.fixed] {
                controls::set_font(ctl, &ui);
            }
            controls::set_font(c.info, &mono);
        }
        self.ui_font = Some(ui);
        self.mono_font = Some(mono);
    }

    fn layout(&self, hwnd: Hwnd) {
        let Some(c) = &self.controls else { return };
        let Ok(client) = inspect::client_rect_screen(hwnd) else { return };
        let s = |v: i32| dpi::scale(v, self.dpi);
        let (margin, row) = (s(12), s(26));
        let mut x = margin;
        let mut place = |ctl: Hwnd, width: i32, dy: i32, h: i32| {
            window::move_child(ctl, Rect::from_xywh(x, margin + dy, width, h));
            x += width + s(6);
        };
        place(c.size_label, s(70), s(5), s(20));
        place(c.width, s(70), 0, row);
        place(c.times, s(12), s(5), s(20));
        place(c.height, s(70), 0, row);
        place(c.apply, s(80), 0, row);
        place(c.fixed, s(300), s(3), s(20));
        let top = margin + row + s(12);
        window::move_child(
            c.info,
            Rect::new(margin, top, (client.width() - margin).max(margin), (client.height() - margin).max(top)),
        );
    }

    fn refresh(&mut self, hwnd: Hwnd) {
        let Some(c) = &self.controls else { return };
        let text = describe(hwnd, &self.status);
        if text != self.shown_info {
            let _ = window::set_text(c.info, &text);
            self.shown_info = text;
        }
        if let Ok(client) = inspect::client_rect_screen(hwnd) {
            let _ = window::set_text(hwnd, &format!("GamingZones Ruler — {}×{}", client.width(), client.height()));
        }
    }

    /// Sizes the window so its client area is exactly `w`×`h`, measuring the real frame
    /// rather than trusting AdjustWindowRectEx.
    fn apply_client_size(&mut self, hwnd: Hwnd, w: i32, h: i32) {
        let mut result = Err(String::new());
        // Second pass only if the frame changed under us (e.g. a menu bar re-wrapped).
        for _ in 0..2 {
            let (Ok(win), Ok(client)) = (inspect::window_rect(hwnd), inspect::client_rect_screen(hwnd)) else {
                return;
            };
            if (client.width(), client.height()) == (w, h) {
                result = Ok(());
                break;
            }
            let frame_w = win.width() - client.width();
            let frame_h = win.height() - client.height();
            if let Err(e) = window::set_window_size(hwnd, w + frame_w, h + frame_h) {
                result = Err(e.message());
                break;
            }
        }
        if result.is_err() {
            if let Ok(client) = inspect::client_rect_screen(hwnd) {
                if (client.width(), client.height()) == (w, h) {
                    result = Ok(());
                }
            }
        }
        self.status = match result {
            Ok(()) => format!("Applied client {w}×{h}."),
            Err(e) if !e.is_empty() => format!("Asked for {w}×{h}; SetWindowPos failed: {e}"),
            Err(_) => {
                let got = inspect::client_rect_screen(hwnd).map(|c| format!("{}×{}", c.width(), c.height()));
                format!("Asked for {w}×{h}; Windows gave {}.", got.unwrap_or_else(|_| "?".into()))
            }
        };
        // The WM_SIZE caused by SetWindowPos arrived re-entrantly and was not handled.
        self.layout(hwnd);
        self.refresh(hwnd);
    }

    fn on_apply(&mut self, hwnd: Hwnd) {
        let Some(c) = &self.controls else { return };
        let parse = |ctl: Hwnd| window::text(ctl).trim().parse::<i32>().ok().filter(|v| (1..=MAX_SIZE).contains(v));
        match (parse(c.width), parse(c.height)) {
            (Some(w), Some(h)) => self.apply_client_size(hwnd, w, h),
            _ => {
                self.status = format!("Enter a width and height from 1 to {MAX_SIZE}.");
                self.refresh(hwnd);
            }
        }
    }

    fn on_fixed_toggled(&mut self, hwnd: Hwnd) {
        let Some(c) = &self.controls else { return };
        let fixed = controls::is_checked(c.fixed);
        let Ok(client) = inspect::client_rect_screen(hwnd) else { return };
        let style = window::style(hwnd);
        let resizable = ws::THICKFRAME | ws::MAXIMIZEBOX;
        let new_style = if fixed { style & !resizable } else { style | resizable };
        if let Err(e) = window::set_style(hwnd, new_style) {
            self.status = format!("Changing the style failed: {}", e.message());
            self.refresh(hwnd);
            return;
        }
        // The frame changed size; put the client area back to what it was.
        self.apply_client_size(hwnd, client.width(), client.height());
    }

    fn on_dpi_changed(&mut self, hwnd: Hwnd, m: &Message) {
        self.dpi = m.loword_wparam() as u32;
        self.set_fonts();
        if let Some(rect) = window::dpi_changed_rect(m) {
            let _ = window::set_window_rect(hwnd, rect);
        }
        self.layout(hwnd);
        self.refresh(hwnd);
    }
}

fn describe(hwnd: Hwnd, status: &str) -> String {
    let mut out = String::new();
    let mut line = |label: &str, value: String| out.push_str(&format!("{label:<18}{value}\r\n"));
    let (Ok(win), Ok(client)) = (inspect::window_rect(hwnd), inspect::client_rect_screen(hwnd)) else {
        return String::new();
    };
    line("Client size", format!("{} × {}", client.width(), client.height()));
    line("Client rect", format!("{client}  (screen)"));
    line("Window rect", format!("{win}"));
    match inspect::frame_bounds(hwnd) {
        Ok(frame) => {
            line("Frame bounds", format!("{frame}"));
            line("Invisible border", format!("{}", win.insets_to(&frame)));
            line("Frame → client", format!("{}", frame.insets_to(&client)));
        }
        Err(e) => line("Frame bounds", format!("unavailable: {}", e.message())),
    }
    line("Window → client", format!("{}", win.insets_to(&client)));
    let window_dpi = dpi::window_dpi(hwnd);
    line("DPI", format!("{window_dpi} ({}%)", dpi::percent(window_dpi)));
    if let Some(m) = monitor::for_window(hwnd) {
        line("Monitor", format!("{} {}", m.device, m.rect));
        line("Work area", format!("{}", m.work));
    }
    let style = window::style(hwnd);
    line("Style", styles::describe_style(style));
    line("Resizable", if style & ws::THICKFRAME != 0 { "yes".into() } else { "no (fixed)".into() });
    if !status.is_empty() {
        out.push_str("\r\n");
        out.push_str(status);
    }
    out
}

impl WindowHandler for Ruler {
    fn handle(&mut self, hwnd: Hwnd, m: &Message) -> Option<isize> {
        match m.msg {
            msg::CREATE => {
                if let Err(e) = self.create_controls(hwnd) {
                    window::message_box(Some(hwnd), &e.message(), "Ruler", MessageBoxKind::Error);
                    return Some(-1); // abort creation
                }
                let _ = window::post_message(hwnd, WM_INITIAL_SIZE, 0, 0);
            }
            WM_INITIAL_SIZE => {
                let (w, h) = INITIAL_CLIENT;
                if let Some(c) = &self.controls {
                    let _ = window::set_text(c.width, &w.to_string());
                    let _ = window::set_text(c.height, &h.to_string());
                    controls::focus(c.width);
                    controls::select_all(c.width);
                }
                self.apply_client_size(hwnd, w, h);
                self.status.clear();
                self.refresh(hwnd);
            }
            msg::SIZE => {
                self.layout(hwnd);
                self.refresh(hwnd);
            }
            msg::MOVE | msg::EXITSIZEMOVE => self.refresh(hwnd),
            msg::DPICHANGED => self.on_dpi_changed(hwnd, m),
            msg::COMMAND => {
                const BN_CLICKED: u16 = 0;
                match (m.loword_wparam(), m.hiword_wparam()) {
                    (ID_APPLY, BN_CLICKED) => self.on_apply(hwnd),
                    (ID_FIXED, BN_CLICKED) => self.on_fixed_toggled(hwnd),
                    _ => return None,
                }
            }
            msg::DESTROY => window::post_quit(0),
            _ => return None,
        }
        Some(0)
    }
}

fn main() {
    let spec = WindowSpec {
        class: "GamingZones.Ruler",
        title: "GamingZones Ruler",
        style: ws::OVERLAPPEDWINDOW | ws::CLIPCHILDREN,
        ex_style: ws::EX_CONTROLPARENT,
        rect: None,
        icon: true,
        background: Background::Dialog,
    };
    let hwnd = match window::create_window(&spec, Box::new(Ruler::default())) {
        Ok(hwnd) => hwnd,
        Err(e) => {
            window::message_box(None, &e.message(), "Ruler", MessageBoxKind::Error);
            std::process::exit(1);
        }
    };
    window::show_window(hwnd);
    std::process::exit(window::run_message_loop(Some(hwnd)));
}
