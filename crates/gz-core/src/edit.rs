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
    /// A merge of zones that move with the gaming zone and zones that don't.
    MixesGaming,
    /// The change would leave zones that come apart when something moves.
    ComesApart,
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
            Self::MixesGaming => write!(
                f,
                "those zones can't be merged: some move with the gaming zone and some don't. To give a zone beside \
                 the gaming zone to its neighbour, drag the divider between them up to the gaming zone"
            ),
            Self::ComesApart => write!(
                f,
                "that would leave zones that only meet by chance, and would overlap or leave a gap when a \
                 divider or the gaming zone moves"
            ),
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

/// Smallest size (px) a divider drag or a typed size may leave a zone. The zones in the
/// gaming zone's leftover space may shrink to nothing.
pub const MIN_ZONE_PX: i32 = 10;

/// Moves divider `id` towards `pos` the way the editor does, as far as it can go (no zone
/// smaller than [`MIN_ZONE_PX`]):
///
/// - Every other guide stays where it is on this monitor (a split that depended on `id` gets
///   a new fraction instead of moving with it), so only the zones on either side change.
/// - A zone in `locked` keeps its size: it's pushed along, and the next unlocked zone gives
///   or takes the space.
/// - The gaming zone keeps its size too. A divider that follows it (the edge of its
///   leftover space) first shrinks the leftover zone between them to nothing, then pushes
///   the gaming zone along, and everything that follows it.
pub fn move_divider(
    layout: &mut Layout,
    id: &str,
    pos: i32,
    locked: &[usize],
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    let axis = layout.guides.iter().find(|g| g.id == id).ok_or_else(|| EditError::NoSuchGuide(id.into()))?.axis;
    let delta = pos - position(&layout.guide_positions(area, monitor)?, id)?;
    if delta == 0 {
        return Ok(());
    }
    let alone = Group { guides: vec![id.to_string()], gaming: false };
    if !follows_gaming(layout, id) {
        let group = grow(layout, alone, axis, locked, true)?;
        return shift_within_limits(layout, &group, axis, delta, area, monitor);
    }
    let first = {
        let group = grow(layout, alone, axis, locked, false)?;
        let (lo, hi) = group_limits(layout, &group, axis, area, monitor, MIN_ZONE_PX)?;
        let first = delta.clamp(lo, hi);
        shift(layout, &group, axis, first, area, monitor)?;
        first
    };
    // Pushing on into the gaming zone moves it, if the divider is on its outside.
    let rest = delta - first;
    let at = position(&layout.guide_positions(area, monitor)?, id)?;
    let Some(g) = layout.gaming_rects(area, monitor)?.first().copied() else { return Ok(()) };
    let (start, end) = span(g, axis);
    if rest == 0 || !((rest > 0 && at <= start) || (rest < 0 && at >= end)) {
        return Ok(());
    }
    let group = grow(layout, Group { guides: vec![id.to_string()], gaming: true }, axis, locked, true)?;
    shift_within_limits(layout, &group, axis, rest, area, monitor)
}

/// Moves the gaming zone by (`dx`, `dy`), as far as it can go. Everything that follows it
/// moves with it (its leftover zones and the dividers around them), so the zones beside it
/// get bigger or smaller; locked zones are pushed along.
pub fn move_gaming_zone(
    layout: &mut Layout,
    dx: i32,
    dy: i32,
    locked: &[usize],
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    if layout.gaming_zones.is_empty() {
        return Err(EditError::NoGamingZone);
    }
    for (axis, delta) in [(Axis::Vertical, dx), (Axis::Horizontal, dy)] {
        if delta != 0 {
            let group = grow(layout, Group { guides: Vec::new(), gaming: true }, axis, locked, true)?;
            shift_within_limits(layout, &group, axis, delta, area, monitor)?;
        }
    }
    Ok(())
}

/// Things that move together along one axis: some guides, and perhaps the gaming zone, which
/// takes everything that follows it along.
#[derive(Clone, Debug)]
struct Group {
    guides: Vec<String>,
    gaming: bool,
}

impl Group {
    fn moves(&self, layout: &Layout, side: &str) -> bool {
        self.guides.iter().any(|g| g == side)
            || (self.gaming && (parse_gaming_guide(side).is_some() || follows_gaming(layout, side)))
    }
}

/// Adds what the group pushes along `axis`: a locked zone keeps its size, so when one of its
/// sides moves the other does too (and so on). A locked zone that would have to push the
/// area's edge can't move: [`EditError::Locked`]. With `whole_gaming`, a guide that follows
/// the gaming zone brings the gaming zone along.
fn grow(
    layout: &Layout,
    mut group: Group,
    axis: Axis,
    locked: &[usize],
    whole_gaming: bool,
) -> Result<Group, EditError> {
    loop {
        let mut more = false;
        if whole_gaming && !group.gaming && group.guides.iter().any(|g| follows_gaming(layout, g)) {
            group.gaming = true;
            more = true;
        }
        for &i in locked {
            let Some(z) = layout.zones.get(i) else { continue };
            let (near, far) = sides(z, axis);
            let other = match (group.moves(layout, near), group.moves(layout, far)) {
                (true, false) => far,
                (false, true) => near,
                _ => continue,
            };
            if parse_gaming_guide(other).is_some() {
                group.gaming = true;
            } else if layout.guides.iter().any(|g| &g.id == other) {
                group.guides.push(other.clone());
            } else {
                return Err(EditError::Locked(i));
            }
            more = true;
        }
        if !more {
            return Ok(group);
        }
    }
}

