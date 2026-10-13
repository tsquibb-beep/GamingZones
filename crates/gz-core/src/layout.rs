//! Layouts: guides (lines) and zones (rectangles bounded by four guides).
//!
//! A layout is resolved against an *area* (normally a monitor's work area, in screen
//! coordinates) to get each zone's rectangle in physical pixels. Guides are positioned
//! relative to the area or to other guides, so the same layout works on any monitor.
//!
//! ```json
//! { "id": "wfh", "name": "WFH",
//!   "guides": [ { "id": "mid-x", "axis": "vertical",   "anchor": { "percent": 50 } },
//!               { "id": "mid-y", "axis": "horizontal", "anchor": { "percent": 50 } } ],
//!   "zones":  [ { "left": "area.left", "right": "mid-x", "top": "area.top", "bottom": "mid-y" }, ... ] }
//! ```

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// A vertical line: its position is an x coordinate.
    Vertical,
    /// A horizontal line: its position is a y coordinate.
    Horizontal,
}

impl Axis {
    fn name(self) -> &'static str {
        match self {
            Axis::Vertical => "vertical",
            Axis::Horizontal => "horizontal",
        }
    }
}

/// Where a guide sits. Positions are clamped to the area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    /// Pixels from the area's left (vertical) or top (horizontal) edge.
    Px(i32),
    /// Percent of the area's width (vertical) or height (horizontal).
    Percent(f64),
    /// A point between two other guides: `a + (b - a) * fraction`.
    Between { a: String, b: String, fraction: f64 },
    /// A fixed distance from another guide (negative = left/up).
    Offset { from: String, px: i32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub id: String,
    pub axis: Axis,
    pub anchor: Anchor,
}

/// A zone, given by the ids of the guides on each side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zone {
    pub left: String,
    pub right: String,
    pub top: String,
    pub bottom: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub guides: Vec<Guide>,
    pub zones: Vec<Zone>,
}

/// The built-in guides on the area's edges. User guide ids may not contain a `.`, so they
/// can never clash with these (or with Phase 2's `gz0.left` etc.).
pub const AREA_LEFT: &str = "area.left";
pub const AREA_RIGHT: &str = "area.right";
pub const AREA_TOP: &str = "area.top";
pub const AREA_BOTTOM: &str = "area.bottom";

