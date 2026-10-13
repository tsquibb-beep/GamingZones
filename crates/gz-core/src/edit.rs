//! Editing a layout: what the editor's canvas does, as plain functions on [`Layout`].
//!
//! - Splitting a zone adds a guide *between* the zone's two sides, so the split stays
//!   proportional when those sides move (for example when the gaming zone moves).
//! - Dragging a divider keeps its anchor kind and changes its number, using the shortest
//!   decimal that lands exactly on the pixel (dragging to 1290 px of 6144 gives `21` %).
//! - Merging replaces a rectangle of zones with one zone bounded by the outer guides.
//! - Guides that nothing uses any more are removed.
//!
//! Positions are screen pixels on the monitor being edited (`area` = its work area,
//! `monitor` = the whole monitor), like everywhere else.

use std::collections::HashSet;
use std::fmt;

use crate::gaming::{GamingSize, GamingZone, Position};
use crate::geometry::Rect;
use crate::layout::{
    between, gaming_guide, percent_of, span, Anchor, Axis, Guide, GuidePositions, Layout, LayoutError, Zone,
    AREA_BOTTOM, AREA_LEFT, AREA_RIGHT, AREA_TOP,
};

#[derive(Clone, Debug, PartialEq)]
pub enum EditError {
    Layout(LayoutError),
    NoSuchZone(usize),
    NoSuchGuide(String),
    /// The zones to merge don't fill a rectangle (or the gaming zone is in the way).
    NotARectangle,
    NoGamingZone,
    /// The last zone of a layout without a gaming zone can't be deleted.
    LastZone,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(e) => write!(f, "{e}"),
            Self::NoSuchZone(i) => write!(f, "there is no zone {}", i + 1),
            Self::NoSuchGuide(id) => write!(f, "there is no guide \"{id}\""),
            Self::NotARectangle => write!(f, "those zones don't form a rectangle"),
            Self::NoGamingZone => write!(f, "the layout has no gaming zone"),
            Self::LastZone => write!(f, "a layout needs at least one zone"),
        }
    }
}

impl std::error::Error for EditError {}

impl From<LayoutError> for EditError {
    fn from(e: LayoutError) -> Self {
        Self::Layout(e)
    }
}

/// A layout with one zone covering the whole area.
pub fn new_layout(id: &str, name: &str) -> Layout {
    Layout {
        id: id.into(),
        name: name.into(),
        gaming_zones: Vec::new(),
        gaming_over_taskbar: false,
        guides: Vec::new(),
        zones: vec![Zone {
            left: AREA_LEFT.into(),
            right: AREA_RIGHT.into(),
            top: AREA_TOP.into(),
            bottom: AREA_BOTTOM.into(),
        }],
    }
}

/// An unused guide id: `g1`, `g2`, …
pub fn new_guide_id(layout: &Layout) -> String {
    (1..).map(|n| format!("g{n}")).find(|id| !layout.guides.iter().any(|g| &g.id == id)).expect("unbounded")
}

fn position(positions: &GuidePositions, id: &str) -> Result<i32, EditError> {
    positions.get(id).map(|&(_, v)| v).ok_or_else(|| EditError::NoSuchGuide(id.into()))
}

/// The shortest decimal (0 to 6 places) near `raw` for which `hits` is true, so stored
/// numbers stay readable (`21`, `0.5`) but still land on the exact pixel.
fn shortest(raw: f64, hits: impl Fn(f64) -> bool) -> f64 {
    let at = |places: i32| {
        let scale = 10f64.powi(places);
        (raw * scale).round() / scale
    };
    (0..=6).map(at).find(|&v| hits(v)).unwrap_or_else(|| at(6))
}

