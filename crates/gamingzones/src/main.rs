//! GamingZones tray app.
//!
//! Single instance, tray icon, settings and logging, hotkeys, and Shift-drag snapping to
//! zones. All Win32 access goes through `gz-win`; this crate has no `unsafe`.
#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

mod app;
mod drag;
mod dump;
mod logger;
mod overlay;
mod snap;
mod zones;

use std::path::PathBuf;

use gz_win::instance::SingleInstance;
use gz_win::window::{self, MessageBoxKind};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MUTEX_NAME: &str = "Local\\GamingZones.SingleInstance";

fn main() {
    std::process::exit(run());
}

fn fail(text: &str) -> i32 {
    window::message_box(None, text, "GamingZones", MessageBoxKind::Error);
    1
}

fn run() -> i32 {
    let _instance = match SingleInstance::acquire(MUTEX_NAME) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            window::message_box(None, "GamingZones is already running.", "GamingZones", MessageBoxKind::Info);
            return 0;
        }
        Err(e) => return fail(&format!("Could not create the single-instance mutex: {}", e.message())),
    };

    let Some(appdata) = std::env::var_os("APPDATA") else {
        return fail("%APPDATA% is not set, so there is nowhere to keep settings.");
    };
    let config_dir = PathBuf::from(appdata).join("GamingZones");
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        return fail(&format!("Could not create {}: {e}", config_dir.display()));
    }
    let log_path = config_dir.join("gamingzones.log");
    if let Err(e) = logger::init(&log_path) {
        return fail(&format!("Could not open {}: {e}", log_path.display()));
    }
    log::info!("GamingZones {VERSION} starting ({})", std::env::current_exe().unwrap_or_default().display());

    if let Err(e) = app::start(config_dir) {
        log::error!("Could not create the main window: {}", e.message());
        return fail(&format!("Could not create the main window: {}", e.message()));
    }
    let code = window::run_message_loop(None);
    log::logger().flush();
    code
}
