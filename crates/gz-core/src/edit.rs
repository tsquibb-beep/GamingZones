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

use crate::gaming::{Align, GamingSize, GamingZone, Position};
use crate::geometry::Rect;
use crate::layout::{
    between, gaming_guide, parse_gaming_guide, percent_of, span, Anchor, Axis, Guide, GuidePositions, Layout,
    LayoutError, ResolvedZone, Zone, AREA_BOTTOM, AREA_LEFT, AREA_RIGHT, AREA_TOP,
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
    /// The change would resize this (locked) zone.
    Locked(usize),
    /// Neither edge of the zone can move to give it that size.
    CantResize,
    AlreadyHasGamingZone,
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
            Self::Locked(i) => write!(f, "zone {} is locked and can't be pushed that way", i + 1),
            Self::CantResize => write!(f, "the zone can't be made that size (its edges can't move that far)"),
            Self::AlreadyHasGamingZone => write!(f, "the layout already has a gaming zone"),
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

/// Moves divider `id` to `pos` the way the editor does: like [`move_guide`], but every
/// other guide stays where it is on this monitor (a split that depended on `id` gets a new
/// fraction instead of moving with it), so only the zones on either side change.
///
/// A zone in `locked` keeps its size: it's pushed along (both its edges move), and the next
/// unlocked zone gives or takes the space. See [`pushed_guides`].
pub fn move_divider(
    layout: &mut Layout,
    id: &str,
    pos: i32,
    locked: &[usize],
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    let before = layout.guide_positions(area, monitor)?;
    let pushed = pushed_guides(layout, id, locked)?;
    let delta = pos - position(&before, id)?;
    let targets: Vec<(String, i32)> = layout
        .guides
        .iter()
        .map(|g| {
            let at = before[&g.id].1;
            (g.id.clone(), if pushed.contains(&g.id) { at + delta } else { at })
        })
        .collect();
    // Moving a guide can move others that depend on it, so go round until all are in place.
    for _ in 0..=targets.len() {
        let now = layout.guide_positions(area, monitor)?;
        let off: Vec<&(String, i32)> =
            targets.iter().filter(|(g, at)| now.get(g).map(|&(_, v)| v) != Some(*at)).collect();
        if off.is_empty() {
            break;
        }
        for (g, at) in off {
            move_guide(layout, g, *at, area, monitor)?;
        }
    }
    Ok(())
}

/// The guides that move together when divider `id` moves: `id`, and the far edge of every
/// locked zone it pushes (and so on, through a row of locked zones). A locked zone that would
/// have to push the area's edge or the gaming zone can't move: [`EditError::Locked`].
pub fn pushed_guides(layout: &Layout, id: &str, locked: &[usize]) -> Result<Vec<String>, EditError> {
    let axis = layout.guides.iter().find(|g| g.id == id).ok_or_else(|| EditError::NoSuchGuide(id.into()))?.axis;
    let mut pushed = vec![id.to_string()];
    loop {
        let mut more = false;
        for &i in locked {
            let Some(z) = layout.zones.get(i) else { continue };
            let (near, far) = sides(z, axis);
            let other = match (pushed.contains(near), pushed.contains(far)) {
                (true, false) => far,
                (false, true) => near,
                _ => continue,
            };
            if !layout.guides.iter().any(|g| &g.id == other) {
                return Err(EditError::Locked(i));
            }
            pushed.push(other.clone());
            more = true;
        }
        if !more {
            return Ok(pushed);
        }
    }
}

/// A zone's two sides across `axis`: (left, right) for a vertical divider, (top, bottom) otherwise.
fn sides(z: &Zone, axis: Axis) -> (&String, &String) {
    match axis {
        Axis::Vertical => (&z.left, &z.right),
        Axis::Horizontal => (&z.top, &z.bottom),
    }
}

/// Whether guide `id` is a gaming zone's edge or is positioned from one (directly or through
/// other guides), so it moves when the gaming zone does.
pub fn follows_gaming(layout: &Layout, id: &str) -> bool {
    let mut todo = vec![id.to_string()];
    let mut seen = HashSet::new();
    while let Some(id) = todo.pop() {
        if parse_gaming_guide(&id).is_some() {
            return true;
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        match layout.guides.iter().find(|g| g.id == id).map(|g| &g.anchor) {
            Some(Anchor::Between { a, b, .. }) => todo.extend([a.clone(), b.clone()]),
            Some(Anchor::Offset { from, .. }) => todo.push(from.clone()),
            _ => {}
        }
    }
    false
}

pub fn guide_mut<'a>(layout: &'a mut Layout, id: &str) -> Result<&'a mut Guide, EditError> {
    layout.guides.iter_mut().find(|g| g.id == id).ok_or_else(|| EditError::NoSuchGuide(id.into()))
}

