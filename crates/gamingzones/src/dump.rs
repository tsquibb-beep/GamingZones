//! The debug hotkey: logs everything we know about a window, in physical pixels.

use gz_win::inspect::{self, Elevation, WindowInfo};
use gz_win::{dpi, process, styles, Hwnd};

use crate::logger::DUMP;

pub fn log_foreground_window() {
    let Some(hwnd) = inspect::foreground_window() else {
        log::info!(target: DUMP, "Debug dump: there is no foreground window");
        return;
    };
    match inspect::window_info(hwnd) {
        Ok(info) => log::info!(target: DUMP, "{}", format(&info)),
        Err(e) => log::info!(target: DUMP, "Debug dump: window {hwnd} vanished: {}", e.message()),
    }
}

fn format(w: &WindowInfo) -> String {
    let mut out = String::from("Debug dump: foreground window\r\n");
    let mut line = |label: &str, value: String| {
        out.push_str(&format!("    {label:<17}{value}\r\n"));
    };

    line("Window", format!("{} class \"{}\" title \"{}\"", w.hwnd, w.class, w.title));
    line(
        "Process",
        match &w.exe {
            Ok(path) => format!("{} (pid {}) {path}", process::file_name(path), w.pid),
            Err(e) => format!("pid {} (exe unavailable: {e})", w.pid),
        },
    );
    line(
        "Elevated",
        match &w.elevation {
            Elevation::Elevated => "yes".into(),
            Elevation::NotElevated => "no".into(),
            Elevation::Unknown(e) => format!("unknown ({e}); usually means it is elevated and we aren't"),
        },
    );
    let state = if w.minimized {
        "minimised"
    } else if w.maximized {
        "maximised"
    } else {
        "normal"
    };
    line(
        "State",
        format!(
            "{state}, {}, cloaked: {}",
            if w.visible { "visible" } else { "hidden" },
            if w.cloaked == 0 { "no".to_string() } else { format!("yes ({:#X})", w.cloaked) }
        ),
    );
    line("Window rect", format!("{}  (GetWindowRect)", w.window_rect));
    line("Client rect", format!("{}  (screen coordinates)", w.client_rect));
    match &w.frame_bounds {
        Ok(frame) => {
            line("Frame bounds", format!("{frame}  (DWMWA_EXTENDED_FRAME_BOUNDS)"));
            line("Invisible border", format!("{}  (window rect → frame)", w.window_rect.insets_to(frame)));
            line("Frame → client", format!("{}", frame.insets_to(&w.client_rect)));
        }
        Err(e) => line("Frame bounds", format!("unavailable: {e}")),
    }
    line("Window → client", format!("{}", w.window_rect.insets_to(&w.client_rect)));
    line("DPI", format!("{} ({}%), awareness: {}", w.dpi, dpi::percent(w.dpi), w.dpi_awareness));
    match &w.monitor {
        Some(m) => {
            line(
                "Monitor",
                format!(
                    "{}{} {}, DPI {} ({}%)",
                    m.device,
                    if m.primary { " (primary)" } else { "" },
                    m.rect,
                    m.dpi,
                    dpi::percent(m.dpi)
                ),
            );
            line("Work area", format!("{}", m.work));
        }
        None => line("Monitor", "unavailable".into()),
    }
    line("Style", styles::describe_style(w.style));
    line("Ex-style", styles::describe_ex_style(w.ex_style));
    out.truncate(out.trim_end().len());
    out
}

/// Startup facts worth having in every log.
pub fn log_self(hwnd: Hwnd) {
    if !dpi::thread_is_per_monitor_v2() {
        log::error!("Not running Per-Monitor-V2 DPI aware: the manifest did not apply. Coordinates will be wrong.");
    }
    match process::current_is_elevated() {
        Ok(elevated) => log::info!("Running elevated: {}", if elevated { "yes" } else { "no" }),
        Err(e) => log::warn!("Could not check our own elevation: {}", e.message()),
    }
    log::debug!("Message window {hwnd}");
}
