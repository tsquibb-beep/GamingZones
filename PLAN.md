# GamingZones — Build Plan

A lightweight FancyZones-style window manager for Windows with an optional **gaming zone**: a zone sized to an exact game resolution (or aspect ratio) that the other zones arrange themselves around. A later phase adds a hotkey to toggle the border on the game window.

This file is the source of truth for scope and order. Work one phase at a time. At the end of each phase, stop and hand over the manual test checklist.

---

## 1. Decisions (locked)

| Area | Decision |
|---|---|
| Language | Rust (stable), cross-compiled from WSL to `x86_64-pc-windows-gnu` (MinGW, static CRT; no MSVC on the dev machine — agreed 2026-10-13). Windows 10 22H2+ / Windows 11, x64 |
| Win32 bindings | `windows` crate (Microsoft) |
| Editor / settings UI | `eframe`/`egui`, in a **separate executable** launched on demand, so the always-running core stays tiny |
| Licence | GPL-3.0 |
| Units | Physical pixels everywhere (config, UI, maths). Per-Monitor-V2 DPI aware via manifest in both executables |
| Layout area | Monitor **work area** (excludes taskbar) by default. Per-layout option: "gaming zone may extend over the taskbar" |
| Window movement | Windows move **only** when the user snaps them. Changing layouts, moving the gaming zone or deadzoning a zone never moves existing windows |
| Gaming zone | Optional per layout. Data model stores a list; UI limits it to one for now |
| Elevation | Runs non-elevated by default. Detect elevated target windows and notify instead of failing silently. Optional elevated start via Scheduled Task (Phase 4) |
| Multi-monitor | Supported (layout per monitor). Primary test rig is a single 6144×2560 monitor at 125% scaling |
| Default hotkeys | Avoid Windows 11's own `Win+Alt+…` keys (D = clock, B = HDR, G = Game Bar). Use `Win+Ctrl+Alt+…` and FancyZones' keys (FancyZones will be disabled). All configurable in `settings.json` |

### Performance rules (non-negotiable)
- **Zero idle work.** No polling timers, no busy loops while the user is gaming.
- **No persistent low-level hooks** (`WH_KEYBOARD_LL` / `WH_MOUSE_LL`). If one is needed during a drag, install it at drag start and remove it at drag end.
- Global shortcuts via `RegisterHotKey` (with `MOD_NOREPEAT`).
- Window drag detection via `SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART..EVENT_SYSTEM_MOVESIZEEND, WINEVENT_OUTOFCONTEXT)`.
- Modifier state (Shift/Ctrl) read during drags only: `GetAsyncKeyState` on cursor moves, plus a drag-scoped `WH_KEYBOARD_LL` hook so pressing Shift or Ctrl with a still mouse updates at once (agreed 2026-10-13).
- Overlay windows are created at drag start and destroyed at drag end.
- Targets: 0.0% CPU at idle (Task Manager, 10 min observation); core process private bytes under ~10 MB.

### Safety rules
- No DLL injection, no remote threads, no `OpenProcess` with VM read/write rights.
- Only `PROCESS_QUERY_LIMITED_INFORMATION` (for exe name / elevation check).
- All window manipulation is external: `SetWindowPos`, `SetWindowLongPtrW`, `DwmGetWindowAttribute`.

---

## 2. Architecture

```
gamingzones/                (cargo workspace)
├─ crates/gz-core/          pure Rust: geometry, layout model, guide resolver,
│                           aspect maths, config (serde). No Win32. Unit-tested.
├─ crates/gz-win/           the only crate with `unsafe`: thin safe wrappers over the
│                           Win32 calls the apps need (shared by gamingzones and ruler)
├─ crates/gamingzones/      tray app (always running): hotkeys, WinEvent hook,
│                           drag handling, overlay, snapping, borderless toggle
├─ crates/gz-editor/        egui app: layout editor + settings (launched on demand)
├─ crates/ruler/            test utility window (see Phase 0)
├─ res/                     PMv2 manifest, icons, shared build.rs (resources + version info)
├─ reference/               gitignored: sparse clones of reference source
└─ PLAN.md, THIRD_PARTY_NOTICES.md, LICENSE, version.txt (CLAUDE.md is gitignored)
```

