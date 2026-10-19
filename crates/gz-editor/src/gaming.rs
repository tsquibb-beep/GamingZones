//! The gaming zone panel: making a zone the gaming zone (and back), size (pixels, aspect
//! ratio or integer scale, with presets), position, and "may cover the taskbar".

use eframe::egui::{self, RichText};
use gz_core::edit;
use gz_core::gaming::{gcd, largest_aspect_step, Align, GamingSize, GamingZone, Position};

use crate::app::{Dialog, Editor, Selection};
use crate::capture;
use crate::style;

const PIXEL_PRESETS: [(u32, u32); 5] = [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)];
const ASPECT_PRESETS: [(u32, u32); 4] = [(4, 3), (16, 9), (16, 10), (21, 9)];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Pixels,
    Aspect,
    IntegerScale,
}

fn mode(size: &GamingSize) -> Mode {
    match size {
        GamingSize::Pixels { .. } => Mode::Pixels,
        GamingSize::Aspect { .. } => Mode::Aspect,
        GamingSize::IntegerScale { .. } => Mode::IntegerScale,
    }
}

/// The same size, described another way.
fn convert(size: GamingSize, to: Mode) -> GamingSize {
    let Ok((w, h)) = size.pixels() else { return size };
    let (w, h) = (w as u32, h as u32);
    match to {
        Mode::Pixels => GamingSize::Pixels { w, h },
        Mode::Aspect => {
            let g = gcd(w, h).max(1);
            GamingSize::Aspect { ratio_w: w / g, ratio_h: h / g, k: g }
        }
        Mode::IntegerScale => GamingSize::IntegerScale { base_w: w, base_h: h, scale: 1 },
    }
}

/// A size box: typed values apply on Enter, not on every keystroke.
fn number(ui: &mut egui::Ui, value: &mut u32, max: u32) -> bool {
    let drag = egui::DragValue::new(value).range(1..=max).update_while_editing(false);
    ui.add_sized([style::PAIR_W, style::ROW_H], drag).changed()
}

impl Editor {
    pub fn set_gaming_size(&mut self, w: u32, h: u32) {
        let Some(mut new) = self.current().gaming_zones.first().cloned() else { return };
        new.size = GamingSize::Pixels { w, h };
        self.apply_gaming(new);
    }

    /// Changes the gaming zone's size (kept centred where it is) or how it's described. The
    /// dividers around it stay put unless it grows past them.
    fn apply_gaming(&mut self, mut new: GamingZone) {
        let (work, monitor) = self.areas();
        let Some(old) = self.current().gaming_rects(work, monitor).ok().and_then(|r| r.first().copied()) else {
            return;
        };
        if let Ok((w, h)) = new.size.pixels() {
            if let Position::Px(_) = new.x {
                new.x = Position::Px(old.left + (old.width() - w) / 2 - work.left);
            }
            if let Position::Px(_) = new.y {
                new.y = Position::Px(old.top + (old.height() - h) / 2 - work.top);
            }
        }
        self.edit_layout("gaming", |l, w, m| edit::set_gaming_zone(l, new, w, m));
    }

    /// The gaming zone's properties (shown while it's selected).
    pub fn gaming_panel(&mut self, ui: &mut egui::Ui) {
        let (work, monitor) = self.areas();
        let Some(gz) = self.current().gaming_zones.first().cloned() else {
            self.selection = Selection::None;
            return;
        };
        ui.heading("Gaming zone");
        let over_taskbar = self.current().gaming_over_taskbar;
        let keep_on_top = self.current().gaming_keep_on_top;
        let bounds = self.current().gaming_bounds(work, monitor);
        let space = edit::gaming_space(self.current(), work, monitor).ok().flatten();
        let placed = self.current().gaming_rects(work, monitor).ok().and_then(|r| r.first().copied());
        let mut new = gz.clone();
        let max = gz_core::gaming::MAX_SIZE;

        // Size.
        let mut m = mode(&gz.size);
        ui.horizontal(|ui| {
            ui.radio_value(&mut m, Mode::Pixels, "Pixels");
            ui.radio_value(&mut m, Mode::Aspect, "Aspect ratio");
            ui.radio_value(&mut m, Mode::IntegerScale, "Integer scale");
        });
        if m != mode(&gz.size) {
            new.size = convert(gz.size, m);
        }
        ui.horizontal(|ui| match &mut new.size {
            GamingSize::Pixels { w, h } => {
                number(ui, w, max);
                ui.label("×");
                number(ui, h, max);
            }
            GamingSize::Aspect { ratio_w, ratio_h, k } => {
                number(ui, ratio_w, 1000);
                ui.label(":");
                number(ui, ratio_h, 1000);
                ui.label("step");
                let drag = egui::DragValue::new(k).range(1..=max).update_while_editing(false);
                ui.add_sized([style::PAIR_W, style::ROW_H], drag);
                if ui.small_button("−").clicked() && *k > 1 {
                    *k -= 1;
                }
                if ui.small_button("+").clicked() {
                    *k += 1;
                }
            }
            GamingSize::IntegerScale { base_w, base_h, scale } => {
                number(ui, base_w, max);
                ui.label("×");
                number(ui, base_h, max);
                ui.label("times");
                number(ui, scale, 64);
            }
        });
        match (new.size.pixels(), placed) {
            (Ok((w, h)), Some(p)) if (w, h) != (p.width(), p.height()) => {
                ui.colored_label(
                    egui::Color32::from_rgb(0xFF, 0xB0, 0x20),
                    format!("= {w} × {h}, too big here: shrunk to {} × {}", p.width(), p.height()),
                );
            }
            (Ok((w, h)), _) => {
                ui.label(RichText::new(format!("= {w} × {h}")).strong());
            }
            (Err(e), _) => {
                ui.colored_label(egui::Color32::from_rgb(0xFF, 0x70, 0x60), format!("Not usable: {e}"));
            }
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("gz-presets").selected_text("Presets").width(style::BUTTON_W).show_ui(
                ui,
                |ui| {
                    for (w, h) in PIXEL_PRESETS {
                        if ui.selectable_label(false, format!("{w} × {h}")).clicked() {
                            new.size = GamingSize::Pixels { w, h };
                        }
                    }
                    ui.separator();
                    for (rw, rh) in ASPECT_PRESETS {
                        if ui.selectable_label(false, format!("{rw}:{rh}")).clicked() {
                            // Keep about the current height, within what fits.
                            let g = gcd(rw, rh).max(1);
                            let current_h = placed.map_or(1080, |p| p.height()) as u32;
                            let mut k = (current_h as f64 / (rh / g) as f64).round().max(1.0) as u32;
                            let fits = largest_aspect_step(rw, rh, bounds.width(), bounds.height());
                            if fits > 0 {
                                k = k.min(fits);
                            }
                            new.size = GamingSize::Aspect { ratio_w: rw, ratio_h: rh, k };
                        }
                    }
                },
            );
        });
        if style::wide_button(ui, "Capture size from window…").clicked() {
            self.dialog = Some(Dialog::Capture(capture::windows()));
        }
        if new != gz {
            self.apply_gaming(new);
        }
        if let Some(space) = space {
            ui.weak(format!(
                "Its space: {} × {}. What it doesn't cover is normal zones; the dividers around it move with it.",
                space.width(),
                space.height()
            ));
            if placed.is_some_and(|p| p != space)
                && style::wide_button(ui, "Fill its space")
                    .on_hover_text(format!("Make it {} × {}", space.width(), space.height()))
                    .clicked()
            {
                self.edit_layout("", edit::fill_space);
            }
        }

