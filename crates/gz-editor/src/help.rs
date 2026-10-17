//! "How to edit": an overlay opened from the top bar, with the mouse and keyboard controls in
//! a table (keys drawn as pills) and a few tips.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke};

use crate::icons::{self, Icon};
use crate::style;

/// Widths of the tables' two columns, the same in both so they line up.
const WHAT_W: f32 = 250.0;
const DOES_W: f32 = 390.0;

/// What you do (keys in [brackets] are drawn as pills), and what it does.
const MOUSE: &[(&str, &str)] = &[
    ("Click a zone", "Split it. The line under the mouse shows where."),
    ("Drag a divider", "Move it."),
    ("Click a divider", "Select it, to type an exact position."),
    ("[Ctrl] + click zones", "Select several, then Merge or Delete them."),
    ("Drag across zones", "Select the zones in between."),
    ("Click a zone's padlock", "Keep its size: dividers next to it push it along."),
    ("Drag the gaming zone", "Move it. The zones beside it make room."),
    ("Right-click a layout", "Rename, duplicate or delete it, or use it on a monitor."),
    ("Double-click a layout's name", "Rename it."),
];

const KEYS: &[(&str, &str)] = &[
    ("[Shift]", "Tap it to switch which way a click splits"),
    ("[Ctrl] + [Z]", "Undo"),
    ("[Ctrl] + [Y]  or  [Ctrl] + [Shift] + [Z]", "Redo"),
    ("[Ctrl] + [S]", "Save"),
    ("[Delete]", "Delete the selected zones"),
    ("[Esc]", "Clear the selection"),
    ("[Enter]", "Apply a number you've typed"),
];

const TIPS: &[&str] = &[
    "Select a zone to type its exact width or height.",
    "Select a zone and make it the gaming zone. Make the gaming zone smaller and the rest of that space becomes \
     zones.",
];

/// The overlay. Returns false once it's been closed (the X, Esc, or a click outside it).
pub fn show(ctx: &egui::Context) -> bool {
    let mut open = true;
    let modal = egui::Modal::new(egui::Id::new("help")).show(ctx, |ui| {
        ui.set_width(WHAT_W + 24.0 + DOES_W);
        ui.horizontal(|ui| {
            ui.heading("How to edit");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Cross, 14.0, None, "Close", false, true).clicked() {
                    open = false;
                }
            });
        });
        ui.add_space(4.0);
        table(ui, "Mouse", MOUSE);
        ui.add_space(10.0);
        table(ui, "Keyboard", KEYS);
        ui.add_space(10.0);
        for tip in TIPS {
            ui.label(format!("• {tip}"));
        }
    });
    open && !modal.should_close()
}

fn table(ui: &mut egui::Ui, title: &str, rows: &[(&str, &str)]) {
    ui.label(RichText::new(title).font(style::semibold(13.0)));
    ui.add_space(2.0);
    egui::Grid::new(title).num_columns(2).striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
        for (what, does) in rows {
            ui.horizontal(|ui| {
                ui.set_min_width(WHAT_W);
                ui.spacing_mut().item_spacing.x = 4.0;
                keys(ui, what);
            });
            ui.horizontal(|ui| {
                ui.set_min_width(DOES_W);
                ui.label(*does);
            });
            ui.end_row();
        }
    });
}

/// `text` with each [Key] drawn as a pill.
fn keys(ui: &mut egui::Ui, text: &str) {
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('[') {
            Some(0) => {
                let end = rest.find(']').unwrap_or(rest.len());
                pill(ui, &rest[1..end]);
                rest = rest.get(end + 1..).unwrap_or("");
            }
            Some(n) => {
                words(ui, &rest[..n]);
                rest = &rest[n..];
            }
            None => {
                words(ui, rest);
                rest = "";
            }
        }
    }
}

fn words(ui: &mut egui::Ui, text: &str) {
    let text = text.trim();
    if text == "+" || text == "or" {
        ui.weak(text);
    } else if !text.is_empty() {
        ui.label(text);
    }
}

/// A key, drawn like a key cap.
pub fn pill(ui: &mut egui::Ui, key: &str) -> egui::Response {
    let visuals = ui.visuals();
    let (fill, edge) = (visuals.widgets.inactive.bg_fill, visuals.widgets.inactive.bg_fill.gamma_multiply(1.6));
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, edge))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| ui.label(RichText::new(key).font(style::semibold(11.5)).color(Color32::from_gray(230))))
        .response
}
