//! The editor window: monitors and toolbar on top, layout cards on the left, the canvas in
//! the middle, properties on the right.

use std::path::PathBuf;

use eframe::egui::{self, Id, Key, Modifiers, RichText, Sense, Vec2};
use gz_core::config::{Settings, SETTINGS_FILE};
use gz_core::edit::{self, EditError, GuideKind};
use gz_core::ipc::Preview;
use gz_core::layout::{span, zone_number, Anchor, Axis, Layout, Zone};
use gz_core::store::{Applied, LayoutsFile, MonitorEntry};
use gz_core::Rect;
use gz_win::Hwnd;

use crate::canvas::CanvasDrag;
use crate::capture::{self, WindowEntry};
use crate::core_link;
use crate::doc::{Doc, Files, Stamps};
use crate::icons::{self, Icon};
use crate::monitors::{self, Monitor};
use crate::style;

/// Smallest zone (px) a split or divider drag may leave.
pub const MIN_ZONE_PX: i32 = edit::MIN_ZONE_PX;

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
    /// The open layout has unsaved changes and Tom asked to show another one.
    UnsavedOnSwitch(Goto),
    /// The files changed on disk. `edits`: the open layout's unsaved changes are part of
    /// this save (Save), not only a layout-list change.
    ChangedOnDisk {
        edits: bool,
    },
    Capture(Vec<WindowEntry>),
}

/// Which monitor and layout to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Goto {
    pub monitor: usize,
    pub layout: usize,
}