        // Position: moving it pushes the zones beside it, as dragging it does.
        ui.add_space(4.0);
        if let Some(p) = placed {
            for (label, axis_x) in [("X", true), ("Y", false)] {
                let (pos, current, centre) = if axis_x {
                    (gz.x, p.left - work.left, (work.width() - p.width()).div_euclid(2))
                } else {
                    (gz.y, p.top - work.top, (work.height() - p.height()).div_euclid(2))
                };
                ui.horizontal(|ui| {
                    style::label_cell(ui, label);
                    let mut value = current;
                    let centred = matches!(pos, Position::Align(Align::Centre));
                    let drag = egui::DragValue::new(&mut value).suffix(" px").update_while_editing(false);
                    let changed = ui
                        .add_enabled_ui(!centred, |ui| ui.add_sized([style::NUMBER_W, style::ROW_H], drag))
                        .inner
                        .changed();
                    let mut centre_it = centred;
                    let toggled = ui.checkbox(&mut centre_it, "centred").changed();
                    let target = if toggled && centre_it { centre } else { value };
                    if changed || toggled {
                        let locked = self.locked_indices();
                        let (dx, dy) = if axis_x { (target - current, 0) } else { (0, target - current) };
                        self.edit_layout("gaming-move", |l, w, m| {
                            edit::move_gaming_zone(l, dx, dy, &locked, w, m)?;
                            let g = &mut l.gaming_zones[0];
                            let at = if axis_x { &mut g.x } else { &mut g.y };
                            if centre_it {
                                *at = Position::Align(Align::Centre);
                            } else if let Position::Align(_) = at {
                                *at = Position::Px(target);
                            }
                            Ok(())
                        });
                    }
                });
            }
            let r = p.offset(-work.left, -work.top);
            ui.weak(format!("On this monitor: ({}, {}) to ({}, {})", r.left, r.top, r.right, r.bottom));
        }

        let mut over = over_taskbar;
        if ui
            .checkbox(&mut over, "May cover the taskbar")
            .on_hover_text("Lets it be taller or wider than the work area.")
            .changed()
        {
            self.edit_layout("", |l, _, _| {
                l.gaming_over_taskbar = over;
                l.gaming_keep_on_top &= over;
                Ok(())
            });
        }
        let mut on_top = keep_on_top;
        if ui
            .add_enabled(over_taskbar, egui::Checkbox::new(&mut on_top, "Keep the game on top"))
            .on_hover_text(
                "Otherwise the taskbar draws over the part of the game that covers it. \
                 A game on top also covers other windows that overlap it.",
            )
            .changed()
        {
            self.edit_layout("", |l, _, _| {
                l.gaming_keep_on_top = on_top;
                Ok(())
            });
        }
        ui.add_space(4.0);
        if style::wide_button(ui, "Arrange zones around it")
            .on_hover_text("Replaces this layout's zones with a column either side and a strip above and below.")
            .clicked()
        {
            self.edit_layout("", |l, _, _| edit::arrange_around_gaming_zone(l));
            self.selection = Selection::None;
        }
        let back = if gz.from_zone {
            "Its space becomes one zone again (the zones in it are merged)."
        } else {
            "The zones around it stay where they are."
        };
        if style::wide_button(ui, "Turn it back into a normal zone").on_hover_text(back).clicked() {
            self.edit_layout("", edit::remove_gaming_zone);
            self.selection = Selection::None;
        }
    }
}
