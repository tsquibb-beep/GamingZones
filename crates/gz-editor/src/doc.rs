//! What the editor edits: `layouts.json` and `applied.json`, loaded and saved together.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gz_core::store::{Applied, LayoutsFile, MonitorEntry, APPLIED_FILE, LAYOUTS_FILE};

use crate::monitors::Monitor;

#[derive(Clone, PartialEq)]
pub struct Doc {
    pub layouts: LayoutsFile,
    pub applied: Applied,
}

/// When the files were last read or written, to notice edits made outside the editor.
#[derive(Clone, Copy, PartialEq, Default)]
pub struct Stamps {
    layouts: Option<SystemTime>,
    applied: Option<SystemTime>,
}

impl Stamps {
    /// Only applied.json differs between `self` and `other`.
    pub fn only_applied_differs(&self, other: &Stamps) -> bool {
        self.layouts == other.layouts && self.applied != other.applied
    }
}

pub struct Files {
    pub dir: PathBuf,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Files {
    pub fn layouts_path(&self) -> PathBuf {
        self.dir.join(LAYOUTS_FILE)
    }

    pub fn applied_path(&self) -> PathBuf {
        self.dir.join(APPLIED_FILE)
    }

    pub fn stamps(&self) -> Stamps {
        Stamps { layouts: modified(&self.layouts_path()), applied: modified(&self.applied_path()) }
    }

    /// Loads both files. A missing `layouts.json` gives the built-in layouts and a missing
    /// `applied.json` puts every monitor on the first layout, as the tray app does; neither
    /// is written until Save. A file with an error is an error (and is never overwritten).
    pub fn load(&self, monitors: &[Monitor]) -> Result<(Doc, Stamps), String> {
        let layouts = match std::fs::read_to_string(self.layouts_path()) {
            Ok(text) => LayoutsFile::from_json(&text).map_err(|e| format!("{LAYOUTS_FILE}: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => LayoutsFile::defaults(),
            Err(e) => return Err(format!("{LAYOUTS_FILE}: {e}")),
        };
        let applied = match Applied::load(&self.applied_path()).map_err(|e| format!("{APPLIED_FILE}: {e}"))? {
            Some(a) => a,
            None => {
                let first = layouts.layouts.first().map(|l| l.id.clone());
                Applied {
                    default_layout: first.clone(),
                    monitors: monitors
                        .iter()
                        .map(|m| (m.id.clone(), MonitorEntry { layout: first.clone(), name: m.name.clone() }))
                        .collect(),
                }
            }
        };
        Ok((Doc { layouts, applied }, self.stamps()))
    }

    /// Writes both files (after checking every layout).
    pub fn save(&self, doc: &Doc) -> Result<Stamps, String> {
        LayoutsFile::from_json(&doc.layouts.to_json()).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        doc.layouts.save(&self.layouts_path()).map_err(|e| format!("{LAYOUTS_FILE}: {e}"))?;
        doc.applied.save(&self.applied_path()).map_err(|e| format!("{APPLIED_FILE}: {e}"))?;
        Ok(self.stamps())
    }
}