/// Splits zone `index` at `pos` (an x for [`Axis::Vertical`], a y for horizontal) with a new
/// guide between the zone's two sides. The two parts replace the zone, left/top part first.
/// Returns the new guide's id.
pub fn split(
    layout: &mut Layout,
    index: usize,
    axis: Axis,
    pos: i32,
    area: Rect,
    monitor: Rect,
) -> Result<String, EditError> {
    let zone = layout.zones.get(index).ok_or(EditError::NoSuchZone(index))?.clone();
    let positions = layout.guide_positions(area, monitor)?;
    let (a, b) = match axis {
        Axis::Vertical => (zone.left.clone(), zone.right.clone()),
        Axis::Horizontal => (zone.top.clone(), zone.bottom.clone()),
    };
    let (pa, pb) = (position(&positions, &a)?, position(&positions, &b)?);
    let fraction = if pa == pb {
        0.5
    } else {
        shortest((pos - pa) as f64 / (pb - pa) as f64, |f| between(pa, pb, f) == pos as i64)
    };
    let id = new_guide_id(layout);
    layout.guides.push(Guide { id: id.clone(), axis, anchor: Anchor::Between { a, b, fraction } });
    let (mut first, mut second) = (zone.clone(), zone);
    match axis {
        Axis::Vertical => {
            first.right = id.clone();
            second.left = id.clone();
        }
        Axis::Horizontal => {
            first.bottom = id.clone();
            second.top = id.clone();
        }
    }
    layout.zones.splice(index..=index, [first, second]);
    Ok(id)
}

/// Moves guide `id` to `pos`, keeping its kind of anchor (pixels stay pixels, a percentage
/// stays a percentage, a split stays proportional between its two guides).
pub fn move_guide(layout: &mut Layout, id: &str, pos: i32, area: Rect, monitor: Rect) -> Result<(), EditError> {
    let positions = layout.guide_positions(area, monitor)?;
    let guide = layout.guides.iter().find(|g| g.id == id).ok_or_else(|| EditError::NoSuchGuide(id.into()))?;
    let (start, end) = span(area, guide.axis);
    let pos = pos.clamp(start, end);
    let anchor = match &guide.anchor {
        Anchor::Px(_) => Anchor::Px(pos - start),
        Anchor::Percent(_) if end != start => {
            let raw = (pos - start) as f64 * 100.0 / (end - start) as f64;
            Anchor::Percent(shortest(raw, |p| percent_of(start, end, p) == pos as i64))
        }
        Anchor::Percent(p) => Anchor::Percent(*p),
        Anchor::Between { a, b, fraction } => {
            let (pa, pb) = (position(&positions, a)?, position(&positions, b)?);
            let fraction = if pa == pb {
                *fraction
            } else {
                shortest((pos - pa) as f64 / (pb - pa) as f64, |f| between(pa, pb, f) == pos as i64)
            };
            Anchor::Between { a: a.clone(), b: b.clone(), fraction }
        }
        Anchor::Offset { from, .. } => Anchor::Offset { from: from.clone(), px: pos - position(&positions, from)? },
    };
    guide_mut(layout, id)?.anchor = anchor;
    Ok(())
}

/// A divider's kind of anchor, for the editor to switch between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideKind {
    /// Pixels from the area's left or top edge.
    Px,
    /// Percent of the area.
    Percent,
}

/// Changes guide `id` to another kind of anchor, at the same position on this monitor.
pub fn convert_guide(
    layout: &mut Layout,
    id: &str,
    kind: GuideKind,
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    let pos = position(&layout.guide_positions(area, monitor)?, id)?;
    guide_mut(layout, id)?.anchor = match kind {
        GuideKind::Px => Anchor::Px(0),
        GuideKind::Percent => Anchor::Percent(0.0),
    };
    move_guide(layout, id, pos, area, monitor)
}

pub fn guide_mut<'a>(layout: &'a mut Layout, id: &str) -> Result<&'a mut Guide, EditError> {
    layout.guides.iter_mut().find(|g| g.id == id).ok_or_else(|| EditError::NoSuchGuide(id.into()))
}

/// How far guide `id` can be dragged: every zone it bounds keeps at least `min` px.
pub fn guide_limits(layout: &Layout, positions: &GuidePositions, id: &str, area: Rect, min: i32) -> (i32, i32) {
    let Some(&(axis, current)) = positions.get(id) else { return (0, 0) };
    let (mut lo, mut hi) = span(area, axis);
    let pos = |id: &str| positions.get(id).map(|&(_, v)| v);
    for z in &layout.zones {
        let (near, far) = match axis {
            Axis::Vertical => (&z.left, &z.right),
            Axis::Horizontal => (&z.top, &z.bottom),
        };
        if near == id {
            if let Some(far) = pos(far) {
                hi = hi.min(far - min);
            }
        }
        if far == id {
            if let Some(near) = pos(near) {
                lo = lo.max(near + min);
            }
        }
    }
    if lo > hi {
        (current, current)
    } else {
        (lo, hi)
    }
}

