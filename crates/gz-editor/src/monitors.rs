//! The connected monitors, as the editor shows them.

use gz_core::Rect;
use gz_win::monitor::{self, MonitorIds, MonitorInfo};

pub struct Monitor {
    pub info: MonitorInfo,
    /// Stable id, as used in `applied.json`.
    pub id: String,
    /// Friendly name ("DELL U5226KW"), may be empty.
    pub name: String,
}

impl Monitor {
    pub fn work(&self) -> Rect {
        self.info.work
    }

    pub fn rect(&self) -> Rect {
        self.info.rect
    }

    /// "1 · DELL U5226KW · 6144 × 2560"
    pub fn label(&self, n: usize) -> String {
        let r = self.info.rect;
        let name = if self.name.is_empty() { &self.info.device } else { &self.name };
        format!("{} · {name} · {} × {}", n + 1, r.width(), r.height())
    }
}

/// Every monitor, left to right (then top to bottom).
pub fn all() -> Vec<Monitor> {
    let ids = MonitorIds::query();
    let mut list: Vec<Monitor> = monitor::all()
        .into_iter()
        .map(|info| {
            let id = ids.id(&info);
            Monitor { info, id: id.id, name: id.name }
        })
        .collect();
    list.sort_by_key(|m| (m.info.rect.left, m.info.rect.top));
    list
}
