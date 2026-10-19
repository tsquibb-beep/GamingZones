//! One window drag, from `EVENT_SYSTEM_MOVESIZESTART` to `EVENT_SYSTEM_MOVESIZEEND`.
//!
//! Everything a drag needs exists only while it lasts, and goes when the `Drag` is dropped:
//! - a `LOCATIONCHANGE` WinEvent hook (cursor moves and the window moving);
//! - a `DESTROY` WinEvent hook for the dragged window's process;
//! - a low-level keyboard hook, so Shift/Ctrl/Esc take effect while the mouse is still;
//! - the zone overlay window.

use gz_core::config::Settings;
use gz_core::layout::{self, zone_label, zone_range, zones_at};
use gz_core::Rect;
use gz_win::keyhook::{vk, KeyboardHook};
use gz_win::monitor::{self, MonitorIds};
use gz_win::winevent::{event, Delivery, Filter, WinEventHook};
use gz_win::{input, process, Hwnd};

use crate::overlay::Overlay;
use crate::zones::{MonitorZones, ZoneConfig};

/// Messages the hooks post to the main window.
pub struct DragMessages {
    pub location: u32,
    pub destroy: u32,
    pub key: u32,
}

/// Where to snap the window when the drag ends.
pub struct DropTarget {
    pub rect: Rect,
    /// The zones, for the log: `4`, `4+5`, `G`.
    pub label: String,
    /// The gaming zone on its own: fit the client area, not the frame.
    pub gaming: bool,
    /// Make the window topmost ([`MonitorZones::keeps_on_top`]).
    pub keep_on_top: bool,
}

pub struct Drag {
    pub window: Hwnd,
    pub exe: String,
    location_hook: Option<WinEventHook>,
    _destroy_hook: Option<WinEventHook>,
    key_hook: Option<KeyboardHook>,
    lshift: bool,
    rshift: bool,
    lctrl: bool,
    rctrl: bool,
    cancelled: bool,
    /// `snapping.require_shift` when the drag started.
    require_shift: bool,
    /// Queried when first needed (Shift pressed), not for every drag.
    ids: Option<MonitorIds>,
    /// Device of the monitor the cursor was last on (`None` = not on any).
    monitor_device: Option<String>,
    /// That monitor's zones, if it has a layout.
    monitor: Option<MonitorZones>,
    overlay: Option<Overlay>,
    /// Ctrl-span: the zones under the cursor when Ctrl went down.
    anchor: Vec<usize>,
    selected: Vec<usize>,
}

impl Drag {
    pub fn start(main: Hwnd, window: Hwnd, exe: String, require_shift: bool, msgs: &DragMessages) -> Drag {
        let location_hook = WinEventHook::new(
            event::OBJECT_LOCATIONCHANGE,
            event::OBJECT_LOCATIONCHANGE,
            0,
            Filter { window: Some(window), cursor: true },
            Delivery { target: main, msg: msgs.location, coalesce: true },
        )
        .map_err(|e| log::warn!("Drag: location hook failed ({}); zones follow key presses only", e.message()))
        .ok();
        let destroy_hook = WinEventHook::new(
            event::OBJECT_DESTROY,
            event::OBJECT_DESTROY,
            process::window_pid(window),
            Filter { window: Some(window), cursor: false },
            Delivery { target: main, msg: msgs.destroy, coalesce: false },
        )
        .map_err(|e| log::warn!("Drag: destroy hook failed: {}", e.message()))
        .ok();
        let keys = [vk::LSHIFT, vk::RSHIFT, vk::LCONTROL, vk::RCONTROL, vk::ESCAPE];
        let key_hook = KeyboardHook::new(main, msgs.key, &keys)
            .map_err(|e| log::warn!("Drag: keyboard hook failed ({}); reading keys on mouse moves", e.message()))
            .ok();
        Drag {
            window,
            exe,
            location_hook,
            _destroy_hook: destroy_hook,
            key_hook,
            lshift: input::key_down(vk::LSHIFT),
            rshift: input::key_down(vk::RSHIFT),
            lctrl: input::key_down(vk::LCONTROL),
            rctrl: input::key_down(vk::RCONTROL),
            cancelled: false,
            require_shift,
            ids: None,
            monitor_device: None,
            monitor: None,
            overlay: None,
            anchor: Vec::new(),
            selected: Vec::new(),
        }
    }

    pub fn shift(&self) -> bool {
        self.lshift || self.rshift
    }