/// A user guide drawn as a divider on the canvas: where it is and where it borders zones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divider {
    pub guide: String,
    pub axis: Axis,
    /// x for a vertical divider, y for a horizontal one.
    pub pos: i32,
    /// The stretches along the line (y ranges for a vertical divider) where it is a zone's edge.
    pub spans: Vec<(i32, i32)>,
}

/// The dividers of the layout's own guides (not the area's or gaming zone's edges).
pub fn dividers(layout: &Layout, positions: &GuidePositions) -> Vec<Divider> {
    let pos = |id: &str| positions.get(id).map(|&(_, v)| v);
    layout
        .guides
        .iter()
        .filter_map(|g| {
            let spans: Vec<(i32, i32)> = layout
                .zones
                .iter()
                .filter_map(|z| {
                    let (near, far, a, b) = match g.axis {
                        Axis::Vertical => (&z.left, &z.right, &z.top, &z.bottom),
                        Axis::Horizontal => (&z.top, &z.bottom, &z.left, &z.right),
                    };
                    (*near == g.id || *far == g.id).then(|| Some((pos(a)?, pos(b)?))).flatten()
                })
                .collect();
            (!spans.is_empty()).then(|| Some(Divider { guide: g.id.clone(), axis: g.axis, pos: pos(&g.id)?, spans }))
        })
        .flatten()
        .collect()
}

/// Merges the selected zones, and any other zone inside the rectangle they span, into one.
/// Returns the merged zone's index.
pub fn merge(layout: &mut Layout, selected: &[usize], area: Rect, monitor: Rect) -> Result<usize, EditError> {
    let resolved = layout.resolve(area, monitor, 0)?;
    let count = layout.zones.len();
    if let Some(&bad) = selected.iter().find(|&&i| i >= count) {
        return Err(EditError::NoSuchZone(bad));
    }
    let Some(bounds) = selected.iter().map(|&i| resolved[i].rect).reduce(|a, b| a.union(&b)) else {
        return Err(EditError::NotARectangle);
    };
    let within =
        |r: &Rect| bounds.left <= r.left && r.right <= bounds.right && bounds.top <= r.top && r.bottom <= bounds.bottom;
    let inside: Vec<usize> = (0..count).filter(|&i| within(&resolved[i].rect)).collect();
    let rects: Vec<Rect> = inside.iter().map(|&i| resolved[i].rect).filter(|r| !r.is_empty()).collect();
    let area_of = |r: &Rect| r.width() as i64 * r.height() as i64;
    let overlap =
        |a: &Rect, b: &Rect| a.left.max(b.left) < a.right.min(b.right) && a.top.max(b.top) < a.bottom.min(b.bottom);
    let gaming_in_the_way = resolved[count..].iter().any(|g| overlap(&g.rect, &bounds));
    let tiles = rects.iter().map(area_of).sum::<i64>() == area_of(&bounds)
        && rects.iter().enumerate().all(|(n, a)| rects[n + 1..].iter().all(|b| !overlap(a, b)));
    if gaming_in_the_way || !tiles {
        return Err(EditError::NotARectangle);
    }
    // The outer guides: taken from a zone that has that edge on the bounding rectangle.
    let edge = |on_edge: &dyn Fn(&Rect) -> bool, side: &dyn Fn(&Zone) -> &String| {
        inside.iter().find(|&&i| on_edge(&resolved[i].rect)).map(|&i| side(&layout.zones[i]).clone())
    };
    let merged = Zone {
        left: edge(&|r| r.left == bounds.left, &|z| &z.left).ok_or(EditError::NotARectangle)?,
        right: edge(&|r| r.right == bounds.right, &|z| &z.right).ok_or(EditError::NotARectangle)?,
        top: edge(&|r| r.top == bounds.top, &|z| &z.top).ok_or(EditError::NotARectangle)?,
        bottom: edge(&|r| r.bottom == bounds.bottom, &|z| &z.bottom).ok_or(EditError::NotARectangle)?,
    };
    let first = inside[0];
    for &i in inside.iter().rev() {
        layout.zones.remove(i);
    }
    layout.zones.insert(first, merged);
    remove_unused_guides(layout);
    Ok(first)
}

