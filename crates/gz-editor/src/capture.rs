//! "Capture size from window": the open windows and their client sizes.

use gz_win::styles::ws;
use gz_win::{inspect, process, window};

pub struct WindowEntry {
    pub title: String,
    pub exe: String,
    pub width: i32,
    pub height: i32,
}

/// Visible, normal top-level windows of other processes, front to back.
pub fn windows() -> Vec<WindowEntry> {
    let me = std::process::id();
    inspect::top_level_windows()
        .into_iter()
        .filter(|&w| {
            inspect::is_visible(w)
                && !inspect::is_minimized(w)
                && inspect::cloaked(w) == 0
                && window::ex_style(w) & ws::EX_TOOLWINDOW == 0
                && process::window_pid(w) != me
        })
        .filter_map(|w| {
            let title = window::text(w);
            let client = inspect::client_rect_screen(w).ok()?;
            if title.trim().is_empty() || client.is_empty() {
                return None;
            }
            let path = process::exe_path(process::window_pid(w)).unwrap_or_default();
            Some(WindowEntry {
                title,
                exe: process::file_name(&path).to_string(),
                width: client.width(),
                height: client.height(),
            })
        })
        .collect()
}