/// How far guide `id` can be dragged (pushing locked zones, see [`move_divider`]): every
/// zone that gets smaller keeps at least `min` px. Zones beside the gaming zone don't count,
/// as they move with it. A divider that can't move at all gives (current, current).
pub fn guide_limits(
    layout: &Layout,
    positions: &GuidePositions,
    id: &str,
    locked: &[usize],
    area: Rect,
    min: i32,
) -> (i32, i32) {
    let Some(&(axis, current)) = positions.get(id) else { return (0, 0) };
    let Ok(pushed) = pushed_guides(layout, id, locked) else { return (current, current) };
    let pos = |id: &str| positions.get(id).map(|&(_, v)| v);
    // Limits on how far (delta) the pushed guides move.
    let (start, end) = span(area, axis);
    let (mut lo, mut hi) = (i32::MIN, i32::MAX);
    for p in pushed.iter().filter_map(|g| pos(g)) {
        lo = lo.max(start - p);
        hi = hi.min(end - p);
    }
    for z in &layout.zones {
        let (near, far) = sides(z, axis);
        let (near_moves, far_moves) = (pushed.contains(near), pushed.contains(far));
        if near_moves == far_moves || follows_gaming(layout, if near_moves { far } else { near }) {
            continue;
        }
        let (Some(a), Some(b)) = (pos(near), pos(far)) else { continue };
        // A zone already under `min` may grow but not shrink.
        if near_moves {
            hi = hi.min((b - a - min).max(0));
        } else {
            lo = lo.max((min - (b - a)).min(0));
        }
    }
    if lo > hi {
        (current, current)
    } else {
        (current.saturating_add(lo), current.saturating_add(hi))
    }
}

/// The first zone in `locked` whose size differs between two resolutions of a layout
/// (zones may move; only their size counts).
pub fn resized_locked(before: &[ResolvedZone], after: &[ResolvedZone], locked: &[usize]) -> Option<usize> {
    let size = |z: &ResolvedZone| (z.rect.width(), z.rect.height());
    locked.iter().copied().find(|&i| match (before.get(i), after.get(i)) {
        (Some(b), Some(a)) => size(b) != size(a),
        _ => false,
    })
}