pub struct Editor {
    pub(crate) files: Files,
    /// The files couldn't be read: the editor only shows this.
    load_error: Option<String>,
    pub doc: Doc,
    /// What's in the files. It only differs from `doc` in the open layout: layout-list
    /// changes (new, rename, delete…) go to both and are written at once.
    saved: Doc,
    stamps: Stamps,
    /// `saved` couldn't be written (an error, or Cancel on "Files changed").
    unwritten: bool,
    /// Earlier versions of the open layout. Cleared when another layout is opened.
    undo: Vec<Layout>,
    redo: Vec<Layout>,
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
    pub(crate) preview_on: bool,
    last_preview: Option<String>,
    own_window: Option<Hwnd>,
    pub(crate) status: Option<(String, bool)>,
    pub dialog: Option<Dialog>,
    allow_close: bool,
    close_after_save: bool,
    /// Zones whose size edits mustn't change: (layout id, the zone). Only while the editor is open.
    locked: Vec<(String, Zone)>,
    /// The layout whose name is being edited in its card, and the text so far.
    pub(crate) renaming: Option<(usize, String)>,
    /// Renaming just started: focus the name box.
    pub(crate) rename_focus: bool,
    /// Where the canvas drew the monitor last frame (for the UI tests).
    pub(crate) canvas_monitor: egui::Rect,
    /// Tell the tray app to reload after writing (off in the UI tests).
    notify_tray: bool,
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
            unwritten: false,
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
            renaming: None,
            rename_focus: false,
            canvas_monitor: egui::Rect::NOTHING,
            notify_tray: true,
        };
        if let Some(i) = editor.applied_layout(monitor) {
            editor.layout = i;
        }
        editor
    }

    /// An editor on made-up monitors and layouts, for the UI tests. It writes to its own
    /// new temp folder, never to the real files, and doesn't tell the tray app.
    #[cfg(test)]
    pub fn for_test(monitors: Vec<Monitor>, layouts: LayoutsFile, deadzone: i32) -> Editor {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let applied = Applied { default_layout: layouts.layouts.first().map(|l| l.id.clone()), ..Default::default() };
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gz-editor-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = Files { dir };
        let mut editor = Editor::new(files, monitors, Doc { layouts, applied }, Stamps::default(), None, deadzone);
        editor.notify_tray = false;
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

    /// The open layout has changes that aren't in the files.
    pub fn dirty(&self) -> bool {
        self.unwritten || self.doc != self.saved
    }

    /// The layout monitor `n` uses.
    fn applied_layout(&self, n: usize) -> Option<usize> {
        let m = self.monitors.get(n)?;
        let id = self.doc.applied.layout_for(&m.id)?;
        self.doc.layouts.layouts.iter().position(|l| l.id == id)
    }

    /// Shows another monitor or layout, asking first if the open layout has unsaved changes.
    pub fn request(&mut self, to: Goto) {
        if to.layout != self.layout && self.dirty() {
            self.dialog = Some(Dialog::UnsavedOnSwitch(to));
        } else {
            self.go(to);
        }
    }

    /// Shows monitor `n`, with the layout it uses.
    pub fn request_monitor(&mut self, n: usize) {
        let layout = self.applied_layout(n).unwrap_or(self.layout);
        self.request(Goto { monitor: n, layout });
    }

    pub fn go(&mut self, to: Goto) {
        self.monitor = to.monitor.min(self.monitors.len().saturating_sub(1));
        if to.layout != self.layout {
            self.layout = to.layout.min(self.doc.layouts.layouts.len() - 1);
            self.forget_history();
        }
        self.selection = Selection::None;
        self.canvas_drag = None;
    }

    fn forget_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_change = None;
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
        zone_name_in(self.current(), i, self.areas())
    }

    /// An edit error as the editor shows it (zone numbers as on the canvas).
    fn describe(&self, e: &EditError) -> String {
        match e {
            EditError::Locked(i) => {
                format!("Zone {} is locked, and there's nothing on its other side to push it into.", self.zone_name(*i))
            }
            EditError::NoSuchZone(i) => format!("There is no zone {}.", self.zone_name(*i)),
            e => format!("{}.", capitalise(&e.to_string())),
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

    /// `layout`'s locked zones, with their size on this monitor.
    fn locked_sizes(&self, layout: &Layout) -> Vec<(Zone, (i32, i32))> {
        let (work, monitor) = self.areas();
        let Ok(zones) = layout.resolve(work, monitor, 0) else { return Vec::new() };
        layout
            .zones
            .iter()
            .zip(&zones)
            .filter(|(z, _)| self.locked.iter().any(|(id, l)| *id == layout.id && l == *z))
            .map(|(z, r)| (z.clone(), (r.rect.width(), r.rect.height())))
            .collect()
    }

    /// Changes the open layout as one undo step, or as part of the previous step if it has
    /// the same non-empty `key` and came within a second. If the change fails, or leaves the
    /// layout invalid, nothing changes and the error is shown.
    pub fn edit_layout<T>(
        &mut self,
        key: &str,
        f: impl FnOnce(&mut Layout, Rect, Rect) -> Result<T, EditError>,
    ) -> Option<T> {
        let (work, monitor) = self.areas();
        let before = self.current().clone();
        let mut layout = before.clone();
        let result = f(&mut layout, work, monitor).map_err(|e| self.describe(&e));
        let result = result.and_then(|v| self.check(&before, &mut layout).map(|()| v));
        match result {
            Ok(v) => {
                let i = self.layout;
                self.doc.layouts.layouts[i] = layout;
                self.record(key, before);
                Some(v)
            }
            Err(e) => {
                self.set_error(e);
                None
            }
        }
    }

    /// Whether `after` (the open layout, changed from `before`) may replace it. Dividers
    /// that only meet by chance (from older edits) are joined once the layout is touched.
    fn check(&self, before: &Layout, after: &mut Layout) -> Result<(), String> {
        if before == after {
            return Ok(());
        }
        let (work, monitor) = self.areas();
        edit::join_coincident(after, work, monitor);
        after.validate().map_err(|e| format!("{}: {e}", after.name))?;
        // Locked zones keep their size, whatever the change (a zone that no longer exists,
        // after a merge or delete, isn't checked).
        let now = self.locked_sizes(after);
        for (zone, size) in &self.locked_sizes(before) {
            if now.iter().find(|(z, _)| z == zone).is_some_and(|(_, s)| s != size) {
                let i = after.zones.iter().position(|z| z == zone).unwrap_or(0);
                let n = zone_name_in(after, i, (work, monitor));
                return Err(format!("That would resize zone {n}, which is locked."));
            }
        }
        // Nor may a change leave zones that only meet by chance (they'd come apart when a
        // divider or the gaming zone moves).
        let whole = |l: &Layout| edit::stays_whole(l, work, monitor);
        if whole(before) && !whole(after) {
            return Err(format!("{}.", capitalise(&EditError::ComesApart.to_string())));
        }
        Ok(())
    }

    fn record(&mut self, key: &str, before: Layout) {
        if *self.current() == before {
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

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let i = self.layout;
            self.redo.push(std::mem::replace(&mut self.doc.layouts.layouts[i], prev));
            self.after_jump();
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let i = self.layout;
            self.undo.push(std::mem::replace(&mut self.doc.layouts.layouts[i], next));
            self.after_jump();
        }
    }

    fn after_jump(&mut self) {
        self.last_change = None;
        self.selection = Selection::None;
        self.canvas_drag = None;
    }

    /// Moves a divider as far towards `pos` as it can go (see [`edit::move_divider`]): locked
    /// zones and the gaming zone beside it are pushed along.
    pub fn move_divider(&mut self, id: &str, pos: i32) {
        let locked = self.locked_indices();
        self.edit_layout(&format!("divider:{id}"), |l, w, m| edit::move_divider(l, id, pos, &locked, w, m));
    }

    fn delete_selected_zones(&mut self) {
        if let Selection::Zones(v) = self.selection.clone() {
            if self.edit_layout("", |l, _, _| edit::delete_zones(l, &v)).is_some() {
                self.selection = Selection::None;
            }
        }
    }

    // --- saving ---

    /// Saves the open layout's changes.
    pub(crate) fn save(&mut self, overwrite: bool) -> bool {
        let i = self.layout;
        let previous = self.saved.layouts.layouts[i].clone();
        self.saved.layouts.layouts[i] = self.doc.layouts.layouts[i].clone();
        if self.write(overwrite, true) {
            return true;
        }
        // Not written: the changes are still unsaved.
        self.saved.layouts.layouts[i] = previous;
        false
    }

    /// Throws away the open layout's unsaved changes.
    pub(crate) fn discard(&mut self) {
        let i = self.layout;
        self.doc.layouts.layouts[i] = self.saved.layouts.layouts[i].clone();
        self.forget_history();
        self.after_jump();
        self.status = None;
    }

    /// Makes a layout-list change (`f` runs on both the editor's and the saved copy) and
    /// writes it straight away. The open layout's unsaved changes stay unsaved.
    pub(crate) fn commit(&mut self, f: impl Fn(&mut Doc)) -> bool {
        let (mut doc, mut saved) = (self.doc.clone(), self.saved.clone());
        f(&mut doc);
        f(&mut saved);
        if let Some((name, e)) =
            saved.layouts.layouts.iter().find_map(|l| l.validate().err().map(|e| (l.name.clone(), e)))
        {
            self.set_error(format!("{name}: {e}"));
            return false;
        }
        (self.doc, self.saved) = (doc, saved);
        self.write(false, false)
    }

    /// Writes `saved` to the files, unless they changed on disk (then it asks first).
    fn write(&mut self, overwrite: bool, edits: bool) -> bool {
        if !overwrite && self.files.stamps() != self.stamps {
            self.unwritten |= !edits;
            self.dialog = Some(Dialog::ChangedOnDisk { edits });
            return false;
        }
        match self.files.save(&self.saved) {
            Ok(stamps) => {
                self.stamps = stamps;
                self.unwritten = false;
                let text = if !self.notify_tray {
                    "Saved."
                } else if core_link::reload() {
                    "Saved. GamingZones is using the new layouts."
                } else {
                    "Saved. GamingZones isn't running; it will use them when it starts."
                };
                self.status = Some((text.into(), false));
                true
            }
            Err(e) => {
                self.unwritten |= !edits;
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

    pub(crate) fn new_layout(&mut self) {
        let id = self.doc.layouts.unique_id("New layout");
        if self.commit(|d| d.layouts.layouts.push(edit::new_layout(&id, "New layout"))) {
            let layout = self.doc.layouts.layouts.len() - 1;
            self.request(Goto { monitor: self.monitor, layout });
        }
    }

    /// Copies layout `i` as it is saved, and opens the copy.
    pub(crate) fn duplicate_layout(&mut self, i: usize) {
        let mut copy = self.saved.layouts.layouts[i].clone();
        copy.name = format!("{} copy", copy.name);
        copy.id = self.doc.layouts.unique_id(&copy.name);
        let at = i + 1;
        if self.commit(|d| d.layouts.layouts.insert(at, copy.clone())) {
            if self.layout >= at {
                self.layout += 1;
            }
            self.request(Goto { monitor: self.monitor, layout: at });
        }
    }

    pub(crate) fn rename_layout(&mut self, i: usize, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() || self.doc.layouts.layouts.get(i).is_none_or(|l| l.name == name) {
            return;
        }
        if self.commit(|d| d.layouts.layouts[i].name = name.clone()) && i == self.layout {
            // Undo mustn't bring the old name back.
            for l in self.undo.iter_mut().chain(&mut self.redo) {
                l.name = name.clone();
            }
        }
    }

    /// Puts monitor `n` on layout `i`.
    pub(crate) fn use_on(&mut self, i: usize, n: usize) {
        let id = self.doc.layouts.layouts[i].id.clone();
        let m = &self.monitors[n];
        let (mon_id, name) = (m.id.clone(), m.name.clone());
        if self.doc.applied.layout_for(&mon_id) == Some(id.as_str()) {
            return;
        }
        self.commit(|d| {
            d.applied.monitors.insert(mon_id.clone(), MonitorEntry { layout: Some(id.clone()), name: name.clone() });
        });
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
        let ok = self.commit(|d| {
            d.layouts.layouts.remove(i);
            if d.applied.default_layout.as_deref() == Some(&id) {
                d.applied.default_layout = d.layouts.layouts.first().map(|l| l.id.clone());
            }
        });
        if !ok {
            return;
        }
        if i < self.layout {
            self.layout -= 1;
        } else if i == self.layout {
            self.layout = self.layout.min(self.doc.layouts.layouts.len() - 1);
            self.forget_history();
            self.selection = Selection::None;
        }
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
        if save && self.dirty() && self.dialog.is_none() {
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
        ui.horizontal(|ui| {
            for n in 0..self.monitors.len() {
                let (caption, name) = (self.monitors[n].short_label(), self.monitors[n].label(n));
                let selected = n == self.monitor;
                if icons::button(ui, Icon::Monitor(n + 1), 30.0, Some(&caption), &name, selected, true).clicked()
                    && !selected
                {
                    self.request_monitor(n);
                }
            }
            ui.separator();
            if icons::button(ui, Icon::Undo, 24.0, None, "Undo", false, self.can_undo())
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                self.undo();
            }
            if icons::button(ui, Icon::Redo, 24.0, None, "Redo", false, self.can_redo())
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            if icons::button(ui, Icon::Eye, 26.0, Some("Preview"), "Preview", self.preview_on, true)
                .on_hover_text("Shows this layout's zones on the monitor itself (GamingZones must be running).")
                .clicked()
            {
                self.preview_on = !self.preview_on;
            }
        });
        ui.add_space(4.0);
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Properties");
        ui.add_space(4.0);
        self.layout_props(ui);
        ui.separator();
        self.selection_props(ui);
        ui.separator();
        help(ui);
    }

    fn layout_props(&mut self, ui: &mut egui::Ui) {
        let layout = self.current();
        let (name, id) = (layout.name.clone(), layout.id.clone());
        ui.label(RichText::new(&name).font(style::semibold(15.0)))
            .on_hover_text(format!("id: {id}\napplied.json refers to the layout by this; renaming doesn't change it."));
        let (work, monitor) = self.areas();
        if let Ok(zones) = layout.resolve(work, monitor, 0) {
            let n = (0..layout.zones.len()).filter(|&i| zone_number(&zones, i).is_some()).count();
            ui.weak(if n == 1 { "1 zone".to_string() } else { format!("{n} zones") });
        }
        ui.add_space(6.0);
        let used = self.used_on(&id);
        if !used.is_empty() {
            ui.label("Used on:");
            for text in used {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Monitor(0), ui.visuals().text_color());
                    ui.label(text);
                });
            }
        }
        if self.doc.applied.layout_for(&self.mon().id) != Some(id.as_str())
            && style::button(ui, "Use on this monitor").clicked()
        {
            self.use_on(self.layout, self.monitor);
        }
    }

    fn selection_props(&mut self, ui: &mut egui::Ui) {
        match self.selection.clone() {
            Selection::Zones(v) if v.len() == 1 => self.zone_props(ui, v[0]),
            Selection::Zones(v) if !v.is_empty() => {
                ui.heading(format!("{} zones", v.len()));
                ui.horizontal(|ui| {
                    if style::button(ui, "Merge").clicked() {
                        if let Some(i) = self.edit_layout("", |l, w, m| edit::merge(l, &v, w, m)) {
                            self.selection = Selection::Zones(vec![i]);
                        }
                    }
                    if style::button(ui, "Delete").on_hover_text("Delete key").clicked() {
                        self.delete_selected_zones();
                    }
                });
            }
            Selection::Divider(id) => self.divider_props(ui, &id),
            Selection::Gaming => self.gaming_panel(ui),
            _ => {
                ui.weak("Select a zone, a divider or the gaming zone to change it.");
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
            let tip = if locked {
                "Locked: it keeps its size, and dividers next to it push it along. Click to unlock."
            } else {
                "Lock its size: dividers next to it push it along instead of resizing it."
            };
            if icons::button(ui, Icon::Lock(locked), 18.0, None, "Lock", locked, true).on_hover_text(tip).clicked() {
                self.set_locked(i, !locked);
            }
        });
        ui.add_enabled_ui(!locked, |ui| self.zone_size_boxes(ui, i, r, work));
        if locked {
            ui.weak("Unlock it to change its size.");
        }
        ui.weak(format!("Top-left corner at ({}, {})", r.left - work.left, r.top - work.top));
        if style::button(ui, "Delete").on_hover_text("Delete key").clicked() {
            self.delete_selected_zones();
        }
        if self.current().gaming_zones.is_empty() {
            ui.add_space(4.0);
            if style::wide_button(ui, &format!("Make zone {name} the gaming zone"))
                .on_hover_text(
                    "A zone of an exact size (a game's resolution). It takes this zone's space; make it smaller \
                     and the rest of that space becomes normal zones.",
                )
                .clicked()
                && self.edit_layout("", |l, w, m| edit::make_gaming_zone(l, i, w, m)).is_some()
            {
                self.selection = Selection::Gaming;
            }
        }
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
            if !matches!(guide.anchor, Anchor::Px(_)) && style::button(ui, "Make it fixed px").clicked() {
                self.edit_layout("", |l, w, m| edit::convert_guide(l, id, GuideKind::Px, w, m));
            }
            if !matches!(guide.anchor, Anchor::Percent(_)) && style::button(ui, "Make it % of the area").clicked() {
                self.edit_layout("", |l, w, m| edit::convert_guide(l, id, GuideKind::Percent, w, m));
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut open = true;
        let modal = |id: &str| egui::Modal::new(Id::new(id));
        let button = |ui: &mut egui::Ui, text: &str| style::sized_button(ui, text, style::DIALOG_W).clicked();
        match &dialog {
            Dialog::DeleteLayout(i) => {
                let name = self.doc.layouts.layouts.get(*i).map(|l| l.name.clone()).unwrap_or_default();
                let r = modal("delete").show(ctx, |ui| {
                    ui.heading("Delete layout");
                    ui.label(format!("Delete \"{name}\"? This can't be undone."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if button(ui, "Delete") {
                            self.delete_layout(*i);
                            open = false;
                        }
                        if button(ui, "Cancel") {
                            open = false;
                        }
                    });
                });
                open &= !r.should_close();
            }
            Dialog::UnsavedOnClose => {
                let r = modal("unsaved").show(ctx, |ui| {
                    ui.heading("Unsaved changes");
                    ui.label(format!("Save the changes to \"{}\" before closing?", self.current().name));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if button(ui, "Save") {
                            open = false;
                            self.close_after_save = true;
                            if self.save(false) {
                                self.close(ctx);
                            }
                        }
                        if button(ui, "Don't save") {
                            open = false;
                            self.close(ctx);
                        }
                        if button(ui, "Cancel") {
                            open = false;
                        }
                    });
                });
                open &= !r.should_close();
            }
            Dialog::UnsavedOnSwitch(to) => {
                let r = modal("switch").show(ctx, |ui| {
                    ui.heading("Unsaved changes");
                    ui.label(format!("Save the changes to \"{}\"?", self.current().name));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if button(ui, "Save") {
                            open = false;
                            if self.save(false) {
                                self.go(*to);
                            }
                        }
                        if button(ui, "Discard") {
                            open = false;
                            self.discard();
                            self.go(*to);
                        }
                        if button(ui, "Cancel") {
                            open = false;
                        }
                    });
                });
                open &= !r.should_close();
            }
            Dialog::ChangedOnDisk { edits } => {
                let r = modal("changed").show(ctx, |ui| {
                    ui.heading("Files changed");
                    ui.label("layouts.json or applied.json was changed outside the editor after it opened.");
                    ui.label("Saving replaces those changes with what's in the editor.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if style::sized_button(ui, "Save anyway", style::BUTTON_W).clicked() {
                            open = false;
                            let saved = if *edits { self.save(true) } else { self.write(true, false) };
                            if saved && self.close_after_save {
                                self.close(ctx);
                            }
                        }
                        if button(ui, "Cancel") {
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
                        refresh = button(ui, "Refresh");
                        if button(ui, "Cancel") {
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
            if style::button(ui, "Close").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

/// Zone `i`'s number in `layout` as the canvas shows it (empty zones have none).
fn zone_name_in(layout: &Layout, i: usize, (work, monitor): (Rect, Rect)) -> String {
    let zones = layout.resolve(work, monitor, 0).unwrap_or_default();
    zone_number(&zones, i).map_or_else(|| format!("{}", i + 1), |n| n.to_string())
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
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
        let label = style::label_cell(ui, label);
        let drag = setup(egui::DragValue::new(value).update_while_editing(false));
        ui.add_sized([style::NUMBER_W, style::ROW_H], drag).labelled_by(label.id).changed()
    })
    .inner
}

fn help(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("How to edit").default_open(false).show(ui, |ui| {
        for line in [
            "Click a zone to split it. Hold Shift to split it the other way.",
            "Drag a divider to move it. Click it to type an exact position.",
            "Ctrl+click zones, or drag across them, to select them. Then Merge or Delete.",
            "Select a zone to type its exact width or height. Click a zone's padlock to keep its size: dividers next to it push it along.",
            "Select a zone and make it the gaming zone. Make the gaming zone smaller and the rest of that space becomes zones.",
            "Drag the gaming zone to move it: the zones beside it make room.",
            "Right-click a layout to rename, copy or delete it, or to use it on a monitor. Double-click its name to rename it.",
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
        egui::Panel::left("layouts").resizable(false).exact_size(260.0).show(ui, |ui| self.layout_list(ui));
        egui::Panel::right("properties").resizable(false).exact_size(330.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.properties(ui));
        });
        egui::Panel::bottom("footer")
            .resizable(false)
            .exact_size(44.0)
            .show_separator_line(false)
            .show(ui, |ui| self.footer(ui));
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));
        self.dialogs(&ctx);
        self.sync_preview();
        // Forget locks on zones that no longer exist (split, merged, deleted).
        let layouts = &self.doc.layouts.layouts;
        self.locked.retain(|(id, z)| layouts.iter().any(|l| l.id == *id && l.zones.contains(z)));
    }
}
