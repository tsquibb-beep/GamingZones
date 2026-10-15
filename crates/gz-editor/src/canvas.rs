//! The canvas: the selected monitor drawn to scale, with the layout's zones, dividers and
//! gaming zone. A click splits a zone; dragging moves a divider or the gaming zone, or
//! selects a range of zones.

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Sense, Stroke, StrokeKind, Vec2};
use gz_core::edit::{self, Divider};
use gz_core::layout::{zone_number, Axis, ResolvedZone};
use gz_core::Rect;

use crate::app::{Editor, Selection, MIN_ZONE_PX};

pub const ZONE_FILL: Color32 = Color32::from_rgb(0x3B, 0x46, 0x60);
pub const DISABLED_FILL: Color32 = Color32::from_rgb(0x46, 0x46, 0x46);
pub const GAMING_FILL: Color32 = Color32::from_rgb(0x1C, 0x7A, 0x48);
const ZONE_HOVER: Color32 = Color32::from_rgb(0x4C, 0x5A, 0x7A);
const SELECTED_FILL: Color32 = Color32::from_rgb(0x00, 0x6E, 0xD0);
const GAMING_SELECTED: Color32 = Color32::from_rgb(0x27, 0xA0, 0x60);
const GAMING_STROKE: Color32 = Color32::from_rgb(0x4A, 0xE8, 0x8C);
const ZONE_STROKE: Color32 = Color32::from_rgb(0x8A, 0x97, 0xB4);
const MONITOR_BG: Color32 = Color32::from_rgb(0x14, 0x16, 0x1A);
const WORK_BG: Color32 = Color32::from_rgb(0x22, 0x26, 0x2E);
const DIVIDER: Color32 = Color32::from_rgb(0xC8, 0xD0, 0xE0);
const DIVIDER_SELECTED: Color32 = Color32::from_rgb(0xFF, 0xB0, 0x20);
const TEXT: Color32 = Color32::from_rgb(0xF0, 0xF2, 0xF6);
const TEXT_WEAK: Color32 = Color32::from_rgb(0xA8, 0xB0, 0xC0);
/// How close (points) the pointer must be to grab a divider.
const GRAB: f32 = 5.0;

#[derive(Clone)]
pub enum CanvasDrag {
    Divider {
        guide: String,
        axis: Axis,
    },
    /// Offset of the grab point from the gaming zone's top-left corner (px).
    Gaming {
        dx: i32,
        dy: i32,
    },
    /// Selecting a range of zones, starting at this one.
    Select {
        from: usize,
    },
}

#[derive(Clone, PartialEq)]
enum Target {
    Divider(String, Axis),
    Gaming,
    Zone(usize),
    Nothing,
}

/// Maps a rect inside `from` (screen px) onto `to` (points).
pub fn map_rect(r: Rect, from: Rect, to: egui::Rect) -> egui::Rect {
    let sx = to.width() / from.width().max(1) as f32;
    let sy = to.height() / from.height().max(1) as f32;
    let p = |x: i32, y: i32| egui::pos2(to.left() + (x - from.left) as f32 * sx, to.top() + (y - from.top) as f32 * sy);
    egui::Rect::from_min_max(p(r.left, r.top), p(r.right, r.bottom))
}

/// Screen px ↔ canvas points, keeping the monitor's shape.
struct View {
    origin: Pos2,
    scale: f32,
    monitor: Rect,
}

impl View {
    fn fit(space: egui::Rect, monitor: Rect) -> View {
        let (w, h) = (monitor.width().max(1) as f32, monitor.height().max(1) as f32);
        let scale = (space.width() / w).min(space.height() / h).max(0.001);
        let size = Vec2::new(w * scale, h * scale);
        View { origin: space.center() - size / 2.0, scale, monitor }
    }

    fn pos(&self, x: i32, y: i32) -> Pos2 {
        self.origin + Vec2::new((x - self.monitor.left) as f32 * self.scale, (y - self.monitor.top) as f32 * self.scale)
    }

