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
        let file: LayoutsFile = serde_json::from_str(crate::config::without_bom(json))
            .map_err(|e| LayoutsError::Config(ConfigError::Json(e)))?;
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

    /// An id for a new layout called `name`: the name in lower case with dashes, made unique.
    pub fn unique_id(&self, name: &str) -> String {
        let mut slug = String::new();
        for c in name.trim().chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
        }
        let slug = match slug.trim_end_matches('-') {
            "" => "layout".to_string(),
            s => s.to_string(),
        };
        std::iter::once(slug.clone())
            .chain((2..).map(|n| format!("{slug}-{n}")))
            .find(|id| self.get(id).is_none())
            .expect("unbounded")
    }

    /// The file as the editor writes it: one line per gaming zone, guide and zone, in the
    /// same style as the built-in file, so it stays easy to edit by hand.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n  \"layouts\": [\n");
        for (n, l) in self.layouts.iter().enumerate() {
            let mut fields = vec![format!("\"id\": {}", inline(&l.id)), format!("\"name\": {}", inline(&l.name))];
            if l.gaming_over_taskbar {
                fields.push("\"gaming_over_taskbar\": true".into());
            }
            if l.gaming_keep_on_top {
                fields.push("\"gaming_keep_on_top\": true".into());
            }
            if !l.gaming_zones.is_empty() {
                fields.push(format!("\"gaming_zones\": {}", lines(&l.gaming_zones)));
            }
            fields.push(format!("\"guides\": {}", lines(&l.guides)));
            fields.push(format!("\"zones\": {}", lines(&l.zones)));
            out += "    {\n      ";
            out += &fields.join(",\n      ");
            out += if n + 1 == self.layouts.len() { "\n    }\n" } else { "\n    },\n" };
        }
        out + "  ]\n}\n"
    }

    /// Writes [`Self::to_json`] to `path` via a temporary file, so a failed write can't
    /// leave a half-written file behind.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        save_atomically(path, &self.to_json())
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

/// A value on one line, with a space after `:` and `,` and inside braces (`{ "px": 100 }`),
/// and whole numbers without `.0`.
fn inline<T: Serialize + ?Sized>(value: &T) -> String {
    let compact = serde_json::to_string(value).expect("layouts always serialise");
    let mut out = String::with_capacity(compact.len() * 5 / 4);
    let (mut in_string, mut escaped) = (false, false);
    let mut chars = compact.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            ':' | ',' => {
                out.push(c);
                out.push(' ');
            }
            '{' if chars.peek() != Some(&'}') => out.push_str("{ "),
            '}' if !out.ends_with('{') => out.push_str(" }"),
            '.' if chars.peek() == Some(&'0') => {
                // "21.0" → "21", but keep "0.05".
                let mut ahead = chars.clone();
                ahead.next();
                if ahead.peek().is_some_and(|n| !n.is_ascii_digit()) {
                    chars.next();
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// A JSON array with one [`inline`] item per line (or `[]`).
fn lines<T: Serialize>(items: &[T]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let rows: Vec<String> = items.iter().map(|i| format!("        {}", inline(i))).collect();
    format!("[\n{}\n      ]", rows.join(",\n"))
}

fn save_atomically(path: &Path, text: &str) -> Result<(), ConfigError> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(ConfigError::Io)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ConfigError::Io(e)
    })
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
        serde_json::from_str(crate::config::without_bom(json)).map_err(ConfigError::Json)
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
        save_atomically(path, &self.to_json())
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
    fn written_like_the_built_in_file() {
        assert_eq!(LayoutsFile::defaults().to_json(), DEFAULT_LAYOUTS_JSON);
        // Odd strings and numbers survive.
        let mut file = LayoutsFile::defaults();
        file.layouts[0].name = "Quote \" comma, {brace}: 1.0".into();
        file.layouts[0].gaming_over_taskbar = true;
        file.layouts[0].gaming_keep_on_top = true;
        file.layouts[1].guides[0].anchor = crate::layout::Anchor::Percent(0.05);
        let again = LayoutsFile::from_json(&file.to_json()).unwrap();
        assert_eq!(again, file);
        assert!(file.to_json().contains("{ \"percent\": 0.05 }"));
    }

    #[test]
    fn unique_ids() {
        let file = LayoutsFile::defaults();
        assert_eq!(file.unique_id("My Layout!"), "my-layout");
        assert_eq!(file.unique_id("WFH"), "wfh-2");
        assert_eq!(file.unique_id("  ***  "), "layout");
        assert_eq!(file.unique_id("Ünïcode 2"), "n-code-2");
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
