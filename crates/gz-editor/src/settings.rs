//! The settings window (`gz-editor.exe --settings`): everything in settings.json, plus
//! starting GamingZones with Windows.
//!
//! Changes apply straight away: the file is written and the tray app reloads. A drag (a
//! number, the opacity slider, a colour picker) is written when the mouse button goes up.
//! The tray app's reply says which hotkeys it couldn't register, and they're marked here.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eframe::egui::{self, RichText, Vec2};
use gz_core::config::{HotkeyName, Overlap, SETTINGS_FILE};
use gz_core::{Capture, Captured, Color, DisabledZones, GamingFit, Hotkey, LogLevel, Modifiers, Settings};
use gz_win::hotkey as win_hotkey;
use gz_win::keyhook::CaptureHook;
use gz_win::startup::{self, Elevated};
use gz_win::{inspect, process};

use crate::core_link;
use crate::style;

/// Width of the label column (wider than the editor's: the labels are longer).
const LABEL_W: f32 = 180.0;
/// Width of a hotkey box.
const HOTKEY_W: f32 = 190.0;
/// A capture with no key for this long gives up (seconds).
const CAPTURE_TIMEOUT: f64 = 15.0;

/// How GamingZones starts with Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    Off,
    /// HKCU Run.
    SignIn,
    /// The elevated Scheduled Task.
    Admin,
}

impl Startup {
    fn current() -> Startup {
        if startup::task_exists() {
            Startup::Admin
        } else if startup::run_value().is_some() {
            Startup::SignIn
        } else {
            Startup::Off
        }
    }
}

/// A hotkey box waiting for keys.
struct Capturing {
    name: HotkeyName,
    capture: Capture,
    /// `None` in the UI tests (they feed keys through `fake_keys`).
    hook: Option<CaptureHook>,
    started: f64,
}

pub struct SettingsWindow {
    path: PathBuf,
    pub settings: Settings,
    /// What's in settings.json.
    saved: Settings,
    load_error: Option<String>,
    /// The line at the bottom: (text, is an error).
    pub(crate) status: Option<(String, bool)>,
    /// [`HotkeyName::bit`]s the tray app couldn't register (its last reply).
    pub(crate) failed: u32,
    /// A hotkey change that was refused, shown under its row: (hotkey, why).
    pub(crate) refused: Option<(HotkeyName, String)>,
    capturing: Option<Capturing>,
    /// Quick-switch modifiers to use when it's turned back on.
    last_quick_switch: Modifiers,
    pub(crate) startup: Startup,
    /// A startup change running on another thread (it may wait for a UAC prompt).
    startup_job: Option<mpsc::Receiver<Result<Startup, String>>>,
    new_exclusion: String,
    now: f64,
    /// Off in the UI tests: no tray app, no registry, no hooks.
    live: bool,
    /// UI tests: key presses for a capturing hotkey box.
    pub(crate) fake_keys: Vec<(u16, bool)>,
    /// UI tests: hotkeys "another app" holds.
    pub(crate) fake_taken: Vec<Hotkey>,
}

