//! The gaming zone panel: on/off, size (pixels, aspect ratio or integer scale, with
//! presets), position and alignment, and "may cover the taskbar".

use eframe::egui::{self, RichText};
use gz_core::edit;
use gz_core::gaming::{gcd, largest_aspect_step, Align, GamingSize, GamingZone, Position};

use crate::app::{Dialog, Editor};
use crate::capture;

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

fn number(ui: &mut egui::Ui, value: &mut u32, max: u32) -> bool {
    ui.add(egui::DragValue::new(value).range(1..=max)).changed()
}

impl Editor {
    pub fn set_gaming_size(&mut self, w: u32, h: u32) {
        self.edit_layout("", |l, _, _| {
            if let Some(gz) = l.gaming_zones.first_mut() {
                gz.size = GamingSize::Pixels { w, h };
            }
            Ok(())
        });
    }

    pub fn gaming_panel(&mut self, ui: &mut egui::Ui) {
        let (work, monitor) = self.areas();
        let mut on = !self.current().gaming_zones.is_empty();
        ui.horizontal(|ui| {
            ui.heading("Gaming zone");
            if ui.checkbox(&mut on, "").changed() {
                if on {
                    self.edit_layout("", |l, _, _| {
                        edit::add_gaming_zone(l, GamingSize::Pixels { w: 1920, h: 1080 });
                        // A fresh layout: put zones round it straight away.
                        if l.zones.len() <= 1 && l.guides.is_empty() {
                            edit::arrange_around_gaming_zone(l)?;
                        }
                        Ok(())
                    });
                } else {
                    self.edit_layout("", edit::remove_gaming_zone);
                }
                self.selection = crate::app::Selection::None;
            }
        });
        let Some(gz) = self.current().gaming_zones.first().copied() else {
            ui.weak("A zone of an exact size (a game's resolution) that the other zones are arranged around.");
            return;
        };
        let over_taskbar = self.current().gaming_over_taskbar;
        let bounds = if over_taskbar { monitor } else { work };
        let placed = self.current().gaming_rects(work, monitor).ok().and_then(|r| r.first().copied());
        let mut new = gz;
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
                number(ui, k, max);
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
            egui::ComboBox::from_id_salt("gz-presets").selected_text("Presets").show_ui(ui, |ui| {
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
            });
        });
        if ui.button("Capture size from window…").clicked() {
            self.dialog = Some(Dialog::Capture(capture::windows()));
        }

        // Position.
        ui.add_space(4.0);
        let size = placed.map_or((0, 0), |p| (p.width(), p.height()));
        let position_row = |ui: &mut egui::Ui, label: &str, pos: &mut Position, current: i32| {
            ui.horizontal(|ui| {
                ui.label(label);
                let mut centred = matches!(pos, Position::Align(Align::Centre));
                match pos {
                    Position::Px(px) => {
                        ui.add(egui::DragValue::new(px).suffix(" px"));
                    }
                    Position::Align(_) => {
                        ui.weak(format!("{current} px"));
                    }
                }
                if ui.checkbox(&mut centred, "centred").changed() {
                    *pos = if centred { Position::Align(Align::Centre) } else { Position::Px(current) };
                }
            });
        };
        let (cx, cy) = placed.map_or((0, 0), |p| (p.left - work.left, p.top - work.top));
        position_row(ui, "X", &mut new.x, cx);
        position_row(ui, "Y", &mut new.y, cy);
        let px = |v: i32| Position::Px(v);
        ui.horizontal(|ui| {
            ui.label("Across");
            if ui.small_button("Left").clicked() {
                new.x = px(bounds.left - work.left);
            }
            if ui.small_button("Centre").clicked() {
                new.x = Position::Align(Align::Centre);
            }
            if ui.small_button("Right").clicked() {
                new.x = px(bounds.right - size.0 - work.left);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Down");
            if ui.small_button("Top").clicked() {
                new.y = px(bounds.top - work.top);
            }
            if ui.small_button("Middle").clicked() {
                new.y = Position::Align(Align::Centre);
            }
            if ui.small_button("Bottom").clicked() {
                new.y = px(bounds.bottom - size.1 - work.top);
            }
        });
        if let Some(p) = placed {
            let r = p.offset(-work.left, -work.top);
            ui.weak(format!("On this monitor: ({}, {}) to ({}, {})", r.left, r.top, r.right, r.bottom));
        }
        if new != gz {
            self.edit_layout("gaming", |l, _, _| {
                l.gaming_zones[0] = GamingZone { ..new };
                Ok(())
            });
        }

        let mut over = over_taskbar;
        if ui
            .checkbox(&mut over, "May cover the taskbar")
            .on_hover_text(
                "Lets it be taller or wider than the work area. The taskbar still draws on top of it for now.",
            )
            .changed()
        {
            self.edit_layout("", |l, _, _| {
                l.gaming_over_taskbar = over;
                Ok(())
            });
        }
        if ui
            .button("Arrange zones around it")
            .on_hover_text("Replaces this layout's zones with a column either side and a strip above and below.")
            .clicked()
        {
            self.edit_layout("", |l, _, _| edit::arrange_around_gaming_zone(l));
            self.selection = crate::app::Selection::None;
        }
    }
}
