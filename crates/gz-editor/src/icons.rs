//! Flat, front-on icons drawn with the painter (no icon font), and the buttons that use them.

use std::f32::consts::PI;

use eframe::egui::{
    self, Align2, Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, WidgetInfo, WidgetType,
};

use crate::style;

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    /// A monitor, with its number (from 1) on the screen; 0 shows none.
    Monitor(usize),
    Undo,
    Redo,
    Eye,
    Save,
    Bin,
    /// A padlock: closed (true) or open.
    Lock(bool),
    Plus,
}

/// Points along a quadratic Bézier curve.
fn quad(a: Pos2, c: Pos2, b: Pos2) -> Vec<Pos2> {
    (0..=16)
        .map(|i| {
            let t = i as f32 / 16.0;
            let u = 1.0 - t;
            (a.to_vec2() * u * u + c.to_vec2() * 2.0 * u * t + b.to_vec2() * t * t).to_pos2()
        })
        .collect()
}

/// Draws `icon` in `rect` (square works best).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, colour: Color32) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - Vec2::splat(s / 2.0);
    // A point in the icon's own 0..1 square.
    let p = |x: f32, y: f32| o + Vec2::new(x * s, y * s);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(p(x0, y0), p(x1, y1));
    let line = Stroke::new((s / 13.0).max(1.2), colour);
    match icon {
        Icon::Monitor(n) => {
            painter.rect_stroke(r(0.06, 0.12, 0.94, 0.72), s * 0.06, line, StrokeKind::Middle);
            painter.line_segment([p(0.5, 0.72), p(0.5, 0.86)], line);
            painter.line_segment([p(0.3, 0.88), p(0.7, 0.88)], line);
            if n > 0 {
                painter.text(p(0.5, 0.43), Align2::CENTER_CENTER, n.to_string(), style::semibold(s * 0.36), colour);
            }
        }
        Icon::Undo | Icon::Redo => {
            // An arc that rises from the bottom and curves back over the top, ending in an
            // arrowhead pointing down: backwards (left) for undo, forwards for redo.
            let flip = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let (cx, cy, radius) = (0.5, 0.56, 0.27);
            let at = |a: f32| p(0.5 + flip * (cx - 0.5 + radius * a.cos()), cy + radius * a.sin());
            let (from, to) = (0.35 * PI, -PI);
            let points: Vec<Pos2> = (0..=24).map(|i| at(from + (to - from) * i as f32 / 24.0)).collect();
            let end = *points.last().expect("24 points");
            painter.add(Shape::line(points, line));
            let h = s * 0.16;
            painter.add(Shape::convex_polygon(
                vec![end + Vec2::new(-h, -h * 0.2), end + Vec2::new(h, -h * 0.2), end + Vec2::new(0.0, h)],
                colour,
                Stroke::NONE,
            ));
        }
        Icon::Eye => {
            let (a, b) = (p(0.06, 0.5), p(0.94, 0.5));
            painter.add(Shape::line(quad(a, p(0.5, 0.08), b), line));
            painter.add(Shape::line(quad(a, p(0.5, 0.92), b), line));
            painter.circle_stroke(p(0.5, 0.5), s * 0.15, line);
            painter.circle_filled(p(0.5, 0.5), s * 0.06, colour);
        }
        Icon::Save => {
            let body = vec![p(0.14, 0.14), p(0.72, 0.14), p(0.86, 0.28), p(0.86, 0.86), p(0.14, 0.86)];
            painter.add(Shape::closed_line(body, line));
            painter.rect_stroke(r(0.3, 0.14, 0.64, 0.36), 0.0, line, StrokeKind::Middle);
            painter.rect_stroke(r(0.28, 0.56, 0.72, 0.86), 0.0, line, StrokeKind::Middle);
        }
        Icon::Bin => {
            painter.line_segment([p(0.16, 0.25), p(0.84, 0.25)], line);
            painter.add(Shape::line(vec![p(0.4, 0.25), p(0.4, 0.13), p(0.6, 0.13), p(0.6, 0.25)], line));
            let body = vec![p(0.25, 0.25), p(0.75, 0.25), p(0.7, 0.88), p(0.3, 0.88)];
            painter.add(Shape::closed_line(body, line));
            for x in [0.43, 0.57] {
                painter.line_segment([p(x, 0.4), p(x, 0.74)], line);
            }
        }
        Icon::Lock(closed) => {
            // The shackle: two legs and a half circle, straight on. Open, it's lifted out of
            // the body on the left.
            let lift = if closed { 0.0 } else { 0.1 };
            let (cx, cy, radius) = (0.5, 0.3 - lift, 0.19);
            let mut shackle = vec![p(cx - radius, if closed { 0.48 } else { cy + 0.05 })];
            shackle.extend((0..=16).map(|i| {
                let a = PI + PI * i as f32 / 16.0;
                p(cx + radius * a.cos(), cy + radius * a.sin())
            }));
            shackle.push(p(cx + radius, 0.48));
            painter.add(Shape::line(shackle, line));
            painter.rect_filled(r(0.2, 0.46, 0.8, 0.9), s * 0.07, colour);
            let hole = Color32::from_black_alpha(150);
            painter.circle_filled(p(0.5, 0.63), s * 0.065, hole);
            painter.rect_filled(r(0.475, 0.63, 0.525, 0.77), 0.0, hole);
        }
        Icon::Plus => {
            painter.line_segment([p(0.5, 0.18), p(0.5, 0.82)], line);
            painter.line_segment([p(0.18, 0.5), p(0.82, 0.5)], line);
        }
    }
}

