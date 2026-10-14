//! The editor window: monitors and toolbar on top, layout cards on the left, the canvas in
//! the middle, properties on the right.

use std::path::PathBuf;

use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Sense, Vec2};
use gz_core::config::{Settings, SETTINGS_FILE};
use gz_core::edit::{self, EditError, GuideKind};
use gz_core::ipc::Preview;
use gz_core::layout::{span, zone_number, Anchor, Axis, Layout, Zone};
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
    /// Zones whose size edits mustn't change: (layout id, the zone). Only while the editor is open.
    locked: Vec<(String, Zone)>,
}

impl Editor {
    pub fn load(dir: PathBuf) -> Editor {
        let files = Files { dir };
        let monitors = monitors::all();
        let (doc, stamps, load_error) = match files.load(&monitors) {
            Ok((doc, stamps)) => (doc, stamps, None),
            Err(e) => {
                (Doc { layouts: LayoutsFile::defaults(), applied: Applied::default() }, Stamps::default(), Some(e))
            }
        };
        let settings = std::fs::read_to_string(files.dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|t| Settings::from_json(&t).ok())
            .unwrap_or_default();
        Editor::new(files, monitors, doc, stamps, load_error, settings.snapping.deadzone_px)
    }

    fn new(
        files: Files,
        monitors: Vec<Monitor>,
        doc: Doc,
        stamps: Stamps,
        mut load_error: Option<String>,
        deadzone: i32,
    ) -> Editor {
        if monitors.is_empty() {
            load_error = Some("No monitors were found.".into());
        }
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
            deadzone,
            preview_on: false,
            last_preview: None,
            own_window: None,
            status: None,
            dialog: None,
            allow_close: false,
            close_after_save: false,
            locked: Vec::new(),
        };
        editor.select_applied_layout();
        editor
    }

    /// An editor on made-up monitors and layouts, for the UI tests (never saves).
    #[cfg(test)]
    pub fn for_test(monitors: Vec<Monitor>, layouts: LayoutsFile, deadzone: i32) -> Editor {
        let applied = Applied { default_layout: layouts.layouts.first().map(|l| l.id.clone()), ..Default::default() };
        let files = Files { dir: std::env::temp_dir().join("gz-editor-test-never-written") };
        Editor::new(files, monitors, Doc { layouts, applied }, Stamps::default(), None, deadzone)
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

    /// The status line: (text, is it an error).
    #[cfg(test)]
    pub fn status(&self) -> Option<&(String, bool)> {
        self.status.as_ref()
    }

    /// Zone `i`'s number as the canvas shows it (empty zones have none).
    pub fn zone_name(&self, i: usize) -> String {
        let (work, monitor) = self.areas();
        let zones = self.current().resolve(work, monitor, 0).unwrap_or_default();
        zone_number(&zones, i).map_or_else(|| format!("{}", i + 1), |n| n.to_string())
    }

    /// An edit error as the editor shows it (zone numbers as on the canvas).
    fn describe(&self, e: &EditError) -> String {
        match e {
            EditError::Locked(i) => {
                format!("Zone {} is locked, and there's nothing on its other side to push it into.", self.zone_name(*i))
            }
            EditError::NoSuchZone(i) => format!("There is no zone {}.", self.zone_name(*i)),
            e => e.to_string(),
        }
    }

    /// Indices of the selected layout's locked zones.
    pub fn locked_indices(&self) -> Vec<usize> {
        let layout = self.current();
        (0..layout.zones.len()).filter(|&i| self.is_locked(i)).collect()
    }

    pub fn is_locked(&self, index: usize) -> bool {
        let layout = self.current();
        layout.zones.get(index).is_some_and(|z| self.locked.iter().any(|(id, l)| *id == layout.id && l == z))
    }

    pub fn set_locked(&mut self, index: usize, lock: bool) {
        let layout = self.current();
        let Some(zone) = layout.zones.get(index).cloned() else { return };
        let id = layout.id.clone();
        self.locked.retain(|(l, z)| !(*l == id && *z == zone));
        if lock {
            self.locked.push((id, zone));
        }
    }

    /// The selected layout's locked zones in `doc`, with their size on this monitor.
    fn locked_sizes(&self, doc: &Doc) -> Vec<(Zone, (i32, i32))> {
        let (work, monitor) = self.areas();
        let Some(layout) = doc.layouts.layouts.get(self.layout) else { return Vec::new() };
        let Ok(zones) = layout.resolve(work, monitor, 0) else { return Vec::new() };
        layout
            .zones
            .iter()
            .zip(&zones)
            .filter(|(z, _)| self.locked.iter().any(|(id, l)| *id == layout.id && l == *z))
            .map(|(z, r)| (z.clone(), (r.rect.width(), r.rect.height())))
            .collect()
    }

    /// Changes the document as one undo step, or as part of the previous step if it has the
    /// same non-empty `key` and came within a second. If the change fails, or leaves a
    /// layout invalid, nothing changes and the error is shown.
    pub fn change<T>(&mut self, key: &str, f: impl FnOnce(&mut Doc) -> Result<T, String>) -> Option<T> {
        let before = self.doc.clone();
        let locked_before = self.locked_sizes(&before);
        let result = f(&mut self.doc).and_then(|v| {
            match self.doc.layouts.layouts.iter().find_map(|l| l.validate().err().map(|e| (l.name.clone(), e))) {
                Some((name, e)) => Err(format!("{name}: {e}")),
                None => Ok(v),
            }
        });
        // Locked zones keep their size, whatever the change (a zone that no longer exists,
        // after a merge or delete, isn't checked).
        let result = result.and_then(|v| {
            let after = self.locked_sizes(&self.doc);
            let layout = &self.doc.layouts.layouts[self.layout.min(self.doc.layouts.layouts.len() - 1)];
            for (zone, size) in &locked_before {
                let now = after.iter().find(|(z, _)| z == zone).map(|(_, s)| *s);
                if now.is_some_and(|now| now != *size) {
                    let i = layout.zones.iter().position(|z| z == zone).unwrap_or(0);
                    let n = self.zone_name(i);
                    return Err(format!("That would resize zone {n}, which is locked."));
                }
            }
            Ok(v)
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
        let mut error = None;
        let result = self.change(key, |d| {
            f(&mut d.layouts.layouts[i], work, monitor).map_err(|e| {
                error = Some(e);
                String::new()
            })
        });
        if let Some(e) = error {
            let text = self.describe(&e);
            self.set_error(text);
        }
        result
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

    /// Moves a divider, within the limits that keep its zones at least [`MIN_ZONE_PX`]. Locked
    /// zones beside it are pushed along.
    pub fn move_divider(&mut self, id: &str, pos: i32) {
        let (work, monitor) = self.areas();
        let Ok(positions) = self.current().guide_positions(work, monitor) else { return };
        let locked = self.locked_indices();
        if positions.get(id).is_some_and(|&(_, at)| at != pos) {
            if let Err(e) = edit::pushed_guides(self.current(), id, &locked) {
                let text = self.describe(&e);
                self.set_error(text);
                return;
            }
        }
        let (lo, hi) = edit::guide_limits(self.current(), &positions, id, &locked, work, MIN_ZONE_PX);
        self.edit_layout(&format!("divider:{id}"), |l, w, m| {
            edit::move_divider(l, id, pos.clamp(lo, hi), &locked, w, m)
        });
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
        let applied = &self.doc.applied;
        // Connected monitors, including those on the default layout (no entry of their own).
        let connected = self
            .monitors
            .iter()
            .enumerate()
            .filter(|(_, m)| applied.layout_for(&m.id) == Some(id))
            .map(|(n, m)| m.label(n));
        let remembered = applied
            .monitors
            .iter()
            .filter(|(mid, e)| e.layout.as_deref() == Some(id) && !self.monitors.iter().any(|m| &m.id == *mid))
            .map(|(mid, e)| format!("{} (not connected)", if e.name.is_empty() { mid } else { &e.name }));
        connected.chain(remembered).collect()
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
            Selection::Zones(v) if v.len() == 1 => self.zone_props(ui, v[0]),
            Selection::Zones(v) if !v.is_empty() => {
                ui.heading(format!("{} zones", v.len()));
                ui.horizontal(|ui| {
                    if ui.button("Merge").clicked() {
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
            Selection::Gaming => {
                ui.weak("The gaming zone: its size and position are below.");
            }
            _ => {
                ui.weak("Select a zone or a divider to change it.");
            }
        }
    }

    /// Makes zone `i` `size` px wide or tall, keeping locked zones as they are.
    pub fn resize_zone(&mut self, i: usize, axis: Axis, size: i32) {
        let locked = self.locked_indices();
        self.edit_layout(&format!("zone-size:{i}"), |l, w, m| edit::resize_zone(l, i, axis, size, &locked, w, m));
    }

    fn zone_props(&mut self, ui: &mut egui::Ui, i: usize) {
        let (work, monitor) = self.areas();
        let Some(r) = self.current().resolve(work, monitor, self.deadzone).ok().and_then(|z| z.get(i).map(|z| z.rect))
        else {
            self.selection = Selection::None;
            return;
        };
        let name = self.zone_name(i);
        let locked = self.is_locked(i);
        ui.horizontal(|ui| {
            ui.heading(format!("Zone {name}"));
            let mut lock = locked;
            if ui
                .toggle_value(&mut lock, "🔒 Lock size")
                .on_hover_text("Keeps its size: moving a divider next to it pushes it along instead.")
                .changed()
            {
                self.set_locked(i, lock);
            }
        });
        ui.add_enabled_ui(!locked, |ui| self.zone_size_boxes(ui, i, r, work));
        if locked {
            ui.weak("Unlock it to change its size.");
        }
        ui.weak(format!("Top-left corner at ({}, {})", r.left - work.left, r.top - work.top));
        ui.horizontal(|ui| {
            if ui.button("Delete").on_hover_text("Delete key").clicked() {
                self.delete_selected_zones();
            }
        });
    }

    /// Zone `i`'s width and height, in px and % of the area.
    fn zone_size_boxes(&mut self, ui: &mut egui::Ui, i: usize, r: Rect, work: Rect) {
        for (axis, label, size, len) in [
            (Axis::Vertical, "width", r.width(), work.width()),
            (Axis::Horizontal, "height", r.height(), work.height()),
        ] {
            let len = len.max(1);
            let mut px = size;
            let title = if axis == Axis::Vertical { "Width" } else { "Height" };
            if number_box(ui, title, &mut px, |d| d.range(MIN_ZONE_PX..=len).suffix(" px")) {
                self.resize_zone(i, axis, px);
            }
            let mut percent = size as f64 * 100.0 / len as f64;
            let pct_label = format!("% of the {label}");
            if number_box(ui, &pct_label, &mut percent, |d| {
                d.range(0.0..=100.0).speed(0.05).max_decimals(3).suffix(" %")
            }) {
                self.resize_zone(i, axis, (len as f64 * percent / 100.0).round() as i32);
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
        if number_box(ui, &format!("From the {edge}"), &mut px, |d| d.range(0..=len).suffix(" px")) {
            self.move_divider(id, start + px);
        }
        let mut percent = (pos - start) as f64 * 100.0 / len as f64;
        let pct_label = format!("% of the {dimension}");
        if number_box(ui, &pct_label, &mut percent, |d| d.range(0.0..=100.0).speed(0.05).max_decimals(3).suffix(" %")) {
            self.move_divider(id, start + (len as f64 * percent / 100.0).round() as i32);
        }
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
            if number_box(ui, "Fraction", &mut f, |d| d.range(0.0..=1.0).speed(0.001).max_decimals(4)) {
                self.edit_layout(&format!("divider:{id}"), |l, _, _| {
                    if let Anchor::Between { fraction, .. } = &mut edit::guide_mut(l, id)?.anchor {
                        *fraction = f;
                    }
                    Ok(())
                });
            }
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

/// A labelled number box. Typed values apply on Enter (or when the box loses focus), not on
/// every keystroke, so typing "79" doesn't first apply "7". Dragging still applies live.
pub fn number_box<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut N,
    setup: impl FnOnce(egui::DragValue<'_>) -> egui::DragValue<'_>,
) -> bool {
    ui.horizontal(|ui| {
        let label = ui.add_sized([110.0, 18.0], egui::Label::new(label));
        let drag = setup(egui::DragValue::new(value).update_while_editing(false));
        ui.add(drag).labelled_by(label.id).changed()
    })
    .inner
}

fn help(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("How to edit").default_open(true).show(ui, |ui| {
        for line in [
            "Click a zone to split it. Hold Shift to split it the other way.",
            "Drag a divider to move it. Click it to type an exact position.",
            "Ctrl+click zones, or drag across them, to select them. Then Merge or Delete.",
            "Select a zone to type its exact width or height. Lock a zone to keep its size: dividers next to it push it along.",
            "Select a zone and make it the gaming zone. Make the gaming zone smaller and the rest of that space becomes zones.",
            "Drag the gaming zone to move it within its space.",
            "Ctrl+Z / Ctrl+Y undo and redo. Ctrl+S saves.",
        ] {
            ui.label(format!("• {line}"));
        }
    });
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let (Some(me), Some(_)) = (self.own_window, &self.last_preview) {
            core_link::preview(me, None);
        }
    }
}

impl Editor {
    /// The whole window (also driven directly by the UI tests).
    pub fn show(&mut self, ui: &mut egui::Ui) {
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
        // Forget locks on zones that no longer exist (split, merged, deleted).
        let layouts = &self.doc.layouts.layouts;
        self.locked.retain(|(id, z)| layouts.iter().any(|l| l.id == *id && l.zones.contains(z)));
    }
}
