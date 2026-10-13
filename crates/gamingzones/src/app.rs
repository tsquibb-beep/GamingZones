//! The hidden message window: tray icon, hotkeys, config reload.
//!
//! Everything here is driven by window messages. Between messages the thread sleeps in
//! GetMessageW, so the app uses no CPU while idle.

use std::path::PathBuf;

use gz_core::config::{LoadOutcome, SETTINGS_FILE};
use gz_core::{Hotkey, Settings};
use gz_win::tray::{self, MenuItem, TrayIcon};
use gz_win::window::{self, msg, Background, Message, WindowHandler, WindowSpec};
use gz_win::{hotkey, styles::ws, Hwnd};

use crate::{dump, logger};

/// Tray icon callback.
const WM_TRAY: u32 = msg::APP + 1;

const CMD_RELOAD: u32 = 1;
const CMD_EDITOR: u32 = 2;
const CMD_SETTINGS: u32 = 3;
const CMD_EXIT: u32 = 4;

/// Hotkey ids (`WM_HOTKEY` wParam).
const HOTKEY_DEBUG_DUMP: i32 = 1;

/// Posted by the editor (Phase 3) after it saves, and usable by anything else.
pub const RELOAD_MESSAGE: &str = "GamingZones.Reload";

pub struct App {
    config_dir: PathBuf,
    settings: Settings,
    tray: Option<TrayIcon>,
    registered: Vec<i32>,
    reload_msg: u32,
    taskbar_created_msg: u32,
}

/// Creates the hidden window; the app lives in its handler until the window is destroyed.
pub fn start(config_dir: PathBuf) -> gz_win::Result<Hwnd> {
    let app = App {
        config_dir,
        settings: Settings::default(),
        tray: None,
        registered: Vec::new(),
        reload_msg: window::register_window_message(RELOAD_MESSAGE),
        taskbar_created_msg: tray::taskbar_created_message(),
    };
    // A hidden top-level window rather than a message-only one: only top-level windows get
    // the TaskbarCreated and WM_DISPLAYCHANGE broadcasts.
    let spec = WindowSpec {
        class: "GamingZones.Main",
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
        match TrayIcon::add(hwnd, WM_TRAY, "GamingZones") {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => log::error!("Could not add the tray icon: {}", e.message()),
        }
        self.reload(hwnd);
    }

    /// Re-reads settings.json and applies it. On a parse error the previous settings stay.
    fn reload(&mut self, hwnd: Hwnd) {
        let path = self.config_dir.join(SETTINGS_FILE);
        match Settings::load_or_create(&path) {
            Ok((settings, outcome)) => {
                self.settings = settings;
                logger::set_level(self.settings.log_level);
                match outcome {
                    LoadOutcome::Created => log::info!("Created {} with defaults", path.display()),
                    LoadOutcome::Loaded => log::info!("Loaded {}", path.display()),
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
        self.register_hotkeys(hwnd);
    }

    fn register_hotkeys(&mut self, hwnd: Hwnd) {
        for id in self.registered.drain(..) {
            if let Err(e) = hotkey::unregister(hwnd, id) {
                log::warn!("Unregistering hotkey {id} failed: {}", e.message());
            }
        }
        let wanted: [(i32, &str, Option<Hotkey>); 1] =
            [(HOTKEY_DEBUG_DUMP, "debug_dump", self.settings.hotkeys.debug_dump)];
        let mut failed = Vec::new();
        for (id, name, hotkey) in wanted {
            let Some(hotkey) = hotkey else {
                log::info!("Hotkey {name} is disabled");
                continue;
            };
            match hotkey::register(hwnd, id, &hotkey) {
                Ok(()) => {
                    log::info!("Hotkey {name} = {hotkey}");
                    self.registered.push(id);
                }
                Err(e) => {
                    log::warn!(
                        "Hotkey {name} = {hotkey} could not be registered: {} ({:#010X}). \
                         Another app or Windows already uses it; change it in {SETTINGS_FILE}.",
                        e.message(),
                        e.code().0
                    );
                    failed.push(format!("{hotkey} ({name})"));
                }
            }
        }
        if !failed.is_empty() {
            self.notify("Hotkey already in use", &format!("{}\nChange it in {SETTINGS_FILE}.", failed.join("\n")));
        }
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
            MenuItem::Item { id: CMD_EDITOR, label: "Open &editor", enabled: false },
            MenuItem::Item { id: CMD_SETTINGS, label: "&Settings", enabled: false },
            MenuItem::Separator,
            MenuItem::Item { id: CMD_EXIT, label: "E&xit", enabled: true },
        ];
        match tray::show_menu(hwnd, &items) {
            Some(CMD_RELOAD) => {
                log::info!("Reload requested from the tray menu");
                self.reload(hwnd);
            }
            Some(CMD_EXIT) => {
                // Posted, not DestroyWindow here: WM_DESTROY must reach `handle` (see
                // WindowHandler's re-entrancy note).
                let _ = window::post_message(hwnd, msg::CLOSE, 0, 0);
            }
            _ => {}
        }
    }

    fn on_destroy(&mut self, hwnd: Hwnd) {
        for id in self.registered.drain(..) {
            let _ = hotkey::unregister(hwnd, id);
        }
        self.tray = None; // removes the icon
        log::info!("Exiting");
        window::post_quit(0);
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
            msg::HOTKEY => {
                if m.wparam == HOTKEY_DEBUG_DUMP as usize {
                    dump::log_foreground_window();
                }
            }
            msg::DISPLAYCHANGE => log::debug!("WM_DISPLAYCHANGE"),
            msg::DPICHANGED => log::debug!("WM_DPICHANGED"),
            msg::SETTINGCHANGE if m.wparam == msg::SPI_SETWORKAREA => log::debug!("Work area changed"),
            msg::DESTROY => self.on_destroy(hwnd),
            other if other != 0 && other == self.reload_msg => {
                log::info!("Reload requested by {RELOAD_MESSAGE}");
                self.reload(hwnd);
            }
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