/// Makes zone `index` `size` px wide ([`Axis::Vertical`]: moves a vertical edge) or tall, by
/// moving its right/bottom edge or, if that can't move, its left/top edge (with
/// [`move_divider`], so only the zones beside that edge change and locked zones are pushed).
/// Only the layout's own guides move (not the area's or the gaming zone's edges), and no
/// zone may be squeezed to nothing (except beside the gaming zone, which they follow).
pub fn resize_zone(
    layout: &mut Layout,
    index: usize,
    axis: Axis,
    size: i32,
    locked: &[usize],
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    let zone = layout.zones.get(index).ok_or(EditError::NoSuchZone(index))?.clone();
    if locked.contains(&index) {
        return Err(EditError::Locked(index));
    }
    let before = layout.resolve(area, monitor, 0)?;
    let r = before[index].rect;
    let (near, far, near_pos, far_pos) = match axis {
        Axis::Vertical => (&zone.left, &zone.right, r.left, r.right),
        Axis::Horizontal => (&zone.top, &zone.bottom, r.top, r.bottom),
    };
    if far_pos - near_pos == size {
        return Ok(());
    }
    let mut error = EditError::CantResize;
    for (guide, pos) in [(far, near_pos + size), (near, far_pos - size)] {
        if !layout.guides.iter().any(|g| &g.id == guide) {
            continue;
        }
        let mut trial = layout.clone();
        match move_divider(&mut trial, guide, pos, locked, area, monitor) {
            Ok(()) => {}
            Err(e @ EditError::Locked(_)) => {
                error = e;
                continue;
            }
            Err(_) => continue,
        }
        let Ok(after) = trial.resolve(area, monitor, 0) else { continue };
        let got = match axis {
            Axis::Vertical => after[index].rect.width(),
            Axis::Horizontal => after[index].rect.height(),
        };
        let squeezed = |i: usize| {
            let z = &trial.zones[i];
            !before[i].rect.is_empty()
                && after[i].rect.is_empty()
                && ![&z.left, &z.right, &z.top, &z.bottom].iter().any(|g| follows_gaming(&trial, g))
        };
        if got != size || (0..trial.zones.len()).any(squeezed) {
            continue;
        }
        if let Some(i) = resized_locked(&before, &after, locked) {
            error = EditError::Locked(i);
            continue;
        }
        *layout = trial;
        return Ok(());
    }
    Err(error)
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
    let mut used: HashSet<String> = layout
        .zones
        .iter()
        .chain(layout.gaming_zones.iter().filter_map(|g| g.within.as_ref()))
        .flat_map(|z| [&z.left, &z.right, &z.top, &z.bottom])
        .cloned()
        .collect();
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

/// Adds a gaming zone (centred in the whole area) if the layout has none. Existing zones
/// are kept; see [`arrange_around_gaming_zone`].
pub fn add_gaming_zone(layout: &mut Layout, size: GamingSize) {
    if layout.gaming_zones.is_empty() {
        layout.gaming_zones.push(GamingZone::new(size, Position::default(), Position::default()));
    }
}

/// Replaces the zones with four around the gaming zone: a full-height column either side
/// and a strip above and below it. The gaming zone's space becomes the whole area.
pub fn arrange_around_gaming_zone(layout: &mut Layout) -> Result<(), EditError> {
    let Some(g) = layout.gaming_zones.first_mut() else {
        return Err(EditError::NoGamingZone);
    };
    g.within = None;
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

/// Turns zone `index` into the gaming zone: the zone's space becomes its space, and it fills
/// it exactly (its size is the zone's size here, centred). The zones around are untouched.
///
/// Four zones take the zone's place and partition whatever the gaming zone doesn't cover: a
/// strip above it (the space's full width), one left and right of it, and one below. They're
/// empty (and have no number) until the gaming zone is made smaller than its space.
pub fn make_gaming_zone(layout: &mut Layout, index: usize, area: Rect, monitor: Rect) -> Result<(), EditError> {
    if !layout.gaming_zones.is_empty() {
        return Err(EditError::AlreadyHasGamingZone);
    }
    let zone = layout.zones.get(index).ok_or(EditError::NoSuchZone(index))?.clone();
    let r = layout.resolve(area, monitor, 0)?[index].rect;
    let size = GamingSize::Pixels { w: r.width().max(1) as u32, h: r.height().max(1) as u32 };
    layout.gaming_zones.push(GamingZone {
        size,
        x: Position::Align(Align::Centre),
        y: Position::Align(Align::Centre),
        within: Some(zone.clone()),
    });
    let gz = |edge: &str| gaming_guide(0, edge);
    let make =
        |left: &String, right: String, top: String, bottom: String| Zone { left: left.clone(), right, top, bottom };
    layout.zones.splice(
        index..=index,
        [
            make(&zone.left, zone.right.clone(), zone.top.clone(), gz("top")),
            make(&zone.left, gz("left"), gz("top"), gz("bottom")),
            make(&gz("right"), zone.right.clone(), gz("top"), gz("bottom")),
            make(&zone.left, zone.right.clone(), gz("bottom"), zone.bottom.clone()),
        ],
    );
    Ok(())
}

/// Turns the gaming zone(s) back into normal zones.
///
/// - One with a space of its own ([`make_gaming_zone`]): the zones inside that space (the
///   bits it didn't cover, however they were split) are replaced by one zone filling it.
/// - One in the whole area: its edges become percentage guides at the same place and a
///   zone fills it, so on this monitor nothing moves.
pub fn remove_gaming_zone(layout: &mut Layout, area: Rect, monitor: Rect) -> Result<(), EditError> {
    if layout.gaming_zones.is_empty() {
        return Err(EditError::NoGamingZone);
    }
    let positions = layout.guide_positions(area, monitor)?;
    let resolved = layout.resolve(area, monitor, 0)?;
    let mut spaces = Vec::new();
    for i in 0..layout.gaming_zones.len() {
        if let Some(z) = layout.gaming_zones[i].within.clone() {
            spaces.push((z, layout.gaming_space(i, area, monitor)?.0, false));
        }
    }
    let inside = |r: &Rect, s: &Rect| {
        r.left.min(r.right) >= s.left
            && r.left.max(r.right) <= s.right
            && r.top.min(r.bottom) >= s.top
            && r.top.max(r.bottom) <= s.bottom
    };
    let mut zones = Vec::new();
    for (z, r) in layout.zones.iter().zip(&resolved) {
        match spaces.iter_mut().find(|(_, s, _)| inside(&r.rect, s)) {
            Some((space, _, placed)) => {
                if !*placed {
                    zones.push(space.clone());
                    *placed = true;
                }
            }
            None => zones.push(z.clone()),
        }
    }
    zones.extend(spaces.into_iter().filter(|(_, _, placed)| !placed).map(|(z, _, _)| z));
    layout.zones = zones;

    // Anything else that used the gaming zone's edges gets guides at the same place.
    for i in 0..layout.gaming_zones.len() {
        let mut edges = Vec::new();
        for edge in ["left", "right", "top", "bottom"] {
            let old = gaming_guide(i, edge);
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
            edges.push(id);
        }
        if layout.gaming_zones[i].within.is_none() {
            let [left, right, top, bottom] = <[String; 4]>::try_from(edges).expect("four edges");
            layout.zones.push(Zone { left, right, top, bottom });
        }
    }
    layout.gaming_zones.clear();
    layout.gaming_over_taskbar = false;
    remove_unused_guides(layout);
    Ok(())
}

fn rename_references(layout: &mut Layout, old: &str, new: &str) {
    let fix = |s: &mut String| {
        if s == old {
            *s = new.to_string();
        }
    };
    for z in layout.zones.iter_mut().chain(layout.gaming_zones.iter_mut().filter_map(|g| g.within.as_mut())) {
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
    fn moving_a_divider_leaves_the_others() {
        let mut l = LayoutsFile::defaults().get("split-focus").unwrap().clone();
        move_divider(&mut l, "left-col", 1500, &[], WORK, MONITOR).unwrap();
        let r = rects(&l);
        // centre-split (halfway between the columns) stays at 3072; only zones beside left-col change.
        assert_eq!(r[3], Rect::new(1500, 375, 3072, 2125));
        assert_eq!(r[4], Rect::new(3072, 375, 4854, 2125));
        assert_eq!(r[0], Rect::new(0, 0, 1500, 1250));
        assert!(matches!(l.guides[2].anchor, Anchor::Between { .. }));
    }

    #[test]
    fn limits_and_dividers() {
        let l = LayoutsFile::defaults().get("split-focus").unwrap().clone();
        let positions = l.guide_positions(WORK, MONITOR).unwrap();
        // left-col (1290) is bounded by area.left on one side and centre-split (3072) on the other.
        assert_eq!(guide_limits(&l, &positions, "left-col", &[], WORK, 50), (50, 3022));
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

    /// Three columns made by splitting: 0..2000 | 2000..4000 | 4000..6144 (guides g1, g2).
    fn three_columns() -> Layout {
        let mut l = new_layout("t", "T");
        split(&mut l, 0, Axis::Vertical, 2000, WORK, MONITOR).unwrap();
        split(&mut l, 1, Axis::Vertical, 4000, WORK, MONITOR).unwrap();
        l
    }

    fn widths(l: &Layout) -> Vec<i32> {
        rects(l)[..l.zones.len()].iter().map(|r| r.width()).collect()
    }

    #[test]
    fn locked_zones_are_pushed() {
        // Tom: moving a divider next to a locked zone pushes it into the next unlocked zone.
        let mut l = three_columns();
        move_divider(&mut l, "g1", 2500, &[1], WORK, MONITOR).unwrap();
        assert_eq!(widths(&l), [2500, 2000, 1644]);
        move_divider(&mut l, "g1", 1000, &[1], WORK, MONITOR).unwrap();
        assert_eq!(widths(&l), [1000, 2000, 3144]);
        // Limits: zone 1 can shrink to 10 px, zone 3 to 10 px (the locked zone stays 2000).
        let positions = l.guide_positions(WORK, MONITOR).unwrap();
        assert_eq!(guide_limits(&l, &positions, "g1", &[1], WORK, 10), (10, 4134));
        // Typing zone 1's width pushes the locked zone too.
        resize_zone(&mut l, 0, Axis::Vertical, 2028, &[1], WORK, MONITOR).unwrap();
        assert_eq!(widths(&l), [2028, 2000, 2116]);
        // Two locked zones in a row move together.
        split(&mut l, 2, Axis::Vertical, 5000, WORK, MONITOR).unwrap();
        move_divider(&mut l, "g1", 1828, &[1, 2], WORK, MONITOR).unwrap();
        assert_eq!(widths(&l), [1828, 2000, 972, 1344]);
        // A locked zone against the screen's edge can't be pushed.
        assert_eq!(move_divider(&mut l, "g1", 1900, &[0], WORK, MONITOR), Err(EditError::Locked(0)));
        let positions = l.guide_positions(WORK, MONITOR).unwrap();
        assert_eq!(guide_limits(&l, &positions, "g1", &[0], WORK, 10), (1828, 1828));
        // A locked zone's own size can't be typed.
        assert_eq!(resize_zone(&mut l, 1, Axis::Vertical, 100, &[1], WORK, MONITOR), Err(EditError::Locked(1)));
    }

    #[test]
    fn making_a_zone_the_gaming_zone() {
        // Tom's Slay test: the middle column (1597..4547) becomes the gaming zone, all of it.
        let slay = LayoutsFile::defaults().get("slay").unwrap().clone();
        let mut l = slay.clone();
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        assert_eq!(l.gaming_zones[0].size, GamingSize::Pixels { w: 2950, h: 2500 });
        assert_eq!(l.gaming_zones[0].within.as_ref(), Some(&slay.zones[1]));
        let zones = l.resolve(WORK, MONITOR, 300).unwrap();
        let r: Vec<Rect> = zones.iter().map(|z| z.rect).collect();
        assert_eq!(r[0], Rect::new(0, 0, 1597, 2500)); // the columns either side are untouched
        assert_eq!(r[5], Rect::new(4547, 0, 6144, 2500));
        assert_eq!(r[6], Rect::new(1597, 0, 4547, 2500)); // the gaming zone fills the column
                                                          // The four zones in its place are empty, so they have no number.
        assert!(r[1..5].iter().all(|r| r.is_empty()), "{r:?}");
        let labels: Vec<String> = (0..zones.len()).map(|i| crate::layout::zone_label(&zones, i)).collect();
        assert_eq!(labels, ["1", "-", "-", "-", "-", "2", "G"]);

        // Made smaller, the bits it doesn't cover become zones.
        l.gaming_zones[0].size = GamingSize::Pixels { w: 1920, h: 1080 };
        let zones = l.resolve(WORK, MONITOR, 0).unwrap();
        let r: Vec<Rect> = zones.iter().map(|z| z.rect).collect();
        assert_eq!(
            r,
            [
                Rect::new(0, 0, 1597, 2500),
                Rect::new(1597, 0, 4547, 710),     // above
                Rect::new(1597, 710, 2112, 1790),  // left
                Rect::new(4032, 710, 4547, 1790),  // right
                Rect::new(1597, 1790, 4547, 2500), // below
                Rect::new(4547, 0, 6144, 2500),
                Rect::new(2112, 710, 4032, 1790), // the gaming zone, centred in the column
            ]
        );
        let labels: Vec<String> = (0..zones.len()).map(|i| crate::layout::zone_label(&zones, i)).collect();
        assert_eq!(labels, ["1", "2", "3", "4", "5", "6", "G"]);
        // Placed by pixels, it's relative to its space (the column).
        l.gaming_zones[0].x = Position::Px(0);
        assert_eq!(rects(&l)[6].left, 1597);
        assert_eq!(rects(&l)[2].width(), 0);

        // Turning it back gives the original layout.
        remove_gaming_zone(&mut l, WORK, MONITOR).unwrap();
        assert_eq!(l, slay);
        assert_eq!(make_gaming_zone(&mut l, 9, WORK, MONITOR), Err(EditError::NoSuchZone(9)));
    }

    #[test]
    fn gaming_zone_space_follows_its_dividers() {
        let mut l = three_columns();
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        l.gaming_zones[0].size = GamingSize::Pixels { w: 1600, h: 900 };
        // Dragging the space's left divider: the gaming zone stays centred in it, and the
        // strips beside it don't stop the drag.
        let positions = l.guide_positions(WORK, MONITOR).unwrap();
        assert_eq!(guide_limits(&l, &positions, "g1", &[], WORK, 10), (10, 3990));
        move_divider(&mut l, "g1", 1800, &[], WORK, MONITOR).unwrap();
        assert_eq!(rects(&l).last(), Some(&Rect::new(2100, 800, 3700, 1700)));
        // A space smaller than the gaming zone shrinks it.
        move_divider(&mut l, "g1", 3000, &[], WORK, MONITOR).unwrap();
        assert_eq!(rects(&l).last(), Some(&Rect::new(3000, 800, 4000, 1700)));
        // Splitting a leftover strip still turns back into the one zone.
        let mut l = three_columns();
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        l.gaming_zones[0].size = GamingSize::Pixels { w: 1600, h: 900 };
        split(&mut l, 1, Axis::Vertical, 3000, WORK, MONITOR).unwrap(); // the strip above
        remove_gaming_zone(&mut l, WORK, MONITOR).unwrap();
        assert_eq!(l, three_columns());
    }

    #[test]
    fn gaming_zone_in_a_single_zone_layout() {
        let mut l = new_layout("t", "T");
        make_gaming_zone(&mut l, 0, WORK, MONITOR).unwrap();
        assert!(l.validate().is_ok());
        l.gaming_zones[0].size = GamingSize::Pixels { w: 3840, h: 2160 };
        let mut b = new_layout("t", "T");
        add_gaming_zone(&mut b, GamingSize::Pixels { w: 3840, h: 2160 });
        assert_eq!(rects(&l)[4], rects(&b)[1]);
        assert_eq!(rects(&l)[0], Rect::new(0, 0, 6144, 170));
        // Over the taskbar: its space reaches the area's bottom, so it may go to the monitor's.
        l.gaming_over_taskbar = true;
        l.gaming_zones[0].size = GamingSize::Pixels { w: 4552, h: 2560 };
        assert_eq!(rects(&l)[4], Rect::new(796, 0, 5348, 2560));
    }

    #[test]
    fn resizing_zones_with_locks() {
        // Three columns made by splitting; aim for 33 % / 34 % / 33 %.
        let mut l = new_layout("t", "T");
        split(&mut l, 0, Axis::Vertical, 2000, WORK, MONITOR).unwrap();
        split(&mut l, 1, Axis::Vertical, 4000, WORK, MONITOR).unwrap();
        let third = (WORK.width() as f64 * 0.33).round() as i32; // 2028
        resize_zone(&mut l, 0, Axis::Vertical, third, &[], WORK, MONITOR).unwrap();
        let r = rects(&l);
        // Only zones 1 and 2 changed, although the second split was proportional to the first.
        assert_eq!((r[0].width(), r[1].width(), r[2].width()), (2028, 1972, 2144));
        // Zone 3 with zone 1 locked: its right edge is the area's, so its left edge moves.
        resize_zone(&mut l, 2, Axis::Vertical, third, &[0], WORK, MONITOR).unwrap();
        let r = rects(&l);
        assert_eq!((r[0].width(), r[1].width(), r[2].width()), (2028, 2088, 2028));
        // Zone 2 wider, with zone 3 locked: its left edge moves instead, taking from zone 1.
        resize_zone(&mut l, 1, Axis::Vertical, 2188, &[2], WORK, MONITOR).unwrap();
        let r = rects(&l);
        assert_eq!((r[0].width(), r[1].width(), r[2].width()), (1928, 2188, 2028));
        // Both neighbours locked: refused, nothing changes.
        let before = l.clone();
        assert_eq!(resize_zone(&mut l, 1, Axis::Vertical, 2000, &[0, 2], WORK, MONITOR), Err(EditError::Locked(0)));
        assert_eq!(l, before);
        // The area's own edges can't move.
        let mut one = new_layout("t", "T");
        assert_eq!(resize_zone(&mut one, 0, Axis::Vertical, 100, &[], WORK, MONITOR), Err(EditError::CantResize));
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
        // Removing it leaves the other zones exactly where they were, and a normal zone in its place.
        remove_gaming_zone(&mut l, WORK, MONITOR).unwrap();
        assert!(l.gaming_zones.is_empty());
        assert_eq!(rects(&l), with);
        assert!(l.validate().is_ok());
        assert_eq!(remove_gaming_zone(&mut l, WORK, MONITOR), Err(EditError::NoGamingZone));
    }
}
