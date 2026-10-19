# GamingZones

A lightweight window manager for Windows, in the style of PowerToys FancyZones, with an optional
**gaming zone**: a zone sized to an exact game resolution that the other zones arrange around, and a
hotkey that removes and restores a game window's border.

**Status: early development.** Phases 0 to 5 are built: Shift-drag snapping, the gaming zone, the layout editor,
the layout selector and quick-switch hotkeys, the settings window and the borderless toggle. See [PLAN.md](PLAN.md)
for the scope and phase order.

## What's here now

- `gamingzones.exe`: tray app. Hold Shift while dragging a window to see the zones and drop it into one.
  Settings and layouts live in `%APPDATA%\GamingZones\`. Default hotkeys:
  - `Win+Ctrl+Alt+G`: snap the foreground window to the gaming zone.
  - `Win+Ctrl+Alt+B`: remove the border of the window in the gaming zone, or put it back. If the game puts
    its border back, it's removed again.
  - ``Win+Shift+` ``: layout selector. `Win+Ctrl+Alt+1`..`9`: switch layout.
  - `Win+Ctrl+Alt+I`: write the foreground window's details to the log.
- `gz-editor.exe`: layout editor, opened from the tray menu. Split, merge and resize zones, type exact sizes,
  lock zones, and turn a zone into a gaming zone of an exact resolution. `gz-editor.exe --settings` (also
  in the tray menu) changes the hotkeys and other settings.
- `ruler.exe`: test window that shows its own client size, window rect, DWM frame bounds and DPI in physical
  pixels. It can set an exact client size and toggle a fixed (non-resizable) frame.

## Design rules

- No work while idle: no polling, no persistent low-level hooks. Hotkeys use `RegisterHotKey`.
- Only external window APIs are used: no DLL injection and no reading other processes' memory.
- Everything is measured in physical pixels. Both executables are Per-Monitor-V2 DPI aware.
- Windows only move when you snap them.

## Building

Built from WSL by cross-compiling to `x86_64-pc-windows-gnu`. You need rustup with that target, plus `mingw-w64`.

```
cargo build --release
cargo test
cargo clippy --workspace -- -D warnings
```

The executables end up in `target/x86_64-pc-windows-gnu/release/`.

## Licence

GPL-3.0. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
