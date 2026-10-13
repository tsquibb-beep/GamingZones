//! `layouts.json` (the layouts) and `applied.json` (which layout each monitor uses).

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::layout::{Layout, LayoutError};

pub const LAYOUTS_FILE: &str = "layouts.json";
pub const APPLIED_FILE: &str = "applied.json";

/// Written to `layouts.json` when it doesn't exist: Split focus, WFH, Slay and Gaming-Focus
/// (PLAN §3). The first is the one new monitors get.
pub const DEFAULT_LAYOUTS_JSON: &str = include_str!("default_layouts.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutsFile {
    pub layouts: Vec<Layout>,
}

#[derive(Debug)]
pub enum LayoutsError {
    Config(ConfigError),
    DuplicateLayout(String),
    Layout { id: String, error: LayoutError },
}

impl fmt::Display for LayoutsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "{e}"),
            Self::DuplicateLayout(id) => write!(f, "layout id \"{id}\" is used twice"),
            Self::Layout { id, error } => write!(f, "layout \"{id}\": {error}"),
        }
    }
}

impl std::error::Error for LayoutsError {}

impl LayoutsFile {
    pub fn defaults() -> LayoutsFile {
        LayoutsFile::from_json(DEFAULT_LAYOUTS_JSON).expect("built-in layouts are valid")
    }

    /// Parses and validates every layout.
    pub fn from_json(json: &str) -> Result<LayoutsFile, LayoutsError> {
        let file: LayoutsFile = serde_json::from_str(json).map_err(|e| LayoutsError::Config(ConfigError::Json(e)))?;
        let mut ids = HashSet::new();
        for layout in &file.layouts {
            if !ids.insert(layout.id.as_str()) {
                return Err(LayoutsError::DuplicateLayout(layout.id.clone()));
            }
            layout.validate().map_err(|error| LayoutsError::Layout { id: layout.id.clone(), error })?;
        }
        Ok(file)
    }

    pub fn get(&self, id: &str) -> Option<&Layout> {
        self.layouts.iter().find(|l| l.id == id)
    }

    /// Loads `path`, or writes the built-in layouts there if it doesn't exist.
    pub fn load_or_create(path: &Path) -> Result<(LayoutsFile, bool), LayoutsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok((LayoutsFile::from_json(&text)?, false)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(path, DEFAULT_LAYOUTS_JSON).map_err(|e| LayoutsError::Config(ConfigError::Io(e)))?;
                Ok((LayoutsFile::defaults(), true))
            }
            Err(e) => Err(LayoutsError::Config(ConfigError::Io(e))),
        }
    }
}

/// One monitor's entry in `applied.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorEntry {
    /// Layout id, or `null` for no zones on this monitor.
    pub layout: Option<String>,
    /// The monitor's name, to help when editing by hand. Not used for matching.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// `applied.json`: monitors are keyed by their stable id (see `gz_win::monitor`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Applied {
    /// Layout for monitors not listed in `monitors` (`null` = no zones).
    pub default_layout: Option<String>,
    pub monitors: BTreeMap<String, MonitorEntry>,
}

impl Applied {
    pub fn from_json(json: &str) -> Result<Applied, ConfigError> {
        serde_json::from_str(json).map_err(ConfigError::Json)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("applied always serialises") + "\n"
    }