impl SettingsWindow {
    pub fn load(dir: &Path) -> SettingsWindow {
        let path = dir.join(SETTINGS_FILE);
        let (settings, load_error) = match std::fs::read_to_string(&path) {
            Ok(text) => match Settings::from_json(&text) {
                Ok(s) => (s, None),
                Err(e) => (Settings::default(), Some(format!("{SETTINGS_FILE} has an error: {e}"))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (Settings::default(), Some(format!("{SETTINGS_FILE} can't be read: {e}"))),
        };
        let mut w = SettingsWindow::new(path, settings, load_error, Startup::current(), true);
        // What the running tray app couldn't register, without changing anything.
        if w.load_error.is_none() {
            w.tell_tray(false);
        }
        w
    }

    #[cfg(test)]
    pub fn for_test(settings: Settings) -> SettingsWindow {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gz-settings-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        SettingsWindow::new(dir.join(SETTINGS_FILE), settings, None, Startup::Off, false)
    }

    fn new(path: PathBuf, settings: Settings, load_error: Option<String>, startup: Startup, live: bool) -> Self {
        let last_quick_switch =
            settings.hotkeys.quick_switch.unwrap_or(Modifiers { win: true, ctrl: true, alt: true, shift: false });
        SettingsWindow {
            path,
            saved: settings.clone(),
            settings,
            load_error,
            status: None,
            failed: 0,
            refused: None,
            capturing: None,
            last_quick_switch,
            startup,
            startup_job: None,
            new_exclusion: String::new(),
            now: 0.0,
            live,
            fake_keys: Vec::new(),
            fake_taken: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn is_capturing(&self) -> Option<HotkeyName> {
        self.capturing.as_ref().map(|c| c.name)
    }

    /// Writes settings.json and has the tray app reload it.
    fn save(&mut self) {
        let text = self.settings.to_json() + "\n";
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&self.path, text) {
            Ok(()) => {
                self.saved = self.settings.clone();
                self.tell_tray(true);
            }
            Err(e) => self.status = Some((format!("Couldn't save {}: {e}", self.path.display()), true)),
        }
    }

    /// Asks the tray app to reload, and notes which hotkeys it couldn't register. `saved`:
    /// after a change (not when the window opens).
    fn tell_tray(&mut self, saved: bool) {
        if !self.live {
            self.status = saved.then(|| ("Saved.".into(), false));
            return;
        }
        let reply = core_link::reload_settings();
        self.failed = reply.unwrap_or(0);
        let text = match (reply.is_some(), saved) {
            (true, true) => Some("Saved. GamingZones is using these settings."),
            (true, false) => None,
            (false, true) => Some("Saved. GamingZones isn't running; it will use these when it starts."),
            (false, false) => Some("GamingZones isn't running, so hotkeys can't be checked."),
        };
        self.status = text.map(|t| (t.into(), false));
    }

    /// The whole window (also driven directly by the UI tests).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.now = ctx.input(|i| i.time);
        if let Some(error) = &self.load_error {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.heading("Settings can't be shown");
                ui.add_space(8.0);
                ui.label(error);
                ui.label("Fix the file, or delete it to start again from the defaults.");
            });
            return;
        }
        self.poll_capture(&ctx);
        self.poll_startup_job(&ctx);
        egui::Panel::bottom("status").resizable(false).exact_size(36.0).show(ui, |ui| self.status_line(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(700.0);
                self.hotkeys(ui);
                self.dragging(ui);
                self.overlay(ui);
                self.small_zones(ui);
                self.gaming(ui);
                self.excluded(ui);
                self.startup_section(ui);
                self.log(ui);
                ui.add_space(12.0);
            });
        });
        // Changes are written once the mouse is up (not on every step of a drag).
        if self.settings != self.saved && !ctx.input(|i| i.pointer.any_down()) {
            self.save();
        }
    }

    fn status_line(&self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| match &self.status {
            Some((text, true)) => {
                ui.label(RichText::new(text).color(style::RED_HOVER));
            }
            Some((text, false)) => {
                ui.weak(text);
            }
            None => {
                ui.weak("Changes apply straight away.");
            }
        });
    }

    // --- hotkeys ---

    fn hotkeys(&mut self, ui: &mut egui::Ui) {
        section(ui, "Hotkeys");
        ui.weak("Click a box and press the new keys. Esc cancels. × turns a hotkey off.");
        ui.add_space(4.0);
        for name in [HotkeyName::LayoutSelector, HotkeyName::GamingSnap, HotkeyName::DebugDump] {
            self.hotkey_row(ui, name);
        }
        self.quick_switch_row(ui);
    }

    fn hotkey_slot(&mut self, name: HotkeyName) -> Option<&mut Option<Hotkey>> {
        let h = &mut self.settings.hotkeys;
        match name {
            HotkeyName::DebugDump => Some(&mut h.debug_dump),
            HotkeyName::GamingSnap => Some(&mut h.gaming_snap),
            HotkeyName::LayoutSelector => Some(&mut h.layout_selector),
            HotkeyName::QuickSwitch => None,
        }
    }

    fn hotkey_row(&mut self, ui: &mut egui::Ui, name: HotkeyName) {
        let current = self.hotkey_slot(name).and_then(|h| *h);
        let capturing = self.is_capturing() == Some(name);
        ui.horizontal(|ui| {
            label_cell(ui, hotkey_label(name));
            let text = if capturing {
                match &self.capturing {
                    Some(c) if c.capture.modifiers().any() => format!("{}+…", c.capture.modifiers()),
                    _ => "Press the keys…".into(),
                }
            } else {
                current.map_or("Off".into(), |h| h.to_string())
            };
            let mut button = egui::Button::new(text).min_size(Vec2::new(HOTKEY_W, style::ROW_H));
            if capturing {
                button = button.fill(ui.visuals().selection.bg_fill);
            }
            let response = ui.add(button).on_hover_text("Click, then press the new keys");
            if response.clicked() {
                if capturing {
                    self.stop_capture();
                } else {
                    self.start_capture(ui.ctx(), name);
                }
            }
            if current.is_some() && ui.small_button("×").on_hover_text("Turn this hotkey off").clicked() {
                self.stop_capture();
                if let Some(slot) = self.hotkey_slot(name) {
                    *slot = None;
                }
                self.failed &= !name.bit();
            }
        });
        self.row_notes(ui, name);
    }

    fn quick_switch_row(&mut self, ui: &mut egui::Ui) {
        let name = HotkeyName::QuickSwitch;
        ui.horizontal(|ui| {
            label_cell(ui, hotkey_label(name));
            let mut on = self.settings.hotkeys.quick_switch.is_some();
            if ui.checkbox(&mut on, "On").changed() {
                self.settings.hotkeys.quick_switch = on.then_some(self.last_quick_switch);
                self.failed &= !name.bit();
            }
            ui.add_space(8.0);
            ui.add_enabled_ui(on, |ui| {
                let mut m = self.last_quick_switch;
                ui.checkbox(&mut m.win, "Win");
                ui.checkbox(&mut m.ctrl, "Ctrl");
                ui.checkbox(&mut m.alt, "Alt");
                ui.checkbox(&mut m.shift, "Shift");
                ui.label("+ 1 … 9");
                // Win, Ctrl or Alt stays ticked: unticking the last one is ignored.
                if m != self.last_quick_switch && m.usable() {
                    self.set_quick_switch(m);
                }
            });
        });
        self.row_notes(ui, name);
    }

    /// Red notes under a hotkey row: taken by another app, the same keys as another of
    /// ours, or a refused key press.
    fn row_notes(&self, ui: &mut egui::Ui, name: HotkeyName) {
        let mut notes = Vec::new();
        if self.failed & name.bit() != 0 {
            notes.push("In use by another app or by Windows. Choose other keys.".to_string());
        }
        for (a, b) in self.settings.hotkeys.clashes() {
            if a == name || b == name {
                let other = if a == name { b } else { a };
                notes.push(format!("Same keys as \"{}\".", hotkey_label(other)));
            }
        }
        if let Some((n, why)) = &self.refused {
            if *n == name {
                notes.push(why.clone());
            }
        }
        for note in notes {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
                ui.label(RichText::new(note).color(style::RED_HOVER));
            });
        }
    }

    /// New quick-switch modifiers, unless one of the nine hotkeys is taken.
    fn set_quick_switch(&mut self, m: Modifiers) {
        let name = HotkeyName::QuickSwitch;
        let taken = gz_core::config::DIGIT_KEYS.map(|vk| m.with(vk)).find(|h| !self.is_free(h));
        if let Some(h) = taken {
            self.refused = Some((name, format!("{h} is already used by Windows or another app.")));
            return;
        }
        if self.refused.as_ref().is_some_and(|(n, _)| *n == name) {
            self.refused = None;
        }
        self.last_quick_switch = m;
        self.settings.hotkeys.quick_switch = Some(m);
        self.failed &= !name.bit();
    }

    /// Whether nothing else holds `hotkey`: our tray app's own hotkeys count as free (they
    /// are checked against each other by [`Hotkeys::clashes`](gz_core::config::Hotkeys::clashes)).
    fn is_free(&self, hotkey: &Hotkey) -> bool {
        if self.saved.hotkeys.bindings().iter().any(|(_, h)| h == hotkey) {
            return true;
        }
        if self.live {
            win_hotkey::is_free(hotkey)
        } else {
            !self.fake_taken.contains(hotkey)
        }
    }

    fn start_capture(&mut self, ctx: &egui::Context, name: HotkeyName) {
        self.stop_capture();
        self.refused = None;
        let hook = if self.live {
            let ctx = ctx.clone();
            match CaptureHook::new(move || ctx.request_repaint()) {
                Ok(h) => Some(h),
                Err(e) => {
                    self.refused = Some((name, format!("Couldn't read the keyboard: {}", e.message())));
                    return;
                }
            }
        } else {
            None
        };
        self.capturing = Some(Capturing { name, capture: Capture::default(), hook, started: self.now });
    }

    fn stop_capture(&mut self) {
        self.capturing = None; // drops the hook
    }

    /// Reads the keys pressed since the last frame. Ends the capture when a hotkey is
    /// complete, on Esc, when the window loses the focus or after a while with no key.
    fn poll_capture(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.capturing else { return };
        let keys = match &c.hook {
            Some(hook) => hook.take(),
            None => std::mem::take(&mut self.fake_keys),
        };
        let name = c.name;
        let mut result = None;
        for (vk, down) in keys {
            match c.capture.key(vk, down) {
                Captured::Waiting => {}
                done @ (Captured::Done(_) | Captured::Cancelled) => {
                    result = Some(done);
                    break;
                }
                Captured::NeedsModifier => {
                    self.refused = Some((name, "Add Win, Ctrl or Alt.".into()));
                }
                Captured::WindowsShortcut(hotkey, what) => {
                    self.refused =
                        Some((name, format!("{hotkey} is a Windows shortcut ({what}). Choose other keys.")));
                }
                Captured::Unusable => self.refused = Some((name, "That key can't be used for a hotkey.".into())),
            }
        }
        let lost_focus = self.live && !ctx.input(|i| i.focused);
        let timed_out = self.now - c.started > CAPTURE_TIMEOUT;
        match result {
            Some(Captured::Done(hotkey)) => {
                self.stop_capture();
                self.set_hotkey(name, hotkey);
            }
            Some(_) => self.stop_capture(),
            None if lost_focus || timed_out => self.stop_capture(),
            // Key presses wake the window (the hook's `wake`); this is for the time-out.
            None => ctx.request_repaint_after(std::time::Duration::from_secs_f64(CAPTURE_TIMEOUT - (self.now - c.started))),
        }
    }

    /// Sets a captured hotkey, unless another of ours already uses those keys.
    fn set_hotkey(&mut self, name: HotkeyName, hotkey: Hotkey) {
        let mut trial = self.settings.hotkeys.clone();
        if let Some(slot) = match name {
            HotkeyName::DebugDump => Some(&mut trial.debug_dump),
            HotkeyName::GamingSnap => Some(&mut trial.gaming_snap),
            HotkeyName::LayoutSelector => Some(&mut trial.layout_selector),
            HotkeyName::QuickSwitch => None,
        } {
            *slot = Some(hotkey);
        }
        let clash = trial.clashes().into_iter().find(|(a, b)| *a == name || *b == name);
        if let Some((a, b)) = clash {
            let other = if a == name { b } else { a };
            self.refused = Some((name, format!("{hotkey} is already used for \"{}\".", hotkey_label(other))));
            return;
        }
        if !self.is_free(&hotkey) {
            self.refused = Some((name, format!("{hotkey} is already used by Windows or another app.")));
            return;
        }
        self.refused = None;
        self.failed &= !name.bit();
        self.settings.hotkeys = trial;
    }

    // --- the other sections ---

    fn dragging(&mut self, ui: &mut egui::Ui) {
        section(ui, "Dragging windows");
        let s = &mut self.settings.snapping;
        ui.horizontal(|ui| {
            label_cell(ui, "Show zones");
            ui.radio_value(&mut s.require_shift, true, "While Shift is held");
            ui.radio_value(&mut s.require_shift, false, "Always (hold Shift to hide them)");
        });
        ui.horizontal(|ui| {
            label_cell(ui, "Overlapping zones");
            egui::ComboBox::from_id_salt("overlap").width(HOTKEY_W).selected_text(overlap_name(s.overlap)).show_ui(
                ui,
                |ui| {
                    for o in [Overlap::Smallest, Overlap::Largest, Overlap::ClosestCentre] {
                        ui.selectable_value(&mut s.overlap, o, overlap_name(o));
                    }
                },
            );
        });
        ui.horizontal(|ui| {
            label_cell(ui, "Near a border");
            ui.checkbox(&mut s.edge_span, "Snap to both zones, within");
            ui.add_enabled(
                s.edge_span,
                egui::DragValue::new(&mut s.edge_span_radius_px)
                    .range(1..=200)
                    .suffix(" px")
                    .update_while_editing(false),
            );
        });
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            ui.weak("Hold Ctrl while dragging to span several zones.");
        });
    }

    fn overlay(&mut self, ui: &mut egui::Ui) {
        section(ui, "Zones while dragging");
        let o = &mut self.settings.overlay;
        for (label, colour) in [
            ("Zone colour", &mut o.zone_color),
            ("Border colour", &mut o.border_color),
            ("Highlight colour", &mut o.highlight_color),
            ("Number colour", &mut o.number_color),
        ] {
            ui.horizontal(|ui| {
                label_cell(ui, label);
                colour_button(ui, colour, label);
            });
        }
        ui.horizontal(|ui| {
            label_cell(ui, "Opacity");
            ui.add(egui::Slider::new(&mut o.opacity, 0..=100).suffix(" %"));
        });
        ui.horizontal(|ui| {
            label_cell(ui, "");
            ui.checkbox(&mut o.show_numbers, "Show zone numbers");
        });
        ui.horizontal(|ui| {
            label_cell(ui, "");
            ui.checkbox(&mut o.flash_on_switch, "Show the zones briefly after switching layout");
        });
    }

    fn small_zones(&mut self, ui: &mut egui::Ui) {
        section(ui, "Small zones");
        let (s, o) = (&mut self.settings.snapping, &mut self.settings.overlay);
        ui.horizontal(|ui| {
            label_cell(ui, "Smaller than");
            ui.add_sized(
                [style::NUMBER_W, style::ROW_H],
                egui::DragValue::new(&mut s.deadzone_px).range(0..=4000).suffix(" px").update_while_editing(false),
            )
            .on_hover_text("Zones narrower or shorter than this can't be snapped to");
            ui.weak("can't be snapped to");
        });
        ui.horizontal(|ui| {
            label_cell(ui, "Show them");
            ui.radio_value(&mut o.disabled_zones, DisabledZones::Greyed, "Greyed");
            ui.radio_value(&mut o.disabled_zones, DisabledZones::Hidden, "Hidden");
        });
        if o.disabled_zones == DisabledZones::Greyed {
            ui.horizontal(|ui| {
                label_cell(ui, "Grey colour");
                colour_button(ui, &mut o.disabled_color, "Grey colour");
            });
        }
    }

    fn gaming(&mut self, ui: &mut egui::Ui) {
        section(ui, "Gaming zone");
        let s = &mut self.settings.snapping;
        ui.horizontal(|ui| {
            label_cell(ui, "A game of another size");
            ui.radio_value(&mut s.gaming_fit, GamingFit::Resize, "Resize it to fit");
            ui.radio_value(&mut s.gaming_fit, GamingFit::Centre, "Centre it, keep its size");
        });
    }

    fn excluded(&mut self, ui: &mut egui::Ui) {
        section(ui, "Apps that never snap");
        let apps = &mut self.settings.snapping.excluded_apps;
        if apps.is_empty() {
            ui.weak("None.");
        }
        let mut remove = None;
        for (i, app) in apps.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([LABEL_W + HOTKEY_W, style::ROW_H], egui::Label::new(app.as_str()).truncate());
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            apps.remove(i);
        }
        ui.horizontal(|ui| {
            let edit = egui::TextEdit::singleline(&mut self.new_exclusion).hint_text("game.exe").desired_width(LABEL_W);
            let response = ui.add(edit);
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if style::sized_button(ui, "Add", style::DIALOG_W).clicked() || enter {
                add_exclusion(apps, &self.new_exclusion);
                self.new_exclusion.clear();
            }
            let live = self.live;
            ui.menu_button("Add a running app…", |ui| {
                let running = if live { running_apps() } else { vec!["test.exe".into()] };
                let running: Vec<String> =
                    running.into_iter().filter(|e| !apps.iter().any(|a| a.eq_ignore_ascii_case(e))).collect();
                if running.is_empty() {
                    ui.weak("No other apps with windows");
                }
                for exe in running {
                    if ui.button(&exe).clicked() {
                        add_exclusion(apps, &exe);
                        ui.close();
                    }
                }
            });
        });
    }

    fn startup_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "Start with Windows");
        let busy = self.startup_job.is_some();
        let mut want = self.startup;
        ui.add_enabled_ui(!busy, |ui| {
            ui.radio_value(&mut want, Startup::Off, "Don't start GamingZones");
            ui.radio_value(&mut want, Startup::SignIn, "Start it when I sign in");
            ui.radio_value(&mut want, Startup::Admin, "Start it when I sign in, as administrator");
        });
        ui.weak(
            "As administrator, it can snap windows of apps that run as administrator. \
             It uses a Scheduled Task, and Windows asks for permission to set it up or remove it.",
        );
        if busy {
            ui.label("Waiting for Windows…");
        }
        if want != self.startup && !busy {
            self.change_startup(want);
        }
    }

    fn change_startup(&mut self, want: Startup) {
        let have = self.startup;
        if !self.live {
            self.startup = want;
            return;
        }
        let tray = std::env::current_exe().unwrap_or_default().with_file_name("gamingzones.exe");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(apply_startup(have, want, &tray));
        });
        self.startup_job = Some(rx);
    }

    fn poll_startup_job(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.startup_job else { return };
        match rx.try_recv() {
            Ok(Ok(now)) => {
                self.startup = now;
                self.startup_job = None;
                self.status = Some((
                    match now {
                        Startup::Off => "GamingZones won't start with Windows.",
                        Startup::SignIn => "GamingZones will start when you sign in.",
                        Startup::Admin => "GamingZones will start as administrator when you sign in.",
                    }
                    .into(),
                    false,
                ));
            }
            Ok(Err(e)) => {
                self.startup_job = None;
                self.startup = Startup::current();
                self.status = Some((e, true));
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.startup_job = None;
                self.startup = Startup::current();
            }
        }
    }

    fn log(&mut self, ui: &mut egui::Ui) {
        section(ui, "Log");
        let level = &mut self.settings.log_level;
        ui.horizontal(|ui| {
            label_cell(ui, "Log level");
            egui::ComboBox::from_id_salt("log_level").width(HOTKEY_W).selected_text(level_name(*level)).show_ui(
                ui,
                |ui| {
                    for l in [
                        LogLevel::Off,
                        LogLevel::Error,
                        LogLevel::Warn,
                        LogLevel::Info,
                        LogLevel::Debug,
                        LogLevel::Trace,
                    ] {
                        ui.selectable_value(level, l, level_name(l));
                    }
                },
            );
            if style::button(ui, "Open the log folder").clicked() && self.live {
                if let Some(dir) = self.path.parent() {
                    let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
                }
            }
        });
    }
}

