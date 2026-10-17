# Third-party notices

GamingZones is licensed under the GNU General Public License v3.0 (see `LICENSE`).

## Code adapted from other projects

### Microsoft PowerToys — FancyZones (MIT)

Source: https://github.com/microsoft/PowerToys, `src/modules/fancyzones`, commit `43276127`.

| GamingZones | Adapted from |
|---|---|
| `crates/gz-core/src/layout.rs`: `zones_at` (zones under the cursor, sensitivity radius for edge spanning, smallest zone wins on overlap) | `FancyZonesLib/Layout.cpp`: `Layout::ZonesFromPoint`, `ZoneSelectionAlgorithms` (Smallest) |
| `crates/gz-core/src/layout.rs`: `zone_range`, `bounding_rect` (Ctrl-span) | `FancyZonesLib/Layout.cpp`: `GetCombinedZoneRange`, `GetCombinedZonesRect`; `HighlightedZones.cpp` |
| `crates/gamingzones/src/snap.rs`: `check` (which windows can be snapped) | `FancyZonesLib/FancyZonesWindowProcessing.cpp`, `WindowUtils.cpp` |
| `crates/gz-win/src/input.rs`: `cursor_is_resize` | `FancyZonesLib/WindowUtils.cpp`: `IsCursorTypeIndicatingSizeEvent` |

```text
The MIT License

Copyright (c) Microsoft Corporation. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

Borderless Gaming (GPL-2.0) is used as a behavioural reference only. No code is copied from it.

## Rust crates linked into the executables

| Crate | Licence |
|---|---|
| `windows`, `windows-core`, `windows-link`, `windows-result`, `windows-strings` (Microsoft) | MIT or Apache-2.0 |
| `serde`, `serde_json` | MIT or Apache-2.0 |
| `log` | MIT or Apache-2.0 |

`embed-resource` is used at build time only and is not linked into the executables.

### Layout editor (`gz-editor.exe` only)

The editor is built with [egui/eframe](https://github.com/emilk/egui) (MIT or Apache-2.0) and its dependencies
(`winit`, `glutin`, `glow`, `epaint`, `skrifa`, `image`, `arboard` and others). Every crate in its dependency tree is
under a permissive licence compatible with the GPL-3.0: MIT, Apache-2.0, Zlib, BSD-2/3-Clause, ISC, BSL-1.0, 0BSD or
Unlicense (`cargo tree -p gz-editor --format "{p} {l}"` lists them).

It embeds egui's default fonts (crate `epaint_default_fonts`), each under its own licence:

| Font | Licence |
|---|---|
| Ubuntu Light | Ubuntu Font Licence 1.0 |
| Hack | MIT, with the Bitstream Vera licence for the glyphs it derives from |
| Noto Emoji | SIL Open Font License 1.1 |
| emoji-icon-font | MIT |

It also embeds **Lexend** (`res/fonts/Lexend-VariableFont_wght.ttf`, from https://github.com/googlefonts/lexend),
the editor's main font: Copyright 2018 The Lexend Project Authors, licensed under the SIL Open Font License 1.1
(full text in `res/fonts/OFL.txt`).

None of this is linked into the always-running `gamingzones.exe`.
