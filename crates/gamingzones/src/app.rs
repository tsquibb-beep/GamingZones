//! The hidden message window: tray icon, hotkeys, config reload, window drags.
//!
//! Everything here is driven by window messages. Between messages the thread sleeps in
//! GetMessageW, so the app uses no CPU while idle. The only hook that exists while no
//! window is being dragged is the move/size WinEvent hook, which Windows calls only when a
//! drag starts or ends. (And a keyboard hook while a settings window hotkey box asks for
//! one: see [`ipc::CAPTURE_MESSAGE`].)

use std::collections::HashSet;
use std::path::PathBuf;

use gz_core::config::{HotkeyName, DIGIT_KEYS};
use gz_core::config::{LoadOutcome, SETTINGS_FILE};
use gz_core::ipc::{self, CAPTURE_KEY_MESSAGE, CAPTURE_MESSAGE, MAIN_WINDOW_CLASS, RELOAD_MESSAGE};
use gz_core::{Hotkey, Settings};
use gz_win::keyhook::{CaptureHook, KeyboardHook};
use gz_win::monitor::{self, MonitorIds, MonitorInfo};
use gz_win::tray::{self, MenuItem, TrayIcon};
use gz_win::window::{self, msg, Background, Message, WindowHandler, WindowSpec};
use gz_win::winevent::{self, event, Delivery, Filter, WinEventHook};
use gz_win::{hotkey, input, inspect, ipc as win_ipc, process, styles::ws, Hwnd};

use crate::drag::{Drag, DragMessages, DropTarget};
use crate::overlay::Overlay;
use crate::preview::Preview;
use crate::selector::{self, Selector};
use crate::snap::Picked;
use crate::zones::{MonitorZones, ZoneConfig};
use crate::{dump, logger, snap};

/// Tray icon callback.
const WM_TRAY: u32 = msg::APP + 1;
/// From the move/size WinEvent hook: wParam = event, lParam = window.
const WM_MOVESIZE: u32 = msg::APP + 2;
/// From the drag's hooks.
const WM_DRAG_LOCATION: u32 = msg::APP + 3;
const WM_DRAG_DESTROY: u32 = msg::APP + 4;
/// From the drag's keyboard hook: wParam = virtual key, lParam = 1 down / 0 up.
const WM_DRAG_KEY: u32 = msg::APP + 5;
/// The editor's window (showing a preview) was destroyed.
const WM_PREVIEW_CLOSED: u32 = msg::APP + 6;
/// The layout selector closed: wParam = chosen card or `selector::CANCELLED`.
const WM_SELECTOR_DONE: u32 = msg::APP + 7;

const DRAG_MESSAGES: DragMessages =
    DragMessages { location: WM_DRAG_LOCATION, destroy: WM_DRAG_DESTROY, key: WM_DRAG_KEY };

const CMD_RELOAD: u32 = 1;
const CMD_EDITOR: u32 = 2;
const CMD_SETTINGS: u32 = 3;
const CMD_EXIT: u32 = 4;

/// Hotkey ids (`WM_HOTKEY` wParam).
const HOTKEY_DEBUG_DUMP: i32 = 1;
const HOTKEY_GAMING_SNAP: i32 = 2;
const HOTKEY_LAYOUT_SELECTOR: i32 = 3;
/// Quick-switch: this + 1..9 for layouts 1..9.
const HOTKEY_QUICK_SWITCH: i32 = 10;

/// `WM_TIMER` id: the end of the zone flash after a layout switch.
const TIMER_FLASH: usize = 1;
const FLASH_MS: u32 = 600;
/// `WM_TIMER` id: a hotkey capture nobody stopped ends anyway.
const TIMER_CAPTURE: usize = 2;
const CAPTURE_MAX_MS: u32 = 30_000;

pub struct App {
    config_dir: PathBuf,
    settings: Settings,
    zones: ZoneConfig,
    tray: Option<TrayIcon>,
    registered: Vec<i32>,
    reload_msg: u32,
    capture_msg: u32,
    capture_key_msg: u32,
    taskbar_created_msg: u32,
    self_elevated: bool,
    movesize_hook: Option<WinEventHook>,
    drag: Option<Drag>,
    /// Windows we've already shown a "can't snap this" notice for.
    warned: HashSet<Hwnd>,
    /// The editor's "Preview on screen".
    preview: Option<Preview>,
    /// The new layout's zones, shown briefly after a switch.
    flash: Option<Overlay>,
    selector: Option<Selector>,
    /// Keys for a settings window hotkey box.
    capture: Option<CaptureHook>,
}

