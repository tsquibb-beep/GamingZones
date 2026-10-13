//! "Preview on screen" from the editor: the real zone overlay for the layout being edited,
//! on the monitor being edited, just under the editor's window. It goes when the editor
//! says so or its window closes (a WinEvent hook on that one window), and is hidden while
//! a window is being dragged.

use gz_core::config::Settings;
use gz_core::ipc;
use gz_core::Rect;
use gz_win::monitor::{self, MonitorIds};
use gz_win::winevent::{event, Delivery, Filter, WinEventHook};
use gz_win::{process, Hwnd};

use crate::overlay::Overlay;

pub struct Preview {
    pub editor: Hwnd,
    overlay: Option<Overlay>,
    /// What the overlay was made for; another area or DPI needs a new window.
    made_for: (Rect, u32),
    suspended: bool,
    _closed_hook: Option<WinEventHook>,
}

impl Preview {
    /// `closed_msg` is posted to `main` when the editor's window is destroyed.
    pub fn new(main: Hwnd, editor: Hwnd, closed_msg: u32) -> Preview {
        let hook = WinEventHook::new(
            event::OBJECT_DESTROY,
            event::OBJECT_DESTROY,
            process::window_pid(editor),
            Filter { window: Some(editor), cursor: false },
            Delivery { target: main, msg: closed_msg, coalesce: false },
        )
        .map_err(|e| log::warn!("Preview: could not watch the editor window: {}", e.message()))
        .ok();
        Preview { editor, overlay: None, made_for: (Rect::default(), 0), suspended: false, _closed_hook: hook }
    }

    /// Shows (or updates) the preview.
    pub fn update(&mut self, p: &ipc::Preview, settings: &Settings) -> Result<(), String> {
        let ids = MonitorIds::query();
        let m = monitor::all()
            .into_iter()
            .find(|m| ids.id(m).id == p.monitor)
            .ok_or_else(|| format!("monitor {} isn't connected", p.monitor))?;
        let zones = p.layout.resolve(m.work, m.rect, settings.snapping.deadzone_px).map_err(|e| e.to_string())?;
        let area = zones.iter().filter(|z| z.gaming && !z.disabled).fold(m.work, |a, z| a.union(&z.rect));
        match &self.overlay {
            Some(o) if self.made_for == (area, m.dpi) => o.set_zones(zones),
            _ => {
                self.overlay = None;
                let o = Overlay::create(area, zones, m.dpi, &settings.overlay).map_err(|e| e.message())?;
                self.overlay = Some(o);
                self.made_for = (area, m.dpi);
            }
        }
        if let Some(o) = &self.overlay {
            o.set_selected(&p.selected);
            if !self.suspended {
                o.show_below(self.editor);
            }
        }
        Ok(())
    }

    /// Hidden while a window is dragged (the drag has its own overlay).
    pub fn suspend(&mut self) {
        self.suspended = true;
        if let Some(o) = &self.overlay {
            o.hide();
        }
    }

    pub fn resume(&mut self) {
        self.suspended = false;
        if let Some(o) = &self.overlay {
            o.show_below(self.editor);
        }
    }
}
