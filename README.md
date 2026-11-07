# PDF Reader

A fast, lightweight, native PDF viewer written in **Rust** — no browser, no Electron, no Java. Just open a PDF and read it.

> Built for everyday reading: continuous scrolling like a browser, single-page mode like Adobe Reader, and edge-aware navigation that scrolls inside a page before jumping to the next one.

[License: GPL-3.0](https://img.shields.io/badge/License-GPLv3-blue.svg)
[Rust](https://img.shields.io/badge/Made%20with-Rust-orange.svg)
[Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)

<img width="1100" height="832" alt="image" src="https://github.com/user-attachments/assets/8a5b7c57-7fcd-4521-864f-f6d64792e5d5" />

---

## What is this?

**PDF Reader** is a single-window desktop PDF viewer focused on speed, correct rendering, and staying out of your way.

It renders PDFs natively with [Hayro](#renderer-hayro) — a pure-Rust PDF rasterizer — and draws the UI with [egui / eframe](#ui-egui--eframe). There is no web view and no external PDF engine to install.

Open a file by:

- `File > Open` / `Ctrl+O`
- Drag & drop a `.pdf` onto the window
- Double-clicking a `.pdf` in Explorer (Windows file association)
- `pdf-reader path/to/file.pdf` from the command line

Your place is remembered per file — page, zoom, rotation, continuous mode, and dark mode — so you can close and resume exactly where you left off.

---

## Features

### Reading

- **Single-page mode** — one page centered, with a thin proportional scrollbar on the right (`thumb = 1 / total_pages`)
- **Continuous mode** — all pages in one smooth scroll, like a browser, with lazy rendering and virtualized layout
- **Edge-aware navigation (Adobe-style)** — this is the key behavior:
  - `Wheel` / `Up` / `Down` scroll *inside* the current page first (e.g. in **Fit Width** fullscreen where only the top of the page is visible)
  - Only when you reach the bottom/top edge do they jump to the next/previous page
  - If the whole page fits (Fit Height / 100%), they jump directly
  - `Left` / `Right`, `PageUp` / `PageDown` always jump instantly, regardless of zoom
- **Fit Width, Fit Height, 100%**, zoom slider `10% – 500%`, `Ctrl + Wheel` to zoom
- **Rotation** — `0° / 90° / 180° / 270°`, `R` to rotate clockwise, `Shift+R` counter-clockwise
- **Dark mode** — `D` / `Ctrl+D`, inverts the rendered page for night reading
- **Hand tool** — `Alt+H`, drag to pan when zoomed
- **Outline / Bookmarks** — `F9` / `Ctrl+B` side panel, click an entry to jump
- **Fullscreen** — `F11`
- **Properties** — `Ctrl+I`, shows file name, size, page count, current page, zoom

### Session & History

- **Per-file history** saved in `config.json`: page, scale, continuous mode, rotation, dark mode
- **Recent files** menu (last 12 files), with “Clear” and “Open containing folder”
- **Session restore** — reopens the last file on launch
- **Config location**
  - Windows: `%APPDATA%\\pdf-reader\\config.json`
  - Linux: `~/.config/pdf-reader/config.json`
  - macOS: `~/Library/Application Support/pdf-reader/config.json`
  - Fallback: `pdf-reader-config.json` next to the executable

### Printing (Windows)

- Native Windows printing via **WinSpool** — vector printing, not a screenshot
- Printer picker, default printer detection
- Print range: **All / Current page / Custom** (e.g. `1-3, 5, 8-10`)
- Page extraction for custom ranges uses `lopdf` to build a minimal PDF for the spooler

> On non-Windows platforms printing is currently a stub (`Windows only`).

### Performance & Safety

- **LRU texture cache** (max 20 pages) — fast back/forward navigation
- **24 MP cap per page** — huge pages are automatically downscaled to avoid OOM
- **Quantized scale** — avoids re-rendering on tiny zoom changes
- **Password-protected PDFs** — detected and reported; Hayro does not yet support encrypted PDFs, the app shows a dialog instead of crashing

---


### Windows file association

To open PDFs by double-click:

1. Right-click a `.pdf` → **Open with** → **Choose another app**
2. Pick `pdf-reader.exe` → check **Always use this app**

The app reads the file path from `std::env::args()` on startup.

---

## Usage

### Toolbar

| Control | What it does |
| :--- | :--- |
| Open | Open a PDF file dialog |
| Print | Open printer picker (Windows) |
| Properties | Show file info |
| ↻ / ↺ | Rotate 90° CW / CCW |
| < / > | Previous / Next page |
| Page input | Type a page number + Enter to jump |
| Fit W / Fit H / 100% | Fit mode |
| Zoom slider | 10% – 500% |
| 🌙 Dark / ☀ Light | Toggle dark mode |
| ✋ Hand / 🖱 Select | Toggle hand tool |
| 📑 Outline | Toggle outline sidebar |
| Continuous | Toggle single-page vs continuous scrolling |

### Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl+O` | Open file |
| `Ctrl+P` | Print |
| `Ctrl+I` | Properties |
| `Ctrl+Wheel` | Zoom |
| `+` / `-` / `Ctrl+0` | Zoom in / out / reset |
| `W` / `H` | Fit Width / Fit Height (when not typing) |
| `Left` / `Right` | Previous / Next page (instant) |
| `Up` / `Down` | Scroll inside page, jump at edge (single-page) / Next/Prev page (continuous) |
| `PageUp` / `PageDown` | Previous / Next page (instant) |
| `Home` / `End` | First / Last page |
| `G` | Focus page input |
| `R` / `Shift+R` / `Ctrl+R` | Rotate CW / CCW |
| `D` / `Ctrl+D` | Toggle dark mode |
| `Alt+H` | Toggle hand tool |
| `F9` / `Ctrl+B` | Toggle outline |
| `F11` | Fullscreen |
| `Esc` | Close dialogs / clear error |
| `Space` | Next page (single-page) |

---

## File Structure

```
.
├── Cargo.toml              # Package metadata + dependencies
├── Cargo.lock              # Pinned dependency versions
├── LICENSE                 # GPL-3.0 license text (see License)
├── README.md               # This file
└── src/
    ├── main.rs             # ~1,500 lines: ViewerApp, UI, rendering, printing, config
    │                       # - eframe::App implementation
    │                       # - Single-page + Continuous rendering
    │                       # - Edge-aware wheel / Up/Down navigation
    │                       # - Zoom, Fit W/H, Rotation, Dark mode, Hand tool
    │                       # - Recent files + per-file history (config.json)
    │                       # - Windows printing via WinSpool (mod print_win)
    │                       # - Password detection, drag & drop, CLI args
    └── outline.rs          # Outline / bookmark extraction
                            # - Parses PDF outline tree via hayro_syntax
                            # - Returns Vec<OutlineItem> { title, page, children }
                            # - Used by the left sidebar
```

> **Note:** `print_win` is an inline `mod` inside `main.rs`, not a separate file. It is `#[cfg(windows)]` only.

### Config

No database. Everything is a single JSON file:

```json
{
  "last_file": "C:\\Users\\...\\book.pdf",
  "last_page": 42,
  "last_scale": 1.5,
  "continuous": false,
  "dark_mode": false,
  "hand_tool": false,
  "recent": ["C:\\...\\book.pdf"],
  "file_history": {
    "C:\\...\\book.pdf": {
      "page": 42,
      "scale": 1.5,
      "continuous": false,
      "rotation": 0,
      "dark_mode": false
    }
  }
}
```

---

## Renderer: Hayro

This project does **not** use Poppler, MuPDF, PDFium, or a browser engine.

### What is Hayro?

[Hayro](https://github.com/LaurenzV/hayro) by LaurenzV is a **pure-Rust PDF renderer**:

> “A crate for rendering PDF files. This crate allows you to render pages of a PDF file into bitmaps. The ultimate goal is to be a feature-complete and performant PDF rasterizer. Currently the most comprehensive and feature-complete implementation of a PDF rasterizer in pure Rust.” — docs.rs

How it is used here:

```rust
use hayro::hayro_syntax::Pdf;
use hayro::{RenderCache, RenderSettings, PixmapSettings};
use hayro::hayro_interpret::InterpreterSettings;

let pdf = Pdf::new(bytes)?;
let page = &pdf.pages()[page_idx];
let pixmap = hayro::render(
    page,
    &RenderCache::new(),
    &InterpreterSettings::default(),
    &RenderSettings::default(),
    &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
);
```

- `hayro_syntax` — low-level PDF parsing (the “Syntax” chapter of the PDF spec), page tree, dimensions
- `hayro` + `hayro_interpret` — interprets PDF content streams, fonts, color spaces
- `vello_cpu` (via Hayro) — CPU rasterizer that produces the final RGBA bitmap, which this app uploads as an `egui::TextureHandle`

Why Hayro?

- **Pure Rust, memory-safe** — no C bindings, no unsafe PDF parser
- **No system dependencies** — works on Windows without installing anything
- **Accurate** — good font, vector, and image handling for real-world PDFs
- **Fast enough** — with the LRU cache and 24 MP cap, page turns feel instant

### Hayro Limitations (inherited)

- **Password-protected / encrypted PDFs are not supported** — the app detects this and shows: “This PDF is encrypted and requires a password. Hayro does not yet support password-protected PDFs. Please decrypt it first.”
- Very complex PDFs may still be missing a rare feature — upstream Hayro is under active development.

### Hayro License

Hayro and `hayro_syntax` are licensed under **Apache-2.0**.

> “This crate is available under the Apache 2.0 license.” — docs.rs / hayro-syntax

Apache-2.0 is compatible with this project’s GPL-3.0 license (see [License](#license)). You must keep Hayro’s Apache-2.0 attribution when redistributing.

---

## UI: egui / eframe

The UI is built with [egui](https://github.com/emilk/egui) + [eframe] — an immediate-mode GUI in pure Rust.

- No HTML/CSS, no retained widgets
- Native window, drag & drop, clipboard, and file dialogs via `eframe` / `rfd`
- **License:** `egui` / `eframe` are dual-licensed **MIT OR Apache-2.0**

---

## Dependencies & Their Licenses

| Dependency | Purpose | License |
| :--- | :--- | :--- |
| **This project** | PDF viewer | **GPL-3.0** |
| `hayro`, `hayro_syntax` | PDF parsing + rendering | Apache-2.0 |
| `egui`, `eframe` | GUI | MIT OR Apache-2.0 |
| `rfd` | Native file dialog | MIT OR Apache-2.0 |
| `dirs` | Config directory | MIT OR Apache-2.0 |
| `serde`, `serde_json` | Config serialization | MIT OR Apache-2.0 |
| `bytemuck` | Pixmap byte casting | MIT OR Apache-2.0 / Zlib |
| `lopdf` | PDF page extraction for printing (Windows) | MIT OR Apache-2.0 |
| `winapi` | Windows WinSpool printing (Windows only) | MIT OR Apache-2.0 |
| `vello_cpu` (via Hayro) | CPU rasterization | Apache-2.0 / MIT |

> If you fork this project, keep this table up to date and keep third-party `LICENSE` notices intact.

---

## Building for Release

```bash
cargo build --release
```

On Windows this builds with `windows_subsystem = "windows"` (no console window in release).

To check for warnings:

```bash
cargo clippy --release
cargo fmt --check
```

---

## Roadmap / Ideas

- [ ] Text search (`Ctrl+F`) + highlight
- [ ] Text selection / copy
- [ ] Annotations / highlights
- [ ] Tabs for multiple PDFs
- [ ] Thumbnail sidebar
- [ ] Password input (when Hayro adds support)
- [ ] Linux/macOS printing
- [ ] SyncTeX / PDF bookmarks editing

Contributions are welcome — see [Contributing](#contributing).

---

## Contributing

1. Fork the repo
2. Create a branch: `git checkout -b feat/my-feature`
3. `cargo fmt` + `cargo clippy` before committing
4. Open a PR with a short description + screenshot if UI changed

Please keep PRs focused and small.

---

## License

This project is licensed under the **GNU General Public License v3.0** — see the [`LICENSE`](./LICENSE) file.

```
Copyright (C) 2026 Turan Tanriverdi

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU General Public License for more details.
```

You should have received a copy of the GPL-3.0 along with this program. If not, see <https://www.gnu.org/licenses/>.

**Third-party code keeps its own license** — Hayro (Apache-2.0), egui (MIT OR Apache-2.0), etc. See the table above.

---

## Credits

- **Renderer:** [Hayro](https://github.com/LaurenzV/hayro) by LaurenzV — Apache-2.0
- **GUI:** [egui / eframe](https://github.com/emilk/egui) by Emil Ernerfeldt — MIT OR Apache-2.0
- **PDF spec:** Adobe PDF Reference

---

## FAQ

**Q: Why not use PDFium / Poppler?**
A: To keep the binary self-contained and pure Rust. Hayro gives accurate rendering without C dependencies or a browser.

**Q: Why is Fit Width scrolling inside the page first?**
A: That's intentional — same as Adobe/Foxit. Wheel/Up/Down pan within the page until the edge, then flip.

**Q: Where is my progress saved?**
A: In `config.json` (see File Structure). Delete it to reset.

**Q: Does it work on Linux/macOS?**
A: Yes for viewing. Printing is Windows-only for now.