/// Creates the hidden window; the app lives in its handler until the window is destroyed.
pub fn start(config_dir: PathBuf) -> gz_win::Result<Hwnd> {
    let app = App {
        config_dir,
        settings: Settings::default(),
        zones: ZoneConfig::default(),
        tray: None,
        registered: Vec::new(),
        reload_msg: window::register_window_message(RELOAD_MESSAGE),
        capture_msg: window::register_window_message(CAPTURE_MESSAGE),
        capture_key_msg: window::register_window_message(CAPTURE_KEY_MESSAGE),
        taskbar_created_msg: tray::taskbar_created_message(),
        self_elevated: process::current_is_elevated().unwrap_or(false),
        movesize_hook: None,
        drag: None,
        warned: HashSet::new(),
        preview: None,
        flash: None,
        selector: None,
        capture: None,
    };
    // A hidden top-level window rather than a message-only one: only top-level windows get
    // the TaskbarCreated and WM_DISPLAYCHANGE broadcasts.
    let spec = WindowSpec {
        class: MAIN_WINDOW_CLASS,
        title: "GamingZones",
        style: ws::POPUP,
        ex_style: ws::EX_TOOLWINDOW,
        rect: Some(gz_core::Rect::default()),
        icon: true,
        background: Background::None,
    };
    window::create_window(&spec, Box::new(app))
}

impl App {
    fn on_create(&mut self, hwnd: Hwnd) {
        dump::log_self(hwnd);
        // If we run elevated, the (possibly non-elevated) editor must still reach us.
        for m in [self.reload_msg, self.capture_msg, msg::COPYDATA] {
            if let Err(e) = win_ipc::allow_message(hwnd, m) {
                log::warn!("Could not allow message {m:#X} from other processes: {}", e.message());
            }
        }
        match TrayIcon::add(hwnd, WM_TRAY, "GamingZones") {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => log::error!("Could not add the tray icon: {}", e.message()),
        }
        self.reload(hwnd, false);
        // Windows calls this hook only when a window drag/resize starts or ends.
        match WinEventHook::new(
            event::MOVESIZESTART,
            event::MOVESIZEEND,
            0,
            Filter { window: None, cursor: false },
            Delivery { target: hwnd, msg: WM_MOVESIZE, coalesce: false },
        ) {
            Ok(hook) => self.movesize_hook = Some(hook),
            Err(e) => {
                log::error!("Could not watch window drags (SetWinEventHook): {}", e.message());
                self.notify("Zones won't work", "Could not watch window drags. Details are in the log.");
            }
        }
    }

    /// Re-reads settings.json, layouts.json and applied.json. A file with an error keeps
    /// its previous contents. Returns the hotkey settings that couldn't be registered
    /// ([`HotkeyName::bit`]s); with `quiet` they aren't shown as a notification.
    fn reload(&mut self, hwnd: Hwnd, quiet: bool) -> u32 {
        let path = self.config_dir.join(SETTINGS_FILE);
        match Settings::load_or_create(&path) {
            Ok((settings, outcome)) => {
                self.settings = settings;
                logger::set_level(self.settings.log_level);
                match outcome {
                    LoadOutcome::Created => log::info!("Created {} with defaults", path.display()),
                    LoadOutcome::Loaded => log::info!("Loaded {}", path.display()),
                    LoadOutcome::Upgraded => {
                        log::info!("Loaded {} and added settings new in this version", path.display())
                    }
                }
            }
            Err(e) => {
                log::error!("{} could not be read: {e}. Keeping the previous settings.", path.display());
                self.notify(
                    "settings.json has an error",
                    &format!("{e}\nKeeping the previous settings. Details are in the log."),
                );
            }
        }
        let failed = self.register_hotkeys(hwnd, quiet);
        let errors = self.zones.reload(&self.config_dir, self.settings.snapping.deadzone_px);
        if !errors.is_empty() {
            self.notify("Layout files have an error", &format!("{}\nDetails are in the log.", errors.join("\n")));
        }
        failed
    }