/// Deletes zones (leaving a gap) and the guides only they used.
pub fn delete_zones(layout: &mut Layout, indices: &[usize]) -> Result<(), EditError> {
    if let Some(&bad) = indices.iter().find(|&&i| i >= layout.zones.len()) {
        return Err(EditError::NoSuchZone(bad));
    }
    let doomed: HashSet<usize> = indices.iter().copied().collect();
    if doomed.len() == layout.zones.len() && layout.gaming_zones.is_empty() {
        return Err(EditError::LastZone);
    }
    let mut i = 0;
    layout.zones.retain(|_| {
        i += 1;
        !doomed.contains(&(i - 1))
    });
    remove_unused_guides(layout);
    Ok(())
}

/// Removes guides that no zone uses, directly or through other guides.
pub fn remove_unused_guides(layout: &mut Layout) {
    let mut used: HashSet<String> =
        layout.zones.iter().flat_map(|z| [&z.left, &z.right, &z.top, &z.bottom]).cloned().collect();
    loop {
        let more: Vec<String> = layout
            .guides
            .iter()
            .filter(|g| used.contains(&g.id))
            .flat_map(|g| match &g.anchor {
                Anchor::Between { a, b, .. } => vec![a.clone(), b.clone()],
                Anchor::Offset { from, .. } => vec![from.clone()],
                Anchor::Px(_) | Anchor::Percent(_) => vec![],
            })
            .filter(|id| !used.contains(id))
            .collect();
        if more.is_empty() {
            break;
        }
        used.extend(more);
    }
    layout.guides.retain(|g| used.contains(&g.id));
}

/// Adds a gaming zone (centred) if the layout has none. Existing zones are kept; see
/// [`arrange_around_gaming_zone`].
pub fn add_gaming_zone(layout: &mut Layout, size: GamingSize) {
    if layout.gaming_zones.is_empty() {
        layout.gaming_zones.push(GamingZone { size, x: Position::default(), y: Position::default() });
    }
}

/// Replaces the zones with four around the gaming zone: a full-height column either side
/// and a strip above and below it.
pub fn arrange_around_gaming_zone(layout: &mut Layout) -> Result<(), EditError> {
    if layout.gaming_zones.is_empty() {
        return Err(EditError::NoGamingZone);
    }
    let gz = |edge: &str| gaming_guide(0, edge);
    let zone = |l: String, t: String, r: String, b: String| Zone { left: l, right: r, top: t, bottom: b };
    let a = |id: &str| id.to_string();
    layout.zones = vec![
        zone(a(AREA_LEFT), a(AREA_TOP), gz("left"), a(AREA_BOTTOM)),
        zone(gz("left"), a(AREA_TOP), gz("right"), gz("top")),
        zone(gz("left"), gz("bottom"), gz("right"), a(AREA_BOTTOM)),
        zone(gz("right"), a(AREA_TOP), a(AREA_RIGHT), a(AREA_BOTTOM)),
    ];
    remove_unused_guides(layout);
    Ok(())
}