/// How far (as a delta) the group can move along `axis`: its guides stay in the area, the
/// gaming zone in its bounds, and every zone that gets smaller keeps `min` px (a zone already
/// smaller may grow but not shrink). The zones between a guide and the gaming zone it
/// follows may shrink to nothing.
fn group_limits(
    layout: &Layout,
    group: &Group,
    axis: Axis,
    area: Rect,
    monitor: Rect,
    min: i32,
) -> Result<(i32, i32), EditError> {
    let positions = layout.guide_positions(area, monitor)?;
    let pos = |id: &str| positions.get(id).map(|&(_, v)| v);
    let (start, end) = span(area, axis);
    let (mut lo, mut hi) = (i32::MIN, i32::MAX);
    for g in layout.guides.iter().filter(|g| g.axis == axis && group.moves(layout, &g.id)) {
        let p = pos(&g.id).unwrap_or(start);
        lo = lo.max(start - p);
        hi = hi.min(end - p);
    }
    if group.gaming {
        let (bs, be) = span(layout.gaming_bounds(area, monitor), axis);
        for g in layout.gaming_rects(area, monitor)? {
            let (gs, ge) = span(g, axis);
            lo = lo.max(bs - gs);
            hi = hi.min(be - ge);
        }
    }
    for z in &layout.zones {
        let (near, far) = sides(z, axis);
        let (near_moves, far_moves) = (group.moves(layout, near), group.moves(layout, far));
        if near_moves == far_moves {
            continue;
        }
        let other = if near_moves { far } else { near };
        // The gaming zone's leftover zones (beside its edges) may shrink to nothing.
        let leftover = parse_gaming_guide(near).is_some()
            || parse_gaming_guide(far).is_some()
            || (!group.gaming && follows_gaming(layout, other));
        let min = if leftover { 0 } else { min };
        let (Some(a), Some(b)) = (pos(near), pos(far)) else { continue };
        if near_moves {
            hi = hi.min((b - a - min).max(0));
        } else {
            lo = lo.max((min - (b - a)).min(0));
        }
    }
    Ok(if lo > hi { (0, 0) } else { (lo, hi) })
}

fn shift_within_limits(
    layout: &mut Layout,
    group: &Group,
    axis: Axis,
    delta: i32,
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    let (lo, hi) = group_limits(layout, group, axis, area, monitor, MIN_ZONE_PX)?;
    shift(layout, group, axis, delta.clamp(lo, hi), area, monitor)
}

/// Moves the group by `delta` along `axis`; every other guide stays where it is on this monitor.
fn shift(
    layout: &mut Layout,
    group: &Group,
    axis: Axis,
    delta: i32,
    area: Rect,
    monitor: Rect,
) -> Result<(), EditError> {
    if delta == 0 {
        return Ok(());
    }
    let before = layout.guide_positions(area, monitor)?;
    if group.gaming {
        let rects = layout.gaming_rects(area, monitor)?;
        for (g, r) in layout.gaming_zones.iter_mut().zip(rects) {
            match axis {
                Axis::Vertical => g.x = Position::Px(r.left + delta - area.left),
                Axis::Horizontal => g.y = Position::Px(r.top + delta - area.top),
            }
        }
    }
    let targets: Vec<(String, i32)> = layout
        .guides
        .iter()
        .map(|g| {
            let at = before[&g.id].1;
            let moves = g.axis == axis && group.moves(layout, &g.id);
            (g.id.clone(), if moves { at + delta } else { at })
        })
        .collect();
    place_guides(layout, &targets, area, monitor)
}

