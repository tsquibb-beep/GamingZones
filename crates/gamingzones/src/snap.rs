//! Which windows can be snapped, and snapping one to a zone.

use std::fmt;

use gz_core::config::Snapping;
use gz_core::{GamingFit, Insets, Rect};
use gz_win::inspect;
use gz_win::styles::ws;
use gz_win::{process, window, Hwnd};

/// Why a window is left alone.
#[derive(Debug)]
pub enum Skip {
    Minimized,
    Hidden,
    ToolWindow,
    NotTopLevel,
    /// A popup without a resizable frame and caption: menus, flyouts, splash screens.
    Popup,
    /// Owned by a visible window: a dialog.
    Dialog,
    Cloaked,
    System(String),
    Excluded(String),
    Own,
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Minimized => write!(f, "minimised"),
            Self::Hidden => write!(f, "not visible"),
            Self::ToolWindow => write!(f, "tool window"),
            Self::NotTopLevel => write!(f, "not a top-level window"),
            Self::Popup => write!(f, "popup without a resizable caption"),
            Self::Dialog => write!(f, "dialog owned by another window"),
            Self::Cloaked => write!(f, "cloaked"),
            Self::System(what) => write!(f, "system window ({what})"),
            Self::Excluded(exe) => write!(f, "{exe} is in snapping.excluded_apps"),
            Self::Own => write!(f, "one of GamingZones' own windows"),
        }
    }
}

/// A window that may be snapped.
pub struct Target {
    pub exe: String,
    /// Its process is elevated and we aren't, so Windows won't let us move it.
    pub elevated: bool,
}

/// Window classes that are never snapped (desktop, taskbar, Office splash, UWP host).
const SYSTEM_CLASSES: &[&str] =
    &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "MsoSplash", "Windows.UI.Core.CoreWindow"];

/// How the window was picked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    /// Being dragged: FancyZones' rules apply.
    Drag,
    /// The foreground window, by hotkey: the user chose it, so popups (borderless games)
    /// and owned windows are allowed too.
    Hotkey,
}

/// FancyZones' rules (`FancyZonesWindowProcessing.cpp`, see reference/NOTES.md §3), plus
/// cloaked windows and the exclusion list.
pub fn check(hwnd: Hwnd, snapping: &Snapping, self_elevated: bool, picked: Picked) -> Result<Target, Skip> {
    if inspect::is_minimized(hwnd) {
        return Err(Skip::Minimized);
    }
    if !inspect::is_visible(hwnd) {
        return Err(Skip::Hidden);
    }
    let style = window::style(hwnd);
    if window::ex_style(hwnd) & ws::EX_TOOLWINDOW != 0 {
        return Err(Skip::ToolWindow);
    }
    if inspect::root(hwnd) != Some(hwnd) {
        return Err(Skip::NotTopLevel);
    }
    if style & ws::POPUP != 0 && picked == Picked::Drag {
        let resizable = style & ws::THICKFRAME != 0;
        let captioned = style & (ws::CAPTION | ws::MINIMIZEBOX | ws::MAXIMIZEBOX) != 0;
        if !(resizable && captioned) {
            return Err(Skip::Popup);
        }
    }
    if let Some(owner) = inspect::owner(hwnd).filter(|_| picked == Picked::Drag) {
        let owner_has_size = inspect::window_rect(owner).is_ok_and(|r| !r.is_empty());
        if inspect::is_visible(owner) && owner_has_size {
            return Err(Skip::Dialog);
        }
    }
    if inspect::cloaked(hwnd) != 0 {
        return Err(Skip::Cloaked);
    }
    let class = inspect::class_name(hwnd);
    if SYSTEM_CLASSES.contains(&class.as_str()) {
        return Err(Skip::System(class));
    }

    let pid = process::window_pid(hwnd);
    if pid == std::process::id() {
        return Err(Skip::Own);
    }
    let path = process::exe_path(pid).unwrap_or_default();
    let exe = process::file_name(&path).to_string();
    if path.to_ascii_uppercase().contains(r"\SYSTEMAPPS\") || exe.eq_ignore_ascii_case("SearchUI.exe") {
        return Err(Skip::System(exe));
    }
    if snapping.is_excluded(&exe) {
        return Err(Skip::Excluded(exe));
    }
    // If the elevation check itself fails we still try; a refused move is reported then.
    let elevated = !self_elevated && process::is_elevated(pid).unwrap_or(false);
    Ok(Target { exe: if exe.is_empty() { format!("pid {pid}") } else { exe }, elevated })
}