    /// `Ok(None)` if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Applied>, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Applied::from_json(&text).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        std::fs::write(path, self.to_json()).map_err(ConfigError::Io)
    }

    /// The layout id for a monitor (`None` = no zones there).
    pub fn layout_for(&self, monitor_id: &str) -> Option<&str> {
        match self.monitors.get(monitor_id) {
            Some(entry) => entry.layout.as_deref(),
            None => self.default_layout.as_deref(),
        }
    }

    /// Layout ids referenced here that `layouts` doesn't have, with where they're used.
    pub fn unknown_layouts(&self, layouts: &LayoutsFile) -> Vec<String> {
        let default = self.default_layout.iter().map(|id| ("default_layout".to_string(), id));
        let monitors =
            self.monitors.iter().filter_map(|(m, e)| e.layout.as_ref().map(|id| (format!("monitor {m}"), id)));
        default
            .chain(monitors)
            .filter(|(_, id)| layouts.get(id).is_none())
            .map(|(place, id)| format!("\"{id}\" ({place})"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::layout::ResolvedZone;

    /// Tom's 6144×2560 monitor with a 60 px taskbar (48 px at 125%) at the bottom.
    const WORK: Rect = Rect::new(0, 0, 6144, 2500);
    const MONITOR: Rect = Rect::new(0, 0, 6144, 2560);

    fn rects(id: &str) -> Vec<Rect> {
        let zones: Vec<ResolvedZone> = LayoutsFile::defaults().get(id).unwrap().resolve(WORK, MONITOR, 0).unwrap();
        assert!(zones.iter().all(|z| !z.disabled));
        zones.into_iter().map(|z| z.rect).collect()
    }

    /// Zones cover the area exactly once: inside it, no overlaps, areas add up.
    fn assert_tiles(rects: &[Rect], area: Rect) {
        let total: i64 = rects.iter().map(|r| r.width() as i64 * r.height() as i64).sum();
        assert_eq!(total, area.width() as i64 * area.height() as i64);
        for (i, a) in rects.iter().enumerate() {
            assert_eq!(a.union(&area), area, "zone {} sticks out", i + 1);
            for b in &rects[i + 1..] {
                let overlap = a.left.max(b.left) < a.right.min(b.right) && a.top.max(b.top) < a.bottom.min(b.bottom);
                assert!(!overlap, "{a} overlaps {b}");
            }
        }
    }

    #[test]
    fn default_layouts() {
        let ids: Vec<String> = LayoutsFile::defaults().layouts.into_iter().map(|l| l.id).collect();
        assert_eq!(ids, ["split-focus", "wfh", "slay", "gaming-focus"]);
    }

    #[test]
    fn split_focus() {
        let r = rects("split-focus");
        // Columns at 21% / 79% of 6144 (1290.24 → 1290, 4853.76 → 4854), centre split halfway between.
        // Side rows at 50% of 2500; centre strips at 15% / 85%.
        assert_eq!(
            r,
            [
                Rect::new(0, 0, 1290, 1250),
                Rect::new(0, 1250, 1290, 2500),
                Rect::new(1290, 0, 4854, 375),
                Rect::new(1290, 375, 3072, 2125),
                Rect::new(3072, 375, 4854, 2125),
                Rect::new(1290, 2125, 4854, 2500),
                Rect::new(4854, 0, 6144, 1250),
                Rect::new(4854, 1250, 6144, 2500),
            ]
        );
        assert_tiles(&r, WORK);
    }

    #[test]
    fn wfh() {
        let r = rects("wfh");
        assert_eq!(
            r,
            [
                Rect::new(0, 0, 3072, 1250),
                Rect::new(3072, 0, 6144, 1250),
                Rect::new(0, 1250, 3072, 2500),
                Rect::new(3072, 1250, 6144, 2500)
            ]
        );
        assert_tiles(&r, WORK);
    }

    #[test]
    fn slay() {
        let r = rects("slay");
        // 26% = 1597.44 → 1597, 74% = 4546.56 → 4547.
        assert_eq!(r, [Rect::new(0, 0, 1597, 2500), Rect::new(1597, 0, 4547, 2500), Rect::new(4547, 0, 6144, 2500)]);
        assert_tiles(&r, WORK);
    }

    #[test]
    fn gaming_focus() {
        let r = rects("gaming-focus");
        // 3840×2160 centred in 6144×2500: x 1152..4992, y 170..2330.
        assert_eq!(
            r,
            [
                Rect::new(0, 0, 1152, 1250),
                Rect::new(0, 1250, 1152, 2500),
                Rect::new(1152, 0, 4992, 170),
                Rect::new(1152, 2330, 4992, 2500),
                Rect::new(4992, 0, 6144, 1250),
                Rect::new(4992, 1250, 6144, 2500),
                Rect::new(1152, 170, 4992, 2330),
            ]
        );
        assert_tiles(&r, WORK);
        // With the default 400 px threshold the 170 px strips above and below are disabled.
        let zones = LayoutsFile::defaults().get("gaming-focus").unwrap().resolve(WORK, MONITOR, 400).unwrap();
        let disabled: Vec<usize> = (0..zones.len()).filter(|&i| zones[i].disabled).collect();
        assert_eq!(disabled, [2, 3]);
        assert!(zones[6].gaming);
    }

    #[test]
    fn layouts_tile_odd_areas_too() {
        // A second monitor left of the primary, with an odd size and the taskbar on top.
        let area = Rect::new(-1921, 41, 0, 1081);
        for layout in LayoutsFile::defaults().layouts {
            let r: Vec<Rect> = layout.resolve(area, area, 0).unwrap().into_iter().map(|z| z.rect).collect();
            assert_tiles(&r, area);
        }
    }

    #[test]
    fn duplicate_or_invalid_layouts_are_rejected() {
        let dup = r#"{ "layouts": [
            { "id": "a", "name": "A", "zones": [ { "left": "area.left", "right": "area.right", "top": "area.top", "bottom": "area.bottom" } ] },
            { "id": "a", "name": "B", "zones": [ { "left": "area.left", "right": "area.right", "top": "area.top", "bottom": "area.bottom" } ] } ] }"#;
        assert!(matches!(LayoutsFile::from_json(dup), Err(LayoutsError::DuplicateLayout(id)) if id == "a"));
        let bad = r#"{ "layouts": [ { "id": "a", "name": "A", "zones": [ { "left": "x", "right": "area.right", "top": "area.top", "bottom": "area.bottom" } ] } ] }"#;
        assert!(matches!(LayoutsFile::from_json(bad), Err(LayoutsError::Layout { .. })));
        assert!(matches!(LayoutsFile::from_json("{"), Err(LayoutsError::Config(_))));
    }

    #[test]
    fn applied_lookup() {
        let applied = Applied::from_json(
            r#"{ "default_layout": "wfh",
                 "monitors": { "M1": { "layout": "slay", "name": "Big one" }, "M2": { "layout": null } } }"#,
        )
        .unwrap();
        assert_eq!(applied.layout_for("M1"), Some("slay"));
        assert_eq!(applied.layout_for("M2"), None);
        assert_eq!(applied.layout_for("other"), Some("wfh"));
        assert!(applied.unknown_layouts(&LayoutsFile::defaults()).is_empty());

        let broken =
            Applied::from_json(r#"{ "default_layout": "nope", "monitors": { "M": { "layout": "gone" } } }"#).unwrap();
        assert_eq!(broken.unknown_layouts(&LayoutsFile::defaults()).len(), 2);
        assert_eq!(Applied::from_json(&applied.to_json()).unwrap(), applied);
    }

    #[test]
    fn load_or_create_files() {
        let dir = std::env::temp_dir().join(format!("gz-core-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let layouts = dir.join(LAYOUTS_FILE);
        let applied = dir.join(APPLIED_FILE);
        let _ = std::fs::remove_file(&layouts);
        let _ = std::fs::remove_file(&applied);

        let (file, created) = LayoutsFile::load_or_create(&layouts).unwrap();
        assert!(created);
        assert_eq!(file, LayoutsFile::defaults());
        let (_, created) = LayoutsFile::load_or_create(&layouts).unwrap();
        assert!(!created);

        assert_eq!(Applied::load(&applied).unwrap(), None);
        let a = Applied { default_layout: Some("wfh".into()), ..Default::default() };
        a.save(&applied).unwrap();
        assert_eq!(Applied::load(&applied).unwrap(), Some(a));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
