//! The borderless toggle (PLAN Phase 5): removes the border of the window in the gaming
//! zone so the whole window is its client area, puts the border back, and removes it again
//! if the game puts it back.
//!
//! While a window is borderless, three WinEvent hooks watch it, scoped to its process and
//! filtered to that window: foreground, location change (coalesced) and destroy. Windows
//! calls them only when one of those happens, so a running game costs nothing in between.
//! The optional re-check timer (`borderless.recheck_apps`) is for games that put their
//! border back without any of those events; the app runs it only while a window of a
//! listed exe is borderless.
//!
//! The records are kept in borderless.json, so the border can still be put back after the
//! tray app restarts.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_core::borderless::{self, Reapplies, Record, Styles, BORDERLESS_FILE};
use gz_core::config;
use gz_core::GamingFit;
use gz_win::styles::{describe_ex_style, describe_style};
use gz_win::window::Message;
use gz_win::winevent::{self, event, Delivery, Filter, WinEventHook};
use gz_win::{inspect, process, window, Hwnd};

use crate::snap::{self, Snapped};

/// Index of the location-change hook in [`Watched::hooks`] (the coalesced one).
const LOCATION_HOOK: usize = 1;

struct Watched {
    hwnd: Hwnd,
    record: Record,
    hooks: Vec<WinEventHook>,
    reapplies: Reapplies,
    /// The failsafe stopped re-applying (the hooks are gone too).
    gave_up: bool,
}

pub enum Toggled {
    /// The border was removed; the client area is where it was.
    Removed(Snapped),
    /// The border is back; the client area is where it was.
    Restored(Snapped),
}

/// What a hook event or re-check did, for the app to log or tell the user.
pub enum Outcome {
    Nothing,
    Reapplied,
    /// The game kept putting its border back, so it's left alone now.
    GaveUp(String),
}

pub struct Borderless {
    path: PathBuf,
    /// The message the hooks post to the app's window.
    msg: u32,
    windows: Vec<Watched>,
    clock: Instant,
}

impl Borderless {
    pub fn new(config_dir: &Path, msg: u32) -> Self {
        Self { path: config_dir.join(BORDERLESS_FILE), msg, windows: Vec::new(), clock: Instant::now() }
    }

