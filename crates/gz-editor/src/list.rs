//! The layout list: a card per layout (a small picture of its zones, its number and name),
//! a right-click menu (rename, duplicate, move up/down, use on a monitor, delete) and
//! renaming in place.

use eframe::egui::{self, Align2, Color32, FontId, Key, Sense, Vec2};
use gz_core::gaming::gcd;

use crate::app::{Dialog, Editor, Goto};
use crate::canvas;
use crate::icons::{self, Icon};
use crate::style;

/// Height of the name strip under a card's picture.
const NAME_H: f32 = 32.0;
/// The layout number on a card (the layout selector's blue).
const NUMBER_COLOUR: Color32 = Color32::from_rgb(0x00, 0x8C, 0xFF);

/// How a gaming zone's shape is usually called: "16:9", "21:9" (for 2560×1080, 3440×1440…).
pub fn ratio_name(w: i32, h: i32) -> String {
    let (w, h) = (w.max(1) as u32, h.max(1) as u32);
    let g = gcd(w, h).max(1);
    match (w / g, h / g) {
        (64, 27) | (43, 18) | (12, 5) | (7, 3) => "21:9".into(),
        (8, 5) => "16:10".into(),
        (a, b) if a <= 32 && b <= 32 => format!("{a}:{b}"),
        _ => format!("{:.2}:1", w as f32 / h as f32),
    }
}

impl Editor {
    pub fn layout_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Layouts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Plus, 18.0, None, "New", false, true).on_hover_text("New layout").clicked() {
                    self.new_layout();
                }
            });
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for i in 0..self.doc.layouts.layouts.len() {
                self.card(ui, i);
                ui.add_space(6.0);
            }
        });
    }

    /// A layout card: a small picture of its zones on the selected monitor, and its name.
    fn card(&mut self, ui: &mut egui::Ui, i: usize) {
        let (work, monitor) = self.areas();
        let layout = self.doc.layouts.layouts[i].clone();
        let width = ui.available_width();
        let picture_h = (width - 12.0) * work.height() as f32 / work.width().max(1) as f32;
        let picture_h = picture_h.clamp(30.0, 160.0);
        let (response, painter) = ui.allocate_painter(Vec2::new(width, picture_h + NAME_H), Sense::click());
        let r = response.rect;
        let selected = i == self.layout;
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &layout.name));
        let visuals = ui.visuals().clone();
        let bg = if selected {
            visuals.selection.bg_fill.gamma_multiply(0.5)
        } else if response.hovered() {
            visuals.widgets.hovered.bg_fill
        } else {
            visuals.faint_bg_color
        };
        painter.rect_filled(r, 6.0, bg);
        if selected {
            painter.rect_stroke(r, 6.0, visuals.selection.stroke, egui::StrokeKind::Inside);
        }
        let picture = egui::Rect::from_min_size(r.min + Vec2::new(6.0, 6.0), Vec2::new(width - 12.0, picture_h));
        if let Ok(zones) = layout.resolve(work, monitor, self.deadzone) {
            for z in zones.iter().filter(|z| !z.rect.is_empty()) {
                let zr = canvas::map_rect(z.rect, work, picture).shrink(1.0);
                let fill = if z.gaming {
                    canvas::GAMING_FILL
                } else if z.disabled {
                    canvas::DISABLED_FILL
                } else {
                    canvas::ZONE_FILL
                };
                painter.rect_filled(zr, 2.0, fill);
            }
            if let Some(g) = zones.iter().find(|z| z.gaming) {
                gaming_label(&painter, canvas::map_rect(g.rect, work, picture), g.rect.width(), g.rect.height());
            }
        }
        let name_strip = egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - NAME_H), r.max);
        let text_y = name_strip.center().y;
        let renaming = self.renaming.as_ref().is_some_and(|(n, _)| *n == i);
        if renaming {
            self.rename_box(ui, i, name_strip.shrink2(Vec2::new(4.0, 4.0)));
        } else {
            // Layouts 1-9 are numbered: the quick-switch keys and the layout selector use them.
            let mut x = r.left() + 8.0;
            if i < 9 {
                let number = painter.text(
                    egui::pos2(x, text_y),
                    Align2::LEFT_CENTER,
                    (i + 1).to_string(),
                    style::semibold(14.0),
                    NUMBER_COLOUR,
                );
                x = number.right() + 8.0;
            }
            painter.text(
                egui::pos2(x, text_y),
                Align2::LEFT_CENTER,
                &layout.name,
                FontId::proportional(14.0),
                visuals.strong_text_color(),
            );
        }
        if !renaming && self.doc.applied.layout_for(&self.mon().id) == Some(layout.id.as_str()) {
            painter.text(
                egui::pos2(r.right() - 8.0, text_y),
                Align2::RIGHT_CENTER,
                "in use",
                FontId::proportional(12.0),
                visuals.weak_text_color(),
            );
        }
        let on_name = response.interact_pointer_pos().is_some_and(|p| name_strip.contains(p));
        if response.double_clicked() && on_name {
            self.start_rename(i);
        } else if response.clicked() && !selected {
            self.request(Goto { monitor: self.monitor, layout: i });
        }
        response.context_menu(|ui| self.card_menu(ui, i));
    }

    fn card_menu(&mut self, ui: &mut egui::Ui, i: usize) {
        ui.set_min_width(160.0);
        if ui.button("Rename").clicked() {
            self.start_rename(i);
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            self.duplicate_layout(i);
            ui.close();
        }
        let last = self.doc.layouts.layouts.len() - 1;
        if ui.add_enabled(i > 0, egui::Button::new("Move up")).clicked() {
            self.move_layout(i, true);
            ui.close();
        }
        if ui.add_enabled(i < last, egui::Button::new("Move down")).clicked() {
            self.move_layout(i, false);
            ui.close();
        }
        ui.menu_button("Use on", |ui| {
            let id = self.doc.layouts.layouts[i].id.clone();
            for n in 0..self.monitors.len() {
                let on = self.doc.applied.layout_for(&self.monitors[n].id) == Some(id.as_str());
                let text = format!("{}  {}", if on { "✔" } else { "   " }, self.monitors[n].label(n));
                if ui.button(text).clicked() {
                    self.use_on(i, n);
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button("Delete…").clicked() {
            self.dialog = Some(Dialog::DeleteLayout(i));
            ui.close();
        }
    }

    pub(crate) fn start_rename(&mut self, i: usize) {
        let name = self.doc.layouts.layouts[i].name.clone();
        self.renaming = Some((i, name));
        self.rename_focus = true;
    }

    /// The name box while renaming: Enter or clicking away renames, Esc doesn't.
    fn rename_box(&mut self, ui: &mut egui::Ui, i: usize, rect: egui::Rect) {
        let Some((_, mut text)) = self.renaming.take() else { return };
        let id = egui::Id::new(("rename", i));
        let edit = egui::TextEdit::singleline(&mut text).id(id).font(FontId::proportional(14.0));
        let response = ui.put(rect, edit);
        if std::mem::take(&mut self.rename_focus) {
            // Just started: type straight away, with the whole name selected.
            response.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(text.chars().count()),
                );
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), id);
            }
        }
        if response.lost_focus() {
            if !ui.input(|i| i.key_pressed(Key::Escape)) {
                self.rename_layout(i, &text);
            }
        } else {
            self.renaming = Some((i, text));
        }
    }
}

