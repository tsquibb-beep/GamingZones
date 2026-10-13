//! The editor window: monitors and toolbar on top, layout cards on the left, the canvas in
//! the middle, properties on the right.

use std::path::PathBuf;

use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Sense, Vec2};
use gz_core::config::{Settings, SETTINGS_FILE};
use gz_core::edit::{self, EditError, GuideKind};
use gz_core::ipc::Preview;
use gz_core::layout::{span, Anchor, Axis, Layout};
use gz_core::store::{Applied, LayoutsFile, MonitorEntry};
use gz_core::Rect;
use gz_win::Hwnd;

use crate::canvas::{self, CanvasDrag};
use crate::capture::{self, WindowEntry};
use crate::core_link;
use crate::doc::{Doc, Files, Stamps};
use crate::monitors::{self, Monitor};

/// Smallest zone (px) a split or divider drag may leave.
pub const MIN_ZONE_PX: i32 = 10;

#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    None,
    Zones(Vec<usize>),
    Divider(String),
    Gaming,
}

pub enum Dialog {
    DeleteLayout(usize),
    UnsavedOnClose,
    ChangedOnDisk,
    Capture(Vec<WindowEntry>),
}

pub struct Editor {
    files: Files,
    /// The files couldn't be read: the editor only shows this.
    load_error: Option<String>,
    pub doc: Doc,
    saved: Doc,
    stamps: Stamps,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    /// Key and time of the last change: changes with the same key within a second (one
    /// drag, typing in one box) are one undo step.
    last_change: Option<(String, f64)>,
    now: f64,
    pub monitors: Vec<Monitor>,
    pub monitor: usize,
    pub layout: usize,
    pub selection: Selection,
    pub canvas_drag: Option<CanvasDrag>,
    /// `snapping.deadzone_px` from settings.json: smaller zones are shown greyed.
    pub deadzone: i32,
    preview_on: bool,
    last_preview: Option<String>,
    own_window: Option<Hwnd>,
    status: Option<(String, bool)>,
    pub dialog: Option<Dialog>,
    allow_close: bool,
    close_after_save: bool,
}

impl Editor {
    pub fn load(dir: PathBuf) -> Editor {
        let files = Files { dir };
        let monitors = monitors::all();
        let (doc, stamps, mut load_error) = match files.load(&monitors) {
            Ok((doc, stamps)) => (doc, stamps, None),
            Err(e) => {
                (Doc { layouts: LayoutsFile::defaults(), applied: Applied::default() }, Stamps::default(), Some(e))
            }
        };
        if monitors.is_empty() {
            load_error = Some("No monitors were found.".into());
        }
        let settings = std::fs::read_to_string(files.dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|t| Settings::from_json(&t).ok())
            .unwrap_or_default();
        let monitor = monitors.iter().position(|m| m.info.primary).unwrap_or(0);
        let mut editor = Editor {
            files,
            load_error,
            saved: doc.clone(),
            doc,
            stamps,
            undo: Vec::new(),
            redo: Vec::new(),
            last_change: None,
            now: 0.0,
            monitors,
            monitor,
            layout: 0,
            selection: Selection::None,
            canvas_drag: None,
            deadzone: settings.snapping.deadzone_px,
            preview_on: false,
            last_preview: None,
            own_window: None,
            status: None,
            dialog: None,
            allow_close: false,
            close_after_save: false,
        };
        editor.select_applied_layout();
        editor
    }

    pub fn current(&self) -> &Layout {
        &self.doc.layouts.layouts[self.layout]
    }

    pub fn mon(&self) -> &Monitor {
        &self.monitors[self.monitor]
    }

    /// The monitor being edited: (work area, whole monitor).
    pub fn areas(&self) -> (Rect, Rect) {
        (self.mon().work(), self.mon().rect())
    }

    fn dirty(&self) -> bool {
        self.doc != self.saved
    }

    /// Shows the layout the selected monitor uses.
    fn select_applied_layout(&mut self) {
        let Some(m) = self.monitors.get(self.monitor) else { return };
        if let Some(i) =
            self.doc.applied.layout_for(&m.id).and_then(|id| self.doc.layouts.layouts.iter().position(|l| l.id == id))
        {
            self.layout = i;
        }
        self.selection = Selection::None;
    }

