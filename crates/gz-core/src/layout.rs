//! Layouts: guides (lines) and zones (rectangles bounded by four guides).
//!
//! A layout is resolved against an *area* (normally a monitor's work area, in screen
//! coordinates) to get each zone's rectangle in physical pixels. Guides are positioned
//! relative to the area, to the gaming zone or to other guides, so the same layout works
//! on any monitor.
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

use crate::gaming::{GamingZone, SizeError};
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
    pub fn name(self) -> &'static str {
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
    /// The gaming zone (the editor allows at most one; its guides are `gz0.*`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaming_zones: Vec<GamingZone>,
    /// The gaming zone may extend over the taskbar (it's kept inside the monitor instead of
    /// the work area). Other zones always stay in the work area.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gaming_over_taskbar: bool,
    #[serde(default)]
    pub guides: Vec<Guide>,
    #[serde(default)]
    pub zones: Vec<Zone>,
}

/// The built-in guides on the area's edges. User guide ids may not contain a `.`, so they
/// can never clash with these or with the gaming zone's (`gz0.left` etc., see [`gaming_guide`]).
pub const AREA_LEFT: &str = "area.left";
pub const AREA_RIGHT: &str = "area.right";
pub const AREA_TOP: &str = "area.top";
pub const AREA_BOTTOM: &str = "area.bottom";

/// The built-in guide on one edge of gaming zone `index`: `gz0.left`, `gz0.right`, `gz0.top`
/// or `gz0.bottom` (`edge` is the part after the dot).
pub fn gaming_guide(index: usize, edge: &str) -> String {
    format!("gz{index}.{edge}")
}

