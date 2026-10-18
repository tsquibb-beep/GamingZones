//! GamingZones layout editor (`gz-editor.exe`) and settings window (`gz-editor.exe
//! --settings`), launched from the tray menu.
//!
//! An egui app in its own process, so the always-running tray app stays small: it exits
//! completely when its window closes. The editor edits `layouts.json` and `applied.json`,
//! the settings window `settings.json`; both then ask the tray app to reload. Each keeps
//! itself to one window, and both can be open at once. All Win32 access goes through `gz-win`.
#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

mod app;
mod canvas;
mod capture;
mod core_link;
mod doc;
mod gaming;
mod help;
mod icons;
mod list;
mod monitors;
mod settings;
#[cfg(test)]
mod settings_tests;
mod style;
#[cfg(test)]
mod ui_tests;

use eframe::egui;
use gz_win::instance::SingleInstance;
use gz_win::window::{self, MessageBoxKind};

const TITLE: &str = "GamingZones Editor";
const MUTEX_NAME: &str = "Local\\GamingZones.Editor";
const SETTINGS_TITLE: &str = "GamingZones Settings";
const SETTINGS_MUTEX: &str = "Local\\GamingZones.Settings";
/// Window icon (64×64 RGBA, drawn by tools/make_icons.py).
const ICON: &[u8] = include_bytes!("../../../res/gz-editor.rgba");

fn main() {
    let settings = std::env::args().skip(1).any(|a| a == "--settings");
    let (title, mutex) = if settings { (SETTINGS_TITLE, SETTINGS_MUTEX) } else { (TITLE, MUTEX_NAME) };
    let _instance = match SingleInstance::acquire(mutex) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            // Already open: bring that window to the front instead.
            if let Some(w) = core_link::other_window(title) {
                window::activate(w);
            }
            return;
        }
        Err(_) => None,
    };
    let Some(dir) = gz_core::ipc::config_dir() else {
        window::message_box(
            None,
            "%APPDATA% is not set, so there are no settings or layouts to edit.",
            title,
            MessageBoxKind::Error,
        );
        return;
    };
    let (size, min_size, app): ([f32; 2], [f32; 2], Box<dyn eframe::App>) = if settings {
        ([760.0, 1040.0], [620.0, 480.0], Box::new(settings::SettingsWindow::load(&dir)))
    } else {
        ([1760.0, 760.0], [1200.0, 560.0], Box::new(app::Editor::load(dir)))
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(size)
            .with_min_inner_size(min_size)
            .with_icon(egui::IconData { rgba: ICON.to_vec(), width: 64, height: 64 }),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        title,
        options,
        Box::new(|cc| {
            style::install(&cc.egui_ctx);
            Ok(app)
        }),
    ) {
        window::message_box(None, &format!("The window could not start: {e}"), title, MessageBoxKind::Error);
    }
}