/// Removes the gaming zone(s). Zones and guides that used its edges get a percentage guide
/// at the same place instead, so on this monitor nothing else moves.
pub fn remove_gaming_zone(layout: &mut Layout, area: Rect, monitor: Rect) -> Result<(), EditError> {
    if layout.gaming_zones.is_empty() {
        return Err(EditError::NoGamingZone);
    }
    let positions = layout.guide_positions(area, monitor)?;
    for i in 0..layout.gaming_zones.len() {
        for edge in ["left", "right", "top", "bottom"] {
            let old = gaming_guide(i, edge);
            if !references(layout, &old) {
                continue;
            }
            let (axis, pos) = positions[&old];
            let (start, end) = span(area, axis);
            let pos = pos.clamp(start, end);
            let percent = if end == start {
                0.0
            } else {
                let raw = (pos - start) as f64 * 100.0 / (end - start) as f64;
                shortest(raw, |p| percent_of(start, end, p) == pos as i64)
            };
            let id = new_guide_id(layout);
            layout.guides.push(Guide { id: id.clone(), axis, anchor: Anchor::Percent(percent) });
            rename_references(layout, &old, &id);
        }
    }
    layout.gaming_zones.clear();
    layout.gaming_over_taskbar = false;
    Ok(())
}

fn references(layout: &Layout, id: &str) -> bool {
    layout.zones.iter().any(|z| [&z.left, &z.right, &z.top, &z.bottom].into_iter().any(|s| s == id))
        || layout.guides.iter().any(|g| match &g.anchor {
            Anchor::Between { a, b, .. } => a == id || b == id,
            Anchor::Offset { from, .. } => from == id,
            Anchor::Px(_) | Anchor::Percent(_) => false,
        })
}