/// Makes the change from `have` to `want`. The task needs a UAC prompt, so it goes first:
/// if Tom says No, nothing changes.
fn apply_startup(have: Startup, want: Startup, tray: &Path) -> Result<Startup, String> {
    let elevated = |r: Elevated| match r {
        Elevated::Done => Ok(()),
        Elevated::Declined => Err("Not changed: Windows didn't get permission.".to_string()),
        Elevated::Failed(e) => Err(format!("The Scheduled Task couldn't be changed: {e}")),
    };
    let registry = |r: gz_win::Result<()>| r.map_err(|e| format!("The Run entry couldn't be changed: {}", e.message()));
    if want == Startup::Admin {
        let user = format!(
            "{}\\{}",
            std::env::var("USERDOMAIN").unwrap_or_default(),
            std::env::var("USERNAME").unwrap_or_default()
        );
        let xml_path = std::env::temp_dir().join("GamingZones-task.xml");
        elevated(startup::create_task(&startup::task_xml(tray, &user), &xml_path))?;
        registry(startup::delete_run_value())?;
        return Ok(Startup::Admin);
    }
    if have == Startup::Admin {
        elevated(startup::delete_task())?;
    }
    match want {
        Startup::SignIn => registry(startup::set_run_value(tray))?,
        _ => registry(startup::delete_run_value())?,
    }
    Ok(want)
}