- **Config**: JSON in `%APPDATA%\GamingZones\`: `settings.json`, `layouts.json`, `applied.json` (layout per monitor).
- **Monitor identity**: a stable ID (DisplayConfig target device path), with fallback to the device name plus resolution.
- **Editor → core notification**: the editor saves JSON, then posts a registered window message (`RegisterWindowMessageW("GamingZones.Reload")`) to the core's hidden message window. The core reloads.
- **Display changes**: on `WM_DISPLAYCHANGE`, `WM_DPICHANGED` and `WM_SETTINGCHANGE` (`SPI_SETWORKAREA`), recompute zone geometry. Never move windows.

### Layout model (gz-core)

```text
Layout {
  id, name,
  gaming_zones: Vec<GamingZone>,        // UI allows 0 or 1
  gaming_over_taskbar: bool,
  guides: Vec<Guide>,
  zones: Vec<Zone>,
}

GamingZone {
  size: Pixels { w, h }
      | Aspect { ratio_w, ratio_h, k }          // size = k*(rw/g) x k*(rh/g), g = gcd → always exact
      | IntegerScale { base_w, base_h, scale },
  x, y,                                          // top-left, px, relative to layout area
}

Guide { id, axis: Vertical|Horizontal, anchor }
  anchor = Absolute(px) | Percent(of area) | Between { a, b, fraction }
         | Offset { from, px }
  Built-in guides: area.left/right/top/bottom; gz0.left/right/top/bottom (when a gaming zone exists)