    fn register_hotkeys(&mut self, hwnd: Hwnd, quiet: bool) -> u32 {
        for id in self.registered.drain(..) {
            if let Err(e) = hotkey::unregister(hwnd, id) {
                log::warn!("Unregistering hotkey {id} failed: {}", e.message());
            }
        }
        let keys = &self.settings.hotkeys;
        for name in HotkeyName::ALL {
            let off = match name {
                HotkeyName::DebugDump => keys.debug_dump.is_none(),
                HotkeyName::GamingSnap => keys.gaming_snap.is_none(),
                HotkeyName::LayoutSelector => keys.layout_selector.is_none(),
                HotkeyName::QuickSwitch => keys.quick_switch.is_none(),
            };
            if off {
                log::info!("Hotkey {} is disabled", name.key());
            }
        }
        let mut failed_mask = 0;
        let mut failed = Vec::new();
        for (name, hotkey) in keys.bindings() {
            let id = hotkey_id(name, &hotkey);
            match hotkey::register(hwnd, id, &hotkey) {
                Ok(()) => {
                    log::info!("Hotkey {} = {hotkey}", name.key());
                    self.registered.push(id);
                }
                Err(e) => {
                    log::warn!(
                        "Hotkey {} = {hotkey} could not be registered: {} ({:#010X}). \
                         Another app or Windows already uses it.",
                        name.key(),
                        e.message(),
                        e.code().0
                    );
                    failed_mask |= name.bit();
                    failed.push(hotkey.to_string());
                }
            }
        }
        if !failed.is_empty() && !quiet {
            self.notify("Hotkey already in use", &format!("{}\nChange it in Settings.", failed.join("\n")));
        }
        failed_mask
    }

    fn notify(&self, title: &str, text: &str) {
        if let Some(tray) = &self.tray {
            if let Err(e) = tray.balloon(title, text, true) {
                log::warn!("Tray notification failed: {}", e.message());
            }
        }
    }

    fn show_menu(&mut self, hwnd: Hwnd) {
        let items = [
            MenuItem::Item { id: CMD_RELOAD, label: "&Reload config", enabled: true },
            MenuItem::Item { id: CMD_EDITOR, label: "Open &editor", enabled: true },
            MenuItem::Item { id: CMD_SETTINGS, label: "&Settings", enabled: true },
            MenuItem::Separator,
            MenuItem::Item { id: CMD_EXIT, label: "E&xit", enabled: true },
        ];
        match tray::show_menu(hwnd, &items) {
            Some(CMD_RELOAD) => {
                log::info!("Reload requested from the tray menu");
                self.reload(hwnd, false);
            }
            Some(CMD_EDITOR) => self.open_editor(&[]),
            Some(CMD_SETTINGS) => self.open_editor(&["--settings"]),
            Some(CMD_EXIT) => {
                // Posted, not DestroyWindow here: WM_DESTROY must reach `handle` (see
                // WindowHandler's re-entrancy note).
                let _ = window::post_message(hwnd, msg::CLOSE, 0, 0);
            }
            _ => {}
        }
    }

    /// Starts gz-editor.exe from our own folder (with `--settings`, the settings window).
    /// It's a separate process, so nothing of it stays in ours once it exits; it keeps
    /// itself to one window.
    fn open_editor(&self, args: &[&str]) {
        let path = std::env::current_exe().unwrap_or_default().with_file_name("gz-editor.exe");
        match std::process::Command::new(&path).args(args).spawn() {
            Ok(child) => log::info!("Started gz-editor {} (pid {})", args.join(" "), child.id()),
            Err(e) => {
                log::error!("Could not start {}: {e}", path.display());
                self.notify("Couldn't open the editor", &format!("{}: {e}", path.display()));
            }
        }
    }