fn add_exclusion(apps: &mut Vec<String>, exe: &str) {
    let exe = exe.trim();
    if !exe.is_empty() && !apps.iter().any(|a| a.eq_ignore_ascii_case(exe)) {
        apps.push(exe.to_string());
    }
}

/// Exe names of apps with visible windows, sorted, without GamingZones' own.
fn running_apps() -> Vec<String> {
    let mut exes: Vec<String> = inspect::top_level_windows()
        .into_iter()
        .filter(|&w| inspect::is_visible(w) && inspect::cloaked(w) == 0)
        .filter_map(|w| process::exe_path(process::window_pid(w)).ok())
        .filter_map(|p| Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()))
        .filter(|n| !["gamingzones.exe", "gz-editor.exe"].contains(&n.to_lowercase().as_str()))
        .collect();
    exes.sort_by_key(|e| e.to_lowercase());
    exes.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    exes
}

pub fn hotkey_label(name: HotkeyName) -> &'static str {
    match name {
        HotkeyName::LayoutSelector => "Layout selector",
        HotkeyName::GamingSnap => "Snap to gaming zone",
        HotkeyName::DebugDump => "Window info to the log",
        HotkeyName::QuickSwitch => "Switch to layout 1-9",
    }
}

fn overlap_name(o: Overlap) -> &'static str {
    match o {
        Overlap::Smallest => "The smallest one wins",
        Overlap::Largest => "The largest one wins",
        Overlap::ClosestCentre => "The nearest centre wins",
    }
}

fn level_name(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Off => "Off",
        LogLevel::Error => "Errors",
        LogLevel::Warn => "Warnings",
        LogLevel::Info => "Info",
        LogLevel::Debug => "Debug",
        LogLevel::Trace => "Trace (a lot)",
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.heading(title);
    ui.separator();
}

fn label_cell(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.allocate_ui_with_layout(
        Vec2::new(LABEL_W, style::ROW_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(LABEL_W);
            ui.label(text)
        },
    )
    .inner
}

fn colour_button(ui: &mut egui::Ui, colour: &mut Color, label: &str) {
    let mut rgb = [colour.r, colour.g, colour.b];
    ui.color_edit_button_srgb(&mut rgb).on_hover_text(label);
    *colour = Color::rgb(rgb[0], rgb[1], rgb[2]);
    ui.weak(colour.to_string());
}

impl eframe::App for SettingsWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