/// The gaming zone's size on a card, with its ratio under it when there's room.
fn gaming_label(painter: &egui::Painter, r: egui::Rect, w: i32, h: i32) {
    let font = style::semibold(11.0);
    let colour = Color32::from_rgb(0xF0, 0xF2, 0xF6);
    let size = painter.layout_no_wrap(format!("{w}×{h}"), font.clone(), colour);
    let ratio =
        painter.layout_no_wrap(ratio_name(w, h), FontId::proportional(10.0), Color32::from_rgb(0xC8, 0xE8, 0xD4));
    let (sw, sh) = (size.size().x + 6.0, size.size().y);
    if sw > r.width() || sh + 4.0 > r.height() {
        return;
    }
    let both = sh + ratio.size().y + 6.0 <= r.height() && ratio.size().x + 6.0 <= r.width();
    let top = if both { r.center().y - (sh + ratio.size().y) / 2.0 } else { r.center().y - sh / 2.0 };
    painter.galley(egui::pos2(r.center().x - size.size().x / 2.0, top), size, colour);
    if both {
        let x = r.center().x - ratio.size().x / 2.0;
        painter.galley(egui::pos2(x, top + sh), ratio, colour);
    }
}

#[cfg(test)]
mod tests {
    use super::ratio_name;

    #[test]
    fn ratio_names() {
        assert_eq!(ratio_name(1920, 1080), "16:9");
        assert_eq!(ratio_name(3392, 1908), "16:9");
        assert_eq!(ratio_name(1920, 1200), "16:10");
        assert_eq!(ratio_name(2560, 1080), "21:9");
        assert_eq!(ratio_name(3440, 1440), "21:9");
        assert_eq!(ratio_name(1024, 768), "4:3");
        assert_eq!(ratio_name(1000, 999), "1.00:1");
    }
}