    /// `WM_COPYDATA` from the editor: show, update or hide the preview.
    fn on_copy_data(&mut self, hwnd: Hwnd, m: &Message) {
        let Some((kind, bytes, sender)) = win_ipc::received_data(m) else { return };
        match kind {
            ipc::PREVIEW_SHOW => {
                let Some(editor) = sender else { return };
                let p: ipc::Preview = match serde_json::from_slice(&bytes) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("Preview: bad data from the editor: {e}");
                        return;
                    }
                };
                if self.preview.as_ref().is_none_or(|p| p.editor != editor) {
                    log::debug!("Preview: showing for editor window {editor}");
                    self.preview = Some(Preview::new(hwnd, editor, WM_PREVIEW_CLOSED));
                    if self.drag.is_some() {
                        self.preview.as_mut().expect("just set").suspend();
                    }
                }
                let preview = self.preview.as_mut().expect("set above");
                if let Err(e) = preview.update(&p, &self.settings) {
                    log::warn!("Preview: {e}");
                }
            }
            ipc::PREVIEW_HIDE => {
                let had = self.preview.take().is_some();
                log::debug!("Preview: hidden{}", if had { "" } else { " (wasn't showing)" });
            }
            _ => {}
        }
    }

    fn on_move_size(&mut self, hwnd: Hwnd, m: &Message) {
        let Some(window) = winevent::message_window(m.lparam) else { return };
        match m.wparam as u32 {
            event::MOVESIZESTART => self.start_drag(hwnd, window),
            event::MOVESIZEEND => self.end_drag(window),
            _ => {}
        }
    }

    fn start_drag(&mut self, hwnd: Hwnd, window: Hwnd) {
        if let Some(old) = self.drag.take() {
            log::warn!("Drag of {} never reported its end; cleaning up", old.exe);
        }
        if input::cursor_is_resize() {
            log::debug!("Resize of {window} started; ignored");
            return;
        }
        let target = match snap::check(window, &self.settings.snapping, self.self_elevated, Picked::Drag) {
            Ok(t) => t,
            Err(skip) => {
                log::debug!("Drag of {window} ignored: {skip}");
                return;
            }
        };
        if target.elevated {
            // Windows hides key presses from us while an admin window is in front, so we
            // can't tell whether Shift is held: say so at the start of the drag, as FancyZones does.
            log::debug!("Drag of {} ({window}) ignored: it runs as administrator", target.exe);
            if self.warned.insert(window) {
                log::warn!("{} runs as administrator and GamingZones doesn't, so it can't be snapped", target.exe);
                self.notify(
                    "Can't snap this window",
                    &format!("{} runs as administrator. Run GamingZones as administrator to snap it.", target.exe),
                );
            }
            return;
        }
        self.flash = None;
        self.selector = None;
        if let Some(p) = &mut self.preview {
            p.suspend();
        }
        let drag = Drag::start(hwnd, window, target.exe, self.settings.snapping.require_shift, &DRAG_MESSAGES);
        log::debug!(
            "Drag of {} ({window}) started; hooks: {} WinEvent, keyboard {}",
            drag.exe,
            WinEventHook::live_count(),
            if KeyboardHook::is_installed() { "yes" } else { "no" }
        );
        self.drag = Some(drag);
        self.update_drag();
    }

    fn update_drag(&mut self) {
        let Some(drag) = &mut self.drag else { return };
        drag.update(&self.settings, &self.zones);
    }

    fn end_drag(&mut self, window: Hwnd) {
        match &self.drag {
            Some(d) if d.window == window => {}
            _ => return,
        }
        let drag = self.drag.take().expect("checked above");
        let exe = drag.exe.clone();
        let target = drag.finish(); // hooks and overlay are gone after this
        if let Some(p) = &mut self.preview {
            p.resume();
        }
        log::debug!(
            "Drag of {exe} ended; hooks left: {} WinEvent (1 = the move/size hook), keyboard {}",
            WinEventHook::live_count(),
            if KeyboardHook::is_installed() { "yes" } else { "no" }
        );
        let Some(target) = target else { return };
        if inspect::is_maximized(window) {
            log::info!("Not snapping {exe}: it was maximised at the end of the drag (Windows Snap or a tab merge)");
            return;
        }
        self.snap(window, &exe, &target);
    }

    /// The gaming-snap hotkey: fits the foreground window's client area to the gaming zone
    /// on its monitor or, if that monitor's layout has none, on the first monitor with one.
    fn snap_to_gaming_zone(&mut self) {
        let Some(window) = inspect::foreground_window() else { return };
        let target = match snap::check(window, &self.settings.snapping, self.self_elevated, Picked::Hotkey) {
            Ok(t) => t,
            Err(skip) => {
                log::info!("Gaming snap: foreground window {window} left alone: {skip}");
                return;
            }
        };
        if target.elevated {
            log::warn!(
                "Gaming snap: {} runs as administrator and GamingZones doesn't, so it can't be moved",
                target.exe
            );
            self.notify(
                "Can't snap this window",
                &format!("{} runs as administrator. Run GamingZones as administrator to snap it.", target.exe),
            );
            return;
        }
        let Some((mz, i)) = self.gaming_monitor(window) else {
            log::info!("Gaming snap: none of the layouts in use has a gaming zone");
            self.notify("No gaming zone", "None of the layouts in use has a gaming zone.");
            return;
        };
        if inspect::is_maximized(window) {
            log::debug!("Gaming snap: restoring maximised {} first", target.exe);
            window::restore(window);
        }
        let drop = DropTarget { rect: mz.zones[i].rect, label: format!("G on {}", mz.device), gaming: true };
        self.snap(window, &target.exe, &drop);
    }

    /// The monitor whose gaming zone the hotkey uses, and the zone's index.
    fn gaming_monitor(&self, window: Hwnd) -> Option<(MonitorZones, usize)> {
        let ids = MonitorIds::query();
        let mut others = monitor::all();
        others.sort_by_key(|m| !m.primary);
        monitor::for_window(window)
            .into_iter()
            .chain(others)
            .filter_map(|m| self.zones.for_monitor(&m, &ids, self.settings.snapping.deadzone_px))
            .find_map(|mz| mz.gaming_zone().map(|i| (mz, i)))
    }

    /// Moves `window`: its client area onto the gaming zone, or its visible frame onto other zones.
    fn snap(&mut self, window: Hwnd, exe: &str, target: &DropTarget) {
        let (result, what) = if target.gaming {
            (snap::snap_client(window, target.rect, self.settings.snapping.gaming_fit), "client")
        } else {
            (snap::snap_frame(window, target.rect), "frame")
        };
        let zone = &target.label;
        match result {
            Ok(s) if s.exact() => {
                log::info!("Snapped {exe} to zone {zone}: {what} {} (passes: {})", s.actual, s.passes)
            }
            Ok(s) => log::warn!(
                "Snapped {exe} to zone {zone}, but its {what} is {} instead of {} (the app limits its size?)",
                s.actual,
                s.target
            ),
            Err(e) => {
                log::warn!("Snapping {exe} to zone {zone} failed: {e}");
                if self.warned.insert(window) {
                    self.notify("Couldn't snap this window", &format!("{exe}: {e}"));
                }
            }
        }
    }

    /// Quick-switch: layout `index` (0-based, list order) on the monitor with the foreground window.
    fn quick_switch(&mut self, index: usize, hwnd: Hwnd) {
        let Some(layout) = self.zones.layouts().get(index) else {
            log::info!("Quick-switch: there is no layout {} ({} layouts)", index + 1, self.zones.layouts().len());
            return;
        };
        let id = layout.id.clone();
        if let Some(m) = target_monitor() {
            self.switch_layout(hwnd, &m, Some(&id));
        }
    }

    /// The selector hotkey: opens the selector on the foreground window's monitor, or
    /// closes it if it's open.
    fn toggle_selector(&mut self, hwnd: Hwnd) {
        if self.selector.take().is_some() {
            log::debug!("Layout selector closed by its hotkey");
            return;
        }
        let previous = inspect::foreground_window();
        let Some(m) = target_monitor() else { return };
        let ids = MonitorIds::query();
        let entries = selector::Entry::all(self.zones.layouts(), &m, self.settings.snapping.deadzone_px);
        let current = self.zones.layout_id(&m, &ids).map(str::to_string);
        let current = entries.iter().position(|e| e.layout == current);
        let name = ids.id(&m).name;
        let disabled = self.settings.overlay.disabled_zones;
        match Selector::open(hwnd, WM_SELECTOR_DONE, m, &name, entries, current, disabled, previous) {
            Ok(s) => {
                log::debug!("Layout selector open on {}", s.monitor.device);
                self.flash = None;
                self.selector = Some(s);
            }
            Err(e) => log::error!("Could not open the layout selector: {}", e.message()),
        }
    }

    fn selector_done(&mut self, hwnd: Hwnd, choice: usize) {
        let Some(s) = self.selector.take() else { return };
        let layout = s.layout_of(choice);
        let (monitor, previous) = (s.monitor.clone(), s.previous);
        drop(s);
        // Give the focus back to the window that had it.
        if let Some(w) = previous.filter(|&w| inspect::exists(w)) {
            window::activate(w);
        }
        match layout {
            Some(layout) => self.switch_layout(hwnd, &monitor, layout.as_deref()),
            None => log::debug!("Layout selector closed without a choice"),
        }
    }

    /// Puts `m` on `layout` (`None` = no zones), then flashes its zones. Windows don't move.
    fn switch_layout(&mut self, hwnd: Hwnd, m: &MonitorInfo, layout: Option<&str>) {
        let ids = MonitorIds::query();
        if self.zones.layout_id(m, &ids) == layout {
            log::info!("{} already uses layout {}", m.device, layout.unwrap_or("(none)"));
        } else if let Err(e) = self.zones.set_layout(&self.config_dir, m, &ids, layout) {
            log::error!("Could not switch {} to layout {}: {e}", m.device, layout.unwrap_or("(none)"));
            self.notify("Couldn't switch layout", &e);
            return;
        } else {
            log::info!("{} now uses layout {}", m.device, layout.unwrap_or("(none)"));
        }
        self.flash_zones(hwnd, m, &ids);
    }

    /// Shows `m`'s zones on top of everything for [`FLASH_MS`] (if `flash_on_switch` is on).
    fn flash_zones(&mut self, hwnd: Hwnd, m: &MonitorInfo, ids: &MonitorIds) {
        self.flash = None;
        if !self.settings.overlay.flash_on_switch || self.drag.is_some() {
            return;
        }
        let Some(mz) = self.zones.for_monitor(m, ids, self.settings.snapping.deadzone_px) else { return };
        match Overlay::create(mz.overlay_area(), mz.zones, mz.dpi, &self.settings.overlay) {
            Ok(o) => {
                o.show_on_top();
                self.flash = Some(o);
                if let Err(e) = window::set_timer(hwnd, TIMER_FLASH, FLASH_MS) {
                    log::warn!("Could not time the zone flash: {}", e.message());
                    self.flash = None;
                }
            }
            Err(e) => log::warn!("Could not create the zone overlay: {}", e.message()),
        }
    }

    fn on_destroy(&mut self, hwnd: Hwnd) {
        self.drag = None;
        self.flash = None;
        self.selector = None;
        self.preview = None;
        self.movesize_hook = None;
        for id in self.registered.drain(..) {
            let _ = hotkey::unregister(hwnd, id);
        }
        self.tray = None; // removes the icon
        log::info!("Exiting");
        window::post_quit(0);
    }
}