/// The built-in guides on a rectangle's edges: `(prefix.left, …)` with their positions.
fn edge_guides(prefix: &str, r: Rect) -> [(String, Axis, i32); 4] {
    [
        (format!("{prefix}.left"), Axis::Vertical, r.left),
        (format!("{prefix}.right"), Axis::Vertical, r.right),
        (format!("{prefix}.top"), Axis::Horizontal, r.top),
        (format!("{prefix}.bottom"), Axis::Horizontal, r.bottom),
    ]
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
    GamingZone {
        index: usize,
        error: SizeError,
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
            Self::GamingZone { index, error } => write!(f, "gaming zone gz{index}: {error}"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Guide id → (axis, position in screen px).
pub type GuidePositions = HashMap<String, (Axis, i32)>;

/// A zone resolved against an area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedZone {
    pub rect: Rect,
    /// Too small to use (width or height below the threshold, or not positive). Never a
    /// snap target.
    pub disabled: bool,
    /// A gaming zone: windows snapped to it alone get their client area fitted, not their frame.
    pub gaming: bool,
}

impl Layout {
    /// Checks guide ids, references, axes, cycles and gaming zone sizes without needing an area.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let area = Rect::new(0, 0, 1000, 1000);
        self.resolve(area, area, 0).map(|_| ())
    }

    /// Resolves every zone against `area` (the monitor's work area; `monitor` is the whole
    /// monitor, used only by a gaming zone allowed over the taskbar).
    ///
    /// The layout's zones come first, in order, then the gaming zones. A zone narrower or
    /// shorter than `deadzone_px` (or not positive) is marked disabled; gaming zones have an
    /// exact size and aren't.
    pub fn resolve(&self, area: Rect, monitor: Rect, deadzone_px: i32) -> Result<Vec<ResolvedZone>, LayoutError> {
        if self.zones.is_empty() && self.gaming_zones.is_empty() {
            return Err(LayoutError::NoZones);
        }
        let positions = self.guide_positions(area, monitor)?;
        let pos = |referenced_by: String, id: &str, expected: Axis| -> Result<i32, LayoutError> {
            let (axis, value) = *positions
                .get(id)
                .ok_or_else(|| LayoutError::UnknownGuide { referenced_by: referenced_by.clone(), id: id.into() })?;
            if axis != expected {
                return Err(LayoutError::WrongAxis { referenced_by, id: id.into(), expected });
            }
            Ok(value)
        };
        let zone_rect = |z: &Zone, name: &dyn Fn(&str) -> String| -> Result<Rect, LayoutError> {
            Ok(Rect::new(
                pos(name("left"), &z.left, Axis::Vertical)?,
                pos(name("top"), &z.top, Axis::Horizontal)?,
                pos(name("right"), &z.right, Axis::Vertical)?,
                pos(name("bottom"), &z.bottom, Axis::Horizontal)?,
            ))
        };
        let min = deadzone_px.max(1);
        let zones = self.zones.iter().enumerate().map(|(i, z)| {
            let rect = zone_rect(z, &|side| format!("zone {} {side}", i + 1))?;
            Ok(ResolvedZone { rect, disabled: rect.width() < min || rect.height() < min, gaming: false })
        });
        let gaming = (0..self.gaming_zones.len()).map(|i| {
            let edge = |side: &str| gaming_guide(i, side);
            let rect = Rect::new(
                positions[&edge("left")].1,
                positions[&edge("top")].1,
                positions[&edge("right")].1,
                positions[&edge("bottom")].1,
            );
            Ok(ResolvedZone { rect, disabled: rect.is_empty(), gaming: true })
        });
        zones.chain(gaming).collect()
    }

    /// Every guide's axis and position (screen px), built-in ones included.
    pub fn guide_positions(&self, area: Rect, monitor: Rect) -> Result<GuidePositions, LayoutError> {
        Resolver::new(self, area, monitor)?.resolve_all()
    }

    /// Each gaming zone's rectangle (see [`GamingZone::place`]).
    pub fn gaming_rects(&self, area: Rect, monitor: Rect) -> Result<Vec<Rect>, LayoutError> {
        let mut resolver = Resolver::new(self, area, monitor)?;
        (0..self.gaming_zones.len()).map(|i| resolver.gaming(i).map(|g| g.rect)).collect()
    }

    /// Gaming zone `index`'s space (the area, or its `within` guides) and the bounds it's
    /// kept inside (the space, stretched to the monitor's edges over the taskbar if allowed).
    pub fn gaming_space(&self, index: usize, area: Rect, monitor: Rect) -> Result<(Rect, Rect), LayoutError> {
        let g = Resolver::new(self, area, monitor)?.gaming(index)?;
        Ok((g.space, g.bounds))
    }
}

/// A zone's number (1, 2, …), counting only the layout's zones that have a size here: the
/// empty strips beside a gaming zone that fills its space don't get one. `None` for the
/// gaming zone and empty zones.
pub fn zone_number(zones: &[ResolvedZone], i: usize) -> Option<usize> {
    let counts = |z: &ResolvedZone| !z.gaming && !z.rect.is_empty();
    counts(zones.get(i)?).then(|| zones[..=i].iter().filter(|z| counts(z)).count())
}

/// A zone's label in the overlay and the log: its [`zone_number`], `G` for the gaming zone
/// (`G1`, `G2`, … if a layout has several), or `-` for an empty zone.
pub fn zone_label(zones: &[ResolvedZone], i: usize) -> String {
    if !zones[i].gaming {
        return zone_number(zones, i).map_or("-".to_string(), |n| n.to_string());
    }
    let gaming = zones.iter().filter(|z| z.gaming).count();
    let nth = zones[..i].iter().filter(|z| z.gaming).count() + 1;
    if gaming == 1 {
        "G".to_string()
    } else {
        format!("G{nth}")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State<T> {
    Visiting,
    Done(T),
}

/// A gaming zone worked out by the [`Resolver`].
#[derive(Clone, Copy, PartialEq)]
struct PlacedGaming {
    rect: Rect,
    space: Rect,
    bounds: Rect,
}

/// Evaluates guides in dependency order (depth-first with cycle detection). A gaming zone
/// is placed when one of its edges is first needed, since its space can depend on guides.
struct Resolver<'a> {
    area: Rect,
    monitor: Rect,
    layout: &'a Layout,
    guides: HashMap<&'a str, &'a Guide>,
    state: HashMap<&'a str, State<i32>>,
    gaming: Vec<Option<State<PlacedGaming>>>,
    /// Current dependency chain, for the cycle error message.
    stack: Vec<String>,
}

impl<'a> Resolver<'a> {
    fn new(layout: &'a Layout, area: Rect, monitor: Rect) -> Result<Self, LayoutError> {
        let mut guides = HashMap::new();
        for g in &layout.guides {
            if g.id.contains('.') {
                return Err(LayoutError::ReservedGuideId(g.id.clone()));
            }
            if guides.insert(g.id.as_str(), g).is_some() {
                return Err(LayoutError::DuplicateGuide(g.id.clone()));
            }
        }
        Ok(Self {
            area,
            monitor,
            layout,
            guides,
            state: HashMap::new(),
            gaming: vec![None; layout.gaming_zones.len()],
            stack: Vec::new(),
        })
    }

    fn resolve_all(mut self) -> Result<GuidePositions, LayoutError> {
        let mut out: GuidePositions =
            edge_guides("area", self.area).into_iter().map(|(id, axis, v)| (id, (axis, v))).collect();
        for i in 0..self.layout.gaming_zones.len() {
            let g = self.gaming(i)?;
            out.extend(edge_guides(&format!("gz{i}"), g.rect).into_iter().map(|(id, axis, v)| (id, (axis, v))));
        }
        let ids: Vec<&'a str> = self.guides.keys().copied().collect();
        for id in ids {
            let axis = self.guides[id].axis;
            let value = self.position(id, axis, &|| format!("guide \"{id}\""))?;
            out.insert(id.to_string(), (axis, value));
        }
        Ok(out)
    }

    fn cycle(&self, key: &str) -> LayoutError {
        let start = self.stack.iter().position(|s| s == key).unwrap_or(0);
        let mut cycle: Vec<String> = self.stack[start..].to_vec();
        cycle.push(key.to_string());
        LayoutError::Cycle(cycle)
    }

    /// Places gaming zone `index` (once).
    fn gaming(&mut self, index: usize) -> Result<PlacedGaming, LayoutError> {
        let key = format!("gz{index}");
        match self.gaming.get(index) {
            Some(Some(State::Done(g))) => return Ok(*g),
            Some(Some(State::Visiting)) => return Err(self.cycle(&key)),
            Some(None) => {}
            None => return Err(LayoutError::UnknownGuide { referenced_by: "the editor".into(), id: key }),
        }
        self.gaming[index] = Some(State::Visiting);
        self.stack.push(key.clone());
        let g = &self.layout.gaming_zones[index];
        let space = match &g.within {
            None => self.area,
            Some(z) => {
                let me = || format!("gaming zone {key}'s space");
                Rect::new(
                    self.position(&z.left, Axis::Vertical, &me)?,
                    self.position(&z.top, Axis::Horizontal, &me)?,
                    self.position(&z.right, Axis::Vertical, &me)?,
                    self.position(&z.bottom, Axis::Horizontal, &me)?,
                )
            }
        };
        let bounds = if self.layout.gaming_over_taskbar {
            let (a, m) = (self.area, self.monitor);
            let stretch = |side: i32, area_side: i32, monitor_side: i32| {
                if side == area_side {
                    monitor_side
                } else {
                    side
                }
            };
            Rect::new(
                stretch(space.left, a.left, m.left),
                stretch(space.top, a.top, m.top),
                stretch(space.right, a.right, m.right),
                stretch(space.bottom, a.bottom, m.bottom),
            )
        } else {
            space
        };
        let rect = g.place(space, bounds).map_err(|error| LayoutError::GamingZone { index, error })?;
        let placed = PlacedGaming { rect, space, bounds };
        self.stack.pop();
        self.gaming[index] = Some(State::Done(placed));
        Ok(placed)
    }

    fn span(&self, axis: Axis) -> (i32, i32) {
        span(self.area, axis)
    }

    /// Position of guide `id`, which must be on `expected` axis.
    fn position(&mut self, id: &str, expected: Axis, referenced_by: &dyn Fn() -> String) -> Result<i32, LayoutError> {
        let wrong_axis = |axis: Axis| {
            (axis != expected).then(|| LayoutError::WrongAxis {
                referenced_by: referenced_by(),
                id: id.into(),
                expected,
            })
        };
        if let Some((_, axis, value)) = edge_guides("area", self.area).into_iter().find(|(g, _, _)| g == id) {
            return wrong_axis(axis).map_or(Ok(value), Err);
        }
        if let Some((index, edge)) = parse_gaming_guide(id).filter(|&(i, _)| i < self.layout.gaming_zones.len()) {
            let rect = self.gaming(index)?.rect;
            if let Some((_, axis, value)) = edge_guides("", rect).into_iter().find(|(g, _, _)| g[1..] == *edge) {
                return wrong_axis(axis).map_or(Ok(value), Err);
            }
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
            Some(State::Visiting) => return Err(self.cycle(key)),
            None => {}
        }
        self.state.insert(key, State::Visiting);
        self.stack.push(key.to_string());

        let (start, end) = self.span(guide.axis);
        let bad = || LayoutError::BadNumber { guide: key.to_string() };
        let me = || format!("guide \"{key}\"");
        let raw = match &guide.anchor {
            Anchor::Px(px) => start as i64 + *px as i64,
            Anchor::Percent(p) => {
                if !p.is_finite() {
                    return Err(bad());
                }
                percent_of(start, end, *p)
            }
            Anchor::Between { a, b, fraction } => {
                if !fraction.is_finite() {
                    return Err(bad());
                }
                let a = self.position(a, guide.axis, &me)?;
                let b = self.position(b, guide.axis, &me)?;
                between(a, b, *fraction)
            }
            Anchor::Offset { from, px } => self.position(from, guide.axis, &me)? as i64 + *px as i64,
        };
        let value = raw.clamp(start as i64, end as i64) as i32;

        self.stack.pop();
        self.state.insert(key, State::Done(value));
        Ok(value)
    }
}

/// `gz0.left` → `(0, "left")`.
pub fn parse_gaming_guide(id: &str) -> Option<(usize, &str)> {
    let (zone, edge) = id.strip_prefix("gz")?.split_once('.')?;
    Some((zone.parse().ok()?, edge))
}

/// The area's start and end along the axis a guide measures.
pub fn span(area: Rect, axis: Axis) -> (i32, i32) {
    match axis {
        Axis::Vertical => (area.left, area.right),
        Axis::Horizontal => (area.top, area.bottom),
    }
}

/// [`Anchor::Percent`]'s position (before clamping to the area).
pub(crate) fn percent_of(start: i32, end: i32, percent: f64) -> i64 {
    start as i64 + round((end - start) as f64 * percent / 100.0)
}

/// [`Anchor::Between`]'s position.
pub(crate) fn between(a: i32, b: i32, fraction: f64) -> i64 {
    a as i64 + round((b - a) as f64 * fraction)
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
    use crate::gaming::{GamingSize, Position};

    fn guide(id: &str, axis: Axis, anchor: Anchor) -> Guide {
        Guide { id: id.into(), axis, anchor }
    }

    fn zone(l: &str, t: &str, r: &str, b: &str) -> Zone {
        Zone { left: l.into(), right: r.into(), top: t.into(), bottom: b.into() }
    }

    fn layout(guides: Vec<Guide>, zones: Vec<Zone>) -> Layout {
        Layout { id: "t".into(), name: "Test".into(), gaming_zones: vec![], gaming_over_taskbar: false, guides, zones }
    }

    fn rz(l: i32, t: i32, r: i32, b: i32) -> ResolvedZone {
        ResolvedZone { rect: Rect::new(l, t, r, b), disabled: false, gaming: false }
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
        let zones = halves().resolve(Rect::new(100, 50, 1100, 850), Rect::new(100, 50, 1100, 850), 0).unwrap();
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
        let zones = l.resolve(Rect::new(0, 0, 1000, 500), Rect::new(0, 0, 1000, 500), 0).unwrap();
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
        assert_eq!(
            l.resolve(Rect::new(10, 0, 1010, 100), Rect::new(10, 0, 1010, 100), 0).unwrap()[0].rect,
            Rect::new(10, 0, 1010, 100)
        );
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
        let zones = l.resolve(Rect::new(0, 0, 1000, 500), Rect::new(0, 0, 1000, 500), 0).unwrap();
        assert_eq!(zones.iter().map(|z| z.disabled).collect::<Vec<_>>(), [false, true, true, true]);
        assert_eq!(zones[2].rect.width(), -300);
    }

    #[test]
    fn deadzone_threshold() {
        let zones = halves().resolve(Rect::new(0, 0, 1000, 500), Rect::new(0, 0, 1000, 500), 400).unwrap();
        assert!(!zones[0].disabled); // 500 × 500
        let zones = halves().resolve(Rect::new(0, 0, 700, 500), Rect::new(0, 0, 700, 500), 400).unwrap();
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
        assert_eq!(
            l.resolve(Rect::new(0, 0, 1000, 1000), Rect::new(0, 0, 1000, 1000), 0).unwrap()[0].rect,
            Rect::new(500, 100, 750, 900)
        );
        let again: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(again, l);
    }

    // --- gaming zone ---

    /// A 400×300 gaming zone with a column either side and a strip above and below it.
    fn around(x: Position, y: Position) -> Layout {
        let mut l = layout(
            vec![],
            vec![
                zone(AREA_LEFT, AREA_TOP, "gz0.left", AREA_BOTTOM),
                zone("gz0.left", AREA_TOP, "gz0.right", "gz0.top"),
                zone("gz0.left", "gz0.bottom", "gz0.right", AREA_BOTTOM),
                zone("gz0.right", AREA_TOP, AREA_RIGHT, AREA_BOTTOM),
            ],
        );
        l.gaming_zones = vec![GamingZone::new(GamingSize::Pixels { w: 400, h: 300 }, x, y)];
        l
    }

    #[test]
    fn neighbours_follow_the_gaming_zone() {
        let area = Rect::new(0, 0, 1000, 500);
        let rects = |l: Layout| l.resolve(area, area, 0).unwrap().into_iter().map(|z| z.rect).collect::<Vec<_>>();
        assert_eq!(
            rects(around(Position::default(), Position::default())),
            [
                Rect::new(0, 0, 300, 500),
                Rect::new(300, 0, 700, 100),
                Rect::new(300, 400, 700, 500),
                Rect::new(700, 0, 1000, 500),
                Rect::new(300, 100, 700, 400), // the gaming zone, last
            ]
        );
        // Moving the gaming zone moves its neighbours' edges with it.
        assert_eq!(
            rects(around(Position::Px(100), Position::Px(0))),
            [
                Rect::new(0, 0, 100, 500),
                Rect::new(100, 0, 500, 0),
                Rect::new(100, 300, 500, 500),
                Rect::new(500, 0, 1000, 500),
                Rect::new(100, 0, 500, 300),
            ]
        );
    }

    #[test]
    fn gaming_zone_flags_and_labels() {
        let area = Rect::new(0, 0, 1000, 500);
        let zones = around(Position::default(), Position::Px(0)).resolve(area, area, 150).unwrap();
        // The strip above is 0 px, the one below 200 px (above the 150 px threshold).
        assert_eq!(
            zones.iter().map(|z| (z.disabled, z.gaming)).collect::<Vec<_>>(),
            [(false, false), (true, false), (false, false), (false, false), (false, true),]
        );
        // The empty strip above has no number; the others are numbered without it.
        let labels: Vec<String> = (0..zones.len()).map(|i| zone_label(&zones, i)).collect();
        assert_eq!(labels, ["1", "-", "2", "3", "G"]);
        assert_eq!((zone_number(&zones, 1), zone_number(&zones, 3), zone_number(&zones, 4)), (None, Some(3), None));

        // The gaming zone is a snap target like any other.
        assert_eq!(zones_at(&zones, 500, 200, None), [4]);
        let mut two = zones.clone();
        two.push(ResolvedZone { gaming: true, ..zones[4] });
        assert_eq!(zone_label(&two, 5), "G2");
    }

    #[test]
    fn gaming_zone_errors() {
        // gz0 guides only exist when there is a gaming zone.
        let mut l = around(Position::default(), Position::default());
        l.gaming_zones.clear();
        assert!(matches!(l.validate(), Err(LayoutError::UnknownGuide { id, .. }) if id == "gz0.left"));
        // gz0.top is horizontal.
        let mut l = around(Position::default(), Position::default());
        l.zones[0].left = "gz0.top".into();
        assert!(matches!(l.validate(), Err(LayoutError::WrongAxis { .. })));
        // A zero size is reported with the zone's index.
        let mut l = around(Position::default(), Position::default());
        l.gaming_zones[0].size = GamingSize::Pixels { w: 0, h: 10 };
        assert_eq!(l.validate(), Err(LayoutError::GamingZone { index: 0, error: SizeError::Zero }));
        // A layout may be just a gaming zone.
        let mut l = around(Position::default(), Position::default());
        l.zones.clear();
        assert_eq!(l.resolve(Rect::new(0, 0, 1000, 500), Rect::new(0, 0, 1000, 500), 0).unwrap().len(), 1);
        assert_eq!(gaming_guide(0, "left"), "gz0.left");
    }

    #[test]
    fn gaming_zone_over_the_taskbar() {
        let (work, monitor) = (Rect::new(0, 0, 1000, 460), Rect::new(0, 0, 1000, 500));
        let mut l = around(Position::default(), Position::default());
        l.gaming_zones[0].size = GamingSize::Pixels { w: 400, h: 500 };
        // Not allowed: shrunk to the work area's height.
        assert_eq!(l.resolve(work, monitor, 0).unwrap()[4].rect, Rect::new(300, 0, 700, 460));
        // Allowed: full height, over the taskbar; the strip below it is gone (negative).
        l.gaming_over_taskbar = true;
        let zones = l.resolve(work, monitor, 0).unwrap();
        assert_eq!(zones[4].rect, Rect::new(300, 0, 700, 500));
        assert!(zones[2].disabled);
        // Other zones stay in the work area.
        assert_eq!(zones[0].rect, Rect::new(0, 0, 300, 460));
    }

    #[test]
    fn gaming_zone_within_guides() {
        // A gaming zone in the right half (mid..area.right), centred there.
        let mut l = halves();
        l.gaming_zones = vec![GamingZone {
            within: Some(zone("mid", AREA_TOP, AREA_RIGHT, AREA_BOTTOM)),
            ..GamingZone::new(GamingSize::Pixels { w: 200, h: 100 }, Position::default(), Position::Px(0))
        }];
        let area = Rect::new(0, 0, 1000, 500);
        assert_eq!(l.gaming_rects(area, area).unwrap(), [Rect::new(650, 0, 850, 100)]);
        assert_eq!(
            l.gaming_space(0, area, area).unwrap(),
            (Rect::new(500, 0, 1000, 500), Rect::new(500, 0, 1000, 500))
        );
        // Over the taskbar, only the space's sides on the area's edges stretch.
        l.gaming_over_taskbar = true;
        let monitor = Rect::new(0, 0, 1000, 540);
        assert_eq!(l.gaming_space(0, area, monitor).unwrap().1, Rect::new(500, 0, 1000, 540));
        // JSON round trip.
        let json = serde_json::to_string(&l).unwrap();
        assert!(json.contains(r#""within":{"left":"mid""#), "{json}");
        assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), l);
        // Its space can't depend on its own edges.
        l.guides[0].anchor = Anchor::Offset { from: "gz0.left".into(), px: -10 };
        assert!(matches!(l.validate(), Err(LayoutError::Cycle(_))), "{:?}", l.validate());
        // Unknown guides in its space are reported.
        let mut l = halves();
        l.gaming_zones = vec![GamingZone {
            within: Some(zone("nope", AREA_TOP, AREA_RIGHT, AREA_BOTTOM)),
            ..GamingZone::new(GamingSize::Pixels { w: 200, h: 100 }, Position::default(), Position::default())
        }];
        assert!(matches!(l.validate(), Err(LayoutError::UnknownGuide { id, .. }) if id == "nope"));
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
