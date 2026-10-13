# GamingZones

A lightweight window manager for Windows, in the style of PowerToys FancyZones, with an optional
**gaming zone**: a zone sized to an exact game resolution that the other zones arrange around.
A hotkey to remove and restore a game window's border is planned.

**Status: early development.** Phase 0 (skeleton and test tools) is done. Zones and snapping come next.
See [PLAN.md](PLAN.md) for the scope and phase order.

## What's here now

- `gamingzones.exe`: tray app. It runs a single instance, keeps its settings in `%APPDATA%\GamingZones\`,
  and has a debug hotkey (`Win+Ctrl+Alt+I`) that logs the geometry of the foreground window to
  `%APPDATA%\GamingZones\gamingzones.log`.
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