/// A button showing `icon` (`size` points), with `caption` under it if given. `name` is what
/// screen readers and the UI tests call it.
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    caption: Option<&str>,
    name: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(11.0);
    let caption_galley = caption.map(|c| ui.painter().layout_no_wrap(c.to_string(), font, Color32::PLACEHOLDER));
    let (w, h) = match &caption_galley {
        Some(g) => ((size + 16.0).max(g.size().x + 16.0), size + g.size().y + 14.0),
        None => (size + 12.0, size + 12.0),
    };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, h), sense);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, name));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if selected {
            ui.painter().rect_filled(rect, 6.0, visuals.selection.bg_fill.gamma_multiply(0.6));
        } else if enabled && response.hovered() {
            ui.painter().rect_filled(rect, 6.0, visuals.widgets.hovered.weak_bg_fill);
        }
        let colour = if !enabled {
            visuals.weak_text_color().gamma_multiply(0.6)
        } else if selected || response.hovered() {
            visuals.strong_text_color()
        } else {
            visuals.text_color()
        };
        let icon_rect =
            Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 6.0 + size / 2.0), Vec2::splat(size));
        paint(ui.painter(), icon_rect, icon, colour);
        if let Some(g) = caption_galley {
            let at = egui::pos2(rect.center().x - g.size().x / 2.0, icon_rect.bottom() + 4.0);
            ui.painter().galley(at, g, colour);
        }
    }
    response
}

/// A framed button with `icon` before `text`. `accent` gives it the selection colour (the
/// main action).
pub fn labelled(ui: &mut egui::Ui, icon: Icon, text: &str, accent: bool) -> egui::Response {
    let icon_size = 16.0;
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), Color32::PLACEHOLDER);
    let w = (12.0 + icon_size + 6.0 + galley.size().x + 12.0).max(style::DIALOG_W);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, style::ROW_H + 6.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, text));
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&response);
        let (fill, colour) = if accent {
            let fill = ui.visuals().selection.bg_fill;
            (if response.hovered() { fill.gamma_multiply(1.25) } else { fill }, Color32::WHITE)
        } else {
            (v.weak_bg_fill, v.text_color())
        };
        ui.painter().rect(rect, 6.0, fill, v.bg_stroke, StrokeKind::Inside);
        let content_w = icon_size + 6.0 + galley.size().x;
        let left = rect.center().x - content_w / 2.0;
        let icon_rect =
            Rect::from_min_size(egui::pos2(left, rect.center().y - icon_size / 2.0), Vec2::splat(icon_size));
        paint(ui.painter(), icon_rect, icon, colour);
        let at = egui::pos2(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, colour);
    }
    response
}