impl App {
    /// The settings window starts or stops a hotkey capture. Keys go to its window as
    /// `capture_key_msg` and aren't logged.
    fn on_capture(&mut self, hwnd: Hwnd, m: &Message) -> isize {
        let was_capturing = self.capture.take().is_some();
        window::kill_timer(hwnd, TIMER_CAPTURE);
        if m.wparam != ipc::CAPTURE_START {
            if was_capturing {
                log::debug!("Hotkey capture stopped");
            }
            return 0;
        }
        let Some(target) = winevent::message_window(m.lparam) else { return 0 };
        let (key_msg, stop_msg) = (self.capture_key_msg, self.capture_msg);
        let hook = CaptureHook::new(move |vk, down| {
            if window::post_message(target, key_msg, vk as usize, down as isize).is_err() {
                // The settings window has gone.
                let _ = window::post_message(hwnd, stop_msg, ipc::CAPTURE_STOP, 0);
            }
        });
        match hook {
            Ok(hook) => {
                log::debug!("Hotkey capture started for window {target}");
                self.capture = Some(hook);
                if let Err(e) = window::set_timer(hwnd, TIMER_CAPTURE, CAPTURE_MAX_MS) {
                    log::warn!("Could not set the capture timer: {}", e.message());
                }
                ipc::CAPTURE_STARTED
            }
            Err(e) => {
                log::warn!("Could not start the hotkey capture: {}", e.message());
                0
            }
        }
    }
}