    fn rect(&self, r: Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.pos(r.left, r.top), self.pos(r.right, r.bottom))
    }

    fn px(&self, p: Pos2) -> (i32, i32) {
        let v = (p - self.origin) / self.scale;
        (self.monitor.left + v.x.round() as i32, self.monitor.top + v.y.round() as i32)
    }
}

/// What's under point `p`: the gaming zone, then dividers, then the smallest zone.
fn target(view: &View, p: Pos2, zones: &[ResolvedZone], normal: usize, dividers: &[Divider]) -> Target {
    let (x, y) = view.px(p);
    if zones.get(normal).is_some_and(|g| g.rect.contains(x, y)) {
        return Target::Gaming;
    }
    for d in dividers {
        let (along, across) = match d.axis {
            Axis::Vertical => (y, (p.x - view.pos(d.pos, 0).x).abs()),
            Axis::Horizontal => (x, (p.y - view.pos(0, d.pos).y).abs()),
        };
        if across <= GRAB && d.spans.iter().any(|&(a, b)| a <= along && along <= b) {
            return Target::Divider(d.guide.clone(), d.axis);
        }
    }
    zones[..normal]
        .iter()
        .enumerate()
        .filter(|(_, z)| !z.rect.is_empty() && z.rect.contains(x, y))
        .min_by_key(|(_, z)| z.rect.width() as i64 * z.rect.height() as i64)
        .map_or(Target::Nothing, |(i, _)| Target::Zone(i))
}

/// Zones inside the bounding rectangle of zones `a` and `b`.
fn range(zones: &[ResolvedZone], normal: usize, a: usize, b: usize) -> Vec<usize> {
    let bounds = zones[a].rect.union(&zones[b].rect);
    (0..normal)
        .filter(|&i| {
            let r = zones[i].rect;
            bounds.left <= r.left && r.right <= bounds.right && bounds.top <= r.top && r.bottom <= bounds.bottom
        })
        .collect()
}

/// A zone's number (big) and size (small), if there's room.
fn label(painter: &egui::Painter, r: egui::Rect, big: &str, small: &str, colour: Color32) {
    if r.width() < 14.0 || r.height() < 14.0 {
        return;
    }
    let size = (r.width().min(r.height()) / 3.0).clamp(10.0, 40.0);
    let room_for_small = r.height() > size + 34.0 && r.width() > 80.0;
    let centre = if room_for_small { r.center() - Vec2::new(0.0, 8.0) } else { r.center() };
    painter.text(centre, Align2::CENTER_CENTER, big, FontId::proportional(size), colour);
    if room_for_small {
        let below = centre + Vec2::new(0.0, size * 0.55 + 4.0);
        painter.text(below, Align2::CENTER_TOP, small, FontId::proportional(12.0), TEXT_WEAK);
    }
}