    /// Reads borderless.json (written before a restart) and watches the windows that still
    /// exist. Hooks post to `main`.
    pub fn load(&mut self, main: Hwnd) {
        let records = match std::fs::read_to_string(&self.path) {
            Ok(text) => match borderless::records_from_json(&text) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("{} could not be read: {e}. Starting with no borderless windows.", self.path.display());
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                log::warn!("{} could not be read: {e}", self.path.display());
                return;
            }
        };
        let count = records.len();
        for record in records {
            let Some(hwnd) = winevent::message_window(record.hwnd).filter(|&h| alive(h, record.pid)) else {
                log::debug!("Borderless: {} ({:#X}) has closed since the last run", record.exe, record.hwnd);
                continue;
            };
            log::info!("Borderless: still watching {} ({hwnd}) from the last run", record.exe);
            self.watch(main, hwnd, record);
        }
        if self.windows.len() != count {
            self.save();
        }
        for i in 0..self.windows.len() {
            self.check(i);
        }
    }

    /// GamingZones removed this window's border (and hasn't put it back).
    pub fn is_borderless(&self, hwnd: Hwnd) -> bool {
        self.windows.iter().any(|w| w.hwnd == hwnd)
    }

    /// Removes `hwnd`'s border, or puts it back if we removed it. `exe` is for the log and
    /// the re-check list.
    pub fn toggle(&mut self, main: Hwnd, hwnd: Hwnd, exe: &str) -> Result<Toggled, String> {
        self.forget_closed();
        match self.windows.iter().position(|w| w.hwnd == hwnd) {
            Some(i) => self.restore(i).map(Toggled::Restored),
            None => self.remove(main, hwnd, exe).map(Toggled::Removed),
        }
    }

    fn remove(&mut self, main: Hwnd, hwnd: Hwnd, exe: &str) -> Result<Snapped, String> {
        let saved = styles(hwnd);
        if !saved.has_border() {
            return Err("It has no border, and GamingZones didn't remove one, so there's none to put back.".into());
        }
        let window = inspect::window_rect(hwnd).map_err(|e| e.message())?;
        let client = inspect::client_rect_screen(hwnd).map_err(|e| e.message())?;
        let bare = saved.without_border();
        log::debug!(
            "Borderless: {exe} style {} ex {}; window {window}, client {client}",
            describe_style(saved.style),
            describe_ex_style(saved.ex_style)
        );
        window::set_frame(hwnd, bare.style, bare.ex_style, client).map_err(|e| e.message())?;
        let snapped = snap::snap_client(hwnd, client, GamingFit::Resize)?;
        let record = Record {
            hwnd: hwnd.as_isize(),
            pid: process::window_pid(hwnd),
            exe: exe.to_string(),
            saved,
            window,
            client,
            place: snapped.actual,
        };
        self.watch(main, hwnd, record);
        self.save();
        Ok(snapped)
    }

    fn restore(&mut self, i: usize) -> Result<Snapped, String> {
        // Removed (and its hooks dropped) first, so our own changes aren't seen as the game's.
        let w = self.windows.remove(i);
        self.save();
        let hwnd = w.hwnd;
        let back = w.record.saved.restored_over(styles(hwnd));
        let client = inspect::client_rect_screen(hwnd).map_err(|e| e.message())?;
        // The frame it had, around where the client area is now; snap_client then measures.
        let frame = w.record.window.insets_to(&w.record.client);
        window::set_frame(hwnd, back.style, back.ex_style, client.inflate(&frame)).map_err(|e| e.message())?;
        snap::snap_client(hwnd, client, GamingFit::Resize)
    }

    fn watch(&mut self, main: Hwnd, hwnd: Hwnd, record: Record) {
        let filter = Filter { window: Some(hwnd), cursor: false };
        let events =
            [(event::SYSTEM_FOREGROUND, false), (event::OBJECT_LOCATIONCHANGE, true), (event::OBJECT_DESTROY, false)];
        let hooks: Result<Vec<_>, _> = events
            .into_iter()
            .map(|(e, coalesce)| {
                WinEventHook::new(e, e, record.pid, filter, Delivery { target: main, msg: self.msg, coalesce })
            })
            .collect();
        let hooks = hooks.unwrap_or_else(|e| {
            log::warn!(
                "Borderless: can't watch {} for a border it puts back (SetWinEventHook: {})",
                record.exe,
                e.message()
            );
            Vec::new()
        });
        self.windows.push(Watched { hwnd, record, hooks, reapplies: Reapplies::default(), gave_up: false });
    }

    /// A watched window was snapped: its border is removed around its new place from now on.
    pub fn moved(&mut self, hwnd: Hwnd) {
        let Some(w) = self.windows.iter_mut().find(|w| w.hwnd == hwnd) else { return };
        if let Ok(r) = inspect::window_rect(hwnd) {
            w.record.place = r;
            self.save();
        }
    }

    /// A hook's message (wParam = event, lParam = window).
    pub fn on_event(&mut self, m: &Message) -> Outcome {
        let Some(hwnd) = winevent::message_window(m.lparam) else { return Outcome::Nothing };
        let Some(i) = self.windows.iter().position(|w| w.hwnd == hwnd) else { return Outcome::Nothing };
        match m.wparam as u32 {
            event::OBJECT_DESTROY => {
                let w = self.windows.remove(i);
                log::info!("Borderless: {} closed", w.record.exe);
                self.save();
                Outcome::Nothing
            }
            event::OBJECT_LOCATIONCHANGE => {
                if let Some(h) = self.windows[i].hooks.get(LOCATION_HOOK) {
                    h.rearm();
                }
                self.check(i)
            }
            _ => self.check(i),
        }
    }

    /// Whether the re-check timer is needed: a watched window's exe is on the list.
    pub fn wants_recheck(&self, settings: &config::Borderless) -> bool {
        self.windows.iter().any(|w| !w.gave_up && settings.rechecks(&w.record.exe))
    }

    /// The re-check timer: checks the windows of listed exes.
    pub fn recheck(&mut self, settings: &config::Borderless) -> Vec<Outcome> {
        self.forget_closed();
        let listed: Vec<usize> =
            (0..self.windows.len()).filter(|&i| settings.rechecks(&self.windows[i].record.exe)).collect();
        listed.into_iter().map(|i| self.check(i)).collect()
    }

    /// If the game put any of the border back, removes it again (within the failsafe).
    fn check(&mut self, i: usize) -> Outcome {
        let now = self.clock.elapsed().as_millis() as u64;
        let w = &mut self.windows[i];
        if w.gave_up {
            return Outcome::Nothing;
        }
        let current = styles(w.hwnd);
        if !current.has_border() {
            return Outcome::Nothing;
        }
        let exe = w.record.exe.clone();
        if !w.reapplies.allow(now) {
            w.gave_up = true;
            w.hooks.clear();
            log::warn!(
                "Borderless: {exe} put its border back {} times in {} s; leaving it alone. \
                 The hotkey still puts its original border back.",
                Reapplies::MAX + 1,
                Reapplies::WINDOW_MS / 1000
            );
            return Outcome::GaveUp(exe);
        }
        let bare = current.without_border();
        let place = w.record.place;
        log::info!(
            "Borderless: {exe} put its border back (style {} ex {}); removing it again",
            describe_style(current.style),
            describe_ex_style(current.ex_style)
        );
        if let Err(e) = window::set_frame(w.hwnd, bare.style, bare.ex_style, place) {
            log::warn!("Borderless: removing {exe}'s border again failed: {}", e.message());
        }
        Outcome::Reapplied
    }

    /// Drops windows that closed without us seeing it (no hooks, or the failsafe gave up).
    fn forget_closed(&mut self) {
        let before = self.windows.len();
        self.windows.retain(|w| alive(w.hwnd, w.record.pid));
        if self.windows.len() != before {
            self.save();
        }
    }

    fn save(&self) {
        let records: Vec<Record> = self.windows.iter().map(|w| w.record.clone()).collect();
        if let Err(e) = std::fs::write(&self.path, borderless::records_to_json(&records) + "\n") {
            log::warn!("Could not write {}: {e}", self.path.display());
        }
    }
}

fn styles(hwnd: Hwnd) -> Styles {
    Styles { style: window::style(hwnd), ex_style: window::ex_style(hwnd) }
}

/// The window still exists and belongs to the same process (handles are reused).
fn alive(hwnd: Hwnd, pid: u32) -> bool {
    inspect::exists(hwnd) && process::window_pid(hwnd) == pid
}