    /// Whether zones show: Shift held, or (with `require_shift` off) Shift not held.
    fn zones_on(&self) -> bool {
        self.shift() == self.require_shift
    }

    fn ctrl(&self) -> bool {
        self.lctrl || self.rctrl
    }

    /// The location hook posted: allow its next message.
    pub fn rearm(&self) {
        if let Some(hook) = &self.location_hook {
            hook.rearm();
        }
    }

    /// A watched key went down or up (from the keyboard hook).
    pub fn on_key(&mut self, key: u16, down: bool) {
        match key {
            vk::LSHIFT => self.lshift = down,
            vk::RSHIFT => self.rshift = down,
            vk::LCONTROL => self.lctrl = down,
            vk::RCONTROL => self.rctrl = down,
            vk::ESCAPE if down => {
                // Esc ends the system's move loop and puts the window back; don't snap.
                log::info!("Drag of {} cancelled with Esc", self.exe);
                self.cancelled = true;
                self.clear();
            }
            _ => {}
        }
    }

    /// Re-reads the cursor and updates the overlay and the selection.
    pub fn update(&mut self, settings: &Settings, zones: &ZoneConfig) {
        if self.cancelled {
            return;
        }
        if self.key_hook.is_none() {
            self.lshift = input::key_down(vk::LSHIFT);
            self.rshift = input::key_down(vk::RSHIFT);
            self.lctrl = input::key_down(vk::LCONTROL);
            self.rctrl = input::key_down(vk::RCONTROL);
        }
        if !self.zones_on() {
            self.clear();
            return;
        }
        let Some((x, y)) = input::cursor_pos() else { return };

        let monitor = monitor::from_point(x, y);
        let device = monitor.as_ref().map(|m| m.device.clone());
        if device != self.monitor_device {
            self.monitor_device = device;
            let ids = self.ids.get_or_insert_with(MonitorIds::query);
            self.monitor = monitor.and_then(|m| zones.for_monitor(&m, ids, settings.snapping.deadzone_px));
            self.overlay = None;
            self.anchor.clear();
            self.selected.clear();
            if let Some(mz) = &self.monitor {
                log::debug!("Drag: cursor on {} (\"{}\", {} zones)", mz.device, mz.layout_name, mz.zones.len());
            }
        }
        let Some(mz) = &self.monitor else { return };

        let current = zones_at(&mz.zones, x, y, settings.snapping.edge_radius(), settings.snapping.overlap);
        let selected = if self.ctrl() {
            if self.anchor.is_empty() {
                self.anchor = current.clone();
                current
            } else {
                zone_range(&mz.zones, &self.anchor, &current)
            }
        } else {
            self.anchor.clear();
            current
        };

        if self.overlay.is_none() {
            match Overlay::create(mz.overlay_area(), mz.zones.clone(), mz.dpi, &settings.overlay) {
                Ok(o) => {
                    log::debug!("Drag: overlay created on {} at {}", mz.device, o.area());
                    self.overlay = Some(o);
                }
                Err(e) => log::warn!("Could not create the zone overlay: {}", e.message()),
            }
        }
        if let Some(overlay) = &self.overlay {
            overlay.set_selected(&selected);
            overlay.show_below(self.window);
        }
        self.selected = selected;
    }

    /// Hides the overlay and forgets the selection (Shift changed, Esc).
    fn clear(&mut self) {
        if let Some(o) = &self.overlay {
            o.hide();
        }
        self.anchor.clear();
        self.selected.clear();
    }

    /// Ends the drag and returns where to snap: the selected zones' bounding rectangle.
    /// Hooks and overlay are removed (by dropping `self`) before the caller moves any
    /// window, so a slow app can't hold up keyboard input through our keyboard hook.
    pub fn finish(self) -> Option<DropTarget> {
        if self.cancelled || !self.zones_on() || self.selected.is_empty() {
            return None;
        }
        let mz = self.monitor.as_ref()?;
        let rect = layout::bounding_rect(&mz.zones, &self.selected)?;
        let label = self.selected.iter().map(|&i| zone_label(&mz.zones, i)).collect::<Vec<_>>().join("+");
        let gaming = matches!(self.selected[..], [i] if mz.zones[i].gaming);
        let keep_on_top = matches!(self.selected[..], [i] if mz.keeps_on_top(i));
        Some(DropTarget { rect, label, gaming, keep_on_top })
    }
}

impl Drop for Drag {
    fn drop(&mut self) {
        // Overlay first, then the hooks (fields drop after this).
        self.overlay = None;
    }
}