    pub fn set_error(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), true));
    }

    /// Changes the document as one undo step, or as part of the previous step if it has the
    /// same non-empty `key` and came within a second. If the change fails, or leaves a
    /// layout invalid, nothing changes and the error is shown.
    pub fn change<T>(&mut self, key: &str, f: impl FnOnce(&mut Doc) -> Result<T, String>) -> Option<T> {
        let before = self.doc.clone();
        let result = f(&mut self.doc).and_then(|v| {
            match self.doc.layouts.layouts.iter().find_map(|l| l.validate().err().map(|e| (l.name.clone(), e))) {
                Some((name, e)) => Err(format!("{name}: {e}")),
                None => Ok(v),
            }
        });
        match result {
            Ok(v) => {
                self.record(key, before);
                Some(v)
            }
            Err(e) => {
                self.doc = before;
                self.set_error(e);
                None
            }
        }
    }

    /// [`Self::change`] on the selected layout, with the monitor's areas.
    pub fn edit_layout<T>(
        &mut self,
        key: &str,
        f: impl FnOnce(&mut Layout, Rect, Rect) -> Result<T, EditError>,
    ) -> Option<T> {
        let (work, monitor) = self.areas();
        let i = self.layout;
        self.change(key, |d| f(&mut d.layouts.layouts[i], work, monitor).map_err(|e| e.to_string()))
    }

    fn record(&mut self, key: &str, before: Doc) {
        if self.doc == before {
            return;
        }
        let joined = !key.is_empty() && matches!(&self.last_change, Some((k, t)) if k == key && self.now - t < 1.0);
        if !joined {
            self.undo.push(before);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.last_change = Some((key.to_string(), self.now));
        self.status = None;
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, prev));
            self.after_jump();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, next));
            self.after_jump();
        }
    }

    fn after_jump(&mut self) {
        self.last_change = None;
        self.layout = self.layout.min(self.doc.layouts.layouts.len() - 1);
        self.selection = Selection::None;
        self.canvas_drag = None;
    }

    /// Moves a divider, within the limits that keep its zones at least [`MIN_ZONE_PX`].
    pub fn move_divider(&mut self, id: &str, pos: i32) {
        let (work, monitor) = self.areas();
        let Ok(positions) = self.current().guide_positions(work, monitor) else { return };
        let (lo, hi) = edit::guide_limits(self.current(), &positions, id, work, MIN_ZONE_PX);
        self.edit_layout(&format!("divider:{id}"), |l, w, m| edit::move_guide(l, id, pos.clamp(lo, hi), w, m));
    }

    fn delete_selected_zones(&mut self) {
        if let Selection::Zones(v) = self.selection.clone() {
            if self.edit_layout("", |l, _, _| edit::delete_zones(l, &v)).is_some() {
                self.selection = Selection::None;
            }
        }
    }

    fn save(&mut self, overwrite: bool) -> bool {
        if !overwrite && self.files.stamps() != self.stamps {
            self.dialog = Some(Dialog::ChangedOnDisk);
            return false;
        }
        match self.files.save(&self.doc) {
            Ok(stamps) => {
                self.stamps = stamps;
                self.saved = self.doc.clone();
                let text = if core_link::reload() {
                    "Saved. GamingZones is using the new layouts."
                } else {
                    "Saved. GamingZones isn't running; it will use them when it starts."
                };
                self.status = Some((text.into(), false));
                true
            }
            Err(e) => {
                self.set_error(format!("Couldn't save: {e}"));
                false
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // --- layouts ---

    fn new_layout(&mut self) {
        let id = self.doc.layouts.unique_id("New layout");
        if self
            .change("", |d| {
                d.layouts.layouts.push(edit::new_layout(&id, "New layout"));
                Ok(())
            })
            .is_some()
        {
            self.layout = self.doc.layouts.layouts.len() - 1;
            self.selection = Selection::None;
        }
    }

    fn duplicate_layout(&mut self) {
        let mut copy = self.current().clone();
        copy.name = format!("{} copy", copy.name);
        copy.id = self.doc.layouts.unique_id(&copy.name);
        let at = self.layout + 1;
        if self
            .change("", |d| {
                d.layouts.layouts.insert(at, copy);
                Ok(())
            })
            .is_some()
        {
            self.layout = at;
            self.selection = Selection::None;
        }
    }

    /// Monitors (connected or remembered in applied.json) whose entry uses layout `id`.
    fn used_on(&self, id: &str) -> Vec<String> {
        self.doc
            .applied
            .monitors
            .iter()
            .filter(|(_, e)| e.layout.as_deref() == Some(id))
            .map(|(mid, e)| {
                let n = self.monitors.iter().position(|m| &m.id == mid);
                match n {
                    Some(n) => self.monitors[n].label(n),
                    None if !e.name.is_empty() => format!("{} (not connected)", e.name),
                    None => format!("{mid} (not connected)"),
                }
            })
            .collect()
    }

    fn delete_layout(&mut self, i: usize) {
        let id = self.doc.layouts.layouts[i].id.clone();
        let used = self.used_on(&id);
        if self.doc.layouts.layouts.len() == 1 {
            self.set_error("There has to be at least one layout.");
            return;
        }
        if !used.is_empty() {
            self.set_error(format!("It's in use on {}. Use another layout there first.", used.join(", ")));
            return;
        }
        self.change("", |d| {
            d.layouts.layouts.remove(i);
            if d.applied.default_layout.as_deref() == Some(&id) {
                d.applied.default_layout = d.layouts.layouts.first().map(|l| l.id.clone());
            }
            Ok(())
        });
        self.layout = self.layout.min(self.doc.layouts.layouts.len() - 1);
        self.selection = Selection::None;
    }

    // --- preview ---

    /// Keeps the tray app's on-screen preview in step with the editor.
    fn sync_preview(&mut self) {
        if self.own_window.is_none() {
            self.own_window = core_link::own_window();
        }
        let Some(me) = self.own_window else { return };
        if !self.preview_on {
            if self.last_preview.take().is_some() {
                core_link::preview(me, None);
            }
            return;
        }
        let selected = match &self.selection {
            Selection::Zones(v) => v.clone(),
            Selection::Gaming => vec![self.current().zones.len()],
            _ => Vec::new(),
        };
        let p = Preview { monitor: self.mon().id.clone(), layout: self.current().clone(), selected };
        let json = serde_json::to_string(&p).expect("preview always serialises");
        if self.last_preview.as_deref() == Some(json.as_str()) {
            return;
        }
        if core_link::preview(me, Some(&p)) {
            self.last_preview = Some(json);
        } else {
            self.preview_on = false;
            self.last_preview = None;
            self.set_error("GamingZones isn't running, so there's nothing to show the preview.");
        }
    }

    // --- UI ---

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let save = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S));
        if save && self.dirty() {
            self.save(false);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (redo_shift, undo, redo, delete, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Y),
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::Escape),
            )
        });
        if undo {
            self.undo();
        }
        if redo || redo_shift {
            self.redo();
        }
        if delete {
            self.delete_selected_zones();
        }
        if escape {
            self.selection = Selection::None;
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::UnsavedOnClose);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Monitor:");
            for n in 0..self.monitors.len() {
                let label = self.monitors[n].label(n);
                if ui.selectable_label(n == self.monitor, label).clicked() && n != self.monitor {
                    self.monitor = n;
                    self.select_applied_layout();
                }
            }
            ui.separator();
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
                self.undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
                self.redo();
            }
            ui.separator();
            ui.toggle_value(&mut self.preview_on, "Preview on screen")
                .on_hover_text("Shows this layout's zones on the monitor itself (GamingZones must be running).");
            ui.separator();
            let dirty = self.dirty();
            if ui.add_enabled(dirty, egui::Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
                self.save(false);
            }
            match &self.status {
                Some((text, true)) => {
                    ui.colored_label(Color32::from_rgb(0xFF, 0x70, 0x60), text);
                }
                Some((text, false)) => {
                    ui.label(text);
                }
                None if dirty => {
                    ui.weak("Unsaved changes");
                }
                None => {}
            }
        });
        ui.add_space(2.0);
    }

    fn layout_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Layouts");
        ui.horizontal(|ui| {
            if ui.button("New").clicked() {
                self.new_layout();
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate_layout();
            }
            if ui.button("Delete").clicked() {
                self.dialog = Some(Dialog::DeleteLayout(self.layout));
            }
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
        let layout = &self.doc.layouts.layouts[i];
        let width = ui.available_width();
        let picture_h = (width - 12.0) * work.height() as f32 / work.width().max(1) as f32;
        let picture_h = picture_h.clamp(30.0, 160.0);
        let (response, painter) = ui.allocate_painter(Vec2::new(width, picture_h + 32.0), Sense::click());
        let r = response.rect;
        let selected = i == self.layout;
        let visuals = ui.visuals();
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
            for z in &zones {
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
        }
        let text_y = r.bottom() - 13.0;
        painter.text(
            egui::pos2(r.left() + 8.0, text_y),
            egui::Align2::LEFT_CENTER,
            &layout.name,
            egui::FontId::proportional(14.0),
            visuals.strong_text_color(),
        );
        if self.doc.applied.layout_for(&self.mon().id) == Some(layout.id.as_str()) {
            painter.text(
                egui::pos2(r.right() - 8.0, text_y),
                egui::Align2::RIGHT_CENTER,
                "in use",
                egui::FontId::proportional(12.0),
                visuals.weak_text_color(),
            );
        }
        if response.clicked() && !selected {
            self.layout = i;
            self.selection = Selection::None;
        }
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.layout_props(ui);
        ui.separator();
        self.selection_props(ui);
        ui.separator();
        self.gaming_panel(ui);
        ui.separator();
        help(ui);
    }

    fn layout_props(&mut self, ui: &mut egui::Ui) {
        ui.heading("Layout");
        let mut name = self.current().name.clone();
        ui.horizontal(|ui| {
            ui.label("Name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).changed() {
                let i = self.layout;
                self.change("name", |d| {
                    d.layouts.layouts[i].name = name;
                    Ok(())
                });
            }
        });
        let id = self.current().id.clone();
        ui.label(RichText::new(format!("id: {id}")).weak())
            .on_hover_text("applied.json refers to the layout by this; renaming doesn't change it.");
        let mon_id = self.mon().id.clone();
        if self.doc.applied.layout_for(&mon_id) == Some(id.as_str()) {
            ui.label("✔ In use on this monitor");
        } else if ui.button("Use on this monitor").clicked() {
            let name = self.mon().name.clone();
            self.change("", |d| {
                d.applied.monitors.insert(mon_id, MonitorEntry { layout: Some(id), name });
                Ok(())
            });
        }
        let used = self.used_on(&self.current().id);
        if !used.is_empty() {
            ui.label(RichText::new(format!("Used on: {}", used.join(", "))).weak());
        }
        let (work, monitor) = self.areas();
        if let Ok(zones) = self.current().resolve(work, monitor, self.deadzone) {
            let disabled = zones.iter().filter(|z| z.disabled && !z.rect.is_empty()).count();
            let count = zones.iter().filter(|z| !z.gaming).count();
            let mut text = format!("{count} zones");
            if disabled > 0 {
                text += &format!(", {disabled} greyed (under the {} px deadzone)", self.deadzone);
            }
            ui.label(text);
        }
    }

    fn selection_props(&mut self, ui: &mut egui::Ui) {
        match self.selection.clone() {
            Selection::Zones(v) if !v.is_empty() => {
                let (work, monitor) = self.areas();
                if v.len() == 1 {
                    ui.heading(format!("Zone {}", v[0] + 1));
                    if let Ok(zones) = self.current().resolve(work, monitor, self.deadzone) {
                        if let Some(z) = zones.get(v[0]) {
                            let r = z.rect.offset(-work.left, -work.top);
                            ui.label(format!("{} × {} at ({}, {})", r.width(), r.height(), r.left, r.top));
                        }
                    }
                } else {
                    ui.heading(format!("{} zones", v.len()));
                }
                ui.horizontal(|ui| {
                    if v.len() > 1 && ui.button("Merge").clicked() {
                        if let Some(i) = self.edit_layout("", |l, w, m| edit::merge(l, &v, w, m)) {
                            self.selection = Selection::Zones(vec![i]);
                        }
                    }
                    if ui.button("Delete").on_hover_text("Delete key").clicked() {
                        self.delete_selected_zones();
                    }
                });
            }
            Selection::Divider(id) => self.divider_props(ui, &id),
            _ => {
                ui.weak("Select zones or a divider to change them.");
            }
        }
    }

    fn divider_props(&mut self, ui: &mut egui::Ui, id: &str) {
        let (work, monitor) = self.areas();
        let layout = self.current();
        let Some(guide) = layout.guides.iter().find(|g| g.id == id).cloned() else {
            self.selection = Selection::None;
            return;
        };
        let Ok(positions) = layout.guide_positions(work, monitor) else { return };
        let pos = positions[id].1;
        let (start, end) = span(work, guide.axis);
        let len = (end - start).max(1);
        let (edge, dimension) = match guide.axis {
            Axis::Vertical => ("left", "width"),
            Axis::Horizontal => ("top", "height"),
        };
        ui.heading("Divider");
        ui.label(RichText::new(format!("\"{id}\", {}", guide.axis.name())).weak());
        let mut px = pos - start;
        ui.horizontal(|ui| {
            ui.label("Position");
            if ui.add(egui::DragValue::new(&mut px).range(0..=len).suffix(" px")).changed() {
                self.move_divider(id, start + px);
            }
            ui.label(format!("from the {edge}"));
        });
        let mut percent = (pos - start) as f64 * 100.0 / len as f64;
        ui.horizontal(|ui| {
            ui.label("Position");
            let drag = egui::DragValue::new(&mut percent).range(0.0..=100.0).speed(0.05).max_decimals(3).suffix(" %");
            if ui.add(drag).changed() {
                self.move_divider(id, start + (len as f64 * percent / 100.0).round() as i32);
            }
            ui.label(format!("of the {dimension}"));
        });
        let kind = match &guide.anchor {
            Anchor::Px(px) => format!("Fixed: {px} px from the {edge} on every monitor."),
            Anchor::Percent(p) => format!("{p} % of the {dimension} on every monitor."),
            Anchor::Between { a, b, fraction } => {
                format!("A split: {fraction} of the way from \"{a}\" to \"{b}\", so it moves with them.")
            }
            Anchor::Offset { from, px } => format!("{px} px from \"{from}\"."),
        };
        ui.label(kind);
        if let Anchor::Between { fraction, .. } = guide.anchor {
            let mut f = fraction;
            ui.horizontal(|ui| {
                ui.label("Fraction");
                let drag = egui::DragValue::new(&mut f).range(0.0..=1.0).speed(0.001).max_decimals(4);
                if ui.add(drag).changed() {
                    self.edit_layout(&format!("divider:{id}"), |l, _, _| {
                        if let Anchor::Between { fraction, .. } = &mut edit::guide_mut(l, id)?.anchor {
                            *fraction = f;
                        }
                        Ok(())
                    });
                }
            });
        }
        ui.horizontal(|ui| {
            if !matches!(guide.anchor, Anchor::Px(_)) && ui.button("Make it fixed px").clicked() {
                self.edit_layout("", |l, w, m| edit::convert_guide(l, id, GuideKind::Px, w, m));
            }
            if !matches!(guide.anchor, Anchor::Percent(_)) && ui.button("Make it % of the area").clicked() {
                self.edit_layout("", |l, w, m| edit::convert_guide(l, id, GuideKind::Percent, w, m));
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut open = true;
        let modal = |id: &str| egui::Modal::new(Id::new(id));
        match &dialog {
            Dialog::DeleteLayout(i) => {
                let name = self.doc.layouts.layouts.get(*i).map(|l| l.name.clone()).unwrap_or_default();
                let r = modal("delete").show(ctx, |ui| {
                    ui.heading("Delete layout");
                    ui.label(format!("Delete \"{name}\"? You can undo this until you close the editor."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.delete_layout(*i);
                            open = false;
                        }
                        if ui.button("Cancel").clicked() {
                            open = false;
                        }
                    });
                });
                open &= !r.should_close();
            }
            Dialog::UnsavedOnClose => {
                let r = modal("unsaved").show(ctx, |ui| {
                    ui.heading("Unsaved changes");
                    ui.label("Save the layouts before closing?");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            open = false;
                            self.close_after_save = true;
                            if self.save(false) {
                                self.close(ctx);
                            }
                        }
                        if ui.button("Don't save").clicked() {
                            open = false;
                            self.close(ctx);
                        }
                        if ui.button("Cancel").clicked() {
                            open = false;
                        }
                    });
                });
                open &= !r.should_close();
            }
            Dialog::ChangedOnDisk => {
                let r = modal("changed").show(ctx, |ui| {
                    ui.heading("Files changed");
                    ui.label("layouts.json or applied.json was changed outside the editor after it opened.");
                    ui.label("Saving replaces those changes with what's in the editor.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save anyway").clicked() {
                            open = false;
                            if self.save(true) && self.close_after_save {
                                self.close(ctx);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            open = false;
                            self.close_after_save = false;
                        }
                    });
                });
                open &= !r.should_close();
            }
            Dialog::Capture(list) => {
                let mut picked = None;
                let mut refresh = false;
                let r = modal("capture").show(ctx, |ui| {
                    ui.heading("Capture size from window");
                    ui.label("Pick a window: the gaming zone gets its client size (the inside, without the frame).");
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for w in list {
                            let text = format!("{} × {}   {}   ({})", w.width, w.height, w.title, w.exe);
                            if ui.selectable_label(false, text).clicked() {
                                picked = Some((w.width, w.height));
                            }
                        }
                        if list.is_empty() {
                            ui.weak("No windows found.");
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        refresh = ui.button("Refresh").clicked();
                        if ui.button("Cancel").clicked() {
                            open = false;
                        }
                    });
                });
                open &= !r.should_close();
                if let Some((w, h)) = picked {
                    open = false;
                    self.set_gaming_size(w as u32, h as u32);
                }
                if refresh && open {
                    self.dialog = Some(Dialog::Capture(capture::windows()));
                    return;
                }
            }
        }
        if open {
            self.dialog = Some(dialog);
        }
    }

    fn error_screen(&mut self, ui: &mut egui::Ui, error: &str) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(20.0);
            ui.heading("The layouts can't be opened");
            ui.add_space(8.0);
            ui.label(error);
            ui.add_space(8.0);
            ui.label(format!(
                "Fix the file in {} (or delete it to get the built-in layouts back), then open the editor again.",
                self.files.dir.display()
            ));
            ui.add_space(12.0);
            if ui.button("Close").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

fn help(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("How to edit").default_open(true).show(ui, |ui| {
        for line in [
            "Click a zone to split it. Hold Shift to split it the other way.",
            "Drag a divider to move it. Click it to type an exact position.",
            "Ctrl+click zones, or drag across them, to select them. Then Merge or Delete.",
            "Drag the gaming zone to move it.",
            "Ctrl+Z / Ctrl+Y undo and redo. Ctrl+S saves.",
        ] {
            ui.label(format!("• {line}"));
        }
    });
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.now = ctx.input(|i| i.time);
        if let Some(error) = self.load_error.clone() {
            self.error_screen(ui, &error);
            return;
        }
        self.handle_close(&ctx);
        self.shortcuts(&ctx);
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("layouts").resizable(false).exact_size(240.0).show(ui, |ui| self.layout_list(ui));
        egui::Panel::right("properties").resizable(false).exact_size(360.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.properties(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));
        self.dialogs(&ctx);
        self.sync_preview();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let (Some(me), Some(_)) = (self.own_window, &self.last_preview) {
            core_link::preview(me, None);
        }
    }
}
