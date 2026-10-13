//! Layouts and which monitor uses which: `layouts.json` + `applied.json`.
//!
//! Zone rectangles are worked out when a drag needs them (from the monitor's current work
//! area), so display, DPI and taskbar changes need no handling and cost nothing while idle.

use std::path::Path;

use gz_core::layout::ResolvedZone;
use gz_core::store::{Applied, LayoutsFile, MonitorEntry, APPLIED_FILE, LAYOUTS_FILE};
use gz_core::Rect;
use gz_win::monitor::{self, MonitorIds, MonitorInfo};

/// The zones on one monitor, resolved against its work area.
pub struct MonitorZones {
    /// GDI device name (`\\.\DISPLAY1`), to notice when the cursor changes monitor.
    pub device: String,
    pub work: Rect,
    pub dpi: u32,
    pub layout_name: String,
    pub zones: Vec<ResolvedZone>,
}

#[derive(Default)]
pub struct ZoneConfig {
    layouts: Option<LayoutsFile>,
    applied: Applied,
}

impl ZoneConfig {
    /// Loads both files, creating them if missing. On an error, the part that failed keeps
    /// its previous value and the error is returned for the tray notification.
    pub fn reload(&mut self, dir: &Path) -> Vec<String> {
        let mut errors = Vec::new();
        let layouts_path = dir.join(LAYOUTS_FILE);
        match LayoutsFile::load_or_create(&layouts_path) {
            Ok((layouts, created)) => {
                let names: Vec<&str> = layouts.layouts.iter().map(|l| l.name.as_str()).collect();
                let verb = if created { "Created" } else { "Loaded" };
                log::info!("{verb} {} ({} layouts: {})", layouts_path.display(), names.len(), names.join(", "));
                self.layouts = Some(layouts);
            }
            Err(e) => {
                log::error!("{} could not be used: {e}. Keeping the previous layouts.", layouts_path.display());
                errors.push(format!("{LAYOUTS_FILE}: {e}"));
            }
        }

        let applied_path = dir.join(APPLIED_FILE);
        match Applied::load(&applied_path) {
            Ok(Some(applied)) => {
                log::info!("Loaded {}", applied_path.display());
                self.applied = applied;
            }
            Ok(None) => {
                self.applied = self.initial_applied();
                match self.applied.save(&applied_path) {
                    Ok(()) => log::info!("Created {}", applied_path.display()),
                    Err(e) => log::warn!("Could not write {}: {e}", applied_path.display()),
                }
            }
            Err(e) => {
                log::error!("{} could not be read: {e}. Keeping the previous assignments.", applied_path.display());
                errors.push(format!("{APPLIED_FILE}: {e}"));
            }
        }
        if let Some(layouts) = &self.layouts {
            let unknown = self.applied.unknown_layouts(layouts);
            if !unknown.is_empty() {
                log::warn!("{APPLIED_FILE} refers to layouts that don't exist: {}", unknown.join(", "));
                errors.push(format!("{APPLIED_FILE}: unknown layout {}", unknown.join(", ")));
            }
        }
        self.log_monitors();
        errors
    }

    /// Every current monitor on the first layout, so the file lists the monitor ids.
    fn initial_applied(&self) -> Applied {
        let default = self.layouts.as_ref().and_then(|l| l.layouts.first()).map(|l| l.id.clone());
        let ids = MonitorIds::query();
        let monitors = monitor::all()
            .iter()
            .map(|m| {
                let id = ids.id(m);
                (id.id, MonitorEntry { layout: default.clone(), name: id.name })
            })
            .collect();
        Applied { default_layout: default, monitors }
    }

    fn log_monitors(&self) {
        let ids = MonitorIds::query();
        for m in monitor::all() {
            let id = ids.id(&m);
            let label = if id.name.is_empty() { String::new() } else { format!(" \"{}\"", id.name) };
            log::info!("Monitor {}{label} {} work area {}, id {}", m.device, m.rect, m.work, id.id);
            match self.for_monitor(&m, &ids) {
                Some(mz) => {
                    log::info!("    layout \"{}\", {} zones:", mz.layout_name, mz.zones.len());
                    for (i, z) in mz.zones.iter().enumerate() {
                        let note = if z.disabled { "  (disabled: too small)" } else { "" };
                        log::info!("    zone {}: {}{note}", i + 1, z.rect);
                    }
                }
                None => log::info!("    no zones"),
            }
        }
    }

    /// The zones for a monitor, or `None` if it has no layout (or the layout is missing).
    pub fn for_monitor(&self, m: &MonitorInfo, ids: &MonitorIds) -> Option<MonitorZones> {
        let layouts = self.layouts.as_ref()?;
        let id = ids.id(m);
        let layout = layouts.get(self.applied.layout_for(&id.id)?)?;
        match layout.resolve(m.work, 0) {
            Ok(zones) => Some(MonitorZones {
                device: m.device.clone(),
                work: m.work,
                dpi: m.dpi,
                layout_name: layout.name.clone(),
                zones,
            }),
            Err(e) => {
                // validate() ran at load, so this shouldn't happen.
                log::error!("Layout \"{}\" could not be resolved: {e}", layout.id);
                None
            }
        }
    }
}