Zone { id, left, right, top, bottom }            // guide ids
```

- **Resolver**: evaluate guides in dependency order (reject cycles), then produce zone rects for a given area.
- A zone is **disabled** if its width or height is below `deadzone_threshold_px`, or is ≤ 0.
- Gaming zone placement is clamped to the area. With `gaming_over_taskbar`, the clamp uses the full monitor rect instead of the work area.
- **Editor splits**: splitting zone Z vertically at f creates `Between{Z.left, Z.right, f}` and replaces Z with two zones. Splits therefore stay proportional when the gaming zone moves. Merging takes the outer guides of a rectangular selection. Unreferenced guides are garbage-collected.
- **Aspect sizing** only offers exact sizes (e.g. 16:9 steps through 16k × 9k) and always displays the real W×H.

### Snapping rules
- **Normal zone**: the window's *visible frame* (`DWMWA_EXTENDED_FRAME_BOUNDS`) fills the zone exactly. Compensate for the invisible resize borders, as FancyZones does.
- **Gaming zone**: the window's *client area* equals the zone exactly, and the frame sits outside it.
  - Setting for when the client size differs from the zone: **Resize to fit** (default) or **Centre without resizing**.
- **Overlapping zones**: the smallest zone under the cursor wins (setting can offer other rules later).
- **Ctrl-span**: while Shift-dragging, hold Ctrl and move across zones. As in FancyZones, the selection is every zone inside the bounding rectangle of the zone where Ctrl was pressed and the zone under the cursor (a range, not a cumulative add). The window snaps to the bounding rectangle of the selection.
- **Edge spanning** (optional, off by default): with it on, dropping within `edge_span_radius_px` of a border shared by two zones snaps to both, as FancyZones does. Off, the overlap rule picks one zone.
- **Skip**: tool windows, popups without a caption, cloaked windows, minimised windows, windows of excluded exes (settings list), and the app's own windows.
- **Maximised windows**: restore them before snapping.
- **Elevated windows**: if the target belongs to an elevated process and we are not elevated, show a tray balloon/toast once per window and do nothing.

---

## 3. Tom's layouts (reference set for testing)

Proportions are approximate from FancyZones screenshots. Tom will fine-tune them.

1. **Gaming-Focus** (gaming zone)
   - Gaming zone: placeholder 3840×2160, centred horizontally, vertically centred in the work area.
   - Left column: area.left → gz.left, full height, split into 2 rows at 50%.
   - Right column: gz.right → area.right, full height, split into 2 rows at 50%.
   - Top strip: gz.left → gz.right, area.top → gz.top.
   - Bottom strip: gz.left → gz.right, gz.bottom → area.bottom.
2. **Split focus** (no gaming zone)
   - Columns ≈ 21% / 58% / 21%.
   - Side columns: 2 rows at 50%.
   - Centre column: top strip ≈ 15%, bottom strip ≈ 15%, middle split into 2 columns at 50%.
3. **WFH** (no gaming zone): 2×2 grid at 50% / 50%.
4. **Slay** (no gaming zone): 3 columns ≈ 26% / 48% / 26%.

---

## 4. Phases

Each phase ends with: `cargo build --release`, `cargo test`, `cargo clippy -- -D warnings` all clean. Update the checklist in this file, write a short summary of what changed, and give Tom a **manual test checklist**. Then **stop** and wait for Tom's go-ahead.

### Phase 0 — Skeleton and test kit
- [x] Cargo workspace as above; release profile with LTO, `opt-level = 3`, `panic = "abort"`, `strip = true`.
- [x] Sparse shallow clone of PowerToys FancyZones and Borderless Gaming into `reference/` (gitignored). Write `reference/NOTES.md` summarising the relevant FancyZones code paths: drag handling, DWM frame adjustment, window filtering, overlay drawing, layout JSON shape.
- [x] Core tray app:
  - Single instance (named mutex).
  - Hidden message window and tray icon. Menu: Reload config, Open editor (disabled until Phase 3), Settings (disabled until Phase 4), Exit.
  - Per-Monitor-V2 manifest.
  - Config dir creation; file logging with a configurable level.
- [x] Hotkey plumbing (`RegisterHotKey`), with failures logged clearly.
- [x] Debug hotkey (default `Win+Ctrl+Alt+I`) logs for the foreground window:
  - exe name, elevated or not;
  - `GetWindowRect`, client rect in screen coordinates, `DWMWA_EXTENDED_FRAME_BOUNDS`;
  - window DPI, monitor rect, work area, style and ex-style.
- [x] **Ruler** utility:
  - A resizable window that live-displays its client W×H, window rect, frame bounds and DPI in physical px.
  - A box to type an exact client size and apply it.
  - A toggle to remove `WS_THICKFRAME`, so it can simulate a fixed-size game window.

**Done when**
- Idle CPU is 0.0% over 10 minutes.
- Private bytes are recorded in the summary.
- On Tom's 6144×2560 @125% monitor, the debug log and the ruler agree to the pixel.
- The ruler set to 1920×1080 reports a 1920×1080 client.

**Result (2026-10-13):** all of the above confirmed by Tom's manual checklist. Idle CPU 0% with CPU time static; private bytes ≈ 1.7 MB.

### Phase 1 — Zone engine and Shift-drag snapping (no gaming zone)
> **Decisions (agreed 2026-10-13)**, after reading FancyZones (see `reference/NOTES.md`):
> 1. Shift/Ctrl pressed with a still mouse: a `WH_KEYBOARD_LL` hook installed at drag start and removed at drag end.
> 2. Ctrl-span: FancyZones' range from the anchor zone to the current zone.
> 3. Overlay: one constant opacity (`SetLayeredWindowAttributes`), painted normally; no per-pixel alpha bitmap.
> 4. Overlay z-order: directly under the dragged window, as FancyZones does.
> 5. Edge spanning: an optional setting, off by default (`settings.json` in Phase 1, settings UI in Phase 4).
>
> Also: skip resizes (MOVESIZESTART fires for them too; FancyZones checks for a resize cursor), and
> skip snapping if the window is maximised at drag end (Aero Snap / browser tab merge).

- [x] gz-core: guides, zones, resolver, deadzone flagging, JSON (de)serialisation. Unit tests for:
  - the three non-gaming reference layouts;
  - cycle detection;
  - zero and negative widths.
- [x] Hand-written `layouts.json` with Split focus, WFH and Slay; `applied.json` per monitor.
  (Both are written on first run if missing: layouts from `gz-core/src/default_layouts.json`, and `applied.json` with every connected monitor's stable id on the first layout.)
- [x] Drag handling:
  1. On `MOVESIZESTART`, check the window is snappable.
  2. Track the cursor and modifiers while dragging, using a mechanism that exists only during the drag.
  3. Show the overlay when Shift is held; hide it on Shift release.
  4. Highlight the zone under the cursor.
  5. Ctrl-span multi-select.
  6. On `MOVESIZEEND`, snap if a zone is selected.
  7. Tear everything down.
- [x] Overlay: per-monitor layered, click-through window placed directly under the dragged window. Draws zones, highlighted zone(s) and zone numbers. Colours and opacity come from settings.
- [x] Snapping to the visible frame with DWM compensation; skip and elevated rules as in §2.
- [x] Multi-monitor: zones come from the monitor under the cursor.

**Done when**
- The ruler snapped into every zone of every layout has its visible frame exactly equal to the zone rect (verified with the debug hotkey).
- Ctrl-span across two zones gives their bounding rectangle.
- The log confirms no hook or timer survives after a drag.
- Idle CPU is still 0.0%.

**Built (2026-10-13), awaiting Tom's manual test.** Also: `settings.json` gained `snapping` (edge spanning, radius,
excluded apps) and `overlay` (colours, opacity, numbers); an older `settings.json` gets the new keys added on load.
Esc during a drag cancels the snap. Fixed-size windows keep their size and go to the zone's top-left (as FancyZones).

### Phase 2 — Gaming zone
- [ ] gz-core: `GamingZone`, `gz*` built-in guides, aspect and integer-scale maths with unit tests, clamping, `gaming_over_taskbar`.
- [ ] Add Gaming-Focus to `layouts.json`.
- [ ] Client-area snapping for the gaming zone; "resize to fit / centre" setting.
- [ ] Deadzone threshold setting (default 400 px). Disabled zones are drawn greyed in the overlay (setting: greyed or hidden) and are never snap targets.
- [ ] Hotkey to snap the active window to the gaming zone (default `Win+Ctrl+Alt+G`), so borderless or fixed windows can be placed without dragging.

**Done when**
- Ruler at 1920×1080 snapped to a 1920×1080 gaming zone has client rect == zone rect exactly.
- Editing the gaming zone position in JSON and reloading resizes the neighbours per their anchors.
- A zone pushed below the threshold is greyed and can't be snapped to.
- With `gaming_over_taskbar`, a gaming zone taller than the work area is placed correctly.
- Tested with at least one real windowed game.

### Phase 3 — Layout editor (`gz-editor.exe`)
- [ ] Layout list with preview cards, like FancyZones: create, duplicate, rename, delete; launched from the tray.
- [ ] Canvas showing a scaled preview of the selected monitor's area:
  - click a zone to split it (Shift toggles horizontal/vertical);
  - drag dividers;
  - select a divider and type its position in px or %;
  - select adjacent zones and merge them;
  - delete zones.
- [ ] Gaming zone panel:
  - on/off;
  - size mode (Pixels / Aspect + scale / Integer scale of a base), with presets (1280×720, 1600×900, 1920×1080, 2560×1440, 3840×2160, 4:3, 16:10, 21:9) and an exact W×H readout;
  - X/Y inputs; drag the body or its edges to move it (edges move, never resize); align buttons (centre, left, right, top, bottom);
  - "Capture size from window": pick from a list of open top-level windows and read their client size;
  - "extend over taskbar" tick box.
- [ ] Live deadzone preview against the current threshold.
- [ ] "Preview on screen": the core shows the real overlay for the layout being edited.
- [ ] Save writes JSON and posts the reload message; the editor process exits fully on close.

**Done when**
- Tom recreates all four reference layouts in the editor.
- Their JSON resolves to the same rects as the hand-written versions.
- Moving the gaming zone in the editor visibly reflows the neighbours.
- The core's memory is unchanged after the editor closes.

### Phase 4 — Layout selector, shortcuts, settings
- [ ] Layout selector popup:
  - hotkey (default ``Win+Shift+` ``, FancyZones' editor key); preview cards; number keys to choose;
  - applies to the monitor with the foreground window.