fn builtin(id: &str) -> Option<Axis> {
    match id {
        AREA_LEFT | AREA_RIGHT => Some(Axis::Vertical),
        AREA_TOP | AREA_BOTTOM => Some(Axis::Horizontal),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    DuplicateGuide(String),
    /// User guide ids may not contain `.` (reserved for built-in guides).
    ReservedGuideId(String),
    UnknownGuide {
        referenced_by: String,
        id: String,
    },
    /// A guide refers to a guide on the other axis, or a zone side uses the wrong axis.
    WrongAxis {
        referenced_by: String,
        id: String,
        expected: Axis,
    },
    /// Guides that depend on each other in a loop, in order.
    Cycle(Vec<String>),
    NoZones,
    BadNumber {
        guide: String,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGuide(id) => write!(f, "guide \"{id}\" is defined twice"),
            Self::ReservedGuideId(id) => {
                write!(f, "guide id \"{id}\" may not contain a '.' (reserved for built-in guides)")
            }
            Self::UnknownGuide { referenced_by, id } => write!(f, "{referenced_by} refers to unknown guide \"{id}\""),
            Self::WrongAxis { referenced_by, id, expected } => {
                write!(f, "{referenced_by} needs a {} guide, but \"{id}\" is not", expected.name())
            }
            Self::Cycle(ids) => write!(f, "guides depend on each other in a loop: {}", ids.join(" → ")),
            Self::NoZones => write!(f, "layout has no zones"),
            Self::BadNumber { guide } => write!(f, "guide \"{guide}\" has a number that is not finite"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// A zone resolved against an area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedZone {
    pub rect: Rect,
    /// Too small to use (width or height below the threshold, or not positive). Never a
    /// snap target.
    pub disabled: bool,
}

impl Layout {
    /// Checks guide ids, references, axes and cycles without needing an area.
    pub fn validate(&self) -> Result<(), LayoutError> {
        self.resolve(Rect::new(0, 0, 1000, 1000), 0).map(|_| ())
    }

    /// Resolves every zone against `area`. Zones keep their order (zone number = index + 1).
    /// A zone narrower or shorter than `deadzone_px` (or not positive) is marked disabled.
    pub fn resolve(&self, area: Rect, deadzone_px: i32) -> Result<Vec<ResolvedZone>, LayoutError> {
        if self.zones.is_empty() {
            return Err(LayoutError::NoZones);
        }
        let positions = Resolver::new(self, area)?.resolve_all()?;
        let pos = |referenced_by: String, id: &str, expected: Axis| -> Result<i32, LayoutError> {
            let (axis, value) = *positions
                .get(id)
                .ok_or_else(|| LayoutError::UnknownGuide { referenced_by: referenced_by.clone(), id: id.into() })?;
            if axis != expected {
                return Err(LayoutError::WrongAxis { referenced_by, id: id.into(), expected });
            }
            Ok(value)
        };
        self.zones
            .iter()
            .enumerate()
            .map(|(i, z)| {
                let side = |name: &str| format!("zone {} {name}", i + 1);
                let rect = Rect::new(
                    pos(side("left"), &z.left, Axis::Vertical)?,
                    pos(side("top"), &z.top, Axis::Horizontal)?,
                    pos(side("right"), &z.right, Axis::Vertical)?,
                    pos(side("bottom"), &z.bottom, Axis::Horizontal)?,
                );
                let min = deadzone_px.max(1);
                Ok(ResolvedZone { rect, disabled: rect.width() < min || rect.height() < min })
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Visiting,
    Done(i32),
}

/// Evaluates guides in dependency order (depth-first with cycle detection).
struct Resolver<'a> {
    area: Rect,
    guides: HashMap<&'a str, &'a Guide>,
    state: HashMap<&'a str, State>,
    /// Current dependency chain, for the cycle error message.
    stack: Vec<&'a str>,
}

impl<'a> Resolver<'a> {
    fn new(layout: &'a Layout, area: Rect) -> Result<Self, LayoutError> {
        let mut guides = HashMap::new();
        for g in &layout.guides {
            if g.id.contains('.') {
                return Err(LayoutError::ReservedGuideId(g.id.clone()));
            }
            if guides.insert(g.id.as_str(), g).is_some() {
                return Err(LayoutError::DuplicateGuide(g.id.clone()));
            }
        }
        Ok(Self { area, guides, state: HashMap::new(), stack: Vec::new() })
    }

    fn resolve_all(mut self) -> Result<HashMap<String, (Axis, i32)>, LayoutError> {
        let mut out = HashMap::new();
        for id in [AREA_LEFT, AREA_RIGHT, AREA_TOP, AREA_BOTTOM] {
            let axis = builtin(id).expect("built-in");
            out.insert(id.to_string(), (axis, self.builtin_position(id)));
        }
        let ids: Vec<&'a str> = self.guides.keys().copied().collect();
        for id in ids {
            let axis = self.guides[id].axis;
            let value = self.position(id, axis, &|| format!("guide \"{id}\""))?;
            out.insert(id.to_string(), (axis, value));
        }
        Ok(out)
    }

    fn builtin_position(&self, id: &str) -> i32 {
        match id {
            AREA_LEFT => self.area.left,
            AREA_RIGHT => self.area.right,
            AREA_TOP => self.area.top,
            _ => self.area.bottom,
        }
    }

    /// The area's start and end along the axis a guide measures.
    fn span(&self, axis: Axis) -> (i32, i32) {
        match axis {
            Axis::Vertical => (self.area.left, self.area.right),
            Axis::Horizontal => (self.area.top, self.area.bottom),
        }
    }

    /// Position of guide `id`, which must be on `expected` axis.
    fn position(&mut self, id: &str, expected: Axis, referenced_by: &dyn Fn() -> String) -> Result<i32, LayoutError> {
        if let Some(axis) = builtin(id) {
            if axis != expected {
                return Err(LayoutError::WrongAxis { referenced_by: referenced_by(), id: id.into(), expected });
            }
            return Ok(self.builtin_position(id));
        }
        let Some(&guide) = self.guides.get(id) else {
            return Err(LayoutError::UnknownGuide { referenced_by: referenced_by(), id: id.into() });
        };
        if guide.axis != expected {
            return Err(LayoutError::WrongAxis { referenced_by: referenced_by(), id: id.into(), expected });
        }
        let key = guide.id.as_str();
        match self.state.get(key) {
            Some(State::Done(v)) => return Ok(*v),
            Some(State::Visiting) => {
                let start = self.stack.iter().position(|&s| s == key).unwrap_or(0);
                let mut cycle: Vec<String> = self.stack[start..].iter().map(|s| s.to_string()).collect();
                cycle.push(key.to_string());
                return Err(LayoutError::Cycle(cycle));
            }
            None => {}
        }
        self.state.insert(key, State::Visiting);
        self.stack.push(key);

        let (start, end) = self.span(guide.axis);
        let bad = || LayoutError::BadNumber { guide: key.to_string() };
        let me = || format!("guide \"{key}\"");
        let raw = match &guide.anchor {
            Anchor::Px(px) => start as i64 + *px as i64,
            Anchor::Percent(p) => {
                if !p.is_finite() {
                    return Err(bad());
                }
                start as i64 + round((end - start) as f64 * p / 100.0)
            }
            Anchor::Between { a, b, fraction } => {
                if !fraction.is_finite() {
                    return Err(bad());
                }
                let a = self.position(a, guide.axis, &me)?;
                let b = self.position(b, guide.axis, &me)?;
                a as i64 + round((b - a) as f64 * fraction)
            }
            Anchor::Offset { from, px } => self.position(from, guide.axis, &me)? as i64 + *px as i64,
        };
        let value = raw.clamp(start as i64, end as i64) as i32;

        self.stack.pop();
        self.state.insert(key, State::Done(value));
        Ok(value)
    }
}

/// Rounds half away from zero, so results don't depend on the sign of the offset.
fn round(v: f64) -> i64 {
    v.round() as i64
}

// ---------------------------------------------------------------------------------------
// Picking zones under the cursor.
//
// Adapted from PowerToys FancyZones (MIT), `FancyZonesLib/Layout.cpp`: `ZonesFromPoint`
// (sensitivity radius + overlap test) and `GetCombinedZoneRange`. See THIRD_PARTY_NOTICES.md.
// ---------------------------------------------------------------------------------------

/// Zones (indices) to highlight with the cursor at (`x`, `y`). Disabled zones are ignored.
///
/// - Without edge spanning (`edge_radius` = `None`): the zone under the cursor; where
///   zones overlap, the smallest.
/// - With edge spanning: zones within `edge_radius` px of the cursor are captured too, so
///   on a border shared by two zones both are returned. A zone captured only by the radius,
///   with nothing actually under the cursor, doesn't count. If the captured zones really
///   overlap (by more than the radius), the smallest one wins as above.
pub fn zones_at(zones: &[ResolvedZone], x: i32, y: i32, edge_radius: Option<i32>) -> Vec<usize> {
    let radius = edge_radius.unwrap_or(0).max(0);
    let enabled = || zones.iter().enumerate().filter(|(_, z)| !z.disabled);
    let strict: Vec<usize> = enabled().filter(|(_, z)| z.rect.contains(x, y)).map(|(i, _)| i).collect();
    let captured: Vec<usize> = if edge_radius.is_some() {
        enabled()
            .filter(|(_, z)| {
                let r = &z.rect;
                r.left - radius <= x && x <= r.right + radius && r.top - radius <= y && y <= r.bottom + radius
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        strict.clone()
    };
    if captured.is_empty() || (captured.len() == 1 && strict.is_empty()) {
        return Vec::new();
    }

    let overlap = captured.iter().enumerate().any(|(n, &i)| {
        captured[n + 1..].iter().any(|&j| {
            let (a, b) = (&zones[i].rect, &zones[j].rect);
            a.top.max(b.top) + radius < a.bottom.min(b.bottom) && a.left.max(b.left) + radius < a.right.min(b.right)
        })
    });
    if !overlap {
        return captured;
    }
    // Smallest area wins; ties go to the lower zone number.
    let area = |i: usize| zones[i].rect.width() as i64 * zones[i].rect.height() as i64;
    captured.into_iter().min_by_key(|&i| (area(i), i)).into_iter().collect()
}

/// Ctrl-span: every enabled zone inside the bounding box of `anchor` and `current`
/// (FancyZones' `GetCombinedZoneRange`). Returns indices in zone order.
pub fn zone_range(zones: &[ResolvedZone], anchor: &[usize], current: &[usize]) -> Vec<usize> {
    let Some(bounds) = bounding_rect(zones, anchor.iter().chain(current)) else {
        return Vec::new();
    };
    zones
        .iter()
        .enumerate()
        .filter(|(_, z)| {
            let r = &z.rect;
            !z.disabled
                && bounds.left <= r.left
                && r.right <= bounds.right
                && bounds.top <= r.top
                && r.bottom <= bounds.bottom
        })
        .map(|(i, _)| i)
        .collect()
}

/// Bounding rectangle of the given zones (the snap target for a multi-zone selection).
pub fn bounding_rect<'a>(zones: &[ResolvedZone], indices: impl IntoIterator<Item = &'a usize>) -> Option<Rect> {
    indices.into_iter().filter_map(|&i| zones.get(i)).map(|z| z.rect).reduce(|a, b| a.union(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guide(id: &str, axis: Axis, anchor: Anchor) -> Guide {
        Guide { id: id.into(), axis, anchor }
    }

    fn zone(l: &str, t: &str, r: &str, b: &str) -> Zone {
        Zone { left: l.into(), right: r.into(), top: t.into(), bottom: b.into() }
    }

    fn layout(guides: Vec<Guide>, zones: Vec<Zone>) -> Layout {
        Layout { id: "t".into(), name: "Test".into(), guides, zones }
    }

    fn rz(l: i32, t: i32, r: i32, b: i32) -> ResolvedZone {
        ResolvedZone { rect: Rect::new(l, t, r, b), disabled: false }
    }

    /// Two columns split at 50%.
    fn halves() -> Layout {
        layout(
            vec![guide("mid", Axis::Vertical, Anchor::Percent(50.0))],
            vec![zone(AREA_LEFT, AREA_TOP, "mid", AREA_BOTTOM), zone("mid", AREA_TOP, AREA_RIGHT, AREA_BOTTOM)],
        )
    }

    #[test]
    fn resolves_in_screen_coordinates() {
        let zones = halves().resolve(Rect::new(100, 50, 1100, 850), 0).unwrap();
        assert_eq!(zones[0].rect, Rect::new(100, 50, 600, 850));
        assert_eq!(zones[1].rect, Rect::new(600, 50, 1100, 850));
        assert!(zones.iter().all(|z| !z.disabled));
    }

    #[test]
    fn anchors_between_offset_and_px() {
        let l = layout(
            vec![
                guide("a", Axis::Vertical, Anchor::Px(100)),
                guide("b", Axis::Vertical, Anchor::Offset { from: AREA_RIGHT.into(), px: -100 }),
                guide("c", Axis::Vertical, Anchor::Between { a: "a".into(), b: "b".into(), fraction: 0.25 }),
            ],
            vec![zone("a", AREA_TOP, "c", AREA_BOTTOM), zone("c", AREA_TOP, "b", AREA_BOTTOM)],
        );
        let zones = l.resolve(Rect::new(0, 0, 1000, 500), 0).unwrap();
        // a = 100, b = 900, c = 100 + 800 * 0.25 = 300
        assert_eq!(zones[0].rect, Rect::new(100, 0, 300, 500));
        assert_eq!(zones[1].rect, Rect::new(300, 0, 900, 500));
    }

    #[test]
    fn guides_are_clamped_to_the_area() {
        let l = layout(
            vec![guide("far", Axis::Vertical, Anchor::Px(5000)), guide("neg", Axis::Vertical, Anchor::Percent(-10.0))],
            vec![zone("neg", AREA_TOP, "far", AREA_BOTTOM)],
        );
        assert_eq!(l.resolve(Rect::new(10, 0, 1010, 100), 0).unwrap()[0].rect, Rect::new(10, 0, 1010, 100));
    }

    #[test]
    fn zero_and_negative_sizes_are_disabled() {
        let l = layout(
            vec![guide("a", Axis::Vertical, Anchor::Px(300)), guide("b", Axis::Vertical, Anchor::Px(300))],
            vec![
                zone(AREA_LEFT, AREA_TOP, "a", AREA_BOTTOM), // 300 wide
                zone("a", AREA_TOP, "b", AREA_BOTTOM),       // 0 wide
                zone("b", AREA_TOP, AREA_LEFT, AREA_BOTTOM), // negative
                zone(AREA_LEFT, AREA_BOTTOM, "a", AREA_TOP), // negative height
            ],
        );
        let zones = l.resolve(Rect::new(0, 0, 1000, 500), 0).unwrap();
        assert_eq!(zones.iter().map(|z| z.disabled).collect::<Vec<_>>(), [false, true, true, true]);
        assert_eq!(zones[2].rect.width(), -300);
    }

    #[test]
    fn deadzone_threshold() {
        let zones = halves().resolve(Rect::new(0, 0, 1000, 500), 400).unwrap();
        assert!(!zones[0].disabled); // 500 × 500
        let zones = halves().resolve(Rect::new(0, 0, 700, 500), 400).unwrap();
        assert!(zones[0].disabled); // 350 wide
    }

    #[test]
    fn cycles_are_rejected() {
        let l = layout(
            vec![
                guide("a", Axis::Vertical, Anchor::Offset { from: "b".into(), px: 10 }),
                guide("b", Axis::Vertical, Anchor::Between { a: AREA_LEFT.into(), b: "c".into(), fraction: 0.5 }),
                guide("c", Axis::Vertical, Anchor::Offset { from: "a".into(), px: 10 }),
            ],
            vec![zone("a", AREA_TOP, "b", AREA_BOTTOM)],
        );
        match l.validate() {
            Err(LayoutError::Cycle(ids)) => {
                assert_eq!(ids.len(), 4, "{ids:?}");
                assert_eq!(ids.first(), ids.last());
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
        let self_ref = layout(
            vec![guide("a", Axis::Vertical, Anchor::Offset { from: "a".into(), px: 1 })],
            vec![zone("a", AREA_TOP, AREA_RIGHT, AREA_BOTTOM)],
        );
        assert_eq!(self_ref.validate(), Err(LayoutError::Cycle(vec!["a".into(), "a".into()])));
    }

    #[test]
    fn reference_errors() {
        let unknown = layout(vec![], vec![zone(AREA_LEFT, AREA_TOP, "nope", AREA_BOTTOM)]);
        assert!(matches!(unknown.validate(), Err(LayoutError::UnknownGuide { id, .. }) if id == "nope"));

        let wrong_axis = layout(vec![], vec![zone(AREA_TOP, AREA_TOP, AREA_RIGHT, AREA_BOTTOM)]);
        assert!(matches!(wrong_axis.validate(), Err(LayoutError::WrongAxis { expected: Axis::Vertical, .. })));

        let wrong_axis_ref = layout(
            vec![
                guide("h", Axis::Horizontal, Anchor::Percent(50.0)),
                guide("v", Axis::Vertical, Anchor::Offset { from: "h".into(), px: 0 }),
            ],
            vec![zone(AREA_LEFT, AREA_TOP, "v", AREA_BOTTOM)],
        );
        assert!(matches!(wrong_axis_ref.validate(), Err(LayoutError::WrongAxis { .. })));

        let dup = layout(
            vec![guide("a", Axis::Vertical, Anchor::Px(1)), guide("a", Axis::Vertical, Anchor::Px(2))],
            vec![zone(AREA_LEFT, AREA_TOP, "a", AREA_BOTTOM)],
        );
        assert_eq!(dup.validate(), Err(LayoutError::DuplicateGuide("a".into())));

        let reserved = layout(
            vec![guide("gz0.left", Axis::Vertical, Anchor::Px(1))],
            vec![zone(AREA_LEFT, AREA_TOP, AREA_RIGHT, AREA_BOTTOM)],
        );
        assert_eq!(reserved.validate(), Err(LayoutError::ReservedGuideId("gz0.left".into())));

        assert_eq!(layout(vec![], vec![]).validate(), Err(LayoutError::NoZones));

        let nan = layout(
            vec![guide("a", Axis::Vertical, Anchor::Percent(f64::NAN))],
            vec![zone(AREA_LEFT, AREA_TOP, "a", AREA_BOTTOM)],
        );
        assert_eq!(nan.validate(), Err(LayoutError::BadNumber { guide: "a".into() }));
    }

    #[test]
    fn json_shape() {
        let json = r#"{ "id": "x", "name": "X",
            "guides": [ { "id": "a", "axis": "vertical", "anchor": { "percent": 50 } },
                        { "id": "b", "axis": "vertical", "anchor": { "between": { "a": "a", "b": "area.right", "fraction": 0.5 } } },
                        { "id": "c", "axis": "horizontal", "anchor": { "px": 100 } },
                        { "id": "d", "axis": "horizontal", "anchor": { "offset": { "from": "area.bottom", "px": -100 } } } ],
            "zones": [ { "left": "a", "right": "b", "top": "c", "bottom": "d" } ] }"#;
        let l: Layout = serde_json::from_str(json).unwrap();
        assert_eq!(l.guides[1].anchor, Anchor::Between { a: "a".into(), b: AREA_RIGHT.into(), fraction: 0.5 });
        assert_eq!(l.resolve(Rect::new(0, 0, 1000, 1000), 0).unwrap()[0].rect, Rect::new(500, 100, 750, 900));
        let again: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(again, l);
    }

    // --- zone picking ---

    /// 2×2 grid of 500 px squares.
    fn grid() -> Vec<ResolvedZone> {
        vec![rz(0, 0, 500, 500), rz(500, 0, 1000, 500), rz(0, 500, 500, 1000), rz(500, 500, 1000, 1000)]
    }

    #[test]
    fn single_zone_under_cursor() {
        assert_eq!(zones_at(&grid(), 100, 100, None), [0]);
        assert_eq!(zones_at(&grid(), 999, 999, None), [3]);
        // Right/bottom edges are exclusive: x = 500 is in the second column.
        assert_eq!(zones_at(&grid(), 500, 100, None), [1]);
        assert!(zones_at(&grid(), 1000, 100, None).is_empty());
    }

    #[test]
    fn edge_spanning() {
        let r = Some(20);
        // Off: near the border still picks one zone.
        assert_eq!(zones_at(&grid(), 495, 100, None), [0]);
        // On: within 20 px of the shared border picks both.
        assert_eq!(zones_at(&grid(), 495, 100, r), [0, 1]);
        assert_eq!(zones_at(&grid(), 510, 100, r), [0, 1]);
        // Far from the border: just one.
        assert_eq!(zones_at(&grid(), 450, 100, r), [0]);
        // At the centre crossing: all four.
        assert_eq!(zones_at(&grid(), 500, 500, r), [0, 1, 2, 3]);
        // Outside every zone but within the radius of one: nothing.
        assert!(zones_at(&grid(), 1010, 100, r).is_empty());
    }

    #[test]
    fn overlapping_zones_pick_the_smallest() {
        let zones = vec![rz(0, 0, 1000, 1000), rz(200, 200, 400, 400), rz(300, 300, 700, 700)];
        assert_eq!(zones_at(&zones, 350, 350, None), [1]);
        assert_eq!(zones_at(&zones, 350, 350, Some(20)), [1]);
        assert_eq!(zones_at(&zones, 600, 600, None), [2]);
        assert_eq!(zones_at(&zones, 900, 900, None), [0]);
    }

    #[test]
    fn disabled_zones_are_never_picked() {
        let mut zones = grid();
        zones[0].disabled = true;
        assert!(zones_at(&zones, 100, 100, None).is_empty());
        // Over the disabled zone, near zone 2's edge: only captured by the radius, so nothing.
        assert!(zones_at(&zones, 495, 100, Some(20)).is_empty());
        assert_eq!(zones_at(&zones, 510, 100, Some(20)), [1]);
        assert_eq!(zone_range(&zones, &[2], &[1]), [1, 2, 3]);
    }

    #[test]
    fn ctrl_span_range() {
        let zones = grid();
        assert_eq!(zone_range(&zones, &[0], &[0]), [0]);
        assert_eq!(zone_range(&zones, &[0], &[1]), [0, 1]);
        assert_eq!(zone_range(&zones, &[0], &[3]), [0, 1, 2, 3]);
        assert_eq!(zone_range(&zones, &[2], &[1]), [0, 1, 2, 3]);
        assert!(zone_range(&zones, &[], &[]).is_empty());
        assert_eq!(bounding_rect(&zones, &[0, 1]), Some(Rect::new(0, 0, 1000, 500)));
    }

    #[test]
    fn ctrl_span_skips_zones_sticking_out() {
        // Columns 1 | 2 | 3, with a full-width strip below. Spanning 1→2 must not take the strip.
        let zones = vec![rz(0, 0, 300, 800), rz(300, 0, 700, 800), rz(700, 0, 1000, 800), rz(0, 800, 1000, 1000)];
        assert_eq!(zone_range(&zones, &[0], &[1]), [0, 1]);
        assert_eq!(zone_range(&zones, &[0], &[3]), [0, 1, 2, 3]);
    }
}