impl WindowHandler for App {
    fn handle(&mut self, hwnd: Hwnd, m: &Message) -> Option<isize> {
        match m.msg {
            msg::CREATE => self.on_create(hwnd),
            WM_TRAY => {
                if matches!(m.loword_lparam(), msg::RBUTTONUP | msg::LBUTTONUP | msg::CONTEXTMENU) {
                    self.show_menu(hwnd);
                }
            }
            msg::HOTKEY => match m.wparam as i32 {
                HOTKEY_DEBUG_DUMP => dump::log_foreground_window(),
                HOTKEY_GAMING_SNAP => self.snap_to_gaming_zone(),
                HOTKEY_LAYOUT_SELECTOR => self.toggle_selector(hwnd),
                id if (HOTKEY_QUICK_SWITCH + 1..=HOTKEY_QUICK_SWITCH + 9).contains(&id) => {
                    self.quick_switch((id - HOTKEY_QUICK_SWITCH - 1) as usize, hwnd)
                }
                _ => {}
            },
            WM_SELECTOR_DONE => self.selector_done(hwnd, m.wparam),
            msg::TIMER if m.wparam == TIMER_FLASH => {
                window::kill_timer(hwnd, TIMER_FLASH);
                self.flash = None;
            }
            msg::TIMER if m.wparam == TIMER_CAPTURE => {
                window::kill_timer(hwnd, TIMER_CAPTURE);
                if self.capture.take().is_some() {
                    log::debug!("Hotkey capture timed out");
                }
            }
            WM_MOVESIZE => self.on_move_size(hwnd, m),
            WM_DRAG_LOCATION => {
                if let Some(drag) = &self.drag {
                    drag.rearm();
                }
                self.update_drag();
            }
            WM_DRAG_KEY => {
                if let Some(drag) = &mut self.drag {
                    drag.on_key(m.wparam as u16, m.lparam != 0);
                }
                self.update_drag();
            }
            msg::COPYDATA => {
                self.on_copy_data(hwnd, m);
                return Some(1);
            }
            WM_PREVIEW_CLOSED => {
                let window = winevent::message_window(m.lparam);
                if self.preview.as_ref().is_some_and(|p| Some(p.editor) == window) {
                    log::debug!("Preview: the editor closed");
                    self.preview = None;
                }
            }
            WM_DRAG_DESTROY => {
                let window = winevent::message_window(m.lparam);
                if self.drag.as_ref().is_some_and(|d| Some(d.window) == window) {
                    let drag = self.drag.take().expect("checked above");
                    log::info!("{} closed during the drag", drag.exe);
                }
            }
            // Zones are worked out from the current work area at each drag, so these need
            // nothing more than a log line.
            msg::DISPLAYCHANGE => log::debug!("WM_DISPLAYCHANGE"),
            msg::DPICHANGED => log::debug!("WM_DPICHANGED"),
            msg::SETTINGCHANGE if m.wparam == msg::SPI_SETWORKAREA => log::debug!("Work area changed"),
            msg::DESTROY => self.on_destroy(hwnd),
            other if other != 0 && other == self.reload_msg => {
                log::info!("Reload requested by {RELOAD_MESSAGE}");
                let failed = self.reload(hwnd, m.wparam == ipc::RELOAD_QUIET);
                return Some(ipc::RELOAD_DONE | failed as isize);
            }
            other if other != 0 && other == self.capture_msg => return Some(self.on_capture(hwnd, m)),
            other if other != 0 && other == self.taskbar_created_msg => {
                log::info!("Taskbar re-created; adding the tray icon again");
                if let Some(tray) = &self.tray {
                    if let Err(e) = tray.re_add() {
                        log::error!("Could not re-add the tray icon: {}", e.message());
                    }
                }
            }
            _ => return None,
        }
        Some(0)
    }
}

fn hotkey_id(name: HotkeyName, hotkey: &Hotkey) -> i32 {
    match name {
        HotkeyName::DebugDump => HOTKEY_DEBUG_DUMP,
        HotkeyName::GamingSnap => HOTKEY_GAMING_SNAP,
        HotkeyName::LayoutSelector => HOTKEY_LAYOUT_SELECTOR,
        HotkeyName::QuickSwitch => HOTKEY_QUICK_SWITCH + (hotkey.vk - DIGIT_KEYS.start() + 1) as i32,
    }
}

/// The monitor a layout hotkey acts on: the foreground window's, or the cursor's when the
/// desktop (or nothing) has the focus.
fn target_monitor() -> Option<MonitorInfo> {
    let window = inspect::foreground_window()
        .filter(|&w| inspect::is_visible(w) && !matches!(inspect::class_name(w).as_str(), "Progman" | "WorkerW"));
    window.and_then(monitor::for_window).or_else(|| {
        let (x, y) = input::cursor_pos()?;
        monitor::from_point(x, y)
    })
}