- [ ] Quick-switch hotkeys `Win+Ctrl+Alt+1..9`.
- [ ] Settings window in `gz-editor.exe --settings`:
  - all hotkeys, using a capture box with conflict detection (report when `RegisterHotKey` fails);
  - Shift-drag behaviour (require Shift / always show zones);
  - overlay colours and opacity;
  - deadzone threshold; deadzoned zone display;
  - overlap rule; exclusion list (exe names);
  - gaming snap behaviour;
  - run at logon: HKCU Run, or an elevated Scheduled Task;
  - log level.
- [ ] Settings apply live without restarting the core.

**Done when**
- Behaviour survives a reboot.
- Hotkey conflicts are reported clearly.
- Every setting takes effect immediately.

### Phase 5 — Borderless toggle
- [ ] Hotkey (default `Win+Ctrl+Alt+B`) acts on the target window:
  - the foreground window if it occupies the gaming zone;
  - otherwise the last window snapped to the gaming zone, if it still exists.
- [ ] **Off**:
  1. Save the original style and ex-style per HWND.
  2. Remove `WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX` and `WS_EX_DLGMODALFRAME | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE | WS_EX_WINDOWEDGE`.
  3. Call `SetWindowPos(..., SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE)` so that window rect == client rect == gaming zone.
- [ ] **On**: restore the saved styles, then reposition so the client rect == gaming zone again.
- [ ] Re-apply if the game restores its border:
  - WinEvent hook scoped to that process (`idProcess`) and window; re-check styles on foreground/location changes for that HWND only.
  - Optional per-exe slow re-check fallback (off by default).
  - Remove the hooks on `EVENT_OBJECT_DESTROY` or toggle-on.
- [ ] Written from the Win32 docs. Borderless Gaming is a behavioural reference only (GPL-2.0; do not copy code).

**Done when**
- Toggling 10× on the ruler and on two real games causes no position or size drift.
- A game that re-adds its border is corrected.
- Idle CPU is 0.0% while a borderless game is running.

### Phase 6 — Backlog (not scheduled)
- Per-game profiles: auto-snap to the gaming zone and auto-borderless by exe on launch.
- Multiple gaming zones.
- On-screen drag-to-edit canvas (FancyZones-style full-screen editor).
- Zones spanning monitors.
- Code signing; release packaging.

---

## 5. References
- PowerToys FancyZones (MIT): `microsoft/PowerToys`, path `src/modules/fancyzones`. FancyZonesLib is C++; the editor is C#/WPF. Adapting MIT code is fine; record it in `THIRD_PARTY_NOTICES.md`.
- Borderless Gaming (GPL-2.0): `andrewmd5/Borderless-Gaming`. Reference for behaviour only.
- Win32 docs: `SetWinEventHook`, `RegisterHotKey`, `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`, `AdjustWindowRectExForDpi`, `GetMonitorInfoW`, `UpdateLayeredWindow`, `SetWindowLongPtrW`, `SetWindowPos`.