fn rename_references(layout: &mut Layout, old: &str, new: &str) {
    let fix = |s: &mut String| {
        if s == old {
            *s = new.to_string();
        }
    };
    for z in &mut layout.zones {
        fix(&mut z.left);
        fix(&mut z.right);
        fix(&mut z.top);
        fix(&mut z.bottom);
    }
    for g in &mut layout.guides {
        match &mut g.anchor {
            Anchor::Between { a, b, .. } => {
                fix(a);
                fix(b);
            }
            Anchor::Offset { from, .. } => fix(from),
            Anchor::Px(_) | Anchor::Percent(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LayoutsFile;

    const WORK: Rect = Rect::new(0, 0, 6144, 2500);
    const MONITOR: Rect = Rect::new(0, 0, 6144, 2560);

    fn rects(l: &Layout) -> Vec<Rect> {
        l.resolve(WORK, MONITOR, 0).unwrap().into_iter().map(|z| z.rect).collect()
    }

    #[test]
    fn split_lands_on_the_pixel_with_a_short_number() {
        let mut l = new_layout("t", "T");
        let id = split(&mut l, 0, Axis::Vertical, 1290, WORK, MONITOR).unwrap();
        assert_eq!(id, "g1");
        assert_eq!(l.guides[0].anchor, Anchor::Between { a: AREA_LEFT.into(), b: AREA_RIGHT.into(), fraction: 0.21 });
        assert_eq!(rects(&l), [Rect::new(0, 0, 1290, 2500), Rect::new(1290, 0, 6144, 2500)]);
        split(&mut l, 1, Axis::Horizontal, 1000, WORK, MONITOR).unwrap();
        assert_eq!(
            rects(&l),
            [Rect::new(0, 0, 1290, 2500), Rect::new(1290, 0, 6144, 1000), Rect::new(1290, 1000, 6144, 2500)]
        );
        assert!(matches!(split(&mut l, 9, Axis::Vertical, 5, WORK, MONITOR), Err(EditError::NoSuchZone(9))));
    }

    /// The editor can build Split focus exactly: the same rects as the built-in layout.
    #[test]
    fn rebuild_split_focus() {
        let mut l = new_layout("split-focus", "Split focus");
        split(&mut l, 0, Axis::Vertical, 1290, WORK, MONITOR).unwrap(); // 0 | 1
        split(&mut l, 1, Axis::Vertical, 4854, WORK, MONITOR).unwrap(); // 0 | 1 | 2
        split(&mut l, 0, Axis::Horizontal, 1250, WORK, MONITOR).unwrap(); // left: 0 / 1
        split(&mut l, 2, Axis::Horizontal, 375, WORK, MONITOR).unwrap(); // centre: 2 / 3
        split(&mut l, 3, Axis::Horizontal, 2125, WORK, MONITOR).unwrap(); // 3 / 4
        split(&mut l, 3, Axis::Vertical, 3072, WORK, MONITOR).unwrap(); // 3 | 4
        split(&mut l, 6, Axis::Horizontal, 1250, WORK, MONITOR).unwrap(); // right: 6 / 7
        let built_in = LayoutsFile::defaults();
        assert_eq!(rects(&l), rects(built_in.get("split-focus").unwrap()));
    }

    #[test]
    fn splits_stay_proportional_when_the_gaming_zone_moves() {
        let mut l = LayoutsFile::defaults().get("gaming-focus").unwrap().clone();
        // Split the bottom-left zone (x 0..1152) in half.
        split(&mut l, 1, Axis::Vertical, 576, WORK, MONITOR).unwrap();
        l.gaming_zones[0].size = GamingSize::Pixels { w: 1920, h: 1080 };
        l.gaming_zones[0].x = Position::Px(2000);
        // The left column is now 0..2000; its split is still halfway.
        assert_eq!(rects(&l)[1], Rect::new(0, 1250, 1000, 2500));
        assert_eq!(rects(&l)[2], Rect::new(1000, 1250, 2000, 2500));
    }

    #[test]
    fn moving_guides_keeps_their_kind() {
        let mut l = LayoutsFile::defaults().get("split-focus").unwrap().clone();
        // left-col is a percentage: dragging it to 1500 gives the shortest exact percentage.
        move_guide(&mut l, "left-col", 1500, WORK, MONITOR).unwrap();
        assert_eq!(l.guides[0].anchor, Anchor::Percent(24.41)); // 24.4 % would be 1499

        assert_eq!(rects(&l)[0].right, 1500);
        // centre-split is between left-col and right-col.
        move_guide(&mut l, "centre-split", 3000, WORK, MONITOR).unwrap();
        assert!(matches!(l.guides[2].anchor, Anchor::Between { .. }));
        assert_eq!(rects(&l)[3].right, 3000);
        // Px and Offset guides.
        l.guides[3].anchor = Anchor::Px(10);
        move_guide(&mut l, "side-split", 1300, WORK, MONITOR).unwrap();
        assert_eq!(l.guides[3].anchor, Anchor::Px(1300));
        l.guides[4].anchor = Anchor::Offset { from: AREA_BOTTOM.into(), px: -5 };
        move_guide(&mut l, "centre-top", 400, WORK, MONITOR).unwrap();
        assert_eq!(l.guides[4].anchor, Anchor::Offset { from: AREA_BOTTOM.into(), px: -2100 });
        assert_eq!(rects(&l)[2].bottom, 400);
        // Clamped to the area.
        move_guide(&mut l, "side-split", 99999, WORK, MONITOR).unwrap();
        assert_eq!(l.guides[3].anchor, Anchor::Px(2500));
        assert!(matches!(move_guide(&mut l, "nope", 1, WORK, MONITOR), Err(EditError::NoSuchGuide(_))));
    }

    #[test]
    fn converting_guides_keeps_them_in_place() {
        let mut l = LayoutsFile::defaults().get("split-focus").unwrap().clone();
        convert_guide(&mut l, "centre-split", GuideKind::Px, WORK, MONITOR).unwrap();
        assert_eq!(l.guides[2].anchor, Anchor::Px(3072));
        convert_guide(&mut l, "centre-split", GuideKind::Percent, WORK, MONITOR).unwrap();
        assert_eq!(l.guides[2].anchor, Anchor::Percent(50.0));
        assert_eq!(rects(&l), rects(LayoutsFile::defaults().get("split-focus").unwrap()));
    }

    #[test]
    fn limits_and_dividers() {
        let l = LayoutsFile::defaults().get("split-focus").unwrap().clone();
        let positions = l.guide_positions(WORK, MONITOR).unwrap();
        // left-col (1290) is bounded by area.left on one side and centre-split (3072) on the other.
        assert_eq!(guide_limits(&l, &positions, "left-col", WORK, 50), (50, 3022));
        let d = dividers(&l, &positions);
        assert_eq!(d.len(), 6);
        let side = d.iter().find(|d| d.guide == "side-split").unwrap();
        assert_eq!((side.axis, side.pos), (Axis::Horizontal, 1250));
        assert!(side.spans.contains(&(0, 1290)) && side.spans.contains(&(4854, 6144)));
    }

    #[test]
    fn merge_zones() {
        let mut l = LayoutsFile::defaults().get("split-focus").unwrap().clone();
        // Zones 4 and 5 (the two middle ones) → one zone; centre-split goes.
        assert_eq!(merge(&mut l, &[3, 4], WORK, MONITOR), Ok(3));
        assert_eq!(rects(&l)[3], Rect::new(1290, 375, 4854, 2125));
        assert!(!l.guides.iter().any(|g| g.id == "centre-split"));
        // Zones 3 and 5 (top strip and bottom strip) span the middle zone too: all three merge.
        let mut l2 = l.clone();
        assert_eq!(merge(&mut l2, &[2, 4], WORK, MONITOR), Ok(2));
        assert_eq!(rects(&l2)[2], Rect::new(1290, 0, 4854, 2500));
        // Zone 1 (left top) and zone 3 (top strip): their bounding box also covers part of the
        // middle zone, which sticks out of it, so it isn't a rectangle of whole zones.
        assert_eq!(merge(&mut l, &[0, 2], WORK, MONITOR), Err(EditError::NotARectangle));
    }

    #[test]
    fn merge_refuses_the_gaming_zone() {
        let mut l = LayoutsFile::defaults().get("gaming-focus").unwrap().clone();
        // Top strip and bottom strip: the gaming zone is between them.
        assert_eq!(merge(&mut l, &[2, 3], WORK, MONITOR), Err(EditError::NotARectangle));
        // The gaming zone itself (index 6) isn't a zone you can merge.
        assert_eq!(merge(&mut l, &[6], WORK, MONITOR), Err(EditError::NoSuchZone(6)));
        // Left top + left bottom is fine.
        assert_eq!(merge(&mut l, &[0, 1], WORK, MONITOR), Ok(0));
        assert_eq!(rects(&l)[0], Rect::new(0, 0, 1152, 2500));
    }

    #[test]
    fn delete_and_unused_guides() {
        let mut l = LayoutsFile::defaults().get("wfh").unwrap().clone();
        delete_zones(&mut l, &[0, 2]).unwrap(); // the left column
        assert_eq!(rects(&l), [Rect::new(3072, 0, 6144, 1250), Rect::new(3072, 1250, 6144, 2500)]);
        assert_eq!(l.guides.len(), 2); // mid-x is still the remaining zones' left edge
        delete_zones(&mut l, &[1]).unwrap();
        assert_eq!(l.guides.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["mid-x", "mid-y"]);
        assert_eq!(delete_zones(&mut l, &[0]), Err(EditError::LastZone));
        // A guide only used by another guide is kept while that one is used.
        let mut l = new_layout("t", "T");
        split(&mut l, 0, Axis::Vertical, 1000, WORK, MONITOR).unwrap();
        split(&mut l, 1, Axis::Vertical, 3000, WORK, MONITOR).unwrap(); // g2 between g1 and area.right
        delete_zones(&mut l, &[0, 1]).unwrap(); // only zone g2..area.right is left
        assert_eq!(l.guides.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["g1", "g2"]);
        assert_eq!(rects(&l), [Rect::new(3000, 0, 6144, 2500)]);
    }

    #[test]
    fn gaming_zone_on_and_off() {
        let mut l = new_layout("t", "T");
        assert_eq!(arrange_around_gaming_zone(&mut l), Err(EditError::NoGamingZone));
        add_gaming_zone(&mut l, GamingSize::Pixels { w: 3840, h: 2160 });
        arrange_around_gaming_zone(&mut l).unwrap();
        let with = rects(&l);
        assert_eq!(with[4], Rect::new(1152, 170, 4992, 2330));
        assert_eq!(with[0], Rect::new(0, 0, 1152, 2500));
        // Removing it leaves the other zones exactly where they were.
        remove_gaming_zone(&mut l, WORK, MONITOR).unwrap();
        assert!(l.gaming_zones.is_empty());
        assert_eq!(rects(&l), with[..4]);
        assert!(l.validate().is_ok());
        assert_eq!(remove_gaming_zone(&mut l, WORK, MONITOR), Err(EditError::NoGamingZone));
    }
}
