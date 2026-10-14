//! GamingZones layout editor (`gz-editor.exe`), launched from the tray menu.
//!
//! An egui app in its own process, so the always-running tray app stays small: it exits
//! completely when its window closes. It edits `layouts.json` and `applied.json`, then
//! asks the tray app to reload. All Win32 access goes through `gz-win`.
#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

mod app;
mod canvas;
mod capture;
mod core_link;
mod doc;
mod gaming;
mod monitors;
#[cfg(test)]
mod ui_tests;

use eframe::egui;
use gz_win::instance::SingleInstance;
use gz_win::window::{self, MessageBoxKind};

const TITLE: &str = "GamingZones Editor";
const MUTEX_NAME: &str = "Local\\GamingZones.Editor";
/// Window icon (64×64 RGBA, drawn by tools/make_icons.py).
const ICON: &[u8] = include_bytes!("../../../res/gz-editor.rgba");

fn main() {
    let _instance = match SingleInstance::acquire(MUTEX_NAME) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            // Already open: bring that window to the front instead.
            if let Some(w) = core_link::other_editor_window() {
                window::activate(w);
            }
            return;
        }
        Err(_) => None,
    };
    let Some(dir) = gz_core::ipc::config_dir() else {
        window::message_box(
            None,
            "%APPDATA% is not set, so there are no layouts to edit.",
            TITLE,
            MessageBoxKind::Error,
        );
        return;
    };
    let editor = app::Editor::load(dir);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_icon(egui::IconData { rgba: ICON.to_vec(), width: 64, height: 64 }),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(TITLE, options, Box::new(|_cc| Ok(Box::new(editor)))) {
        window::message_box(None, &format!("The editor could not start: {e}"), TITLE, MessageBoxKind::Error);
    }
}