/// What a snap achieved.
pub struct Snapped {
    pub target: Rect,
    /// The visible frame (or, for the gaming zone, the client area) afterwards.
    pub actual: Rect,
    pub passes: u32,
}

impl Snapped {
    pub fn exact(&self) -> bool {
        self.actual == self.target
    }
}

/// Moves `hwnd` so its *visible frame* (DWM extended frame bounds) fills `zone`.
///
/// Windows 10/11 windows have invisible resize borders outside the visible frame, so the
/// window rect is the zone grown by those borders (measured, not assumed). A window
/// without a resizable frame keeps its size and moves to the zone's top-left corner, as
/// in FancyZones. If the frame isn't exact afterwards (a move to a monitor with another
/// DPI makes the app resize itself and changes the borders), it measures and tries again.
pub fn snap_frame(hwnd: Hwnd, zone: Rect) -> Result<Snapped, String> {
    let resizable = window::style(hwnd) & ws::THICKFRAME != 0;
    let mut target = zone;
    let mut frame = Rect::default();
    for pass in 1..=3 {
        let win = inspect::window_rect(hwnd).map_err(|e| e.message())?;
        frame = inspect::frame_bounds(hwnd).unwrap_or(win);
        if !resizable {
            target = Rect::from_xywh(zone.left, zone.top, frame.width(), frame.height());
        }
        if frame == target {
            return Ok(Snapped { target, actual: frame, passes: pass - 1 });
        }
        let borders: Insets = win.insets_to(&frame);
        window::set_window_rect(hwnd, target.inflate(&borders)).map_err(|e| e.message())?;
    }
    if let Ok(f) = inspect::frame_bounds(hwnd) {
        frame = f;
    }
    Ok(Snapped { target, actual: frame, passes: 3 })
}

/// Moves `hwnd` so its *client area* fills `zone` (the gaming zone), with the frame and
/// caption outside it. With [`GamingFit::Centre`] the client keeps its size and is centred
/// in the zone instead. Measures and retries like [`snap_frame`].
pub fn snap_client(hwnd: Hwnd, zone: Rect, fit: GamingFit) -> Result<Snapped, String> {
    let mut target = zone;
    let mut client = Rect::default();
    for pass in 1..=3 {
        let win = inspect::window_rect(hwnd).map_err(|e| e.message())?;
        client = inspect::client_rect_screen(hwnd).map_err(|e| e.message())?;
        if fit == GamingFit::Centre {
            target = centred(zone, client.width(), client.height());
        }
        if client == target {
            return Ok(Snapped { target, actual: client, passes: pass - 1 });
        }
        let frame: Insets = win.insets_to(&client);
        window::set_window_rect(hwnd, target.inflate(&frame)).map_err(|e| e.message())?;
    }
    if let Ok(c) = inspect::client_rect_screen(hwnd) {
        client = c;
    }
    Ok(Snapped { target, actual: client, passes: 3 })
}

/// A `w × h` rect centred in `zone` (rounding towards the top-left).
fn centred(zone: Rect, w: i32, h: i32) -> Rect {
    let x = zone.left + (zone.width() - w).div_euclid(2);
    let y = zone.top + (zone.height() - h).div_euclid(2);
    Rect::from_xywh(x, y, w, h)
}