/// Moves each guide to its target position (keeping its kind of anchor). Moving a guide can
/// move others that depend on it, so this goes round until all are in place.
fn place_guides(layout: &mut Layout, targets: &[(String, i32)], area: Rect, monitor: Rect) -> Result<(), EditError> {
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

/// How far divider `id` can be dragged without pushing the gaming zone (see [`move_divider`]),
/// keeping zones at least `min` px: (lowest, highest) position. (current, current) if it
/// can't move.
pub fn guide_limits(
    layout: &Layout,
    id: &str,
    locked: &[usize],
    area: Rect,
    monitor: Rect,
    min: i32,
) -> Result<(i32, i32), EditError> {
    let guide = layout.guides.iter().find(|g| g.id == id).ok_or_else(|| EditError::NoSuchGuide(id.into()))?;
    let current = position(&layout.guide_positions(area, monitor)?, id)?;
    let whole = !follows_gaming(layout, id);
    let (lo, hi) = match grow(layout, Group { guides: vec![id.into()], gaming: false }, guide.axis, locked, whole) {
        Ok(group) => group_limits(layout, &group, guide.axis, area, monitor, min)?,
        Err(_) => (0, 0),
    };
    Ok((current + lo, current + hi))
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
    let within = |r: &Rect| {
        bounds.left <= r.left.min(r.right)
            && r.left.max(r.right) <= bounds.right
            && bounds.top <= r.top.min(r.bottom)
            && r.top.max(r.bottom) <= bounds.bottom
    };
    // Empty zones never give the merged zone its edges.
    let inside: Vec<usize> =
        (0..count).filter(|&i| !resolved[i].rect.is_empty() && within(&resolved[i].rect)).collect();
    let empty_inside: Vec<usize> =
        (0..count).filter(|&i| resolved[i].rect.is_empty() && within(&resolved[i].rect)).collect();
    let rects: Vec<Rect> = inside.iter().map(|&i| resolved[i].rect).collect();
    let area_of = |r: &Rect| r.width() as i64 * r.height() as i64;
    let overlap =
        |a: &Rect, b: &Rect| a.left.max(b.left) < a.right.min(b.right) && a.top.max(b.top) < a.bottom.min(b.bottom);
    let gaming_in_the_way = resolved[count..].iter().any(|g| overlap(&g.rect, &bounds));
    let tiles = rects.iter().map(area_of).sum::<i64>() == area_of(&bounds)
        && rects.iter().enumerate().all(|(n, a)| rects[n + 1..].iter().all(|b| !overlap(a, b)));
    if gaming_in_the_way || !tiles {
        return Err(EditError::NotARectangle);
    }
    // Each side must move one way: with the gaming zone, or not. (Zones whose sides only meet
    // because the gaming zone is against the screen's edge would come apart when it moves.)
    let side_guides = |on_edge: &dyn Fn(&Rect) -> bool, side: &dyn Fn(&Zone) -> &String| -> Vec<String> {
        inside.iter().filter(|&&i| on_edge(&resolved[i].rect)).map(|&i| side(&layout.zones[i]).clone()).collect()
    };
    for ids in [
        side_guides(&|r| r.left == bounds.left, &|z| &z.left),
        side_guides(&|r| r.right == bounds.right, &|z| &z.right),
        side_guides(&|r| r.top == bounds.top, &|z| &z.top),
        side_guides(&|r| r.bottom == bounds.bottom, &|z| &z.bottom),
    ] {
        let follows = |g: &String| follows_gaming(layout, g);
        if ids.iter().any(follows) && !ids.iter().all(follows) {
            return Err(EditError::MixesGaming);
        }
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
    // Empty zones on its edges may belong inside it (the strip beside a gaming zone that fills
    // its space, inside the merge) or not: keep them unless that leaves the layout coming apart
    // when something moves.
    for absorb in [false, true] {
        let mut gone = inside.clone();
        if absorb {
            gone.extend(&empty_inside);
            gone.sort_unstable();
        }
        let mut trial = layout.clone();
        let first = gone[0];
        for &i in gone.iter().rev() {
            trial.zones.remove(i);
        }
        trial.zones.insert(first, merged.clone());
        remove_unused_guides(&mut trial);
        if stays_whole(&trial, area, monitor) {
            *layout = trial;
            return Ok(first);
        }
    }
    Err(EditError::ComesApart)
}

/// Joins guides that only meet by chance: where one zone's side and a neighbour's side are
/// different guides on the same line (they'd come apart when one is dragged), the neighbour
/// uses the first zone's guide from then on. Guides that move differently (one with the
/// gaming zone, one not) are left alone. Nothing moves on this monitor. Returns whether
/// anything changed.
pub fn join_coincident(layout: &mut Layout, area: Rect, monitor: Rect) -> bool {
    let mut changed = false;
    // Each join can make another visible, so repeat (each one removes a guide).
    for _ in 0..=layout.guides.len() {
        let (Ok(zones), Ok(positions)) = (layout.resolve(area, monitor, 0), layout.guide_positions(area, monitor))
        else {
            return changed;
        };
        let user = |id: &String| layout.guides.iter().any(|g| &g.id == id);
        let mut join = None;
        'search: for (a, za) in layout.zones.iter().enumerate() {
            for (b, zb) in layout.zones.iter().enumerate() {
                let (ra, rb) = (zones[a].rect, zones[b].rect);
                if ra.is_empty() || rb.is_empty() {
                    continue;
                }
                for (keep, drop, meet) in [
                    (&za.right, &zb.left, ra.right == rb.left && ra.top.max(rb.top) < ra.bottom.min(rb.bottom)),
                    (&za.bottom, &zb.top, ra.bottom == rb.top && ra.left.max(rb.left) < ra.right.min(rb.right)),
                ] {
                    if meet
                        && keep != drop
                        && user(drop)
                        && positions.get(keep) == positions.get(drop)
                        && follows_gaming(layout, keep) == follows_gaming(layout, drop)
                    {
                        join = Some((keep.clone(), drop.clone()));
                        break 'search;
                    }
                }
            }
        }
        let Some((keep, drop)) = join else { return changed };
        // `keep` mustn't depend on `drop` (that would be a loop once renamed).
        let mut l = layout.clone();
        rename_references(&mut l, &drop, &keep);
        l.guides.retain(|g| g.id != drop);
        if l.validate().is_err()
            || l.guide_positions(area, monitor).ok().as_ref().map(|p| p.get(&keep)) != Some(positions.get(&keep))
        {
            return changed;
        }
        *layout = l;
        changed = true;
    }
    changed
}

/// How much the zones (gaming zones included, clipped to the area) overlap each other, and
/// how much of the area they cover.
fn coverage(layout: &Layout, area: Rect, monitor: Rect) -> Option<(i64, i64)> {
    let rects: Vec<Rect> = layout
        .resolve(area, monitor, 0)
        .ok()?
        .iter()
        .filter(|z| !z.rect.is_empty())
        .map(|z| z.rect.intersect(&area))
        .filter(|r| !r.is_empty())
        .collect();
    let size = |r: &Rect| r.width() as i64 * r.height() as i64;
    let mut overlap = 0;
    for (n, a) in rects.iter().enumerate() {
        for b in &rects[n + 1..] {
            let (w, h) = (a.right.min(b.right) - a.left.max(b.left), a.bottom.min(b.bottom) - a.top.max(b.top));
            if w > 0 && h > 0 {
                overlap += w as i64 * h as i64;
            }
        }
    }
    Some((overlap, rects.iter().map(size).sum::<i64>() - overlap))
}

/// Whether the layout stays in one piece when any divider or the gaming zone is moved a
/// little (the way the editor moves them): no new overlaps and, if the zones covered the
/// whole area, no gaps. Two zones that meet only by coincidence (on two guides that don't
/// move together) fail this.
pub fn stays_whole(layout: &Layout, area: Rect, monitor: Rect) -> bool {
    let Some((overlap, covered)) = coverage(layout, area, monitor) else { return false };
    let full = covered == area.width() as i64 * area.height() as i64;
    let still_whole = |edit: &dyn Fn(&mut Layout) -> Result<(), EditError>| {
        let mut l = layout.clone();
        if edit(&mut l).is_err() {
            return true;
        }
        coverage(&l, area, monitor).is_some_and(|(o, c)| o <= overlap && (!full || c == covered))
    };
    let Ok(positions) = layout.guide_positions(area, monitor) else { return false };
    for g in &layout.guides {
        let at = positions[&g.id].1;
        for d in [-5, 5] {
            if !still_whole(&|l| move_divider(l, &g.id, at + d, &[], area, monitor)) {
                return false;
            }
        }
    }
    if !layout.gaming_zones.is_empty() {
        for (dx, dy) in [(-5, 0), (5, 0), (0, -5), (0, 5)] {
            if !still_whole(&|l| move_gaming_zone(l, dx, dy, &[], area, monitor)) {
                return false;
            }
        }
    }
    true
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
        layout.gaming_zones.push(GamingZone::new(size, Position::default(), Position::default()));
    }
}