impl Editor {
    pub fn canvas(&mut self, ui: &mut egui::Ui) {
        let (work, monitor) = self.areas();
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let view = View::fit(response.rect.shrink(16.0), monitor);
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));

        // Input, against the layout as it was at the start of the frame.
        {
            let layout = self.current().clone();
            let (Ok(zones), Ok(positions)) =
                (layout.resolve(work, monitor, self.deadzone), layout.guide_positions(work, monitor))
            else {
                return;
            };
            let dividers = edit::dividers(&layout, &positions);
            let normal = layout.zones.len();
            let at = |p: Pos2| target(&view, p, &zones, normal, &dividers);

            if response.drag_started() {
                let origin = ui.input(|i| i.pointer.press_origin());
                self.canvas_drag = match origin.map(at) {
                    Some(Target::Divider(guide, axis)) => {
                        self.selection = Selection::Divider(guide.clone());
                        Some(CanvasDrag::Divider { guide, axis })
                    }
                    Some(Target::Gaming) => {
                        let (x, y) = view.px(origin.expect("matched Some"));
                        let g = zones[normal].rect;
                        self.selection = Selection::Gaming;
                        Some(CanvasDrag::Gaming { dx: x - g.left, dy: y - g.top })
                    }
                    Some(Target::Zone(i)) => Some(CanvasDrag::Select { from: i }),
                    _ => None,
                };
            }
            if response.dragged() {
                if let (Some(drag), Some(p)) = (self.canvas_drag.clone(), response.interact_pointer_pos()) {
                    let (x, y) = view.px(p);
                    match drag {
                        CanvasDrag::Divider { guide, axis } => {
                            self.move_divider(&guide, if axis == Axis::Vertical { x } else { y });
                        }
                        CanvasDrag::Gaming { dx, dy } => self.move_gaming_zone(x - dx, y - dy),
                        CanvasDrag::Select { from } => {
                            if let Target::Zone(to) = at(p) {
                                self.selection = Selection::Zones(range(&zones, normal, from, to));
                            }
                        }
                    }
                }
            }
            if response.drag_stopped() {
                self.canvas_drag = None;
            }
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    match at(p) {
                        Target::Divider(guide, _) => self.selection = Selection::Divider(guide),
                        Target::Gaming => self.selection = Selection::Gaming,
                        Target::Zone(i) if ctrl => {
                            let mut v = match &self.selection {
                                Selection::Zones(v) => v.clone(),
                                _ => Vec::new(),
                            };
                            match v.iter().position(|&s| s == i) {
                                Some(n) => {
                                    v.remove(n);
                                }
                                None => v.push(i),
                            }
                            v.sort_unstable();
                            self.selection = Selection::Zones(v);
                        }
                        Target::Zone(i) => {
                            let (x, y) = view.px(p);
                            self.split_at(i, x, y, shift, zones[i].rect);
                        }
                        Target::Nothing => self.selection = Selection::None,
                    }
                }
            }
        }

        // Drawing, with the layout after this frame's changes.
        let layout = self.current().clone();
        let zones = match layout.resolve(work, monitor, self.deadzone) {
            Ok(z) => z,
            Err(e) => {
                painter.text(response.rect.center(), Align2::CENTER_CENTER, e.to_string(), FontId::default(), TEXT);
                return;
            }
        };
        let Ok(positions) = layout.guide_positions(work, monitor) else { return };
        let dividers = edit::dividers(&layout, &positions);
        let normal = layout.zones.len();
        let hover_pos = response.hover_pos();
        let hover = match (&self.canvas_drag, hover_pos) {
            (None, Some(p)) => target(&view, p, &zones, normal, &dividers),
            _ => Target::Nothing,
        };

        painter.rect_filled(view.rect(monitor), 4.0, MONITOR_BG);
        painter.rect_filled(view.rect(work), 0.0, WORK_BG);
        let selected: &[usize] = match &self.selection {
            Selection::Zones(v) => v,
            _ => &[],
        };
        for (i, z) in zones[..normal].iter().enumerate() {
            if z.rect.is_empty() {
                continue;
            }
            let r = view.rect(z.rect).shrink(1.0);
            let fill = if selected.contains(&i) {
                SELECTED_FILL
            } else if z.disabled {
                DISABLED_FILL
            } else if hover == Target::Zone(i) {
                ZONE_HOVER
            } else {
                ZONE_FILL
            };
            painter.rect_filled(r, 3.0, fill);
            painter.rect_stroke(r, 3.0, Stroke::new(1.0, ZONE_STROKE), StrokeKind::Inside);
            let lock = if self.is_locked(i) { "🔒 " } else { "" };
            let size = format!("{lock}{} × {}", z.rect.width(), z.rect.height());
            let number = zone_number(&zones, i).map_or(String::new(), |n| n.to_string());
            label(&painter, r, &number, &size, if z.disabled { TEXT_WEAK } else { TEXT });
        }
        if let Some(g) = zones.get(normal) {
            let r = view.rect(g.rect);
            let fill = if self.selection == Selection::Gaming { GAMING_SELECTED } else { GAMING_FILL };
            painter.rect_filled(r, 3.0, fill.gamma_multiply(0.9));
            painter.rect_stroke(r, 3.0, Stroke::new(2.0, GAMING_STROKE), StrokeKind::Inside);
            label(&painter, r, "G", &format!("{} × {}", g.rect.width(), g.rect.height()), TEXT);
        }
        for d in &dividers {
            let hot = matches!(&hover, Target::Divider(g, _) if *g == d.guide)
                || matches!(&self.canvas_drag, Some(CanvasDrag::Divider { guide, .. }) if *guide == d.guide);
            let colour = if self.selection == Selection::Divider(d.guide.clone()) {
                DIVIDER_SELECTED
            } else if hot {
                Color32::WHITE
            } else {
                DIVIDER
            };
            let stroke = Stroke::new(if hot { 4.0 } else { 2.0 }, colour);
            for &(a, b) in &d.spans {
                let ends = match d.axis {
                    Axis::Vertical => [view.pos(d.pos, a), view.pos(d.pos, b)],
                    Axis::Horizontal => [view.pos(a, d.pos), view.pos(b, d.pos)],
                };
                painter.line_segment(ends, stroke);
            }
        }

        // Where a click would split, and the cursor.
        let cursor = match (&self.canvas_drag, &hover) {
            (Some(CanvasDrag::Divider { axis: Axis::Vertical, .. }), _)
            | (None, Target::Divider(_, Axis::Vertical)) => CursorIcon::ResizeHorizontal,
            (Some(CanvasDrag::Divider { .. }), _) | (None, Target::Divider(..)) => CursorIcon::ResizeVertical,
            (Some(CanvasDrag::Gaming { .. }), _) => CursorIcon::Grabbing,
            (None, Target::Gaming) => CursorIcon::Grab,
            (None, Target::Zone(_)) if ctrl => CursorIcon::PointingHand,
            (None, Target::Zone(_)) => CursorIcon::Crosshair,
            _ => CursorIcon::Default,
        };
        if response.hovered() || self.canvas_drag.is_some() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if let (Target::Zone(i), Some(p), false) = (&hover, hover_pos, ctrl) {
            let zr = view.rect(zones[*i].rect);
            let (x, y) = view.px(p);
            let (ends, text) = if shift {
                ([egui::pos2(zr.left(), p.y), egui::pos2(zr.right(), p.y)], format!("y = {}", y - work.top))
            } else {
                ([egui::pos2(p.x, zr.top()), egui::pos2(p.x, zr.bottom())], format!("x = {}", x - work.left))
            };
            painter.line_segment(ends, Stroke::new(1.5, Color32::from_white_alpha(170)));
            painter.text(p + Vec2::new(10.0, -10.0), Align2::LEFT_BOTTOM, text, FontId::proportional(12.0), TEXT);
        }
    }

    /// Splits zone `i` at (`x`, `y`): vertically, or horizontally with Shift.
    fn split_at(&mut self, i: usize, x: i32, y: i32, horizontal: bool, zone: Rect) {
        let axis = if horizontal { Axis::Horizontal } else { Axis::Vertical };
        let (pos, lo, hi) = match axis {
            Axis::Vertical => (x, zone.left, zone.right),
            Axis::Horizontal => (y, zone.top, zone.bottom),
        };
        if pos - lo < MIN_ZONE_PX || hi - pos < MIN_ZONE_PX {
            self.set_error("That's too close to the zone's edge to split there.");
            return;
        }
        if self.edit_layout("", |l, w, m| edit::split(l, i, axis, pos, w, m)).is_some() {
            self.selection = Selection::None;
        }
    }

    /// Moves the gaming zone's top-left corner towards (`left`, `top`), pushing the zones
    /// beside it (see [`edit::move_gaming_zone`]).
    pub fn move_gaming_zone(&mut self, left: i32, top: i32) {
        let (work, monitor) = self.areas();
        let Ok(rects) = self.current().gaming_rects(work, monitor) else { return };
        let Some(g) = rects.first().copied() else { return };
        let (dx, dy) = (left - g.left, top - g.top);
        if (dx, dy) == (0, 0) {
            return;
        }
        let locked = self.locked_indices();
        self.edit_layout("gaming-move", |l, w, m| edit::move_gaming_zone(l, dx, dy, &locked, w, m));
    }
}
