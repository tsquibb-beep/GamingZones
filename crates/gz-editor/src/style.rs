//! The editor's look in one place: the Lexend font (as in Spotify Masher), text sizes, and
//! the fixed widths that keep labels, boxes and buttons lined up.

use std::sync::Arc;

use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak, TextStyle, Vec2};

/// Lexend, a variable font (weight 100–900, 400 by default). SIL Open Font License 1.1.
const LEXEND: &[u8] = include_bytes!("../../../res/fonts/Lexend-VariableFont_wght.ttf");
const SEMIBOLD: &str = "Lexend SemiBold";

/// Width of the label in front of a number box.
pub const LABEL_W: f32 = 110.0;
/// Width of a number box.
pub const NUMBER_W: f32 = 96.0;
/// Width of each of a pair of boxes on one row (width × height).
pub const PAIR_W: f32 = 56.0;
/// Width of an ordinary button.
pub const BUTTON_W: f32 = 150.0;
/// Width of a button in a dialog.
pub const DIALOG_W: f32 = 96.0;
/// Height of a row of controls.
pub const ROW_H: f32 = 24.0;

/// The green of the gaming zone (and the logo), for buttons that make one.
pub const GREEN: Color32 = Color32::from_rgb(0x27, 0xA0, 0x60);
/// Red, for buttons that throw something away.
pub const RED: Color32 = Color32::from_rgb(0xC8, 0x3C, 0x3C);
/// [`RED`] under the mouse, for small red icons that need to stand out on hover.
pub const RED_HOVER: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x5A);

/// Text in Lexend SemiBold.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Sets the font and sizes (the exe and the UI tests both call this).
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("lexend".into(), Arc::new(FontData::from_static(LEXEND)));
    let tweak = FontTweak { coords: VariationCoords::new([("wght", 600.0)]), ..Default::default() };
    fonts.font_data.insert("lexend-600".into(), Arc::new(FontData::from_static(LEXEND).tweak(tweak)));
    // egui's own fonts stay behind Lexend, for symbols it doesn't have.
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.get_mut(&FontFamily::Proportional).expect("default family").insert(0, "lexend".into());
    let mut heavier = vec!["lexend-600".to_string()];
    heavier.extend(fallbacks);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), heavier);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, semibold(17.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Small, FontId::proportional(10.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(8.0, 3.0);
        style.spacing.interact_size.y = ROW_H;
    });
}

/// A button at least `width` wide.
pub fn sized_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    // A stretchy gap either side keeps the text centred in the wider button.
    let atoms = (egui::Atom::grow(), text, egui::Atom::grow());
    ui.add(egui::Button::new(atoms).min_size(Vec2::new(width, ROW_H)))
}

/// A button as wide as the panel, filled with `fill`, with white text.
pub fn wide_coloured_button(ui: &mut egui::Ui, text: &str, fill: Color32) -> egui::Response {
    let text = egui::RichText::new(text).color(Color32::WHITE);
    let atoms = (egui::Atom::grow(), text, egui::Atom::grow());
    ui.add(egui::Button::new(atoms).fill(fill).min_size(Vec2::new(ui.available_width(), ROW_H)))
}

/// A button of the ordinary width.
pub fn button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    sized_button(ui, text, BUTTON_W)
}

/// A button as wide as the panel.
pub fn wide_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    sized_button(ui, text, ui.available_width())
}

/// A left-aligned label in a cell `LABEL_W` wide.
pub fn label_cell(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(LABEL_W, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(LABEL_W);
        ui.label(text)
    })
    .inner
}