/// Replaces the zones with four around the gaming zone: a full-height column either side
/// and a strip above and below it.
pub fn arrange_around_gaming_zone(layout: &mut Layout) -> Result<(), EditError> {
    let Some(g) = layout.gaming_zones.first_mut() else {
        return Err(EditError::NoGamingZone);
    };
    g.from_zone = false;
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

/// The edge `side` ("left", …) of gaming zone 0 a guide follows directly: its anchor is an
/// offset from that edge.
fn edge_followed(g: &Guide) -> Option<&str> {
    match &g.anchor {
        Anchor::Offset { from, .. } => parse_gaming_guide(from).map(|(_, side)| side),
        _ => None,
    }
}

/// Turns zone `index` into the gaming zone, exactly the zone's size and place.
///
/// - The dividers around the zone now follow the gaming zone (they're kept at a distance
///   from its edges, 0 to start with), so moving it moves them, and the zones beside it.
/// - Four zones take the zone's place and partition whatever the gaming zone doesn't cover:
///   a strip above it, one left and right of it, and one below. They're empty (and have no
///   number) until it's made smaller.
pub fn make_gaming_zone(layout: &mut Layout, index: usize, area: Rect, monitor: Rect) -> Result<(), EditError> {
    if !layout.gaming_zones.is_empty() {
        return Err(EditError::AlreadyHasGamingZone);
    }
    let zone = layout.zones.get(index).ok_or(EditError::NoSuchZone(index))?.clone();
    let r = layout.resolve(area, monitor, 0)?[index].rect;
    if r.is_empty() {
        return Err(EditError::NoSuchZone(index));
    }
    let size = GamingSize::Pixels { w: r.width() as u32, h: r.height() as u32 };
    // Written as "centre" when that's where it is.
    let place = |at: i32, len: i32, start: i32, area_len: i32| {
        if at == start + (area_len - len).div_euclid(2) {
            Position::Align(Align::Centre)
        } else {
            Position::Px(at - start)
        }
    };
    layout.gaming_zones.push(GamingZone {
        size,
        x: place(r.left, r.width(), area.left, area.width()),
        y: place(r.top, r.height(), area.top, area.height()),
        from_zone: true,
    });
    let gz = |edge: &str| gaming_guide(0, edge);
    let sides = [("left", &zone.left), ("right", &zone.right), ("top", &zone.top), ("bottom", &zone.bottom)];
    for (side, id) in sides {
        if let Ok(g) = guide_mut(layout, id) {
            g.anchor = Anchor::Offset { from: gz(side), px: 0 };
        }
    }
    // Only the zone's own sides follow it: guides positioned from them stay where they are.
    let others: Vec<String> = layout
        .guides
        .iter()
        .filter(|g| !sides.iter().any(|(_, id)| **id == g.id) && follows_gaming(layout, &g.id))
        .map(|g| g.id.clone())
        .collect();
    for id in others {
        convert_guide(layout, &id, GuideKind::Percent, area, monitor)?;
    }
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

/// The space a gaming zone made from a zone has: bounded by the dividers that follow its
/// edges, or the area's edges where there are none. `None` for other gaming zones.
pub fn gaming_space(layout: &Layout, area: Rect, monitor: Rect) -> Result<Option<Rect>, EditError> {
    if !layout.gaming_zones.first().is_some_and(|g| g.from_zone) {
        return Ok(None);
    }
    Ok(Some(space_sides(layout, area, monitor)?.1))
}

/// The space's sides (guide ids, left/right/top/bottom) and rectangle.
fn space_sides(layout: &Layout, area: Rect, monitor: Rect) -> Result<([String; 4], Rect), EditError> {
    let positions = layout.guide_positions(area, monitor)?;
    let side = |name: &str, area_side: &str| -> String {
        layout
            .guides
            .iter()
            .find(|g| edge_followed(g) == Some(name))
            .map_or_else(|| area_side.to_string(), |g| g.id.clone())
    };
    let ids = [side("left", AREA_LEFT), side("right", AREA_RIGHT), side("top", AREA_TOP), side("bottom", AREA_BOTTOM)];
    let p = |id: &String| positions.get(id).map_or(0, |&(_, v)| v);
    let rect = Rect::new(p(&ids[0]), p(&ids[2]), p(&ids[1]), p(&ids[3]));
    Ok((ids, rect))
}

/// Changes the gaming zone (size or position as in `new`) without moving any divider that
/// doesn't have to: its leftover zones get bigger or smaller instead. If it grows past the
/// dividers around its space, they're pushed out, and the zones beside it shrink.
pub fn set_gaming_zone(layout: &mut Layout, new: GamingZone, area: Rect, monitor: Rect) -> Result<(), EditError> {
    if layout.gaming_zones.is_empty() {
        return Err(EditError::NoGamingZone);
    }
    let before = layout.guide_positions(area, monitor)?;
    let resolved = layout.resolve(area, monitor, 0)?;
    layout.gaming_zones[0] = new;
    let g = layout.gaming_rects(area, monitor)?[0];
    let mut targets: Vec<(String, i32)> = layout.guides.iter().map(|g| (g.id.clone(), before[&g.id].1)).collect();
    for (id, at) in &mut targets {
        let guide = layout.guides.iter().find(|x| &x.id == id).expect("listed");
        *at = match edge_followed(guide) {
            Some("left") => (*at).min(g.left),
            Some("right") => (*at).max(g.right),
            Some("top") => (*at).min(g.top),
            Some("bottom") => (*at).max(g.bottom),
            _ => *at,
        };
    }
    place_guides(layout, &targets, area, monitor)?;
    let after = layout.resolve(area, monitor, 0)?;
    let inverted = |r: &Rect| r.width() < 0 || r.height() < 0;
    if resolved.iter().zip(&after).any(|(b, a)| !inverted(&b.rect) && inverted(&a.rect)) {
        return Err(EditError::CantResize);
    }
    Ok(())
}

/// Makes the gaming zone fill its space again (see [`gaming_space`]).
pub fn fill_space(layout: &mut Layout, area: Rect, monitor: Rect) -> Result<(), EditError> {
    let Some(space) = gaming_space(layout, area, monitor)? else {
        return Err(EditError::NoGamingZone);
    };
    let mut new = layout.gaming_zones[0].clone();
    new.size = GamingSize::Pixels { w: space.width().max(1) as u32, h: space.height().max(1) as u32 };
    new.x = Position::Px(space.left - area.left);
    new.y = Position::Px(space.top - area.top);
    set_gaming_zone(layout, new, area, monitor)
}

/// Turns the gaming zone(s) back into normal zones.
///
/// - Made from a zone ([`make_gaming_zone`]): the zones inside its space (the leftovers,
///   however they were split) and the gaming zone become one zone filling the space again.
/// - Otherwise a zone takes its place, and the zones around it stay where they are.
///
/// Either way nothing else moves on this monitor: guides that followed the gaming zone
/// become percentages of the area at the same place.
pub fn remove_gaming_zone(layout: &mut Layout, area: Rect, monitor: Rect) -> Result<(), EditError> {
    if layout.gaming_zones.is_empty() {
        return Err(EditError::NoGamingZone);
    }
    // Empty zones on the space's edge may belong inside it or not (see `merge`): take in as
    // few as keep the layout whole.
    for absorb in 0..3 {
        let mut trial = layout.clone();
        remove_gaming_zone_absorbing(&mut trial, absorb, area, monitor)?;
        if stays_whole(&trial, area, monitor) {
            *layout = trial;
            return Ok(());
        }
    }
    Err(EditError::ComesApart)
}

/// [`remove_gaming_zone`], taking in the empty zones on its space's edges that use the
/// gaming zone's edges (`absorb` = 0), or any guide that follows it (1), or all of them (2).
fn remove_gaming_zone_absorbing(layout: &mut Layout, absorb: u8, area: Rect, monitor: Rect) -> Result<(), EditError> {
    let positions = layout.guide_positions(area, monitor)?;
    let resolved = layout.resolve(area, monitor, 0)?;
    let from_zone = layout.gaming_zones[0].from_zone;
    if from_zone {
        let (sides, space) = space_sides(layout, area, monitor)?;
        let inside = |z: &Zone, r: &Rect| {
            let within = r.left.min(r.right) >= space.left
                && r.left.max(r.right) <= space.right
                && r.top.min(r.bottom) >= space.top
                && r.top.max(r.bottom) <= space.bottom;
            let sides = [&z.left, &z.right, &z.top, &z.bottom];
            within
                && (!r.is_empty()
                    || match absorb {
                        0 => sides.iter().any(|g| parse_gaming_guide(g).is_some()),
                        1 => sides.iter().any(|g| follows_gaming(layout, g)),
                        _ => true,
                    })
        };
        let [left, right, top, bottom] = sides;
        let whole = Zone { left, right, top, bottom };
        let mut zones = Vec::new();
        let mut placed = false;
        for (z, r) in layout.zones.iter().zip(&resolved) {
            if !inside(z, &r.rect) {
                zones.push(z.clone());
            } else if !placed {
                zones.push(whole.clone());
                placed = true;
            }
        }
        if !placed {
            zones.push(whole);
        }
        layout.zones = zones;
    }
    // Guides positioned from the gaming zone become percentages where they are.
    let percent_at = |axis: Axis, pos: i32| {
        let (start, end) = span(area, axis);
        let pos = pos.clamp(start, end);
        if end == start {
            0.0
        } else {
            let raw = (pos - start) as f64 * 100.0 / (end - start) as f64;
            shortest(raw, |p| percent_of(start, end, p) == pos as i64)
        }
    };
    let followers: Vec<String> =
        layout.guides.iter().filter(|g| follows_gaming(layout, &g.id)).map(|g| g.id.clone()).collect();
    for id in &followers {
        let (axis, pos) = positions[id];
        guide_mut(layout, id)?.anchor = Anchor::Percent(percent_at(axis, pos));
    }
    // Zones that used its edges get guides there: one that followed it on that line if there
    // is one (two guides on one line would come apart when one is dragged), or a new one.
    for i in 0..layout.gaming_zones.len() {
        let mut edges = Vec::new();
        for edge in ["left", "right", "top", "bottom"] {
            let old = gaming_guide(i, edge);
            let (axis, pos) = positions[&old];
            let id = match followers.iter().find(|f| positions[*f] == (axis, pos)) {
                Some(f) => f.clone(),
                None => {
                    let id = new_guide_id(layout);
                    layout.guides.push(Guide { id: id.clone(), axis, anchor: Anchor::Percent(percent_at(axis, pos)) });
                    id
                }
            };
            rename_references(layout, &old, &id);
            edges.push(id);
        }
        if !from_zone {
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
        assert_eq!(guide_limits(&l, "left-col", &[], WORK, MONITOR, 50), Ok((50, 3022)));
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
        assert_eq!(guide_limits(&l, "g1", &[1], WORK, MONITOR, 10), Ok((10, 4134)));
        // Typing zone 1's width pushes the locked zone too.
        resize_zone(&mut l, 0, Axis::Vertical, 2028, &[1], WORK, MONITOR).unwrap();
        assert_eq!(widths(&l), [2028, 2000, 2116]);
        // Two locked zones in a row move together.
        split(&mut l, 2, Axis::Vertical, 5000, WORK, MONITOR).unwrap();
        move_divider(&mut l, "g1", 1828, &[1, 2], WORK, MONITOR).unwrap();
        assert_eq!(widths(&l), [1828, 2000, 972, 1344]);
        // A locked zone against the screen's edge can't be pushed.
        assert_eq!(move_divider(&mut l, "g1", 1900, &[0], WORK, MONITOR), Err(EditError::Locked(0)));
        assert_eq!(guide_limits(&l, "g1", &[0], WORK, MONITOR, 10), Ok((1828, 1828)));
        // A locked zone's own size can't be typed.
        assert_eq!(resize_zone(&mut l, 1, Axis::Vertical, 100, &[1], WORK, MONITOR), Err(EditError::Locked(1)));
    }

    fn gaming(l: &Layout) -> Rect {
        *rects(l).last().unwrap()
    }

    /// The gaming zone at `w`×`h`, centred where it is now (as the editor's size boxes do).
    fn resize_gaming(l: &mut Layout, w: u32, h: u32) -> Result<(), EditError> {
        let g = gaming(l);
        let mut new = l.gaming_zones[0].clone();
        new.size = GamingSize::Pixels { w, h };
        if let Position::Px(_) = new.x {
            new.x = Position::Px(g.left + (g.width() - w as i32) / 2);
        }
        if let Position::Px(_) = new.y {
            new.y = Position::Px(g.top + (g.height() - h as i32) / 2);
        }
        set_gaming_zone(l, new, WORK, MONITOR)
    }

    #[test]
    fn making_a_zone_the_gaming_zone() {
        // Tom's Slay test: the middle column (1597..4547) becomes the gaming zone, all of it.
        let slay = LayoutsFile::defaults().get("slay").unwrap().clone();
        let mut l = slay.clone();
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        assert_eq!(l.gaming_zones[0].size, GamingSize::Pixels { w: 2950, h: 2500 });
        assert_eq!((l.gaming_zones[0].x, l.gaming_zones[0].from_zone), (Position::Align(Align::Centre), true));
        let zones = l.resolve(WORK, MONITOR, 300).unwrap();
        let r: Vec<Rect> = zones.iter().map(|z| z.rect).collect();
        assert_eq!(r[0], Rect::new(0, 0, 1597, 2500));
        assert_eq!(r[5], Rect::new(4547, 0, 6144, 2500));
        assert_eq!(r[6], Rect::new(1597, 0, 4547, 2500));
        // The four zones in its place are empty, so they have no number.
        assert!(r[1..5].iter().all(|r| r.is_empty()), "{r:?}");
        let labels: Vec<String> = (0..zones.len()).map(|i| crate::layout::zone_label(&zones, i)).collect();
        assert_eq!(labels, ["1", "-", "-", "-", "-", "2", "G"]);
        assert_eq!(gaming_space(&l, WORK, MONITOR), Ok(Some(Rect::new(1597, 0, 4547, 2500))));
        // Turned straight back: exactly the layout it was.
        let mut back = l.clone();
        remove_gaming_zone(&mut back, WORK, MONITOR).unwrap();
        assert_eq!(back, slay);

        // Made smaller, the bits it doesn't cover become zones; no divider moves.
        resize_gaming(&mut l, 1920, 1080).unwrap();
        assert_eq!(
            rects(&l),
            [
                Rect::new(0, 0, 1597, 2500),
                Rect::new(1597, 0, 4547, 710),     // above
                Rect::new(1597, 710, 2112, 1790),  // left
                Rect::new(4032, 710, 4547, 1790),  // right
                Rect::new(1597, 1790, 4547, 2500), // below
                Rect::new(4547, 0, 6144, 2500),
                Rect::new(2112, 710, 4032, 1790), // the gaming zone
            ]
        );
        // Fill its space again.
        let mut full = l.clone();
        fill_space(&mut full, WORK, MONITOR).unwrap();
        assert_eq!(gaming(&full), Rect::new(1597, 0, 4547, 2500));
        // Back to one zone where the column was.
        remove_gaming_zone(&mut l, WORK, MONITOR).unwrap();
        assert_eq!(rects(&l), rects(&slay));
        assert_eq!(make_gaming_zone(&mut l, 9, WORK, MONITOR), Err(EditError::NoSuchZone(9)));
    }

    #[test]
    fn the_gaming_zone_keeps_its_size() {
        // Tom: sizing up the zones around it mustn't resize it; it's pushed instead.
        let mut l = three_columns(); // 2000 | 2000 | 2144
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        // It fills its space: dragging the left divider right pushes it straight away.
        move_divider(&mut l, "g1", 2300, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(2300, 0, 4300, 2500));
        assert_eq!((rects(&l)[0].width(), rects(&l)[5].width()), (2300, 1844));
        // Typing the left zone's width does the same.
        resize_zone(&mut l, 0, Axis::Vertical, 2400, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(2400, 0, 4400, 2500));
        // Making it smaller (typed or dragged) leaves it where it is, with a leftover zone beside it.
        resize_zone(&mut l, 0, Axis::Vertical, 2028, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(2400, 0, 4400, 2500));
        assert_eq!(rects(&l)[2], Rect::new(2028, 0, 2400, 2500)); // left of it
                                                                  // Dragging back in closes that first, then pushes.
        move_divider(&mut l, "g1", 2500, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(2500, 0, 4500, 2500));
        assert!(rects(&l)[2].is_empty());
        // Pushed as far as the zone beyond allows (10 px left of it).
        move_divider(&mut l, "g1", 6000, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(4134, 0, 6134, 2500));
    }

    #[test]
    fn moving_the_gaming_zone_pushes_its_neighbours() {
        let mut l = three_columns();
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        resize_gaming(&mut l, 1600, 900).unwrap();
        assert_eq!(gaming(&l), Rect::new(2200, 800, 3800, 1700));
        // Across: its space (and the dividers around it) moves with it.
        move_gaming_zone(&mut l, 300, 0, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(2500, 800, 4100, 1700));
        let r = rects(&l);
        assert_eq!((r[0].width(), r[1], r[5].width()), (2300, Rect::new(2300, 0, 4300, 800), 1844));
        // Down: the column is the full height, so the strips above and below change.
        move_gaming_zone(&mut l, 0, 10_000, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l), Rect::new(2500, 1600, 4100, 2500));
        assert!(rects(&l)[4].is_empty());
        // Left as far as zone 1 allows (10 px).
        move_gaming_zone(&mut l, -10_000, 0, &[], WORK, MONITOR).unwrap();
        assert_eq!(gaming(&l).left, 210);
        // A locked zone beyond can't be pushed against the screen's edge.
        let mut l = three_columns();
        make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
        assert_eq!(move_gaming_zone(&mut l, 100, 0, &[5], WORK, MONITOR), Err(EditError::Locked(5)));
        // Growing past its space pushes the dividers out.
        resize_gaming(&mut l, 2560, 1440).unwrap();
        assert_eq!(gaming(&l), Rect::new(1720, 530, 4280, 1970));
        assert_eq!((rects(&l)[0].width(), rects(&l)[5].width()), (1720, 1864));
    }

    #[test]
    fn merging_beside_the_gaming_zone_keeps_dividers() {
        // Tom: after merging, the dividers couldn't be grabbed.
        let mut l = three_columns();
        split(&mut l, 0, Axis::Horizontal, 1000, WORK, MONITOR).unwrap(); // left column in two
        make_gaming_zone(&mut l, 2, WORK, MONITOR).unwrap();
        // Merging the left column's halves mustn't take in the empty strip beside the gaming
        // zone (or its edge): the merged zone still ends at g1.
        let i = merge(&mut l, &[0, 1], WORK, MONITOR).unwrap();
        assert_eq!(l.zones[i].right, "g1");
        assert_eq!(l.zones.len(), 6);
        let positions = l.guide_positions(WORK, MONITOR).unwrap();
        let g1 = dividers(&l, &positions).into_iter().find(|d| d.guide == "g1").unwrap();
        assert!(g1.spans.contains(&(0, 2500)), "{g1:?}");
    }

    #[test]
    fn only_the_zones_own_sides_follow_it() {
        // g2 is a split between g1 and the right edge; making zone 1 the gaming zone makes g1
        // follow it, but g2 stays where it is.
        let mut l = three_columns();
        make_gaming_zone(&mut l, 0, WORK, MONITOR).unwrap();
        assert!(!follows_gaming(&l, "g2"));
        move_divider(&mut l, "g1", 2500, &[], WORK, MONITOR).unwrap(); // a leftover strip opens
        assert_eq!(l.guide_positions(WORK, MONITOR).unwrap()["g2"].1, 4000);
        assert_eq!(gaming(&l), Rect::new(0, 0, 2000, 2500));
    }

    #[test]
    fn gaming_zone_in_a_single_zone_layout() {
        let mut l = new_layout("t", "T");
        make_gaming_zone(&mut l, 0, WORK, MONITOR).unwrap();
        assert!(l.validate().is_ok());
        resize_gaming(&mut l, 3840, 2160).unwrap();
        assert_eq!(gaming(&l), Rect::new(1152, 170, 4992, 2330));
        assert_eq!(rects(&l)[0], Rect::new(0, 0, 6144, 170));
        remove_gaming_zone(&mut l, WORK, MONITOR).unwrap();
        assert_eq!(l, new_layout("t", "T"));
    }

    /// Random edits, as a person might make them, each checked like the editor does (a
    /// failed edit changes nothing). After each one the zones and the gaming zone must still
    /// tile the screen exactly, and only resizing it may change the gaming zone's size.
    #[test]
    fn random_edits_keep_the_layout_whole() {
        // GZ_SEED tries other sequences (from WSL: WSLENV=GZ_SEED GZ_SEED=7 cargo test random).
        let mut seed: u64 = std::env::var("GZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x5EED_1234_ABCD_0001);
        let mut rnd = |n: i32| -> i32 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % n.max(1) as u64) as i32
        };
        let mut refused = 0;
        for run in 0..20 {
            let mut l = new_layout("t", "T");
            for step in 0..120 {
                let before = l.clone();
                let size_before = l.gaming_rects(WORK, MONITOR).unwrap().first().map(|g| (g.width(), g.height()));
                let zones = l.resolve(WORK, MONITOR, 0).unwrap();
                let count = l.zones.len() as i32;
                let pick = rnd(count) as usize;
                let op = rnd(9);
                let result = match op {
                    0 | 1 => {
                        let r = zones[pick].rect;
                        let axis = if rnd(2) == 0 { Axis::Vertical } else { Axis::Horizontal };
                        let (a, b) = span(r, axis);
                        if b - a > 2 * MIN_ZONE_PX {
                            split(&mut l, pick, axis, a + MIN_ZONE_PX + rnd(b - a - 2 * MIN_ZONE_PX), WORK, MONITOR)
                                .map(|_| ())
                        } else {
                            Ok(())
                        }
                    }
                    2 => merge(&mut l, &[pick, rnd(count) as usize], WORK, MONITOR).map(|_| ()),
                    3 => make_gaming_zone(&mut l, pick, WORK, MONITOR),
                    4 if !l.guides.is_empty() => {
                        let g = l.guides[rnd(l.guides.len() as i32) as usize].clone();
                        let (s, e) = span(WORK, g.axis);
                        let locked: Vec<usize> = (0..count as usize).filter(|_| rnd(4) == 0).collect();
                        move_divider(&mut l, &g.id, s + rnd(e - s), &locked, WORK, MONITOR)
                    }
                    5 => move_gaming_zone(&mut l, rnd(3000) - 1500, rnd(2000) - 1000, &[], WORK, MONITOR),
                    6 if !l.gaming_zones.is_empty() => {
                        resize_gaming(&mut l, 200 + rnd(4000) as u32, 200 + rnd(2400) as u32)
                    }
                    7 => resize_zone(&mut l, pick, Axis::Vertical, MIN_ZONE_PX + rnd(3000), &[], WORK, MONITOR),
                    8 if rnd(4) == 0 => remove_gaming_zone(&mut l, WORK, MONITOR),
                    _ => Ok(()),
                };
                let what = format!("run {run} step {step} op {op}");
                if std::env::var("GZ_TRACE").ok() == Some(run.to_string()) {
                    eprintln!("=== {what} {result:?}\n{l:?}");
                }
                // As in the editor: chance meetings are joined, and a change that leaves the
                // layout coming apart is refused.
                if result.is_ok() && l != before {
                    join_coincident(&mut l, WORK, MONITOR);
                }
                let result = match result {
                    Ok(()) if l.validate().is_ok() && !stays_whole(&l, WORK, MONITOR) => {
                        refused += 1;
                        Err(EditError::ComesApart)
                    }
                    r => r,
                };
                if result.is_err() || l.validate().is_err() {
                    assert!(l.validate().is_ok() || result.is_err(), "{what}: {:?}\n{l:#?}", l.validate());
                    l = before;
                    continue;
                }
                let zones = l.resolve(WORK, MONITOR, 0).unwrap();
                let rects: Vec<Rect> = zones.iter().map(|z| z.rect).filter(|r| !r.is_empty()).collect();
                let area = |r: &Rect| r.width() as i64 * r.height() as i64;
                let overlap = |a: &Rect, b: &Rect| {
                    a.left.max(b.left) < a.right.min(b.right) && a.top.max(b.top) < a.bottom.min(b.bottom)
                };
                for (n, a) in rects.iter().enumerate() {
                    assert!(
                        a.left >= 0 && a.top >= 0 && a.right <= 6144 && a.bottom <= 2500,
                        "{what}: {a:?} sticks out\n{l:#?}"
                    );
                    for b in &rects[n + 1..] {
                        assert!(!overlap(a, b), "{what}: {a:?} overlaps {b:?}\nbefore {before:#?}\nafter {l:#?}");
                    }
                }
                assert_eq!(
                    rects.iter().map(area).sum::<i64>(),
                    area(&WORK),
                    "{what}: gaps\nbefore {before:#?}\nafter {l:#?}"
                );
                // Zones meeting along a line must share the guide there, or dragging it
                // pulls them apart.
                for (a, za) in l.zones.iter().enumerate() {
                    for (b, zb) in l.zones.iter().enumerate() {
                        let (ra, rb) = (zones[a].rect, zones[b].rect);
                        if ra.is_empty() || rb.is_empty() {
                            continue;
                        }
                        // (Empty zones between them still hold them apart.)
                        let between = |near: &String, far: &String, vertical: bool| {
                            let mut reached = vec![near.clone()];
                            for _ in 0..l.zones.len() {
                                for (z, r) in l.zones.iter().zip(&zones) {
                                    let (a, b) = if vertical { (&z.left, &z.right) } else { (&z.top, &z.bottom) };
                                    if r.rect.is_empty() && reached.contains(a) && !reached.contains(b) {
                                        reached.push(b.clone());
                                    }
                                }
                            }
                            reached.contains(far)
                        };
                        let across = ra.right == rb.left
                            && ra.top.max(rb.top) < ra.bottom.min(rb.bottom)
                            && za.right != zb.left
                            && !between(&za.right, &zb.left, true);
                        let down = ra.bottom == rb.top
                            && ra.left.max(rb.left) < ra.right.min(rb.right)
                            && za.bottom != zb.top
                            && !between(&za.bottom, &zb.top, false);
                        assert!(
                            !across && !down,
                            "{what}: zones {a} and {b} meet on two guides\nbefore {before:#?}\nafter {l:#?}"
                        );
                    }
                }
                let size_after = l.gaming_rects(WORK, MONITOR).unwrap().first().map(|g| (g.width(), g.height()));
                if op != 6 && size_before.is_some() && size_after.is_some() {
                    assert_eq!(
                        size_before, size_after,
                        "{what}: the gaming zone was resized\nbefore {before:#?}\nafter {l:#?}"
                    );
                }
            }
        }
        // Rare: only odd sequences of edits get there.
        assert!(refused < 30, "{refused} edits refused");
        eprintln!("random edits: {refused} refused as coming apart");
    }

    /// Checks a real layouts.json (GZ_LAYOUTS = its path): every layout loads, and says which
    /// ones come apart when something moves, before and after joining chance meetings.
    #[test]
    #[ignore = "needs GZ_LAYOUTS"]
    fn check_a_layouts_file() {
        let path = std::env::var("GZ_LAYOUTS").expect("GZ_LAYOUTS");
        let file = LayoutsFile::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        for l in &file.layouts {
            let mut joined = l.clone();
            let changed = join_coincident(&mut joined, WORK, MONITOR);
            eprintln!(
                "{:<20} whole: {:<5} joined: {:<5} whole after: {}",
                l.name,
                stays_whole(l, WORK, MONITOR),
                changed,
                stays_whole(&joined, WORK, MONITOR)
            );
        }
    }

    #[test]
    fn joining_guides_that_meet_by_chance() {
        // Tom's "Six Panel": the middle column's two zones meet at 50 % on two guides.
        let mut l = three_columns();
        l.guides.push(Guide { id: "a".into(), axis: Axis::Horizontal, anchor: Anchor::Percent(50.0) });
        l.guides.push(Guide { id: "b".into(), axis: Axis::Horizontal, anchor: Anchor::Percent(50.0) });
        let mid = l.zones[1].clone();
        l.zones[1] = Zone { bottom: "a".into(), ..mid.clone() };
        l.zones.insert(2, Zone { top: "b".into(), ..mid });
        assert!(!stays_whole(&l, WORK, MONITOR));
        let before = rects(&l);
        assert!(join_coincident(&mut l, WORK, MONITOR));
        assert_eq!(l.zones[2].top, "a");
        assert!(!l.guides.iter().any(|g| g.id == "b"));
        assert_eq!(rects(&l), before);
        assert!(stays_whole(&l, WORK, MONITOR));
        assert!(!join_coincident(&mut l, WORK, MONITOR));
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
