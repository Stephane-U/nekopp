# Build Plan — Notepad++ Clone in Rust

Status: draft for review
Baseline: Notepad++ **v8.9.9.1** (released 2026-11-12)
Target: **cross-platform** — Linux (primary), Windows, macOS
Renderer: **GPU primary (`wgpu`), CPU fallback behind the same interface**
Plan date: 2026-11-18

---

## 0. Scope reality check

Notepad++ is ~20 years of C++ on top of Scintilla (~25 years). A literal 1:1 clone is not a
project, it is a decade. What *is* achievable is a **behavioural clone of the user-visible
feature set**, built on a new engine, with three explicit tiers:

| Tier | Meaning | Target |
|---|---|---|
| **P0** | Daily-driver parity. A Notepad++ user can switch without losing their workflow. | Ship gate |
| **P1** | Long-tail parity: UDL, macros, Function List, Folder-as-Workspace, print, i18n. | v1.0 |
| **P2** | Ecosystem parity: plugin ABI, Plugin Admin, Scintilla C API shim. | post-1.0 |

### The brief, and what "all" costs

Four requirements arrived in sequence:

1. Rust, desktop, cross-platform.
2. Light and fast as Notepad++.
3. No GPU.
4. **GPU, high resolution, fast refresh.**

Requirements 2 and 4 cannot both be maximised. A GPU context costs 30–80 MB of driver memory
and 100–400 ms of initialisation before the first pixel — that is the fixed price of 4K at
120 Hz, and it is exactly what Notepad++ does not pay. There is no architecture that delivers
Notepad++-class footprint *and* GPU-composited 4K at 120 Hz. Anyone who says otherwise is
selling something.

So the plan commits to **GPU as the primary renderer**, defines "light" as **Sublime-class
rather than Notepad++-class**, and keeps a **CPU backend behind the same interface** so the
light path still exists for VMs, remote sessions, containers and old hardware. That is the only
honest way to deliver "all": two backends, one scene, runtime selection.

This also stops the renderer from being a rewrite every time the requirement moves. The scene
representation, damage tracking, glyph atlas keying and layout are backend-agnostic. Only the
final composite differs.

### What "superb" means here

Time is not the constraint, so the acceptance criteria are not dates. They are seven properties,
each of which is testable:

| Property | Test |
|---|---|
| **Never loses data** | Atomic saves, crash-safe writes, backup-on-save; a fuzz suite that kills the process mid-write 10,000 times without losing a byte |
| **Never stutters** | 120 fps sustained at 4K on the GPU backend; p99 keystroke-to-pixel under 8 ms; zero dropped frames in a 10-minute scroll trace |
| **Never surprises** | Undo is exact; search is exact; encoding round-trips byte-for-byte; a file opened and saved unchanged is bit-identical |
| **Feels instant** | Window at 40 ms, text at 200 ms, 10 MB file open at 150 ms |
| **Looks right** | Typography, spacing, dark mode, HiDPI at 1.25/1.5/2.0 — reviewed by a designer, not by the implementer |
| **Accessible** | Full keyboard operation; Orca, NVDA and Narrator read the document and the menus; high-contrast themes |
| **Documented** | User manual, config reference, plugin SDK, migration guide from Notepad++ |

That is the contract. Every phase in §6 exits on one of these, not on a date.

### Cross-platform, Linux-first

The whole stack below is portable by construction — `winit`, `wgpu`, `glyphon`, `cosmic-text`,
`ropey`, `syntect` all build on all three OSes. **The only OS-specific code is the platform
layer** (§5), which is a trait with three backends. That is the structural consequence of
"runs on any OS", and it is what keeps the port cost bounded.

Recommended sequencing: **ship Linux first, then port.** The core is identical everywhere, so
the ports are additive rather than a rewrite. This is a dependency argument, not a schedule
one: the Linux backend exercises the hardest constraints (Wayland IME, fractional scaling,
portals, client-side decorations), so getting it right first makes the other two easier. See §6.

### What "clone" means on each OS

Notepad++ is Win32-only. There is no native build on Linux or macOS, so the clone is **the
feature set and the muscle memory**, not the window decorations:

- **Keep identical everywhere:** menu tree, command names, default shortcuts, dialog
  information architecture, the four per-line metadata arrays, the config file schemas.
- **Re-express per OS:** file dialogs, print, dark mode, single-instance, file watching,
  desktop integration, packaging, menu bar presentation, modifier keys.

| Concern | Linux | Windows | macOS |
|---|---|---|---|
| Display | Wayland + X11 | Win32 | Cocoa |
| GPU backend | Vulkan, GLES | DX12, Vulkan | Metal |
| Menu bar | drawn in-app | drawn in-app | **native global NSMenu** |
| File dialogs | XDG portal | `IFileDialog` | `NSOpenPanel` |
| Dark mode | portal `color-scheme` | registry + DWM | `NSAppearance` |
| Single instance | DBus name | named mutex + `WM_COPYDATA` | `NSWorkspace` / `open` |
| File watch | inotify | `ReadDirectoryChangesW` | FSEvents |
| Accessibility | AT-SPI2 | UI Automation | `NSAccessibility` |
| Package | deb/rpm/AppImage/Flatpak | MSI/MSIX/winget | .app/dmg/Homebrew |

### Non-goals (explicit)

- **Binary compatibility with existing `.dll` plugins.** Win32 PE binaries linking Scintilla's
  C API. Reimplementing ~200 `SCI_*` functions is a project of its own. Optional Phase 11.
- **Copying Notepad++ source.** GPL-3.0. Clean-room from observed behaviour and public docs.
  Scintilla itself is permissively licensed (HPND), so its *model* is fair game; its code is
  not needed.
- **The name.** "Notepad++" is a trademark. The program is `npp`, the workspace `nn`, the product Neko++.
- **Mobile, web, WASM targets.** Desktop only.

---

## 1. Reference baseline — what v8.9.9.1 actually does

Feature inventory from the v8.9.9 release notes, the user manual, and the 8.x changelog. This
is the checklist the parity matrix in §7 is scored against.

**Editing surface.** Tabbed multi-document (drag-reorder, pin, colour, multi-line, vertical
tab bar, hide); split view with synchronised scrolling; column/rectangular selection and
column editor; multi-caret; overwrite mode; read-only toggle; word wrap with wrap symbol and
indent mode; show whitespace/EOL/indent guides; zoom; distraction-free and full-screen modes;
RTL text direction.

**Text model.** Three EOL conventions (CR, LF, CRLF) with live conversion; encoding detect and
convert (ANSI codepages, UTF-8 ±BOM, UTF-16 LE/BE); large-file mode; external-modification
detection and reload; backup-on-save (simple/verbose); autosave; session persistence.

**Navigation.** Bookmarks (toggle, next/prev, cut/copy/paste/remove bookmarked lines); change
history with margin markers; go-to-line; go-to-matching-brace; go-to next/prev change;
Document Map (minimap); Document Peeker (Ctrl+Tab overlay); Document List panel; Function List
panel (regex-driven, per-language XML); Folder-as-Workspace; Project panels.

**Search.** Find / Replace / Find-in-Files / Find-in-Projects; regex (Boost.Regex, Perl
syntax); extended mode (`\n`, `\t`, `\xNN`); match case, whole word, wrap, in-selection,
backward; Mark and Count; Find All in document / all open documents; incremental search;
"Find (volatile)".

**Transform.** Line ops (duplicate, delete, move, split, join, sort with options, trim);
case conversion (UPPER, lower, Proper, Sentence, iNVERT, Random); comment/uncomment;
tab↔space; EOL conversion; encoding conversion; insert character panel.

**Assist.** Auto-completion (word, function, document-derived); smart highlighting of all
occurrences; brace matching; auto-close brackets/quotes; auto-indent and smart indent.

**Chrome.** Menu bar (fully remappable), toolbar, status bar (length, lines, sel, col,
encoding, EOL, INS/OVR, language, zoom), tab bar, plugin menu, context menus.

**Configuration.** `config.xml`, `stylers.xml` (per-language styles + themes), `langs.xml`
(language list, extensions, keywords), `shortcuts.xml` (shortcuts, macros, run commands),
`userDefineLang.xml` (UDLs), `session.xml`. Style Configurator with built-in themes and a
dark-mode toggle. Shortcut Mapper covering app commands, Scintilla commands, plugin commands,
macros and run commands. ~15 Preferences tabs. XML-based localisation (~90 languages).
Run-command (F5) with `$(FULL_CURRENT_PATH)`-style variables.

**Extensibility.** Macro record/play/save/run-N-times; plugin architecture; Plugin Admin;
UDL 2.1 with folding.

---

## 2. Stack decision

### Chosen: `winit` + `wgpu` + `glyphon` + `cosmic-text`, with a CPU fallback backend

Verified coherent as of 2026-11-18 — `glyphon 0.12` declares `wgpu ^30.0` and
`cosmic-text ^0.19`, so the three move together.

| Concern | Crate | Version |
|---|---|---|
| Windowing / input / IME | `winit` | 0.30.13 |
| GPU device + swapchain | `wgpu` | 30.0.2 |
| Glyph atlas + text quads | `glyphon` | 0.12.0 |
| Shaping, fallback, bidi | `cosmic-text` | 0.19.0 |
| CPU presentation (fallback) | `softbuffer` | 0.4.8 |
| CPU glyph rasterisation (fallback) | `swash` | 0.2.10 |
| Vector shapes (icons, rounded rects) | `tiny-skia` | 0.12.0 |
| SIMD primitives (CPU blit) | `wide` | 1.7.1 |
| Text buffer | `ropey` | 1.6.1 |
| Syntax highlighting | `syntect` | 5.3.0 |
| Incremental parsing (Function List, folding) | `tree-sitter` | 0.27.2 |
| Regex (PCRE-ish, Boost-compatible) | `pcre2` | 0.2.11 |
| Regex (fast default path) | `regex` | 1.13.2 |
| Encodings | `encoding_rs` | 0.8.44 |
| Clipboard | `arboard` | 3.7.0 |
| File watching | `notify` | 8.2.2 |
| XML (config compat) | `quick-xml` | 0.42.0 |
| Diff (change history, compare) | `similar` | 3.2.1 |
| File dialogs | `rfd` | 0.18.0 |
| Native menus (macOS) | `muda` | 0.21.2 |
| Accessibility adapter | `accesskit` + `accesskit_winit` | 0.25.2 / 0.34.2 |
| A11y backends | `accesskit_unix` / `_windows` / `_macos` | 0.24.1 / 0.35.2 / 0.27.2 |
| XDG portals (Linux) | `ashpd` | 0.14.0 |
| DBus (Linux) | `zbus` | 6.3.0 |
| Font discovery | `fontdb` | 0.24.0 |
| Print to PDF | `printpdf` | 0.12.9 |
| Packaging | `cargo-packager` | 0.11.8 |

**GPU backends:** Metal (macOS), DX12 (Windows), Vulkan + GLES (Linux), `llvmpipe` software
path for VMs and CI. All must be exercised in CI — a Vulkan-only build fails on a meaningful
slice of Linux desktops, and a DX12-only build fails on older Windows GPUs.

### Why not a framework

| Option | Verdict |
|---|---|
| **egui 0.37** | Immediate mode repaints the whole frame. At 120 Hz that is 120 full-screen composites per second — a battery disaster, and the opposite of what a text editor needs. `egui::TextEdit` also cannot do per-line styling, folding, or 100 MB files. |
| **iced 0.14** | Retained and architecturally clean, but its text stack is not a code-editor text stack. A custom `shader` widget for the canvas means running two text pipelines side by side. |
| **gpui 0.2.2** | Built for exactly this (Zed), but pre-1.0 with active API churn, and it drags in its own text system. Adopting it means inheriting Zed's roadmap. |
| **floem** | Lapce's framework, editor-shaped, but pre-1.0 with thin docs and a small contributor base. |
| **GTK4 / libadwaita** | GSK has a Vulkan renderer, so "GPU accelerated" is technically satisfied — but `GtkTextView` is not a large-file editor, you still write a custom widget, and you inherit a C dependency tree plus a GNOME-only design language. |
| **Scintilla via FFI** | Instant parity, zero GPU acceleration. Contradicts requirement 4. |

The editor surface is the product. Owning the render loop is the only way to hit the budget in
§8, and the widget kit we must build (rect, text, button, menu, scrollbar, splitter, tab bar,
dialog, tooltip) is ~3–4k LOC of well-understood work.

---

## 3. Workspace layout

```
nn/
├── Cargo.toml                  # workspace
├── crates/
│   ├── nn-core/               # buffer, edits, undo, selections, markers, folds
│   ├── npp-syntax/             # syntect bridge, UDL engine, langs.xml, fold computation
│   ├── npp-search/             # find/replace, find-in-files, mark, regex engines
│   ├── npp-render/             # scene, damage, atlas keying — backend-agnostic
│   │   ├── npp-render-gpu/     # wgpu + glyphon backend
│   │   └── npp-render-cpu/     # softbuffer + swash backend
│   ├── npp-ui/                 # widget kit, layout, theme, menus, dialogs, tab bar
│   ├── npp-config/             # XML import/export, settings, session, shortcuts
│   ├── npp-cmd/                # command bus, Scintilla-message-shaped dispatch, undo boundaries
│   ├── npp-macro/              # record/playback, shortcuts.xml macro compat
│   ├── npp-plugin/             # WASM host (wasmtime) + C ABI shim
│   ├── npp-i18n/               # Native-Langue XML loader
│   ├── npp-platform/           # TRAITS ONLY — the OS boundary
│   │   ├── npp-platform-linux/     # Wayland/X11, portals, DBus, XDG
│   │   ├── npp-platform-windows/   # Win32, registry, COM, DWM
│   │   └── npp-platform-macos/     # Cocoa, NSMenu, NSWorkspace, FSEvents
│   └── nn-app/                # binary: winit event loop, wiring
├── xtask/                      # build, package, bench, golden-file regeneration
├── packaging/                  # deb, rpm, AppImage, Flatpak, MSI, dmg, Homebrew
└── tests/                      # integration + golden files
```

`npp-platform` is the only crate with `#[cfg(target_os)]` in it. Everything above it is
portable, and that is enforced by CI building the core crates for all three targets on every
commit.

---

## 4. Core designs

### 4.1 Adopt Scintilla's *model*, not its code

This is the load-bearing decision. Notepad++'s entire feature set is expressed in terms of
four per-line metadata arrays. Mirroring them makes parity a matter of filling arrays rather
than re-deriving behaviour:

| Array | Type | Drives |
|---|---|---|
| style bytes | `Vec<u8>` per line, one per char | syntax colours, UDL, Style Configurator |
| fold levels | `u16` per line, header flag in high bits | folding, UDL folding, fold margin |
| markers | `u32` bitmask per line | bookmarks, change history, saved-state |
| annotations | `Option<Box<str>>` per line | lint/diagnostic gutters (plugin surface) |

Consequences that fall out for free:

- **UDL parity.** Notepad++ UDLs assign styles 0–7 to keyword groups, plus operators,
  delimiters, comments, numbers, strings. That maps 1:1 onto style bytes.
- **syntect integration is a mapping step.** `syntect` scope → style id, resolved through a
  theme table. No renderer changes.
- **Folding is a fold-level array**, exactly as Scintilla computes it, so UDL folding and
  language folding share one code path.
- **Change history** is a marker bitmask plus a saved-state line, which is precisely how
  Notepad++ renders green/yellow/orange.

### 4.2 Command bus shaped like Scintilla messages

Every mutation — keystroke, menu item, macro action, plugin call, shortcut — goes through one
dispatcher keyed by a `SciMessage` enum that mirrors `SCI_*` ids.

```
input ─┐
menu  ─┤
macro ─┼──▶ npp-cmd::dispatch(SciMessage, wparam, lparam) ──▶ nn-core ──▶ undo boundary
plugin─┤
shortcut─┘
```

This single choice buys four things at once:

1. **Macro import.** `shortcuts.xml` macros are literally `<Action type="0" message="2170"
   wParam="0" lParam="0"/>`. If our dispatcher speaks message ids, existing macros load.
2. **Shortcut Mapper parity.** Notepad++ remaps Scintilla commands by id.
3. **A plugin compatibility path** that is a subset of the same surface.
4. **Clean undo boundaries** — one dispatch, one transaction.

### 4.3 Buffer

- `ropey::Rope` as the primary store. O(log n) edits, ~10% memory overhead, cheap clones for
  the split view.
- Line index derived from the rope, cached, invalidated on edit.
- **Large-file path:** files >64 MB open through an mmap-backed read-only piece table
  (`memmap2`), with syntax highlighting limited to the viewport and editing disabled until the
  user opts in — matching Notepad++'s large-file behaviour rather than inventing one.
- **Undo:** transaction log of `(range, removed, inserted)` grouped by typing burst (500 ms) or
  command boundary. Change history is a separate, coarser log of touched line ranges.
- **Selections:** `Vec<Selection { anchor, head, kind: Stream | Rect | Line }>`. Rectangular
  selection is a distinct kind, not a special case in the renderer.

### 4.4 Renderer — one scene, two backends

The renderer is split so the backend is a swap, not a rewrite. This is the direct lesson of
having changed the renderer requirement three times.

```
nn-core ──▶ Scene { quads, glyph_runs, images } ──▶ Renderer::paint(scene, damage)
                                                        ├── GpuRenderer  (wgpu + glyphon)
                                                        └── CpuRenderer  (softbuffer + swash)
```

**Backend-agnostic (shared, ~60% of the work):**

- **Scene representation.** Flat `Vec<Quad>`, `Vec<GlyphRun>`, `Vec<ImageRef>` in logical
  coordinates. No backend types leak into `npp-ui`.
- **Damage tracking.** Every mutation produces dirty rects. An idle editor presents zero
  frames. At 120 Hz this matters *more*, not less: a full repaint every frame is 120
  full-screen composites per second, which the GPU can do and the battery cannot.
- **Glyph atlas keying.** `(glyph, size, scale, subpixel offset)`. Subpixel positioning is what
  makes text look right; keying on the fractional offset is what keeps it cheap.
- **Layout and shaping** via `cosmic-text`. We do not reimplement shaping — that is a decade of
  work and `cosmic-text` is good at it.

**GPU backend (primary):**

- Instanced quads for chrome, `glyphon` `TextArea` per visible line for text.
- One draw call per layer; the whole viewport is a handful of submissions.
- Glyph atlas as a texture, LRU-evicted, multi-page at 2× scale on 5K displays.
- **Present mode:** `Mailbox` by default (low latency, no tearing), `Fifo` for battery,
  `Immediate` as an opt-in. Expose it in Preferences.
- **Frame pacing:** present only on damage. A 120 Hz display does not mean 120 frames per
  second of work — it means the *ability* to present within 8.3 ms when something moves.

**CPU backend (fallback):**

- `softbuffer` surface, `swash` glyph rasterisation, SIMD alpha blend via `wide`, threaded
  bands via `rayon`.
- Scroll by `memmove` plus rasterising the exposed band.
- Selected at runtime when no suitable GPU adapter exists: VMs, containers, remote X11, broken
  drivers, `WGPU_BACKEND` overrides.
- Built in Phase 11, not Phase 2. The seam exists from day one; the implementation waits until
  there is evidence it is needed.

**Long lines** (minified JS, base64 blobs): cap shaping at viewport width + overscan, force
wrap off, show a horizontal scrollbar.

### 4.5 Syntax

- **Built-in languages:** `syntect` with Sublime grammars — ~200 languages, comfortably past
  Notepad++'s 78. Ship a curated set; lazy-load grammars on first use to keep startup fast.
- **UDL:** a real interpreter for Notepad++'s UDL 2.1 definition — 8 keyword groups, operators,
  delimiters, 3 comment styles, 3 code-folding markers, prefix/suffix modes, case sensitivity,
  "nesting", and the `userDefineLang.xml` schema. This is the single most fiddly parity item;
  budget for it explicitly (§6, Phase 6).
- **Function List:** Notepad++ drives it from per-language XML regex configs. Reuse the same
  XML files so the shipped configs work, with `tree-sitter` as an optional higher-fidelity
  backend where a grammar exists.
- **Folding:** computed from the lexer's fold markers into the fold-level array; collapsed
  state is a set of header lines, persisted in `session.xml`.

### 4.6 Search

- Default engine: `regex` (linear, fast, no backtracking).
- Boost-compatible mode: `pcre2` — lookbehind, backreferences, atomic groups. Notepad++ uses
  Boost.Regex with Perl syntax; PCRE2 is the closest semantic match available in Rust.
- Extended mode (`\n`, `\t`, `\xNN`) is a pre-pass that rewrites the pattern, as Notepad++ does.
- Find-in-Files: parallel walk (`ignore`-style filtering), `rayon`, streamed results into a
  results panel with double-click-to-jump.
- Mark: writes marker bits, no document mutation.

### 4.7 Configuration compatibility

Import on first run, export on save, round-trip where the schema allows. Stored under the
platform config dir (`$XDG_CONFIG_HOME/npp`, `%APPDATA%\npp`,
`~/Library/Application Support/npp`), with an optional "point at an existing Notepad++
profile" mode for users migrating from Windows or Wine.

| File | Contents | Approach |
|---|---|---|
| `config.xml` | ~15 preference groups | serde structs, `quick-xml` |
| `stylers.xml` | global + per-language styles, themes | style-id table |
| `langs.xml` | language list, extensions, keywords | language registry |
| `shortcuts.xml` | shortcuts, macros, run commands | command bus ids |
| `userDefineLang.xml` | UDLs | UDL interpreter |
| `session.xml` | open files, carets, bookmarks, folds | session model |

### 4.8 Plugins

- **v1.0:** WASM plugin host (`wasmtime` 50), sandboxed, cross-platform, with a Rust-native
  plugin trait. Capability-scoped: filesystem, clipboard, editor mutation, UI panels.
- **Not in the default build.** `wasmtime` is 8–15 MB and directly contradicts the lightness
  budget (§8.2). Ship it as a Cargo feature, with the default build plugin-free and a
  `nn-full` variant that includes it. Revisit once the core is under budget.

---

## 5. Platform layer

The only OS-specific code in the project. `npp-platform` defines the traits; three backends
implement them. Everything above this line is portable.

### 5.1 The trait boundary

```rust
pub trait Platform {
    fn dialogs(&self) -> &dyn Dialogs;          // open/save/folder pickers
    fn single_instance(&self) -> &dyn SingleInstance;
    fn shell(&self) -> &dyn Shell;              // reveal-in-file-manager, open-with
    fn theme(&self) -> &dyn SystemTheme;        // dark/light, accent colour
    fn paths(&self) -> &dyn Paths;              // config, data, cache, temp
    fn watcher(&self) -> &dyn FileWatch;        // external-modification detection
    fn print(&self) -> &dyn Print;              // to PDF, then to the OS
    fn modifiers(&self) -> ModifierMap;         // Ctrl vs Cmd
    fn lifecycle(&self) -> AppLifecycle;        // quit-on-last-window, etc.
}
```

Nine traits. If a feature needs a tenth, that is a design smell worth arguing about.

### 5.2 Cross-cutting concerns

These bite on all three OSes and are worth designing once:

**Modifier mapping.** Notepad++ is Ctrl-based. `shortcuts.xml` stores `Ctrl`/`Alt`/`Shift`/`Key`.
On macOS, Ctrl must map to Cmd by default or the app is unusable. Implement a `ModifierMap`
that translates at the boundary, with a per-platform default and a user override in the
Shortcut Mapper. Get this wrong and macOS users bounce in the first minute.

**Menu bar divergence.** macOS requires a real global `NSMenu` — there is no acceptable
in-window menu bar. Windows and Linux get our own GPU-drawn menu bar, which matches Notepad++
and keeps the look consistent. Same command tree, two presentations, driven from one
`MenuModel`. Use `muda 0.21` for the macOS side.

**Window and quit semantics.** macOS: closing the last window does not quit; Cmd+Q quits; the
app menu carries About/Preferences/Services/Hide. Windows and Linux: closing the last window
quits by default. This is `AppLifecycle`, and it must be a policy object, not `#[cfg]` sprinkled
through the event loop.

**Font rendering.** `cosmic-text`/`swash` rasterises identically on all three platforms. That
gives a consistent look — a genuine advantage — but it does not match native hinting. Default
to grayscale AA everywhere (macOS dropped subpixel AA in Mojave; most Linux desktops are there
too), expose hinting as a preference, and accept that text will differ slightly from native
apps. Do not chase pixel-parity with three different rasterisers.

**Accessibility.** `accesskit_winit` plus the three backends: AT-SPI2 on Linux, UI Automation
on Windows, `NSAccessibility` on macOS. Start in Phase 3, not Phase 11 — retrofitting a11y into
a custom widget kit is miserable.

**Single instance.** Three mechanisms, one trait: DBus name ownership (Linux), named mutex plus
`WM_COPYDATA` (Windows), `NSWorkspace`/`open` (macOS). Second launch forwards argv and exits.
`--new-instance` bypasses it.

**File watching.** `notify` covers inotify, `ReadDirectoryChangesW` and FSEvents. Watch the
*directory* as well as the file — editors that only watch the inode miss atomic-save
replacements, which is how most Unix tools write.

### 5.3 Linux backend

- **Wayland primary, X11 fallback.** Both shipped, both in CI (headless `weston`, `Xvfb`).
- **Client-side decorations.** Wayland gives no server-side titlebar, so we draw one: window
  controls, drag region, tab bar folded in. On X11 with a server-side-decorated WM, fall back
  to a plain titlebar.
- **Fractional scaling.** `wp_fractional_scale_v1` means 1.25 and 1.5. The glyph atlas must
  rasterise at the effective scale and rebuild on change, keyed by scale factor, or text goes
  blurry — the most common complaint about GPU-rendered Linux apps.
- **High refresh on Wayland is compositor-controlled.** We can request a frame callback and
  present as fast as the compositor allows, but we cannot force 120 Hz. On X11, presentation
  timing is ours. Document the difference rather than promising parity.
- **IME.** `text-input-v3` on Wayland, XIM on X11. Support in the Rust stack has historically
  lagged, so **this is a Phase 0 spike, not a Phase 8 assumption.** Fallback:
  `smithay-client-toolkit` 0.21 directly.
- **Clipboard history is a parity gap.** Notepad++ has a clipboard history panel. On Wayland
  that needs `wlr-data-control`, which GNOME does not implement. Full history on X11 and
  wlroots via `wl-clipboard-rs`; on GNOME/Wayland degrade to "what this app copied" and say so
  in the UI. Do not silently ship a broken panel.
- **Portals** via `ashpd`: file chooser, print, `color-scheme` for dark mode.

**Packaging.** AppImage is the primary "just works" channel, with deb/rpm for people who want
package-manager integration and Flatpak for sandbox users.

| Format | Priority | Role |
|---|---|---|
| AppImage | **P0, primary** | zero-install, any distro, no root |
| `.deb` / `.rpm` | P0 | package-manager integration, desktop files, MIME |
| Flatpak | P1 | sandboxed; needs `--filesystem=home` for Folder-as-Workspace |
| AUR | P1 | trivial once the tarball exists |

Flatpak is a design constraint, not just a build target — under a sandbox, Folder-as-Workspace
and Find-in-Files do not work over arbitrary paths without filesystem permission. Decide the
permission model early.

#### AppImage

This stack is unusually well suited to AppImage, for one specific reason: **there is no GTK or
Qt dependency to bundle.** `ashpd` talks to portals over DBus and `rfd` uses the portal backend,
so the classic AppImage failure mode — a bundled GTK fighting the host theme, file dialogs
rendering wrong, icons missing — simply does not apply. The bundle is our binary, the Vulkan
loader, fonts and grammar data.

Four things break AppImages in practice. Each has a fix, and all four are cheap if done up
front:

1. **FUSE.** The runtime needs `libfuse2`. Ubuntu 24.04+ ships `libfuse2t64` and not
   `libfuse2`, so a fresh install fails with `dlopen(): error loading libfuse.so.2`. Fix:
   document the one-line install, and document `--appimage-extract-and-run` as the no-root
   fallback. Verify whether the current runtime handles FUSE 3 — do not assume it does.
2. **No desktop integration.** An AppImage installs no `.desktop` file, no icon, no MIME
   association, so the app never appears in the launcher and "Open With" does not work. Fix:
   ship `npp --install-desktop`, which writes
   `~/.local/share/applications/npp.desktop`, installs the icon into the hicolor theme, and
   runs `update-desktop-database` and `xdg-mime default`. Do not depend on AppImageLauncher or
   `appimaged` — both are third-party and neither is reliably maintained.
3. **No updater.** Fix: reuse the cross-platform updater from Phase 11. Publish a `.zsync` next
   to the AppImage if we want AppImageUpdate-compatible delta updates; otherwise full-file
   download is fine at ~25 MB.
4. **glibc floor.** glibc is forward-compatible only. Build the AppImage in a container pinned
   to the oldest glibc we support (Debian 12 or Ubuntu 22.04), or it will not start on older
   distros. This is the single most common AppImage shipping bug.

Two more constraints worth writing down:

- **Bundle the Vulkan loader, never the driver.** ICDs must come from the system. Bundling Mesa
  is a rabbit hole and breaks NVIDIA outright.
- **Expect ~25–35 MB.** Grammars, fonts and the loader dominate.

Tooling: `cargo-packager` 0.11 drives AppImage alongside deb and rpm from one config, so the
whole Linux matrix comes from a single `xtask package` invocation. CI builds inside the pinned
container and smoke-tests the result under `--appimage-extract-and-run` on Ubuntu 22.04, Ubuntu
24.04, Fedora and Arch.

### 5.4 Windows backend

- **File associations** via registry (`HKCU\Software\Classes`), plus `SHChangeNotify` to
  refresh the shell.
- **Shell context menu** entry ("Edit with npp"), opt-in at install.
- **Dark title bar** via `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`; system theme
  from `AppsUseLightTheme`.
- **Dialogs** via `IFileDialog` (through `rfd`), which gives the modern Explorer-style picker.
- **Per-monitor DPI v2** awareness in the manifest; the atlas is already scale-keyed, so this
  is mostly plumbing.
- **High refresh** via the DXGI flip model, which `wgpu` uses by default. `Mailbox` present mode
  gives low-latency 120 Hz without tearing.
- **Packaging:** MSI (WiX) and a portable zip (P0); MSIX and winget (P1). **Code signing is a
  budget item** — an OV certificate runs ~$200–400/yr and without it every install shows a
  SmartScreen warning.

### 5.5 macOS backend

- **App bundle is mandatory.** Menus, dock icon, file associations and document types all
  require a proper `.app` with an `Info.plist`. A bare binary is not shippable.
- **Global menu bar** via `muda`: App menu (About, Preferences, Services, Hide, Quit), File,
  Edit, Search, View, Encoding, Language, Settings, Macro, Run, Plugins, Window, Help. The
  standard Edit menu items (Undo/Redo/Cut/Copy/Paste/Select All) must be wired to the real
  responder chain or macOS text services misbehave.
- **`NSAccessibility`** via `accesskit_macos`.
- **FSEvents** via `notify`.
- **High refresh** via `CAMetalLayer` with `maximumDrawableCount = 3`; ProMotion displays
  negotiate 120 Hz automatically.
- **Document model:** `NSDocument`-style behaviour is not required, but "close window ≠ quit"
  and "reopen last session on activate" are expected.
- **Packaging:** `.app` in a `.dmg`, plus a Homebrew cask. **Notarisation requires an Apple
  Developer account ($99/yr)** and `codesign` + `notarytool` in CI. Without it, Gatekeeper
  blocks the app on first launch. Budget for it.

---

## 6. Phases

Time is not the constraint. Dependency order is. Each phase exits on a **gate**, not a date,
and the sequence is ordered by what unblocks what rather than by what is most interesting to
build.

| Phase | Depends on | Gate |
|---|---|---|
| **0. Risk retirement** | — | **1080p/60, 1440p/120 and 4K/120 all measured**; 100 MB file at 120 fps; CJK input on Wayland; sharp at 1.5×; Metal + DX12 + Vulkan + GLES all render |
| **1. Editor core** | 0 | Rope edits, undo, multi-selection, rect selection, markers, folds — property-tested against a naive string model |
| **2. Renderer (GPU)** | 1 | Scene + damage + atlas; instanced chrome, glyphon text; 120 fps at 4K; idle presents zero frames |
| **3. Chrome** | 2 | Tabs, split view, menus, toolbar, status bar, Preferences, Style Configurator, Shortcut Mapper; `MenuModel` + `ModifierMap` |
| **4. Dogfood** | 3 | **Daily-driver alpha. From here the team uses it as their only editor.** |
| **5. Search + transform** | 4 | Find/Replace/Find-in-Files/Mark/Count, regex + extended, line ops, case ops, column editor |
| **6. Config + UDL** | 5 | All six XML files import; UDL 2.1 passes a golden corpus; themes round-trip |
| **7. Panels + macros** | 6 | Function List, Folder-as-Workspace, Project, Document Map, Peeker, macro record/play |
| **8. Linux platform** | 7 | Portals, DBus, CSD, `--wait` + stdin, `.desktop` + MIME, AT-SPI, AppImage + `--install-desktop` |
| **9. Windows platform** | 8 | Registry, shell menu, DWM, `IFileDialog`, UIA, MSI + zip, signed |
| **10. macOS platform** | 9 | `.app`, global NSMenu, `NSAccessibility`, FSEvents, Cmd mapping, notarised dmg |
| **11. Long tail** | 10 | Print, i18n, plugin SDK, `SCI_*` shim, docs, a11y audit, updater, **CPU backend** |

### The dogfood gate is the most important line in this document

Quality in an editor comes from **use**, not from time. A team that builds for eighteen months
and then starts using the product has eighteen months of unvalidated assumptions to unwind. A
team that starts using it at month five has a year of daily friction to fix — and that friction
is where "superb" actually comes from.

So Phase 4 is not a milestone, it is a **mode change**. From that point the team's own editor is
the product, and every annoyance is a bug with a priority. The long tail in Phase 11 is not
"more features" — it is "a year of using it and fixing what annoys you".

This is also the answer to the obvious risk of an unbounded schedule: scope is bounded by the
parity matrix (§7), and the *time* is spent on refinement, not on addition.

### What is deliberately not scheduled

- **A date.** There is no v1.0 date in this plan. There is a gate list.
- **A feature freeze.** Features land when their gate passes, not when a calendar says so.
- **A team size.** The gates are the same for one person or five; only the wall-clock differs.

---

## 7. Parity matrix

Scored P0 / P1 / P2 per §0. With time no longer the constraint, the tiers describe **order**,
not **whether**. P2 is in scope; it simply lands last.

| Area | P0 | P1 | P2 |
|---|---|---|---|
| Documents | tabs, split view, session, recent files, drag-drop, external-change detect | pin/colour/vertical tabs, multi-line tabs, Document Peeker | — |
| Editing | undo/redo, multi-caret, rect select, column editor, overwrite, wrap, zoom | change history, RTL, distraction-free | — |
| Text model | EOL convert, encoding detect/convert, large-file mode | backup-on-save, autosave | full ANSI codepage table (iconv) |
| Navigation | bookmarks, go-to-line, go-to-brace, Document Map | Function List, Folder-as-Workspace, Project panels | — |
| Search | find, replace, find-in-files, regex, extended, mark, count | find-in-projects, incremental, volatile | — |
| Transform | line ops, sort, case ops, comment toggle, tab↔space | insert-character panel | — |
| Assist | brace match, auto-close, word completion, smart highlight | function completion, document-derived completion | — |
| Chrome | menus, toolbar, status bar, Preferences, Style Configurator, Shortcut Mapper | Run command (F5), custom context menus | — |
| Config | `config.xml`, `stylers.xml`, `langs.xml`, `session.xml` | `shortcuts.xml`, `userDefineLang.xml` | — |
| Syntax | syntect built-ins, folding | UDL 2.1 | — |
| Extensibility | — | macros, WASM plugins | C ABI shim, `SCI_*` shim, Plugin Admin |
| **Linux** | Wayland + X11, fractional scaling, IME, clipboard, inotify, DBus single-instance, `--wait`/stdin, `.desktop` + MIME, **AppImage + `--install-desktop`**, deb | portals (file/print/dark mode), AT-SPI, Flatpak, AUR, print-to-PDF, i18n | Snap, tray icon |
| **Windows** | DX12, per-monitor DPI, `IFileDialog`, registry associations, DWM dark titlebar, named-mutex single-instance, MSI + zip | shell context menu, UIA, MSIX, winget, print-to-PDF | jump list, taskbar progress |
| **macOS** | Metal, `.app` bundle, global NSMenu, Cmd-key mapping, lifecycle policy, `NSOpenPanel`, dmg | `NSAccessibility`, FSEvents, Homebrew cask, print-to-PDF | Services menu, document types |

---

## 8. Lightness and speed budgets

### 8.1 What "light" means with a GPU

The GPU context is a fixed cost that cannot be optimised away: 30–80 MB of driver memory and
100–400 ms of initialisation. So the target is **Sublime-class**, not Notepad++-class, and the
CPU backend exists for users who need the lighter profile.

| Metric | Notepad++ | Sublime | **GPU target** | **CPU fallback** |
|---|---|---|---|---|
| Install size | ~15 MB | ~20 MB | **< 30 MB** | **< 15 MB** |
| RSS, 5 files, 1080p | ~25 MB | ~50 MB | **< 90 MB** | **< 40 MB** |
| RSS, 5 files, 4K | — | — | **< 140 MB** | **< 70 MB** |
| Cold start to first frame | ~200 ms | ~150 ms | **< 250 ms** | **< 100 ms** |
| Idle CPU | 0% | 0% | **0%** | **0%** |
| Scroll, 10 MB | — | — | **see §8.3 tiers** | 60 fps @1080p |
| Keystroke → pixel | < 16 ms | < 16 ms | **< 8 ms (p99)** | < 16 ms |
| Open 100 MB file | ~1 s | ~1 s | **< 1.2 s** | < 1.2 s |
| Find-in-files, 10k files | ~3 s | ~3 s | **< 3 s** | < 3 s |

Measure the Notepad++ column yourself on day one, on the target machine. Do not trust these
numbers — they are placeholders to anchor the argument, and the real ones become the CI gate.

### 8.2 What "light" costs, architecturally

Six cuts. All must be made at the start; none can be retrofitted cheaply.

| Cut | Saves | Consequence |
|---|---|---|
| **No `wasmtime` in the default build** | 8–15 MB | WASM plugins become a feature flag or a later deliverable. Biggest binary lever. |
| **No bundled fonts** | 5–10 MB | System fonts via `fontdb`. Accept that text renders with whatever the user has. |
| **Curated grammar set, compressed, lazy-loaded** | ~10 MB | `syntect`'s default set is ~10 MB of embedded YAML. Use `two-face`'s curated set, compress it, load per-language on first use. |
| **No `tokio`** | ~1.5 MB + a runtime | A small scoped thread pool for file I/O and find-in-files. We do not need an async runtime. |
| **`tree-sitter` optional** | 2–5 MB | Each grammar is a C library. Feature-gate it; the regex-driven Function List works without it. |
| **Aggressive release profile** | 30–50% | `panic = "abort"`, `lto = "fat"`, `strip = true`, `codegen-units = 1`, `opt-level = 3` for hot paths. |

### 8.3 Resolution and refresh targets

| Target | Pixels | @60 | @120 | @144 |
|---|---|---|---|---|
| 1080p (1920×1080) | 2.07 M | 124 | 249 | 298 Mpx/s |
| 1440p (2560×1440) | 3.69 M | 221 | 442 | 531 Mpx/s |
| 4K (3840×2160) | 8.29 M | 498 | 995 | 1194 Mpx/s |

The app targets whatever the display reports. The only engineering question is whether the
renderer can composite that many pixels per frame — 1.2 Gpx/s at 4K/144 — and that is a
renderer-efficiency question, not a scheduling one.

### 8.4 Refresh rate: take what the display gives you

The renderer does not compute a frame budget, does not run a quality controller, and does not
special-case VRR. Three rules:

1. **Present with vsync** (`PresentMode::Fifo`). The compositor paces the app to the display's
   refresh rate. No arithmetic, no rate to read, no budget to derive.
2. **Render only on damage.** Idle presents zero frames. This is what makes 144 Hz cost nothing
   when nothing is moving — the display's rate is irrelevant if there is nothing to show.
3. **Read the rate once, for diagnostics.** Show it in the stats overlay and use it in CI to
   know what you are testing against. Do not branch on it at runtime.

If the renderer cannot hold the display's rate, that is a performance bug to fix. An adaptive
quality controller would hide it, which is why there isn't one.

**The one preference:** "limit to 60 Hz on battery", implemented as a present-skip cap, not as a
quality tier. Off by default.

### 8.5 The one decision that makes high refresh possible

**Scrolling must be a uniform update, not a scene rebuild.**

The naive design rebuilds the scene and re-uploads vertex buffers on every scroll. At 4K/144
that is ~20,000 glyph quads re-emitted 144 times per second — the CPU never makes the frame,
regardless of how fast the GPU is.

The correct design: the scene is built once per *document state*, not per frame. Scrolling
changes a transform uniform. The GPU re-rasterises the same geometry at a different offset,
which is precisely what a GPU is for. The CPU does ~0.2 ms of work per scroll frame.

Consequences:

- **Edits damage lines; scrolls damage nothing.** A keystroke rebuilds one line's glyph run. A
  scroll updates one `mat4`.
- **The vertex buffer is persistent.** Upload on damage, never per frame.
- **Idle presents zero frames.** `ControlFlow::Wait` plus explicit `request_redraw`.

This is also why the CPU backend cannot match the top tier: without a GPU there is no cheap
"re-rasterise the same geometry at a new offset", so scroll becomes a `memmove` plus a band
rasterisation. That holds 60 fps at 1080p and 1440p, and is why the CPU path is a fallback
rather than a peer.

### 8.6 The `glyphon` risk

`glyphon`'s API assumes `prepare()` is called every frame — it rebuilds vertex buffers for all
`TextArea`s each time. That is the opposite of the persistent-geometry model in §8.5.

Two options, decided in Phase 0 by measurement:

1. **Use `glyphon` and accept per-frame prepare.** Fine at 1080p/60. Likely fails above 120 Hz.
2. **Write a custom glyph renderer.** Instanced quads plus an atlas texture is ~600 LOC and
   gives full control over when geometry is uploaded. This is the likely outcome.

The `npp-render` seam makes this a backend-internal decision, not an architectural one.

### 8.7 Budgets are CI gates from week 1

Binary size, RSS, startup time and frame time are measured on every commit and fail the build on
regression. Frame time is measured at 1080p/60, 1440p/120 and 4K/120. A regression at 1080p/60
is still a regression. A lightness requirement that is not measured is a wish.

## 9. Testing

- **Property tests** — rope edit sequences replayed against a `String` model; undo/redo
  invariants; selection normalisation.
- **Golden files** — syntax output per language, UDL interpretation, config import/export
  round-trip, macro replay. Regenerated by `xtask`.
- **Render snapshots** — headless render to PNG, perceptual diff against checked-in references.
  Run at scale factors 1.0, 1.25, 1.5, 2.0. **Baselines are per-backend**, not per-OS: Metal,
  DX12 and Vulkan differ in the last bit, so compare with a tolerance rather than byte-exact.
  The CPU backend is deterministic and can be byte-exact.
- **Frame-time traces** — a 10-minute scroll trace at 4K/120 with a dropped-frame budget of
  zero. This is the test that proves the headline claim.
- **Fuzzing** — UDL parser, regex pre-pass, encoding detection, XML importers.
- **Benchmarks** — `criterion`, tracked per-commit; §8 budgets enforced in CI.
- **Platform matrix in CI** — Linux (Wayland via headless `weston`, X11 via `Xvfb`; Vulkan,
  GLES, llvmpipe), Windows (DX12, Vulkan), macOS (Metal). Plus GNOME and KDE portal
  implementations on Linux.
- **Soak** — 8-hour session with file watching, autosave and plugin churn, on each OS.

---

## 10. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| **4K/120 is not reached on the target hardware** | High — it is the headline claim | Phase 0 measures all three resolutions before anything else is built; frame-time trace in CI from Phase 2; the CPU-side budget in §8.6 is the thing to watch, not the GPU; ship the hardware tiers in §8.3 rather than one number |
| **`glyphon` cannot hold 4K/120** | High — its `prepare()` rebuilds geometry every frame, which contradicts §8.5 | Phase 0 measures it; the fallback is a ~600 LOC custom instanced-quad glyph renderer, contained inside `npp-render-gpu` |
| **Refresh-rate detection is unreliable** | Low — it is diagnostics only, not control flow | Wayland reports the mode, not the actual cadence. Show it in the stats overlay as a hint; never branch on it |
| **Wayland IME (`text-input-v3`) is immature in the Rust stack** | High — blocks CJK users on the primary platform | Phase 0 spike with a hard go/no-go; fallback to `smithay-client-toolkit` directly; X11 path always works |
| **UDL 2.1 is under-specified** | High — top "clone" complaint if wrong | Build the interpreter in Phase 6 against a corpus of real UDLs; golden-file every one |
| **Fractional scaling blurs text** | High — most visible Linux-specific defect | Atlas keyed by scale factor, rebuilt on change; snapshot tests at 1.25/1.5 |
| **Vulkan/DX12 driver coverage** | High — app fails to start on older hardware | GLES and llvmpipe fallbacks, all backends in CI, clear error dialog naming `WGPU_BACKEND`; CPU backend as the last resort |
| **macOS notarisation + Windows code signing** | High — blocks distribution, costs money, needs accounts | Start the Apple Developer and code-signing applications in Phase 0; they take weeks to approve |
| **High refresh is compositor-controlled on Wayland** | Medium — cannot promise 120 Hz there | Document the difference; X11 and Windows are ours; do not advertise a number we cannot hold |
| **macOS menu bar divergence** | Medium — the one place "clone" and "native" genuinely conflict | One `MenuModel`, two presentations; native NSMenu on macOS, drawn menu elsewhere |
| **Modifier mapping wrong on macOS** | Medium — users bounce in the first minute | `ModifierMap` at the boundary, Cmd default, user override in Shortcut Mapper |
| **Two renderer backends double the surface** | Medium | Shared scene + damage + atlas (~60%); CPU backend deferred to Phase 11; the seam is cheap, the second implementation is not |
| **Clipboard history impossible on GNOME/Wayland** | Medium — visible parity gap | Degrade honestly per §5.3; full support on X11, wlroots, Windows, macOS |
| **Flatpak sandbox breaks Folder-as-Workspace** | Medium — discovered late if ignored | Decide the permission model in Phase 0, not Phase 11 |
| **`glyphon`/`wgpu` version coupling** | Medium — a `wgpu` bump can strand `glyphon` | Pin all three; the `npp-render` seam means a swap to `parley`+`swash` is contained |
| **Shaping cost on pathological files** | Medium | Viewport-only shaping, per-run cache, long-line cap, large-file mode |
| **GPL / trademark** | High — legal, not technical | Clean-room from docs and behaviour; no Notepad++ source in the tree; distinct name and icon |
| **Scope creep from "clone"** | High | The parity matrix is the contract. Anything not in it is a v2 conversation |

---

## 11. Assumptions and open decisions

Confirmed: **Rust, cross-platform, Linux primary, GPU, high resolution, high refresh, quality
over schedule.**

Still assumed, not confirmed:

1. **"Light" means Sublime-class, not Notepad++-class.** The GPU context makes exact parity
   impossible. If that is unacceptable, the CPU backend becomes the default and 4K/120 goes
   away — those two cannot coexist.
2. **Linux-first sequencing — for dependency reasons, not schedule reasons.** The Linux backend
   exercises the hardest constraints (Wayland IME, fractional scaling, portals, CSD), so getting
   it right first makes the other two easier rather than harder.
3. **Team size is unknown.** The gates are identical for one person or five; only the wall-clock
   differs. A solo build is viable, just long.
4. **Clean-room reimplementation is acceptable.** If the intent is to fork Notepad++
   itself, this plan is wrong and the answer is "wrap Scintilla and modernise the shell", which
   is a different project.
5. **WASM plugins are an acceptable answer** for extensibility, shipped as a feature flag so the
   default build stays inside the lightness budget.
6. **Desktop-neutral UI.** We draw our own chrome rather than adopting GNOME's, Windows' or
   macOS's design language — except the macOS menu bar, where native is non-negotiable.
7. **Signing accounts are obtainable.** Apple Developer ($99/yr) and a Windows code-signing
   certificate (~$200–400/yr). Without them, both platforms show scary warnings on install.

Decisions that would most change this plan:

- **Is 120 Hz at 4K a hard requirement, or is 60 Hz at 4K enough?** 60 Hz halves the CPU-side
  frame budget pressure and removes the hardest engineering in the renderer. If 120 Hz is
  aspirational rather than contractual, say so and the plan gets materially simpler.
- **Which packaging formats are launch targets?** Flatpak-first forces the portal and sandbox
  design from day one; deb/AppImage-first lets us defer it.
- **Is there a hard external deadline?** Nothing in this plan assumes one. If one appears, the
  parity matrix is the thing to cut, in reverse tier order — not the quality gates.

---

## 12. First two weeks

1. `cargo new` the workspace, pin the §2 versions, get a `winit` window with a `wgpu` surface
   on **Linux (Wayland + X11), Windows and macOS**.
2. Render 10k lines through `glyphon` + `cosmic-text`; measure shaping, submission and GPU time
   separately at 4K.
3. **Resolution/refresh matrix test:** full-screen text at 1080p/60, 1440p/120 and 4K/120;
   scroll by uniform, measure frame time and dropped frames over 60 seconds at each.
   **This is the gate that decides whether the headline claim is real.**
4. Load a 100 MB file into `ropey`; scroll it; measure. **This is the go/no-go gate.**
5. **IME spike:** type CJK on Wayland with `fcitx5` and `ibus`; if `winit` cannot deliver
   preedit, prototype the `smithay-client-toolkit` path. Decide by end of week 2.
6. Clipboard round-trip on all three OSes; confirm the `wlr-data-control` situation on GNOME.
7. **Start the signing paperwork** — Apple Developer enrolment and a Windows OV certificate.
   These take weeks to approve and gate the Windows and macOS phases, not the Linux one.
8. Write the `npp-platform` traits and stub all three backends, so the OS boundary is fixed
   before any UI code depends on it.
9. Write the `npp-render` scene types and the `Renderer` trait, with only the GPU backend
   implemented. The CPU backend is a stub that returns `Unsupported` until Phase 11.
10. **Pin the AppImage build container** to the oldest glibc we support, and get a hello-world
    AppImage running on Ubuntu 22.04, Ubuntu 24.04, Fedora and Arch. Cheap now, painful later.
11. Write the `SciMessage` enum skeleton and one end-to-end path: keystroke → dispatch → rope
    edit → dirty rects → scene → present.
12. Draft the parity matrix as a machine-readable checklist so progress is countable, not
    vibes.

---

## Shipped since the plan was written

### Minimap

A narrow column on the right edge showing the whole document. Each pixel row of
the column stands for a band of document rows; the longest of three sampled rows
in the band is drawn as up to six style runs, so the picture reads as code rather
than as a bar chart. Indentation is skipped — a bar at the left edge of every line
would read as a solid block.

The picture is **cached** and rebuilt only when the document revision, the band
count, or the column width changes. This is not an optimisation detail: reaching
into a rope is a tree descent per line, and doing it per frame cost 8 ms at 1080p
— twenty times the whole rest of the frame. Cached, idle and scroll are unchanged
and a rebuild costs about 2 ms, paid only on the frame after an edit.

Clicking the column scrolls to that point. `Ctrl+Alt+M` toggles it.

### Word wrap

`Document` gained `wrap_cols` and a `line_lens` cache. Rows are now
`(line, segment)` pairs rather than plain line indices, so one document line can
occupy several rows. `row_of` still gives a line's first row; `segments_of_line`
gives how many rows it takes.

`line_lens` exists because wrapping needs every line's length and `Rope::line` is
a tree descent: 20 000 of them per keystroke is 10 ms. The array is spliced and
recomputed for the edited lines only, which also makes `line_len` O(1) everywhere
else.

The shaping cache is keyed by `(line, segment)`, so a wrapped row is shaped on its
own and a scroll costs one shape per newly revealed row, as before. Wrapping
disables horizontal scrolling: every row fits the window by construction.

`Ctrl+Alt+W` toggles it. Measured cost: none — 1080p and 4K frame times are
identical wrapped and unwrapped.

### Current frame times (p99, 20 000-line document)

| | idle | scroll | type |
|---|---|---|---|
| 1080p | 0.56 | 0.69 | 2.74 |
| 1440p | 0.72 | 0.73 | 2.30 |
| 4K | 1.00 | 1.21 | 3.13 |

Budget at 120 Hz is 8.3 ms. Typing is the worst case because the minimap rebuilds
on every edit; real typing is ~10 Hz, not 120.

---

## Second pass: the things a Notepad++ user reaches for

### Bookmarks

A marker bit per line, already in the document model, now with an API and a
margin. `Ctrl+F2` toggles, `F2` and `Shift+F2` step, both wrapping — a bookmark
key that does nothing at the end of the file is a key you stop pressing.

The margin is its own column between the line numbers and the fold boxes, twelve
pixels wide, so a click can tell "bookmark this line" from "fold this line"
without a modifier. The dot is a six-pixel square: the quad renderer draws
rectangles, and at that size a circle is indistinguishable.

Bookmarks survive edits. `splice_lines` carries the marker of the edited line
onto the first line the edit produces, the same way it already carried the lexer
state and the fold depth — so pressing Enter in a bookmarked line keeps the
bookmark on the first half rather than dropping it.

### Go to line

`Ctrl+G`, reusing the path prompt with a third kind. The prompt was already a
single-line text field with a label; adding a mode was cheaper than a second
dialog and keeps one place where "type something and press Enter" lives.

### Recent files and session restore

`session.txt` under the platform's config directory — `$XDG_CONFIG_HOME` or
`~/.config` on Linux, `%APPDATA%` on Windows, `~/Library/Application Support` on
macOS. Line-oriented, keyword first, numbers before paths so a path with spaces
needs no quoting:

```
recent /home/me/notes.txt
tab 12 4 /home/me/main.rs
active 0
```

The File menu is rebuilt from the recent list, which is why menu labels became
owned strings. Recent entries show the file name alone: a dropdown is a fixed
width and a full path would either be truncated or make the menu enormous.

On exit the open tabs and their caret positions are written; on launch they are
reopened. A file that has been moved or deleted since is skipped silently —
restoring is a courtesy, not a promise.

### Light theme

`Ctrl+Alt+T`. The theme lives in the view and every other widget is handed it at
draw time, so flipping it is one assignment. Every cached shape carries its
colours baked in, so the cache is dropped rather than revalidated: a theme flip
is rare and deliberate, and one frame of reshaping is cheaper than a per-line
colour comparison.

### Column editing

`Alt+Shift+arrows` grows a rectangular block; `Alt`+drag does the same with the
mouse. A block is one selection per line, which is what `Selections` was built
for from the start.

Two things had to change to make it work rather than merely draw:

**Carets survive an edit.** `insert` used to collapse to a single caret. It now
keeps one per selection, so a keystroke lands on every line of the block.

**A block edit is one undo step.** The step is the whole span the carets cover,
with the untouched text between them carried in both the removed and the
inserted string. Undo restores every line at once instead of one line per press.
`replace_ranges` — used by replace-all — got the same treatment, which fixed a
latent bug where undoing a replace-all took one press per match.

The carets are recorded against the document as it was *before* the edit, so each
one is shifted by the net change of every edit below it. Getting that wrong is
invisible on one line and wrong on every line after the first.

### Macros

`Ctrl+Shift+R` records, `Ctrl+Shift+P` plays. The recorder stores editor calls,
not keystrokes: a recorded `Insert("a")` means "insert an a at every caret", not
"press the key labelled A", so a macro recorded on a US layout replays correctly
on an AZERTY one. Replay goes through the same `Editor` methods the keyboard
does, so a macro cannot do anything a person could not.

Every keystroke that changes the document goes through one `edit(step, f)` helper
rather than `after_edit` directly, which is what keeps the recorder complete
without a second dispatch table to keep in sync.

### Packaging

`packaging/build-appimage.sh` builds a 4.8 MB AppImage: squashfs with `AppRun` at
the root, the binary under `usr/bin`, a desktop file and an icon beside it. The
runtime mounts the image and runs `AppRun`, so nothing is installed and nothing
is unpacked — the file is the program.

The binary is statically linked against everything it can be. What it cannot
avoid is the graphics stack: Vulkan, X11, and the C library are the host's
business, and every desktop that can run a game can run this.

### Cross-platform

`cargo check` passes for `x86_64-pc-windows-gnu` and `x86_64-apple-darwin`. The
only platform-specific code is the config directory lookup; `winit`, `wgpu`,
`glyphon`, and `arboard` are already portable.

### Frame times after the second pass (p99, 20 000-line document)

| | idle | scroll | type |
|---|---|---|---|
| 1080p | 0.67 | 0.75 | 2.21 |
| 1440p | 0.75 | 0.81 | 2.36 |
| 4K | 1.02 | 1.10 | 2.69 |

Unchanged. Bookmarks, the extra margin, the light theme, and the recorder all
cost nothing measurable, because none of them touch the per-frame path: the
margin is one more rectangle, the theme is a palette swap, and the recorder is
idle unless it is recording.

## Third pass: line operations

Notepad++'s Edit menu has a dozen commands that rewrite the selected lines in
place. They are the cheapest features in the program — no new state, no new
widgets, no per-frame cost — and they are the ones people reach for after a
paste goes wrong.

### The transform is a pure function

`nn-core/src/lines.rs` holds every operation as `&str -> String`. Nothing there
knows what a `Document` is, so the whole module is tested with string literals
and the editor only has to work out which characters the block covers:

```rust
pub fn transform_lines(&mut self, f: impl Fn(&str) -> String) -> bool {
    let (first, last) = self.selected_lines();
    self.transform_span(first, last, f)
}
```

`transform_span` slices the block, calls `f`, and applies the result through
`replace_ranges`, which already records one compound undo step. So sorting 5 000
lines is one `Ctrl+Z`, not 5 000.

### Two facts a `split('\n')` loses

A block of lines is not a `Vec<&str>`. It also carries whether it ended with a
terminator and whether that terminator was CRLF. `Block::parse` keeps both and
`Block::render` puts them back, which is why:

- sorting a CRLF file leaves it CRLF,
- the last line of a document does not silently gain a newline,
- `duplicate` on a one-line file produces two lines rather than one and a half.

Getting this wrong is invisible until someone opens a Windows file, so it is
tested directly: `sort_keeps_crlf`, `duplicate_keeps_the_trailing_newline`,
`move_lines_keeps_a_missing_final_newline`.

### A selection that stops at column zero

Dragging from line 3 to the start of line 8 selects five lines, not six. The
naive `char_to_line(sel.end())` says six, because the end offset is the first
character of line 8. `selected_lines` corrects for it:

```rust
if last > first && self.doc.line_to_char(last) == sel.end() {
    last -= 1;
}
```

Without that, sorting a five-line selection would drag a sixth line into the
sort and the user would never trust the command again.

### Move line is a rotate, not a swap

Moving a block down one line is not a swap with the line below: the block may be
several lines tall. `move_lines` widens the span by the neighbouring line and
rotates within it, so a ten-line block moves as a unit and the undo step covers
the whole move. At the top or bottom of the document there is nothing to rotate
against and the call returns false without touching the undo stack.

### Numeric sort reads the first number

`SortKey::Numeric` finds the first number in each line rather than requiring the
line to *be* a number, so `item 10` sorts after `item 2`. Lines with no number
sort as zero. The scan walks `char_indices`, not bytes: a line starting with a
multi-byte character would otherwise produce a slice boundary in the middle of a
code point.

### Commands are not recorded into macros

Line operations go through `after_edit`, not `edit`. A macro that replays a sort
would sort an already-sorted block on the second run, and a macro that replays
"trim trailing space" would be a no-op. Recording them would make macros
non-idempotent for no benefit.

### What is bound

| Command | Key |
|---|---|
| Duplicate line | `Ctrl+D` |
| Move line up / down | `Ctrl+Shift+Up` / `Ctrl+Shift+Down` |
| UPPER CASE | `Ctrl+Shift+U` |
| lower case | `Ctrl+U` |
| Toggle comment | `Ctrl+Q` |
| Everything else | Lines menu |

`Ctrl+Q` is Notepad++'s comment toggle, so Exit moved to `Ctrl+Shift+Q`.

### Cost

Nothing. The operations run once, on a keystroke, over the selected block. The
frame after a sort is an ordinary frame: the highlighter re-lexes from the first
affected line and stops as soon as the lexer state converges, which for a sorted
block is usually the first line.

## Fourth pass: find in files

`Ctrl+Shift+F` searches a directory tree and lists every matching line in a
panel docked above the status bar. Clicking a row opens that file at that line.

### One field, one optional suffix

A dialog with a pattern box, a filter box, a directory box, and three buttons is
four widgets and a focus model. The prompt bar already exists, so the whole
feature is one field with a suffix:

```
needle
needle in src/
```

`split_query` takes the directory half only when it names a real directory, so a
pattern that happens to contain " in " still searches for itself. Without a
suffix the search runs over the folder holding the active file, which is what
"find in files" means when you are looking at one of them.

### Three limits, because a directory walk can hang

`FindInFiles` caps depth at 24, skips `.git`, `target`, `node_modules` and the
rest of the never-interesting list, and stops the walk the moment the hit list
reaches `max_hits`. Files over 8 MB are skipped, as are files that are not
UTF-8 or that contain a NUL byte — a binary file is not text this editor can
show, so it is not text it should report matches in.

The walk is synchronous. A background thread would keep the UI responsive on a
huge tree, but it would also need a channel, a cancellation token, and a way to
merge partial results into the panel. With the hit cap the common case finishes
in a few milliseconds, and a search that finishes before the next frame does not
need to be cancellable.

### One regex pass per file, not per line

`Search::find_in_text` returns byte ranges, and the file's line-start table is
built once. Each match is then a binary search for its line and a slice for its
column. Running the matcher per line would be simpler and would re-enter the
regex engine once per line of every file in the tree.

`find_all` is now `find_in_text` plus the byte-to-character translation, so the
in-document search and the directory search cannot drift apart.

### The panel is a fixed eight rows

A drag handle is a whole interaction to build and test. Eight rows is enough to
see whether the search found the right thing, the mouse wheel scrolls it, and
`Esc` closes it. Shaping is the expensive part, so the row buffers are rebuilt
only when the hit count, the scroll offset, or the panel width changes; a frame
where nothing moved costs one rectangle and no text work.

### The bug this found

Adding the panel exposed a layout error that had been there since the find bar
was written: the find bar was positioned at `height - status_height - bar_height`,
which is only correct while nothing else is docked above the status bar. With
the results panel open, the find bar drew on top of the last result row. Both
now subtract `results_height`, and the order from the bottom edge is status bar,
results panel, find bar, editor.

### Frame times with the panel open (p99, 20 000-line document)

| | idle | scroll | type |
|---|---|---|---|
| 1080p | 0.54 | 0.65 | 2.60 |
| 1440p | 0.76 | 0.69 | 2.94 |
| 4K | 1.08 | 0.85 | 2.85 |

Within noise of the closed-panel numbers. The panel is eight shaped rows that
are shaped once, and the editor's viewport shrinks by the panel's height, so it
shapes fewer lines than it did before.

---

## Fifth pass: editing feel

Three things that separate a text box from an editor: the caret lands where the
language wants it, the bracket you are looking at is the one that is highlighted,
and you can see the shape of the file without scrolling through it.

### Auto-indent

`Editor::auto_indent` returns the current line's leading whitespace, plus four
spaces when the text before the caret ends with `{`, `(`, or `[` after trimming.
`Enter` inserts `"\n" + indent` instead of `"\n"`.

Four spaces is hard-coded. Notepad++ reads the indent width from the language's
settings; there is no per-language settings table here yet, and a preference that
only one language honours is worse than a constant that all of them do. When the
preferences dialog lands, this becomes a field.

The rule is deliberately shallow: it does not look at the next line, does not
know whether the brace opens a block or a struct literal, and does not re-indent
what is already there. That is the same trade Notepad++ makes, and it is the one
that does not surprise you.

### Brace matching

`Editor::matching_brace` returns the `(open, close)` character offsets of the
pair the caret is next to, or `None`. `scan_brace` walks forward from an opener or
backward from a closer, counting depth, bounded at 200 000 characters.

The bound matters: an unbalanced `{` at the top of a large file would otherwise
scan to the end of the document on every keystroke. 200 000 characters is far
past any block a person reads on one screen and cheap enough to run
unconditionally — `refresh_brace` is called after every edit and every mouse
caret move, not only when the character under the caret happens to be a bracket.

No string or comment awareness. `"{"` inside a string literal counts. Making it
aware means threading the lexer's state through the scan, and the lexer is
line-oriented while the scan is not. Notepad++ has the same blind spot.

`EditorView::push_braces` draws both ends as `match_current` rectangles, queued
before `push_matches` so a search highlight on the same character wins.

### The function list

`nn_core::symbols::scan` is a line-at-a-time heuristic, not a parser. It matches
the shapes that declare a named thing in Rust, C, C++, Python, JavaScript,
TypeScript, Go, Java, and C#, and returns `Symbol { name, kind, line, depth }`.

Two rules do most of the work:

- **Qualifiers are stripped repeatedly.** `pub(crate) async unsafe fn` reduces to
  `fn`. The list is a fixed array of prefixes, applied in a loop until nothing
  matches, so the order in the array does not matter.
- **A declaration opens a block; a call ends in a semicolon.** For the C-like
  fallback, a line with a `(` is a declaration when a `{` appears before any `;`,
  or when the line ends at the parameter list with the brace on the next line.
  The Allman form additionally requires a space in front of the name, which is
  what keeps `print(x)` and `self.assertEqual(a, b)` out of the list.

`const` and `static` are both a declaration keyword and a qualifier, so they are
checked before the qualifier strip and only count as a constant when what follows
has a `:` or `=` and no `(`. `const fn cheap()` and `static void helper(void)`
fall through to the function rules.

The panel is right-docked, because the results panel already owns the bottom edge
and a symbol list wants height. `EditorView::set_right_dock` shrinks the text
column and slides the minimap left, so nothing is drawn underneath.

Indentation is shown one step deep and no further. The scan knows indentation,
not scope: a method inside an `impl` and a closure inside that method both read as
"nested", which is true, and a deeper tree would be claiming more than the scan
knows.

### One text area, not a hundred

The first version gave each visible row its own `Buffer` and its own `TextArea`,
copying the results panel. At 4K that is about 107 areas, and glyphon lays out
every area it is handed: type p99 went from 2.69 ms to 4.66 ms.

Joining the visible rows into one newline-separated buffer makes it one area and
one layout. Same pixels, 4.01 ms. The per-row `Buffer` cache bought nothing here
because the panel reshapes only when the symbol count, the scroll offset, or the
width changes — the cache was never hit in a steady-state frame anyway.

The results panel keeps its per-row buffers: eight rows is not worth the change.

### Frame times with the function list open (p99, 20 000-line document)

| | idle | scroll | type |
|---|---|---|---|
| 1080p | 0.58 | 0.72 | 3.04 |
| 1440p | 0.77 | 0.83 | 3.53 |
| 4K | 1.09 | 1.18 | 4.01 |

The panel is a second column of text, so it costs roughly what a second column of
text costs. At 4K that is 4.01 ms against an 8.3 ms budget at 120 Hz.

### Rescanning

`refresh_symbols` is keyed on `(active tab, document revision)`. The tab is part
of the key because two documents can sit at the same revision number, and a
revision-only key would show the previous file's symbols after a tab switch. The
scan is one pass over the text, but on a 20 000-line file that is still work
worth skipping on a frame that only moved the caret.

## Sixth pass: smart highlighting, auto-close, word completion

Three features that all hang off the same question — what word is the caret in —
and one shared answer, `Editor::highlight_word`.

### Smart highlighting

`highlight_word` returns the selection if there is one, otherwise the word ending
at the caret, otherwise the word starting at it. Both directions matter: with the
caret at the end of `total`, the word is `total`, not the space after it.

`occurrences(word, limit)` then finds every whole-word match. Whole-word means
the character before and after each hit is not a word character, which is what
keeps `cat` out of `concat` and `scatter`. It returns **char** offsets, not byte
offsets, because everything downstream — selections, the view, the caret — works
in chars; the conversion happens once per hit via `Document::byte_to_char`.

The cost is the reason for the cache. `occurrences` calls `doc.text()`, which
copies the whole document, so on a 20 000-line file every keystroke would pay a
half-megabyte memcpy to highlight a word. `refresh_smart` is keyed on
`(active tab, revision, word)`: a caret move inside the same word — most
arrow-key navigation — hits the cache and does nothing. The tab is in the key for
the same reason as the function list: two documents can share a revision.

Drawing order in `EditorView::push_*` is `smart`, then `matches`, then `brace`.
Find hits win over smart highlights, and both win over the brace box, because a
find hit is something the user asked for and a smart highlight is ambient.

### Auto-close

`Editor::typed(c) -> Typed` decides what a single typed character should do,
returning `Plain`, `Pair(close)`, or `Skip`. The ordering of the checks is the
whole design:

1. **A closer whose partner is already next → `Skip`.** Typing `)` when the next
   char is `)` steps over it. This is what makes typing `()` by hand work: the
   `(` inserts `()`, then the `)` steps over the one that is already there.
2. **Not an opener → `Plain`.** Letters, digits, spaces, and the closers that
   did not match rule 1.
3. **A selection → `Pair`.** Wrapping a selection is always wanted.
4. **The closer is already next → `Skip`.** Same as rule 1 but for openers, so
   typing `(` in front of an existing `()` does not produce `(())`.
5. **A word character follows → `Plain`.** `(` before `abc` is a call, not a
   group.
6. **A quote with a word character on either side → `Plain`.** This is the
   apostrophe rule: `don't` must not become `don''t`.
7. **Otherwise → `Pair`.**

Macro fidelity: `Pair` records `Insert(pair)` followed by `Move(CharLeft)`, not a
single insert. A replayed macro then leaves the caret between the pair, exactly
where typing left it. `Skip` records `Move(CharRight)` for the same reason — it
is a caret move, and a macro that dropped it would replay into the wrong place.

### Word completion

`nn-core/src/complete.rs` is pure text work: `collect` gathers distinct words of
at least `MIN_LEN` characters, `matching` filters them by prefix, and
`prefix_range` finds the word being typed at the caret. `matching` returns
case-sensitive hits before case-insensitive ones and drops the prefix itself, so
completing `sync` does not offer `sync`.

`nn-app/src/complete.rs` is the popup. Like the function list it is **one text
area, not one per row** — the same lesson from the fifth pass. Ten rows of
`Buffer` would be ten layouts per frame; one newline-joined buffer is one.

Two placement rules:

- The anchor is the caret point **plus one line height**, so the box opens below
  the line being typed rather than covering it.
- `set_viewport` records the window height, and `top()` flips the box above the
  caret when it would not fit below. `push_quads`, `text_areas`, and `item_at`
  all go through `top()`, so the drawn box and the clickable box cannot drift
  apart — a bug that is invisible until someone clicks the wrong row.

The popup draws into the renderer's `overlay` batch and its text areas are
appended to `menu_areas`, so the second text pass runs when either the menu or
the popup is open. One flag, two users.

### Verification

Both features were checked by dumping the offscreen `bench` frame to PPM and
reading the PNG, since the physical monitor is disconnected:

- `bench ... smart` puts the caret on `fn sync_rows` and highlights all three
  occurrences in `document.rs`.
- `bench ... complete` opens the popup below the caret with the third row
  selected.

The `bench` argument list is positional and easy to miscount — a flag that lands
one slot early silently becomes the previous flag. Both dumps were re-run after
the first attempt produced two byte-identical images, which is the tell.

### Frame times

Smart highlighting adds no measurable cost to the frame: it is a handful of
rectangles in the existing quad batch, and the search behind it is cached.
Completion adds a second text pass, but only while the popup is open.

| | idle | scroll | type |
|---|---|---|---|
| 1080p | 0.32 | 0.32 | 2.26 |
| 1600×1000 | 0.34 | 0.34 | 2.24 |

Measured on `document.rs` (1 259 lines), not the 20 000-line synthetic document,
so the type figure is lower than the fifth-pass table.

## Backlog

Work identified but not started, in rough priority order.

1. **NL command layer and voice.** Planned in `NL-COMMAND.md`. Deferred at the
   user's request until the editor itself is finished: a tiny local CPU model
   interprets typed or spoken commands, Pocket TTS speaks the replies.

## Seventh pass: preferences

The settings that used to need a keybinding each — theme, font size, tab width,
wrap, minimap, function list — now live in one file and one dialog.

### The file

`settings.txt` sits beside `session.txt` in the platform's config directory, and
uses the same line-oriented format: a keyword, a space, the value. The backlog
said TOML; the format is not TOML, and that is deliberate. TOML would mean
pulling in `toml` and `serde`, which is roughly half a megabyte on a 4.9 MB
AppImage — a tenth of the binary to parse six lines. The session file already
proved the dumb format works, and `paths.rs` now owns the directory logic both
files share.

Unknown keywords are skipped, so a file written by a newer version still loads
what this one understands. Out-of-range numbers are clamped rather than
rejected: a font size of 400 is a typo, not a reason to refuse the file.

### The dialog

`Preferences` is a centred modal drawn in the overlay pass, the same one the
menu dropdown and the completion popup use. It edits a **copy** of the settings
and hands it back after every change; the caller decides when to apply and when
to write. The dialog never touches the disk.

That split is what makes the live preview possible: `apply_settings` runs on
every arrow key, so the font size changes as you press it. A dialog that only
applied on close would make you guess at a number you cannot see.

Two buffers, not seven: a title and one body holding the rows plus the hint.
The font is monospace, so padding the labels to column 18 lines the values up
without a second layout — the same lesson as the function list, applied before
it could bite.

### Applying a setting

Each row maps to something the view already knew how to do, with two additions:

- `set_font_size` is **absolute**, unlike `zoom`. The dialog shows a number, and
  a number has to mean the same thing every time it is applied; `zoom` is
  relative because a keystroke is.
- `set_tab_width` reaches `Buffer::set_tab_width` through `LineCache`. Every
  cached shape has the old width baked into its glyph positions, so the cache is
  dropped rather than revalidated.

`load_settings` applies without saving. Startup is not a change: a first run
should not create a settings file just because the defaults were applied.

### The bench flags

The offscreen benchmark took its flags positionally, one slot each. That was a
trap: a missing empty string shifted every later flag into the previous slot,
and the run still looked like it worked — it printed frame times and wrote a
PPM, just not the one asked for. It cost three debugging rounds, twice producing
two byte-identical images that should have differed.

Flags are now order-independent (`bench file w h frames dump.ppm light prefs`),
with `find=PATTERN` and `tabs=N` for the two that take values. An unknown flag
prints a warning instead of being silently absorbed.

### Verification

Both palettes were dumped and read: the dialog centres, the six rows align, the
hint line fits inside the frame, and the selected row is highlighted in each
theme. The width is set by the hint line, which is the longest thing in the box —
at 380 px it was clipped, at 440 px it fits.

402 tests pass, clippy is clean, both cross-targets check, and the AppImage is
4.9 MB and starts.

## Eighth pass: remappable shortcuts

Every command the menu can invoke now has its chord in one table, and the menu
reads its shortcut column from that table. Before this, `handle_key` was a
hundred lines of `match` arms and the menu hard-coded the same strings a second
time — two places to change, and nothing to catch them drifting apart.

### The shape

`keymap.rs` holds three things:

- `KeyName` — a key without modifiers: a lowercased character, `F1`–`F24`, or one
  of the named keys. `parse` treats a single character as itself and anything
  longer as a name, so `Ctrl+,` and `Ctrl+Space` share one syntax with no quoting
  rule.
- `Chord` — a `KeyName` plus ctrl/shift/alt. `parse` strips modifiers from the
  front, which is what makes `Ctrl++` work: the trailing `+` is what is left once
  `Ctrl+` is gone.
- `Keymap` — a `Vec<(Chord, Action)>` scanned linearly. Forty-odd comparisons per
  keypress is nothing next to shaping a line, and a `HashMap` would need `Action`
  to be hashable for no measurable gain.

`Action` gained `name()` and `from_name()`. They are written out by hand rather
than derived from `Debug`, because a user's `key` lines have to keep meaning the
same thing across versions and a rename of an enum variant must not silently
rebind a key.

### What is not in the table

Motions, typing, and the modal widgets. `Ctrl+Left` is not a command with a name,
and a user who rebinds `Ctrl+D` should not lose word motion. The lookup sits
after the preferences dialog, the prompt, the completion popup, and the find bar
have had their chance at the key, and before the motion block.

### One chord per command

`Keymap::set` clears the command's old chord before binding the new one, and
`bind` takes a chord away from whatever held it. The alternative — letting a
command accumulate chords — round-trips through the file only if the file lists
every chord, and then a hand-edit adds an alias instead of moving the command.
`key save F5` leaving `Ctrl+S` also saving is the kind of thing that makes a
settings file untrustworthy.

The cost is real: `Ctrl+T` no longer opens a tab and `Ctrl+=` no longer zooms,
because `Ctrl+N` and `Ctrl++` are the canonical chords and a second one would not
survive a save. `Ctrl++` still fires on a US layout, where it arrives as
`Ctrl+Shift++` — see below.

### Shift on punctuation

`Ctrl++` arrives as `Ctrl+Shift++` on any layout where `+` needs shift. A binding
that only fired on some layouts would be a trap, so `Chord::from_char` drops
shift for punctuation and keeps it for letters: `Ctrl+U` and `Ctrl+Shift+U` are
different commands, `Ctrl+,` and `Ctrl+Shift+,` are the same physical chord.

### The file

`settings.txt` gained `key <action> <chord>` lines, one per binding, written by
`Settings::render` and read by `Settings::parse`. A file written by a newer
version still loads: an unknown action name or an unparsable chord is skipped,
not rejected.

### Also

The find bar gained `Ctrl+R` for "replace this one and move on". `replace_current`
existed but nothing called it: `Ctrl+Enter` replaces all, and there was no way to
replace a single match from the keyboard. The arm takes only that chord, so every
other control key still falls through to the keymap and `Ctrl+F` reopens the bar
it is already in.

### Verification

The menu was dumped with the File and View menus open and read: every shortcut
in the column comes from the keymap, including `Ctrl++` and `Ctrl+Alt+Shift+F`.
A settings file with `key save F5` was then fed to the bench through a new
`settings=PATH` flag, and the File menu showed `F5` beside Save — the whole path
from disk to glyph, without a pointer.

433 tests pass (132 in `nn-app`, 301 in `nn-core`), clippy is clean, both
cross-targets check, and the AppImage is 4.9 MB and starts.

## Ninth pass: driving the real app

The app was run under X11 and driven with synthetic input, which is the first
time the prompt bar was exercised by hand rather than by unit test. Two things
came out of it.

### The prompt label was never shaped

`PromptBar::prepare` guarded its `set_text` with

```rust
if self.label_text() != label { ... }
```

where `label_text()` returned `self.kind.label()` and `label` was
`self.kind.label()`. The condition compared a value with itself, so it was
always false and the label buffer stayed empty. The bar drew its background,
its border and its caret, and the field sat at the right x — but `Open:` and
`Go to line:` were blank. Nothing failed; the bar just looked like it had no
label.

The guard now compares against a `label_kind: Option<PromptKind>` field that
records which label is actually shaped into the buffer. `None` on construction,
so the first open always shapes, and a change of kind reshapes. Two tests cover
it: one that `prepare` shapes the label for the current kind, one that it
reshapes when the kind changes.

### The rest of the prompt path holds

Driven by hand: `Ctrl+O` opens it with `Open:`, `Ctrl+G` with `Go to line:`,
typing lands in the field and not in the document, `Escape` closes it, and
`Enter` submits — `Go to line: 100` moved the caret to line 100, and
`Open: /tmp/a.txt` loaded the file into a second tab with the right content and
status bar. `handle_prompt_key`, reconstructed from the `PromptBar` API after a
bad scripted edit, maps onto that API correctly.

### Note for future runs

`import -window` steals the X input focus, so a capture between two key batches
silently swallows the second batch. Synthetic keys also drop under about 100 ms
of spacing. Set the focus explicitly with `XSetInputFocus` before each batch and
hold each key for 200 ms; then the injection is reliable.

435 tests pass (134 in `nn-app`, 301 in `nn-core`), clippy is clean.

## Tenth pass: the shortcut editor

The eighth pass made the keymap data. This pass gives it a surface: a seventh
row in the preferences dialog that opens a list of every bindable command, with
the chord each one holds, and captures the next chord pressed.

### Capture is a mode, not a field

The obvious design is a text field per row that parses what is typed into it.
It does not work. A field inside a modal cannot receive `Ctrl+N`, because the
modal is what decides that `Ctrl+N` means *new document* — the chord is
consumed before any field sees it. Every editor that gets this right treats
capture as a state: the dialog stops interpreting keys and the next chord that
arrives, whatever it is, is bound.

So `Shortcuts` holds a `capturing: bool`. `Enter` or `Right` on a row sets it,
the hint line changes to `Press a key to bind it   Esc cancel`, the row grows a
caret-coloured left edge, and `handle_shortcuts_key` returns before the keymap
lookup runs. `Escape` cancels without binding. The first chord that reaches
`chord_of` is bound and capture ends.

The dialog edits a copy. `Preferences::set_keymap` hands the edited map back
when a chord is bound or unbound, and `apply_settings` writes the file — so a
rebind is live and persisted in the same keystroke, and closing the dialog
without touching anything writes nothing.

### A control character is not a key

`Ctrl+Shift+J` bound as `Ctrl+Shift+I` in the first live run. That turned out to
be the input script, not the app: the first chord after a focus change loses its
modifier, so the app receives a bare character. It reproduced twice more —
`Ctrl+,` and `Ctrl+O` both arrived as plain text in the document — and stopped
once the script primes the modifier state with a bare `Shift` tap after
focusing. The `I` itself was never reproduced, and is put down to the same
fault; the chord binds correctly now.

The investigation did turn up a real cross-platform bug next to it. `chord_of`
reads `event.logical_key`, and on Windows winit fills that from `ToUnicode`,
which returns the *typed text*: `Ctrl+J` arrives as `U+000A`, not `j`. Binding
that stores a chord no keypress can ever produce again — the row would show a
control glyph and the command would be unreachable. `Chord::from_char` now
folds control characters back to the key that produced them through
`uncontrol`: `0x01..=0x1a` are `a..=z`, and `0x1b..=0x1f` are `[ \ ] ^ _`, the
punctuation keys that share the row. `0x7f` is `?`. X11 does the same thing
whenever the layout maps the control level, so the fix is not Windows-only.

### Two renderer bugs the dialog exposed

Both were latent; the dialog is the first thing that draws a second text pass
over a first one that is still in flight.

**The first pass was never submitted.** `render` prepared the main text
renderer, encoded its draws, then prepared the overlay renderer into the *same*
encoder and submitted once. `glyphon` recycles its vertex buffer on the second
`prepare`, so the first pass's draws referenced a buffer that had already been
destroyed by the time the queue ran. The moment the dialog opened, wgpu
reported `Buffer with 'glyphon vertices' label has been destroyed`. The first
pass is now submitted before the second `prepare`, and the second pass gets its
own encoder.

**The two dialogs painted on top of each other.** The shortcut editor is drawn
after the preferences dialog, so with both open the prefs rows showed through
the list. Prefs quads and prefs text areas are now gated on
`!shortcuts.is_open()`.

### The title was shaped from nothing

`Shortcuts::prepare` guarded its title with `self.title.content()`, which is not
a method on `cosmic_text::Buffer` — the title is a `&'static str` shaped once at
construction. It now uses a `title_shaped: bool`, the same shape as the
`label_kind` fix in the prompt bar: record what is in the buffer, not what you
hope is in it.

### Verification

Driven by hand under X11: `Ctrl+,` opens the preferences with the seventh row
`Shortcuts  Edit...`; `Down`×6 then `Enter` opens the list with the header
`Command`/`Shortcut` and sixteen rows; `Enter` on `Open` switches the hint to
`Press a key to bind it   Esc cancel` and marks the row; `Ctrl+Shift+J` binds
`Ctrl+Shift+J`, writes `key open Ctrl+Shift+J` to `settings.txt`, and the chord
then opens the `Open:` prompt from the editor. `Escape` cancels capture and
`Escape` again closes the dialog.

462 tests pass (161 in `nn-app`, 301 in `nn-core`), clippy is clean, and both
cross-target checks (`x86_64-pc-windows-gnu`, `x86_64-apple-darwin`) finish.
The AppImage rebuilds at 4.9 MB.

## Eleventh pass: importing a Notepad++ configuration

A user switching from Notepad++ should not have to retype their theme, their
font size, their tab width and forty shortcuts. `Settings → Import Notepad++
Config...` asks for a path, reads `config.xml` and `shortcuts.xml` out of it,
applies what maps cleanly, and says what it did in the status bar.

### A scanner, not a parser

The crate has no XML dependency and is not getting one for two files. What the
import needs is a handful of elements with quoted attributes, so `nppimport.rs`
carries a scanner: `elements(text, name)` walks the text for `<name ...>` and
returns each one's attributes as borrowed `&str` pairs. It does not build a
tree, does not validate, and cannot fail — a file it cannot make sense of
imports nothing, and a missing file is not an error. That is the right failure
mode for an import: the worst case is that nothing changes.

`Import::read` takes either the directory or either file, so a user who pastes
the path they had open in a file manager still lands on the right thing.

### Partial by design

Notepad++ has commands this editor does not, and settings with no counterpart
here. The `menuCmdID` table maps twenty-one ids — File 41001–41012, Edit
42001–42007, Search 43001–43006, Macro 47001/47003, Settings 49001 — and
anything else is counted in `skipped` rather than guessed at. The View, Format
and Language ranges are deliberately absent: their ids are not stable across
Notepad++ versions, and a wrong guess would silently rebind a key to the wrong
command. A skipped count in the status bar is honest; a wrong binding is not.

The same reasoning applies to the settings. `DarkMode` becomes the theme,
`TabSettings tabSize` the tab width, `WidgetStyle name="Default Style"
fontSize` the font — and nothing else. Notepad++'s colour overrides, its
per-language styles and its plugin list have no meaning here.

### Points are not pixels

Notepad++ stores the font size in points; this editor stores it in pixels. The
conversion is `× 4/3`, clamped to the settings' own bounds, so a Notepad++ at
12 pt arrives as 16 px. Without it every imported font would be a quarter
smaller than the one the user was looking at.

### The status bar learned to talk

The import needs to report a count, and the status bar had no place for prose —
it showed the encoding, the line ending and the insert mode, or the recording
indicator. It now carries an optional `message` that takes precedence over both,
cleared at the top of `handle_key` so the next keystroke dismisses it. That is
the whole notification system: no toast, no dialog, no timer.

### Verification

Driven by hand under X11 against a fixture in `/tmp/nppcfg`: `config.xml` with
`DarkMode enable="no"`, `TabSettings tabSize="8"` and a 12 pt default style;
`shortcuts.xml` with four `ScintKey` entries, three of them known and one
(44001) not. `Ctrl+Shift+I` opened the `Import from:` prompt, the path was
typed, and `Enter` switched the editor to the light theme with a visibly larger
font and the status bar reading

```
Imported theme, font, tab width, 3 shortcuts (1 skipped)
```

`settings.txt` then held `theme light`, `font_size 16`, `tab_width 8` and
`key open Ctrl+Alt+O` — the fourth entry, the unknown one, correctly absent.

494 tests pass (193 in `nn-app`, 301 in `nn-core`), clippy is clean, and both
cross-target checks (`x86_64-pc-windows-gnu`, `x86_64-apple-darwin`) finish.
The AppImage rebuilds at 4.9 MB.

## Twelfth pass: a file browser for Open

`Ctrl+O` used to open a text prompt and ask for a path. That is fine for a user
who knows where the file is and useless for one who does not, so the prompt is
gone and `Open` now opens a modal list of the directory's contents.

### Why not a native chooser

A GTK or xdg-desktop-portal dialog is the obvious answer and the wrong one here.
The AppImage cannot assume either is present, so the dialog would have to be
optional, and the fallback would be — this widget. Two code paths, one of which
is only exercised on machines that happen to have GTK. One widget that looks the
same everywhere and costs nothing at startup is the better trade, and it is the
same trade the preferences dialog and the shortcut editor already made.

### The listing

`Browser` reads the directory on every change and keeps a flat `Vec<Entry>`:
the parent first, then directories, then files, each sorted case-insensitively.
Directories carry a trailing slash rather than a second colour, because the list
is one text area and a glyph is cheaper than a per-row style.

Hidden files are skipped unless the filter starts with a dot. That is the whole
hidden-file feature: no toggle, no setting, and the one gesture that means "show
me the dotfiles" is the one a user would try anyway.

The parent entry is never filtered out. A filter that matches nothing still has
to leave a way back up, and an unreadable directory still lists `..` so it is not
a trap — the error message goes in the list, where the user is already looking.

### Two bugs the live run found

**`dir.join("..")` does not normalise.** Entering the parent entry produced
`/opt/DEV/nn/crates/nn-core/..`, and `parent()` on that walks back *into* the
directory just left, so the second Backspace went nowhere. `..` now routes
through `up()`, which also gives the behaviour every file manager has: going up
highlights the directory you came out of.

**A filter left the highlight on `..`.** Typing `core` narrowed the list to
`../` and `nn-core/`, and Enter — the obvious next key — walked up the tree
instead of descending into the one thing the filter matched. Typing a filter is
a search and the answer is never `..`, so `insert_char` and `backspace` now
select the first entry that is not the parent, falling back to it when nothing
else matches.

Both were invisible to the unit tests as written and obvious within ten seconds
of driving the app. The tests were wrong in the same way the code was.

### The filter doubles as a path field

Typing an absolute path that names a file and pressing Enter opens it directly,
so a user who knows where they are going does not have to walk there. The check
is `is_absolute() && is_file()`; anything else falls through to the list.

### Verification

Driven by hand under X11. `Ctrl+O` opened the browser at `/opt/DEV/nn`, the
active document's directory; `Down`/`Enter` descended into `crates/`; typing
`core` narrowed the list to `nn-core/` with the highlight on it; `Enter`
descended; `src` then `Enter` reached `crates/nn-core/src`; typing `document`
and pressing `Enter` opened `document.rs` in a new tab, highlighted, 1 259 lines,
minimap populated. `Ctrl+O` again reopened in that file's directory. `Backspace`
on an empty filter went up to `nn-core` with `src/` highlighted. `Esc` closed
the dialog with the editor untouched. The light theme was checked by toggling
`Ctrl+Alt+T` with the dialog open, and `/etc/hostname` typed into the filter
opened directly.

521 tests pass (220 in `nn-app`, 301 in `nn-core`), clippy is clean, and both
cross-target checks (`x86_64-pc-windows-gnu`, `x86_64-apple-darwin`) finish.
The AppImage rebuilds at 4.9 MB.

## Thirteenth pass: a toolbar, and an honest INS

Two gaps closed, both of them things a user sees in the first ten seconds.

### The toolbar draws words

Notepad++ puts icons here. This puts words, and the reason is not laziness: the
binary has no bitmap in it, every widget is monospace text over rectangles, and
an icon set would be the only asset to ship, scale, theme twice, and keep in step
with the menu. A word is already the thing the menu calls the command.

`BUTTONS` is one table, and layout, hit testing, and the draw list all walk it in
the same order. That is the whole design: a button cannot be drawn somewhere it
cannot be clicked, because there is no second list to disagree with the first.
The test that walks every button and asserts the click lands on it is the one
that would catch a regression here.

The bar is toggleable from View and from Preferences, and the setting persists.
A toolbar nobody can turn off is a toolbar that becomes the first thing a user
complains about.

### Overwrite mode was a lie

The status bar had printed `INS` since the second pass. It was a string literal.
Pressing Insert did nothing, and there was no way to overwrite a character short
of selecting it and typing.

`Editor` now carries an `overwrite` flag — per document, not per window, because
Notepad++ keeps the mode with the buffer and a mode that changed when you
switched tabs would be worse than no mode at all. `overwrite_char` replaces the
character under the caret and steps past it.

Three cases fall back to a plain insert, and each is a decision:

- **A selection is replaced, not overwritten.** Overwriting a selection would
  delete text the user pointed at and then some.
- **At the end of a line there is nothing to replace**, so the character is
  appended. Without this the caret sticks at a line end and the mode becomes a
  trap.
- **A newline is never overwritten.** Joining two lines because the user typed a
  letter is not a trade anyone would accept.

### The undo bug this found

Overwrite typing is a run of one-character replacements, and the undo stack
coalesced them as typing — which it should. But `merge` only appended the
*inserted* text, so the second character's removed text was dropped and undo
restored `Xbcde` instead of `abcde`. The coalescing rule was right and the merge
was wrong, which is why the test that catches it is
`a_run_of_overwrites_undoes_as_one_step` and not a test of the coalescing rule.

The recorder gained a `Step::Overwrite(char)` for the same reason: a macro
recorded in overwrite mode has to replay in overwrite mode, or the recording is a
lie about what happened.

### Verification

545 tests became 567 (234 in `nn-app`, 333 in `nn-core`), clippy is clean, and
the release build runs on X11 at 120 Hz. The toolbar was checked by dumping a
frame offscreen: menu bar, then the button row with its four group separators,
then the tab strip, then the text — each bar one line lower than before, and the
text column starting below all three.

Frame times on `document.rs` (1 310 lines) at 1200×800: idle 0.28 ms, scroll
0.37 ms, type 2.04 ms at p99. The toolbar costs one rectangle, one hairline, and
fourteen text areas that are shaped once and never again.

## Fourteenth pass: noticing that the file changed

The plan promises "if a file changes on disk, we notice and reload it" and "never
loses a byte". Until this pass the editor did neither: a file rewritten by a
build tool stayed stale on screen until the user hit Reload, and there was no
drag-and-drop at all.

### Polling, not inotify

A watch per file means a descriptor per file, a thread or a poll loop to service
them, and a dependency. One `stat` per open file per second costs microseconds
and cannot fail in a way that loses data. Measured: **zero CPU ticks over five
seconds** with a file open, because the wake does three `stat` calls and goes
back to sleep.

`ControlFlow::WaitUntil` when something has a path, `ControlFlow::Wait` when
nothing does. An untitled buffer has nothing on disk to watch, and an idle window
should stay asleep rather than wake to stat nothing.

### The policy lives in the model

`Document::disk_change()` returns `None`, `Reload`, or `Conflict`, and the event
loop matches on it. The policy is three lines and it is the three lines that
decide whether an unsaved edit survives, so it belongs somewhere a test can
reach it rather than in a `match` arm inside the window handler.

A dirty buffer is **never** reloaded. The file is flagged, the tab grows a `!`
next to its `*`, and the decision is the user's. That is the whole difference
between noticing a change and losing an edit.

### The stamp, and its known hole

`FileStamp` is modification time plus length. The length is the cheap half that
catches a rewrite inside one clock tick; the mtime catches everything else.

It does not catch a same-length rewrite inside one tick, because this filesystem
reports the identical modification time for two writes in the same nanosecond
tick. That is written down as a test (`a_same_length_rewrite_in_the_same_clock_tick_is_missed`)
rather than left to be discovered: closing it means hashing the file on every
poll, which costs more than the bug. Every editor that polls instead of watching
has the same hole.

A deleted file reads as `None`, which compares unequal to any recorded stamp, so
a deletion is noticed the same way a write is.

### Drag and drop

`WindowEvent::DroppedFile` opens the path in a new tab. Three lines, and the
parity matrix's "drag-drop" line is done.

### Verification

567 tests became 581 (234 in `nn-app`, 347 in `nn-core`), clippy is clean.
Driven by hand: the app opened `eol.rs`, a shell appended a line to it, and the
log read `npp-rs: reloaded watched2.rs (changed on disk)` within a second with
the buffer showing the new text. The dirty path is covered by
`a_dirty_buffer_is_flagged_not_reloaded` and `a_conflict_does_not_touch_the_text`
rather than by hand, because there is no `xdotool` on this machine to type into
the window.

## Fifteenth pass: large files

The plan's target is "100 MB file opens in about a second". Measured before this
pass, in release, on a 100 MB / 2.2-million-line file:

| Stage | Time |
|---|---|
| read + parse | 1.31 s |
| syntax highlighting | 6.50 s |
| fold computation | 4.09 s |
| **total** | **11.90 s** |

The parse was already at target. The other ten seconds were two passes that are
O(file) and that nobody can see on a file they are scrolling through rather than
reading.

### What large-file mode is

`Document::is_large()` is true above 16 million characters. Above it the editor
skips syntax highlighting and fold computation, at load and after every edit, and
turns off the three features that copy the whole document to answer a question:

- **smart highlighting**, which scans for every occurrence of the caret's word;
- **word completion**, which builds a word index of the whole file;
- **the function list**, which copies the text and scans it for symbols.

The status bar says `LARGE`, because a user who does not know why their file is
uncoloured will think the editor is broken rather than thrifty.

### What it is not

Not the mmap-backed read-only piece table the plan sketched. That design buys a
memory ceiling this pass does not: a 100 MB file still costs about 215 MB, because
the rope holds the text and the style array holds a byte per character. It buys
nothing on time, which is what was actually wrong. The plan's sketch was written
before there was a measurement; the measurement says the problem is the two O(file)
passes, not the storage.

Editing stays enabled. Notepad++ disables some features on large files rather than
the file itself, and a read-only mode nobody asked for is a worse trade than an
uncoloured one.

### Verification

581 tests became 585 (235 in `nn-app`, 350 in `nn-core`), clippy is clean. The
open path is now 1.31 s of parse and nothing else — a 9× improvement, and inside
the plan's target once the GPU is up. Frame times on the same file at 1200×800 are
unchanged: 0.36 ms idle, 0.39 ms scroll, 3.33 ms type at p99, because the viewport
was never the problem.

## Sixteenth pass: the Style Configurator

Notepad++'s Style Configurator is a two-pane dialog: languages on the left, styles
on the right, a font and colour picker underneath. This editor has one lexer, so
the language pane would be a list of one. What is left is the part that matters —
one row per colour the editor draws with, and a way to change it.

### Sixteen slots

`Slot` names every colour a user can reasonably want to change: the nine syntax
styles the highlighter writes, plus the background, the gutter, the default text,
the selection, the caret, the current-line band, and the line numbers. Each has a
label for the dialog and a stable `key` for the settings file. The key is not
derived from the label, because a `style` line written by this version has to keep
meaning the same thing in the next one.

`Slot::get` and `Slot::set` are the only places that know which `Theme` field a
slot maps to. That is what makes the round-trip testable: every slot is written and
read back, so a copy-paste slip in the match arms fails a test rather than silently
painting the wrong thing.

### A palette, not a picker

Colours are chosen by stepping through a fixed palette of twenty-four, with
Left/Right. A real picker means a hue wheel or three sliders plus a hex field, which
is a dialog of its own; the palette is one row of swatches and covers every colour a
syntax theme actually uses. This is the one deliberate subset in the pass, and it is
written down in the module header rather than left for the user to discover.

The palette is also why the dark theme's own colours are in it: stepping from a
colour that is already a palette entry moves to the next entry, and stepping from
one that is not — a light-theme colour, say — goes to the nearest end rather than
jumping to the middle. Both cases are tested.

### Sparse overrides

`Settings.styles` is a `Vec<(Slot, Rgb)>` holding only the slots the user changed,
not a dump of the whole theme. Two consequences, both wanted:

- switching between light and dark keeps the edits, because the base palette is
  chosen first and the overrides go on top (`styles::effective_theme`);
- the settings file stays a list of deliberate decisions, which is what a person
  reading it wants.

`styles::record` is the single writer: it replaces any earlier entry for the slot
and re-sorts into dialog order. `Settings::render` sorts again rather than trusting
its caller, so the file is canonical however the vector was built.

### The dialog

It copies the preferences dialog's shape — a centred modal, a highlighted row, a
hint line — because two dialogs that behave differently for no reason are two
dialogs to learn. The differences are the swatch column and the live theme.

The swatch is drawn as a quad at column 16 and the hex value starts at column 22.
The first attempt put both at 16 and the swatch sat on top of the digits; the
screenshot caught it, the test could not have.

The dialog is seeded from the theme in force, not from the base theme, so it shows
what the editor is actually drawing with. Every change is committed immediately:
the slot is recorded, the settings are applied, and the file is written. There is no
OK button, which means there is no way to leave the dialog believing a change was
made when it was not.

### Verification

585 tests became 610 (260 in `nn-app`, 350 in `nn-core`), clippy is clean.

Checked offscreen at 1100x700 in both themes: the dialog centres, all sixteen rows
read, the swatches match their hex values, and the selected row is highlighted. The
Settings menu shows `Style Configurator...` under `Preferences...`.

The whole path from a settings file to the rendered colours was checked by writing
`style keyword #FF0000`, `style string #00FF00`, `style comment #808080`, and
`style background #101820` into a file and rendering with it: keywords came out red,
comments grey, and the background dark blue. The gutter kept its old colour in that
render, which is what prompted adding `Slot::Gutter` — a background change that
leaves a mismatched gutter is a half-finished feature.

Not driven by a pointer or a keyboard: this machine has no `xdotool` and no
`python-xlib`, so the key and click routing is covered by the widget's own tests and
by reading the code, not by a live session.

## Seventeenth pass: the Run command

Notepad++ has a Run menu with a dialog that takes a command line, substitutes a
handful of `$(NAME)` macros for facts about the current document, and hands the
result to the shell. This is the same idea with the same macro names, because a
user switching over should be able to paste their existing command and have it
work.

### Expansion is a pure function, and lives in `nn-core`

`run::expand(template, context)` takes a string and a `RunContext` and returns a
string. No I/O, no environment beyond `$(SYS.NAME)`, no process. That is what makes
it testable, and it is worth testing: a command line that silently expands to the
wrong path is a command that runs on the wrong file.

Ten macros, matching Notepad++: `FULL_CURRENT_PATH`, `CURRENT_DIRECTORY`,
`FILE_NAME`, `NAME_PART`, `EXT_PART`, `CURRENT_WORD`, `CURRENT_LINE`,
`CURRENT_COLUMN`, `SELECTION`, `NPP_DIRECTORY`, plus `$(SYS.NAME)` for an
environment variable.

Two decisions worth naming:

- **An unknown macro is left exactly as written.** Turning `$(CURRNET_LINE)` into
  an empty string would run the command against nothing and report success.
  Leaving it visible means the shell complains, which is a better failure.
- **An unclosed `$(` is literal text.** A command line containing `$(this` is more
  likely to be a shell construct than a typo, and swallowing the rest of the line
  would be worse than passing it through.

`NAME_PART` and `EXT_PART` split at the last dot, so `archive.tar.gz` gives
`archive.tar` and `gz`. That matches Notepad++ and it is the only split that makes
`$(NAME_PART).$(EXT_PART)` round-trip.

### `$(CURRENT_WORD)` walks the rope

The obvious implementation copies the document and scans it. On a 100 MB file that
is a 100 MB allocation to answer a question about four characters, and it happens
on every Run. `Document::word_at` walks outward from the caret with `char_at`
instead, which is O(word length) and allocates only the word.

It is on `Document` rather than in `run` because it is a text question, not a run
question, and because the completion index already has an opinion about what a word
is: alphanumeric or underscore. `foo.bar` with the caret on `bar` gives `bar`.

### The shell does the splitting

`run::spawn` runs `sh -c <expanded>`. Quoting, pipes, and redirection are the
user's to use; reimplementing them would be a worse shell. The child is not waited
for — a command that compiles a project takes minutes, and an editor that freezes
until it finishes is an editor that looks hung. Output goes to the terminal the
editor was started from, which is where someone who typed a command expects to find
it.

The working directory is the current file's directory, not the editor's. A relative
path in a command should mean what the user sees in the tab.

`spawn` is in `nn-core` rather than in the binary for one reason: it can then be
tested. `a_command_runs_and_writes_where_it_was_told` starts a real `sh`, writes a
file, and reads it back. That is the only way to know the argument order is right.

### Saved commands

`Settings.runs` is a `RunList`, written as `run <command>` lines with a leading `!`
meaning "do not save first". The rest of the line is the command, so a command with
spaces needs no quoting.

A saved command is labelled with the command itself, truncated at 48 characters. A
name would be a second thing to invent and keep in sync, and the command is what the
user recognises.

`save_first` defaults to on. A command that compiles the current file is useless if
the file on disk is the version from before the last edit. `save_active` skips a
document with no path rather than prompting: a modal Save As in the middle of
running a command is a surprise, not a service.

### Verification

610 tests became 646 (263 in `nn-app`, 383 in `nn-core`), clippy is clean.

Checked offscreen: the Run menu sits between Macro and Help, shows `Run...` with
`F5` in the shortcut column, and lists three saved commands from a settings file
below a separator. The app starts with that settings file and does not rewrite it,
because startup is not a change.

The expansion is covered by 27 tests in `run`, including every macro, an unknown
macro, an unclosed macro, a document with no path, a file with no extension, a
doubly-dotted name, adjacent macros, and an environment variable that is set and one
that is not.

Not driven by a pointer or a keyboard: this machine has no `xdotool` and no
`python-xlib`, so the F5 and menu-click routing is covered by the keymap test and by
reading the code, not by a live session.

## Eighteenth pass: split view

Two documents on screen at once, each with its own scroll position, its own tab
strip, and its own caret. This is the one feature that changes the shape of
everything else, so the pass was mostly about finding the cheapest way to add a
second pane without rewriting two thousand lines of code that assume there is one.

### The focused pane is the state

The obvious model is `panes: [Pane; 2]` plus `focused: usize`. It is also the
expensive one: `self.tabs`, `self.active`, `self.view` and `self.tab_bar` appear at
roughly two hundred and fifteen sites in `main.rs`, and every one of them would have
had to become `self.panes[self.focused].tabs`.

Instead the focused pane stays in the `State`'s own fields and the other one lives in
`other: Option<Pane>`. Focusing the other pane is four `std::mem::swap`s. Nothing
else in the file learns that splits exist.

The cost is that "the focused pane" is implicit: a reader has to know that `self.tabs`
means "the tabs of the pane with the caret". That is written down once, on the `Pane`
struct, and the alternative was two hundred edits with two hundred chances to pick the
wrong index.

### Geometry is a pure function

`layout.rs` holds `Split`, `Rect`, `panes`, `divider` and `ratio_at`, and nothing else.
The renderer and the click handler both call `panes` with the same arguments, which is
the only reason a click lands in the pane the user sees under the cursor. Two copies
of that arithmetic would drift the first time either was touched.

`MIN_PANE` is 120 pixels and `DIVIDER` is 6. A window too narrow for two panes at the
minimum splits evenly rather than showing one pane and a sliver: a window that cannot
hold a split should still show both documents.

The divider is six pixels rather than Notepad++'s hairline because a hairline is a
divider you cannot grab.

### Absolute coordinates

`EditorView` used to emit coordinates relative to the window, which was the same thing
as relative to itself. It now carries `origin_x`/`origin_y` and adds them to every
coordinate it emits: text, gutter, fold margin, bookmarks, minimap, line numbers, and
the bounds of every `TextArea`. `bar_height` became an explicit `text_height` because a
pane's height is no longer the window's.

This was done first, on its own, and verified by pixel-diffing a dump against the
previous build: `ImageChops.difference(...).getbbox()` over the region below the menu
bar returned `None`. A refactor that is supposed to change nothing is worth proving
that way before anything is built on top of it.

### What is per pane and what is not

Per pane: the editor, its view, its tab strip. Window furniture, drawn once across the
full width: menu, toolbar, prompt, find bar, results panel, status bar, function list.

The function list is docked to the window's right edge, so it spans the whole pane area
and the last pane's `right_dock` is set to its width. The first pane's is zero: a
minimap squeezed against a divider is a minimap with nowhere to go.

### Commands

`Split Vertical` and `Split Horizontal` create the second pane with a fresh untitled
buffer, which is what a split editor does everywhere else. `Move to Other View` moves
the active tab across and the focus follows it, because the point of moving a tab is to
go on working on it over there. `Clone to Other View` copies the document: two panes
editing one buffer would need a shared undo stack and a shared dirty flag, and
Notepad++'s clone is a snapshot too.

A new pane has to start with a document, because every invariant in `main.rs` assumes
`tabs` is non-empty. That placeholder is dropped the moment a real tab arrives, so a
split does not leave an empty tab for the user to close.

Notepad++ leaves these unbound. The defaults here are `Ctrl+Alt+Shift+V/H/U` and
`Ctrl+Alt+Shift+Right/Left`, which stay clear of the block-selection chords.

### Commands that mean "all documents"

Save All, Close All, the disk watch and the session file all used to iterate
`self.tabs`. With a split that is half the open files, so they now go through
`all_tabs`, which chains the focused pane's tabs with the other's. `watches_disk`
has its own `&self` version because the poll loop asks it before it has anything
mutable to hand out.

Close All collapses the split. An empty pane has no tab strip and no document, and
every invariant in the file assumes `tabs` is non-empty; leaving a split standing
over one fresh buffer would mean a second pane with nothing in it.

### Input routing

A click in the unfocused pane focuses it before anything else looks at the coordinates,
so the rest of `click` can go on assuming `self.tabs` is the pane under the pointer. The
divider is tested first: a click on the bar must not also move the caret, and a drag
that started on the bar has to keep moving it when the pointer crosses into a pane.

The wheel scrolls the pane under the pointer without moving the focus. Having to click
a pane before scrolling it would be a step too many.

### Two bugs found by looking

The divider was first drawn with `theme.menu_border`, which is the same colour as the
gutter it sits against: the bar read as part of the right pane's margin. `Theme` gained
a `divider` field.

More seriously, text was clipped at the pane's right edge rather than at `text_right`,
so a long line was painted over the minimap. That was true before this pass and is
visible in any single-pane screenshot with a long line; the split made it obvious
because the second pane's minimap sits in the middle of the window. `text_bounds.right`
is now `text_right()`, which accounts for both the minimap and the function list.

### Verification

646 tests became 670 (287 in `nn-app`, 383 in `nn-core`), clippy is clean.

`layout` has 21 tests: the two orientations, the origin, the minimum pane, a window too
narrow to split, the divider's position in both orientations, a drag that round-trips, a
drag past the edge, a drag that ignores the other axis, a horizontal drag measured from
the pane area's top rather than the window's, and a click landing in at most one pane at
three ratios.

Checked offscreen at 1100x700: a vertical split with two tab strips, two gutters, two
minimaps and a visible divider; the same horizontally; the same in the light theme; and
a split with the function list open, where the last pane's text stops at the list and
the first pane's minimap stays inside its own pane. The View menu shows the five new
commands with their chords.

The text-clipping fix was pixel-diffed against the previous build on a file of 200-column
lines: the difference is confined to `(1016, 62, 1100, 670)`, which is exactly the
minimap column. Nothing else moved.

Not driven by a pointer or a keyboard: this machine has no `xdotool` and no
`python-xlib`, so the divider drag, the pane-focus click and the wheel routing are
covered by the `layout` tests and by reading the code, not by a live session.

## Nineteenth pass: multiple carets

### What was already there, and what was not

`Selections` has held a set of carets since the column-editing pass, and `insert`
has always written to every one of them, recording the whole run as a single undo
step. What was missing was everything around that: nothing in the UI could create
a second caret, and every other operation on `Editor` -- arrows, backspace,
delete -- read `selections.primary()` and ignored the rest. So the feature was
half present and unreachable, which is worse than absent because it looks done.

This pass is therefore mostly about making the rest of `Editor` honour a set it
already had.

### Growing the set

`add_caret_above` and `add_caret_below` grow from the **outermost** caret in that
direction, not from the most recently added one. With carets already spread over
several lines, "one more above" has to mean above all of them, or a second press
after a Ctrl+click somewhere in the middle would drop a caret into the middle of
the block instead of extending it.

The new caret keeps the column of the caret it grew from, clamped to the target
line. Clamping rather than refusing means a stack can cross a blank line, which
is the common case: you put a caret at the top of a block, press down four times,
and one of the lines in between is empty.

`add_caret_at` -- Ctrl+click -- makes the new caret primary. The next keystroke
then anchors where the user just pointed, which is what makes "click, click,
click, type" behave the way it looks like it should. Adding a caret at a position
that already has one is a no-op rather than a duplicate, because `normalize`
merges them.

### Moving a set

`move_caret` now branches to `move_all_carets` when there is more than one. Each
caret gets a **fresh** desired column rather than sharing `desired_col`. Sharing
it would drag every caret onto the same column the moment one of them crossed a
short line, which is exactly the thing the remembered column exists to prevent.
The cost is that a caret stack drifts left when it passes through a short line and
does not recover; the single-caret path, which is the one users feel, keeps the
remembered column.

The horizontal-collapse rule is applied per caret: a left arrow with a selection
collapses to the selection's left edge instead of stepping past it. Applying it
only to the primary would have made a stack of selections behave differently from
one selection, for no reason a user could predict.

`Motion::DocStart` and `DocEnd` collapse the whole set onto one position, and
`normalize` merges them into a single caret. That is the right outcome: after
"go to end of document" there is one place to be.

### Deleting a set

`backspace` and `delete_forward` branch to `delete_at_carets`, which collects one
range per caret, applies them back to front, and records the run as **one** undo
step spanning from the first deletion to the last -- the same shape `insert` uses,
with the untouched text between them carried in both the removed and the inserted
string. Undo therefore puts every line back at once instead of one line per press.

A caret with nothing to delete in that direction is skipped, not clamped: at the
start of the document going backwards there is no character, and moving the caret
to make the deletion possible would be inventing an edit the user did not ask for.

### A bug the tests found

`insert` built its caret list in the order the edits were applied, which is back
to front, and handed it straight to `set_carets`. `Selections` documents itself as
ordered and non-overlapping, and `normalize` is what enforces that, but `insert`
never called it. The result was a set in descending order. Nothing downstream
happened to care yet -- `delete_at_carets` sorts its own ranges, `move_all_carets`
normalises -- but the invariant was a lie, and the first test that asserted
document order failed on it. `insert` now normalises. The single-caret path is
untouched: `normalize` returns early when there is one selection.

### Reaching it

Ctrl+click is the only way Notepad++ offers, and it is the only way to place
carets on lines that are not adjacent, so it is here. `Ctrl+Alt+Up` and
`Ctrl+Alt+Down` are added on top because a caret stack that can only be built with
a pointer is not reachable from a keyboard at all, and because they are bindable
like every other command. Escape drops the extra carets and keeps the primary one;
with a single caret it does nothing and falls through.

`char_at_point` was extracted out of `set_caret_from_point` so that a plain click
and a Ctrl+click resolve a point to a position the same way. It takes `&mut self`
because the row lookups cache.

### Verification

670 tests became 689 (287 in `nn-app`, 402 in `nn-core`), clippy is clean.

The 19 new core tests cover: growing down and up, the column clamp, both ends of
the document refusing to grow, growing from the topmost caret rather than the
last one, Ctrl+click making its caret primary, Ctrl+click on an existing caret not
duplicating it, clearing keeping the primary, clearing a single caret reporting
nothing to do, typing landing on every caret, a multi-caret insert undoing as one
step, backspace and delete across a set, a caret with nothing to delete being
skipped, a multi-caret backspace undoing as one step, arrows moving every caret,
shift+arrow extending every caret, a horizontal move collapsing every selection,
and DocEnd merging the set.

Checked offscreen at 900x500 on `editor.rs`: four carets on four consecutive
lines, then one `insert("// ")` that prefixes all four lines and leaves the four
carets at column 4. The status bar reads `Ln: 3, Col: 4`, which is the primary
caret. The renderer already drew one caret per selection -- that was written for
column blocks -- so nothing in `view.rs` changed.

Not driven by a pointer: this machine has no `xdotool` and no `python-xlib`, so
Ctrl+click and Escape are covered by the core tests and by reading the code, not
by a live session.

## Twentieth pass: context menus

### The gap

Right-click did nothing anywhere in the window. `MouseInput` handled the left
button and ignored the rest, so the one gesture every desktop user tries first
was dead. Notepad++ has two menus -- one in the text area, one on the tab strip --
and both were missing.

### One widget, two lists

`context.rs` holds a single `ContextMenu` that is handed a `Vec<Entry>`. The two
menus differ only in that list, so a second widget would have been the same code
with different strings.

The lists themselves are free functions, `editor_entries` and `tab_entries`,
taking plain state rather than the editor or the tab strip. Three reasons: the
lists can be tested without building an `Editor`; the bench draws the same menu
the application does instead of a copy that can drift; and the decision about
which rows are greyed is in one place rather than spread through `main.rs`.

### Rows that cannot run

A row that cannot do anything is greyed, not dropped. A menu whose length changes
with the state of the buffer is harder to aim at than one where the row you want
is always in the same place -- and a greyed row is information: it says the
command exists and that it does not apply right now.

Two buffers, not per-span colouring. `set_text` takes one attribute set for the
whole buffer, so the enabled labels are shaped into one buffer and the disabled
ones into another, each with blank lines where the other has text. Identical
metrics mean the two line up exactly, and the alternative -- `set_rich_text` with
an attribute per span -- is more machinery for the same picture.

`Theme` gained `menu_text_disabled`. It is dimmer than `menu_text` but still
legible: a greyed row that is nearly invisible is not telling anyone anything.

### Geometry

Every row is one line high, separators included, with the rule drawn through the
middle of the row. Variable-height separators would mean a second layout pass for
the sake of a horizontal line, and the hit-test arithmetic would have to walk the
row heights instead of dividing.

The menu is nudged back inside the window when the pointer is near an edge, and
pinned to the left or top edge when it is wider or taller than the window. A menu
opened at the bottom-right corner of the screen is the common case, not an edge
case.

### Input

A right-click while a menu is open closes it and opens nothing. A left-click
anywhere closes it, and a left-click on a row runs it. Any key closes it and does
nothing else -- a context menu is modal to the keyboard, which is what every
desktop does and which means Escape needs no special case.

The menu is tested before the menu bar in `click`, because it is drawn above
everything in the overlay pass. It is also tested before the modal dialogs: a
right-click while the preferences dialog is up opens nothing, so the two can
never be on screen together.

Right-clicking a tab makes that tab active before opening the menu. Notepad++
leaves the active tab alone and acts on the one under the pointer; switching is
the simpler rule, and the only visible difference is which tab is highlighted.

### Three commands that only the menu needed

`Close Others`, `Close to the Left` and `Close to the Right` did not exist. They
share `close_tabs_where`, which refuses when any tab it would close has unsaved
changes -- the same policy as Close All, and for the same reason: there is no save
prompt, so a bulk close that went ahead would silently drop edits. Only the
focused pane is touched; a bulk close that reached across a split would be closing
documents the user is not looking at.

`Action::Delete` was added for the editor menu's Delete row. The Delete key
already did this, but the menu needs a command to name, and naming it makes it
bindable.

### Verification

689 tests became 715 (313 in `nn-app`, 402 in `nn-core`), clippy is clean.

`context` has 26 tests: a closed menu has no size, an empty list leaves it closed,
every row is one line high, the width follows the longest label and is clamped at
both ends, a row maps back to its command, a disabled row and a separator offer
nothing, a click outside the frame or in the padding misses, the menu is nudged
left and up near the edges, a menu that fits is not moved, a menu wider than the
window sticks to the left edge, hovering marks the row and clears on a miss,
closing drops the entries, and the theme's disabled colour differs from its normal
one. Eight more cover the two lists: what is greyed with no history and no
selection, what is offered when there is, the caret commands, Close Others with
one tab, Close to the Left on the first tab, Close to the Right on the last, the
split commands without a split, and the read-only row's label following the state.

Checked offscreen at 900x500: the editor menu over `editor.rs` in both themes,
with Redo greyed because there is nothing to redo; and the tab menu with three
tabs, where Close to the Left is enabled and Close to the Right is not.

A dump with no flags is pixel-identical to the one taken before this pass
(`ImageChops.difference(...).getbbox()` is `None`), which is the check that the
renderer change is inert when nothing is open.

Not driven by a pointer: this machine has no `xdotool` and no `python-xlib`, so
the right-click routing, the click-outside close and the keyboard dismissal are
covered by the widget tests and by reading the code, not by a live session.

## Twenty-first pass: the document peeker

### What it is for

`Ctrl+Tab` in Notepad++ does not cycle to the next tab. It opens a list of every
open document, and holding the chord walks down it. That is a different gesture
from cycling: with twelve documents open, cycling to the one you want means
counting, and the list means looking.

The two were conflated here -- `Ctrl+Tab` was bound to Next Tab. Notepad++ leaves
plain cycling on `Ctrl+PageDown` and `Ctrl+PageUp`, so those are the bindings now
and `Ctrl+Tab` opens the peeker.

### A chooser, not a menu

`peeker.rs` is its own widget rather than a mode of `ContextMenu`, because the two
differ in more than their contents: a context menu hangs off the pointer and is
nudged back inside the window, while the peeker is centred and has no anchor at
all. Sharing the code would have meant a widget with two positioning strategies
and a flag to pick between them.

The list is capped at twelve rows and scrolls with the selection, so a session
with forty documents still shows a box you can read. The selection wraps at both
ends: `Ctrl+Tab` held down should walk the whole list and come back round, which
is the only way to reach the document above the current one without knowing how
many there are.

### Input

The peeker is modal to the keyboard. `Ctrl+Tab` and Down move down, Up moves up,
Enter commits, and anything else cancels. A chooser that let a keystroke through
to the document behind it would be a trap: the user cannot see the caret while
the box is up, so a stray character would land somewhere they were not looking.

A click on a row commits it and a click anywhere else cancels, which is the same
rule the context menu follows.

`open_peeker` refuses with one document. A list of one is not a choice, and the
box would cover the text for nothing.

### Verification

715 tests became 733 (331 in `nn-app`, 402 in `nn-core`), clippy is clean.

`peeker` has 18 tests: a closed peeker has no size, an empty list leaves it
closed, the list opens on the document it was given, a selection past the end is
clamped, the selection wraps at both ends, the box is centred, a box wider or
taller than the window sticks to the edge, the height caps at the visible row
count, the width follows the longest title and has a floor, a row maps back to its
document, a click outside the box or in the padding misses, scrolling keeps the
selection visible in both directions, hovering marks the row and ignores a miss,
and closing drops the list.

Checked offscreen at 900x500: six documents centred in the window with the third
highlighted. A dump with no flags is still pixel-identical to the one taken before
the context-menu pass, which is the check that both overlay widgets are inert when
closed.

Not driven by a pointer or a keyboard: this machine has no `xdotool` and no
`python-xlib`, so the `Ctrl+Tab` routing, the commit and the cancel are covered by
the widget tests and by reading the code, not by a live session.

## Twenty-second pass: never losing a byte

### The two ways a save loses data

A save is the only operation in an editor that destroys data on purpose. Every
other one is recoverable — an undo, a reload, a re-open — but the bytes on disk
are replaced by whatever is in the buffer, and if that was the wrong buffer the
old file is gone.

There are two distinct worries behind that, and they need two different answers:

- **The file you had.** You opened a config, changed one line, saved, and the
  version from last week is gone. A copy taken before the write answers this.
- **The work you have not saved.** You typed for an hour in a buffer that was
  never written anywhere, and the process died. A copy taken before the write
  does nothing for this, because there was never a write.

Notepad++ has both, under Preferences, Backup. So does this now.

### Backup on save

`nn-core/src/backup.rs` owns the policy, because it is all path arithmetic and
file copying — the sort of thing that is wrong in a way nobody notices until they
need the backup.

Two modes, matching Notepad++:

| Mode | Where the copy goes | What it is for |
|---|---|---|
| Simple | `foo.txt.bak` beside the file | The one you find without knowing the editor exists |
| Verbose | `backup/<encoded path>@<stamp>.bak` in the config directory | Several generations, and it works on read-only media |

The extension is **appended**, not substituted: `foo.txt` and `foo.rs` in one
directory must not both become `foo.bak`. Notepad++ does the same, so someone
arriving from it finds the file where they expect.

The verbose name encodes the full path with the separators turned into `@`,
because a backup directory is flat and two files called `main.rs` in different
projects must not share one backup. A path long enough to blow past a file name
limit falls back to `<name>@<hash8>@<stamp>.bak` — an unreadable backup is still a
backup, and a backup that fails to write because the name was too long is not.
The hash is FNV-1a, four lines and no dependency, stable across runs and machines.

Timestamped copies need pruning or the directory grows by one file per save
forever, and a feature that fills the disk is one the user turns off. Pruning
runs on the save that grew the directory, keeps the newest `backup_keep` per file,
and refuses to touch anything whose name is not in our own shape — deleting a
file we did not write would be a poor way to save space.

**Off by default.** Writing `.bak` files into someone's source tree without being
asked is a surprise, and version control already exists. This is the one place
where matching Notepad++'s default would have been the wrong call.

Every save goes through one function, `write_tab`, so no path can forget the
backup. A save that skipped it would be the one that mattered. Save As backs up
the file it is about to replace rather than the buffer's old path, which is not
the one being overwritten.

### Autosave

`nn-app/src/snapshot.rs` owns the manifest. Every few seconds the editor writes
the contents of each **modified** buffer into its own directory and records what
it wrote; on the next launch the manifest is read back and the buffers reappear,
still marked as modified, so the user can save them or throw them away.

Nothing is written over a file the user owns. The snapshot lives entirely inside
the configuration directory, which is why it is **on by default** at seven
seconds while backup-on-save is off: the two differ in who they disturb.

The buffer files are named by position (`0.txt`, `1.txt`) and the manifest is the
index. Deriving the name from the path would leave an untitled buffer with
nothing to derive from, and a buffer renamed by Save As would strand its snapshot
under the old name.

The text is written **before** the manifest. A crash between the two leaves a
manifest naming a file that is not there, which the restore skips; the other
order would name files that were never written at all.

A buffer whose file is already open from the session is replaced in place rather
than opened twice — the session and the snapshot describe the same tab, and two
copies of one file is how an edit gets lost. A buffer whose file has since been
deleted comes back untitled: the path is lost, which is a nuisance, and the text
is not, which is the promise.

The manifest is deleted once it has been read. Leaving it would restore the same
buffers again on the next launch, on top of whatever the user has done since.
Turning autosave off in the preferences clears it for the same reason.

### The event loop

The loop already woke once a second to poll the open files. Autosave needed a
second timer, so `about_to_wait` now sleeps until whichever comes first.

The combination is a free function, `snapshot::soonest`, rather than
`a.min(b)`. `Option`'s ordering puts `None` below `Some`, so `min` would read "no
timer" as "wake immediately" and spin the loop at full speed. Here `None` means
"nothing wants this timer" and loses to any real wait. That is a one-line trap
worth a test, and it has one.

`ControlFlow::Wait` when neither timer is wanted, which is what keeps an idle
window at zero CPU: an untitled clean buffer has nothing to watch and nothing to
save, so there is no reason to wake at all.

Closing the window snapshots before it saves the session. Closing is not a
decision to throw the work away, and the next launch offers it back.

### Verification

733 tests became 780 (357 in `nn-app`, 423 in `nn-core`), clippy is clean.

`backup` has 20 tests: the default mode writes nothing, every mode round-trips
through its key, stepping wraps both ways, a simple backup appends rather than
replaces the extension, encoding flattens separators, the hash is stable and
eight digits, a verbose backup carries the path and the stamp, two files with one
name get different backups, a path too long for a name falls back to a hash and
stays unique, no mode copies nothing, a simple backup holds the bytes that were
replaced, a verbose backup lands in the backup directory and leaves the source
tree alone, a file that does not exist yet has nothing to back up, a stamp is
recognised only in our own shape, pruning keeps the newest and drops the rest,
pruning counts each file separately, pruning leaves files it did not write,
pruning a missing directory is not an error, and pruning keeps at least one.

`snapshot` has 19 tests: an empty snapshot round-trips, a saved buffer
round-trips, an untitled buffer round-trips with no path, a path with spaces
survives, tab order is kept, a truncated line is skipped and the rest kept, a
line with a bad number is skipped, an entry with no file is skipped, comments and
blank lines are ignored, an unknown keyword is ignored, buffer names are
positional, a buffer round-trips through the directory, writing creates the
directory, reading a missing buffer is an error, autosave waits the interval then
is due, autosave wants nothing when off or clean, the earlier of two waits wins,
a missing timer loses rather than winning, and a snapshot knows which paths it
holds.

`settings` gained five tests for the new keywords and their clamps; `prefs` gained
two for the new rows, including that the autosave row reaches Off and stops there
rather than wrapping to five minutes.

Checked offscreen at 900x500 in both themes: the preferences dialog now lists
Backup on Save and Autosave, and still fits. A dump with no flags is
pixel-identical to the one taken before the peeker pass.

The restore and autosave loop was driven live, not just unit tested. A throwaway
`XDG_CONFIG_HOME` was seeded with a manifest naming a real file and a buffer
holding text that file did not have; the app started, printed `restored 1 unsaved
buffer(s)`, and seven seconds later the autosave tick rewrote the manifest and the
buffer with the same content. The file on disk was never touched. The process was
then killed with SIGTERM, so the snapshot present afterwards came from the timer
and not from the close handler.

Not driven live: the save path. This machine has no `xdotool` and no
`python-xlib`, so a save cannot be triggered from outside. `backup_before_save`
is covered by the core tests, and the three call sites all go through one
function, but the composition of backup-then-save is verified by reading rather
than by running.

## Twenty-third pass: pinning and painting tabs

Notepad++ lets a tab be pinned, which keeps it at the front of the strip and
refuses to close it by accident, and lets a tab be given one of five colours so a
group of related files can be spotted at a glance. Both are small features that
touch a lot of surfaces: the strip has to draw them, the context menu has to
offer them, the session file has to remember them, and every close path has to
respect them.

### Moving the tab, not sorting the strip

The obvious way to keep pinned tabs at the front is to sort at draw time: keep
`self.tabs` in whatever order the user opened files, and have the strip render
pinned ones first. That is a trap. The strip reports a click as an index into
what it drew, and every caller turns that index into `self.tabs[index]`. If the
drawn order differs from the stored order, every click maps to the wrong tab, and
so does every drag, every middle-click close, and the active-tab highlight.

So `pin_tab` moves the tab in `self.tabs` itself: it is removed from wherever it
sits and reinserted after the last pinned tab. The invariant that the strip index
equals the tab index is preserved by construction, and nothing downstream has to
know that pinning exists. The cost is a `Vec` remove and insert, which is
irrelevant at tab counts a human can read.

The same reasoning drives the restore path: a session is loaded, then the tabs
are stably partitioned so pinned ones come first. Doing it once at load keeps the
invariant true for the whole session rather than re-establishing it per frame.

### Pinned tabs are hard to close, not impossible

Three close paths exist: `close_tab` (one tab, from the menu or a middle click),
`close_tabs_where` (close others, close to the left, close to the right), and
`close_all`. All three now skip pinned tabs. `close_all` truncates to the pinned
prefix rather than clearing the vector, which is correct because pinning has
already moved every pinned tab to the front.

The context menu disables Close on a pinned tab rather than hiding it, so the
reason a click does nothing is visible in the menu instead of being a mystery.
Unpinning is always available, so a pinned tab is never stuck.

### A second session line, not a wider first one

The session format had `tab <line> <column> <path>`. Adding two fields to that
line would have made every existing session file unreadable, and sessions are
written on every clean exit, so a user who downgrades once would lose their tab
list. Instead there is a new keyword, `tab2 <line> <column> <pinned> <colour>
<path>`, and the parser still accepts the old `tab` line, defaulting to unpinned
and uncoloured. Writing always emits `tab2`. The reader is a match on the
keyword, so the two forms coexist without a version header.

### Colours are mixed toward the theme

The five colours are fixed hues, but painting a tab with the raw hue looks wrong
in one of the two palettes: a saturated yellow is unreadable behind light text in
the dark theme and behind dark text in the light theme. Each colour is therefore
mixed halfway toward the theme's tab background (`COLOUR_MIX = 0.5`), which keeps
the hue recognisable while guaranteeing contrast against the label in both
themes. The mix is done in linear-ish sRGB space by simple component averaging,
which is not colorimetrically correct but is stable, cheap, and produces the
muted look Notepad++ has.

The colour is chosen from a single cycling row in the context menu rather than a
submenu. A submenu needs hover tracking, a second popup layer, and dismissal
rules; a row that reads `Tab Colour: Green` and advances on click costs one line
of dispatch and is discoverable by clicking it once. Notepad++ itself uses a
submenu, so this is a deliberate simplification, noted here so it is not mistaken
for an oversight.

### The pin mark

A pinned tab gets a 6x6 square at its left edge in the active-rule colour, and
its title is shifted right by `PIN_INSET` so the square does not sit on the first
glyph. A square rather than a glyph because the strip draws text through glyphon
and adding a second text run per tab for one character is more machinery than the
feature deserves.

### A bug this pass found

`renderer.rs` called `pane.tabs.set_bounds(rect.x, rect.y)`, passing the pane's
top edge where the strip's width belongs. The strip therefore believed it was a
few hundred pixels wide when it was a few pixels tall, and clipped every title to
the first two or three characters. It went unnoticed because the default bench
dump has one tab whose title happens to be short. It is now
`set_bounds(rect.x, rect.width)`. This is worth recording because it is the kind
of defect that a screenshot review catches and a unit test does not: the layout
code was internally consistent, it was just given the wrong number.

### Verification

799 tests pass (376 in `nn-app`, 423 in `nn-core`), up from 780. Clippy reports
zero warnings across all targets, and the release build is clean.

`tabbar` gained 12 tests: the colour list is complete and round-trips through its
key, an unknown key is rejected, cycling visits every colour and returns to none,
each colour has a distinct rgb, a pinned tab is reported as pinned, a plain tab is
not, a coloured tab is reported as coloured, setting tabs keeps the active index,
an out-of-range active index is clamped, a pinned tab's title is inset, a pinned
tab draws its mark, and an empty strip draws nothing.

`session` gained 4 tests: a pinned coloured tab round-trips, an old `tab` line
still parses as unpinned and uncoloured, a `tab2` line with a bad colour falls
back to none, and a `tab2` line with a bad pinned flag falls back to false.

`context` gained 3 tests: a pinned tab offers Unpin and a disabled Close, a plain
tab offers Pin and an enabled Close, and the colour row names the current colour.

Checked offscreen at 900x400 with `tabs=5 pinned` in both themes: `file0.rs` and
`file1.rs` carry pin squares, `file1.rs` is green, `file2.rs` is blue, and the
strip background spans the full width. A dump with no flags is pixel-identical to
`/tmp/plain3.ppm`, so the strip change did not disturb the default layout.

## Twenty-fourth pass: distraction-free mode

Notepad++'s distraction-free mode is the same window with most of its furniture
switched off. That framing is the whole design: it is not a second window, not a
second layout, and not a second render path. It is a policy about which bars are
drawn, and policy is the part that has to be testable, so it lives in a module of
its own rather than as a dozen `if` statements scattered through the renderer.

### A value, not a flag

`Chrome` answers one question per bar: is it drawn? The renderer asks once at the
top of a frame and turns each answer into a height, so a hidden bar occupies
nothing and the panes grow to fill the space. No layout arithmetic needed a
special case, which is the point: the arithmetic that decides where a click lands
is the same arithmetic that decides where a pixel goes, and both read the same
value.

The alternative — a `distraction_free: bool` on `State` consulted at each call
site — was rejected because there are fourteen call sites between the renderer
and the hit tests, and a single one that forgot to consult it would put a click
in the wrong place. That is exactly the class of bug this pass found in the tab
strip last time.

### The menu bar reveals itself

Hiding the menu bar in a program whose only other route to its commands is a
keyboard shortcut the user has not learned yet is a trap. The bar therefore
reappears when the pointer is within two line heights of the top edge, which is
the gesture every full-screen video player has taught people. Two line heights
rather than a pixel count so the band scales with the font.

The band is evaluated against the pointer position the renderer is given, and
`CursorMoved` compares the answer before and after the move: crossing the
boundary changes the layout, so it has to repaint even though nothing else about
the frame changed. Without that comparison the bar would appear only on the next
unrelated redraw, which reads as a stutter.

The bar reserves its space when it appears rather than overlaying the text. An
overlay would avoid a one-line jump, but the menu bar's background is queued in
the quad batch that is drawn *before* the glyphs, so an overlay would put the
first line of text on top of the bar. Fixing that means a third render pass for
one bar, which is more machinery than a one-line shift is worth. Notepad++
animates the shift; we do not, and the difference is a jump instead of a slide.

### The column is capped and centred

Hiding the chrome on a wide monitor leaves a 200-character line stretched edge to
edge, which is not restful to read. The text column is therefore capped at 100
characters and centred. This is an addition to what Notepad++ does, recorded here
so it is not mistaken for a bug.

The cap is a character count rather than a pixel count so it follows the font:
zooming in should not silently widen the column. The inset is floored to a whole
character so the column edge lands on a cell boundary and the caret does not sit
half a glyph off the last column.

The inset is applied to the whole pane area *before* it is divided, not to each
pane afterwards. Insetting each half of a split would take twice the margin out
of half the width, and on a narrow window could leave a pane a pixel wide. Applied
before the division, a split in distraction-free mode gives each pane half of the
capped column, which is the behaviour that reads correctly.

### The minimap is suppressed, not disabled

`Minimap` gained a `suppressed` flag separate from `enabled`. Distraction-free
mode sets the first; the user's View > Minimap choice sets the second. Using
`set_enabled(false)` would have been one line shorter and would have turned the
minimap back on for someone who had deliberately turned it off, every time they
left the mode.

The same distinction reaches the click handler, which now tests `width() > 0.0`
rather than `is_enabled()`. A suppressed column that still claimed clicks would
swallow every click in the margin beside the text.

### What is not hidden

The prompt bar is never hidden. It is a modal question — "Save changes?" — and
hiding it would leave the program waiting for an answer the user cannot see.

The results panel is hidden only while it is empty. A search that has just
finished has results the user asked for, and hiding them would make the search
look like it did nothing. The find bar itself is user-driven and is not part of
the chrome policy at all.

### The mode does not persist

Notepad++ does not remember distraction-free mode across launches, and neither do
we. A user who quits in the mode and restarts into it would reasonably conclude
the menu bar had broken. The mode is a view of the current session, not a
preference, so it is not written to the settings file and `Mode::key` exists only
so the bench and any future session field can name it.

### Verification

815 tests pass (392 in `nn-app`, 423 in `nn-core`), up from 799. Clippy reports
zero warnings across all targets, and the release build is clean.

`chrome` is new and has 14 tests: normal mode shows everything, distraction-free
hides the furniture, the menu appears within two line heights of the top and not
below, a pinned menu stays put, toggling round-trips and drops the pin, the prompt
is never hidden, results survive when they exist, a narrow window gets no inset, a
wide window centres the column at exactly the cap, the inset lands on a character
boundary, normal mode never insets, a zero character width does not divide by
zero, the mode round-trips through its key, and every mode has a distinct label.

`minimap` gained 2 tests: suppression hides the column without forgetting the
preference, and a suppressed minimap stays hidden when the user turns it on.

Checked offscreen at 1400x700 in both themes. With `distraction` the menu bar is
present, the toolbar, tab strip, status bar and minimap are gone, and the text
column is centred at 100 characters. With `distraction_bare` the pointer is parked
mid-window and the menu bar is genuinely absent. With `distraction split` the
inset applies to the whole area before the split, so both panes sit inside the
capped column. With `distraction results find=fn` the results panel and the find
bar are still drawn. With `distraction menu` the File dropdown opens over the text
at the right place.

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`, so the chrome
plumbing did not disturb the default layout.

## Twenty-fifth pass: the change history

Notepad++'s Document Change History is the undo stack made visible: a docked list
of every change to the current document, newest at the bottom, where clicking a
row reverts the document to the state it was in after that change. It is the
feature that makes undo legible, because "Insert 3 chars" at line 412, four
minutes ago, is something a person can recognise and "undo, undo, undo" is not.

### The history is the undo stack, not a log beside it

The obvious implementation is a parallel `Vec<Change>` appended to on every edit.
It is also the one that drifts: every future edit path has to remember to append,
and the first one that forgets produces a panel that disagrees with Ctrl+Z. The
two lists would then have to be reconciled, which is a bug class rather than a
feature.

So `UndoStack` now stores `Vec<Step>` instead of `Vec<Edit>`, where a `Step` is an
`Edit` plus the two things the panel needs and the edit does not carry: when it
happened and which line it happened on. `push`, `undo` and `redo` survive as thin
wrappers over `push_step`, `undo_step` and `redo_step`, so no call site outside
the stack had to change. The history cannot drift from the undo stack because it
*is* the undo stack.

### The line is recorded at edit time

An `Edit` holds character offsets into the document as it was when the edit was
made. By the time the panel asks "which line was this on", the document has moved
on and those offsets mean something else — or nothing. The line therefore has to
be computed when the edit is recorded, which is why `Editor::record` exists as a
single funnel: all six places that used to call `self.undo.push(Edit { .. })` now
call `self.record(Edit { .. })`, which converts the offset to a 1-based line and
stamps the clock. One funnel means one place to get it right.

The timestamp comes from `now_secs`, and the panel formats it with a new
`clock_stamp`, derived from the existing `local_stamp` so the two cannot disagree
about what time zone they are in. A step with no timestamp — one built by
`Step::bare`, which the tests use — renders as `--:--:--` rather than as 1970.

### A merged typing run keeps its first stamp and line

Typing a word produces one undo step, not one per character: `UndoStack` merges
consecutive inserts at the caret. The merged step keeps the timestamp and line of
the *first* keystroke, because that is when the change began. Keeping the last
would make a word typed over four seconds appear to have been typed in the final
millisecond, and keeping the line of the last keystroke would put a wrapped word
on the wrong line.

### `redo_steps` is newest-first, `steps` is oldest-first

This was a real bug, caught by a test rather than by reading. `steps()` returns
the undo stack oldest-first, because that is the order the panel draws. The redo
stack pops from the *other* end, so `redo_steps()` has to return newest-first for
the two lists to concatenate into a single chronological history. Writing it the
same way as `steps()` produced a history whose redo half was reversed, which is
invisible until you undo something and look.

### Reverting is itself undoable

`truncate_to(keep)` pops steps off the undo stack and pushes them onto the redo
stack, exactly as repeated `undo` would. Reverting to a row is therefore an
ordinary undo of everything after it, and Ctrl+Z brings it back. A revert that
cleared the redo stack would be a destructive operation hiding behind a click on
a list row.

`Row::keep` is a *count* of steps to leave, not an index into the stack. The panel
is rebuilt from the stack on every edit, so an index would go stale the moment a
new edit arrived; a count means "undo until this many remain", which stays true.

### The panel knows nothing about documents

`ChangeHistory` is handed a `Vec<Row>` and reports back which row was clicked. It
has no reference to an `Editor`, no notion of an undo stack, and no way to change
a document. That is what lets its fifteen tests run without a document, and it is
why the panel could be reused for a search history or a macro list later.

Rows are shaped lazily and cached against `(row count, scroll, width in chars)`,
the same scheme the results panel uses: a frame where nothing moved costs one
rectangle and no text work. The panel scrolls to the bottom when it opens, because
the newest change is the one being looked for.

### It stacks above the results panel

Both panels are docked at the bottom and both can be open. The results panel sits
above the history panel, the find bar above both, and the panes shrink by the sum.
The alternative — one panel replacing the other — would mean opening the history
silently closed a search the user was reading.

### Verification

845 tests pass (408 in `nn-app`, 437 in `nn-core`), up from 815. Clippy reports
zero warnings across all targets, and the release build is clean.

`undo` gained 7 tests: a step records its line and time, a merged run keeps the
first, `describe` names inserts, deletes and replacements, `truncate_to` returns
the popped steps and leaves the rest, a truncated step is redoable, `redo_steps`
is newest-first while `steps` is oldest-first, and a bare step has no timestamp.

`editor` gained 6: `history` is oldest-first, `redo_history` is newest-first,
`revert_to` leaves the requested count and is undoable, `revert_to` past the end
clamps, `break_undo_group` splits a run into two steps, and the recorded line
follows the caret rather than the document start.

`datetime` gained 1: `clock_stamp` formats a known instant and agrees with
`local_stamp` on the hour.

`changelog` is new with 15 tests: a closed panel takes no space, an open one is a
header plus rows, the header counts changes, a row shows line, clock and
description, a missing timestamp renders as dashes, a long description is
truncated with a mark, scrolling clamps to the list, `row_at` maps a y to a row
and follows the scroll, a click past the end is `None`, and the panel caps at the
visible row count.

Checked offscreen at 900x500 in both themes. With `history` the panel shows
"3 changes" and three rows — `1 13:35:07 Insert 10 chars`,
`2348 13:35:07 Insert 15 chars`, `1 13:35:07 Replace 9 with 10 chars` — built
from three real edits, and the document pane shrinks to make room. With
`history find=fn` at 900x620 the find bar, the results panel and the history panel
stack in that order above the status bar.

A dump with no flags is pixel-identical to the re-baselined `/tmp/plain3.ppm`. The
baseline moved once during this pass because `editor.rs` itself gained two `use`
lines, which changes the document the bench renders; the render path is unchanged,
and the closed panel is proven to take no space by its own test.

## Twenty-sixth pass: three ways to arrange a tab strip

Notepad++ offers three tab bar arrangements under Settings > Preferences >
General > Tab Bar: a single row that scrolls, a multi-line block that wraps, and
a vertical column down the left edge. This pass adds all three, which finishes
the Documents group of the parity list.

### One geometry function, three arrangements

The strip already had a `tab_width` and a `tab_at` that walked the tabs
accumulating widths. Adding two more arrangements to that shape would have meant
three copies of the same walk, and the copies would have drifted: a click would
land on a tab the renderer had drawn somewhere else, and the bug would only show
up in whichever arrangement was tested least.

So the walk became `slots(line_height) -> Vec<Slot>`, where a `Slot` is a
rectangle relative to the strip's own corner. The rectangles, the titles and the
hit test all read it. There is one place that decides where a tab is, and the
three arrangements differ only inside it.

`tab_at` gained a `y` parameter as a consequence. A hit test on `x` alone was
correct while every tab was in the same row; with rows and with a column it would
report a tab in the wrong place. The signature change is the compiler's way of
finding every caller that had assumed one row.

### The strip now knows its own top

`set_bounds` used to take `(left, width)` and `push_quads`/`text_areas` took
`top` as a parameter. That worked while the strip was always a band at a known
height, but a column has to know where it starts to place its rows, and passing
the same `top` to three functions that all needed it was three chances to pass a
different one. `set_bounds` now takes `(left, top, width)` and the drawing
functions take neither.

### Height and width are separate questions

`height(line_height)` returns zero for the vertical arrangement and `width()`
returns zero for the two horizontal ones. The renderer asks both and insets the
pane by whichever is non-zero. A single `extent()` returning a number whose axis
the caller had to remember would have been shorter and wrong at every call site.

The column is a fixed 168 pixels rather than a fraction of the pane. A column
that resized with the window would move every tab under the pointer whenever the
window was dragged, and the point of a column is that a tab stays where it was
put. It is capped at half the pane width so a narrow window keeps some text.

### The column lives inside the pane, not beside it

A split view gives each pane its own strip, so the column belongs to the pane it
labels. It is therefore drawn in the pane's own rectangle and the text is inset
past it, rather than the column being a window-level sidebar.

The inset is applied to the pane *rectangle*, through a new
`layout::inset_left`, not to the view's origin. The view derives the gutter, the
caret, the selection rectangles and the hit testing from its origin and width; a
view that knew about the column separately could disagree with the rectangle
about where the text begins, and the disagreement would be a caret drawn one
column off. Insetting the rectangle moves all of them together.

`State::pane_rects` applies the same inset, because that is what decides which
pane a click landed in. Without it a click in the column would be treated as a
click on the text underneath.

### A click on a tab stops there

The click handler used to switch tabs and then fall through to the text handling,
which was harmless while the strip was a band above the text: the fall-through
found no pane containing the point. With a column the point *is* inside the pane,
so the fall-through would have moved the caret in the document the click had just
left. A click on a tab now returns.

The band below the toolbar still swallows clicks that miss a tab, so a click in
the empty space to the right of the last tab does not move the caret either.

### Multi-line stops at three rows

Notepad++ grows the multi-line strip without bound, which on a small window
leaves a strip taller than the text it is labelling. Three rows is the point past
which the strip costs more than it gives; beyond that the tabs keep going on the
last row and are clipped, which is what single-row mode does with every tab that
does not fit. This is a deliberate deviation and is recorded here so it is not
mistaken for a bug.

The rows are counted by the same walk that places the tabs, so the height and the
slots cannot disagree about how many rows there are.

### One document still has a strip

*Corrected in the forty-second pass.* This section used to say that all three
arrangements hide when there is a single tab. That was wrong twice over: it was
not what Notepad++ does, and it was not what the code did after the tab strip was
rewritten. Notepad++ always shows the strip, even for one document, and it has no
option to hide it -- the close and pin buttons live on the tab, so a hidden strip
would take two commands away from a single-document window.

`TabBar::is_shown` is therefore `!self.tabs.is_empty()`, and an empty strip is the
only one with no height. In a split view where one pane has several documents and
the other has one, both panes' text starts at the same offset, which is the point.

### The arrangement is a View menu choice, not a preference

`TabLayout` is persisted in `settings.txt` as `tab_layout <key>`, but it is
restored in `load_settings` beside the split rather than through the preferences
dialog, because the dialog has no control for it. A dialog round trip must not
disturb an arrangement the user made with the menu.

The menu entry cycles rather than offering three entries with a tick beside the
current one: the menu has no check marks yet, and a cycle is one command to bind
and one line to persist. The status bar names the arrangement that was picked, so
the cycle is not silent.

### Verification

865 tests pass (428 in `nn-app`, 437 in `nn-core`), up from 845. Clippy reports
zero warnings across all targets, and the release build is clean.

`tabbar` gained 15 tests: every layout round-trips through its key, stepping wraps
back to the first, every layout has a distinct label, only the vertical one is
vertical, a single tab has no column either, the vertical layout takes width and
no height, the horizontal ones take height and no width, a column never takes more
than half the pane, a vertical tab is a row of the column, a vertical tab does not
extend past the column, a multi-line strip wraps when it runs out of room, it
stops growing at the cap, a single-row strip never wraps, a multi-line strip that
fits is one row, and the layout survives a change of tabs.

`layout` gained 3: an inset moves the left edge and keeps the right one, an inset
never swallows the whole pane, and a zero inset changes nothing.

`settings` gained 2: a tab layout round-trips through the file, and an unknown key
falls back to a single row.

Checked offscreen in both themes. With `tabs=6 vtabs` the column runs down the
left edge, the active tab carries a rule on its leading edge, and the text starts
to the right of the column. With `tabs=20 mtabs` the strip wraps onto three rows
and the pane shrinks by three rows. With `tabs=20` and no layout flag the same
twenty tabs stay on one row and clip, which is the old behaviour unchanged. With
`tabs=5 vtabs split` each pane gets its own column and the right pane, holding one
document, gets none. With `tabs=5 vtabs pinned light` the pin squares and the
muted tab colours render in the column in the light theme. With
`tabs=5 vtabs distraction` the strip is hidden entirely, because distraction-free
mode hides the tabs whatever their arrangement. The View menu shows "Tab Bar
Layout" with its chord between "Document Change History" and "Word Wrap".

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`, so the default
single-row strip is unchanged.

## Twenty-seventh pass: the character grid

Notepad++'s "ASCII Insertion Panel" is a grid of the 256 code points of Latin-1
with a click inserting the character at the caret. It exists for the characters a
keyboard cannot produce: the box-drawing set, the accented letters of a language
the layout is not set to, the currency signs. This pass adds it, which finishes
the Transform group of the parity list.

### The whole range, not a curated set

The grid is all 256 cells. A curated set -- the arrows, the box-drawing
characters, the common accents -- would be a different feature, a favourites
list, and would still leave the user without the character they came for. The
point of the panel is that you can find something you cannot type, and you cannot
find it in a list somebody else chose.

### Control characters are shown but not insertable

Notepad++ inserts them. That puts a NUL or a BEL into a text file: invisible in
the editor, and a surprise for every tool that reads the file afterwards. The
cells are still drawn, dimmed, because a table with holes in it reads as a bug
rather than as a decision -- but a click on one does nothing and the readout says
`control`.

The rule is `!char::from(byte).is_control()`, which is one line and covers the C0
range, DEL, and the C1 range at 0x80-0x9F that Latin-1 hides inside what looks
like printable space. The no-break space at 0xA0 is not a control character and
is insertable, which is correct: it is a real character that real documents use.

### A space needs a glyph

A space is a real character and has to be clickable, but a blank cell is
indistinguishable from a cell that failed to draw. Whitespace cells draw a middle
dot, which says "there is something here and it is not visible". The no-break
space gets the same treatment for the same reason.

### The panel floats

Notepad++ docks it. This editor's right edge already belongs to the function
list, and stacking a second docked panel there would make the character grid's
height depend on how many functions the current document happens to have -- so
the grid would move every time the caret moved. A palette of fixed size never
moves, which is what a grid you are aiming at needs.

Floating also makes it a palette rather than a modal. It is opened from the Edit
menu while the caret is in the text, it takes the arrow keys and Enter while it
is up, and it lets every other key through to the document. Picking one character
from a grid is rarely the whole errand -- the box-drawing set is used a row at a
time -- so a click inserts and leaves the panel open. Escape closes it, and so
does a click outside it, which is the only way out that does not need the
keyboard.

### Cells shrink only when the panel would not otherwise fit

Twenty pixels a cell makes the panel 336 wide, which fits beside the text on any
window this editor is usable in. A window smaller than that would clip the bottom
rows, and a row you cannot reach is worse than a cell that moved.

So `cell()` returns twenty pixels unless the window cannot show the panel at that
size, in which case it shrinks just enough to fit, down to a floor of eight
pixels below which the glyphs stop being distinguishable. The shrink is a
fallback rather than a policy: a grid whose cells resize with the window moves
every character under the pointer whenever the window is dragged, and the point
of a grid is that a cell stays where it was put. It only gives way when the
alternative is unreachable rows.

### The readout is the caption's right half

The header carries the title on the left and the hovered code point on the right,
as `U+00B7 (183)`. Hex and decimal because the two are used for different things:
hex is what a source file or a font tool wants, decimal is what a character-map
reference lists. The readout is right-aligned by counting characters, which is
exact because the font is monospace.

### Verification

892 tests pass (455 in `nn-app`, 437 in `nn-core`), up from 865. Clippy reports
zero warnings across all targets, and the release build is clean.

`charset` is a new module with 27 tests: the grid covers the whole range, a click
outside the grid hits nothing, the header is not a cell, a closed panel hits
nothing, control characters are not insertable, printable ASCII is, the Latin-1
supplement is, a control cell draws nothing, a space cell draws a placeholder, a
letter cell draws the letter, the panel is closed until opened, toggling opens and
closes, closing forgets the hover, the panel is centred, it never leaves the
window, it is wider than the grid by its padding, it is taller than the grid by
its header, a roomy window gets full-size cells, a small window shrinks them to
fit, they never shrink past the minimum, a shrunk grid still hits every cell, the
readout is empty when nothing is hovered, it names the code point, it says
`control` for a control character, a control character has no character to
insert, stepping the hover wraps at the edges, and stepping moves one cell.

Checked offscreen in both themes. With `charset` the panel is 336 by 364 centred
in a 900 by 500 window: the header rule sits at y=100, the first dimmed row at
y=104, the last row ends at y=424 and the frame closes at y=431, which is the
arithmetic the tests assert. The hovered cell at 0xB7 carries the selection
colour and the readout reads `U+00B7 (183)`. The two C0 rows, DEL and the two C1
rows are dimmed; the printable rows are not. In the light theme the dimmed cells
read as light grey blocks against the white frame. At 420 by 300 the cells shrink
to 16 pixels and the panel is 272 by 300, which fits the window exactly. The Edit
menu shows "Character Panel" with `Ctrl+Alt+C` between "Column Editor..." and
"Word Completion".

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`, so nothing else
moved.

## Twenty-eighth pass: searching as you type

Notepad++ has two search modes that look like one. `Ctrl+F` opens the find bar
and waits for you to press Enter. `Ctrl+Alt+I` opens the same bar and searches on
every keystroke, moving the selection to the first match as you type. The
temptation is to build a second bar for the second mode. That would be a mistake:
the toggles matter just as much while typing as they do after Enter. Match case,
whole word, regex, the search mode buttons -- all of them have to be live in
incremental mode, and duplicating them means duplicating every bug they will ever
have. So incremental search is a *mode* of the existing bar, one private boolean
on `FindBar`, and everything else is shared.

### The anchor is the caret, not the previous match

The subtle part of incremental search is where the next search starts. The
obvious implementation searches from the current match, so that typing `f`, `fo`,
`foo` walks forward through the document one match at a time. That is wrong. If
you type `f` and the first `f` in the file is on line 900, the view jumps to line
900; then you type `o` and the search starts from line 900, so you land on the
first `fo` *after* line 900 and the view jumps again. Refining a search should
narrow it, not scroll away from where you were.

So `open_incremental_search` records an anchor: the caret position, the
viewport's `first_line`, and its `top_offset`, captured at the moment the bar
opens. Every keystroke searches from that anchor. Typing `f`, `fo`, `foo` all
search the same region of the document, and the selection moves to the first
match of the *current* pattern at or after the anchor. The view stays put unless
a match is found, and then it scrolls to the match -- once, to the right place.

Escape restores the anchor: caret, `first_line`, `top_offset`. An abandoned
search should leave the document where it found it. Find mode does not do this,
and the asymmetry is deliberate: in find mode you pressed Enter on each match,
so each match was a decision you made and the caret belongs where you left it. In
incremental mode you never confirmed anything, so nothing should stick.

### Volatile by default

Notepad++'s incremental search is volatile: close the bar and the highlights go
away. Its ordinary find is not: close the bar and the highlights stay, so you can
read the document with the matches marked. We default `volatile_find` to on,
which matches Notepad++, and expose it as a preferences switch for people who
want the other behaviour in both modes.

`close()` takes the flag as a parameter rather than reading settings itself. The
bar does not know about `Settings` and should not start: the policy -- "does
closing this bar discard its results" -- belongs to whoever is closing it, and
`main.rs` is where that decision lives. Passing it in also makes the two
behaviours testable without constructing a settings object.

### A closed bar with live matches is still active

The bug this pass had to avoid: `refresh` and `after_edit` both guarded on
`self.open`, so a closed bar with retained highlights would keep showing stale
offsets after the document changed underneath it. Highlights would drift onto the
wrong characters, and the counter would keep claiming a match count that no
longer held. The fix is `is_active()`, which is `open || !matches.is_empty()` --
the bar has work to do if it is visible *or* if it is still painting something.
Both guards changed to use it. This is the kind of thing that only shows up in
the non-volatile configuration, which is why the switch and the guard landed in
the same pass.

### Wiring

`Ctrl+Alt+I` is Notepad++'s own chord for incremental search, so the keymap takes
it verbatim. The Search menu gains "Incremental Search" after "Find in Files...".
The preferences dialog gains a "Volatile Find" switch, which pushed `ROWS` from
10 to 11; the row sits after "Function List" because both are behaviour toggles
rather than appearance settings.

`State.incremental_anchor` holds the `(caret, first_line, top_offset)` triple.
`follow_incremental()` runs after `refresh_find()` in the printable-character
arm, so the pattern is already updated when the search runs. `Action::Find` and
`Action::Replace` clear the anchor, because opening the ordinary bar means the
user is no longer refining.

### Verification

906 tests pass (469 in `nn-app`, 437 in `nn-core`), up from 892. Clippy reports
zero warnings across all targets, and the release build is clean.

`findbar` gained its first test module, 10 tests: a fresh bar is not incremental,
`open_incremental` makes it so, `open_find` clears it, `open_replace` clears it,
a volatile close drops the matches, a non-volatile close keeps them, a
non-volatile close still clears the error, a closed bar with matches is active, a
closed bar without matches is not, and an open bar is active. `settings` gained 2
tests for the new line and its round trip; `prefs` gained 2 for the row's toggle
and for every row having a label and a value.

Checked offscreen. With `incr` the bar reads `Find: fn `, the counter reads
`1/195`, the status bar reads `Ln: 56, Col: 4, Sel: 3`, and the selection is
visible on line 56 -- the first `fn ` at or after the anchor, scrolled to
mid-screen. With `nonvolatile` the bar is closed, the status bar is back to
`Ln: 1, Col: 1, Sel: 0`, and the `the` highlights persist on lines 1, 3, 4 and 5
with 369 matches retained. The preferences dialog shows "Volatile Find  On" as
its eleventh row.

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`, so nothing else
moved.

## Twenty-ninth pass: the assist group

Notepad++'s "Auto-Completion" submenu is four commands that look unrelated and
are not. Word completion, function completion, function parameter hints, and
path completion all answer the same question -- "what could go here" -- and all
answer it in a small box anchored to the caret. The previous pass had one of the
four. This pass adds the other three and, in the process, turns the popup from a
word list into a general chooser.

### One popup, four modes

The temptation is four popups. They would share ninety percent of their code and
drift apart in the remaining ten, which is exactly the ten that matters: the
width calculation, the flip-above-the-caret rule, the scroll offset, the
highlight. So there is one `CompletionPopup` and one new type:

```rust
pub struct Item {
    pub label: String,
    pub insert: String,
    pub back: usize,
}
```

`label` and `insert` are separate because a function row must *show* its
signature and *insert* something shorter. The row reads
`set_caret(&mut self, pos: usize)`; accepting it must produce `set_caret()` with
the caret between the parentheses, ready for the first argument. That is
`insert = "set_caret()"` and `back = 1` -- one `CharLeft` after the insertion
puts the caret where the user wants it. `Item::word` sets `insert == label` and
`back == 0`, so the old behaviour is the degenerate case rather than a special
one.

`CompletionMode` carries the rest of the difference: which characters refilter
the list, and what the list is filled from. `Path` accepts any non-space
character, because a path may contain dots, dashes, and slashes; the other three
accept only alphanumerics and underscore, because a word that contains a space
is two words. `refilters_on` is a method on the mode rather than a flag on the
popup, so `type_text` and `handle_completion_key` cannot disagree about it.

### Function completion reuses the function list's scanner

`complete::functions` walks the document with `symbols::scan_line`, the same
scanner the Function List panel uses. Two scanners would eventually disagree
about what a declaration looks like, and the disagreement would be visible: a
function in the panel that completion refuses to offer. `scan_line` became
`pub(crate)` for this and nothing else.

The signature is assembled from the whole document, not from the declaration
line. A Rust signature routinely spans lines:

```rust
pub fn set_caret_from_point(
    &mut self,
    x: f32,
    y: f32,
    extend: bool,
) -> bool
```

Reading only the declaration line would show `set_caret_from_point(` and stop.
So `functions` finds the opening parenthesis with `find_call_paren` and then
walks forward with `balanced` until the parens close, wherever that lands.
Declarations whose parentheses never close are dropped rather than shown
truncated -- a signature that lies is worse than a missing row.

Order is declaration order, not alphabetical. The user is usually completing a
call to something they just read, and in a file the reader's mental model is
top-to-bottom. `params_of` takes the first declaration of a name for the same
reason: the first one is the one the reader saw.

### `call_at` walks backwards

Parameter hints need to know which call the caret is inside and which argument
it is on. `call_at(text, caret)` walks backwards from the caret, counting
parentheses, and stops when it reaches a `;`, `{`, or `}` at depth zero. Those
three characters are the statement boundaries in every language this editor
claims to understand: past a semicolon the call is over, and past a brace the
caret is in a different block entirely. Without the stop the walk would happily
report a call from three statements ago and highlight an argument that has
nothing to do with the caret.

The argument index comes from `count_commas`, which tracks `(`, `[`, and `{` but
deliberately not `<` and `>`. Angle brackets are not reliably balanced in real
code -- `a < b`, `Vec<Vec<T>>` in older editions, comparison chains -- and
treating them as nesting would make the argument index wrong far more often than
it would make it right. A comma inside a generic argument list is rare; a
comparison operator is not.

`arg_range(params, index)` returns the byte range of one argument, trimmed of
whitespace, so the hint can underline exactly the argument the caret is in
rather than the whole signature.

### A hint is not a list

`ParamHint` is a separate type from `CompletionPopup` even though both are boxes
near the caret. A popup is a list: it has a selection, it scrolls, it accepts and
cancels. A hint is a label: it has no selection, nothing to scroll, and nothing
to accept. Modelling it as a one-item popup would mean carrying a selection index
that is always zero and an accept path that does nothing, and every future change
to the popup would have to ask whether it applies to the degenerate case.

What the two do share is the geometry: `set_viewport`, `prepare`, `push_quads`,
`text_areas`, and the flip-above-the-caret rule when there is no room below.
`ParamHint` implements that itself in about eighty lines, which is cheaper than
the abstraction that would unify them.

### The hint appears on `(` and then follows the caret

Notepad++ has a setting for this -- "on input" versus "on `(`" -- because a hint
on every keystroke is noise. We implement the quieter behaviour and no setting:
the hint opens when `(` is typed, and from then on `after_edit` refreshes it only
if it is already open. Typing an argument updates the highlight; typing `)`
closes it, because `call_at` no longer finds a call. Escape closes it before it
clears extra carets, so the first Escape dismisses the hint and the second
returns to a single caret -- the order a user expects from a transient overlay.

`follow_param_hint` bails out on large documents. It reads the whole text to find
the declaration, and on a file past the large-document threshold that read is
more expensive than the hint is worth. The hint simply does not appear there,
which is the same trade the word-completion pool makes with `MAX_POOL_BYTES`.

### The active argument is drawn in the accent colour

`prepare` builds the hint with `set_rich_text` and three spans: the text before
the active argument, the argument itself, and the text after. The middle span
uses `theme.toggle_on`, the same accent the editor uses for a toggled setting, so
the highlight reads as "this one is live" rather than as a selection. The shaped
text is cached under `(String, usize, Rgb)` -- the signature, the active start,
and the colour -- because the same signature is re-prepared on every keystroke
while the caret moves within one argument, and re-shaping identical text is pure
waste.

### Policy is passed in, not read

`ParamHint::show` takes the signature, the active range, and the anchor;
`close` takes nothing. Neither reads `Settings`. The hint does not know whether
the user has hints enabled, whether the file is large, or whether a call is
under the caret -- `main.rs` decides all three and calls `show` or `close`
accordingly. This keeps the widget testable without a settings object and keeps
the policy in one place, which is the same reason `FindBar::close` takes its
volatile flag as a parameter.

### Path completion resolves against the document's directory

`path_prefix(text, caret)` returns the run of non-space, non-quote characters
ending at the caret, but only if it contains `/` or `\` and does not start with
`//`. The separator requirement is what stops the popup from firing on every
word; the `//` exclusion is what stops it from firing on a comment.

`split_fragment` cuts the prefix at its last separator into a directory part and
a stem. `resolve_dir` makes the directory absolute against the document's own
directory, so `src/ma` in a file at `/opt/DEV/nn/crates/nn-app/src/main.rs`
resolves to `/opt/DEV/nn/crates/nn-app/src/` -- the directory the user is
thinking in, not the process working directory, which for a GUI launched from a
desktop file is `/` and would make every relative path fail.

`complete_path` lists the directory and keeps entries whose name starts with the
stem, case-sensitively, because filesystems are. Directories sort first and carry
a trailing separator, so accepting one continues the path instead of ending it.
Hidden entries appear only when the stem itself starts with a dot, which is how
every shell behaves. The exact name is not re-offered -- completing `main.rs` to
`main.rs` is a no-op the user has to dismiss. The list is capped, because a
directory with ten thousand entries should not build ten thousand rows to show
twelve.

### Two bugs found by looking

`MAX_WIDTH` was 420, chosen when the widest thing a popup showed was a word. A
sixty-character Rust signature is wider than that, and the popup was truncating
it with an ellipsis -- the one piece of information the row exists to convey.
Raised to 560.

The second was subtler. Both `prepare` methods computed the column count as
`(width / char_width).max(4.0) as usize`. In f32, `560.0 / 12.0` is not 46.67 but
46.99999, and `as usize` truncates to 46, so the popup laid out one column fewer
than it had room for and drew a spurious ellipsis on a row that fit. The fix is
`.max(4.0).round() as usize`. It only showed up because the function popup made
rows long enough to reach the clamp.

### Wiring

Four chords, three of them Notepad++'s own:

| Chord | Action |
| --- | --- |
| `Ctrl+Space` | Word Completion |
| `Ctrl+Shift+Space` | Function Completion |
| `Ctrl+Shift+Alt+Space` | Function Parameters Hint |
| `Ctrl+Alt+Space` | Path Completion |

Document Completion has no default chord in Notepad++ and gets none here; it is
reachable from the Edit menu, where all four now sit after "Word Completion" in
the order above. `Action::ParamHint` toggles rather than opens, because a hint
that is already showing should close when the chord is pressed again.

`accept_completion` records a `Step::Insert` and one `Step::Move(Motion::CharLeft)`
per step of `back`, so a macro that accepts a function completion replays the
caret landing between the parentheses. Recording the insertion without the
back-steps would replay it with the caret after the closing paren, which is a
different edit.

`render()` grew its twenty-eighth argument. The overlay order is now menu
dropdown, completion popup, parameter hint, context menu, peeker, then the
dialogs. The hint sits directly above the popup because the two never appear at
once -- a hint is showing means a call is open, and a popup is showing means the
user is choosing -- and putting them adjacent keeps the z-order obvious.

### Verification

963 tests pass (496 in `nn-app`, 467 in `nn-core`), up from 906. Clippy reports
zero warnings across all targets, and the release build is clean.

`nn-core::complete` went from 14 tests to 44: `collect_many` over several
documents with a limit, `functions` finding single-line and multi-line
signatures, dropping an unclosed declaration, and preserving declaration order,
`params_of` taking the first declaration, `call_at` reporting the innermost call
and the argument index across nested parens, brackets, and braces, stopping at a
semicolon, a brace, and the start of the text, ignoring commas inside angle
brackets, `arg_range` trimming and returning `None` past the last argument, and
`path_prefix` accepting separators while rejecting `//` and bare words.

`nn-app::complete` went from 17 tests to 32: `Item::word` and `Item::function`
shapes, `selected_label`, `refilters_on` per mode, `ParamHint` opening, closing,
flipping above the caret near the bottom of the viewport, and highlighting the
active argument. `nn-app::paths` is new with 15 tests over a self-deleting
scratch directory: fragment splitting, relative and absolute resolution,
directories first with a trailing separator, hidden entries only behind a dot,
the exact name suppressed, and the cap.

Checked offscreen at 900x500 against `crates/nn-core/src/editor.rs`. With
`func_complete` the popup shows `set_caret(&mut self, pos: usize)` and the
highlighted `set_caret_from_point(&mut self, x: f32, y: f32, extend: bool)` in
full, no ellipsis, caret at Ln 197 Col 16. With `param_hint` the box reads
`is_closer(c: char)` with `c: char` in the accent colour, caret at Ln 56 Col 14.
With `path_complete` the rows are `main.rs`, `menu.rs`, `minimap.rs` from the
real `crates/nn-app/src` listing with `main.rs` highlighted, caret at Ln 9
Col 12. With `doc_complete` the rows are `doc_start_and_end`, `document`,
`documents`, `DocEnd`, `DocStart`, `Document` -- case-insensitive, exact case
first.

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`. That baseline had
to be regenerated: it predated the `use crate::datetime::now_secs;` line in
`editor.rs`, so the dump's *document* had shifted by a line and the diff was in
the text, not the renderer. Two consecutive dumps of the current build are
byte-identical, which is the property the baseline is for.

## Thirtieth pass: the workspace

Notepad++ has four panels that show a tree of files: Folder as Workspace, and
Project Panel 1, 2, and 3. They differ in exactly one thing -- where the roots
come from. Folder as Workspace takes the folders the user opened; a project
panel takes the folders the user added to that project. Everything below the
roots is identical: the same scan, the same sort, the same filter, the same
rows, the same clicks.

So there is one `Tree` and one `FolderPanel`, and the panel is told which roots
to show. That is the whole design. The alternative -- a `WorkspaceTree` and a
`ProjectTree` behind a shared trait -- would have duplicated the scan, the
filter, and the row layout, and the two would have drifted the first time one
of them gained a feature.

### Why the tree is scanned once and flattened

`Tree::scan` walks the roots eagerly, up to `MAX_DEPTH` levels and `MAX_ENTRIES`
entries, and stores a flat `Vec<Entry>` where each entry carries its `depth`.
Expanding a directory does not touch the filesystem; it flips a flag and
`rows()` re-filters the flat list.

Three reasons. A tree that walks on expand makes every click a potential stall
on a network mount, and the stall lands in the middle of a paint. A flat list
keeps scroll position stable across expand and collapse, because the rows above
the toggled directory do not move. And a flat list is trivially testable:
`rows()` is a pure function of the entries and the expanded set, so the tests
assert on names and depths rather than on a widget.

The cost is that a directory created after the scan does not appear until
`rescan`. That is what Refresh Workspace is for, and it is what Notepad++
does.

### Why rows are `Entry` values with a depth

A nested `Node { children: Vec<Node> }` would be the obvious shape, and it is
the wrong one here. Rendering needs a linear sequence of visible rows; a nested
structure forces a recursive walk into a mutable output vector on every frame.
Filtering needs to keep a match's ancestors, which in a nested structure means
rebuilding the tree; in a flat list it means keeping the entries whose depth is
less than the match's and whose index is before it. And the panel needs to
answer "what is the path at row 12" in constant time, which a flat list gives
directly.

The depth field is what makes the flat list renderable: indentation is
`depth * INDENT`, and the disclosure marker is `-` for an expanded directory,
`+` for a collapsed one, and two spaces for a file.

### Why roots start expanded and deeper levels closed

A panel that opens fully collapsed shows one row per root and hides the thing
the user opened the panel to see. A panel that opens fully expanded on a large
tree is a wall of text and a slow first paint. Opening the roots and stopping
there is the compromise Notepad++ makes, and it is the right one: the first
screen shows the top-level layout of the project, which is what the user is
orienting themselves by.

`expand_all` and `collapse_all` exist for the user who wants the other
behaviour, and they are one flag flip each because the entries are already
scanned.

### Why `SKIP_DIRS` is shared with find-in-files

`findinfiles` already had a list of directories not worth descending into --
`.git`, `target`, `node_modules`, and the rest. The workspace tree wants the
same list for the same reason: nobody wants `target/debug/incremental` in a
file browser. Sharing the constant means the two features cannot disagree about
what counts as noise, and a directory added to the list fixes both at once.

The list is deliberately short. It is a default, not a policy: a user who
genuinely wants to browse `target` can still open it as a root directly,
because the skip applies to descent, not to the root itself.

### Why the filter keeps ancestors and roots

A filter that showed only matching rows would produce a list of bare filenames
with no indication of where they live, which is useless in a tree with two
roots and three `mod.rs` files. Keeping the ancestors keeps the path legible.
Keeping the roots keeps the panel from looking like it lost its structure.

A directory whose *name* matches shows its whole subtree. The alternative --
showing the directory but not its contents -- makes a name match on a directory
useless, since the user's intent when typing `src` is to see what is in `src`.
A file match shows only itself.

Filtering implies expansion, because a match hidden inside a collapsed
directory is indistinguishable from no match at all.

### Why `toggle` returns a bool

`Tree::toggle` returns whether the path is a directory. The panel needs to know:
a click on a directory toggles it, a click on a file opens it. Returning the
answer from the model keeps the panel from having to ask the filesystem a
second time, and it keeps the "is this a directory" question in one place.

### Why `Projects` is a name plus a list of paths

Notepad++'s `projects.xml` nests folders inside folders inside projects. That
nesting is presentation: the filesystem already has a hierarchy, and a project
that mirrors it is storing the same information twice. Storing a project as a
name and a list of root paths means the tree under each root is always the truth
on disk, and adding a subfolder to a project is adding a root.

The cost is that a project cannot show a curated subset of a directory -- you
get the whole subtree or nothing. That is a real limitation, and it is the one
Notepad++ users complain about least.

### Why the file is line-oriented

`projects.conf` is:

```
# npp-rs projects
active 0
project npp-rs
root /opt/DEV/nn/crates/nn-core
root /opt/DEV/nn/crates/nn-app
```

There is no XML writer in the crate, and adding one to persist three project
names and a handful of paths would be more code than the feature. A
line-oriented format is readable, diffable, hand-editable, and parses in twenty
lines. `parse` is total: unknown keywords are skipped, a `root` before any
`project` is dropped, a fourth project is ignored, and `active` is clamped on
read so a hand-edited file with `active 9` does not panic. That is the same
forgiving reading the settings file uses, so a file written by a newer build
still loads.

`MAX_PROJECTS` is three because Notepad++ has three project panels. The limit
lives in the model rather than the UI so that `add` can refuse and the panel
does not have to.

### Why `all_roots` deduplicates

Two projects can legitimately share a root -- a workspace and a library, say.
Find in Projects searches the active project's roots, and a duplicated root
would report every hit twice. `all_roots` deduplicates by path, and `run_roots`
deduplicates again by canonical path per file, because two *different* roots can
still overlap: `/opt/DEV/nn` and `/opt/DEV/nn/crates` both contain
`crates/nn-core/src/lib.rs`.

The canonicalisation is per file rather than per root because canonicalising
roots would not catch the overlap -- the roots are genuinely different paths. It
costs one `canonicalize` per candidate file, which is a syscall, and it is worth
it: a results panel with every hit listed twice is worse than a slightly slower
search.

`max_hits` is shared across roots rather than applied per root, so a search that
hits the cap stops rather than returning three times the cap.

A root that is a file is scanned directly. That is not a special case for its
own sake: a user who adds a single file to a project means that file, and
walking its parent would search the whole directory.

### Why one docked column serves both panels

`State.workspace: Option<usize>` is `None` when no panel is open, `Some(0)` for
Folder as Workspace, and `Some(n)` for project panel `n`. There is one
`FolderPanel` and one column, and opening a different panel replaces the
contents.

Two columns would halve the editor width for a feature that is used one panel
at a time, and Notepad++ itself shows one at a time. The `Option<usize>` also
makes "which panel is showing" a single value that cannot disagree with itself,
which a pair of booleans would not.

The panel is left-docked and the function list is right-docked, matching
Notepad++. The left edge is where a file tree belongs: it is the first thing the
eye hits, and it is where every other editor puts it.

### Why the filter is a header line

The panel has two header lines: the title with the file count, and `Filter: ...`
with a caret when focused. There is no separate text field widget, because there
is no text field widget in this codebase -- every text input is a string plus a
caret index plus a focus flag, and the panel already owns all three.

Escape clears the filter before it gives up the keyboard. A user who typed a
filter and wants out expects one Escape to undo the typing and a second to leave
the panel; the other order loses the filter on the way out.

### Why the panel is hidden in distraction-free mode

Distraction-free mode exists to show the document and nothing else. A file tree
is a distraction by definition. `chrome.rs` gained `shows_folder()` next to the
other `shows_*` predicates so the rule lives with its siblings rather than in
the renderer.

### Why `folder_width` is added to the inset

The renderer computes `left = inset + folder_width` and gives the panes
`width - 2 * inset - folder_width`. Subtracting the panel width from each pane
instead would double-count it in split view and leave a gap between the panes
and the divider. Adding it to the left inset means the panel occupies the
inset's own margin, and the panes and divider see a viewport that is simply
narrower.

### The `folder` name collision in bench.rs

`bench.rs` already had a local `folder: BraceFolder` for the brace-folding dump.
The workspace panel is therefore `workspace` in that file. The two are unrelated
and the rename is local to the binary.

### Verification

963 tests became 1022: 512 in `nn-app`, 510 in `nn-core`. Clippy is at zero
warnings across `--all-targets`, and the release build is clean.

`nn-core::workspace` is new with 18 tests: scanning a scratch tree, directories
before files, case-insensitive ordering, roots expanded and deeper levels
closed, `toggle` reporting directory versus file, `expand_all` and
`collapse_all`, `rows()` honouring the expanded set, the filter keeping
ancestors and roots, a directory name match showing its subtree, filtering
implying expansion, `set_roots` replacing the tree, `is_truncated` at the entry
cap, and `file_count` counting files rather than directories.

`nn-core::project` is new with 20 tests: a new project having no roots, a
duplicate root refused, `remove_root`, `is_empty`, `set_active` clamping,
`add` refusing a fourth project, `remove` and `rename`, `all_roots`
deduplicating across projects, `active_roots`, and a round trip through
`render` and `parse` including unknown keywords, an orphan `root`, a fourth
project, and an out-of-range `active`.

`nn-core::findinfiles` went from 19 tests to 24: `run_roots` over two roots
into one list, overlapping roots not double-reporting, a file root scanned
directly, `max_hits` shared across roots, and `run` still delegating.

`nn-app::folder` is new with 16 tests: the two header lines, the title with its
file count, rows rendering indent and marker, `row_at` and `path_at` agreeing,
`filter_at` hitting only the filter line, selection moving and clamping,
`type_filter` and `backspace_filter` and `clear_filter`, `focus_filter` and
`blur_filter`, `scroll_by` clamping, and `width` staying inside its bounds.

Checked offscreen at 900x500 against `crates/nn-app/src/main.rs`. With `folder`
the panel reads `Folder as Workspace (35 files)` over `Filter:`, then `- src`,
`+ bin`, and the files of `src` down to `minimap.rs`. With `project` it reads
`Project 1 (36 files)` with two roots, `- src` and `- bin`, and `bin` showing
`bench.rs`. With `folder_filter=ma` the filter line reads `Filter: ma_` with the
caret, and the rows are `- src`, `keymap.rs`, `main.rs`, `minimap.rs`. With
`folder split` the panel takes the left column and the two panes divide the
rest, divider at about 450. With `folder distraction` the panel is correctly
absent. With `find_projects=fn` the run prints `find in projects: 1426 hits in
2 roots`, the results panel reads `1426 results in 34 files`, and the first rows
come from `bench.rs`, which lives in the *second* root -- proof that both roots
were searched into one list.

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`, zero differing
pixels.

## Thirty-first pass: exporting shortcuts

The import has read `shortcuts.xml` since the Notepad++ configuration work.
This pass writes one. The two directions share a single table, and that sharing
is the whole design.

### Why one table instead of two `match` arms

The import had `command_of(id) -> Option<Action>` as a `match` over the
`menuCmdID` numbers. The export needs the inverse, `command_id(action)`. Writing
that as a second `match` would have worked until the first time a command was
added to one of them, and the symptom -- a shortcut that survives an export but
not the import back -- is invisible until someone tries it.

So the numbers live in one `const COMMANDS: &[(u32, Action)]` and both
directions are a linear scan over it. Two tests hold the property: every number
reads back as the command it names, and no two numbers name the same command.
The table is twenty-one rows, so the scan costs nothing.

### Why `KeyName::to_vk` is the exact inverse of `from_vk`

Notepad++ names keys by Windows virtual-key code. A chord that cannot be written
as a number cannot be exported, and the honest answer for such a key is `None`
rather than a plausible-looking number.

The test that matters walks every code from `0x00` to `0xff`, asks `from_vk` for
a key, and asserts `to_vk` gives the code back. That is the property the export
rests on: if the two ever disagree, a round trip through Notepad++ moves a
binding, and the failure is silent.

`=` is the case that makes `None` necessary. On a US layout it shares
`VK_OEM_PLUS` with `+`, and this editor's `Ctrl++` binding holds `+`. Writing
`0xbb` for `=` would export a different key than the one bound, so `=` has no
code and is skipped.

### Why the export is partial, and why it says so

Seventy-one bindings ship by default. Twenty-one of them have a Notepad++
command number. The other fifty are not an oversight: they are commands Notepad++
does not have as menu commands -- the split view, the character panel, the change
history, distraction-free mode, the tab layout, the folder and project panels,
find in projects, the multi-caret pair, the case conversions, the line
operations -- or commands it has only as Scintilla messages, which `shortcuts.xml`
names with a `ScintID` rather than a `menuCmdID` and which this editor has no
Scintilla to route to.

Writing a number for any of them would produce a file that looks complete and
binds the wrong thing when it came back. So they are counted, and the status bar
says `Exported 21 shortcuts (50 skipped)` rather than `Exported 71 shortcuts`.

### Why the rows are sorted by command number

`Keymap::bindings` is in insertion order, which is the order the defaults were
written and then whatever order the user rebound things in. Exporting that order
would make two exports of the same keymap differ, and would make a diff between
two exports useless for spotting what actually changed.

Sorting by `menuCmdID` makes the file a function of the keymap alone. Two
exports of the same keymap are byte-identical, which is a test.

### Why the empty sections are written

Notepad++ reads `<Macros>`, `<UserDefinedCommands>` and `<PluginCommands>`
unconditionally. A file without them looks truncated to it. Writing them empty
costs three lines and means the export is structurally the file Notepad++
expects rather than a fragment that happens to parse.

### Why the default path is the config directory

`paths::config_file("shortcuts.xml")` puts the export beside `settings.conf` and
`projects.conf`, which is where a user looking for it would look. The prompt is
prefilled with it and the user can type anything else, so the default is a
convenience rather than a policy.

### Verification

1022 tests became 1033: 523 in `nn-app`, 510 in `nn-core`. Clippy is at zero
warnings across `--all-targets`, and the release build is clean.

`nn-app::keymap` gained two tests: every virtual-key code `from_vk` accepts
comes back out of `to_vk` unchanged, and a key with no code -- `=`, `!`, `F0`,
`F25` -- has no export.

`nn-app::nppimport` gained eight: the command table reading back in both
directions, an export naming the commands Notepad++ knows, an export counting
the ones it does not, an export round-tripping through `read_shortcuts` as the
same bindings, the rows sorted by command number, two exports being identical,
the summary wording, and `write_shortcuts` creating a nested directory and
writing the text it returned.

`nn-app::prompt` gained one: every prompt kind has a label ending in a colon,
so a kind added without one cannot open a blank bar.

Checked offscreen at 900x500 against `crates/nn-core/src/editor.rs`. The
Settings menu now reads `Preferences...`, `Style Configurator...`, a separator,
`Import Notepad++ Config...`, `Export Shortcuts...`. The `export_shortcuts`
bench flag prints the file the menu item would write: `Exported 21 shortcuts
(50 skipped)`, then a `shortcuts.xml` whose rows run from `menuCmdID="41001"`
(`Ctrl+N`, `Key="78"`) through `menuCmdID="49001"` (`Ctrl+,`, `Key="188"`), with
`41005` carrying `Ctrl="yes" Shift="no" Key="83"` for Save and `41007` carrying
`Ctrl="yes" Shift="yes" Key="83"` for Save As.

A dump with no flags is pixel-identical to `/tmp/plain3.ppm`.

## Thirty-second pass: user-defined languages

Notepad++ ships a fixed set of built-in lexers and then hands the user a
dialog that builds more. The second half is the part that matters: a language
nobody wrote a lexer for -- a log format, a config dialect, a DSL -- can be
described in XML and highlighted without touching the binary. This pass adds
that, reading the same `userDefineLang.xml` Notepad++ writes.

### A data-driven lexer, not a match per language

The obvious alternative was to keep adding `match` arms to `CLikeLexer` for
each language we care about. That does not scale to "the user invents a
language at runtime", which is the whole point. So the description is data --
`Udl` -- and one lexer, `UdlLexer`, interprets it. Adding a language is then a
file, not a build.

`Udl` holds what the XML actually says: name, extensions, case sensitivity,
line-comment markers, block-comment pairs, delimiter pairs, operators, eight
keyword groups, and 21 colour slots. `UdlSet` holds the parsed languages and
answers the two questions the app asks -- "which language claims this
extension" and "which language is called this".

### One XML scanner, two users

`nppimport` already needed to read Notepad++ XML for shortcuts and settings,
and it had grown its own ad-hoc scanning. Rather than write a second one for
`userDefineLang.xml`, the scanner moved into `nn-core::xml`: `Element`,
`elements`, `attribute`, `unescape`, `escape`. Both callers now use it.

It is deliberately a scanner and not a parser. It never fails -- a truncated
file yields the elements it did find -- and it does not track nesting of
same-named elements, because nothing Notepad++ writes nests `<UserLang>` or
`<Keywords>` inside itself. That limitation is documented on the module rather
than papered over: a future format that does nest will need a real parser, and
the doc says so.

### Why `LexState::Udl(u8)`

A delimiter construct needs to remember *which* construct is open, because two
delimiters can share an opening character and differ in their close. Reusing
`LexState::String(char)` would have meant the close character alone identifies
the construct, which is false the moment a language has both `"` and `"""`.

So `LexState` gained `Udl(u8)`, an index into the lexer's construct list. The
byte keeps 0..3 for the built-in states (`Normal`, `BlockComment`, `String`,
`RawString`) so `to_u8`/`from_u8` stay a total, stable mapping; UDL states
start at 4. `CLikeLexer` treats `Udl(_)` as an ordinary state -- it has no
opinion about a language it was not asked to lex.

### `Construct` holds `Vec<char>`

`Construct { open: Vec<char>, close: Vec<char>, style }`. Characters, not
`String`, because the lexer compares one character at a time against the open
and close sequences; a `String` would force a `chars()` iterator per
comparison. The vectors are built once at parse time and never mutated.

`constructs` keeps the order the file declared them in, and `order` is a
separate index list sorted longest-open-first. Two lists rather than one
because the style of a construct is looked up by its declaration index, while
matching wants longest-first: `"""` must win over `"`. Sorting `constructs`
itself would have made the indices unstable.

### Ordering rules inside the lexer

Block comments are tried first, then delimiters in `order`, then line
comments. Delimiters before line comments because a language whose line
comment is `#` and whose delimiter is `"` must not treat `"# not a comment"`
as a comment -- the delimiter opens first and wins.

An empty close means "to end of line". That is how Notepad++ expresses a
single-character delimiter like `'` in a language where it does not pair, and
it is cheaper than a second flag.

A backslash escapes the close character. Notepad++ does this for every
delimiter it defines, so a `"` inside a `"`-delimited string does not end it.

`UdlLexer` owns a clone of the `Udl`. It has to: it is handed out as
`Box<dyn Lexer>`, which has no lifetime parameter, so it cannot borrow the set
it came from. The clone is one `Udl` per tab, not per line.

### Style indices 16..31

The eight keyword groups and eight delimiter pairs got their own style
indices rather than folding into `KEYWORD` and `STRING`. Folding would have
made every UDL keyword the same colour as the built-in lexer's keywords, which
is wrong when the UDL says group 1 is red and group 2 is teal -- and the whole
reason a user writes a UDL is to get the colours they want. `style::COUNT`
went 16 to 32 to make room.

`PlainLexer` is a real lexer, not a special case in the view. A tab with no
language still needs *something* to answer `next`, and a lexer that returns
`DEFAULT`/`Normal` for everything is simpler than a branch at every call site.

### `Palette`: colour per document, not per theme

A `Theme` is global; a UDL's colours are per document, and the two panes can
show two different languages. So `EditorView` gained a `palette: Palette`,
built either from the theme (`Palette::new`) or from a theme plus a `Udl`
(`Palette::with_udl`). `set_theme` resets it, because a palette derived from
the old theme is meaningless under the new one.

`with_udl` honours a UDL colour unless it is within `MIN_CONTRAST = 40`
luminance of the background. That number is a judgement call and is documented
as one. The reasoning: a UDL is written against Notepad++'s light default, so
applying it verbatim in dark mode gives black text on a near-black background
-- unreadable. Refusing every colour makes the import pointless. Refusing only
the ones that vanish keeps the intent and drops the mistakes.

The two comment slots share one style. Notepad++ distinguishes "line comment"
from "line comment 2" only so a language can have two markers; it does not
colour them differently, and neither do we.

### `Language` as a three-variant enum

`Language { Plain, BuiltIn, Udl(usize) }` rather than `Option<usize>`. Three
states, not two: "no highlighting", "the built-in C-like lexer", and "the
nth user language". `Option<usize>` would have needed a sentinel for the
built-in case, and the sentinel would have been a lie about the index space.

The choice is per tab, not per extension. Notepad++ lets a document override
its extension-derived language, and a user who renames a file should not lose
the highlighting they picked. `Tab::new` picks by extension; `set_language`
overrides.

`set_language` clones the `UdlSet` before touching the tab. The borrow checker
forces it -- `self.langs` is borrowed while `tab_mut` is held -- and the clone
is a handful of small vectors, paid once per menu click.

### Import merges, and re-lexes

`import_notepad` merges the imported `UdlSet` into the live one rather than
replacing it, because a user importing shortcuts should not silently lose the
languages they defined by hand. After merging it saves, refreshes the menu,
and re-lexes every open tab: a tab already showing "Logfile" must pick up the
new definition of "Logfile", not keep the old colours until reopened.

### Menu placement

The Language menu sits between Encoding and Settings, matching Notepad++'s
order, which pushed `RUN_MENU` from 8 to 9. The separator before the user
languages only appears when there are some -- a trailing rule with nothing
under it reads as a rendering bug.

`Action::Language(_)` is excluded from `BINDABLE` for the same reason as
`OpenRecent`: the payload is an index into a list that changes at runtime, so
a saved binding would point at whatever language happened to be nth later.

### Deliberately not implemented

Number formats, folding keywords, and nesting levels are named in the UDL
schema and not honoured. Each is a real feature with real semantics --
number formats are a small grammar, folding keywords interact with the fold
engine, nesting levels change the state machine -- and approximating them
would produce highlighting that is subtly wrong in ways the user cannot
diagnose. Named, not approximated.

### Verification

1033 tests became 1103: 536 in `nn-app`, 567 in `nn-core`. Clippy is at zero
warnings across `--all-targets`, and the release build is clean.

`nn-core::xml` contributes 14 tests: elements found by name, attributes read
and missing, entities unescaped and escaped, a truncated document yielding
what it has, and same-name elements not nesting.

`nn-core::udl` contributes 41: hex colours parsed and rejected, luminance
ordering, every slot name, keyword groups and delimiter pairs read from the
numbered slots, a language claiming its extensions, the lexer's block
comments, its delimiters, longest-open-first matching, delimiters beating line
comments, an empty close running to end of line, a backslash escaping the
close, keyword lookup being case-sensitive or not, and `UdlSet` add/get/
by-extension/by-name/render round-tripping.

`nn-app::palette` contributes 13: the theme's colours carried through, a UDL
colour honoured, a UDL colour too close to the background refused, the
keyword and delimiter slots defaulting, and `from_rgb8` preserving channels.

`nn-core::lexer` gained two: `PlainLexer` returning `DEFAULT`/`Normal`
forever, and `LexState::Udl` round-tripping through `to_u8`/`from_u8`.

`nn-app::nppimport` gained three: a `userDefineLang.xml` read into the set,
the summary counting languages, and a missing file leaving the set empty.

`nn-app::menu` gained five: the Language menu's position, its two built-in
rows, the separator appearing only when languages follow, a language row
carrying its index, and `Language(_)` staying out of `BINDABLE`.

Checked offscreen at 900x500. `/tmp/sample.log` with `/tmp/userDefineLang.xml`
shows green `#` comments, red `ERROR`/`WARN`/`FATAL`, teal `started`/
`stopped`/`reloaded`, yellow `http`/`tcp`, orange `"config.toml"` and
`'timeout=30'`, light-green numbers, and a two-line `/* */` block in green;
the minimap picks the colours up. The Language menu reads `Plain Text`,
`C / C++ / Rust (built in)`, a rule, then `Logfile`. Without a UDL the same
menu reads only the two built-in rows and no rule.

A dump with no flags differs from the previous baseline only in the menu bar,
`(410, 8, 741, 23)` -- the Language title shifting the menus to its right.

## Thirty-third pass: the plugin host

Notepad++'s plugin system is the reason people stay on it: a DLL dropped into
`plugins/` adds a menu, and the menu runs code that can read and rewrite the
buffer. This pass adds the same shape -- a directory of plugins, a menu built
from what they declare, and a command that edits the document -- with one
difference forced by the platform: the plugin is a WebAssembly module, not a
shared object.

### Why an interpreter, written here

The alternative was a crate. `wasmtime` is hundreds of megabytes of JIT and
does not build offline in this tree; `wasmi` would have been a dependency
whose sandbox limits are its own, not ours. The requirement is narrow -- a
plugin is a few hundred instructions of arithmetic and memory traffic, called
once per menu click -- so an interpreter is the right size of tool, and
writing it means the limits (fuel, stack, memory pages, table size) are
constants in this file that a test can assert on.

The cost is real and worth stating: this is not a conformant WebAssembly
engine. It implements what a Rust `cdylib` compiled for
`wasm32-unknown-unknown` emits, and rejects the rest by name. That is a
deliberate trade -- a plugin that needs `memory.init` gets a clear error
instead of a subtly wrong result.

### Three files, three jobs

`wasm/mod.rs` is the module model: types, imports, exports, globals, tables,
elements, data segments, and the `Error` enum. `wasm/binary.rs` is the
decoder -- LEB128 with range checks, every section Notepad++-style plugins
need, and a `Reader` that never reads past its slice. `wasm/exec.rs` is the
machine: opcode decode, the operand stack, the call stack, and the numeric
operators.

The split is by *what can be tested without the others*. The decoder is
tested against hand-assembled byte strings; the machine is tested against
`Op` slices built in Rust, never through the decoder. A failure then points at
one file.

### No validator

A conformant engine validates a module before running it: every instruction's
operand types are checked against a stack-type simulation. This one does not.
`Value` carries its type, and every operator pops what it expects and returns
`Error::BadType` when the tag is wrong. A malformed module therefore traps on
the instruction that is wrong rather than at load.

The reason is that a validator is a second implementation of the type rules,
and two implementations disagree. With one, the machine's behaviour *is* the
specification, and the tests that exercise it are the tests that matter. The
cost is that a bad module fails later than it could -- acceptable when the
module was produced by a compiler that already type-checked it.

### The jump table is built at load

`Block`, `Loop`, `If` and `Else` need to know where their matching `End` is.
The obvious implementation scans forward on entry, which makes a loop that
runs a million times rescan its body a million times -- O(n^2) in the body
length. So `jumps(code)` walks the code once when the function is decoded and
records `Jump { end, else_at }` per block instruction; `Br` and `End` then
index it.

`br_table` is the exception: its target list is variable-length and sits
inline in the code, so it is walked on demand from the recorded offset rather
than decoded into a `Vec` per table. A `br_table` in a hot loop pays a short
scan; every other branch pays nothing.

### `Store` bundles the mutable state

`Instance` holds the module, the host, and a `Store { memory, globals, table }`.
The three are bundled because every instruction that touches one of them is
inside `exec`, which already takes `&mut Store`; passing three `&mut` would
have meant three parameters on every helper and a borrow-checker fight at each
call site. They are also the three things a plugin can observe across calls,
so grouping them names that fact.

### The ABI

`wasm/abi.rs` fixes the contract: the host module is `npp`, with seven
functions -- `log`, `text_len`, `text_read`, `text_replace`, `text_insert`,
`caret`, `set_caret`. A plugin exports `npp_name`, `npp_command_count`,
`npp_command_name` and `npp_run`.

Strings are NUL-terminated bytes in the plugin's own memory, not
length-prefixed. A Rust plugin's static strings are already NUL-terminated in
`.rodata` -- `b"Uppercase\0"` is what the compiler emits for a `&str` literal
passed to a C-style API -- so the ABI reads what is already there instead of
asking the plugin to build a length-prefixed copy at startup. `MAX_STRING`
bounds the read so a plugin that forgets the terminator gets an error rather
than a scan to the end of memory.

`Host` is one method, `call(name, args, memory)`, dispatched on the name
string, rather than a trait method per function. Seven methods would have
meant seven `impl` blocks in every host, including the `NullHost` used by the
core tests, and the dispatch is a `match` on a `&str` either way.

### Copy-on-write, not a live document

`nn-app::plugin::Bridge` holds a copy of the text, the caret, a read-only
flag, and a log. The plugin edits the copy; when `run` returns, the app
applies the result as **one** `replace_ranges` edit.

Two consequences, both wanted. A plugin that traps halfway through leaves the
document untouched -- the copy is dropped. And the whole plugin run is a
single undo step, which is what a user expects from "run a command", not
fifty small edits they have to unwind one at a time.

`floor_boundary` exists because the plugin speaks byte offsets and the
document is UTF-8: a plugin that computes an offset by counting bytes can land
inside a multi-byte character, and slicing there would panic. The bridge
floors to the nearest character boundary instead.

A read-only document returns `1` from `text_replace` rather than trapping. A
trap would abort the whole command; a return code lets a well-written plugin
notice and stop, and a badly written one still cannot corrupt anything.

### The menu

`Action::Plugin(usize, usize)` carries the plugin and command index;
`Action::PluginNone` is the placeholder row shown when no plugin is installed,
so the menu is never empty and the user learns the feature exists. Both are
excluded from `from_name` and `BINDABLE`: a plugin command has no stable name
to bind, and the placeholder is not an action.

The Plugins menu sits between Run and Help, which is where Notepad++ puts it
and where a menu that grows with installed software belongs -- last before the
one menu that never changes.

`dropdown_left` clamps a dropdown to the window. A plugin label is
`Plugin - Command`, the longest row in the bar, and the rightmost dropdown was
being cut off mid-label. The clamp pulls the panel back; `dropdown_areas`
takes the same width so the text follows the panel rather than hanging off it.

### What is still rejected

By name, with `Error::Unsupported`: `memory.init`, `data.drop`, `table.init`,
`table.copy`, `table.grow`, `table.size`, `table.fill`, blocks with a type
index (multi-value), more than one memory or table, and `externref`. Globals,
tables, `call_indirect`, element segments, bulk `memory.copy`/`memory.fill`,
`select` with types, `ref.null`/`ref.is_null`/`ref.func` and the eight
saturating conversions were all added after the first cut, because a
Rust-compiled plugin needs every one of them -- the first cut rejected the
global, table and element sections outright and could not load the demo.

### Verification

1217 tests, up from 1103. `nn-core::wasm::mod` contributes 5: value types and
their accessors, `Display`, and the error messages. `wasm::binary` contributes
25: LEB128 at its boundaries, a truncated section, a bad magic, every section
the demo uses, all eight element-segment flag forms, and the three data-segment
forms. `wasm::exec` contributes 53: each opcode family, the fuel limit, the
depth limit, the stack limit, `br_table`, `call_indirect` through a table,
every saturating conversion, and the truncations at their edges.
`wasm::abi` contributes 12: a string read and its missing terminator, the
seven host names, and `Plugin::load`/`describe` over a hand-built module.
`nn-app::plugin` contributes 13: the bridge's text and caret, a replace, an
insert, a read-only refusal, a log line, and `Plugins::load_dir` over a
directory with one good and one broken module. `nn-app::menu` gained six: the
Plugins menu's position, the placeholder row, a plugin row carrying both
indices, `Plugin(_)` staying out of `BINDABLE`, a dropdown near the right edge
being pulled back inside the window, and one with room staying under its
title.

Clippy is clean at `--all-targets`. The release build is clean.

Checked against a real plugin, not a hand-built module: a `cdylib` crate at
`/tmp/npp-plugin-demo` built for `wasm32-unknown-unknown`, exporting three
commands -- Uppercase, Strip blank lines, Count lines. `bench ... plugins=<dir>
plugin_menu` prints `plugin: npp_plugin_demo.wasm: Logfile Tools (3 commands:
Uppercase, Strip blank lines, Count lines)`. On a 32-byte, four-line document,
`plugin_run=0:2` logs `4 lines, 32 bytes`; `plugin_run=0:0` returns 32 chars
and the dump reads `HELLO WORLD`, `SECOND LINE`, blank, `FOURTH`;
`plugin_run=0:1` returns 31 chars and the dump shows the blank line gone. The
menu dump reads three `Logfile Tools - ...` rows, all inside the window.

A dump with no flags differs from the previous baseline only in the menu bar,
`(706, 8, 824, 23)` -- the Plugins title shifting Help to its right.

## Thirty-fourth pass: one instance, and the command line

Notepad++ is single-instance by default: a second launch hands its file to the
running window and exits. It also takes a file and a handful of switches. This
pass adds both, plus the two things a Linux editor is expected to do that
Notepad++ never had to -- `--wait`, so it can be `$EDITOR` for git, and reading
a document from a pipe.

### Why the bus, and not a lock file

The obvious single-instance mechanism is a lock file in the config directory
holding a pid. It is also the wrong one: a pid is reused, a crashed editor
leaves the file behind, and there is no way to *talk* to the process that owns
it -- the second launch needs to hand over a path, not just discover that
somebody is home.

A name on the session bus answers all three. The bus owns the lifetime: when
the process dies the name goes with it, so a crash cannot leave a stale lock.
And the name is an address, so the second launch can call a method on it.

`zbus` is the client. It is a large dependency for what is used here, but the
alternative is speaking the DBus wire protocol by hand, and the parts that
matter -- `RequestName` with `DoNotQueue`, `NameHasOwner`, match rules for
signals -- are exactly the parts that are easy to get subtly wrong.

### `DoNotQueue` is the whole trick

`RequestName` without flags *succeeds* when the name is taken: the bus puts the
caller in a queue and reports `InQueue`. zbus's connection builder treats that
as success, so two editors would both believe they were the server and the
second would sit waiting for a name it would never get.

So the name is requested explicitly, with `DoNotQueue`, and only `PrimaryOwner`
and `AlreadyOwner` count as owning it. Everything else -- `InQueue`, `Exists`,
an error -- means "somebody else is the editor", and the caller's next step is
to hand off.

### The bus is optional

Every entry point returns `Option` or `Result`, and "no bus" is not an error.
A bare TTY, a container, a CI run: the editor starts standalone. That is also
what happens when the handoff call itself fails, which is why the fall-through
prints one line and carries on rather than refusing to start.

### `--wait` needs two answers, not one

The protocol is: register a match rule for `Closed`, send `Open`, then block
until the path arrives. The order matters -- a document that closes the instant
it opens would otherwise emit its signal before anyone was listening.

But a signal stream does not end when its sender dies. The bus simply stops
delivering, and a waiter blocked on the stream waits forever. That was found by
killing the editor while a `--wait` caller was blocked: the caller stayed
blocked. So `Waiter` also polls `NameHasOwner` every 200 ms and returns when
the name has no owner. A `--wait` caller now returns when the document closes
*or* when the editor goes away, which are the only two things that can happen.

The polling needs a timeout, and `MessageIterator::next` has none, so a thread
owns the iterator and forwards what it hears into a channel. The waiter then
has something it can time out on. One thread per waiting process, which is
nothing: the process is a shell script's `$EDITOR` and does no other work.

### The command line is a library module

`main.rs` has no test module, by policy: anything with a decision in it belongs
where a test can reach it. The parser is `nn-app::cli`, and the decisions it
owns are the interesting ones:

- `--` ends the switches, so a file called `-w` can be opened.
- A lone `-` is standard input, not a switch, because that is what every other
  Unix tool means by it.
- Short switches cluster (`-nw`), and one that takes a value ends the cluster
  and takes the rest of the word (`-l20`), so both spellings work.
- A switch after a file is still a switch, because the shell puts the file
  first as often as not.
- A position must be a positive number. `--line 0` is rejected rather than
  clamped: a caller that passes it has made a mistake, and silently opening at
  line 1 hides it.

`Args::caret` is the one place the one-based command line becomes the
zero-based editor. Two callers need it -- the first launch and a handed-off
request -- and two conversions would eventually disagree.

### Absolute paths on the bus

The second process resolves `notes.txt` against *its* working directory, and
the first process has a different one. A relative path sent over the bus would
be opened against the wrong directory, so `absolute` joins the current
directory before the path leaves the process. `canonicalize` is not used: it
fails on a file that does not exist yet, and "open this new file" is a normal
request.

### A file already open is focused, not opened twice

`find_open` answers "which tab already shows this path", and `open_request`
focuses it instead of pushing a second tab. Two tabs on one file is not what
`npp-rs notes.txt` means, and it is actively harmful: the two copies diverge
and the last save wins.

### The bug this pass found

Verifying the handoff turned up a real bug that had nothing to do with the bus.
The second launch opened `b.txt`, the title bar and the tab strip both said
`b.txt`, and the editor showed `a.txt`'s text.

`EditorView` caches shaped lines and invalidated them on
`doc.revision() != cache.revision`. Two files loaded from disk are both at
revision 0, so switching between them kept the previous tab's shaped lines.
Every tab switch between two unedited documents showed the wrong text.

The fix is a process-unique `Document::id`, assigned from an atomic counter in
`from_text`, and a cache keyed on `(id, revision)`: the id changes when the tab
does, the revision when the text does. A clone keeps the id, which is correct
-- a clone is the same buffer, and a diverging edit bumps the revision, which
is what invalidates.

This is the kind of bug that only an end-to-end check finds. The unit tests all
passed; the bench dumps all matched; the editor was simply showing the wrong
file.

### Verification

1252 tests, up from 1217. `nn-app::cli` contributes 21: every switch, the
clustering, the inline and separate values, the terminator, a lone dash, a
rejected position, the usage text covering every switch, and the four
one-based-to-zero-based conversions. `nn-app::single` contributes 11: a
relative path made absolute, the handoff rule including the stdin case, a
request's position and wait flag, `find_open`, and four that need a real bus --
the name being exclusive, a request arriving at the server, a close signal
reaching a waiter, and a waiter returning when the editor goes away.
`nn-core::document` gained three: two documents have different ids, an id
survives an edit, and a clone keeps it.

Clippy is clean at `--all-targets`. The release build is clean.

Checked against the real binary on the X11 display, not the bench:

- `--help` prints the usage text, `--version` prints `npp-rs 0.1.0`, and an
  unknown switch prints the error plus the usage and exits 2.
- `nn-app /tmp/a.txt` takes `org.npp_rs.Editor` (`NameHasOwner` returns true).
  A second `nn-app /tmp/b.txt` exits 0 immediately, and the running window
  gains a `b.txt` tab, focuses it, and shows `one`/`two`/`three` -- the fix for
  the cache bug, read off a screenshot.
- `nn-app --wait /tmp/c.txt` blocks while the editor is alive and returns with
  exit 0 when the editor is killed.
- `printf 'from stdin\nsecond\n' | nn-app --new-instance -` opens an untitled
  tab reading `from stdin`/`second`, length 18, three lines.
- `nn-app --new-instance --line 2 --column 3 /tmp/a.txt` puts the caret on
  line 2, column 3, and the status bar reads `Ln: 2, Col: 3`.

A dump with no flags is pixel-identical to the previous baseline: this pass
touches no rendering.

## Thirty-fifth pass: speaking other languages

Notepad++ ships around ninety localizations, and they are not gettext catalogues
or JSON bundles: each is an XML file in `localization/` whose menu items are
keyed by the numeric command id from `resource.h`. `french.xml` says
`<Item id="1" name="Nouveau"/>`, and the only thing that says id 1 is "New" is
`english.xml` sitting beside it.

That shape decides the design. We could have hard-coded a table of `IDM_*`
constants and matched our own `Action` enum against it, but then every
localization would need a translation of our table, and a user dropping in the
real `german.xml` from a Notepad++ install would get nothing. Instead the
mapping is *derived*: read `english.xml` to learn that id 1 is "New", read the
target to learn that id 1 is "Nouveau", and store the pair. The catalog is then
keyed by English label, which is what the menu already has.

### The id spaces do not line up

The first version of `items()` returned a flat `(id, name)` list and joined the
two files on the id alone. Ten of the fifteen tests passed. The five that failed
were all the same bug: `<Main>` and `<SubEntries>` number their items
independently, so id 0 is both "File" (a menu title) and "Edit" (a submenu
title). A `HashMap` keyed on the id keeps the last one, so "File" was translated
with the French for "Edit" and "New" with the French for "Search".

The fix is to carry the section through: `items()` returns
`(section, id, name)` and the join is on `(section, id)`. `<MessageBoxes>` is
still excluded, and for a different reason -- it has its own id space again, and
its strings are dialog text rather than menu rows.

### A file with no reference file

A hand-written localization can skip the numeric indirection entirely:
`<Item id="new" name="Nouveau"/>` needs no `english.xml`, because the id *is*
the English label. `Catalog::parse` takes `Option<&str>` for the reference and
falls back to that reading when there is none.

The fallback originally inserted every id, numeric or not, which meant a
reference-less `french.xml` produced a catalog of six entries keyed `"0"`,
`"1"`, `"2"` -- entries that could never match an English label and that
collided with each other across sections. It now skips numeric ids in that
branch: a catalog that cannot translate anything should be empty rather than
full of keys nothing will look up.

### Relabelling without rebuilding

`MenuBar` gained `set_catalog(Option<Catalog>)`. The obvious implementation
walks the menus and replaces each label with `catalog.tr(label)`, which is wrong
the second time it runs: it translates a translation. So `Item` and `Menu` each
keep their English text in an `english` field, and `relabel` always starts from
that. `set_catalog(None)` therefore restores English exactly, which is what the
Preferences row needs when the user steps back to English.

`relabel` is a free function taking `Option<&Catalog>` and `&mut [Menu]` rather
than a method. A `&mut self` method cannot borrow `self.catalog` and
`self.menus` at once; two disjoint field borrows in one call expression can.

The four list setters -- `set_recent`, `set_runs`, `set_languages`,
`set_plugins` -- rebuild their menu from English literals, so each now calls
`relabel` after assigning. Their "did anything change?" checks compare
`english` rather than `label`: with a catalog active the displayed labels are
French and a freshly built English list would never compare equal, so every
frame would rebuild the menu.

### Where the language lives

`Settings.language` is a file name, not a path. The directory is ours
(`<config>/npp-rs/localization`), and a stored absolute path would break the
moment the config moved. `language none` is how the file spells English, and it
is written unconditionally so the key is discoverable.

`apply_language` runs from `apply_settings_quiet`, which is the single funnel
every settings change already goes through -- the dialog, the Notepad++ import,
and startup. A file that will not parse leaves the menu in English and puts the
error in the status bar. Refusing to start over a broken translation would cost
the user their editor for a cosmetic fault.

The Preferences dialog gained a twelfth row. It steps through a list the caller
supplies, with English prepended by `set_languages` rather than by the caller,
so the row can never be left with no value to show. It clamps rather than wraps,
like every other row: a wrap from the last language back to English reads as a
mistake.

### Verification

1274 tests, up from 1252. `nn-app::i18n` contributes 15: the language name
from a real file, the section-namespaced join, a name-keyed file needing no
reference, a file with no `<Menu>`, a missing file, a missing directory, the
listing skipping `english.xml` and sorting by name, and the entity unescaping.
`nn-app::menu` gained five: a catalog relabels titles and rows, clearing it
restores every English label, applying the same catalog twice does not
translate a translation, a rebuilt recent list is relabelled too, and a catalog
does not change which action a row runs. `nn-app::settings` gained two: the
language round-trips through the file, and English is written as `none` and
read back as `None`.

Clippy is clean at `--all-targets`. The release build is clean.

Checked with the bench, which gained `lang=<file>` and `prefs_lang`:

- `bench ... menu lang=/tmp/lang/french.xml` draws `Fichier Edition Lignes
  Recherche Affichage Encodage Langue Parametres Macro Execution` across the
  bar and `Nouveau Ouvrir... Enregistrer Enregistrer sous... Tout enregistrer
  Recharger depuis le disque` down the File dropdown. The dropdown is wider than
  in English because its width comes from the labels.
- `bench ... prefs_lang lang=/tmp/lang/french.xml` shows the Preferences dialog
  with the Language row highlighted and reading `Francais`.
- `bench ... settings_menu lang=...` opens the Settings menu under
  `Parametres`. That flag had been opening index 6 since the Language menu was
  inserted there in an earlier pass, so it was showing the wrong menu; it and
  `run_menu` are corrected to 7 and 9.

A dump with no flags is pixel-identical to the previous baseline: with no
catalog loaded the bar is built from the same literals as before.

## Thirty-sixth pass: printing

Notepad++ prints through Scintilla's `SCI_FORMATRANGE`, which hands the
platform a device context and lets the driver decide where the page breaks.
There is no such thing on Linux, and there is no such thing in a wgpu window
either: we have no printer device context, no font metrics from a driver, and no
`StartDoc`/`EndDoc` to bracket a job. So the page has to become a value.

That is the whole shape of this pass. `nn-core::print` turns a document plus a
`PageSetup` into a `Layout` of `Page`s, each a fixed grid of rows; `nn-core::pdf`
turns a `Layout` into bytes; `nn-app::print` draws a `Layout` on screen and
hands the bytes to `lp`. Nothing in the chain needs a window, a printer, or a
display, which is why the layout arithmetic is testable at all.

### Why the page model lives in the core

The tempting shortcut is to lay out pages inside the preview widget, since that
is the only place a page is ever seen. It fails on the second consumer: `Print
Now` has no preview, and the PDF writer has no widget. Both need the same
answer to "which line is on which page", and two implementations of that
question drift the moment either changes.

So `layout()` is a pure function of `(text, setup, char_width, fields)`. It takes
a character width rather than a font because a monospaced page is a grid: the
number of columns is `area.width / char_width`, and the caller measures the
character once. That keeps `nn-core` free of any font stack, and it means the
tests can assert exact column counts with a made-up width of 6.0.

### Word wrap, and why a continuation carries no line number

`wrap_line` breaks on the last space that fits and falls back to a hard break
when a single word is longer than the column count. The fallback matters: a
minified JavaScript file is one 40,000-character line, and a wrapper that only
breaks on spaces would put it on one page and clip the rest.

A wrapped continuation is a `PageLine` with `number: None`. The alternative --
repeating the number -- makes a wrapped paragraph look like a list of
one-line paragraphs, and it also breaks the gutter alignment, because the
number column is sized from the widest number and a repeated number is the same
width but reads as a new line. `None` is what the renderer uses to skip the
gutter, and what the PDF writer uses to skip the number run.

### Form feeds

`split_lines` treats `\x0c` as a page break. The subtlety is that a form feed is
*also* a line terminator: `a\x0cb` is two lines, not one line containing a form
feed. So the splitter emits the form feed as its own line and swallows the
newline that follows it, otherwise `a\x0c\nb` would produce a blank line after
the break.

The break itself cannot be represented as "an empty row", because a blank line
in the source is also an empty row and the two must not be confused: one is
content, the other is a forced break. `layout()` therefore works on
`Vec<Option<PageLine>>` where `None` is the break. A blank source line becomes
`Some(PageLine { text: "", .. })`, which occupies a row; `None` starts a new
page.

### PDF rather than a print backend

CUPS accepts PDF directly, and every Linux desktop already has a PDF reader, so
one writer serves both the print job and the "save what I would have printed"
case. The alternative -- emitting PostScript, or driving `gtk_print` -- would
mean a dependency on GTK or on a PostScript generator, and neither is in the
offline crate cache.

The writer is deliberately small: four fixed objects (catalog, page tree, two
Courier fonts) and then a page/stream pair per page. Courier is a base-14 font,
so no font is embedded and the file stays proportional to the text. Widths come
from `COURIER_WIDTH = 0.6`, which is exact for Courier and is why the on-screen
preview and the PDF agree about where a line ends.

`escape` maps to WinAnsi and replaces anything above `0xFF` with `?`. That is a
real limitation -- a document with CJK text prints question marks -- but the
alternative is embedding a font and a `ToUnicode` CMap, which is a much larger
piece of work than the rest of this pass. It is written down here so the next
person does not discover it from a printout.

### The preview is the page-setup dialog

Notepad++ has a separate `Print Setup` dialog and a preview that only previews.
We merged them: the preview's control bar carries paper, orientation, line
numbers, and the two buttons, and every change re-lays-out immediately. The
reason is that page setup is only meaningful in terms of its effect on the page,
and a dialog that changes a paper size without showing the result is a dialog
the user has to close and reopen to evaluate.

`Control::label(&PageSetup)` is a method rather than a stored string for the same
reason: `Paper` shows `A4`, `Orientation` shows `Portrait` or `Landscape`, and
`LineNumbers` shows `Numbers: On` or `Numbers: Off`, all derived from the setup
the preview already holds. There is no second copy to keep in sync.

### Two bugs found while wiring it up

The first was a panic. `relayout()` cleared `controls_shaped` and then indexed
it per control, so the second frame after any change indexed an empty vector.
The fix is to reset each entry in place rather than clear the vector, which also
keeps the allocation.

The second was a font-size mismatch that only showed on screen. The body buffer
was shaped at the UI font size (15) while `layout()` had been given a character
width derived from the print size (10), so the preview drew text about 50% too
wide and it overflowed the sheet. `prepare` now calls
`body.set_metrics(Metrics::new(setup.font_size, setup.line_height))` before
shaping. The PDF was always correct, which is why the bug survived the PDF
checks and only appeared in the screenshot.

`PrintPreview` also keeps `ui_char_width` separate from `char_width`: the
control bar is drawn at the UI size and the sheet at the print size, and using
one width for both put the button labels outside their buttons.

### Verification

1368 tests, up from 1274. `nn-core::print` contributes 33: paper sizes in both
orientations, `from_key` for every key and for an unknown one, the margin
arithmetic, `rows`/`columns` for a known width, `substitute` for each token and
for an unknown one left literal, `zones` for one, two and three tabs, tab
expansion, word wrap at a space, wrap with a word longer than the column count,
a continuation carrying no number, a form feed starting a page, a form feed
swallowing its newline, a blank line occupying a row, and the header/footer
furniture rows. `nn-core::pdf` contributes 23: the header, the xref offsets
pointing at real object starts, `startxref` matching the xref offset, the page
count in `/Count` and in the kids array, escaping of `(`, `)` and `\`, a
non-Latin-1 character becoming `?`, the footer baseline, the line-number gutter
being right-aligned, and a multi-page document numbering its objects without
collision. `nn-app::print` contributes 35: every control's label for both
states, `control_at` inside and outside each button, page navigation clamping at
both ends, `set_page` past the end, cycling paper wrapping, the layout being
rebuilt when the setup changes, `compose` centring and right-aligning, and
`spool_with` invoking a program with the expected arguments.

Clippy is clean at `--all-targets`. The release build is clean.

Checked with the bench, which gained `print`, `print_page=<n>` and
`print_pdf=<path>`:

- `bench ... 900 700 2 print` draws the sheet centred with the text wrapped
  inside the margins and the control bar reading `< > A4 Portrait Numbers: Off
  Print Close`, each label inside its button.
- `bench crates/nn-core/src/editor.rs 900 500 2 out.ppm print
  print_pdf=/tmp/out.pdf` writes a PDF that `pdfinfo` reports as 41 pages,
  595x842 pts (A4), PDF 1.4, 191752 bytes, and `pdftotext` extracts the source
  with the word wrap intact.
- `gs -r80 -dFirstPage=1 -dLastPage=1` on that file renders a correct A4 page:
  2cm margins, Courier, the footer `1` at the bottom left.

Checked in the real binary, which is the only place the modal ordering and the
spool path are exercised:

- `nn-app --new-instance /tmp/a.txt`, then Ctrl+P, opens the preview titled
  `Print - a.txt` with the sheet centred and `alpha beta gamma` on it.
- Clicking `Print` closes the preview and puts the CUPS reply in the status bar:
  `request id is HL-5380DN-246 (1 file(s))`. The spooled file is
  `/tmp/npp-rs-<stamp>.pdf`, 910 bytes, one A4 page, and `pdftotext` reads back
  `alpha beta gamma` followed by the footer `1`.

A dump with no flags is pixel-identical to the previous baseline.

## Thirty-seventh pass: when there is no GPU

The adapter request was one line:

```rust
let adapter = instance.request_adapter(&RequestAdapterOptions {
    power_preference: PowerPreference::HighPerformance,
    compatible_surface: Some(&surface),
    ..Default::default()
}).expect("no compatible GPU adapter");
```

That asks for a discrete GPU and panics if it does not get one. On a laptop with
only an integrated GPU it usually still answers, because wgpu treats the power
preference as a hint. On a machine with no GPU driver at all it does not, and
the editor dies in a constructor with a message that names neither the cause nor
a fix.

The failure is not a missing GPU. It is a question that was too specific.

### A ladder, not a request

The adapter is now asked for in steps, most to least desirable, and the first
answer wins:

| Rung | Power preference | Fallback adapter |
|---|---|---|
| `Fast` | `HighPerformance` | no |
| `Slow` | `LowPower` | no |
| `Software` | `LowPower` | yes |

The last rung is the one that matters. `force_fallback_adapter` is how wgpu
spells "software rasterizer": lavapipe on Vulkan, llvmpipe on GL. Both ship with
Mesa, which is on every Linux desktop, so the rung turns "no GPU driver" from a
refusal into a slow but working editor.

`ladder()` is a plain function returning a plain `Vec<Rung>`, and `Rung::request`
returns a two-field struct rather than a `wgpu::RequestAdapterOptions`. That type
is generic over the surface, and the surface is the one part of the request the
ladder has no opinion about; keeping the rung's half separate is what lets the
order and the flags be tested without a window or a GPU. Six of the nine tests in
`gpu` are about that table.

### A device request can fail after the adapter is found

The first version stopped at the first adapter that enumerated. That is wrong:
`request_device` can fail on an adapter that enumerated fine -- a driver that
advertises a feature it cannot deliver, a container with no render node. An
adapter that cannot open a device is no more use than no adapter, so the loop
keeps the error and tries the next rung.

### Two failures that look the same and are not

When every rung fails, the message has to say what to do, and there are two
different situations behind one symptom:

- **No adapter visible at all.** No driver. The fix is an install, so the message
  names the package for three distributions.
- **An adapter visible that cannot present to this window.** The driver is there
  and the surface is the problem. Telling this user to install Mesa sends them
  down a dead end, so the message says the surface is the problem and points at
  `--software`.

The distinction is `seen.is_empty()`, and it is worth the branch: the second case
is exactly what a nested or remote X session produces, and it is the case where
the install advice is actively wrong.

`enumerate_adapters` is only called on the failure path. It is a second round
trip to the driver and the answer is only wanted for the error text.

### `--software`

The software path is the hardest one to test by accident: on a machine with a
working GPU the first rung always wins, so without a way to skip to the end the
fallback would only ever run on hardware nobody has. `--software` / `-s`
collapses the ladder to its last rung, and the app says so on stdout:

```
npp-rs: llvmpipe (LLVM 15.0.6, 256 bits) (Vulkan)
npp-rs: software rendering, expect a slower frame rate
```

The second line exists because "llvmpipe" means nothing to most people, and it is
the explanation for every performance complaint that follows.

`is_software()` reads `adapter.get_info().device_type == DeviceType::Cpu` rather
than the rung that answered. A driver can hand back a software adapter for a
hardware request, and the log should say what is actually drawing.

### Why not a hand-written rasterizer

The alternative is a CPU renderer of our own: fill the quads into a pixel buffer,
blit glyph masks from `SwashCache`, present through X11 SHM. It would work with
no driver at all, and it was rejected for three reasons.

It only covers X11. A Wayland session would still need llvmpipe, so the fallback
would be partial and the partial part is the one that is hardest to test.

It duplicates the glyph atlas. glyphon packs glyphs into a texture and hands back
UVs; a CPU path would need its own packing, its own subpixel positioning, and its
own cache eviction, and the two would drift the moment either changed.

It is not the failure that happens. Mesa is on every distribution that ships a
desktop, so "no adapter at all" means a machine with no graphics stack, which is
a machine that cannot run a compositor either. The realistic failure is "the
first rung was too specific", and the ladder fixes that in eighty lines.

### Verification

1379 tests, up from 1368. `nn-app::gpu` contributes 9: the ladder order, the
collapse under `--software`, only the last rung asking for a fallback adapter,
the first rung asking for the discrete GPU and the software rung not asking for
high performance (a fallback adapter is by definition not the fast one, and
asking for both is a contradiction some backends resolve by returning nothing),
the three distinct labels, the no-adapter message naming the rungs and the
install, the message listing the adapters it could see, and the visible-adapter
case getting surface advice instead of install advice. `nn-app::cli` gained two:
`--software` and `-s` both set the flag and it is off by default, and `-sn`
clusters.

Clippy is clean at `--all-targets`. The release build is clean.

Checked on the real binary:

- `nn-app --software --new-instance /tmp/a.txt` starts on
  `llvmpipe (LLVM 15.0.6, 256 bits) (Vulkan)` and prints the software-rendering
  line. The window is pixel-identical to the hardware render apart from the
  status bar text and 2473 pixels differing by at most 8, which is antialiasing
  between llvmpipe and the NVIDIA driver.
- With the Vulkan ICD list pointed at an empty directory and the GL driver
  override set to a name that does not exist, the app exits 1 with
  `no usable GPU adapter (tried high-performance, low-power, software)`,
  `adapters visible: NVIDIA GeForce GTX 1080 Ti/PCIe/SSE2 (Gl)`, and the
  surface-advice branch -- which is the correct branch, because the GL adapter
  was visible and could not present to the Vulkan surface.
- `--help` lists `-s, --software`.

A dump with no flags is pixel-identical to the previous baseline.

## Thirty-eighth pass: being readable

A text editor that a screen reader cannot read is a text editor half the
population cannot use. On Linux that means AT-SPI over D-Bus, and the shortest
honest route to it is AccessKit: `accesskit` describes the tree,
`accesskit_unix` speaks AT-SPI, and `accesskit_winit` connects the two to a
winit window. All three were in the offline cache, and `accesskit_winit` 0.34
targets winit 0.30.5, which is what we are on.

### The tree is built from a snapshot, not from the widgets

The obvious implementation walks the live widgets and emits nodes as it goes. It
cannot be tested: every widget needs a font system, a theme and a window, and the
tree is the one thing a screen reader user cannot check by looking at the screen.

So the widgets are flattened into a `Snapshot` -- strings and indices, nothing
else -- and `a11y::tree` is a pure function of it. The whole shape of the tree,
the node ids, the action mapping and the truncation rule are then testable
without a display. Twenty-nine tests in `nn-app::a11y` run in milliseconds.

The snapshot is also what makes the borrow checker happy: the adapter needs a
mutable borrow of itself while the widgets are borrowed immutably, and one owned
struct is what lets both happen at once.

### What is exposed, and what is not

Five things, because they are the five a screen reader user needs to use an
editor:

| Node | Role | Carries |
|---|---|---|
| root | `Window` | the title, which names the document |
| menu bar | `MenuBar` | every menu, every row, every shortcut |
| tab strip | `TabList` | the open documents, active one selected |
| text | `MultilineTextInput` | the document, with the caret's line selected |
| status | `Status` | line, column, selection, length, encoding |

The status bar is left out entirely when it is empty rather than emitted as an
empty label, because a screen reader announces an empty status node as a pause.

### The caret is line-granular, and that is the protocol's fault

AccessKit expresses a text position as `(text run, character index)`, and the
character index selects an entry in `character_lengths`, which is one entry per
*line*. There is no way to say "line 4, column 12" in the tree. So the selection
marks the caret's line, and the column is carried by the status bar's label,
which is where a screen reader reads it from anyway.

This is worth writing down because it looks like a bug from the outside: the
caret offset reported over AT-SPI is the start of the caret's line, not the
caret. It is the finest granularity the format has.

### A trailing newline does not start a line

`line_lengths` counts `"a\n"` as one line, not two. The caret after the newline
is at the end of line one, not at the start of an empty line two, and that is the
same rule the editor's own line count uses. The two disagreeing is how a screen
reader ends up one line off from the caret it is describing.

The field is a `u8`, so a line longer than 255 characters saturates. The count of
lines stays right, which is what the caret index depends on; only the length is
clipped, and only for word wrapping.

### Large documents are cut, and say so

The text goes over D-Bus on every update, so a 100 MB file would be a 100 MB
message. `MAX_TEXT` caps it at 64 KiB -- about a thousand lines of prose, enough
for a whole source file -- and past that the value is cut and a line saying
`[document truncated for the screen reader]` is appended. A screen reader then
reports a truncated document rather than a wrong one.

The cut lands on a character boundary, not a byte one. `&text[..MAX_TEXT]`
panics when the limit falls inside a multi-byte character, and a document with an
accent near the limit would take the accessibility tree down with it.

### Rebuilding only when something changed

The tree is rebuilt from the whole document, so it must not be rebuilt every
frame: at 120 Hz that is 120 copies of the file per second for a screen reader
that is not even running. `a11y_signature` hashes the cheap facts -- active tab,
tab count, document id, document revision, caret line, status text, and the menu
labels -- and `sync_a11y` rebuilds only when the hash moves.

The document's own revision counter answers most of it without touching the text.
The menu is hashed by its labels rather than by a counter because a language
change and a new recent file both alter the tree without touching the document.

The signature is only recorded when the update actually went out. An inactive
adapter drops the update, and recording the signature anyway would leave the tree
stale for the whole time the screen reader was off.

### Two things that only showed up over D-Bus

**The window title was empty.** `a11y_snapshot` read `window.title()`, which on
X11 returns the attributes the window was *created* with rather than the last
string handed to `set_title` -- and since the window is created before the first
document is opened, that is the empty string. The fix is to build the title in
one place, `window_title()`, and have both `update_title` and the snapshot call
it. The title bar and the tree now cannot disagree.

**A menu could be read but not opened.** `Action::Click` was added to menu items
but not to menu titles, so the `Menu` node had no AT-SPI action and
`Command::OpenMenu` was unreachable. A screen reader user could hear "File" and
do nothing with it. The title now carries the action too.

### The window starts hidden

`accesskit_winit::Adapter` panics if the window is already visible, so the window
is created with `with_visible(false)` and shown once the adapter exists. The gap
is one call wide and nothing is drawn in it.

### Verification

1408 tests, up from 1379. `nn-app::a11y` contributes 29: the root being a window
named after the document, the root listing the bars and the text in drawing
order, the status bar being left out when empty, every menu being a node with its
rows as children, a separator not being a row, a menu title being clickable, a
row carrying its label and being clickable, a row with a chord advertising it and
one without not, the tab strip marking the active tab, the text being a multiline
input with one run, the run counting one entry per line, a document without a
trailing newline still counting its last line, an empty document having no lines,
the caret marking its line, a caret past the last line being clamped, a long
document being cut and saying so, a cut landing on a character boundary, a short
document not being cut, the text being focused, clicking a row running it,
clicking a separator doing nothing, clicking a menu title opening it, clicking a
tab selecting it, clicking the text focusing it, the read-only nodes having no
command, an id past the end having no command, the ids of different things not
colliding, and every node being reachable from the root.

Clippy is clean at `--all-targets`. The release build is clean.

Checked against a live AT-SPI bus, which is the only place the protocol is
exercised. `org.a11y.Status.IsEnabled` is false on this machine, so AccessKit
correctly stayed dormant until it was flipped on; it was flipped back afterwards.

- `pyatspi` walking the desktop finds `[application] 'nn-app'` with
  `[frame] 'a.txt — npp-rs'`, a `[menu bar]` of twelve `[menu]` nodes, a
  `[page tab list]` holding `[page tab] 'a.txt'`, an `[entry]` named after the
  document, and a `[status bar]` reading
  `Ln: 1, Col: 1    Sel: 0    length: 17    lines: 4  Tab bar: Single Row`.
- `entry.queryText()` returns 17 characters, `'alpha\nbeta\ngamma\n'`.
- Two Down presses move the caret to line 3; `caretOffset` becomes 11, which is
  the start of `gamma`, and the status bar becomes `Ln: 3, Col: 1`.
- `queryAction().doAction(0)` on the `Edit` menu title opens the dropdown, which
  the screenshot shows.
- `doAction(0)` on `Insert Date/Time` inserts the timestamp: the text interface
  then reads `'2026-11-19 16:51:48alpha\nbeta\ngamma\n'` and the status bar
  reports `length: 36`, `Col: 20`. The tree updated without a restart.

A dump with no flags is pixel-identical to the previous baseline.

## Thirty-ninth pass: the dialog that is not ours

### The problem with drawing our own

`browser.rs` is a file browser, and it is what Notepad++ does: a dialog the
application draws. On Linux that is the wrong answer twice over.

The first is reach. A user's file chooser is where their bookmarks, their recent
files, their network shares and their mounted phone live. A dialog we draw can
only show what we know how to enumerate, which is the filesystem. The screenshot
of the portal dialog on this machine lists twenty bookmarks -- `BRIDGE`,
`BAREMETAL`, `PHOTOPRINT`, `jls_export_pure_rust` -- that our browser has no way
to know about.

The second is sandboxing, and it is the one that decides the design. Under
Flatpak or snap the application cannot read an arbitrary path at all. The only
way to open a file is to ask the portal, which asks the user, which hands back a
document-portal handle. An editor that only knows how to open paths is an editor
that cannot open anything in a sandbox -- not "opens it awkwardly", cannot.

So the portal is tried first and `browser.rs` is the fallback. The fallback is
not a courtesy: the portal is a separate service, and on a bare X session, in a
container, or on a CI box there is nothing to ask. Asking anyway would mean no
file dialog at all.

### The handshake, and why it is shaped like this

`FileChooser.OpenFile` does not return a path. It returns an object path for a
*request*, and the answer arrives later as a `Response` signal on that path. The
path is derived from the caller's unique bus name and a token the caller picks,
which is what makes it possible to subscribe *before* making the call. Subscribing
afterwards would race the signal, and the race is not theoretical: the portal can
answer before `OpenFile` returns.

`handle_path` is therefore a separate function with its own tests. Getting it
wrong means subscribing to a path nothing will ever be sent to, and the symptom
is a dialog that opens, is answered, and then hangs until the timeout -- which is
a bad way to discover a string bug.

The call blocks until the user answers, so it runs on its own thread and the
result comes back through the event loop as `UserEvent::Portal`. `State` carries
a `portal_pending` flag so a second Open while a dialog is up is ignored rather
than starting a second dialog whose answer nothing tracks.

### Response 2 is a cancellation

The first version treated the response codes as 0 success, 1 cancelled, anything
else failed. Pressing Escape in the GTK dialog sends **2**, so the first live test
of cancellation produced a status message reading "System dialog unavailable (the
portal answered 2)" and then opened the built-in browser on top of the dialog the
user had just dismissed.

The spec says 2 is "the interaction ended some other way" -- the dialog was
closed, or the window manager took it away. From the user's side that is the same
thing as cancelling, and answering a closed dialog with a second dialog is the
one behaviour Escape must not produce. `response_outcome` now maps 1 and 2 both
to `Cancelled`, and only an unrecognised code is a failure.

### The fallback has to be a drop-in

`builtin_chooser` keeps the two shapes the two commands always had: Open browses,
Save As types a path. That is what makes the fallback invisible -- neither command
changes behaviour when the portal is missing. `save_as_path` was factored out of
the prompt handler so the portal's answer and the typed path go through exactly
the same code, including the backup of the file being replaced.

A failure *after* the availability check -- the portal died mid-request, or
refused -- reports why in the status bar and then falls back, so the command still
works. A cancellation reports nothing, because it is an answer.

### `file_dialog`

The setting exists because the portal is not always the right dialog: a user on a
remote X connection may prefer the one that does not need a bus, and a bug report
about the portal needs a way to prove the rest of the editor is fine. It is a
`FileDialog` enum with `key`/`label`/`from_key`/`next`, the same shape as
`BackupMode`, and it appears in the Preferences dialog as a row that steps rather
than toggles -- a switch would read as on/off, and neither choice is the off one.

An unreadable keyword falls back to `Portal` rather than `Builtin`, because the
portal choice is the one that degrades: when it is missing we fall back anyway, so
a corrupt settings line cannot leave the user with no dialog.

### Verification

1432 tests, up from 1408. `nn-app::portal` contributes 21: the handle path
dropping the colon and flattening the dots, a sender without a colon, a plain
`file://` URI, a percent escape, a literal `%` round-tripping, a malformed escape
being kept rather than dropped, a UTF-8 escape, a remote host being rejected, a
non-file URI being rejected, the root, filters becoming glob patterns, no filters,
the options always carrying a token and being modal, a save dialog carrying the
suggested name, filters only being sent when there are some, response 0 being
success, both cancellation codes being cancellations, an unknown response being a
failure, and the two kinds calling different methods. `settings` gained four for
the new key and `prefs` one for the new row.

Clippy is clean at `--all-targets`. The release build is clean.

Checked against the live `xdg-desktop-portal` on this machine, which is the only
place the protocol is exercised:

- Ctrl+O opens a GTK dialog titled `Open`, listing the user's real bookmarks.
  `WM_TRANSIENT_FOR` on it is `0x3000004`, our window, so `parent_id` produced the
  right `x11:<hex>` and the dialog is modal to us.
- Typing `/tmp/c.txt` and pressing Enter opened a third tab reading `x y z`. A
  mistyped path (`/tmp/a;txt`, from this AZERTY keyboard) produced
  `npp-rs: cannot open /tmp/a;txt: No such file or directory`, which is the proof
  the path made the whole round trip rather than being guessed.
- Ctrl+Shift+S opens a `Save As` dialog with the name field pre-filled `c.txt`,
  the `current_name` option. Typing `portal-save.txt` and pressing Enter wrote
  `~/portal-save.txt` containing `x\ny\nz\n`, renamed the tab, and cleared the
  modified marker.
- Escape now closes the dialog and does nothing else.
- With `DBUS_SESSION_BUS_ADDRESS` pointed at a socket that does not exist, Ctrl+O
  opens the built-in browser at `/tmp` immediately, with no message and no delay:
  `available()` returns false and the fallback is taken before a thread is
  spawned.
- With `file_dialog builtin` in `settings.txt` and the bus present, Ctrl+O opens
  the built-in browser and no portal dialog appears.
- The Preferences dialog shows `File Dialog    System dialog` between Autosave and
  Language.

A dump with no flags is pixel-identical to the previous baseline.

## Fortieth pass: the wait that ends

The thirty-ninth pass left a hole it documented but did not fill. `portal::TIMEOUT`
was declared, given a paragraph of justification, and never read by anything.
`MessageIterator::next` blocks until a matching signal arrives, and a portal that
stops answering -- one that crashed after accepting `OpenFile`, or a bus that
went away mid-request -- would park the thread forever with `portal_pending`
still set. Open and Save As would then be dead for the rest of the session, with
no message and no way to recover short of restarting the editor.

The comment in `choose` claimed the timeout was "enforced by the caller's thread
being joined". That was not true. Nothing joined the thread.

### Why the blocking iterator cannot be given a deadline

`zbus::blocking::MessageIterator` is a thin wrapper: `next` is
`block_on(stream.next())`, and `block_on` here is `async_io::block_on`, which
parks the thread on the reactor until the future resolves. There is no timeout
parameter, no `next_timeout`, and no way to interrupt a parked `block_on` from
another thread. The public surface is `inner`, `into_inner`, `for_match_rule` and
`match_rule`.

`into_inner` is the way out. It hands back the `zbus::MessageStream` the iterator
was wrapping, and a stream is a future-shaped thing that can be raced. So the
wait became:

    async_io::block_on(futures_lite::future::or(
        async { answers.next().await },
        async { async_io::Timer::after(TIMEOUT).await; None },
    ))

`async_io::block_on` and not `futures_lite::block_on`, and the difference is the
whole trick. `futures_lite::block_on` is a `parking::Parker` loop: it polls the
future, parks when it is pending, and never touches the async-io reactor. A
`Timer` registered inside it would be armed and then never fire, so the race
would resolve only when the stream did -- which is the bug, not the fix. Reading
`futures_lite`'s `block_on` before writing this is what caught it; the two
functions have the same name and the same signature and only one of them works.

Both crates are already in the graph -- `async-io` is what zbus's `async-io`
feature pulls in, `futures-lite` is a non-optional zbus dependency -- so naming
them in `nn-app/Cargo.toml` costs no build time. They are named directly rather
than reached through `zbus::` because `zbus::block_on` is `#[doc(hidden)]`, and
depending on a hidden re-export is a way to be broken by a patch release.

### The race is a function, so it can be tested

`with_timeout` is four lines and exists only so the part that can be wrong has a
test. The D-Bus half of `choose` cannot be tested without a portal; the race can
be tested with `std::future::pending`, which is exactly what a wedged stream
looks like from the caller's side.

Two tests: a future that answers is returned, and a future that never answers
gives up. The second one asserts both the `None` and that it came back quickly,
because a version that returned `None` after the full timeout would pass a
naive assertion and be useless.

A third test pins `TIMEOUT` at no less than a minute. It is a guard against
someone shortening the constant to make a test faster, which would ship an editor
that gives up on a user who is reading a directory listing.

### What a timeout means to the user

`Outcome::Cancelled`, not `Outcome::Failed`. A dialog that never came back is,
from the user's side, a dialog they closed: the right response is to do nothing,
not to pop a message about a portal they have never heard of. This is the same
reasoning as response code 2 in the previous pass, and it is why the timeout
could be added without touching `portal_finished`.

The stream ending is kept separate. `answers.next()` returning `None` means the
connection went away, which is a failure and says so; only the timer produces a
cancellation.

### Verification

1435 tests, up from 1432. `nn-app::portal` contributes 22, three of them new:
an answer that arrives is returned, an answer that never arrives gives up, and
the timeout is long enough to pick a file.

Clippy is clean at `--all-targets`. The release build is clean.

The live check needed a portal that accepts a request and never answers, which
the real one will not do on demand. `xdg-desktop-portal` owns
`org.freedesktop.portal.Desktop` on the session bus, so a fake cannot take the
name there. Instead a private bus:

    dbus-daemon --session --print-address --fork --print-pid

and `/tmp/fakeportal.py`, a PyGObject service that exports
`org.freedesktop.portal.FileChooser`, owns the portal name on that bus, returns a
valid request handle from `OpenFile`, and never emits `Response`. The editor was
started with `DBUS_SESSION_BUS_ADDRESS` pointed at the private bus, so
`available()` found the fake and `choose` talked to it.

- The fake logged `got OpenFile token=npp1466687 -> never answering` at 17:26:43.
- At 17:27:44, 61 seconds later, Ctrl+O did nothing at all: no dialog, no
  built-in browser, no message. `portal_pending` was still set, which is the
  correct behaviour while a dialog is outstanding.
- At 17:32:26, 343 seconds after the request and 43 seconds after the deadline,
  with the fake killed so `available()` would return false, Ctrl+O opened the
  built-in browser at `/tmp`. The flag had been released by the timeout and Open
  worked again.
- The editor was still running throughout; nothing panicked and nothing was
  logged.

The ordinary path was re-checked against the real portal afterwards, because
swapping `MessageIterator::next` for `MessageStream::next` changes the code that
receives the answer: Ctrl+O opened the GTK dialog, typing `/tmp/c.txt` and
pressing Enter opened a third tab, and Escape closed a second dialog silently.

A dump with no flags is pixel-identical to the previous baseline.

## Forty-first pass: getting it onto a machine

The AppImage answers "I want to try this". It does not answer "install this on
the machine I use", which on Debian and its descendants means a `.deb`: a file
in `/usr/bin`, a desktop entry the shell indexes, icons in the theme, a manual
page, and a dependency list the package manager can reason about.

### `debian/` at the top of the tree, not under `packaging/`

`dpkg-buildpackage` looks for `debian/` beside the source it is building, and the
source it is building is the whole workspace -- `cargo build` needs `Cargo.toml`,
`Cargo.lock` and both crates. Putting the directory under `packaging/` would mean
either a copy step or a `--debian-dir` flag that does not exist. So `debian/`
sits at the root, where every Debian-packaged project puts it, and
`packaging/build-deb.sh` is a wrapper that runs the build and moves the artefacts
into `dist/` beside the AppImage.

### Everything debhelper would guess is spelled out

`debian/rules` overrides `dh_auto_configure`, `dh_auto_build`, `dh_auto_test` and
`dh_auto_install` rather than letting debhelper detect a build system. Two
reasons. The build has to be `--offline`, because the machine this is built on
has no network and a build that silently reaches for the registry would fail
there and nowhere else. And debhelper has no cargo build system, so the
auto-detected path would be "nothing to do" followed by an install step that
found no binary -- a failure that reads like a missing file rather than a missing
rule.

`dh_auto_test` runs the whole suite, which is the point: a package that builds
but does not pass its own tests is not a package.

### The dependency list is two lists

`dpkg-shlibdeps` reads the ELF `NEEDED` entries and produces
`libc6 (>= 2.35), libgcc-s1 (>= 4.2)`. That is the whole of it, and it is
misleading: the editor cannot start without X11 or Wayland, and neither appears
in `NEEDED` because winit loads them with `dlopen`. `strings` on the binary finds
`libX11.so.6`, `libX11-xcb.so.1`, `libXcursor.so.1`, `libXi.so.6`,
`libxkbcommon.so.0`, `libxkbcommon-x11.so.0`, `libwayland-client.so.0` and
`libwayland-egl.so.1`, and those are declared by hand in `debian/control`.

Both stacks are declared even though a given machine uses one. A package cannot
express "X11 or Wayland" without alternatives, and alternatives in `Depends` mean
either one satisfies the dependency -- which is true, but it also means a
Wayland-only machine could end up without the X11 libraries and a user who later
runs the binary under XWayland would find it broken. The libraries are small and
every desktop already has both.

`libegl1`, `libvulkan1` and `mesa-vulkan-drivers` are `Recommends`, not
`Depends`. The GPU ladder from the thirty-seventh pass means the editor runs
without any of them -- it falls to the software rasteriser -- so a machine
without a Vulkan ICD is a machine that draws more slowly, not a machine that
cannot install the editor.

### The manual page is not decoration

`lintian` reported exactly one tag on the first build: `no-manual-page`. The fix
is a real page rather than a suppression, because the switches are not obvious:
`-w` is what makes the editor usable as `$EDITOR`, and `-s` is the answer to "it
crashes on my remote display". The page is `debian/nn-app.1` and is installed
through `debian/npp-rs.manpages`, because `dh_installman` only picks up
`debian/<package>.<section>` automatically and the package is `npp-rs` while the
binary is `nn-app`. Naming the file `debian/npp-rs.1` would have installed a
manual page for a command that does not exist.

### `-d`, and why the build-dependency check is skipped

`debian/control` names `cargo` and `rustc` in `Build-Depends`, which is true and
which `dpkg-checkbuilddeps` then refuses to accept on this machine, because the
toolchain here is a rustup install in `/opt/DEV/rust/cargo/bin` and not a Debian
package. `build-deb.sh` passes `-d` and says why in a comment. The alternative --
dropping the build dependencies from `control` so the check passes -- would make
the file lie about what the build needs.

### Verification

`packaging/build-deb.sh` produces `dist/npp-rs_0.1.0_amd64.deb`, 4.2 MB,
`Installed-Size: 13891`.

`lintian --tag-display-limit 0` reports nothing at all.

The payload is eight files: `/usr/bin/nn-app`,
`/usr/share/applications/npp-rs.desktop`, three icons under
`/usr/share/icons/hicolor`, `/usr/share/man/man1/nn-app.1.gz`, and
`/usr/share/doc/npp-rs/{copyright,changelog.gz}`.

`dpkg-deb -x` into a scratch directory and running the extracted binary gives
`npp-rs 0.1.0` for `--version`, the usage text for `--help`, and a window titled
`b.txt — npp-rs` showing `one two three` when given `/tmp/b.txt`. So the payload
is complete and the binary in the package is the one that works.

`dpkg --dry-run -i` gets as far as `Préparation du dépaquetage` before stopping
on `/var/log/dpkg.log` being unwritable, which is the permission of a
non-root user and not a dependency problem: every `Depends` resolved.

The package was not installed. Installing it would put a file in `/usr/bin` on
this machine, which is a system change and not one the packaging work needs in
order to be believed; the extraction test covers the payload and the dry run
covers the dependencies.

## Forty-second pass: the mark, the labels, and two things that were not true

This pass closes the icon and tab work that the previous one started, and it
corrects two claims the document was making that the code no longer supported.
The corrections come first, because a document that is confidently wrong is worse
than one that is silent.

### The logo is ours now, and it is a kitten

The icon was a redrawing of Notepad++'s: a page with a folded corner, a pencil
lying across it, grass along the bottom, and a chameleon standing on the pencil.
The chameleon is the part that reads as *theirs*, but it is not the only part --
the arrangement of page, pencil, grass and creature is the mark, and swapping the
animal while keeping the composition would have produced something that is still
recognisably derived from it. So none of it is reused.

`packaging/npp-rs.svg` is now a rounded orange tile with a kitten's face on it:
two ears, two eyes, a nose, whiskers, a mouth. The tile is not decoration. A cream
kitten on a transparent background disappears against a light taskbar, and a dark
outline disappears against a dark one; a saturated tile is legible on both, and
the orange is the same orange as the active tab's top bar, so the icon and the
window agree about what the application looks like.

The face is drawn to survive sixteen pixels. Ears, eyes and nose are large shapes
with hard edges; the whiskers and the mouth are thin and are the first things to
go when the icon is scaled down, which is why nothing load-bearing is drawn with
them. The 32-pixel rendering was checked by eye before the drawing was accepted.

The ears are drawn before the head and both use the same fill with no stroke. The
union of the two shapes is the silhouette; a stroke would draw the seam where the
ear meets the head, and a single path with the ears built in would have been four
times the coordinates for the same picture.

`crates/nn-app/src/icon.rs` embeds the 256-pixel PNG with `include_bytes!` and
decodes it at startup, so the taskbar, the window list and the launcher all show
one mark and an installed binary needs no icon search path. Its tests pin the
things that would silently rot: the corners are transparent, the tile is orange,
the fur is cream, and there is a dark eye and a pink nose where the drawing puts
them. A test that only checked "the file decodes" would pass on a blank square.

### Two claims that were not true

**The tab strip and one document.** The document said all three arrangements hide
when there is a single tab. That was never what Notepad++ does -- it always shows
the strip, and it has no option to hide it, because the close and pin buttons live
on the tab and a hidden strip would take two commands away from a
single-document window. `TabBar::is_shown` is now `!self.tabs.is_empty()`, and the
section that made the old claim has been rewritten in place rather than left to
contradict the code.

**Two tabs on one file.** `restore_snapshot()` runs before `apply_args()`, so
launching `npp-rs notes.txt` on a file with an unsaved snapshot produced two tabs
on the same path: the restored text in one and the disk text in the other.
`open_request` deduplicated and `open_path` did not, and startup goes through
`open_path`. The dedupe now lives in `open_path`, which is the only place that
opens a document, and `open_request` is a loop over it. The rule is: focus an
existing tab in the focused pane, then in the other pane, and only then open a new
one. Last save wins, which is the same rule the rest of the editor uses for a file
that changed underneath it.

### Tooltips on the toolbar

Notepad++ has no toolbar tooltips. Its `TBBUTTON` array sets `iString = 0` for
every button, so `TBSTYLE_TOOLTIPS` is set on the bar and never fires. This is
therefore an addition rather than a parity item, and it is worth having: the bar is
thirty-two unlabelled icons, and the difference between Close and Close All is a
few pixels of drawing.

The label appears after the pointer has rested on a button for half a second. The
delay is the whole reason `crates/nn-app/src/tooltip.rs` exists rather than a bare
`if hovered` check: a tooltip that appears the instant the pointer crosses a
button flickers as the pointer travels along the bar to reach a distant one, and a
bar that flickers is worse than a bar with no labels at all. Half a second is what
the desktop's own tooltips use.

The timer lives in the widget, not in the application, because the widget owns the
pair "what is under the pointer, and since when". Moving within one button keeps
the clock running -- the pointer jitters by a pixel while the user holds still, and
restarting on every move would mean the label never appears -- while moving to
another button, or off the bar, restarts it and hides whatever was showing.

Three pieces of wiring are worth naming because each one is a way the feature
could have been silently dead:

* **The wake-up.** The event loop sleeps on `ControlFlow::Wait` when nothing is
  pending, so a tooltip waiting out its delay has to be able to say when it wants
  waking. `Tooltip::deadline` feeds `next_wake`, which already arbitrated between
  the disk poll and the autosave. Without it the label would appear on the next
  unrelated event, which on an idle window is never.
* **The hit test.** `Toolbar::tip_at` is separate from `hover_at` because the two
  answer different questions: `hover_at` moves the highlight and is only called
  once the pointer is known to be on the bar, while the tooltip has to be told
  about every position so that leaving the bar clears the label. It is asked at
  the top of `handle_cursor`, before the paths that return early -- a drag in
  progress, an open completion popup -- so the dwell timer keeps running there
  too.
* **The dismissal.** A pointer that leaves the window sends no further move, so
  `CursorLeft` closes the tooltip explicitly. A button press and a key press close
  it as well, and they drop the *pending* label too, so a click does not bring the
  tooltip straight back: it waits for the pointer to move again.

The label is drawn in the overlay pass, above the toolbar and the popups and below
the modals, because a label is an annotation on the window and a dialog is a
replacement for it. `rest_tooltip` refuses to rest anything while a modal is up,
which is the same list `right_click` uses; that list is now `State::modal_open`
rather than a six-condition `if` written out twice.

The tooltip has its own two theme tokens. On the light palette it is deliberately
dark, which is what makes it read as a label stuck on top of the window rather
than as a panel that belongs to it.

The tab strip's close and pin buttons are not tooltipped. They are not menu
actions, and the request that prompted this was about the toolbar's icons; the
same widget would serve them if that changes.

### Verification

`cargo test --offline`: **852 nn-app + 802 nn-core = 1654 pass, 0 fail.**
`cargo clippy --offline --all-targets`: **0 warnings.**
`cargo build --offline --release`: succeeds in 2m11s.

The icon: `inkscape npp-rs.svg --export-width=256` and `--export-width=128`
regenerate `packaging/npp-rs-256.png` and `packaging/npp-rs-128.png`, which are
what `debian/rules` and `packaging/build-appimage.sh` install. The 256-pixel
render was inspected at full size and downsampled to 32 pixels, where the ears,
eyes and nose still read. `xprop -id <window> _NET_WM_ICON` reports
`Icon (256 x 256)`, so the window carries the new mark.

The tooltip, twice over. Offscreen, `bench /tmp/a.txt 1100 750 1 /tmp/tip.ppm
tooltip` drives the real widget through its real API -- rest on the Save button,
tick past the delay -- and the dump shows a dark box reading `Save` under the
third button. On screen, moving the pointer onto the Save button and waiting
750 ms produces the same label in a screenshot of the running window, with the
button's hover highlight behind it.

The tab strip: a screenshot at 1100x750 with three files shows three tabs, the
active one carrying the orange top bar and the pin and close buttons, and a
screenshot with one file shows the strip still present. Typing into the active
document puts a pencil on its tab and no asterisk in its title.

## Forty-third pass: the program is `npp` and the product is Neko++

> **Superseded in part.** The program is still `npp` and the product is still
> Neko++, but the crate names did change after all: they are `nn-app` and
> `nn-core` as of the forty-ninth pass. This section is kept as the record of
> the earlier decision, which is why it still says `npp-app` throughout.

The program was called `npp-app` and the product was called `npp-rs`. Neither
name was chosen; both were placeholders that survived long enough to be quoted
in a desktop entry, a man page, a Debian control file, a D-Bus name, a
configuration directory and about two hundred strings. This pass replaces them:
the binary is `npp`, the product is Neko++, and the two are not the same word on
purpose.

### Two names, and why not one

A program has two names because it is read by two audiences. `npp` is what a
shell types, what a desktop entry puts in `Exec=`, what a man page is filed
under, and what a package is called; it has to be short, lowercase, and free of
characters that need quoting. `Neko++` is what a person reads in a title bar, a
launcher, an About row and a `--version` line; it can carry a capital and two
pluses because nothing has to parse it.

The rule that follows from that is applied consistently:

| Surface | Name | Why |
| --- | --- | --- |
| Binary, package, man page, `Exec=`, `Icon=`, config directory | `npp` | typed or parsed |
| Window title, `--version`, About row, desktop `Name=` | `Neko++` | read |
| `usage:` line, error prefixes on stderr | `npp` | a shell prints them |
| Crate names `npp-app`, `npp-core` | unchanged | see below |

The crate names did not change. `npp` is the abbreviation of Neko Plus Plus, so
`npp-app` and `npp-core` are still accurate descriptions of what they are, and
renaming them would have meant editing the `use` statements of sixty files to
buy nothing a user can see. The rename stops at the boundary of the process.

### The binary target is declared, not discovered

`crates/npp-app/Cargo.toml` now lists both targets explicitly:

```toml
[[bin]]
name = "npp"
path = "src/main.rs"

[[bin]]
name = "bench"
path = "src/bin/bench.rs"
```

Autodiscovery names the first target after the package, which is exactly the
name being changed, and it would have kept working only by accident. Declaring
both means `cargo build --bin npp` is a promise the manifest makes rather than
something the directory layout happens to produce, and adding a third binary
later cannot silently rename this one.

### The configuration directory moved, and the old one is migrated

`~/.config/npp-rs/` became `~/.config/npp/`. The directory is named after the
program and not the product because a path is typed: `Neko++` in a path means
quoting it in every shell and escaping it in every desktop file.

Renaming the directory without more would have presented every existing user
with a fresh install -- theme, font size, key bindings, saved run commands,
session and unsaved buffers all gone, with nothing on screen to say why. So
`paths::migrate_legacy` renames the old directory into place, once, and
`paths::migrate` calls it against the real configuration directory as the first
statement of `main`, before `--help` and `--version` return, so a user who only
ever runs `npp --version` still gets their settings moved.

It is a `rename` and not a copy because the file names inside the directory did
not change; only the directory did. It refuses to act when the new directory
already exists, because that means the migration has run or the user has started
using the new name, and in both cases the old directory is the stale one. A
failure is ignored for the same reason every other write in `paths` is: a
read-only home directory should not stop the editor from starting, and the worst
case is the fresh install the migration existed to avoid.

Four tests cover it: the move itself, a second run doing nothing, an existing
new directory winning over the old one, and a fresh install having nothing to
migrate. The function takes the base directory as an argument rather than
reading `XDG_CONFIG_HOME`, so the tests do not have to mutate a process-global
environment variable that other tests are reading at the same time.

### Which `Notepad++` strings changed and which did not

There are 249 occurrences of `Notepad++` in the source and only a handful of
them name *this* program. The rest are statements about the program being
reproduced -- "Notepad++ draws a 4px top bar", "Notepad++'s `_tabStatus`
default", "the languages Notepad++ ships" -- and rewriting those would make the
documentation false. The distinction applied was: does the sentence describe
what *we* do, or what *they* do?

Changed:

- `About Notepad++` became `About Neko++`. This row is about this program.
- `Read-Only in Notepad++` became `Read-Only in Neko++`. The submenu sets a flag
  in this editor, not in Notepad++.
- The window title suffix, the `--version` line, the About line, the debug-info
  header, the `usage:` line, every `npp: ...` message on stderr, and the four
  file headers (`# Neko++ settings`, `# Neko++ session`, `# Neko++ snapshot`,
  `# Neko++ projects`).

Not changed, deliberately:

- `Notepad++ Home`, `Notepad++ Project Page`, `Notepad++ Online User Manual`,
  `Notepad++ Community (Forum)`. These four rows open
  `notepad-plus-plus.org`. Relabelling them `Neko++ ...` would point a reader at
  a site that does not exist, and the manual they open documents the features
  that are implemented here. They are references to the project this one
  reproduces, and they say so.
- `Import Notepad++ Config...`, `Notepad++ User Defined Languages Collection`,
  `not a Notepad++ localization file`, `Notepad++ config: nothing to import`,
  and the `Notepad++` directory name in the importer. All of these are about
  Notepad++'s files, not ours.

The cost of the two changed menu rows is that a Notepad++ localization no longer
translates them: `i18n` keys the first schema by English text, and `english.xml`
says `About Notepad++`. The row falls back to English rather than to a wrong
translation, which is the right failure. A translation written for this editor
can still reach it through the second schema, which is keyed by action name --
`<Item id="about" name="A propos de Neko++"/>`.

### The D-Bus name

`org.npp_rs.Editor` became `org.npp.Editor`, and the object path
`/org/npp_rs/Editor` became `/org/npp/Editor`. The old name carried an
underscore, which is legal in a D-Bus well-known name but unusual, and it
carried the old project name. Nothing outside the process refers to it: a second
launch asks the bus who owns the name, and a build that asks for a different
name simply does not find the other one and starts a second window, which is the
documented behaviour when no bus is present.

### `StartupWMClass` had to be measured, not guessed

The desktop entry's `StartupWMClass` has to match the `WM_CLASS` the window
actually gets, or the launcher shows a second icon instead of grouping the
window under the first. winit derives `WM_CLASS` from the executable name when
`WindowAttributes::name` is not set, so the value changed with the binary. It
was read back from the running window rather than assumed:

```
$ xwininfo -root -tree | grep Neko
0x2c00004 "b.txt * — Neko++": ("npp" "npp")  1100x750+10+45  +219+307
```

Both the instance and the class are `npp`, so `StartupWMClass=npp` is what the
desktop entry says. Setting `WindowAttributes::name` to `Neko++` would have made
the class `Neko++`, which is not what a window class is for.

### Verification

`cargo test --offline`: **856 npp-app + 802 npp-core = 1658 pass, 0 fail.** The
four extra tests are the migration ones.

`cargo clippy --offline --all-targets`: **0 warnings.**

`cargo build --offline --release --bin npp`: succeeds in 2m09s and produces
`target/release/npp`.

The command line:

```
$ ./target/release/npp --version
Neko++ 0.1.0
$ ./target/release/npp --help
usage: npp [switches] [file ...]
```

The migration, on the real configuration directory: `~/.config/npp-rs/` held
`settings.txt`, `session.txt`, `snapshot.txt`, `snapshot/` and `plugins/`. After
the first run of the new binary, `~/.config/npp/` holds all five and
`~/.config/npp-rs/` is gone. The settings file still begins `# npp-rs settings`
because the old binary wrote it; the next save rewrites the header.

The window: `xwininfo` reports the title `b.txt * — Neko++` and the class
`("npp" "npp")`. A screenshot of the running window shows the Help menu with
`About Neko++` as its last row and the four `Notepad++ ...` reference rows above
it, and the Edit menu with `Read-Only in Neko++` as its last submenu.

The packages, rebuilt from scratch:

```
dist/npp-x86_64.AppImage      6.5 MB
dist/npp_0.1.0_amd64.deb      4.4 MB
```

`dpkg-deb -c` lists `usr/bin/npp`, `usr/share/applications/npp.desktop`, the
three icons under `hicolor/{128x128,256x256,scalable}/apps/npp.*` and
`usr/share/man/man1/npp.1.gz`; `dpkg-deb -f` reports `Package: npp` and a
description that begins `Neko++ is a text editor`. Extracting the AppImage gives
`usr/bin/npp`, `npp.desktop` with `Name=Neko++` and `Exec=npp %F`, and an
`AppRun` that execs `usr/bin/npp`. The AppImage's `npp.png` is byte-identical to
`packaging/npp-256.png`, and that PNG is embedded in the binary, so the
launcher, the window and the package all show the same kitten.

One thing `strings` cannot confirm and a screenshot can: `About Neko++` does not
appear as a contiguous run in the binary. The compiler inlined the twelve-byte
literal as two immediates, `mov rax, "About Ne"` and `mov dword [r14+8],
"ko++"`, so the bytes are there but never adjacent. The rendered menu is the
evidence, not the object file.

## Forty-fourth pass: the text that vanished, and the furniture that grew

Two reports, one cause each, and neither cause was where the symptom pointed.
The first was "issues display disappear when zooming a lot" -- the whole window
going blank, menu bar and status bar included, with no error and no crash. The
second was the menu bar growing to three times its height while its label stayed
fifteen pixels tall. Both are fixed, and both fixes are one line plus the
reasoning that makes the line non-obvious.

### The atlas was never trimmed, so nothing could ever be evicted

`glyphon`'s `TextAtlas` is an LRU packer over one GPU texture. `prepare` walks
the glyphs it is about to draw and marks each one live in `glyphs_in_use`;
`try_allocate` may only evict a glyph that is *not* live. `TextAtlas::trim()`
is what clears `glyphs_in_use` between frames.

We never called `trim()`. So every glyph ever drawn stayed marked live forever,
the packer could never reclaim a single pixel, and the atlas grew until it hit
the device's maximum texture size. After that every `prepare` returned
`AtlasFull`, and the renderer's response to `AtlasFull` is to drop the entire
text pass -- not the offending glyph, the pass. Menu bar, document, status bar,
all of it, in one frame, with nothing on screen to say why.

Zoom is what made it reachable. Each font size rasterises its own copy of every
glyph, so stepping from 15px to 40px one notch at a time caches twenty-six
complete alphabets. A user who never zooms might run for hours before the atlas
filled; a user who holds `Ctrl` and rolls the wheel fills it in seconds. That is
why the report was about zoom and not about long sessions.

The fix is `self.atlas.trim();` as the last statement of `render()`:

```rust
// Called once, after both passes, rather than between them: the second
// pass draws dropdowns and modals, and a trim in the middle would let the
// second `prepare` evict a glyph the first pass had just queued.
self.atlas.trim();
```

After both passes, not between them. The second pass draws dropdowns, tooltips
and modals, and a trim in the middle would let the second `prepare` evict a
glyph the first pass had just queued -- reintroducing the same class of bug in a
narrower window.

### The chrome had two heights and only one of them was fixed

The menu bar, toolbar, tab strip and status bar are shaped once, at startup,
with `Metrics::new(15.0, 20.0)`. Nothing ever reshapes them. But the renderer
was computing their *heights* from `panes[0].view.line_height()` -- the
document's line height -- so at 40px the menu bar was 59 pixels tall with a
15-pixel label pinned to its top edge, and the toolbar's icons were sized from
the document's line height too.

Notepad++'s zoom affects the document only. That is not an arbitrary choice: a
menu bar that grew with the font would push the text off the bottom of the
window at 40px, and the user would have zoomed in to read the text and lost the
space to read it in.

So the two heights are now named and separated:

```rust
pub const UI_FONT_SIZE: f32 = 15.0;
pub const UI_LINE_HEIGHT: f32 = 20.0;
```

`render()` binds `let ui = UI_LINE_HEIGHT;` once and every chrome height, quad,
text area, dropdown area and `toolbar.icon_size` uses `ui`. `main.rs` constructs
all twenty-one chrome widgets with `Metrics::new(UI_FONT_SIZE, UI_LINE_HEIGHT)`
rather than repeating the numbers, so the renderer's constant and the widgets'
metrics cannot drift apart. Hit-testing moved to `UI_LINE_HEIGHT` at the
seventeen sites that test chrome geometry.

Three groups were deliberately left on the document's line height, because they
are document furniture and not window furniture: the completion popup's offset
(3176), the parameter hint's offset (3285), and the scroll deltas (4191-4224).
A completion popup that did not move with the text it completes would point at
the wrong line.

### Two defects found while verifying the above

**The find bar was drawn at the wrong height when the change-history panel was
open.** `find.push_quads` computed its y as
`height - status_height - results_height - bar_height` while `find.text_areas`
used `height - status_height - history_height - results_height - bar_height`.
`panes_height` subtracts all four, so the `text_areas` version is the correct
one and the quads were floating one history-panel too low -- the bar's
background and its text were in different places. `push_quads` now matches.

**The change-history panel could never be opened.** `toggle_change_history`
called `refresh_history`, which begins with `if !self.history.is_open() {
return; }` -- and `set_rows` is what sets `open = true`. The guard and the
opener were each waiting for the other. `Ctrl+Alt+H` therefore did nothing at
all, and the panel was only reachable if something else had already opened it,
which nothing did. The row-building is now `fn change_rows(&self) ->
Vec<ChangeRow>`, and `toggle_change_history` calls
`self.history.set_rows(self.change_rows())` directly. `refresh_history` keeps
its guard, because it is called after every edit and must not resurrect a panel
the user closed.

A third, cosmetic: `toggle_full_screen` had lost its doc comment to
`toggle_change_history` during an earlier edit, leaving a two-line doc block
whose first line described a different function. Restored, with the reason
borderless is used rather than exclusive.

### `Ctrl+0` was unreachable on any layout that shifts its digits

Found while resetting the zoom after the verification below, and worth its own
section because the bug is invisible on QWERTY.

`Chord::from_char` kept Shift for alphanumerics and dropped it for punctuation,
on the reasoning that `Ctrl++` arrives as `Ctrl+Shift++` wherever `+` needs
shift. Digits were filed with the letters. On AZERTY -- and on every layout
where the digits sit on the shifted level -- the only way to produce a `0` is
`Shift+0`, so the only chord a user can press for `Restore Default Zoom` is
`Ctrl+Shift+0`. The binding was `Ctrl+0`. The shortcut was dead, and nothing
said so.

Digits now fold with the punctuation:

```rust
shift: shift && c.is_ascii_alphabetic(),
```

Nothing is lost by it. `Ctrl+1` and `Ctrl+Shift+1` are the same chord
afterwards, and no binding in the default keymap distinguishes them -- `Ctrl+0`
is the only digit binding there is. Letters are untouched, so `Ctrl+U` and
`Ctrl+Shift+U` remain different commands.

### The same keypress typed a space into the document

The four `Ctrl+Shift+0` presses that failed to reset the zoom each inserted one
space at the caret. The fall-through at the bottom of `handle_key` guarded text
insertion with

```rust
Key::Character(_) if ctrl && !alt => {}
```

which assumes a Ctrl chord always arrives as a character. It does not. A Ctrl
chord whose control character collides with a named key is reported by winit as
that key, and `Ctrl+Shift+0` came through as a space -- so it missed the guard,
reached the `_` arm, and `event.text` was inserted.

The guard is now on the wildcard:

```rust
_ if ctrl && !alt => {}
```

Every Ctrl chord that means something has already been handled above it or
found in the keymap, so anything reaching that arm is noise. AltGr still types,
because it arrives as Ctrl+Alt and the guard excludes Alt.

### The viewport could scroll past the end of the document

Found by looking at a restored session: lines 141 to 173 of a 173-line file,
with the last line at the top of the pane and a screenful of empty space below
it. The document was not short; the viewport was simply scrolled past its end.

`scroll_pixels`, `scroll_lines` and `ensure_line_visible` each clamped
`first_line` to `total_lines - 1`. That is the clamp for "the last line may sit
at the top", which is only correct when the pane is one line tall. For a pane of
`n` lines the furthest legal scroll is `total_lines - n`, and `scroll_to_bottom`
already knew that. The other three did not.

The clamp is now one helper, so the four call sites cannot disagree again:

```rust
fn max_first_line(&self, total_lines: usize) -> usize {
    total_lines.saturating_sub(self.full_line_count())
}
```

`saturating_sub` is the whole point: a document shorter than the pane yields 0,
so a short file cannot be scrolled at all, which is what a user expects and what
the old code got wrong in the other direction.

The restored session is how the bug became reachable. The viewport is restored
before the window has a height, so `full_line_count()` is 1 at the moment the
scroll is applied; a clamp of `total_lines - 1` then accepts a scroll that the
real pane height would have refused, and nothing re-clamps it once the height
arrives. `ensure_line_visible` now pulls such a stale scroll back into the
document on the first frame that has a real height.

Two tests changed rather than being added, because they had encoded the bug:
`scroll_pixels_clamps_at_both_ends` and `scroll_lines_clamps` both expected 99
for a 100-line document in a 30-line pane. They now expect 70. Two were added:
`a_short_document_cannot_scroll_at_all` and
`ensure_line_visible_pulls_a_stale_scroll_back_into_the_document`.

### `Ctrl+Home` and `Ctrl+End` were implemented everywhere but the keyboard

`Motion::DocStart` and `Motion::DocEnd` existed in the core, were handled by the
motion code, and had tests. Nothing on the keyboard produced them. The plain
`Home` and `End` arms in `handle_key` matched first and there was no `ctrl`
variant, so the two most-used navigation shortcuts in the program did nothing.

```rust
Key::Named(NamedKey::Home) if ctrl => Some(Motion::DocStart),
Key::Named(NamedKey::End) if ctrl => Some(Motion::DocEnd),
```

Placed before the plain arms, because match arms are ordered and the plain ones
would otherwise swallow the chord. This is the third defect of the same shape
this session -- a capability that exists, is tested, and is unreachable -- which
is worth noting as a pattern: unit tests on the core prove the core, not the
wiring, and the wiring is where the user actually stands.

### Verification

`cargo test --offline`: 857 in `nn-app` plus 804 in `nn-core`, 1661 passing,
0 failing. `cargo clippy --offline --all-targets`: 0 warnings.

On screen, at 40px after twenty-five zoom notches: `grep -c AtlasFull
/tmp/app.log` returns 0, and the menu bar, document and status bar are all
present. The menu bar is 20 pixels tall with a 15-pixel label centred in it, and
stays 20 pixels at 8px as well. The File and Edit dropdowns open with correct
row geometry at 40px; hovering "Reload from Disk" highlights exactly the row
under the pointer; clicking "Select All" reports `Sel: 8493`; the toolbar
tooltip "Print" appears under the printer icon. With the change-history panel
open, the find bar sits above the history panel, which sits above the status
bar. `Ctrl+Alt+H` shows "1 change" and one row.

For the shortcut: six zoom notches in, one `Ctrl+Shift+0` returns the document
to 15px and leaves the status bar reading `length: 8493` -- before the fix the
same keypress left the zoom alone and the length at 8497. `Ctrl+Shift+5`, which
is bound to nothing, also leaves the length at 8493, so the wildcard guard
swallows unbound Ctrl chords without eating anything else.

For the viewport: the restored session now shows lines 141 to 173 with line 173
at the bottom of the pane and no blank space beneath it. `Ctrl+End` reports
`Ln: 173, Col: 1` and `Ctrl+Home` reports `Ln: 1, Col: 1`, both from the same
8493-character buffer, so the two motions reach the true ends of the document
rather than the ends of the visible region.

## Forty-fifth pass: the copy of the document that stayed behind

Found by accident, while checking that the previous pass had not damaged the
buffer it was tested against. It had not -- but the snapshot directory held a
copy of a document that had been clean for an hour, and nothing would ever have
deleted it.

### Buffer files are named by position, and positions move

`Snapshot::save` wrote the manifest and stopped there. The buffer files
themselves were named `0.txt`, `1.txt`, `2.txt` by their index in the list of
*dirty* buffers, and `snapshot_now` only ever writes the dirty ones -- a clean
buffer is already on disk and copying it would double the snapshot for nothing.

Those two facts combine badly. A buffer that becomes clean -- saved, or reloaded
from disk -- drops out of the list, and every buffer after it shifts down one
index. The file that used to hold the last buffer is then named by nothing. It
is never read again, so nothing breaks; it simply stays in
`~/.config/npp/snapshot/` forever, holding a copy of a document the user
reasonably believes exists only where they put it.

That is the part worth fixing. A stale file that is merely wasted space is
housekeeping. A stale file that is a copy of the user's text, in a configuration
directory, with no code path that will ever remove it, is a different thing.

`save` now prunes after it writes:

```rust
crate::paths::write(&path, &self.render());
if let Some(dir) = Self::dir() {
    let keep: Vec<&str> = self.entries.iter().map(|e| e.file.as_str()).collect();
    let _ = Self::prune_in(&dir, &keep);
}
```

`prune_in` takes the directory and the names to keep, so it can be tested
against a temporary directory rather than the user's real one -- the same reason
`write_buffer_in` exists. It considers only `.txt`, so an unrelated file that
happens to be in the directory is left alone rather than deleted on a guess, and
a missing directory is an error rather than a panic, because the first save on a
fresh machine runs before the directory exists.

### How it was found, and what it says about the test

The buffer under test was a 8485-character shell script. After the previous
pass's verification it read 8499 characters, and the extra fourteen were
`\_9999` followed by eight spaces, sitting at the very start of line 1.

They were mine. `Ctrl+Home` had just been verified, which leaves the caret at
character zero, and the keystrokes that followed went into the document instead
of wherever they were meant to go. The file on disk was untouched, so nothing
was lost, and `Ctrl+Alt+R` -- reload from disk, which discards unsaved changes
without asking -- put the buffer back to 8485 characters in one keystroke.

The lesson is not "be careful with keystrokes". It is that a test which leaves
the caret at the start of a real document and then types is a test that can
corrupt the thing it is measuring, and the only reason this was caught is that
the snapshot is a plain file that can be diffed against the original. A test
that types into a scratch buffer would have been safer; a test that types into
the user's file and then checks a character count would have reported success
either way, because the count it checked was taken before the typing.

### Verification

`cargo test --offline`: 860 in `nn-app` plus 804 in `nn-core`, 1664 passing,
0 failing. `cargo clippy --offline --all-targets`: 0 warnings. Three tests are
new: `pruning_removes_the_buffers_the_manifest_no_longer_names`,
`pruning_leaves_files_of_other_kinds_alone`, and
`pruning_a_directory_that_is_not_there_is_an_error_not_a_panic`.

On disk: before the fix the snapshot directory held `0.txt` (12 bytes, the
genuinely dirty buffer) and `1.txt` (8511 bytes, the clean one). After one
autosave tick with the rebuilt binary, `1.txt` is gone, `0.txt` remains, and the
manifest names only `0.txt`. The window reopened with
`publish-release.sh — Neko++` -- no dirty marker -- at `length: 8485`, and the
status bar reported `Restored 1 unsaved buffer(s) from the last session`, which
is the one buffer that really was unsaved.

## Forty-sixth pass: the frame that was dropped and never asked for again

The question that started this was "would it be a refresh issue?", asked about
the disappearing display. The answer for that bug was no -- the atlas was full
and the text pass was dropped, which the log said in as many words. But the
question was worth asking, because there was a refresh bug one layer down, in
the code that decides whether to draw a frame at all.

### Every failure to acquire a frame returned without asking for another

`State::render` began with

```rust
let frame = match self.gpu.surface.get_current_texture() {
    wgpu::CurrentSurfaceTexture::Success(f) => f,
    wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
        self.gpu.surface.configure(&self.gpu.device, &self.gpu.config);
        return;
    }
    _ => return,
};
```

Three separate defects in five lines, and all three have the same shape: the
frame is dropped and nothing asks for another one. A redraw is only ever
delivered because something requested it, so a dropped frame with no request
leaves the window showing whatever was last presented until an unrelated event
-- a mouse move, a keystroke, a timer -- happens to arrive. To a user that is
not "a frame was skipped". It is the window freezing, or going blank, and then
recovering for no reason they can see.

**`Outdated` configured the surface and did not retry.** wgpu's documentation
for this variant says "call `Surface::configure()` and try again". We did the
first half. The configuration is now followed by `request_redraw()`.

**`Lost` was treated as `Outdated`.** It is not. The documentation is explicit:
"call `Instance::create_surface()` to recreate the surface, then
`Surface::configure()`, and try again." A lost surface cannot be configured back
into existence -- the object itself is gone. We were calling `configure` on a
dead surface and returning, which is why a lost surface would have left the
window blank for the rest of the session with no error anywhere.

Fixing it needed two fields that `Gpu` had been throwing away: the `Instance` the
surface came from, and the `Arc<Window>` it draws into. Both are now kept, and
`Gpu::recover` does what the documentation says -- create, renegotiate the
format, configure -- returning whether the window can be drawn into again. The
format is renegotiated rather than assumed because a window dragged to another
output can land on a different driver path with a different preferred format.

**`Timeout` fell into the wildcard.** The documentation says "skip the current
frame and try again later". Skipping was right; never trying again was not. It
now requests a redraw.

The wildcard is gone entirely. Every variant is named, so a future wgpu release
that adds one produces a compile error here rather than silently inheriting
"drop the frame and stop". `Occluded` is the one variant that must *not* retry --
the window is not visible, and a redraw would spin the event loop for nothing --
so it returns deliberately, with a comment saying so. `Validation` is reported
once and dropped, because retrying a validation error just repeats it.

### Why this was invisible until asked about

None of these paths is reachable on a healthy desktop. `Outdated` needs a resize
the driver notices before we do; `Lost` needs a driver reset, a GPU hotplug, or a
compositor restart; `Timeout` needs a driver that misses a frame deadline. The
app had been running for hours without hitting any of them, which is exactly why
the code was never exercised and why the bug survived.

That is the argument for naming every variant rather than writing `_ => return`.
The wildcard was not wrong about what to do with an unknown variant; it was
wrong about how much attention an unknown variant deserved.

### Verification

`cargo test --offline`: 860 in `nn-app` plus 804 in `nn-core`, 1664 passing,
0 failing. `cargo clippy --offline --all-targets`: 0 warnings. The acquire path
has no unit test and cannot easily have one -- it needs a real surface and a
driver willing to lose it -- so the evidence is on screen.

Three resizes in a row (900x600, 1250x820, back to 1100x750) each redrew
completely, with no error in the log. Twenty-five zoom notches to 40px still
produce `AtlasFull` count 0, and `Ctrl+Shift+0` returns the document to 15px.
The window reopened at `length: 8485` with `Restored 1 unsaved buffer(s) from the
last session`.

The `Lost` path itself was not exercised: forcing a driver to lose a surface on
demand is not something this machine can do. It is written to the documented
contract and reviewed against it, and that is the honest limit of the evidence.

## Forty-seventh pass: the theme the desktop chose

The parity matrix's Linux row lists three portals: file, print, and dark mode.
The first two landed in the thirty-ninth and thirty-sixth passes. The third was
the one still missing, and it is the one a user notices without opening a menu:
their desktop goes dark at sunset, and the editor stays the colour it was.

### Three answers, not two

`Settings` had `light: bool`. That is two answers to a question with three: the
user can want the light palette, the dark palette, or whatever the desktop is
doing. A boolean cannot hold the third, and the settings file has to record
which one the user meant -- "follow the desktop" and "dark" are the same thing
until the desktop changes its mind.

So `light: bool` became `theme: ThemeChoice`, with `System`, `Light` and `Dark`.
The file writes `theme system`, `theme light` or `theme dark`. The last two are
the keywords the file already used, so an existing settings file keeps working
and keeps meaning what it meant: a user who chose dark stays dark, and does not
silently start following a desktop they never asked to follow.

The default is `System`. On a machine with no portal -- a bare X session, a
container, a CI box -- the answer is `None`, and `None` resolves to dark, which
is the palette this editor has always opened with. The machines that cannot
answer look exactly as they did before.

### The portal says "no preference", and that is not "light"

`org.freedesktop.appearance` / `color-scheme` is a `u32`: 0 no preference, 1
prefer dark, 2 prefer light. The third value is the one that matters. A desktop
that has not been told what to do is not asking for a light editor; it is
declining to answer, and the editor should keep the palette it had.

Anything the spec does not define is treated the same way. A future value is a
preference this build does not know how to honour, and guessing at it would be
worse than leaving the editor alone.

### The read is bounded

The scheme is read once, before the settings are applied, so the first frame is
already the right palette rather than the dark one for a frame or two. That
places a D-Bus round trip in the startup path, which is the one place this
project has a budget: 250 ms to first frame.

Measured, the call is 1.7 ms at the median and 3.0 ms at the worst of twenty on
this machine, thread spawn included. But a portal that is wedged would not
return at all, and zbus's blocking proxy has no per-call deadline. So the call
runs on its own thread and the caller waits at most 200 ms for it. A wrong
palette is a much smaller problem than an editor that does not start.

### The variant inside the variant

The first version read the answer as

```rust
let value: OwnedValue = proxy.call("Read", &(APPEARANCE, COLOR_SCHEME)).ok()?;
let value = u32::try_from(value).ok()?;
```

and returned `None` on a machine where `gdbus` answered `(<<uint32 2>>,)` in as
many words. `OwnedValue`'s signature is `v`, so deserializing a `v` into it
gives a value that is still inside a variant -- `Value::Value(Box<Value::U32>)`
-- and `u32::try_from` on that is `IncorrectType`, not 2. The debug print is
what gave it away: `OwnedValue(Value(U32(2)))` has one more `Value` in it than
a `U32` should.

`downcast_ref::<u32>()` steps through the wrapper. The same mistake was in the
signal handler, where the third argument of `SettingChanged` is a variant for
the same reason, so both call sites now unwrap the same way.

This is the kind of bug a unit test cannot catch: the pure mapping from `u32` to
`Scheme` was right the whole time, and the failure was in the shape of a value
that only exists on a live bus. It was found by running the editor and looking
at the window, which is the only place it could have been found.

### The watch is a match rule, not a poll

Following the desktop means hearing about changes, and the portal announces them
with `SettingChanged(ssv)`. The subscription is a match rule on the bus with
`arg0` and `arg1` filtered to the namespace and the key, so a desktop changing
its font size does not wake the thread.

A match rule rather than a call on the portal's name, because a rule survives
the portal restarting: the signal comes from whoever owns the name at the time,
and the bus delivers it to every rule that matches. The thread runs for the life
of the process and is ended by the process exiting; there is nothing to shut
down.

A signal that cannot be read is skipped rather than reported. The alternative is
a thread that dies on the first malformed message and leaves the editor
following a preference that changed ten minutes ago.

### An explicit choice is a decision

`scheme_changed` updates `system_light` whatever the setting says, and then
returns early unless the setting is `System`. A user who picked Light picked it
for a reason, and a desktop that changes its mind at sunset does not get to
overrule them. The next time they open Preferences the row still says Light.

The Notepad++ import writes an explicit choice too. `config.xml` says "dark mode
was on", which is a palette, not a policy; importing it sets `Dark` rather than
`System`. That is also why the import test's change count went from three to
four: the default is now `System`, so a file that names a palette changes it.

A `stylers.xml` theme still wins over both. `scheme_changed` re-applies the
saved theme after rebuilding the palette, in the same order `apply_settings`
does, so a user with an imported theme keeps it when the desktop flips.

### The preferences row steps, it does not toggle

`Row::Theme` was a switch, and a switch has an off position. With three values
there is no off position, so the row steps: System, Light, Dark, and back to
System. `is_switch` no longer lists it, which is the only thing that changed
about how the dialog draws it -- the value column already showed a word.

### Verification

`cargo test --offline`: 883 in `nn-app` plus 809 in `nn-core`, 1692 passing,
0 failing. `cargo clippy --offline --all-targets`: 0 warnings.

The live run is the evidence that matters, because the bug above was invisible
to the tests. With `theme system` in the settings file and the desktop on
`prefer-light`, the editor opened light. `gsettings set
org.gnome.desktop.interface color-scheme prefer-dark` turned the window dark
within a frame, with no restart and no error in the log; setting it back to
`prefer-light` turned it light again. With `theme dark` in the file and the
desktop on `prefer-light`, the editor stayed dark, which is the explicit choice
being honoured.

The read costs 1.7 ms at the median over twenty calls. Frame times on the
20 000-line document at 1080p are unchanged: idle 0.66 ms, scroll 0.71 ms, type
2.65 ms at p99, against 0.67 / 0.75 / 2.21 in the second pass's table.

The settings file was put back to `theme dark` afterwards, and the desktop's
colour scheme to `prefer-dark`, so the machine this was tested on is exactly as
it was found.

## Forty-eighth pass: the address you can click

The About box has shown the author's LinkedIn address since it was written, and
the address has never been anything but text. A user who wants to look the
author up has to read forty-nine characters off a dialog and type them into a
browser, which is a transcription task nobody should be given.

### The address is a constant, the label is not part of it

`lines()` built the line with a `format!` and the address inline. The click
handler needs the address without holding a borrow on the box, and the box needs
to know which line carries it, so the address became `LINK_URL` and the label
`LINK_PREFIX`, with `link_url()` as the one public way to ask for it.

The line is found by prefix rather than by index. The address is the sixth line
today; a line added above it would leave an index-based hit test pointing at
whatever now sits in that position, and the bug would be a click that opens a
browser from a line about the version number. `link_line()` searches for the
prefix, so the target moves with the text.

The click target is the address alone, not the whole line. "LinkedIn: " is a
label, and a click on a label that opens a browser is a click the user did not
aim. `link_rect` starts after the prefix and is as wide as the address, measured
in characters because the family is monospaced -- the same measurement `width()`
already uses to size the box.

### A colour of its own

The address is drawn in a new `Theme::link` slot rather than in `icon_accent`,
which happens to be the same blue today. The accent is chosen to read against a
toolbar and the link has to read against the menu background; a theme that
retunes one should not silently retune the other. Dark gets `96, 165, 232` and
light `28, 108, 196`, which are the two blues the icons already use, so the
palette gains a name rather than a colour.

The body is shaped as rich text rather than as one run. `set_rich_text` with the
spans cut on the prefix keeps the label in the body colour and tints only the
address; colouring the whole line would make "LinkedIn: " look clickable too.
The alternative -- a second buffer positioned over the first -- would have meant
blanking the address out of the body text and keeping two shapes in step through
every resize, which is more moving parts for the same picture.

The colours are part of the shaping key, not a separate flag. The frame is drawn
from the theme on every frame, so a desktop that flips its colour scheme while
the box is open would otherwise leave a body shaped against the old palette --
dark text on a light box. `shaped` became `Option<(Rgb, Rgb)>` holding the two
colours it was shaped with, which is the same trick the completion popup uses for
its active row.

A one-pixel rule goes under the address, in the link colour, two pixels above the
bottom of the line box. Colour alone is a weak signal on a background that is
already tinted, and an underline is what every browser and every dialog has
taught people a link looks like.

### The click, and the cursor

`MouseInput` had one line for the About box: any click closes it. It now asks
`link_hit` first. A click on the address calls `open_page`, which is the same
`desktop::open_url` the Help menu's rows use, so a machine with no browser gets
the same status-bar message it gets from those rows. The box stays open, the way
a dialog's link does everywhere else: the browser comes to the front, and the
box is still there when the user comes back to it. Any other click still
dismisses.

The pointer becomes a hand over the address. This is the first cursor change in
the program -- everything else is text, where the platform's arrow is what a
window that never asks gets -- so it is kept to one place: `handle_cursor` asks
the box before its early returns, and `link_hover` records the answer so
`set_cursor` is called only when the pointer crosses the link's edge rather than
once per motion event. `CursorLeft` clears it, because no further move arrives
once the pointer has left the window, and `close_about` clears it too, because a
box dismissed with Escape while the pointer sits on the link would otherwise
leave a hand cursor over a text editor.

### What is not there

The link is mouse-only. The box has no focus model -- every key dismisses it, and
Enter is the Close button, which is what a dialog's default button does -- so
there is nothing to Tab to. The box is also not in the accessibility tree at all,
which is a gap that predates this pass and is not made worse by it: a screen
reader user could not read the address before and cannot click it now. Both are
worth a pass of their own; neither is a reason to leave the address unclickable
for everyone else.

### The rubbish in /tmp

Not part of the feature, but found while testing it. `/tmp` had grown to 3.8 GB
and 5145 entries, 3515 of them `npp-*` directories left by test runs, and the
screenshots from every pass were still sitting there. All of it was deleted --
the test directories, the screenshots, the 100 MB file from the large-file pass,
the AppImage and deb extractions, the Notepad++ source checkout -- leaving the
other projects' directories and the live session directories alone.

The directories were coming back at 116 a run, so the leak was fixed at the
source for the worst of them. `workspace.rs`'s `Scratch` claimed to delete itself
and removed only the scanned root, leaving the empty
`npp-workspace-<name>-<pid>` parent behind; it now holds both paths and removes
both. A run leaves 34 directories instead of 116. The rest are the same shape --
`temp_dir(name) -> PathBuf` helpers in `snapshot.rs`, `backup.rs` and
`browser.rs` that return a bare path with nothing to drop -- and fixing them
means a guard type with a `Deref` in each module, which is a sweep of its own
rather than a line in this pass.

### Verification

`cargo test --offline`: 892 in `nn-app` plus 809 in `nn-core`, 1701 passing,
0 failing. `cargo clippy --offline --all-targets`: 0 warnings. Nine new tests in
`about.rs` cover the line being found by prefix, the rect covering the address
and not the label, the first and last pixel of the target being on it, a click on
the label, a click on the line above, a click past the end of the address, a
closed box having no link, and the target following the box when the window
widens.

Driven live, which this machine has never managed before: there is no `xdotool`
and no `python-xlib` here, but `libXtst` is installed, so the X test extension is
reachable through `ctypes` and a throwaway script can move the pointer and click.
Help -> About Neko++ drew the address in blue with the rule under it and the
label in the body colour. A click at the middle of the address opened Chromium --
the default handler -- on `linkedin.com/in/stephane-urdy-05b2729`, with the tab
titled "Stephane Urdy | LinkedIn", and the About box was still open behind it.
The renderer count went from 11 to 13, which is the new tab. The same box was
checked with `theme light` in the settings file: the address is the darker blue
and still reads as a link against the light background.

The settings file was put back to `theme dark` afterwards, the test file and the
screenshots were deleted, and no `npp` process was left running.

## Forty-ninth pass: the crates are `nn`, the program is still `npp`

The crates were called `npp-core` and `npp-app`. That left the tree with two
names one suffix apart and no rule saying which was which: `npp` is the program
-- the file in `target/release`, the word after `Exec=`, the package, the man
page -- while `npp-app` is the crate that builds it. A reader meeting `npp-app`
in a `use` statement, a build log or a test count had to work out which of the
two was meant, and the name did not say.

The workspace is `nn` now: `crates/nn-core` and `crates/nn-app`. The program
keeps the name it already had, `npp`, and everything a user can type, read or
parse is unchanged by this pass.

### Two names for two jobs

| Surface | Name | Why |
| --- | --- | --- |
| `target/release/…`, `Exec=`, package, man page, config directory, D-Bus name | `npp` | typed or parsed by the system |
| Window title, `--version`, About row, desktop `Name=` | `Neko++` | read by a person |
| Crates `nn-core`, `nn-app`, workspace root | `nn` | read by whoever works on the tree |

The third row is the only thing this pass changes. It is the same argument the
forty-third pass made for splitting the program name from the product name,
applied one level further in: the thing you build from and the thing you run are
different audiences, so they get different words. `nn` also matches the
directory the tree lives in, which is what a reader types first.

### What the rename cost

One `sed` over the `use` statements in twenty-three files, two directory
renames, the workspace member list, and a full rebuild -- a crate name is part of
the metadata hash cargo keys artifacts on, so nothing in `target/release` survived.
The forty-third pass rejected this work as "editing the `use` statements of
sixty files to buy nothing a user can see". That is still true, and it is the
point: the purchase is legibility for the person reading the tree, and the price
was an afternoon of mostly mechanical edits and four minutes of build.

Two things were deliberately not touched. The binary target stays `npp`, because
the desktop entry, the man page, the packages and the shell all said `npp` and
none of them is improved by a rename. The configuration directory stays
`~/.config/npp/` for the same reason, and because no build ever shipped with a
different one: `paths::migrate_legacy` still knows only `npp-rs`, the name the
program had before the forty-third pass.

The strings that name Notepad++ did not move either. `$(NPP_DIRECTORY)` is
Notepad++'s own macro -- renaming it would break every run command written for
Notepad++ -- the plugins' host module is still `npp` with its `npp_name`,
`npp_command_count`, `npp_command_name` and `npp_run` exports, and
`nppimport.rs` is still the module that reads Notepad++'s XML, which is what the
`npp` in its name has always meant.

### Verification

`cargo build --release`: 0 warnings, and `target/release/npp` is 15 MB, with
`bench` beside it. `./target/release/npp --version` prints `Neko++ 0.1.0` and
`--help` still prints `usage: npp [switches] [file ...]`, which is what says the
rename stopped at the workspace boundary.

`cargo test --offline --release`: 892 in `nn-app` plus 809 in `nn-core`, 1701
passing, 0 failing -- the same 1701 as the pass before. A mechanical rename that
changes the count has broken something; this one did not move it.

`cargo clippy --offline --release --all-targets`: 0 warnings.

## Fiftieth pass: twenty-one themes, and a hundred and forty-seven languages

Two tables grew. Neither is code in the sense the rest of the tree is: one is a
list of colour pairs, the other a list of keywords, and the work was in deciding
what shape each list takes before writing it out.

### Themes: ten colours, sixty derived

`Theme` carries sixty-odd colours because every widget needs one. Writing a
constant per theme would have been sixty lines of near-identical hex per entry,
and the seventeen new palettes would have drifted apart at the first edit --
someone fixes the tooltip in Tokyo Night and forgets Nord.

So a theme in the catalogue is a `Palette`: background, text, gutter, selection,
caret, one accent, and the nine syntax styles. Ten decisions. `Theme::from_palette`
derives the chrome from them with one rule -- every neutral is `t` of the way from
the background towards the text -- which makes a light palette darken and a dark
one lighten from the same expression. Three colours are deliberately not derived:
the orange rule on the active tab, the pencil on an unsaved one, and the orange
tab bar underneath both. Those are Notepad++'s, and the tab bar is the one place
a user recognises the editor by its colours rather than reading it.

The values are the published ones, not approximations. Solarized's base03 is
`#002B36`, Tokyo Night's background is `#1A1B26`, Dracula's foreground is
`#F8F8F2`. A theme named after a palette that does not match it is worse than no
theme, because the only reason to pick one by name is that you already know what
it looks like.

Eighteen palettes join the two hand-written ones: Catppuccin Latte and Mocha,
Dracula, Everforest, GitHub Dark and Light, Gruvbox Dark and Light, Kanagawa,
Monokai, Night Owl, Nord, One Dark and Light, Rose Pine, Solarized Dark and
Light, Tokyo Night. Twenty choices plus `System`.

### Where you choose one

The Preferences row steps through all twenty-one with the arrows, which is fine
for comparing two and hopeless for finding Tokyo Night in a list you cannot see.
So Settings gained a `Theme` submenu, one row per theme, built from the same
`ThemeChoice::ALL` the row steps through.

Each row is its own action, `Action::Theme(index)`, addressed as `theme_<key>` --
`theme_catppuccin_latte`, `theme_solarized_dark`. That costs two match arms in
`menu.rs` and buys a shortcut for any theme and a test that walks all twenty-one
keys, which is how the row and the list are kept from disagreeing.

Two colours are checked by test rather than by eye. Every palette's text clears a
4.0 contrast ratio against its own background, and no syntax colour sits within
forty of the background's brightness. The 4.0 rather than WCAG AA's 4.5 is
Solarized Light's doing: base0 on base3 is 4.13 by design, and darkening it to
pass would make it a different theme with the same name. The test pins that
count at one, so a second dim palette cannot slip in unnoticed.

### Languages: ninety-three to a hundred and forty-seven

The table already described each language by four things: how a line comment
starts, how a block comment opens and closes, which characters quote a string,
and which words are keywords. That model does not change; it just gets used
fifty-four more times.

Added: APL, AsciiDoc, Astro, AWK, BibTeX, Blade, Clojure, Crystal, CUDA, Dart,
Dockerfile, Elixir, Elm, F#, Fish, Gleam, GLSL, GraphQL, Groovy, HCL, HLSL,
Jinja, Julia, Kotlin, Less, Markdown, Meson, Mustache, MySQL, Ninja, Nix,
Nushell, Org, PostgreSQL, Prolog, Protocol Buffers, Puppet, PureScript, Racket,
reStructuredText, Sass, SCSS, Sed, Solidity, Svelte, SystemVerilog, Thrift,
Typst, Vala, Vim script, Vue, WGSL, Wolfram, Zig.

The order is Notepad++'s: grouped by first letter, alphabetical within a group,
with the one deliberate exception the table has always had (LaTeX before LISP)
and the three ungrouped rows last. Three new letter groups appear -- K, W and Z --
which the menu builds from the table rather than from a list, so nothing had to be
told about them except the test that counts the groups.

`for_extension` gained fifty-two arms. Two existing ones were reassigned, both
because the old owner was a guess that the new language makes wrong: `.fs` was
Fortran (free form) and is now F#, and `.sv`/`.svh` were Verilog and are now
SystemVerilog. Fortran keeps four extensions, Verilog keeps `.v`. Two of the new
languages get no extension on purpose: Meson's is `build`, which is too generic
to claim, and Puppet's `.pp` belongs to Pascal.

Four of the new rows show where the four-field model stops. Markdown has no line
comment, no quotes and no keywords, so it is one block-comment pair and a
syntax-free file. APL's control words are `:If` and `:EndIf`, and the lexer's
word rule cannot see a leading colon, so they are stored bare and colour only
when written without it. reStructuredText's `..` opens a comment and a directive
alike, so directives colour as comments. Org's marker is `#` followed by a space,
so the entry is `"# "` -- which keeps `#+TITLE:` live. Heredocs (Nix, Crystal,
Elixir), nested block comments (Elm, PureScript) and mixed-dialect files (Blade,
Svelte, Vue) are out of reach for the same reason, and were out of reach before
this pass.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline --release`:
902 in `nn-app` plus 809 in `nn-core`, 1711 passing, 0 failing.

The themes were checked end to end rather than in the abstract: `bench` renders a
frame from a settings file, so `theme tokyo_night` in that file and a dump of the
result gives an editor background of exactly `#1A1B26`, and `theme
solarized_light` gives `#FDF6E3`. The derivation is not asserted against itself;
the pixel is read back.

`cargo clippy --offline --release --all-targets`: 0 warnings.

## Fifty-first pass: a list you can open, in twenty-two languages

Two rows in the Preferences dialog asked you to step through a list with the
arrow keys: `Theme`, twenty-one entries, and `Language`, which until this pass had
exactly one. Both now open a panel. The panel is the row's content rather than a
new window -- twelve rows of it, hung below the value column, closing when you
choose or when you leave.

### The panel, and why it is not a submenu

Settings already has a `Theme` submenu: one row per theme, twenty-one of them,
added last pass because a row you step through cannot show you Tokyo Night unless
you already know where it sits in the cycle. That submenu is the right shape for
the menu bar, where you are browsing. The dialog is a different job: you arrived
at a row with an arrow key, you want to see the neighbours of the current value
and pick one, and you want the dialog to stay where it is. So the panel is drawn
inside the dialog's own panel bounds -- `prefs::picker_panel` -- and the dialog
does not move.

Geometry is measured from the dialog's grid, the same one the rows use: nine
pixels per cell, twenty per line, fourteen pixels of padding. The panel starts
flush with the value column -- x is the dialog's left edge plus the padding plus
`VALUE_COLUMN` (18) cells -- and one line below the row it belongs to. Its width
is the widest label times the cell width plus two paddings, floored at
`PICKER_MIN_WIDTH` (150) and capped at the dialog's own width, so a long
translation widens the list instead of overflowing it. It holds `PICKER_ROWS`
(12) entries. At 1100x800 the theme list lands at
x 506..677, y 290..537 -- 187 by 247 -- which is 12 rows of 20 plus 4 of padding
top and bottom. The language list starts lower, at y 510, and runs to 757.

Twelve rows because a list taller than the dialog is a list you read through a
keyhole, and both lists are longer than twelve: theme hides nine of its
twenty-one, language hides eleven of its twenty-two plus the rule. So the scroll
is not a contingency, it is what you see on open. The list opens scrolled to the
value in force rather than to the top, which is why `Language` opens showing
English pinned above the rule instead of the first file in the alphabet,
Chinese (Simplified).

Two clamps keep it inside the window. Horizontally it slides left if it would run
off the right edge; vertically it flips above its row if there is no room below,
landing one line above the row's top rather than on it. Neither happens at 1100
by 800 -- the theme list ends at 537 and the language list at 757 against an
800-pixel viewport -- so both are covered by tests that set a deliberately short
viewport rather than by a render. The flip is the one worth a test, because a
panel drawn over the row it belongs to looks like a panel that works.

Keys are the row's keys plus the list's. Up and Down move the highlight, Enter
and Space choose the highlighted entry, Right opens, Esc and Left close without
changing anything, and a click on an entry chooses it and closes the panel in one
gesture. `Picker::move_by` steps over `Divider` entries rather than landing on
them and stops at both ends rather than wrapping -- a list that wraps under a held
arrow key is a list you cannot stop on the entry you want.

### Two glyph passes, and what that costs

The renderer draws all its quads first and all its glyphs second, in one pass
each. A panel background therefore cannot hide text queued behind it: the
document's glyphs are already on their way to the atlas when the panel's
rectangle arrives. The menu bar lives with this by being drawn last; the dialog
cannot, because its rows are one `TextArea` and the panel's own labels are
another buffer on top.

The workaround is subtraction. `prepare` blanks the value text of every row the
panel covers, replacing the characters with spaces so the row keeps its shape and
the panel's own label is the only text in that region. The same treatment goes to
the hint line when the panel reaches it, which the language list does: the hint
under the dialog is drawn by the dialog's body buffer, the list hangs over it, and
without the blanking the hint would read through the list's last rows. The theme
list stays above the hint, so it keeps its keys on screen.

This is a renderer limitation worked around at the data layer, not fixed. The
glyph pass could sort by depth if `TextArea` carried a z, and until it does,
anything that wants to sit over dialog text has to blank it first.

### Where the translations came from

Twenty-two files in `localization/`, a hundred and thirteen labels each. They are
not Notepad++'s files, and that decision is worth recording because the obvious
move was to take them.

Notepad++ 8.x localizations key on `menuId` and `subMenuId`: the file is a table
of numeric ids pointing into the program's menu structure, roughly 1171 entries,
and the ids are indices into a tree the file does not describe. Reading one means
carrying a map of all 1171 ids next to the code that builds our menus, and every
new label would have to be added to that map before it could be translated. The
files are also GPL, and this repository has no license chosen, so vendoring them
would decide that question by accident.

So the schema is the one this tree already parses: a flat list of named items,
the menu's identifier on the left and the translated text on the right. A hundred
and thirteen of them, which is every label the menu bar and its panels draw --
`&File`, `&Edit`, `&Search`, `&View`, `&Encoding`, `&Language`, `&Settings`,
`&Window`, `&Help`, and the items under each. A file that names a label we do not
have is ignored; a label it does not name stays English, which is what makes a
partial translation safe to ship.

Shipped: Chinese (Simplified), Chinese (Traditional), Czech, Danish, Dutch,
Finnish, French, German, Greek, Italian, Japanese, Korean, Norwegian, Polish,
Portuguese (Brazil), Portuguese (Portugal), Romanian, Russian, Spanish, Swedish,
Turkish, Ukrainian. The English name is the one sorted and displayed, because the
list you scan for is the one you already know: alphabetically, `Chinese
(Simplified)` sits where you look for it and `简体中文` does not. The file keeps the
native name for the picker's label once chosen, and the dropdown shows English so
two entries cannot look alike to someone who does not read either.

The files are compiled in with `include_str!`, not read from disk at startup. One
binary is the whole installation -- no `localization` folder beside it, no
installer to copy one -- so a translation that lives in a file next to the
executable is a translation that is missing on the machine that needs it. A file
dropped into the config directory under the same name wins over the bundled copy,
which is how a translator tests a correction without a rebuild, and how a
language we do not ship gets added. `Catalog::languages(dir)` merges the two
sources, dedupes by file name, and sorts by English name once, so both paths
produce one list.

### The four that did not render

Chinese (both), Japanese and Korean drew as boxes on the first run. The chrome
built every `Buffer` with `Shaping::Basic`, which lays out glyphs from one font
and performs no fallback; the bundled font has no CJK coverage, so every
character became `.notdef`. cosmic-text supports fallback under
`Shaping::Advanced`, which is what the editor's own text already used -- the
document rendered Japanese correctly while the menu beside it showed rectangles.

`text::shaping_for(&str)` now answers the question in one place: `Advanced` when
the string holds anything outside Latin, `Basic` otherwise, since `Advanced` is
the expensive path and paid per menu rebuild is not free. Seven modules call it
instead of naming a constant, and the two empty-string constructor buffers keep
`Basic` because an empty string has nothing to fall back for.

Advanced shaping returns proportional widths, and the menu laid its text out in
character cells. Titles overlapped, panels clipped. Both are now measured: the
menu bar takes each title's width from its shaped buffer on every `prepare`, and
dropdown panels take theirs from the widest label and the widest shortcut rather
than from `len() * cell`. A fallback keeps the cell grid when the shaped rows and
the item list disagree in count, which is the state before `prepare` runs and the
state the tests build.

CJK filenames in the tab bar still use cell widths. That predates this pass, and
the tab bar is a different layout -- the tab's width is what decides where the
next tab starts, so the fix is a different fix.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline --release`:
929 in `nn-app` plus 809 in `nn-core`, 1738 passing, 0 failing. Twenty-seven of
them are new, eighteen of those in `prefs`, a module that had none before.

`prefs` gained its first tests, eighteen of them, because a picker that opens on
the wrong row or closes on the wrong key is invisible in a render. They cover the
theme list's contents and its divider, language pinning and ordering, opening on
the value in force including scroll, stepping over the divider, clamping at both
ends, choosing a theme and a language, choosing the value already in force,
returning from any language to English, the `picker_at` hit test in and out of the
panel, flipping above when there is no room below, closing without closing the
dialog, the hint fitting the dialog's width, the hint being blanked when covered,
and covered values blanked.

Renders checked with `bench`, frame by frame: the theme picker with the panel over
the rows below it, the dividing rule at y=364, the hint legible and unclipped; the
language picker with English pinned above the rule at y=544, the highlighted row,
the hint blanked; German and Russian menus; the Japanese menu bar spacing all
nine titles correctly; the Japanese `File` dropdown with the panel wide enough
for its labels and the shortcut column right-aligned; and the English menu
unchanged from the pass before.

Five rows in that Japanese dropdown are still English: `Open Containing Folder`,
`Open Folder as Workspace...`, `Save a Copy As...`, `Rename...` and `Close
Multiple Documents`. They are labels Neko++ has that Notepad++ does not, so they
were never in the hundred and thirteen. The Preferences and Shortcuts dialogs are
English in every language for the same reason -- they are ours.

`cargo clippy --offline --release --all-targets`: 0 warnings.

## Fifty-second pass: as many rows as the list has

The first thing anyone did with the last pass was count. Twenty-one themes,
twelve on screen; twenty-two languages, twelve on screen. The scroll worked. The
problem was that nothing said it was there: a list that stops mid-alphabet looks
like a list that ends mid-alphabet, and the missing nine themes looked like
themes the build did not have.

### The cap was the mistake, not the scroll

`PICKER_ROWS` was twelve, chosen for a dialog that is four hundred pixels tall.
It was the wrong constant to write, because it encoded the dialog's height in a
list's capacity: the dialog did not get taller when the catalogue grew, but the
comment beside the constant assumed a list of twenty-one was a list of twelve
with an explanation. There was no explanation -- no scrollbar, no count, no fade.

So the panel asks for the height its contents need and is given it. The order is:
the whole list under its row if it fits; the whole list above its row if it fits
there instead; otherwise the side with more room, as many rows as that side
holds, and a scroll. `PICKER_ROWS` is gone. What is left is `PICKER_MIN_ROWS`,
four, and it is only reached when the window is too short to hold the list on
either side -- a list that has nowhere to go still has to show something you can
move through.

At 1100 by 800 that puts the theme list at 290..738 with all twenty-one entries,
and the language list at 250..738 with all twenty-two. Both cover most of the
dialog, which is what a dropdown of twenty entries does.

### Where a panel may go

Two rules keep a long panel from looking broken. It stays inside the window: a
panel that runs off the top edge loses its first entries to the edge of the
screen, and no amount of scrolling gets them back. And it stays off the title:
covering the word `Preferences` leaves a dialog with no name, which reads as a
rendering fault rather than as a long list. So the panel's top is floored at the
title's baseline unless the list is taller than the space under it, in which case
the floor drops to the dialog's own top edge and the title is blanked -- the same
subtraction the covered values already get, for the same reason: glyphs are drawn
after quads, so a frame cannot hide text behind it.

The language list is the one that uses both rules. Its twenty-three entries do
not fit under the second-to-last row, so the panel flips above it -- and lands at
y 22, over the menu bar, because 468 pixels above the row is where the arithmetic
puts it. That is the case the clamp exists for: the panel slides back down over
the dialog's own rows until its top meets the title's baseline, and stops there.

### The keys, when the hint is underneath

A list that reaches the dialog's hint covers it, and the hint has to be blanked
or it draws through the panel. That trades one problem for another: the keys
were the only thing on screen saying Enter chooses and Escape closes, and the
list had just deleted them.

The panel carries them instead, on its own last line, under a rule so they read
as a footer rather than as an entry that cannot be chosen. The line costs one
line of height, and it is only drawn when the panel actually covers the dialog's
hint -- a short list leaves the dialog's own hint visible, and two hints would be
one too many. When there is no room for the extra line at the bottom of the
window, an entry gives its row up for it: losing the keys to a window edge is how
a list becomes a puzzle.

The footer is not an entry. `picker_at` maps a click to an entry index and the
footer's row is past the last entry, so a click on it chooses nothing -- which is
the behaviour a footer should have, and the reason the hit test was left alone
rather than special-cased.

### Keys and the wheel, for the window that is still too short

A list that scrolls needs more than Up and Down when it is twenty entries long
and the window is six hundred pixels tall. Page Up and Page Down move the
highlight by a screenful, Home and End jump to the first and last choice, and all
four step over the rule at the top of the theme list. The wheel over an open list
moves the highlight too: the list is what the pointer is on, and scrolling the
document behind a panel that covers it would move text nobody can see.

All of it moves the highlight rather than the view, because the highlight is the
view: there is no scrollbar, so an entry scrolled off screen with the highlight
on it is an entry you cannot see is selected.

### One bug the change exposed

`prepare` returned early when the dialog's own text had not changed, before
shaping the list. That was correct while the list was a fixed twelve rows and the
only thing that changed its text was opening it. It is wrong now: scrolling
changes the list's text without changing the dialog's, so the early return left
the panel showing the rows it had before the scroll. The picker is now shaped
after the comparison rather than behind it.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline --release`:
936 in `nn-app` plus 809 in `nn-core`, 1745 passing, 0 failing. Seven of them are
new: the whole list shows when the window has room for it, the language list
slides up and stays inside the window and off the title, a short window scrolls
instead of clipping, Home and End reach both ends, a panel too tall for the title
blanks it, a panel over the hint carries the keys, and a short list leaves the
dialog's hint alone.

Renders, frame by frame: at 1100 by 800 the theme list shows all twenty-one plus
the footer, and the language list shows all twenty-two with English pinned above
the rule at the top; at 1100 by 600 the theme list shows nineteen of twenty-one,
its footer on the last line, and scrolls.

`cargo clippy --offline --release --all-targets`: 0 warnings.

## Fifty-third pass: a font for Korean, Hebrew, and twenty-nine more themes

### Korean was never a translation bug

`korean.xml` was complete and correct. The menu bar still drew as boxes, which
looked like a translation problem and was not: no font on the machine had the
syllables in it. `fc-list :charset=ac00` returned one file, and `fc-query` on it
showed a charset of `ac00` alone next to `4e00-9fa5` and `d7a2-d7a3` -- one Hangul
syllable out of eleven thousand. A sweep of the 1714 font files under
`/usr/share/fonts`, `/opt/DEV` and the cargo registry found nothing that could
draw `한`. Japanese worked, Greek worked, so the fallback and the shaper were
fine; the glyph simply did not exist on this machine.

An editor cannot install a package, and telling the user to install one is a
support answer rather than a program. So the program carries the script: the
binary now includes a 1.3 MB subset of NanumGothic Regular holding all 11172
syllables of U+AC00-U+D7A3, both jamo blocks, Latin-1, general punctuation and
the few symbols a Korean source file needs. Hanja, the fuller Latin and the hinted
outlines are dropped; the original is 2 MB and the subset keeps what Korean text
is made of. It is registered in `fonts::font_system()` through
`FontSystem::new_with_fonts`, not added to the database afterwards: the fallback
lists are computed from the faces present at construction, so a face loaded later
would sit in the database and never be consulted.

NanumGothic is under the SIL Open Font License 1.1, and the licence reserves the
name `Nanum` for unmodified copies. A subset is a modified version, so the face is
renamed `Neko Hangul` in every name record, with the derivation in its description
and the licence text beside it in `crates/nn-app/assets/OFL.txt` -- the same file
`debian/copyright` now points at, because an OFL font has to travel with its
licence however it is distributed.

Re-running the subset, for the record:

    pyftsubset NanumGothic-Regular.ttf --output-file=NekoHangul-Regular.ttf \
      --unicodes=U+0020-007E,U+00A0-00FF,U+2000-206F,U+20A9,U+1100-11FF,\
    U+3131-318F,U+A960-A97F,U+AC00-D7A3,U+D7B0-D7FF,U+3000-303F \
      --layout-features=kern,liga,ccmp,locl,mark,mkmk --no-hinting \
      --desubroutinize --drop-tables+=DSIG --name-IDs='*'

Two tests hold the result still. One shapes `한글파일 A1` in a font system built
with nothing in it but the bundled face, so a codepoint the subset lost is a
failure rather than a fallback; the other shapes `한글파일` in the real font
system and checks that the four glyphs came from the bundled face, which is what
says the fallback search actually finds it.

### Hebrew

`localization/hebrew.xml` is the twenty-third translation, keyed by the same 113
English labels as the others, so it needed nothing but the file and one entry in
`i18n::BUNDLED`.

Right-to-left needed no layout work, which was worth confirming rather than
assuming: cosmic-text runs `unicode-bidi` over every line with the advanced
shaper, and the advanced shaper is what any non-ASCII label already asks for. The
render shows the labels reading right to left, the ellipsis of `פתיחה...` at the
left end where a Hebrew reader expects it, and the shortcut column untouched --
`Ctrl+Shift+S` stays left to right inside a line whose text is not. Rows that are
not translated stay English and stay left aligned, which is what every other
translation does with them.

The menu bar itself stays in File, Edit, Search order rather than mirrored. That
is chrome, not text: mirroring the bar, the tab strip, the status bar and every
dialog is a layout project, and half of it would be worse than none. The words in
the bar are Hebrew, which is what a user asking for Hebrew was missing.

### Twenty-nine more palettes

The catalogue went from 18 to 47, and the row and the Settings submenu from 21 to
50: Ayu Dark, Light and Mirage; Catppuccin Frappé and Macchiato; Darcula;
Everforest Light; GitHub Dark Dimmed; Horizon; Iceberg; Kanagawa Dragon and
Lotus; Material Ocean and Palenight; Molokai; Monokai Pro; Moonlight; Nightfox;
Oceanic Next; Panda Syntax; Quiet Light; Rosé Pine Dawn and Moon; Shades of
Purple; Synthwave '84; Tokyo Night Day and Storm; Vitesse Dark; Zenburn.

Every one is its published palette rather than an approximation, which is the only
reason a name is worth putting in a list: the four properties the tests pin are
the same ones the first eighteen were held to -- `light` agrees with the
background's luminance, text clears 4.0 against its background with exactly one
palette (Solarized Light) below AA, and no syntax colour sits within forty of the
background's brightness. All twenty-nine clear 4.5, so the count of dim palettes
stayed at one and the test that pins it did not have to move.

Names carry their accents (`Catppuccin Frappé`, `Rosé Pine Moon`, `Synthwave
'84`); keys stay bare words (`catppuccin_frappe`, `rose_pine_moon`,
`synthwave_84`), because a key is written to a settings file and read back by
splitting on whitespace.

### The list outgrew the window

Fifty themes do not fit in eight hundred pixels, and the panel already knew how to
scroll. Adding Hebrew did something the theme list had not: the language list grew
to twenty-five rows, which does not fit on one side of the Language row, so the
panel scrolled -- and hid Ukrainian and Swedish behind a footer with nothing on
screen saying there was more. That is precisely the failure the height-aware
panel was built to avoid, arriving from the other side.

So the rule is now about the window rather than about the row: a list that does
not fit beside its row but does fit in the window is shown whole, and the panel
slides to wherever it has to stand -- over the dialog's rows, over its title if
the title is in the way, blanking it as it already did for a panel too tall for
it. Only a list taller than the window drops to a scroll. Twenty-five languages
now show whole; fifty themes still scroll, which is honest, and their footer says
so.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline --release`:
940 in `nn-app` plus 809 in `nn-core`, 1749 passing, 0 failing. Four of them are
new: the bundled face carries Hangul and ASCII with nothing else installed, the
fallback finds it among the installed fonts, a list taller than its row reaches
the top of the window, and the theme list scrolls instead of running off it. Three
existing ones were rewritten because their scaffolding named a second theme or a
second language that had since stopped being second.

Renders: Korean at 1100 by 800 reads `파일 편집 찾기 보기 인코딩 언어 설정 도구
매크로 실행 플러그인 창 도움말` in the menu bar with no boxes; Hebrew reads the same
bar right to left with the File menu open showing `חדש Ctrl+N` and `פתיחה...
Ctrl+O`; the theme list shows System, Light, Dark, the rule and the catalogue to
`Kanagawa Dragon` with its footer on the bottom line; the language list shows
Hebrew pinned above the rule and English through Ukrainian below it, Ukrainian
included. Ayu Dark and Tokyo Night Day were rendered through the settings file to
check that a published palette looks like itself.

`cargo clippy --offline --release --all-targets`: 0 warnings.

`cargo fmt` was not run.

## Fifty-fourth pass: the highlight is the choice

### Choosing blind

Fifty palettes in a list is a list you cannot use. Open the Theme row, press
Down, press Enter, look at the window, dislike it, open the row again -- and the
highlight is one entry further along than yesterday, with no way back to the one
you had. The row behaved like a form submitted once, when the only thing being
edited is the screen in front of you.

So the highlight is now the choice. Every press that moves it puts the entry it
lands on in force at once: the palette repaints, the menu bar retranslates.
Enter is no longer what applies the value; it is what writes it down.

### Three places a value can come from

A preview, a choice and a revert all mean "make this entry the setting", so
there is one function that does it: `Picker::set_choice`, which writes the row's
entry into `self.settings` and reports whether anything changed by comparing the
settings before and after. The preview (`apply_picker_choice`), Enter
(`picker_choose`) and Escape (`close_picker`) each decide *when* to call it, and
none of them decides *what* it means, so none of them can drift from the others
about what entry index 7 is.

`set_choice` is guarded with `get`, so an index left over from a list that has
since changed is a no-op rather than a panic, and the same guard means a revert
to an entry the row no longer has is silent rather than fatal.

The movers report whether the highlight moved: `Picker::step`,
`picker_move`, `picker_page` and `picker_jump` all return `bool`. A press at
either end of a list goes nowhere, and a press that goes nowhere should not
repaint the window -- with fifty palettes and a window of text to recolour behind
each one, that is the difference between a held key and a stuck one. `handle_prefs_key` collects that flag from the movers and calls
`apply_settings_quiet` when it is set; the wheel path in `handle_wheel` does the
same for a scroll through the list.

### Escape is the undo

A preview that cannot be undone is a trap, so the list remembers the entry that
was in force when it opened (`Picker::opened_with`) and `close_picker` puts that
entry back, returning whether there was anything to undo. Clicking away from the
list is the mouse's Escape and takes the same path.

Nothing is written on the way out, and nothing has to be: `apply_settings_quiet`
repaints without saving, so the settings file still holds the value the list
opened on, and the revert restores the screen from the file. That asymmetry is
deliberate and is the reason the quiet variant exists rather than a "save" flag.
Enter and a click on a row call `apply_settings`, which saves -- Enter writes the
file even when the highlighted entry is the one already in force, because "the
value in force" is only as new as the last write.

### A list is not rebuilt under the highlight

`set_languages` used to refresh an open list, which was right while the list was
a menu of what could be chosen. It is wrong now. The language list pins the
language in force above a rule and sorts the rest behind it, so a list rebuilt
on each preview would move the pin to whatever the highlight had just reached,
and every row below it would shift by one under the arrow. Worse, `opened_with`
is an index into the list the user opened: rebuild the entries and it points at
a different language, so Escape reverts to somewhere nobody has been.

So the list is built when it opens and left alone while it is open, and
`set_languages` records the available languages for the next time. The pin is
stale by design while a preview runs -- it says where the list started, which is
exactly what the list is for.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline
--release`: 944 in `nn-app` plus 809 in `nn-core`, 1753 passing, 0 failing. Four
are new: moving the highlight puts the theme in force, moving it puts the
language in force, Escape puts back the theme the list opened on, and Escape
without a move reports no preview to undo. `the_highlight_stops_at_both_ends`
now also asserts that a press at the end reports no change, and the tests that
checked whether Enter had applied anything lost the assertion, because Enter no
longer reports -- it writes.

The behaviour was then driven through the real binary on Xvfb with XTest keys
rather than only through the panel's own API, because "the window repaints when
the arrow moves" is a claim about the event loop and not about `Prefs`. On a
fresh configuration directory the dialog opened on Theme=System; Return opened
the list; one Down put the whole window in the light palette with the list still
open and its highlight reading Light; a second Down put it back in Dark; Escape
returned to System, and `settings.txt` had still not been created. Repeating the
two Downs and pressing Return instead wrote `theme dark` to it. On the Language
row, one Down with the list open retranslated the menu bar to `文件 编辑 视图
编码 语言 设置 工具 宏 运行 插件 窗口 帮助` while the file still said `language
none`; Escape put English back.

Renders: the Preferences panel, the theme list and the language list at 1100 by
800 were re-rendered and are unchanged, which is the point -- the preview moved
when a value takes effect, not what the panel looks like.

`cargo clippy --offline --release --all-targets`: 0 warnings.

`cargo fmt` was not run.

## Fifty-fifth pass: one window, one language

### The complaint that is not a bug report

The user wrote that translations sometimes mix -- *quelques fois il y a des
mélanges de langues, ca ne doit pas se produire*. There is nothing to reproduce:
the window draws, every string on it is a string, and the defect is that two of
them are in different languages at the same moment. So this pass starts from
where strings come from rather than from where they look wrong.

### What the menu had and the rest did not

The menu bar has taken a `Catalog` since the thirty-fifth pass. Everything else
in the frame was built from literals: `Preferences` held `"Preferences"`, the
status bar `Ln:` and `Col:`, the prompt bar `Find what:`, the find bar `Match
case`, the about box `Close`, the toolbar its tooltips. A French session
therefore read `Fichier Édition Recherche Affichage Encodage Langue Paramètres
Outils Macro Exécution Plugins Fenêtre Aide` across the top of the window and
`Preferences`, `Find what:`, `Ln: 3, Col: 1` in the middle of it.

That is worth being exact about, because "mixed languages" sounds like bad
French and is not. Every string in that frame was correct in its own language.
What was wrong is that the frame had two, and a user reading `Preferences` under
a French menu cannot tell whether the file is incomplete or the program is. The
cure is not a better translation; it is that the catalog has to reach every
widget that draws a word.

### Seven holders, one assignment

Each chrome widget now keeps an `Option<Catalog>`, `None` meaning English, and
exposes `set_catalog`: `menu`, `prefs`, `status`, `prompt`, `find`, `about`,
`toolbar`. `State::apply_language` is the only thing that fills them, from the
one catalog it has just loaded.

The alternative was to pass `&Catalog` into each draw call. That was rejected for
a specific reason: a frame is drawn from more than one place -- the live event
loop, the benchmark's frames, the snapshot tests -- and a catalog that arrives as
an argument is a catalog one of those call sites can forget to pass. A catalog
that is a field is either there or not, and `Option` makes "English" an explicit
state rather than an omission. One assignment function is what makes it possible
to see, on one screen, that a window cannot be half-translated.

### The fallback is English, which is exactly what hides the gap

`Catalog::tr` returns its argument when the file has no entry. That is right at
runtime -- a partial file should leave a word in English rather than draw nothing
-- and it is exactly what lets an incomplete translation ship without a symptom,
because the call that draws an untranslated label succeeds and returns something
plausible.

`translates` sits next to it, asking the map rather than the fallback, and the
coverage test uses `translates`. The pair is the point: `tr` is for drawing,
where English is an acceptable answer; `translates` is for auditing, where it is
not.

The cost of `tr` is a lowercase allocation per call, so every translated label
allocates once per frame. With a dozen chrome labels that is not measurable and
worth saying anyway, because the status bar is redrawn every frame and a future
row drawing a thousand segments would find it.

### The test grows with the window

A list of labels kept beside the test would be wrong within a month, so each
module is asked for its own. `Preferences::labels()` walks `Row::ALL`,
`FileDialog::ALL`, `BackupMode::ALL`, `ThemeChoice::ALL` and adds the handful of
strings it draws outside those tables; `StatusBar::labels()`,
`PromptBar::labels()`, `FindBar::labels()`, `AboutBox::labels()` and
`toolbar::labels()` build theirs from the enums they draw from. `chrome_labels()`
unions the six lists, sorts and dedups, and
`every_bundled_language_translates_every_window_label` asks all twenty-three
bundled files for every one of the hundred and thirty-nine labels.

A new row in the Preferences panel is therefore a label every language is asked
for without anybody remembering to add it: the row enum grows, and the test
either passes because the files were updated or names the files that were not.
`is_literal` exempts the labels that are not words: single characters, the five
Convert Case samples (`cmd`, `lowercase`, `UPPERCASE`, `iNVERT cASE`, `ranDOm
CasE`), theme names and language names. `Tokyo Night` is the name of a palette,
not a phrase, and asking a German file to translate it would be a bug in the test
rather than in the file.

The menu bar keeps its own equivalent test next to the rows it is about.
Twenty-three files, four hundred and fifty-five `<Item>` elements each.

### What is still English

The results panel, the changelog, print, the shortcut mapper, the style
configurator, the charset dialog, the browser, the folder panel and the
completion popup. This list is here rather than omitted: some of these draw
derived text -- sizes, timestamps, paths -- where a translated frame around
English content would be the worse outcome, and the rest are content rather than
furniture. A reader who finds `Find All` results in English after this pass
should know the boundary is drawn there on purpose, not that the crawler stopped.

### The second front end

`bench` builds the same widgets and now sets all seven catalogs from the same
file. A benchmark frame that drew an English label inside a translated window
would disagree with the product about what the product looks like, and nothing
would catch it, because `bench` asserts nothing about what it draws. Seven lines
are cheap insurance against a second front end drifting from the first.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline --release`:
948 in `nn-app` plus 816 in `nn-core`, 1764 passing, 0 failing. The coverage test
is new; the count it guards, a hundred and thirty-nine labels, was read by
temporarily raising the bound until the assertion printed what it was guarding,
and the file was restored afterwards -- `diff` against a copy taken before
reports it identical.

Renders, from a configuration directory whose settings file named `language
french.xml` and `theme dark`: the menu bar reads `Fichier Édition Recherche
Affichage Encodage Langue Paramètres Outils Macro Exécution Plugins Fenêtre Aide`;
the Langue menu reads `Aucun (texte normal)` above `A`, `B`, `C`... with
`Langage défini par l'utilisateur` and `Défini par l'utilisateur` in its footer;
the Paramètres menu reads `Préférences...`, `Configurateur de style...`, `Thème`,
`Mappeur de raccourcis`, `Importer`, `Modifier le menu contextuel`; the
Preferences dialog reads `Préférences` over `Thème Sombre`, `Taille de police
15`, `Largeur de tabulation 4`, `Retour à la ligne Désactivé`, `Mini-carte
Activé`, `Barre d'outils Activé`, `Liste des fonctions Désactivé`, `Recherche
volatile Activé`, `Sauvegarde lors de l'enregistrement Aucun`, `Enregistrement
automatique 7 s`, `Boîte de dialogue de fichier Boîte de dialogue système`,
`Langage French`, `Raccourcis Édition...`, with the hint `Haut/Bas
Gauche/Droite Entrée ouvre Échap ferme`; the status bar reads `Ligne : 1,
Colonne : 1 Sél : 0 longueur : 18 lignes : 3`. No word of the frame is in
English, which is the property the pass is about.

`cargo clippy --offline --release --all-targets`: 0 warnings.

`cargo fmt` was not run.

## Fifty-sixth pass: the view that gives its document away

### Parity is a list of sentences

The user came with the documentation rather than with a bug: move a tab to the
other view, clone a document into both, rotate the divider between side-by-side
and stacked, synchronise scrolling, and get back to one window by closing the
second view's tabs. Most of that machinery already existed. The question was
never whether `Action::MoveToOtherView` was wired to something; it was whether a
session that uses all of it behaves. It did not. Using the feature for an hour
found seven defects, and every one of them is predicted by those five sentences.

### One buffer, two documents

Notepad++ puts one buffer in two views: type in either and the other follows.
This engine has one `Document` per tab -- a rope, per-line lexer states, fold
levels, a dirty marker -- and a second view of the same file is a second
`Document`. Pretending otherwise would mean threading a buffer identity through
every edit path, so a clone is a second document that joins the source's *group*,
and the group is what keeps the text equal.

Each tab carries `group` and `mirror_rev`. `sync_clones` runs before every frame:
it collects `(pane, index, group, revision, stamp)` for both panes, finds the
groups with more than one member, and for each group takes the tab whose revision
moved and gives the others `mirror_from` -- the rope, the per-line styles, the
lexer states, the fold levels and depths, the line lengths -- then reopens any
fold the new text no longer has, resizes the marker array, and marks the document
changed. The next frame draws the same bytes in both panes without either pane
knowing it was edited.

`Editor::adopt` is the view-level half: keep this view's caret, take the twin's
text, forget the undo history, and take the twin's dirty flag last, because
`mirror_from` marks the document changed and `set_caret` breaks the undo group
without touching the flag.

### Copying the styling is the whole point

The obvious implementation re-highlights the adopting document. On a
940,000-character file `highlight_all` takes 53 ms. That is not a frame; it is
three frames, on every keystroke, in the pane the user is not typing in. Copying
the style array is 0.42 ms -- two orders cheaper -- and it is also the more
correct answer: both views then show the same colours, rather than whatever the
lexer does with the same bytes a second time.

### Three things not shared, on purpose

The caret, the undo history and the scroll position. The caret is the reason to
clone at all: one view watching the top of a log and the other the bottom cannot
share a cursor. The history cannot be shared because the ranges in the clone's
undo stack describe text that has just been replaced, and replaying one would cut
at an offset that may now be past the end -- so the clone forgets its history and
undo keeps working in the view the user last typed in. The scroll position is the
second half of the same reason as the caret. All three are visible differences
from Notepad++, which has one buffer with one caret and one undo stack, and they
are recorded here rather than left to be discovered.

### Seven defects, found by using it

Using it meant the real binary on a virtual display, driven by the mouse.
Keyboard injection is not available in this environment, so every flow below went
through menus, tabs and the divider.

1. **A click on the inactive view teleported the documents.** Focus was
   implemented by swapping `self.tabs` with `other.tabs`, so clicking in the
   right-hand pane moved the left-hand file to the right and vice versa: two
   documents changed sides under a click meant to say "I am working here". Focus
   is now a flag, `focused_second`, with `layout::by_focus` ordering the two
   rectangles by it, so every hit test and every draw call still addresses "the
   focused pane" and "the other one" and the geometry decides which is which.
   The renderer takes the flag for the same reason: it draws the focused pane's
   tabs in the rectangle the hit test believes in.

2. **The lower strip of a stacked split was dead.** The tab strip's band was read
   from the top of the window; in a top-and-bottom split the second strip sits
   halfway down. `strip_band()` now returns the band of the focused pane's own
   strip, computed from the same `pane_rects()` the renderer draws with.

3. **The split was persisted.** `Settings` carried `split` and `ratio`, so a
   session that had once been split opened split -- a window arrangement the user
   had not asked for on this run. Notepad++ starts in one view. `save_split`, the
   two fields and the tests that round-tripped them are gone. The sync and zoom
   flags stay, because they are preferences about documents, not about a window.

4. **Right-click never focused the pane under the pointer.** The tab context menu
   took the tab index from the strip under the cursor but acted on the *focused*
   pane's tab list, so right-clicking a tab in the unfocused view offered actions
   for a different document -- `Close` would have closed the file next door. The
   focus block from the left-click path now runs in `right_click` first.

5. **Both strip guards used the window-top band**, which is defect 2 seen from the
   input side: a press in the lower strip could be classified as a press outside
   any strip. Both guards now use `strip_band().1`.

6. **Marks survived a pane swap.** Search matches, the current match, the bracket
   pair and the smart occurrences are byte ranges into the text a view is
   drawing. After a swap the views draw each other's documents and the ranges
   point into the wrong text; the visible symptom was a blue rectangle over the
   word `beta` in a file that had never been searched. `EditorView::forget_marks()`
   drops them, `State::forget_marks()` does it for both views and resets the
   smart-highlight cache key -- a key that outlived its ranges would skip the
   recompute that puts the focused view's own highlight back -- and `swap_panes`
   calls it.

7. **Moving a view's last tab out left a phantom document.** An empty pane is
   drawn with a placeholder `new` tab, and the move code pushed that placeholder
   when it emptied the source pane: the pane the user had just emptied kept a
   blank document nobody had created, and the window was still split. The move
   now computes `emptied` before the swap, skips the placeholder, and after
   `swap_panes()` -- so the surviving pane keeps the focus the user expects --
   clears `other`, sets `Split::None`, clears `focused_second`, and resizes the
   surviving view to the whole window.

### The rule the seventh defect made

A view that gives away its last document goes away. That is the rule closing the
last tab of a view already followed, and it is the third way back to a single
window: close the second view's tabs, drag its last tab into the first view, or
run `Move to Other View` on it. Before the seventh fix, the third route left a
ghost pane with a blank document; now all three end in one pane filling the
window, with the focus on the document the user moved.

### What is not claimed

Reordering tabs inside one strip: a drag that ends anywhere but the other view
leaves the tab where it was, and this editor does not claim that gesture.
Scrolling synchronisation exists as two toggles (`sync_scroll_v`,
`sync_scroll_h`) and one per-frame copy -- a frame is the natural granularity,
because it asks "where should this pane be now" once instead of editing two dozen
scroll call sites -- but it was not exercised in the live run. With no keyboard
and no wheel injection available there was no way to scroll one pane and watch
the other, so its evidence is the code and the settings round-trip, not a render.

### Verification

`cargo build --offline --release`: no warnings. `cargo test --offline --release`:
948 in `nn-app` plus 816 in `nn-core`, 1764 passing, 0 failing. Among the new
tests are `the_focused_rectangle_comes_first` and
`a_single_view_ignores_the_focus_order` in `layout`,
`mirroring_copies_text_and_styling_but_not_bookmarks` and
`mirroring_reopens_a_fold_the_new_text_no_longer_has` in `document`, and
`adopting_a_twin_keeps_the_caret_and_drops_the_history`,
`adopting_a_twin_copies_its_styling` and
`adopting_a_shorter_text_pulls_the_caret_back_inside` in `editor`. Defect 7 has no
unit test: `State` needs a real window and `main.rs` has no test module, so its
evidence is the render below. `bench` gained a `flip` keyword so the benchmark can
render the unfocused-first arrangement as well as the focused one.

The binary needs lavapipe in this container (`VK_ICD_FILENAMES` pointing at
`lvp_icd.x86_64.json`, `WGPU_BACKEND=vulkan`), a 1400x1000 Xvfb on `:9`, `HOME`
pointed at a scratch configuration directory, and `--new-instance a.txt b.txt` so
that two documents exist to split.

Flows, each one driven with the mouse and read off a screenshot:

- Right-click the second tab, `Move to Other View`: two panes side by side, the
  moved document in the second, and no stale blue block on the word it had been
  highlighting in its previous pane (defect 6).
- Click in the other pane: the focus moves, the documents stay where they are,
  and the pane that lost the focus drops its marks (defect 1).
- Right-click the divider, `Rotate to Horizontal`: a stacked split. The rotation
  is session state, which is defect 3's point -- a restart comes back to one
  view.
- Right-click a tab in the *lower* strip while the upper pane is focused: that
  tab's menu opens, and the tab it acts on is the tab it was opened on (defects 4
  and 5).
- Drag a tab from one strip to the other: the tab moves, and a source pane that
  had only that tab collapses instead of showing a blank `new` document (defect
  7); the surviving strip fills the window and the moved document keeps the
  focus.
- `Clone to Other View` on the active tab, then `Edit > Convert Case To >
  UPPERCASE` in one pane: both panes read `BETA ONE` and both are marked dirty,
  because they are one group (the clone path).
- Close the second view's only tab: the split collapses to a single full-width
  strip, which is the behaviour the user asked about.

`cargo clippy --offline --release --all-targets`: 0 warnings.

`cargo fmt` was not run.

## Fifty-seventh pass: a strip that clipped to the wrong edge

### Two complaints, one geometry

The user reported two things about the tab strip: a long filename runs under the
pin and the cross, and a window full of tabs offers no way to reach the ones past
the edge. They are one defect seen from two sides. The strip had a single clip
rectangle, the whole row, and no notion of a position within that row. A title
was therefore cut at the strip's edge -- which is to say, not cut at all until it
had already crossed its own buttons and the next tab -- and the row simply ended
at the window, with every tab past it placed and painted but unreachable.

### A clip is only as good as its rectangle

`text_areas` built one `TextBounds` from `self.left` to `self.left + block_width`
and handed it to every title. The tabs' fills had been clipped per tab for a
while; the text had not. So a 49-character filename drew all 49 characters,
through the pin, the cross and the tab beside it, and only stopped at the pane's
far edge. The clip is now the tab's own title slot: from `title_left` -- after the
pencil, if there is one -- to `label_right`, which is the pin's left edge less
`LABEL_GAP`. Eight pixels is not a typographic nicety; it is what makes the gap
visible at the sizes the strip actually uses, since a glyph that ends one pixel
short of the pin reads as touching it.

Measured on the built binary: a tab at `MAX_TAB_WIDTH` (260) shows its title ink
ending at x = 200, the pin's ink starting at 222, and the cross's at 249 -- a
22-pixel gap of bare tab between the name and the first button, where before the
name ran under both. The clip admits glyphs up to
210 and layout stops at the last one that fits, which is the behaviour a
fixed-width strip wants: cut the name, do not squeeze it.

### A row that cannot scroll says nothing

There was no scroll state. `slots` started every horizontal row at zero and ran
on, so the only tabs that existed were the ones that fitted, and the rest were
painted under the arrow zone that did not exist. `TabBar` now carries `offset`,
and the row starts at `-offset`. Everything else already read `slots`, which is
why one field was enough: the fills, the hit test, the titles and the buttons all
moved together.

The arrows are drawn in a reserved zone at the trailing edge --
`BUTTON_ZONE` wide, the same 36 pixels a tab reserves for its own two buttons --
and `usable_width` shrinks the row by that much while it overflows, so the last
tab stops flush against the arrows instead of sliding beneath them. `tab_at`
returns `None` over the zone, because a click there is a scroll and cannot also
be a click on the tab behind it.

### Glyphs cannot be clipped, so they are dropped

A tab's fill is a quad and is clipped to `[0, usable_width]`. The marks on a tab
are atlas glyphs and are not: a pencil on a tab that had scrolled off would have
been drawn at a negative x, over whatever sits to the left of the strip. They are
dropped instead, by `mark_is_visible`, which asks for the whole glyph to be inside
the row rather than a clipped fragment of one. The active tab's own buttons can
never be affected: `ensure_active_visible` keeps the active tab inside the row by
construction.

### Scrolling is state, and state has ends

`scroll_step` moves three quarters of the visible width, so the tab that was at
the edge stays on screen as a landmark. `clamp_offset` holds the result between
zero and `content_width - usable_width`, on every change of tabs and of bounds,
and `ensure_active_visible` scrolls the active tab in when it is opened, cloned,
moved from the other view or reached from the keyboard -- otherwise the keyboard
would reach tabs the pointer could not.

That first version was wrong in a way only a test found. `set_tabs` runs before
`set_bounds` at startup, when the strip's width is still zero; `clamp_offset`
against a zero width allowed a large offset, and `ensure_active_visible` then
computed one from it, so the strip opened parked past its own first tab. Both
functions now return early on a width of zero, and `set_bounds` re-anchors the
strip when the width actually changes -- on a resize, and only then: doing it
every frame would undo the arrows as fast as they are pressed.

### Verification

`cargo build --offline --release`: clean.

`cargo test --offline --release`: 954 passed in `nn-app`, 816 in `nn-core`, 1770
in all, no failures. Six tests are new in `tabbar`: `a_long_title_stops_before_the_pin`
(asserts `label_right` is `LABEL_GAP` short of the pin and that the name is wider
than the room left for it, so the clip is doing work), `the_arrows_appear_only_when_the_tabs_do_not_fit`,
`scrolling_stops_at_both_ends`, `activating_a_far_tab_scrolls_it_into_view`,
`a_click_on_an_arrow_says_which_way` and `a_tab_under_the_arrows_cannot_be_clicked`.
`a_single_row_strip_never_wraps` was rewritten: it used to assert that a tab at
`3 * MIN_TAB_WIDTH` was still hit-testable with a `MIN_TAB_WIDTH` strip, which is
precisely the behaviour the arrows have to break. `icons`' exact-name test gained
`tab_left` and `tab_right`.

`cargo clippy --offline --release --all-targets`: 0 warnings.

Flows, each driven with the mouse at 1100x750 on a 1400x1000 Xvfb on `:9`, read
off a screenshot. The binary needs lavapipe (`VK_ICD_FILENAMES` at
`lvp_icd.x86_64.json`, `WGPU_BACKEND=vulkan`) and `HOME` on a scratch directory.

- Ten files opened at once, the active one last: the strip opens anchored on it,
  the two chevrons are drawn in the trailing zone, and the first tab is cut
  mid-name at the left edge rather than bleeding over the pane.
- The same window after three presses of the left arrow: the row is back at its
  start with the leading tab intact. The two end stops are asserted in
  `scrolling_stops_at_both_ends` rather than read off a screenshot, since a
  scroll that does nothing and a scroll that is refused look the same.
- One press of the right arrow: the row scrolls by three quarters of its width,
  stops at its end because the tabs run out first, and the last tab ends flush
  against the arrow zone with no overlap.
- Hovering either arrow lights its well before the click: the row pixel at the
  arrow is `tab_hover`, not the strip background.
- The long-named tab made active: its title stops with a clear gap before the
  pin, the cross sits inside the tab's inset, and neither is drawn over the text.
- A fresh window with three files: the arrows are absent -- nothing to scroll --
  and the row ends after the last tab with the strip's background to its right.

`cargo fmt` was not run.

## Fifty-eighth pass: a drag is a position, not a payload

### The gesture had two destinations and three meanings

`drop_tab` knew one thing: which pane the pointer was released over. Inside its
own pane a dropped tab did nothing, because a drag that began on the strip could
only mean "send this tab somewhere else". Notepad++ reads the same gesture as
three: released on the other pane it moves there, released on another tab of its
own strip it changes places with it, released anywhere else it goes back. The
third meaning is a reorder, and it was the one missing.

Adding it as a case in `drop_tab` would have been the small change and the wrong
one. A reorder decided on the drop has no feedback until the release, so the user
has to aim at a slot rather than push a tab along. That is why the reorder runs
during the drag instead: `drag_tab_to` moves the tab on every pointer move, and
the strip is the progress bar.

### Live reordering cannot flicker, which is why it needs no threshold

The reason Notepad++ can reorder live, and the reason no hysteresis is needed
here, is that moving a tab out of the slot under the pointer and into it puts the
same tab back under the pointer. Every intermediate state has the dragged tab
where the mouse is; only the order of the others changes. So the plain hit test
`tab_at` is safe to consult every frame, and a half-way threshold -- the usual
guard against a boundary that flips back and forth -- would only add a dead zone
to a gesture that has none.

`move_tab` is separate from the gesture and returns nothing: both indices are
checked and an out-of-range one is ignored rather than clamped. The index arrives
from a hit test that ran against the previous frame's slots, so "the tab moved
away under my pointer" is a normal race here, not a bug, and clamping would turn
it into a silent jump to the end of the row.

### The end of the row is the one place `tab_at` cannot answer

`tab_at` answers only where there is a tab, so a tab dragged past its last
neighbour stopped one short of the end and could never be put last -- the most
common reorder of all. `reorder_end` fills that hole: on the row, past the last
slot's right edge, it answers the last index. Off the row it answers nothing, so
a drag that wanders down into the text moves nothing on the way.

`State::past_strip_end` puts the pane's own rectangle in front of it, and the
check is not decoration. Without it the strip's band extends to the bottom of the
window, and a tab being dragged towards the other view would slide along its own
strip the whole way -- a reorder nobody asked for, performed while attempting a
move.

### The index follows the tab; the document is not touched

After a move, `self.active` is set to the new index, because the active tab is
still the same tab: it simply has more neighbours to its left now. Nothing is
reloaded, no history is dropped, and the other view is resynced only so that a
cloned tab shows the same title in the same slot order. `tab_drag` and
`tab_press` are rewritten to the new index as well -- the press coordinates were
taken in the old slot, and a press that still pointed at the old index would make
the next pointer move drag the wrong tab.

### Verification

`cargo build --offline --release`: clean.

`cargo test --offline --release`: 957 passed in `nn-app`, 816 in `nn-core`, 1773
in all, no failures. Three tests are new in `tabbar`: `a_tab_can_be_moved_along_its_strip`
(forward and backward, asserting the order of the whole row both times),
`moving_a_tab_nowhere_leaves_it_alone` (a self-move and two out-of-range moves),
and `a_drag_past_the_last_tab_lands_on_the_end`, which asserts `reorder_end`
answers the last index past the row, abstains inside the last tab because there
is a tab to hit there, and abstains below the row.

`cargo clippy --offline --release --all-targets`: 0 warnings.

Flows, driven with the mouse at 1100x750 on a 1400x1000 Xvfb on `:9`, five files
open, read off screenshots of the strip row:

- `beta.txt` dragged from the first slot to the third and released: the row is
  `gamma, alpha, delta, epsilon, beta` -- the drop was past the right end, so the
  tab landed last, which is the branch `tab_at` cannot see.
- The same window earlier with `alpha.txt` dragged from the first slot to the
  third and released inside the row: `beta, gamma, alpha, delta, epsilon`, from
  a pixel scan of the row at the tab row's centre showing the active tab's fill
  in the third slot.

`cargo fmt` was not run.

## Fifty-ninth pass: a plugin can be put down and picked up again

### Seven host functions is a list, not an interface

The ABI had seven imports: log a line, read the text, replace the text, insert at
an offset, ask where the caret is, move it, ask how long the text is. That is
enough to write a plugin that rewrites the whole document and nothing else, and
not enough to write any of the commands a plugin repository is actually full of,
because those are all about a *range*: surround this selection, align these lines
on a separator, convert the width of this run. With seven functions each of them
has to copy the document in, find its own range in the copy, rebuild the whole
string and push it back -- which loses the selection, loses the caret, and turns
a three-character edit into a full-document replacement in the undo stack.

The list is fifteen now, and the eight additions are chosen by what the ports
needed rather than by what was easy to expose: `text_replace_range` for an edit
that is not a whole-document rewrite, `line_count`, `line_start`, `line_len` and
`line_of_pos` so a line-oriented command does not have to copy the document to
find out where a line begins, and `sel_start`, `sel_end` and `set_sel` so a
command can work on the selection and leave a selection behind.

They are additions and nothing else. A module compiled last month imports the
names it was built against, and WebAssembly instantiation fails if a single
import is missing -- there is no version negotiation here, and adding one would
mean every plugin carries a version check that the host could do for it. So the
rule the file states is the rule the code keeps: the import list is append-only,
and a name in it never changes meaning.

### The offsets are bytes, converted at one boundary

Everything crossing the boundary is a byte offset, because a plugin written in
Rust, C or Zig can measure the UTF-8 it is handed in bytes and nothing in the ABI
promises a character. The editor counts characters. `Document::char_to_byte` is
the mirror of the `byte_to_char` that was already there, and it is the only
conversion: the window and the benchmark build their `Context` through it, so a
plugin cannot be handed a character offset by one caller and a byte offset by
another. Line indices stay line indices, as the names say, and the offsets the
line functions return are bytes again.

### The three ports are the argument that the ABI is enough

`plugins/` is a second workspace, so a plugin builds without the editor and the
editor does not link a plugin. Three ports ship with it: `align` (Align on
Equals, on Colon, on Comma), `halfwidth` (To Half Width, To Full Width) and
`surround` (quotes, brackets, parentheses, and Select Quoted Text). They are what
the `nn-plugin` SDK is for: `plugin!` generates the four exports, and the `host`
module is the fifteen imports as typed Rust functions, so a port is a function
from `&str` to `String` plus a decision about which range it applies to.

They are clean-room reimplementations of commands whose behaviour is described in
the feature reference, not ports of anyone's source. The repository has no
`LICENSE` file, so there is no licence under which GPL-derived code could be
taken in; the README says so, and says why, rather than leaving the question to
be asked later.

### Unloading drops the module and keeps the path

Load and unload are not a restart. `Plugins` keeps the running set, and `unload`
takes a plugin out of it, drops the `Instance` and keeps only its name and the
path it came from; `load` reads that file again and instantiates it. Re-reading
is the point: a plugin that was rebuilt while the editor was open comes back as
the new build, where keeping the bytes would bring back the old one and make
"reload" mean two different things in two places.

Nothing about this is persisted. The directory is the list of what is installed
and the session is the list of what is running, so unload is a decision about
this window, not a configuration change, and the next start loads everything
again. `Reload Plugins` re-scans the folder, which is also how a hand-copied
`.wasm` gets picked up and how an unloaded plugin comes back.

A file that has disappeared since it was unloaded is the one case that has to
answer rather than act: `load` reports the error, the plugin stays in the unloaded
list, and the menu keeps offering to load it. Forgetting it would be tidier to
write and worse to use -- the row that vanishes is the only explanation the user
would get.

### The verb is translated and the plugin's name is not

`Unload Half Width` is two strings from two sources: `Unload`, which is prose and
belongs to the catalog, and `Half Width`, which is a plugin's own name and
belongs to nobody else. `Item::composed` keeps both, the English one for
`english_labels` and tests, the joined one for drawing. It also means the rows are
*rebuilt* when the language changes rather than relabelled by lookup, because a
label of two halves has no single key to look up. `MenuBar::set_catalog`
regenerates the plugin menu from the last `PluginSource` it was given, and
`sync_plugin_menu` is the only place one is built, so the menu cannot drift from
the running set.

The unload and load actions carry an index and are deliberately not bindable: a
keyboard shortcut to "unload the third plugin" is a shortcut to the wrong plugin
the moment one is added. The index is checked against the list on arrival, so a
stale one is ignored rather than panicking.

Three strings were added to all twenty-three catalogs -- `Reload Plugins`, `Load`,
`Unload` -- and the completeness test that walks every catalog still passes, which
is what keeps a half-translated menu from shipping.

### The rows are built translated, because a relabel cannot rebuild them

The completeness test was not enough, and the reason is the interesting part: it
builds a bar with no plugins, so the composed rows never enter the list it walks.
Adding a test that loads the *real* `italian.xml` and installs a plugin under it
found the menu drawing `Unload Half Width` inside an otherwise Italian menu.

The bug was ordering. `set_plugins` assigned the fresh rows and then called
`relabel`, which walks every item and sets each label from its English text --
correct for a row whose label is a translation, destructive for one whose label is
a verb plus a name, because its English text is the untranslated row. The
synthetic test called `set_plugins` before `set_catalog` and so never took that
path; the application calls `set_catalog` first, from the saved language, and then
imports plugins, which is exactly the order that broke.

So the plugin menu is now built translated and nothing relabels it: `plugin_items`
takes `row(english, action)` for its prose (`Item::composed` with the catalog's
text), and `set_plugins` no longer calls `relabel` at all. `set_catalog` still
regenerates the whole menu from the stored `PluginSource`, which is what a
language change needs. Two tests hold this down:
`every_bundled_catalog_translates_the_plugin_rows` walks all twenty-three bundled
catalogs for the three composed labels, and
`the_italian_catalog_translates_the_unload_verb` drives the startup order with the
real file and asserts the row reads `Scarica Half Width`.

### Verification

`cargo build --offline --release`: clean, `nn-mcp` included.

`cargo test --offline --release`: 974 passed in `nn-app`, 816 in `nn-core`, 51 in
`nn-mcp`, 1841 in all, no failures. Ten tests are new for this pass: in `plugin`,
`unloading_a_plugin_takes_it_out_of_the_menu`,
`a_plugin_loads_back_from_its_file`, `a_plugin_whose_file_is_gone_stays_unloaded`
and `loading_a_plugin_that_was_never_unloaded_is_refused`; in `menu`,
`an_empty_plugins_menu_says_so`, `a_plugin_command_becomes_a_row`,
`an_unloaded_plugin_gets_a_row_that_loads_it_back`,
`the_unload_verb_follows_the_language_but_the_name_does_not`, which checks that
switching to French changes the verb and leaves `Half Width` alone, and the two
real-catalog tests above.

`cd plugins && cargo test --offline --release`: 23 passed -- 6 in `align`, 7 in
`halfwidth`, 8 in `surround`, 2 in `sdk`.

`cargo clippy --offline --release --all-targets`: 0 warnings.

Flows, driven with the mouse at 1100x750 on a 1400x1000 Xvfb on `:9`, read off
screenshots:

- The `Plugins` menu with three plugins installed: `Open Plugins Folder...`,
  `Reload Plugins`, a separator, nine command rows, a separator, three
  `Unload <name>` rows.
- `Unload Half Width` clicked: the status bar reads `Unloaded Half Width`, that
  plugin's two command rows and its unload row are gone, and a `Load Half Width`
  row is at the end of the menu.
- `Load Half Width` clicked: the status bar reads `Loaded Half Width`, the two
  command rows and the unload row are back, in their original order.

The Italian menu was read off a screenshot too, and it is where the ordering bug
above was first seen: `Apri cartella dei plugin...` and `Ricarica i plugin` beside
`Unload Align Columns`. It is not repeated as a flow, because the fix is checked
by the real-catalog tests rather than by the picture that exposed it.

`cargo fmt` was not run.

## Sixtieth pass: a reference, a PDF, and a server that answers from both

### A reference is a reading of the code, not a copy of a manual

`docs/nekopp-features.md` is the whole feature set in prose: 27 chapters, 75
subsections, 102 sections, and the menu tree spelled out row by row wherever a
chapter is about a menu. It was written by reading this code -- the menus, the
keymap, the editor, the theme and localization tables -- and not by reproducing
Notepad++'s manual, which describes a program with DLL plugins and a Scintilla
scrolling model that this one does not have.

Where the build differs from the program it imitates, the chapter says so instead
of smoothing it over: plugins are WebAssembly, the four Help links still point at
Notepad++'s pages because those are the closest documentation that exists, and the
Unicode chapter names the one font the program ships and the licence it is under.
A reference that quietly disagrees with the program is worse than no reference,
because it is confidently wrong.

### The converter is the small part, and it has no dependencies

`docs/build-pdf.py` is a markdown-to-HTML converter for the subset the document
actually uses -- headings, paragraphs, lists, tables, fenced code, block quotes,
rules, and inline code, bold, italic and links -- plus a call into WeasyPrint,
which is looked up on `PATH` first and at its per-user location second, because
that is where `pip install --user` puts it on a machine without root. No
third-party Python: a manual that needs a toolchain to rebuild is a manual that
goes stale.

The slug rule is shared with the server's resource URIs, so `nekopp://features/folding`
and the `#folding` anchor in the PDF name the same section. `--out` writes
elsewhere, `--html` keeps the intermediate, and the CSS beside it is A4 with a
running header, a page number, and a font stack that covers Greek, Cyrillic,
Hebrew, Arabic, Hangul, kana and Han -- checked by extracting the text layer of
the built file and counting characters from each block, not by trusting the font
list.

`--check` rebuilds into a temporary file and compares the bytes with the PDF on
disk, exiting 1 when they differ. It is exact rather than a guess because the
rendering is deterministic: no timestamp goes into the file, so the same markdown
gives the same bytes. That is what makes it an answer to "did the reference go
stale?" instead of a comparison of modification times, which a checkout gets
wrong.

### The server has no dependency on the window

`crates/nn-mcp` is a separate crate with no dependency on `nn-app`: it builds and
runs on a machine that has never started the GUI, and the binary is `npp-mcp`. The
markdown is embedded with `include_str!` and cut into sections at startup, so an
answer cannot drift from the PDF -- they are the same bytes.

It speaks JSON-RPC 2.0, one object per line, protocol revision `2024-11-05`, and
five tools: `search_features`, `get_section`, `list_sections`, `list_shortcuts`
and `get_command`. Every section is also a resource. The rules that make a
hand-written server usable are the unglamorous ones, and they are all tested: a
notification is never answered, not even when its method is unknown; an unknown
method is `-32601`, bad params `-32602`, a line that is not JSON `-32700`, a
resource that names nothing `-32002`; and a tool that fails -- an unknown section,
an unknown command -- is a successful call with `isError: true` and text saying
what to try instead, because a protocol error tells the user nothing. stdout is
the protocol and nothing else; the startup line goes to stderr.

Search ranks an exact heading above a heading that contains the query, above one
word of it, above a mention in the body, and whole words above fragments, so "bin"
does not return every page that says "bindable".

### The counts are a contract, and the document grew

Three tests pinned the document's shape: 27 chapters, 73 subsections, 100
sections, 101 resources. Adding the two plugin subsections in this pass turned all
three red, which is what they are for. The numbers are now 75, 102 and 103, and
the README quotes them.

The Plugins chapter itself was rewritten in the same change. Asking the server for
`get_command "Unload"` returned an honest refusal -- a menu row is not a bound
command -- and reading the chapter afterwards showed why that mattered: it still
described seven host functions and a Plugins menu with one folder row and no
unload, which is what the previous pass had already replaced. The chapter now
documents the fifteen-function ABI, the append-only rule, the Rust SDK with its
three ports, and the load/unload rows.

### Verification

`cargo build --offline --release`: clean.

`cargo test --offline --release`: 974 passed in `nn-app`, 816 in `nn-core`, 51 in
`nn-mcp`, 1841 in all, no failures. `cd plugins && cargo test --offline --release`:
23 passed.

`cargo clippy --offline --release --all-targets`: 0 warnings.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 60 pages A4, 262,523
bytes, 104 bookmarks. `pdftotext` on the result finds the words "Load and
unload", "fifteen host functions" and the French example `Décharger Half Width`,
and counts characters in the Hebrew (29), Arabic (52), Greek (30), Cyrillic (46),
Hangul (11), Han (12) and kana (23) blocks, so the scripts are in the text layer
rather than drawn as boxes.

A piped session against `target/release/npp-mcp`, four lines in:

- `initialize` answers `{"name":"npp-mcp","version":"0.1.0"}` with capabilities
  for tools and resources.
- `get_command "Unload"` answers `isError: true` and the text "No command is called
  "Unload". `list_shortcuts` has the bound ones." -- the correct answer, since that
  row is a menu item and not something a key can be bound to.
- `resources/read nekopp://features/plugins` returns the Plugins chapter as
  markdown.
- stderr carries `npp-mcp 0.1.0: 102 sections, 73 shortcuts, one JSON object per
  line on stdio`, and stdout carries nothing but the four responses.

`cargo fmt` was not run.

## Sixty-first pass: installed was never the same word as running

The plugin folder used to be the whole story. A `.wasm` file was in it, so the
editor ran it, and the only way to stop that was to delete the file. For the one
file type in this program that can rewrite the document without being asked, that
is the wrong default twice over: it runs code nobody approved, and it leaves the
user who wants to look at a plugin before trusting it with no way to say so. This
pass separates the two states. A file in the folder is *installed*. A plugin with
rows in the menu is *running*. `Plugins > Manage Plugins...` is the only way from
the first to the second, and everything starts off.

### Discovery reads the name and stops there

The scan has to know each plugin's name to have a row to draw, and a name is a
pointer into the plugin's own linear memory -- `npp_name` returns an offset, not a
string -- so there is no way to learn it from the file without instantiating the
module. `probe` does precisely that much: load, call `npp_name`, then
`npp_command_count` and `npp_command_name` for each index, keep the three strings,
drop the instance. `npp_run` is never called by a scan, so a plugin that is
switched off has never had a command run; what it can execute at load time is
bounded by the same step budget and the same fifteen host functions, none of which
has a document behind it until a command is given a bridge.

The command count is kept beside the name. It is one call, and it is what lets the
manager say something true about a plugin it is not running.

### The settings file holds names

`settings.txt` gains one `plugin <name>` line per running plugin, in folder order.
Names rather than paths: a list of paths breaks the first time a plugin is
rebuilt and reinstalled, and `build.sh` copies the same file over the same file,
so the name is the stable part. A name that matches nothing on disk is ignored
rather than reported -- deleting a `.wasm` is the whole of uninstalling it, and an
editor that then complained at every start would be arguing with a decision the
user already made.

Startup is `Plugins::discover` and then `enable_saved(&settings.enabled_plugins)`,
in that order, so a file that is on disk and not in the settings is listed and
stays off -- which is the case this whole pass is about. `Reload Plugins` is the
same pair with the running names read from the loaded set first, so a rebuild
picked up by a reload comes back with the same plugins on.

`enable` re-reads the file rather than keeping the module it instantiated during
the scan. Holding one instance per plugin so that enabling is free would mean
every plugin in the folder is instantiated and resident from startup, which is the
thing the folder is no longer allowed to decide.

### The manager reports, the window performs

`PluginList` owns rows, a cursor and a viewport, and nothing else. Space on a row
returns `(index, enabled)` to the window, which does the three things a modal
should not: it switches the module, writes the settings file, and rebuilds the
menu. That is the split the preferences dialog already uses for its settings, and
for the same reason -- a modal that owns a file handle is one you have to trust
with the file, while a modal that returns an index can be tested without a window.
The toggle path is `set_plugin_enabled`, which calls `remember_plugins` (rewrite
the settings from the list of what is actually running, not by editing a line in
place), then `sync_plugin_menu`, then hands the rows back, so the list and the
menu are never a keystroke out of step with the state.

Each row is `(name, file, enabled)`. The file name is drawn in the hint line under
the cursor rather than in the row: for every plugin in this tree it is the same
string as the name, and it earns its line the moment somebody has two builds of
the same plugin side by side.

### Every plugin starts off, and the menu says why it is empty

A copy of a `.wasm` into a folder is not consent to run it, and default-off also
keeps the Plugins menu honest in size: five plugins with seventeen commands
between them make a dropdown longer than most in the bar, which is fine when a
user asked for it and noise when they did not.

Nothing vanishes when they are all off. The menu keeps three rows -- Manage
Plugins..., Open Plugins Folder..., Reload Plugins -- then one `Load <name>` row
per plugin, which is both the state and the instruction for leaving it, and with
an empty folder a final `No Plugins Installed`, because an empty dropdown reads as
a bug rather than as an empty list.

### Five keys, twenty-three files, one source of words

The manager is chrome, so its words come from the catalog: `Manage Plugins...`,
`Plugin Manager`, `Enabled`, `Disabled` and `Space enables or disables, Esc closes`
are five `<Item>` rows added to all twenty-three localisation files directly after
`Unload`. `PluginList::labels` feeds them into `chrome_labels()`, which is what
makes a language that is missing one fall back row by row instead of drawing a
dialog in two languages at once.

The plugin names stay untranslated and the verbs beside them do not, so
`Unload Half Width` is rebuilt from a translated verb and an untranslated name on
every language change -- the same rule those rows had before this pass, and the
reason they are `Item::composed` rather than relabelled.

### Verification

`cargo build --offline --release`: clean, 2m12s.

`cargo test --offline --release`: 990 passed in `nn-app`, 816 in `nn-core`, 51 in
`nn-mcp`, 1857 in all, no failures. `cd plugins && cargo test --offline
--release`: 47 passed (6, 7, 8, 12, 12 and 2 across the six test binaries).

`cargo clippy --offline --release --all-targets`: 0 warnings.

The window, on `:9` at 1100x750, with a fresh profile and the five `.wasm`
installed:

- `Plugins` shows `Manage Plugins...`, `Open Plugins Folder...`, `Reload Plugins`,
  then `Load Align Columns`, `Load Half Width`, `Load JSON`, `Load Surround`,
  `Load XML` -- all five off, none of them offering a command.
- The manager opens titled `Plugin Manager` with five rows reading `Disabled`, and
  the hint under the cursor reads `npp_align.wasm  Space enables or disables, Esc
  closes`.
- Two arrows and Space: the JSON row reads `Enabled`, the status bar says
  `Loaded JSON`, and `settings.txt` holds exactly one `plugin JSON` line.
- The menu, reopened, now carries `JSON - Pretty Print`, `JSON - Minify`,
  `JSON - Validate` and `Unload JSON`, with the other four still at `Load ...`.
- `JSON - Pretty Print` on a document holding `{"a":1}` leaves three lines, `{`,
  `"a": 1` and `}` -- length 12, three lines in the status bar -- which is the same
  edit the same command makes through `bench`, which prints `plugin run: 12
  chars` for `plugin_run=2:0` against the same five files.

An earlier alarm in the same check -- `invalid JSON at line 1, column 7` -- was my
own harness: `bracketright` without `shift` types `]` where `]` was meant to be
`"`. The plugin was right and the typist was wrong.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 61 pages A4, 265,740
bytes, 104 bookmarks. `pdftotext` finds "Manage Plugins" three times, "Space
toggles the row" twice, `plugin <name>` twice, "No Plugins Installed" once and the
French example `Décharger Half Width` once, so the new prose is in the text layer
rather than in an image.

`bench` grew `plugin_manager=<dir>`, which fills the manager with the rows that
directory holds and renders it, so the frame cost of the modal is measured rather
than assumed: 1400x900 with the five plugins listed and the dialog open costs
0.23 ms p99 idle and 0.21 ms p99 scrolling, which is under the cost of the
document behind it. `plugin_run=2:0` against the same folder prints
`plugin run: 12 chars`.

`cargo fmt` was not run.

## Sixty-second pass: one switch, in one place

The manager was added and the dropdown kept its `Load <name>` and
`Unload <name>` rows, which left two ways to change the same flag. The user asked
for the rows to go, and they were right for three reasons. The rows moved: every
toggle shifted every later row, so the next click landed on the neighbour of the
plugin the user was looking at. They read as an invitation to switch plugins on
from a menu of commands, which is the manager's job and was already the first row
of the same menu. And they made the state legible in two places, one of which had
to be rebuilt on every keystroke to keep up.

### What a plugin menu is now

Three rows that never change -- Manage Plugins..., Open Plugins Folder...,
Reload Plugins -- then a separator and one row per command of every running
plugin. Nothing else. Switching a plugin on or off happens in the manager, which
lists every plugin in the folder rather than the running half, and which the
dropdown's first row opens.

### The one state that had to be decided

`PluginSource` carried `loaded` and `unloaded`, which is exactly what the deleted
rows needed and nothing else did. It now carries `commands` and `installed: usize`
-- the number of `.wasm` files found, running or not. That is one field to keep
the empty-state row honest: an empty folder still says `No Plugins Installed`,
because an empty dropdown reads as a bug, while a folder holding five switched-off
plugins draws the three folder rows and no placeholder. Calling that state "none
installed" would be a lie in the one place a user goes to check what is installed.

### What came out with the rows

`Plugins::load`, `Plugins::unload` and `Plugins::unloaded` only existed to turn a
running-plugin index into a folder index for those rows; `enable`, `disable` and
`inventory` take the folder index the manager already has, so the three went, with
the tests that described them rewritten against `disable`/`enable` instead of
deleted where they covered real behaviour. `Action::LoadPlugin` and
`Action::UnloadPlugin` went the same way, with their dispatch arms in `main.rs`.
The `Load` and `Unload` `<Item>` rows left all twenty-three localisation files:
a translation for a word the window cannot draw is a row a translator has to keep
translating for nothing.

Two menu tests went with the rows (`an_unloaded_plugin_gets_a_row_that_loads_it_back`,
`the_unload_verb_follows_the_language_but_the_name_does_not`) and one arrived
(`plugins_that_are_all_switched_off_get_no_placeholder`). The Italian test stopped
checking the unload verb and now checks the manager row, which is the only plugin
row left whose words come from a catalog. `every_bundled_catalog_translates_the_plugin_rows`
was deleted because the completeness test already walks the three fixed rows for
every language.

### Verification

`cargo build --offline --release`: clean.

`cargo test --offline --release`: 987 passed in `nn-app`, 816 in `nn-core`, 51 in
`nn-mcp`, 1854 in all, no failures. `cd plugins && cargo test --offline --release`:
47 passed.

`cargo clippy --offline --release --all-targets`: 0 warnings.

The window, on `:9` at 1100x750, with the five `.wasm` installed:

- JSON switched on: `Manage Plugins...`, `Open Plugins Folder...`, `Reload
  Plugins`, a separator, then `JSON - Pretty Print`, `JSON - Minify`,
  `JSON - Validate`. No `Unload JSON`, no `Load ...` rows.
- Space on the JSON row in the manager: the menu is the three fixed rows and
  nothing else -- no placeholder, because the folder still holds five files.
- `settings.txt` lost its `plugin JSON` line as the row flipped off, which is the
  same write path the manager has always used.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 61 pages A4, 265,833
bytes, 104 bookmarks.

`cargo fmt` was not run.

## Sixty-third pass: the menu keeps only the switches, the server runs the commands

The dropdown still listed one row per command of every running plugin, and the
user said no: nothing about a loaded plugin belongs in it. They were right, and
the reason is the same one that removed the `Load`/`Unload` rows a pass earlier.
A row per command is a row that arrives when a plugin is switched on and leaves
when it is switched off -- under a pointer that may be over the row that moved --
and the manager, one row up, already answers "what is installed and what is it
doing". What was missing was not the row. It was a way to *use* a command once
the editor stopped advertising it, which is the second half of the request: the
plugins a machine has should be part of what `npp-mcp` can do.

### What the dropdown is now

Three rows, always the same three, and a note when the folder is empty:

```
Manage Plugins...
Open Plugins Folder...
Reload Plugins
No Plugins Installed        (only when the folder holds nothing)
```

`plugin_items` takes a `usize` instead of a `PluginSource`, and the bar keeps
that number so a language change rebuilds the note in the new language. There is
no `Action::Plugin(plugin, command)`, no dispatch arm, no `run_plugin`, and the
`plugins` menu no longer needs to be rebuilt on every toggle -- which is also why
the dropdown cannot change length under the pointer any more.

### Where a command goes instead

`npp-mcp` grew two tools. `list_plugins` answers with every installed plugin:
name, file, on or off, and its commands, plus the exact `run_plugin_command`
arguments for the first one that is runnable -- a hint that names something that
exists on this machine rather than in the documentation. `run_plugin_command`
takes `plugin`, `command` and `text`, runs the command over that text through
the same interpreter, and answers with what the plugin left:

```
| JSON | `npp_json.wasm` | on | Pretty Print, Minify, Validate |
| XML  | `npp_xml.wasm`  | off | Pretty Print, Linearize, ... |
```

Two rules the implementation turns on. First, a plugin switched off in the
manager is switched off over the protocol as well: the server reads the same
`plugin <name>` lines from `settings.txt`, so the protocol cannot be a way round
a switch the user has already set. Second, the answer is two content blocks at
most -- the document on its own, then what the plugin reported. The document
block has to be usable as it stands, because a caller that asked for minified
JSON should not have to strip a header off it; and a command that only moves the
caret, or only logs, would otherwise look like it did nothing. A selection set
by the plugin and the caret it left are reported in the second block, in bytes,
for the same reason: a protocol client has no editor to look at.

The folder is read at startup and again when a call needs something it does not
have, so a plugin switched on in the editor five minutes ago is usable by the
next call rather than by the next process. Reading it is a scan: each module is
instantiated once to read its name and commands and then dropped.

### What moved

None of that could be built on the old layout, where the plugin host lived in
`nn-app` beside the window. `plugin.rs` and `paths.rs` moved to `nn-core`
(31 and 18 tests with them, which is why `nn-app` went from 987 tests to 938 and
`nn-core` from 816 to 866). `nn-app` re-exports both, so `nn_app::plugin` and
`nn_app::paths` still name the same things and no call site changed.

Two consequences worth naming. `Plugins::commands()` and its `Command` type
existed to build menu rows; with the rows gone it had no caller, so it went, and
the inventory's `commands` field became a `Vec<String>` of the names instead of
a count of them -- the count was never read by anything, and the names are what
`npp-mcp` lists. The names come from the running module when there is one and
from the scan otherwise, so a plugin rebuilt on disk and switched on again
reports the commands it has now.

The rule for what a `plugin <name>` line means became `nn_core::plugin::enabled_line`,
used by `Settings::parse` and by the server. Two readers, one rule: `npp-mcp`
must not disagree with the editor about whether a plugin is on, and a rule
written twice is a rule that disagrees the first time one side is corrected.

`nn-core` gained a `fixture` feature: the two-command test plugin that used to
live inside `#[cfg(test)]` is a module now, because `nn-mcp` tests its tools
against the same bytes. `nn-core`'s own tests turn it on with `cfg(test)`;
`nn-mcp` asks for it as a dev-dependency, so neither binary ships it.

### Verification

`cargo build --offline --release`: clean, 2m14s. `cargo test --offline --release`:
938 passed in `nn-app`, 866 in `nn-core`, 70 in `nn-mcp`, 1874 in all, no
failures. `cd plugins && cargo test --offline --release`: 47 passed.
`cargo clippy --offline --release --all-targets`: 0 warnings.

The window, on `:9` at 1400x1000, with `npp_json.wasm` and `npp_xml.wasm`
installed and `plugin JSON` in `settings.txt`:

- `Plugins` opens on exactly three rows -- `Manage Plugins...`, `Open Plugins
  Folder...`, `Reload Plugins` -- with one plugin switched on and two installed.
  No command rows, no `Load ...`, no `Unload ...`, and no `No Plugins Installed`
  because the folder is not empty.

A session with the same profile, driving `target/release/npp-mcp` over stdio:

```
npp-mcp 0.1.0: 103 sections, 73 shortcuts, 1 of 2 plugins enabled, ...
```

- `list_plugins` returned both plugins, `JSON` on and `XML` off, with
  `Pretty Print, Minify, Validate` and the five XML commands, and the example
  `{"plugin": "JSON", "command": "Pretty Print", "text": "..."}`.
- `run_plugin_command` on `JSON`/`Minify` over a three-line document answered
  `{"a":1,"b":[2,3]}` in the first block and `selected bytes 0..17` in the second.
- On `JSON`/`Validate` over `{"a": }` it answered with the text unchanged and
  `invalid JSON at line 1, column 7: expected a value` plus `caret at byte 6`.
- On `XML`/`Pretty Print`, which is switched off, it was a tool error:
  `plugin "XML" is switched off; switch it on in Plugins > Manage Plugins...`.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 62 pages A4,
268,620 bytes, 105 bookmarks. `pdftotext` finds "Plugins over MCP" and
`list_plugins answers with one row per installed plugin` in the text layer, so
the new subsection is in the PDF rather than only in the markdown.

`cargo fmt` was not run.

## Sixty-fourth pass: a plugin's commands live under its name

The previous pass took every plugin row out of the dropdown and put the commands
behind `npp-mcp`. The user came back with the other half of the sentence: a
protocol client is not the editor, and "reachable over the protocol" is not
reachable for someone sitting in front of the window. What they asked for is what
Notepad++ does -- `Plugins > JSON > Pretty Print` -- one submenu per plugin,
holding that plugin's commands.

The earlier objection still holds, and the shape answers it. A row per command,
flat in the dropdown, is a row that appears and disappears under the pointer. A
*submenu* per plugin is one row per running plugin, and a submenu is only opened
deliberately, so nothing moves under a pointer that is not already inside it.
That is why the previous pass was not wasted: it established that the three
management rows are the fixed part, and this pass puts the variable part where it
cannot be reached by accident.

### What the dropdown is now

```
Manage Plugins...
Open Plugins Folder...
Reload Plugins
----------------------------
JSON
    Pretty Print
    Minify
    Validate
No Plugins Installed        (only when the folder holds nothing)
```

The three switches, a separator, then one submenu per *running* plugin, named
exactly as the plugin names itself. Switched-off plugins get no row: the folder
is what is installed and the manager says which of those are running, and a
submenu that opens onto nothing would be a third place to look for the same
answer. The `No Plugins Installed` note is still there for an empty folder -- the
note returns early now, so a folder with two plugins in it and none switched on
says nothing rather than claiming the folder is empty.

`MenuBar` keeps `plugins: Vec<PluginMenu>` instead of an `installed_plugins`
count, `PluginMenu { name, enabled, commands }`, and `set_plugins` replaces the
old `set_plugins(len)`. `set_catalog` rebuilds from the stored rows, so a language
change rebuilds the three management labels without losing the submenus.
`sync_plugin_menu` in `main.rs` maps `Plugins::inventory()` into those rows and
is called wherever the manager used to rebuild the menu.

### Indices, not names

`Action::Plugin(usize, usize)` is the inventory index and the command index, not
two strings. The dispatch arm looks the plugin up in the inventory that the row
was built from, so a plugin renamed on disk between the build and the click still
runs the command that was clicked, and two plugins with a command of the same
name cannot be confused -- `JSON > Pretty Print` and `XML > Pretty Print` both
exist, and both do different things. `run_plugin` came back from `6624b08^`
unchanged: it copies the document into the guest, applies a whole-document
replace as one undo step, puts the caret and selection back where the plugin left
them, and prints the plugin's log lines as `[plugin] ...`.

The variant is deliberately absent from `BINDABLE` and from `Action::from_name`.
A plugin command is not a rebindable editor verb: its meaning belongs to a file
that can be replaced, and a `shortcuts.xml` naming one would break the first time
the plugin was rebuilt with a different command list. The three management rows
are still ordinary `Item::composed` rows with `Action::Bindable(Kind::PluginManager)`
and friends, so they are in the table and in every catalog.

### Why the rows are literal

`Item` gained a `literal: bool` with `literal()` and `literal_submenu()`
constructors. `relabel_items` walks past a literal row, and so does
`english_labels`, which is what the catalog completeness test compares against.
Without that flag the Italian catalog would print `Annulla` for a plugin's
`Undo` -- the editor's word for the editor's command, printed on a command it has
never seen -- and the same for `Minify` in any language that happens to have a
word for it. A plugin's vocabulary is the plugin's; the editor translates words it
wrote.

### Verification

`cargo build --offline --release`: clean. `cargo test --offline --release`: 940
passed in `nn-app`, 866 in `nn-core`, 70 in `nn-mcp`, 1876 in all, no failures --
six more than the previous pass, all in the menu tests:
`each_running_plugin_opens_onto_its_commands`,
`a_plugin_that_is_switched_off_gets_no_submenu`,
`a_folder_with_plugins_in_it_gets_no_note` (rewritten),
`a_plugin_submenu_keeps_the_plugins_words_in_every_language` (a plugin's `Undo`
stays `Undo` while Edit's `Undo` becomes `Annulla`),
`the_plugin_rows_that_take_no_index_are_bindable` and the reworked
`the_italian_catalog_translates_the_manager_row`.
`cd plugins && cargo test --offline --release`: 47 passed.
`cargo clippy --offline --release --all-targets`: 0 warnings.

The window, on `:9` at 1400x1000, with `npp_json.wasm` and `npp_xml.wasm`
installed and `plugin JSON` in `settings.txt`:

- `Plugins` opened on the three switches, a separator and one row, the `JSON`
  submenu. No `XML` row, because XML was switched off.
- Hovering `JSON` opened the nested panel: `Pretty Print`, `Minify`, `Validate`.
- Clicking `Minify` turned a three-line `/tmp/j.json` into `{"a":1}` in the
  editor, one undo step, caret where the plugin left it.
- The keyboard path ran it too: `Alt+P`, `Down` three times onto `JSON`, `Right`,
  `Down` twice onto `Minify`, `Enter`.
- Switching XML on in the manager made a second submenu, `XML`, appear in the
  same session, with no reload and no restart, and switching it off took it away.
- `XML > Check Well-Formedness` on JSON text logged `[plugin] not well-formed at
  line 1, column 1: text outside the root element`, which is the dispatch
  reaching the other plugin although both offer `Pretty Print`.

`docs/nekopp-features.md` was rewritten where it had gone stale: the `### Plugins`
entry in the menu reference now describes the submenus, `### Load and unload` says
the dropdown keeps the three switches and gives the space under them to the
plugins, `### Managing plugins` says switching a plugin on gives it a submenu
*and* puts its commands within reach of `npp-mcp`, `### Plugins over MCP` opens
by saying the submenu is one door and the protocol the other, and `### What is
translated` records that plugin rows are literal and why. No heading was added,
so `nn-mcp` still reports 103 sections and 27 chapters and no count in `doc.rs`
or `settings.rs` moved.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 63 pages A4,
270,987 bytes, 105 bookmarks. One page longer than before, same bookmark count,
because the new prose sits inside sections that already existed.

`cargo fmt` was not run.

## Sixty-fifth pass: the caret goes where the pointer is, and a drag selects

The complaint was one sentence long -- "the mouse when I click does not set the
cursor where should be (where i clicked)" -- and the cause was one line long.
`Editor::char_at_point` and `set_caret_from_point` both take a point *inside the
text area*: the renderer draws a line at `origin_y + content_top + viewport_y`,
and the editor's arithmetic is the inverse of that. The `click` handler in
`main.rs` converted the x coordinate (`x - view.text_left()`) and then handed the
raw window y straight through, so the menu bar, the toolbar and the tab strip --
80 px, four lines at this font -- were counted as text and the caret landed four
lines below the pointer. Nothing was wrong with the model; the conversion had one
half.

The gutter was the clue that gave it away, because clicking a line number had
always worked. `View::line_at` subtracts `top()` itself before asking the editor,
so the two handlers disagreed about what a "y" was and only one of them was said
out loud. Measured in the window before the fix: a click at y=138 reported `Ln 7`,
y=97 reported `Ln 5`, y=700 reported `Ln 36` -- every one of them
`floor(y / 20) + 1`, which is what a coordinate that has never been told about
the chrome looks like.

### The conversion, in one place

`View::text_point(&self, x: f32, y: f32) -> (f32, f32)` returns
`(x - text_left, y - top())`, with a doc comment saying it inverts the renderer
and that its result is what the editor's point-taking methods expect. Three call
sites use it: the Ctrl+click that adds a caret, the plain click, and the motion
handler. A private helper would have been enough for `main.rs` alone, but the
function documents the contract between two crates, and the next person to pass a
window position to the editor now has one function to find.

### The drag that never selected

The same handler had a second bug, found while testing the first one.
`handle_cursor` handed its third argument, the `extend` flag of
`set_caret_from_point`, `self.modifiers.shift_key()`. So the flag was whatever
Shift happened to be doing during the motion -- almost always false, which moves
the caret *and collapses the selection*, on every motion event. Dragging looked
dead: the status bar still read `Sel: 0` after a press-and-drag over four lines.
Passing `true` extends from the anchor the press left behind, which is what a
drag is. The rectangular path is untouched, because it re-derives the block from
`block_anchor` after the caret has moved.

### Verification

The window, on `:9` at 1400x1000, `npp --software /tmp/clicktest.txt`:

- Click at y=138: `Ln: 3, Col: 16`, and the caret drawn under the pointer at the
  character that was clicked. The same click read `Ln 7` before the fix.
- Click at y=97, the first text line: `Ln: 1`.
- Click at y=711, the last visible row: `Ln: 32`.
- Drag from line 2 to line 5: lines 2, 3 and 4 fully highlighted, line 5 up to
  the caret, status bar `Ln: 5, Col: 27, Sel: 179`.

`cargo test --offline --release`: 940 nn-app, 866 nn-core, 70 nn-mcp, 1876 in
all, 0 failed. `cd plugins && cargo test --offline --release`: 47 passed.
`cargo clippy --offline --release --all-targets`: 0 warnings.

`View` has no unit tests, and this pass did not add any: the type needs a font
system to exist, so the geometry it computes is checked in a window rather than
in a test binary. What *is* testable -- the editor side of that contract, that a
point inside the text area maps to the character under it -- was already covered
in `nn-core` and did not need to change.

`docs/nekopp-features.md` gained a `### The pointer` subsection under Editing and
the Caret Model, which says that a window position has the pane's chrome taken
off it before it becomes a line and a column, that a drag extends from the press
anchor, and what Ctrl, Shift and Alt do with a press. It also says plainly that
double-click word selection and triple-click line selection are not implemented,
and names the keyboard motions that reach the same ranges -- `Ctrl+Shift+Left`
for the word and `Shift+Down` for the line, checked against the default keymap
rather than assumed, because `Ctrl+Shift+Up` is `MoveLineUp` and would have been
the wrong half of a sentence.

That subsection is the first heading this reference has gained in a while, so the
counts it publishes moved with it: `nn-mcp` now reports 104 sections, 27 chapters
and 77 subsections, `list_sections` returns 105 lines and `resources/list` 105
resources. Three assertions in `doc.rs` and `server.rs` were updated for it, and
the pass is only correct with them: the whole point of hard-coding the shape is
that a heading cannot be added quietly.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 64 pages A4,
272,255 bytes, 106 bookmarks. One page and one bookmark more than before, both
from the new subsection.

`cargo fmt` was not run.

## Sixty-sixth pass: a click has three sizes

The previous pass closed by admitting that double-click word selection and
triple-click line selection did not exist. The question that came back was the
right one -- *why not* -- and the honest answer is that neither half of the
feature was there to be switched on.

On the window side, `WindowEvent::MouseInput` set `dragging` and called
`click(x, y)`. Every press was a first press: nothing recorded when the previous
one happened or where it landed, so two presses 80 ms apart at the same pixel
were two unrelated clicks. On the editor side there was nothing to call. `Editor`
had `select_line()` for the line *the caret is on* and `Document::word_at`, which
returns the word's text -- the wrong shape twice over, because a double click
needs the *range* of the unit at a given character so the selection can point at
it, and `word_at` walks outward from a character that is already inside a word.
The two helpers that do walk (`word_left`, `word_right`) are private to the motion
code and answer "where does the previous word start", not "what is the word
here". So the reference said the feature was missing rather than describing
something the window could not do, and this pass gives both halves what they need.

### The units, in the core

`Editor::word_range_at(pos)` returns the run of text at a character: a word, a
run of whitespace, or a run of punctuation, decided by a new `unit_class` next to
`is_word_char`. Three classes rather than two, because punctuation is a unit
people double-click on purpose -- the `.` in `foo.bar`, the `--` in a comment
rule -- and lumping spaces in with it would make a double click on a space select
the space *and* the `=` after it. The run never crosses a line ending, so a
double click on the space at the end of a line takes that space and not the
indentation below.

A position on a line ending, or past the end of the document, is clamped to the
last character of the line it is on. That is where the pointer actually is when
it is beyond the text, and selecting a newline is never what the second press of
a double click meant. An empty line returns `None`, and `select_word_at` then
leaves the selection alone: a double click on blank space past the text should not
throw away what the user had.

`Editor::line_range_at(pos)` is the triple click's unit, and it includes the line
ending -- `\n` or both halves of `\r\n`. Excluding it, the way `select_line()`
does for the keyboard command, would make a line selected this way paste as a line
glued to the next one.

### The press, in the window

`State` gained `click_count`, `last_press` and `unit_drag`. Counting happens at
the top of `click`, before any widget gets its turn, so the count does not depend
on what was hit: the second press on the text has to know it is the second even
if the first one closed a menu. A press repeats when it lands within
`CLICK_SLOP` (four pixels) and `CLICK_INTERVAL` (half a second) of the one before
it, and a third has to clear the same bar against the second, so a triple click is
three presses in a row rather than one fast pair and a straggler.

The text branch then returns early for a repeat: it takes the range of the unit
under the pointer, selects it, and remembers `(unit, start, end)` in `unit_drag`.
Returning matters. The single-click path ends by placing the caret from the same
point, which would collapse the selection the second press just made.

The drag handler checks `unit_drag` first. A drag out of a double click takes the
unit under the pointer and unions it with the anchor, in whichever direction the
pointer went, so dragging left from a word takes whole words on the left and
dragging back over the anchor shrinks the same selection. Without the anchor the
first motion would have replaced a word selection with a character selection,
which is what the previous pass fixed for plain drags.

Double and triple clicks are deliberately *not* exempt from the drag: a press,
then a drag before release, is how a person selects four words, and the unit is
what makes it four words rather than four characters.

### Verification

The window, on `:9` at 1400x1000, `npp --software /tmp/wordtest.txt`, five lines
of ordinary words, one of them `punct foo.bar(baz) end`:

- Double click on `beta`: `Ln: 1, Col: 11, Sel: 4`, `beta` highlighted.
- Triple click on line 3, `select_this whole_word`: `Ln: 4, Col: 1, Sel: 23` --
  22 characters plus the line ending -- with the whole line highlighted.
- Double click, then drag to `four` on line 2: `Ln: 2, Col: 19, Sel: 18`,
  `one two three four`.
- Double click on the `.` of `foo.bar`: `Ln: 4, Col: 11, Sel: 1`. The same pair
  one character earlier took `foo`, which is the clamp doing its job.
- A single click still places the caret and leaves `Sel: 0`.

`cargo test --offline --release`: 940 nn-app, 876 nn-core, 70 nn-mcp, 1886 in
all, 0 failed -- ten of the core tests are new and cover the word, the run of
spaces, the run of punctuation, the whitespace run that stops at a line ending,
the press past the end of a line, the empty line that selects nothing, a non-ASCII
word, the line with its terminator, the last line without one, and `\r\n`.
`cd plugins && cargo test --offline --release`: 47 passed.
`cargo clippy --offline --release --all-targets`: 0 warnings.

The `### The pointer` subsection now describes the three sizes of a click, the two
bounds that make a repeat, and the drag that pulls a selection out by the unit it
began with; the sentence saying the feature was missing is gone. No heading was
added or removed, so `nn-mcp` still reports 104 sections, 27 chapters and 77
subsections, and no count in `doc.rs` or `server.rs` moved.

`python3 docs/build-pdf.py`: wrote `docs/nekopp-features.pdf`, 65 pages A4,
273,429 bytes, 106 bookmarks. One page longer, same bookmark count, because the
new prose replaced a two-line paragraph inside a section that already existed.

`cargo fmt` was not run.

## Sixty-seventh pass: several files at once, and a pointer that stops turning

Three things came back from using the program rather than from reading it. The open
dialog took one file even when it was asked for several. A right click in the file
manager opened a window without the focus, and turned the pointer into a spinner
for seconds after the file was already on screen. And a cold start spent a third of
a second before anything appeared.

The first two are the same kind of gap: the window half was doing what the toolkit
does by default, and the default is not what the desktop expects.

### The dialog and the listing take more than one file

The portal's `Open` options now carry `multiple: true`. The answer was already a
list of URIs; without the flag every portal returns exactly one, however many the
user picked. `Save` does not ask, because its answer is one buffer written to one
path.

In the built-in browser, Ctrl+click toggles a row, Shift+click takes a range from
the last plain click, and Ctrl+A marks every row *in the listing*, which is the
only way to reach the rows below the fold. Marking is kept in the order the rows
were marked rather than as a sorted set, because Enter opens the selection in that
order and the file chosen last is the one left in front; a set would have made
"last" arbitrary.

`State::open_files` is the one place that loops, and `open_path` still decides what
"already open" means. The two callers answer in different orders: the browser
answers in marking order, and a portal answers in an order of its own, so there it
is the last name in the answer that wins and not the file clicked last. The
reference says so rather than claiming an order the program cannot know.

### Every way in makes the path absolute

`nn_core::paths::absolute` resolves against the working directory without touching
the disk. `canonicalize` is the wrong tool twice over: it fails on a path that does
not exist yet, and it resolves a symlink to a name the user did not type.

It is called on the way in, in `open_path` and in `Session::push_recent`, so the
tab, the session line and the recent entry all name the same file. An entry stored
relative was read back at the next start from whatever directory the editor
happened to be launched in, which is a click that does nothing for a reason the
user cannot see. The same rule collapses one file reached two ways into one recent
entry, and an entry whose file has since gone now says so in the status bar.

`restore_snapshot` takes a `named` flag. Recovering buffers used to activate the
last one, which undid the activation the file named on the command line had just
had; a file the user named is the one they asked to see, so the recovered buffers
queue behind it.

### The minimap is as tall as the document

The column was `text_height()` tall, so a five-line file was spread over the whole
edge and read as a five-hundred-line one. `Minimap::column_height(total, available)`
grows at two pixels a row, has a floor of three rows so a one-line file is still a
target worth clicking, and stops at the pane. The height depends on the document
rather than the pane, so `EditorView::minimap_rect` had to learn the line count;
its one caller already computed it.

### The launcher's token, and the sequence it waits for

A launch from the file manager arrives with `DESKTOP_STARTUP_ID` set. The token is
the desktop's record that the launch was the user's doing: the window that carries
it is the one that is given the focus. The window was carrying none, which is why a
window from a right click had to be clicked before it had the caret.
`read_token_from_env` and `with_activation_token` attach it, and
`reset_activation_token_env` clears it before anything here can spawn a child that
would inherit it and ask for the same focus.

The same variable names a *startup sequence*, which is what the pointer's busy state
belongs to. The sequence ends when the program says the launch is over, and winit
says so as it creates the window -- `remove_activation_token`, called from
`platform_impl/linux/x11/window.rs`. A launch that hands its files to a running
editor creates no window, so nothing ever said so, and the shell kept the pointer
turning until its own timeout: seconds of spinner over a file that had been open
for twenty milliseconds.

`crates/nn-app/src/startup.rs` sends what the window would have sent: `remove:
ID="<token>"` as client messages of type `_NET_STARTUP_INFO_BEGIN` and then
`_NET_STARTUP_INFO`, twenty bytes of text each, addressed to the root window with
the property mask, over a throwaway 1x1 carrier window the way winit does it. The
connection is `x11rb`, already in the graph as a winit dependency, opened for the
one message. Nothing is sent without a token or without `DISPLAY`, and both calls
are `.check()`ed, because a run that reports success without sending anything is
exactly how the first attempt at this looked.

The message is verifiable where the pointer is not: `xev -root -event property`
sees the pair from a cold launch and saw none at all from a hand-off before the
change; afterwards it sees three pieces carrying the token gio hands over. The
pointer itself is drawn by the shell, so whether it stops is the user's report, and
it stops.

The window class is pinned to `npp` in the same spirit. Left to the toolkit it is
the name the binary was invoked by, so the launcher symlink `nn` would have been a
second, icon-less application with its own taskbar group; the desktop entry's
`StartupWMClass` matches on the pinned name.

### The fonts are scanned beside the graphics device

`State::new` did its slow work in a line: 72 ms building the `Renderer`, which scans
the installed fonts, then 28 ms measuring the column width, which is the first text
shaping a process does and therefore the expensive one. Both are pure CPU, neither
needs the device, and the device is opened just before and takes about as long.

The scan now runs on a worker thread started before the event loop, at the point the
process has decided it is going to open a window, and the same thread measures the
column, so `EditorView::with_char_width` takes the answer instead of computing it.
`State::new` joins the thread and falls back to doing the work inline if it
panicked: a thread that fails should cost a slow start, not a window.

Measured through the desktop entry, the way a right click does it: 282 ms from
launch to a focused window before, 235 ms after. About 160 ms of the remainder is
the Vulkan instance and the device, which is the driver's own cost on this machine.

Putting the `wgpu` instance on a thread of its own was tried and reverted. Over six
interleaved runs against a plain build, window creation went from 18 ms to 65 ms
because two driver initialisations in one process contend for the same locks, and
the totals came out no better and much noisier. A resident process would make every
open cost what the hand-off already costs, 29 ms, but that is a different program
and it is not in this pass.

### The version

The manifest said `0.1.0` while the program had grown well past it. The number is
declared once, in `[workspace.package]`, so `--version`, the About window and the
MCP server's `serverInfo` all moved by changing that line and rebuilding. Two
places could not follow it: the debian changelog is a history and needed a new
entry above the old one, and the man page's header names the version itself. The
PDF's title line used to repeat the number too, and now reads the manifest, so a
document cannot claim a version the program does not.

The plugin workspace keeps its own `0.1.0`. The plugins build for a different
target and are versioned on their own; a bump here would say their ABI changed,
and it did not.

### Verification

`cargo test --offline --all`: 1917 passed, 0 failed -- 969 in `nn-app`, 878 in
`nn-core`, 70 in `nn-mcp`. `cargo clippy --offline --all-targets`: 0 warnings.

`./target/release/npp --version`: `Neko++ 0.2.0`. `npp-mcp` prints `npp-mcp 0.2.0:
104 sections, 73 shortcuts, 1 of 5 plugins enabled` and answers `initialize` with
`"version":"0.2.0"`; the README's examples were a section behind and now carry
that line.

`python3 docs/build-pdf.py --check`: up to date, 274,947 bytes. The PDF is 66 pages
A4 with 106 bookmarks, from 65 pages and 106 bookmarks: the three new paragraphs
needed the extra page, and no heading was added or removed, so `nn-mcp` still
reports 104 sections, 27 chapters and 77 subsections. The title line accounts for
the three bytes the rebuild lost.

`cargo fmt` was not run.

## Sixty-eighth pass

### The paste the find bar swallowed

`Ctrl+F` opened the bar and `Ctrl+V` then pasted into the document behind it. The
bar's key handler takes every chord it knows and returns false for the rest, so
the paste fell through to the keymap and the `paste` action edited the text
underneath -- the one thing a user pasting a pattern into a search field is not
asking for.

`Ctrl+V` is now taken by the bar, by the same route the file browser's filter and
the path prompt have had for a while. The text goes into whichever field holds the
caret, query or replacement, so the bar did not need a second kind of paste for
the second field.

Control characters are dropped on the way in. A pattern copied from a file arrives
with the newline that ended its line; the field is one line with no way to render
one, and a pattern carrying a newline would match nothing. Flattening is the
reading that keeps the paste useful, and it is what the browser's filter already
does with a pasted path.

The key is consumed even when the clipboard is empty or holds only white space.
Returning false there would put the paste back on the path that started this pass.

### Verification

`cargo test --offline --all`: 1922 passed, 0 failed -- 974 in `nn-app`, 878 in
`nn-core`, 70 in `nn-mcp`. Five of the new tests are the bar's, on the field rather
than on the window: the text landing at the caret, the control characters being
dropped, the replacement field taking the paste when it has the caret, and an empty
clipboard reporting nothing inserted.

No synthetic input is available on this machine -- `xdotool`, `xclip` and `xsel` are
all absent -- so the chord itself was not pressed here. The arm is four lines beside
the browser's, which was pressed and works.

`cargo clippy --offline --all-targets`: 0 warnings. `python3 docs/build-pdf.py
--check`: up to date, 275,256 bytes, 66 pages A4, 106 bookmarks. `npp-mcp` still
reports 104 sections and 73 shortcuts: the find bar gained a paragraph, not a
heading.

`cargo fmt` was not run.

## Sixty-ninth pass

### Selecting the pattern

`Ctrl+A` with the find bar open selected the document behind it, the same way the
paste did in the pass before. The bar's handler now takes the chord too.

Taking it exposed the real gap: the bar's field had nowhere to put a selection. It
was a string with a caret -- no range, no highlight, nothing for the next
keystroke to delete. So the field grew one, along with the rules every
single-line field obeys: select-all selects everything and leaves the caret at the
end, typing or pasting replaces the selection, `Backspace` and `Delete` remove it,
and any caret move drops it. Switching fields drops both, since a selection left
armed in the field the user is not looking at would let the next keystroke wipe
text they cannot see.

The selection is one range rather than a cursor plus an anchor. This field has no
mouse selection and no Shift+arrow, so there is no second end to drag, and
`Option<Range<usize>>` is the whole story. Should Shift+arrow ever arrive, the
anchor belongs here with it.

It is drawn as a bar of `theme.selection` behind the text, under the caret, because
a selection the user cannot see is a field that looks broken the moment they type
over it.

### Verification

`cargo test --offline --all`: 1931 passed, 0 failed -- 983 in `nn-app`, 878 in
`nn-core`, 70 in `nn-mcp`. Nine of the new ones are the bar's: typing over the
selection, pasting over it, `Backspace` and `Delete` clearing it, the replacement
field taking it when it has the caret, an arrow key and a field switch dropping it,
an empty field selecting nothing, and the search following the new pattern.

As in the pass before, no synthetic input on this machine -- `xdotool`, `xclip` and
`xsel` are absent -- so neither the chord nor the drawn highlight was seen here.
The highlight is a quad in the same list the bar's caret already uses, drawn before
the glyphs, which is what makes it sit behind the text.

`cargo clippy --offline --all-targets`: 0 warnings. `python3 docs/build-pdf.py
--check`: up to date, 275,611 bytes, 66 pages A4, 106 bookmarks. `npp-mcp` still
reports 104 sections and 73 shortcuts.

`cargo fmt` was not run.

## Seventieth pass

### The chord that typed itself

`Ctrl+H` did nothing, or worse: with the find bar open it typed an `h` into the
pattern. The bar's handler trusted `event.text`, and on this platform a control
chord arrives carrying its letter, so every unhandled `Ctrl+<letter>` was typed
rather than passed on. `Ctrl+H` never reached the keymap, so `replace` never ran.
The same fault typed an `a` where `Ctrl+A` was meant, which is what the pass
before tried to fix one chord at a time.

The handler now asks `findbar::types_text(ctrl, alt)` before inserting anything, and
decides on the modifiers rather than on the text: neither held is a letter, both
held is AltGr, and exactly one held is a command for the keymap. `Ctrl+H` opens the
replace row, `Ctrl+F` closes it again without losing the pattern, and `Ctrl+Z` no
longer types a `z`.

### The label under the field

The field started at a fixed sixty-two pixels from the padding, which was enough
for "Find:" and not for "Replace:" in the proportional UI face: the label ran under
the text. The column is now measured from the shaped labels, so it fits the widest
one in whatever face and language is in use, with the old sixty-two kept only until
the first frame shapes them.

### Verification, by pressing the keys

Earlier passes recorded that no synthetic input existed here. It does: `libXtst` is
installed, and sixty lines of `ctypes` (`/tmp/xinject.py`) drive `XTestFakeKeyEvent`
and `XSetInputFocus`. Every claim below was made against a running `npp` and read
back from screenshots.

`Ctrl+H` with the bar open opened the replace row. Typing `alpha` and `beta` into the
two rows showed the two fields clear of their labels. `Ctrl+R` replaced the current
match in the document and moved to the next. `Ctrl+A` selected the replacement field,
drawn as a bar behind the text. `Ctrl+F` returned the bar to one row with the pattern
intact and no stray `f` in it.

`cargo test --offline --all`: 1933 passed, 0 failed -- 985 in `nn-app`, 878 in
`nn-core`, 70 in `nn-mcp`. Two are new: the modifier table, and the field starting
after the widest label.

`cargo clippy --offline --all-targets`: 0 warnings. `python3 docs/build-pdf.py
--check`: up to date, 276,090 bytes, 66 pages A4, 106 bookmarks.

`cargo fmt` was not run.

## Seventy-first pass

### The bar becomes the dialog

Notepad++ keeps its search options in a dialog: a row of checkboxes, a direction
pair, and four buttons. The bar here had a pattern field, three toggles and two
buttons, so `Ctrl+H` could replace one match at a time and nothing more. The user
asked for the feature list rather than for the shape, so the shapes stayed: three
rows at most -- pattern, replacement, settings -- and the settings row is nine
chips and four buttons.

The chips are `Aa` case, `W` whole word, `N`, `E`, `.*` the three search modes,
`∞` wrap, `▭` in selection, `↑` and `↓` the direction. Exactly one mode is lit and
exactly one direction is lit, so the row reads as two radio groups with the rest
as switches. A chip is a rectangle with shaped text, laid out and hit-tested by
the same `layout()`, which is what stops a control from being clickable where it
is not drawn.

The buttons are right-aligned and placed from the right, `Close` first, so the one
button that is always in the same place stays in the same place when a translation
is longer. Twenty-three catalogs gained the "Replace All" label in this pass.

### The chip that changes its name

`W` turns into `\n` when the mode is `.*`, and then it means ". matches newline".
That is Notepad++'s compromise and it is the right one: a whole word is not a
property a regular expression has, the two settings are never both meaningful, and
a tenth chip would make the row a dialog again.

Two marks are not the obvious glyphs. A circular arrow for wrap and a
carriage-return arrow for the newline escape both came out as tofu boxes -- the
interface font on this machine has neither -- so the chips read `∞` and `\n`. The
first says the search goes round; the second is the escape the pattern has to
carry anyway.

### Three modes in the core

`SearchMode` is `Normal`, `Extended`, `Regex`, and it sits next to the
`SearchOptions` it steers in `nn-core`, because the find bar is not the only
caller: find-in-files compiles the same options over a whole tree.

Extended is a small unescaper -- `\n`, `\r`, `\t`, `\0`, `\\`, `\xHH`, `\uHHHH` --
and an escape it does not recognise stays the two characters it is, so a Windows
path in the pattern is not silently mangled. It runs before the matcher is built,
which is what keeps whole word and case folding working on the unescaped text.

### Enter does what the row says

`Enter` replaces when the caret is in the replacement row, finds the next match
otherwise, and takes Replace All with `Ctrl` held; `Shift+Enter` walks the other
way. `Enter` always meaning "next match" is why the replacement field read as a
form: you type a replacement, press `Enter`, and the text does not change.

Wrap decides what happens at the edge, and it is a switch rather than a policy:
with `∞` on a search that runs off the end continues at the first match, with it
off the search stops and leaves the caret where it was.

### Verification, with the mouse

Dragging was the one thing the keyboard could not fake here: `shift+up` through
XTest moves the caret without extending a selection on this platform, so the
selection for the in-selection chip came from twelve synthetic motion events with
the button held down.

That gave the evidence directly. With `▭` off the counter read `1/4` and four
matches were boxed in the document; with `▭` on over a selection holding one of
them, `1/1` and one box. Wrap next: `∞` off, `Ctrl+End`, `Enter`, `Enter`, and the
counter stayed at `4/4` with the caret where it was; `∞` back on and one `Enter`
put the counter at `1/4` with the caret on the first match.

A stale window from the previous pass's multi-file test -- a full-screen debug
window that still held the input focus -- swallowed the first round of synthetic
clicks. It is gone; the clicks land on the bar now.

`cargo test --offline --all`: 1957 passed, 0 failed -- 1002 in `nn-app`, 885 in
`nn-core`, 70 in `nn-mcp`. Twenty-four are new: seven for the escapes and the
modes in `search.rs`, seventeen for the chips, the buttons, wrap, the selection
restriction and the two-row and three-row heights in `findbar.rs`.

`cargo clippy --offline --all-targets`: 0 warnings. `python3 docs/build-pdf.py
--check`: up to date, 280,400 bytes, 66 pages A4, 107 bookmarks.

`cargo fmt` was not run.

## Seventy-second pass: every checkbox in the dialog, and four things the mouse found

The previous pass gave the bar the dialog's checkboxes and its three search
modes. This one finished the list. Notepad++ 8.9's search window is a sheet of
five tabs -- Find, Replace, Mark, Find in Files, Find in Projects -- and the last
two already existed here as the Search menu's own prompts, so what was missing
was the rest of the first three: three buttons out of ten, two chips out of
eleven, the history of both combo boxes, and the swap pull-down.

### Ten buttons, and rows that wrap

`BUTTONS` is now the dialog's whole set in the dialog's order: Find Next, Count,
Find All in Current Document, Find All in All Opened Documents, Replace, Replace
All, Replace All in All Opened Documents, Mark All, Clear Marks, Close. The three
replacement rows are `replace_only()` and are skipped while the bar is in find
mode, which is the bar's way of greying out what the dialog disables.

Ten buttons do not fit on one row at 1100 pixels, and will not fit at any width
once a translation lengthens four of the labels. `button_rows()` fills each row
greedily from the widest label as drawn and starts another when the next button
would pass the margin -- and then, if the last row has one button under a row
packed to the edge, moves a button down, which is the difference between a bar
that looks laid out and one that looks wrapped. `height()` grew a width
parameter so it can answer "how tall am I at this width", and the two callers
that reserve the bar's band -- `find_bar_top()` and `pane_area()` -- pass the
window width.

### Two chips from the Mark tab, and one of them was a no-op

The Mark tab's search has a bookmark checkbox and "Purge for each search".
`Chip::Bookmark` and `Chip::Purge` are those two, on the end of the settings row
rather than on a tab, because a bar has no tabs.

Purge is the interesting one. The first cut cleared the bar's own style slot
before laying down the new marks, and that is a no-op by construction:
`set_marks(0, ...)` replaces slot 0 whether or not it was cleared first. What
the option means is that the marks the *previous* searches left go away,
whichever style they were made in, so the branch now calls
`clear_all_styles()`.

### History in both fields

Each field keeps the twenty patterns last searched for, eight visible at a time,
opened by the arrow at the right end of the field or by `Alt+Down` when that
field has the caret, the way the dialog's combo boxes drop theirs. Once the list
is open it is a mode: the arrows walk it, `Enter` takes the entry, `Escape`
closes the list and not the bar, and a click anywhere else leaves it alone. The
list grows upwards over the document, so it lies outside the bar's rectangle and
presses inside it are routed to the bar anyway.

The history lives in `find_history.txt` beside the settings rather than in the
session, because the session is about which files are open: a search history
that vanished when the last tab closed would be a history of the wrong thing.

### Swap, as three buttons

The dialog has one button and a pull-down to choose what it does. A bar docked
along the bottom of the window has room for three small buttons and no room for
a menu, and three buttons are one click each where a pull-down is two.

### Verification, with the mouse

Same X11 session as the last pass, this time with the bar's layout printed on
every press so a click can be traced from the pixel to the control that claimed
it. That was worth doing: an apparent forty-pixel offset between where a chip was
drawn and where it was clickable turned out to be a misread screenshot, not a
bug -- sampling the PNG rows put the chips at 660, the first button row at 680
and the second at 700 for a bar top of 610, and the logged rectangles are those
rows. Every control was clicked and answered: Count read "4 matches", both
"Find All" buttons filled the results panel, Replace moved `let foo` to
`let BAR` with the counter going 1/4 to 1/3 and the next match selected, Replace
All finished the rest, `B` bookmarked every marked line, `P` toggled, the three
swap buttons swapped and the history list opened, took an entry and filled the
field.

Four defects came out of that session and none of them had failed a test:

- A press anywhere below the bar was claimed by the bar. The press handler
  tested `y >= top` with no bottom edge, so the results panel, which sits between
  the bar and the status bar, could not be clicked at all.
- The bar was measured before it was prepared. `find.prepare()` ran after
  `find.height()` in the renderer, so the first frame was laid out with every
  button width still at zero: all ten fitted on one row for the measurement
  while the same frame drew two, and the second row hung over the status bar.
  Preparing the bar before measuring it fixes both the frame and the label
  column the fields are placed against.
- The all-documents header counted hits as files. One document with four matches
  read "4 of 4 files"; the file count now counts documents.
- Purge cleared one style slot, as above.

The purge fix has a pixel test rather than a status-message test: a style-2 mark
was laid down with Search, Style All Occurrences of Token, and the region
sampled at 1125 pixels of the style-2 colour. After Mark All with `P` on and a
pattern that matches nothing, the same 1125 pixels are the smart-highlight blue
and the style-2 green is gone from the frame, which is the option doing exactly
what its name says.

The layout print was temporary and is out of the source.

`cargo test --offline --all`: 1971 passed, 0 failed -- 1015 in `nn-app`, 886 in
`nn-core`, 70 in `nn-mcp`. Fourteen are new: the hit tests for the two new chips,
the ten buttons, the three swap buttons and the history list, `set_marks` and
`clear_all_styles` in the core, and the named results header.

`cargo clippy --offline --all-targets`: 0 warnings. `python3 docs/build-pdf.py
--check`: up to date, 284,602 bytes, 68 pages A4, 106 bookmarks.

`cargo fmt` was not run.

## Seventy-third pass: the search as a floating window, as a preference

The bar had reached parity with Notepad++'s search window the hard way, one
control at a time, in a band along the bottom of the window. The question that
opened this pass was whether the *window* -- floated over the document, dragged
by its title, with the same buttons in the same order -- was possible at all.
It was, but the answer the user wanted was the one after it: keep both, and put
the choice in Preferences under "Floating Search", off by default, so the bar
that had just been finished is not taken away from anyone who likes it.

### A view, not a second search

`FindDialog` owns no pattern, no options, no matches, no history. It reads the
`FindBar` it is handed: `is_open` is `find.open && find.floating()`, every
control is a `findbar::Hit` -- the same enum the bar's own press handler
answers -- and the matches it counts are the bar's matches. That is the whole
reason toggling the preference mid-search keeps the search, and the reason the
dialog needed no new search code: every control it draws except the tab strip,
the title bar and the slider was already wired.

Only three hits are the dialog's own. `DialogHit::Tab` switches tabs,
`DialogHit::Title` starts a drag, and `DialogHit::Transparency(f32)` sets the
panel's alpha from the fraction along the track. Everything else is wrapped:
`DialogHit::Find(findbar::Hit)` and then the existing `apply_find_hit`, which
knows every one of them.

### The band that is not there

The docked bar reserves a strip of the window: `FindBar::height()` is asked for
it by `find_bar_top()` and again by `pane_area()`, and `hit_at`, `push_quads`
and `text_areas` are asked what lives in the strip. So `floating` is answered in
one place -- `height()` returns zero and the three others return early -- and
every caller that reserves the band follows without being told, because zero is
a width they already handle. The alternative, threading a flag through the four
callers, is four places to forget.

The dialog is not modal. A press inside it belongs to it, a press anywhere else
falls through to the document, and a press on the toolbar is now tested *after*
the dialog's, because the dialog is drawn above the toolbar and the last thing
drawn should be the first thing hit. `Escape` closes both presentations through
the same `close_find`, which also ends a drag in case the release lands outside
the window.

### Three defects the bench dumps found, none of them a test failure

The dialog was written, its twenty-three unit tests passed, and the first
rendered frame still came out wrong. `bench` grew `floating_tab=find|replace|mark`
and `floating_transparency=`, and the dumps said:

- Every button was missing its word, and the frame had been panicking on the
  `expect("every drawn label is in the map")` that guards the shaped map --
  except nobody had reached that path, because `new()` shaped the captions and
  the checkboxes and forgot `BUTTONS`. Fourteen words, shaped nowhere, drawn
  anyway.
- "Find what:" and "Replace with:" were not on screen. `sheet()` lays the sheet
  out from its own origin, so the caption position is `PAD`; `layout()` moves
  every *rectangle* into window coordinates and left the one position behind.
  The captions were drawn at x=12 against a frame at x=312 and clipped by the
  frame's text bounds.
- Close hung off the right edge of the Find tab. Five buttons do not fit 476
  pixels in English and will not fit at any width once a translation lengthens
  them; `button_rows()` now wraps greedily and both `rows()` and `sheet()` read
  the same wrapping, and the frame is measured over all three tabs rather than
  the current one, so no tab's buttons hang below it and the frame does not
  change height under the pointer when a tab is switched.

Three tests were added, one per defect: the buttons' rows fit the frame's inner
width, the caption sits between the frame's left edge and the field, and every
label the sheet draws is in the map -- the last one walks all three tabs through
`prepare` and `text_areas`, which is where the missing fourteen would have
panicked.

### Verification, without a mouse

Input injection is not available on this display: mutter holds a grab that
swallows every XTest device event, the app ignores synthetic `XSendEvent` because
it takes them from XI2, and an Xvfb fallback dies on lavapipe ("wgpu error: Out
of Memory" in `Surface::configure`, no DRI3). So the drawing was verified by
rendering: `bench "" 1100 750 6 out.ppm floating_tab=find` and the other two,
cropped and read, which is how the three defects above were found; the click
mapping is covered by the layout tests, which press at the centre of every
rectangle `push_quads` draws and require the control it belongs to -- tabs,
checkboxes, radios, modes, the four buttons per tab, both fields, their arrows,
the three swaps and the slider.

`cargo test --offline --all`: 1997 passed, 0 failed -- 1041 in `nn-app`, 886 in
`nn-core`, 70 in `nn-mcp`. The three `nn-mcp` failures this pass started with
were the reference's own guard doing its job: `docs/nekopp-features.md` grew a
subsection and a preferences row, so 104 sections became 105 and the counts in
`doc::tests` and `server::tests` were updated with it.

`cargo clippy --offline --all-targets`: 0 warnings. `python3
docs/build-pdf.py --check`: up to date, 287,307 bytes, 68 pages A4, 107
bookmarks.

`cargo fmt` was not run.

## Seventy-fourth pass: the editor's own commands, with and without a window

The user's sentence had two halves and the first one was the important one: "I
need both options, by default when used via a harness use the headless mode -- the
more I think about it I also want to have the full api of neko++ made accessible
as well." The MCP server until now served the documentation and a plugin's
commands, and neither of those is a way to *do* anything to a file: a harness
could read what Neko++ does and could run a plugin, but sorting a list or trimming
a file's trailing spaces meant writing the code itself. That is the gap.

### One table, not a second implementation

The obvious design is a `commands` table inside the MCP crate: a name, a closure
over a `String`. It is also wrong, because there is already a table with the same
names in the menu, and a second one drifts. `nn_core::ops` is the table: thirty-
seven names, each calling the `nn_core::lines` function the menu row already
calls. `sort_asc`, `trim_trailing`, `tabs_to_spaces` -- the action names the
keymap uses, so a caller reading the shortcut list and a caller reading the API
see one vocabulary.

`nn-app` then delegates to it. The giant `run` match lost thirty-six arms and
gained one that hands the action to `ops::run` with the editor's tab width, sort
options and a seed taken from the clock, so a menu click, a keypress, an API call
and an MCP call all arrive at the same function. A test in `menu.rs` walks the
menu's text commands and the ops table in both directions and fails if either has
a name the other does not, modulo two documented rows: `sort_lines` and
`split_lines` are `api_only` because their menu row is a submenu of fourteen sorts
and a delimiter prompt, and a caller has no pointer to pick with.

What is deliberately *not* there is the commands whose effect depends on the
caret: `duplicate_line`, `move_line_up`, `delete_line`. A table of "text in, text
out" cannot express them, and inventing a caret parameter for the sake of a
uniform list would be a worse interface than an honest boundary. They are
reachable in a window, through `command.run`, and `get_command` says which is
which.

### The headless half

`run_command` takes `text` or `path`, never both, plus the nine options the
commands read (`sort_key`, `sort_order`, `case_sensitive`, `width`, `delimiter`,
`prefix`, `block_open`, `block_close`, `seed`). `path` without `write` reads and
changes nothing, which is what a caller checking a result wants; `write` puts it
back. `list_commands` prints the table with each command's options, and the
fourteen tests in `run.rs` cover the arg reading -- including the eight spellings
of a sort key and the width clamp at 64 -- rather than the sorts themselves, which
`nn-core` already tests.

Nothing in the headless path touches a display, a session bus or `nn-app`: it is
`nn_core` plus `serde_json`, so it runs in a container and over ssh.

### The other half, and the two round trips it took

`npp --call '<json>'` sends one request to the window that is already running and
prints its answer. `--call` is handled *before* the hand-off in `main()`, because
`--call` is the one switch that must not become an `Open` on the running editor.

The interface method cannot return the answer directly. zbus delivers messages on
a task of its own, and a method that blocked until the window's event loop had run
the command would stall the loop that has to deliver the reply: a deadlock, not a
slow call. So there are two: `Call(json) -> u64` queues the request and returns an
id at once, and `Answer(id) -> String` returns the answer or an empty string while
it is still pending. `about_to_wait` drains the queue, runs each call between
frames, stores the answer, and the client asks again every 5 ms until the answer
is non-empty or five seconds have passed.

The window's side is `State::call` over `nn_app::api::Call`: `app.status`,
`document.list`, `document.get` (with an optional `range`), `document.set`,
`document.save`, `command.run`, `help`. A refusal is an answer too -- `{"error":
{"code": "unknown_document", "message": ...}}` with exit 0 -- and only a call that
finds no editor exits 1.

### The defect the live run found

`command.run` was written to mean what the menu means, and the first end-to-end
test returned the text unchanged. It was not broken: `sort_asc` with no selection
sorts the line the caret is on, which is exactly what a person pressing the menu
row gets, and a caller pressing nothing gets one line sorted and no clue why. The
fix is not to make commands select everything -- that would make `delete` and
`paste` wipe a document on a typo -- but to let the caller name the block:
`whole_document: true`, or `range: [start, end]` in characters. Both at once is
refused rather than guessed at. Verified against the live window:

```
--call '{"method":"command.run","command":"sort_asc"}'                          -> unchanged
--call '{"method":"command.run","command":"sort_asc","whole_document":true}'    -> sorted
--call '{"method":"command.run","command":"upper_case","range":[0,6]}'         -> first word up
--call '{"method":"command.run","command":"sort_asc","whole_document":true,"range":[0,4]}'
                                        -> bad_request, "send `whole_document` or `range`"
```

### `live_call`, and why it shells out

The MCP server's live tool sends a call to the window and returns the answer, and
it does that by running `npp --call`. The alternative -- a D-Bus client inside
`nn-mcp` -- would put `zbus`, `tokio` and a bus connection into a crate that
otherwise needs `serde_json` and `nn_core`, for one tool whose protocol is
already written and already tested. `live.rs` resolves the editor in the order a
person would look: `NPP_BIN`, then a sibling of `npp-mcp`, then PATH. Six tests
run a shell stand-in for the editor and check that the call arrives word for
word after `--call`, that a refusal is reported with the editor's own words, and
that a binary that cannot be run is named.

The two tools are chosen by what the work touches. `run_command` for text that
only needs transforming; `live_call` for work that has to happen in the window
someone is looking at -- the caret, the selection, the open tabs, a save.

### One defect in the usage text

`npp --help` printed "already running andprints": the string had been wrapped in
the source with `\` continuations and the newlines before the wraps were missing.
Running the binary is what found it; a test compares the switch list, not the
prose.

### Verification

`cargo test --offline --all`: 2062 passed, 0 failed -- 1058 in `nn-app`, 906 in
`nn-core`, 98 in `nn-mcp`. That includes 20 in `ops`, 13 in `api`, 14 in `run`,
6 in `live`, 3 in `cli` for `--call` and 7 the MCP server grew -- 98 against 90 in
`nn-mcp` before this pass.

`cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not run.

Live, on `DISPLAY=:1` with a profile of its own: `app.status`, `document.list`,
`document.get` (whole, by path, and with `range`), `document.set` (whole and
inside a range), `document.save` (the file on disk changed), and the four
`command.run` lines above. Then through the server itself, four JSON-RPC lines
into `target/debug/npp-mcp` with `NPP_BIN` set: `document.list`, `command.run
trim_trailing whole_document`, and a bad path, which came back as a failed tool
with the editor's message.

`python3 docs/build-pdf.py --check`: up to date, 301,096 bytes, 70 pages A4, 111
bookmarks.

`docs/nekopp-features.md` grew a chapter, "Commands over the API", with three
subsections -- one table, without a window, in the window, over MCP -- and its
command-line block gained `-C, --call`; `crates/nn-mcp/README.md` went from seven
tools to ten. The chapter is four sections, so the three count assertions in
`doc.rs` and `server.rs` moved from 105/106 to 109/110 and from 27 chapters and 78
subsections to 28 and 81 -- the counts are the document's own shape, which is why
they are asserted rather than computed.

## Seventy-fifth pass: one lookup, two vocabularies

`get_command {"name": "sort_asc"}` answered "No command is called sort_asc" in
the same session that `run_command {"command": "sort_asc"}` sorted a list. Two
tables had grown names and only one of them was reachable through the lookup:
`Inventory` joins the keymap table with the menu tree, and `nn_core::ops` is the
text commands, and `sort_asc` lives in the second with no keymap row to join.

The join is also why `sort_lines` was wrong in the other direction. `Inventory::lookup`
falls back to "a label containing the query" when it finds nothing exact, so
`sort_lines` matched the menu row "Sort Lines..." -- a row whose own action is
none, because the menu reaches it through a submenu of fourteen sorts. A caller
asking about a command got a menu label with no action and no answer to "can I run
this".

The order is now: a name in the table gets the table's answer, with the options it
reads and no menu path, unless the keymap binds the same action -- then the
keymap's answer wins, because it carries a shortcut, a menu path and a resource.
`upper_case` is in both and comes back with `Ctrl+Shift+U`; `sort_asc` is only in
the table and comes back with `options: none`; `duplicate_line` is only in the
keymap and still says `runs: in a window`.

Four tests: the table's names answer without a shortcut, the options are listed
for the one command that reads three, the keymap wins when both know the name, and
a sweep asserts that every name `run_command` takes is one `get_command` answers
without `isError`. The unknown-name error now names both lists instead of only
`list_shortcuts`.

### A test that failed four runs in fifteen

The full-suite run this pass failed once in `live::tests`, then passed, then failed
again. The message was `could not run /tmp/nn-mcp-live-<pid>-0/npp: Text file busy
(os error 26)`: the tests wrote a stand-in editor into a temporary directory and
ran it, and on Linux `execve` refuses a file that any process still holds open for
writing. Two test threads overlap -- one writing its script, one forking to run
another -- and the fork inherits the writing descriptor. Moving from a path per
thread to a path per call removed most of it and not all: a `static` counter is
per process, but the window is the fork, not the name.

The fix is to stop writing executables in tests. `/bin/echo` prints its arguments,
so it proves the call arrives after `--call` word for word; `/bin/ls` refuses an
argument it does not know, and the test compares against what the same `ls` prints
when run from the test, so it is about the plumbing rather than about a French
translation of `ls`; `/bin/false` is a refusal with nothing to say. Twelve
consecutive runs of the suite: no failures.

The same pass found a second, quieter hazard. The `live_call` handler test set
`NPP_BIN` in the process environment to point at a stand-in editor, and
`std::env::set_var` is not thread-safe while other tests spawn processes. That test
is gone; the handler's answer is now a pure function, `live_answer(&str) -> Value`,
with four assertions -- the editor's JSON, its error object as a failed tool, and
text that is not JSON passed through as words.

Verification: 100 tests in `nn-mcp` (98 before), and the live sweep of all 37
names returned 0 errors -- 5 resolved through the keymap, 32 through the table.
`cargo test --offline --all`: 2064 passed, 0 failed. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run. PDF: up to date, 301,578
bytes, 70 pages A4, 111 bookmarks.

## Seventy-sixth pass: a room with a kitten in it

Playground mode is the first part of this program that Notepad++ does not have at
all. `Ctrl+Alt+K` -- K for kitten, the chord free and next to the other view
switches -- hides the editor and puts a room on the screen: code on the walls, a
floor of punctuation, a table, a plant, and a kitten who walks about, follows the
pointer, bats loose tokens around and falls asleep. `Escape` gives the window
back. The document is never read and never written, so a screenshot of the mode
cannot show anyone's work and five minutes in the room cost no undo step.

The split is the one the rest of the project uses: the simulation is `nn-core`
with no GPU and no clock, and the drawing is `nn-app`. `nn-core::playground`
generates rooms, drops tokens, runs the kitten's moods and projects the camera;
`nn-app::playground` owns the atlas, the three passes and the idle clock that
decides whether the mode may open by itself. The simulation being pure is what
made the mode cheap to get right: `cargo test` drives minutes of cat behaviour in
milliseconds and asserts on moods and positions rather than on pixels, and the
`bench` binary can draw a frame five minutes in without waiting five minutes.

One unit is one line of editor text. A glyph card is one unit tall, the room is
sixteen across, the kitten is two and a quarter -- which is what makes the mode
the same at any font size on any display, and what makes the depth buffer's near
and far planes a constant rather than a guess per window.

### The room

Four cards, one per wall, facing inwards, each papered with lines of code from the
corpus. The corpus is compiled into `nn-core` in eight flavours -- Rust, C, Java,
Python, JSON, XML, UN/EDIFACT and CSV -- and the six that have a lexer here are
coloured by that lexer, so the walls are papered in the same highlighting the
editor uses. EDI and CSV have no lexer and are split on their delimiters instead.
`Style` travels with every character as one byte, which is why colouring eight
languages costs one code path.

The open document is never the material. A room built from the user's file would
look like a view of that file rather than a place beside it, would put their work
where they cannot edit it, and would make every screenshot of the mode a leak.

The room is rebuilt every five minutes from the next seed, over a two and a half
second blend in which the old room fades and its furniture lifts away. Seeds are
passed in, never read from the clock inside the simulation: the window makes one
from the clock, a constant and a visit counter, and `bench` can be handed one.

### The kitten

A body, a chest, a rump, a head carrying two ears, a muzzle, a nose, two eyes,
four legs and two paws -- all boxes, all placed from poses -- and a tail of six
segments that alternate colour so the tip reads as a tip. Moods are `walk`, `sit`,
`crouch`, `pounce`, `bat`, `sleep`, each with a pose the boxes are placed from, so
a mood is a shape and not a sprite. It turns towards the pointer when the pointer
moves, ignores it when it does not, and pounces on a loose token when one is close
enough; three minutes with no input and it curls up where it stands, which the
corner line reports.

### What the frames showed

Five bugs came out of looking at rendered frames rather than at code, and each was
invisible in the tests.

The glyph atlas was clipping every single glyph. The rasteriser set the pen at the
centre of the cell, and measured the ink box *after* clipping to the cell, so the
box always fitted the cell and the fit-to-cell loop never fired: letters were
drawn from their centres and cut on the right and the bottom. The rewrite places
the pen at the cell's left edge and the baseline at its bottom, measures each
glyph's ink from the placement rectangle before any clipping, returns a stamp
with its own size and offset, and copies that stamp at the offset. Rendered proofs
of all 95 characters of the atlas came back upright and complete.

The side walls were mirrored: the cards were built facing out of the room, so
their text read backwards. The yaw is now `-sign * FRAC_PI_2`, and both walls
read.

The sleeping kitten was a slab: the body did not lower, the chest stayed filled
with a bar across the middle, and the ears floated above a head that had shrunk
under them. The asleep body is now lower and longer, the chest is skipped, the
head tucks down and forward, and the ears ride the top of whichever head they
belong to.

The awake head sat a quarter unit too far forward, which left the neck open at the
front -- the cat looked beheaded from a three-quarter view. `HEAD` moved from
`z = 1.28` to `1.04`.

The back wall's rows were cut by the top edge of the window. Rows now start at
`1.18` and step a full unit, so four rows fit above the horizon at any window
shape the mode accepts.

### Cost

Two and a half to three and a half milliseconds a frame at 1280 by 800, measured
with `bench`, of which the simulation is microseconds and the rest is three draw
passes into one depth buffer: cards, boxes, dust. The mode redraws at sixty a
second only while it is open, and it asks for no frames at all when it is not,
because a screensaver that spins a GPU is not a screensaver.

### The rest of the wiring

`Ctrl+Alt+K` in the keymap, a "Playground" row under "Distraction Free Mode" in
the View menu, two preferences rows -- on or off, and the idle minutes that open
it by itself, zero by default -- three strings across the 23 localisation files,
one accessibility label announcing the flavour, the room number and "Esc leaves",
and three `bench` flags (`playground`, `playground_time=`, `playground_seed=`) so
a frame can be reproduced from a command line. The preferences dialog grew from
14 rows to 16, and the test that anchors a dropdown to the row above it now
derives the anchor from `Row::ALL` instead of a hard-coded pixel offset.

The menu-shape test compares the View menu against Notepad++'s row for row, so it
now filters "Playground" out explicitly, with a comment saying why; a second test
asserts the row sits immediately after Distraction Free Mode, where the deviation
is deliberate rather than incidental.

### The documents

`docs/nekopp-features.md` gained the chapter "Playground" -- a table of what the
room does and when, the eight flavours, the two clocks, the screensaver, and why
the material is a corpus -- plus the View menu row, two preference rows, and the
shortcut table row. The keymap is 74 chords now, so the two counts in the
document changed with it.

`nn-mcp` reads that document at build time, including the shortcut table, which is
why its own counts moved without a line of logic changing: 73 chords to 74, and
109 sections to 110 -- one chapter, 29 in total, 81 subsections. Its two
table-of-contents assertions count lines, so they moved from 110 to 111.

Verification: 2145 tests across the workspace (1087 in `nn-app`, 958 in
`nn-core`, 100 in `nn-mcp`), 0 failed. `cargo clippy --offline --all-targets`: 0
warnings. `cargo fmt` was not run. Frames rendered and inspected at 0, 45 and 400
seconds of simulation time, the last one after a rebuild, asleep, with the room
reported as number 2 in EDIFACT.

`python3 docs/build-pdf.py --check`: up to date, 308,373 bytes, 72 pages A4, 112
bookmarks -- one chapter and two pages more than the last pass.

## Seventy-seventh pass: the page the kitten is printed on

The seventy-sixth pass put the kitten in a room: four walls papered in code, a
floor of punctuation, and one unit per line of text so that a two-and-a-quarter
unit cat was two-and-a-quarter lines tall. The arithmetic was right and the
picture was not. A still frame showed a box of space with a cat standing in it,
the walls shrinking towards a horizon, and the letters on those walls huge because
they were being looked at from across the room. The complaint named both halves of
that: the kitten is huge, and it is not a kitten; and the environment should be a
text document, in two dimensions, not a place a document is glued to.

The misread was the word "room". What was wanted was a page -- the thing this
program draws all day, at the size it draws it -- with a kitten on it. Everything
follows from taking that literally.

### One cell, one cell

The page is drawn with an orthographic camera: no perspective, no vanishing point,
one cell of text mapping to one rectangle of screen whatever row it is on. A glyph
lands on whole pixels; the second line is the same size as the thirtieth; the
gutter is a column of numbers and not a wall going away from the reader. The
window is measured in lines, not in pixels -- `rows = round(height / 20)`,
`columns = floor(width / (20 * 0.55))` -- so a 1280 by 800 window is a page 116
columns wide and 40 rows tall at any font size, and a resized window is a resized
page rather than a differently-proportioned room.

The kitten is the one thing that is not flat. Her boxes go through the same
orthographic camera pre-multiplied by a shear (`xform::oblique`), which gives a
cube a visible side and top while the letters under it stay square. That is the
whole of the third dimension in the mode, and it is enough: a cat drawn from boxes
with a lit face reads as solid, and a letter drawn head-on reads as a letter.

### The bug the first render found

The first page came out half off-screen with the kitten in the corner. `look_at`
already centres the page on the origin and the orthographic window was being built
over `0..width_units`, so the projection shifted everything half a viewport left
and half a viewport down; what survived on screen was the top-right corner of the
page, drawn at a tenth of its intended size. The fix is the window symmetric about
zero -- `ortho(-half_w, half_w, -half_h, half_h, NEAR, FAR)` -- which is the same
rule the perspective camera had been following all along by centring its own
bounds. Worth writing down because it will be made again: two matrices that both
place the origin must agree about where the origin is.

Text is now unlit, and dims through one uniform field (`globals.dim.x`, renamed
from `fog`, which had described the room), so the screensaver dims the page and
the kitten together instead of the walls getting darker with distance.

### The furniture is characters

Furniture is not meshes any more; it is ASCII, printed on the page. A table, a
plant, a lamp, a shelf, a framed picture -- each a small array of lines -- and each
standing in its own *cleared box*: the prop owns the cells it occupies and the
generator wipes the code out of them first, so a plant does not grow through a
line of Java. Two to four props per page, placed in the lower half, never
overlapping each other, checked against an `occupied` grid rather than by
comparing rectangles.

Tokens became whole words. They used to be sliced out of a line by column, which
produced fragments -- `"li`, `urn` -- that read as corruption rather than as
vocabulary; now a line is run-parsed into words and a word is taken entire,
`UNDER_PAW = 8` cells wide at most, carrying the style byte it had on its line.

`LABEL_ROWS = 1` reserves the bottom row of the page for the app's own hint line,
which is why the page lays out `body = rows - 1` rows while `Page.rows` stays at
the grid height: the hint line is the page's last line, not a line drawn over it.

### The kitten, again

She was rebuilt for profile and for scale. The body is short (`[1.02, 0.82,
1.24]`), the head big (`[0.9, 1.0, 1.06]`) and set low and forward, the ears tall
in a row along the body, the eyes on the sides of the head, the nose pink and the
paws white: at two and a quarter lines, kittens are read from their proportions
and not from their detail, and a long body with a small head is a cat. Her walk
bounds are the page's now (`room_to_walk`), `x` in `[3.2, width - 1.6]` and `y` in
`[1.6, rows - 2.4]`, so she walks on the text and not through the gutter or off
the bottom.

### What did not change

The rules. The corpus, the eight flavours, the seeds passed in rather than read
from the clock, the moods, the three minutes to sleep, the four to dim, the five
to rebuild, the two and a half second blend, the screensaver wiring and the
preference rows are the seventy-sixth pass's, and all of them survived the change
of projection untouched. That is the argument for keeping simulations free of
GPUs: the thing that was wrong was a camera and a vocabulary, and the tests that
describe the cat's behaviour never had to be rewritten.

### Verification

`cargo test --offline --all`: 2142 tests (1087 in `nn-app`, 955 in `nn-core`, 100
in `nn-mcp`), 0 failed. `cargo clippy --offline --all-targets`: 0 warnings. `cargo
fmt` was not run.

Frames rendered at 0, 45 and 400 seconds of simulation time and inspected: a full
page of code with a gutter, props in cleared boxes, a rug of punctuation
across the bottom five rows, a clear bottom row for the hint, whole words loose on
the page, and a 49 by 46 pixel kitten -- two ears, one eye, a pink nose, white
paws, a tail -- walking in profile at two and a quarter lines.

`docs/nekopp-features.md` had its Playground chapter rewritten for the page rather
than the room, with the table of what the page does and when, the furniture in
cleared boxes, the whole-word tokens, and the two clocks; the View menu paragraph
and both preference rows now say "page" where they said "room".

`python3 docs/build-pdf.py` then `--check`: up to date, 309,573 bytes, 72 pages
A4, 112 bookmarks -- the same page count, 1,200 bytes heavier than the room's.

## Seventy-eighth pass: no gravity, and a face that is a face

Two complaints in one line: "no more gravity and as I said before, the kitten does
not look like a cute kitten at all". Both were right, and both were the
seventy-seventh pass stopping halfway. It had moved the cat onto the page and left
the cat itself alone: a profile animal in a world that pulled words downhill.

### The page is flat, so nothing falls

`GRAVITY` and `RESTITUTION` are gone from `scene.rs`. A token's `vel` is two floats
rather than three, `step_tokens` integrates `x` and `y` only, multiplies both by
`1 - DRAG * dt` with `DRAG = 1.6`, and sets a token to rest once it is under 0.06
cells a second. Paper drag, not a floor.

Gravity had a consequence that only showed over a minute of simulation: every word
ever knocked about ended in the same place, a heap along the bottom of the page.
The rug of punctuation is already there, and words drifting down onto it are
litter, not play. Without gravity a struck word slides and stops where the strike
put it, which is what a word on a sheet of paper does. `bat` no longer sets a
vertical kick either, because a bounce was the other half of the same mistake.

### A profile head is a dog

The silhouette was the real problem, and it was structural. A head built facing
`+z` and turned with the rest of the body by `yaw` -- always ±π/2 -- is seen from
the side: one eye, half a muzzle, an ear in the middle of the skull, no face. At
two lines tall there is no detail budget that repairs that. The reader gets a
quadruped with something on its head, and the something is a dog.

So the animal is built in two spaces. `blocks()` turns the body by `yaw` and the
head by `head_yaw` about the head's own anchor. `head_yaw` follows intent with a
`dt * 2.4` ease: ±0.34 rad while she is following or pouncing on something ahead
of her, 0.30 while asleep, zero otherwise. She looks at the reader, and turns her
head only when she has a reason to.

The proportions carry the rest, and each one is a decision rather than a number
picked to fit. Head `[0.86, 0.72, 0.58]` against body `[0.66, 0.50, 0.76]`: the
head is as wide as the body is long, which is the whole difference between a
kitten and a cat and cannot be traded for detail. Eye `[0.18, 0.23, 0.08]`, a
fifth of the face, with a white glint inside it. A muzzle short enough to sit under
the eye -- the seventy-seventh pass's long muzzle is what made a snout. Ears wide
at the base (`[0.34, 0.24, 0.20]`) and narrower at the tip (`[0.20, 0.24, 0.16]`),
with the pink of the inside only in the part that stands above the crown, because
pink lower down is a pink band across the forehead. A chin narrower than the
cheeks.

`height()` measures the boxes now (`pos[1] + size[1] * 0.5` against the cat's own
`pos[1]`) instead of returning a constant. The ears are the top of the animal, and
a constant maintained by hand reports the animal that was, not the one on screen.

### A box must never be wider than the one in front of it

`render.rs` lights every face it draws, so a top face left visible inside a
silhouette is a bright horizontal band, and a band across a face is a muzzle nobody
drew. That was the bug behind three failed attempts at a face: wide cheeks and a
jaw box behind a narrow nose produced a cream stripe from ear to ear. The rule that
fixed it is the one in the module header: stacked boxes overlap, and each is
narrower than the box in front of it, so the wider box hides its top face behind
the narrower one and the only faces that show are the ones turned toward the
reader.

The second half of the flattening is arithmetic. `lit` was `base * (0.52 + 0.48 *
lambert + rim)`; on a stack of boxes with slightly different normals that is a
staircase of brightness, which reads as stripes rather than as fur. It is
`base * (0.74 + 0.26 * lambert + rim)` with `rim` at 0.16 instead of 0.22, and the
animal reads as one colour with a lit side.

### Print the shape, do not read it

Two throwaway tools, both deleted as soon as they had answered: an ASCII dump of
the silhouette -- every box projected, sorted and printed in the terminal -- and a
numbers dump of the head's boxes in front-to-back order. The band across the muzzle
was found by looking at that ordering and seeing a wider box behind a narrower one.
No amount of reading `blocks()` would have shown it. The habit is worth keeping:
when a shape is wrong, print the shape.

Poses came out of the same argument. Sitting lowers the hips to a `[0, 1.08, 0.10]`
anchor and folds the hind legs under, so a sitting cat is seen front-on by the
reader; sleeping replaces the body with a loaf `[0.84, 0.58, 1.06]`, lays the head
along the front of it and curls the tail up by the nose. The tail is a quadratic
bow in the (z, y) plane with the root, bow and tip all outside the rump, because
the old tail centred on the body had half of itself buried inside the cat.

### Verification

`cargo test --offline --all`: 2144 tests (1087 in `nn-app`, 957 in `nn-core`, 100
in `nn-mcp`), 0 failed -- two of them new in `kitten.rs`: the face is what makes it
a kitten (head width against body length, the eye a fifth, two eyes and three nose
boxes), and the head looks at the reader while the body does not (`|yaw|` near π/2,
`|head_yaw|` under 0.5, two eyes at the same height). `cargo clippy
--offline --all-targets`: 0 warnings. `cargo fmt` was not run.

Frames at 0, 4, 6, 12 and 200 seconds of simulation time were rendered and
inspected: standing, walking, sitting and curled up, every one of them a kitten --
big head, big dark eyes with glints, cream muzzle, pink nose, pink-backed ears,
four socks, a curled cream-tipped tail, and no words at the bottom of the page.

`docs/nekopp-features.md` lost the gravity sentence from the token row, gained the
two spaces and the three bodies, and no longer calls her two and a quarter lines
tall. `python3 docs/build-pdf.py` then `--check`: up to date, 310,388 bytes, 72
pages A4, 112 bookmarks -- the same page count, 815 bytes heavier than the pass
before.

## Seventy-ninth pass: a cat with something to do

"the kitten needs to evolve by itself in the page and interact with things play etc
..". The seventy-eighth pass made an animal that looks like a kitten; this one is
about what she does. Until it, every mood was a point on the page: walk here, sit
here, slide this word a little. Nothing on the page answered her, and a page that
cannot answer is a kitten walking past a photograph of a room. A screensaver with a
cat in it, not a kitten in a page.

### The furniture left the text

Props were rows of characters written into `Page::text`: scenery, drawn once, good
for nothing but being walked past. `Prop` is public now -- `kind`, `pos` (the
centre of its box), `size`, its `cards`, a `wobble` and a `phase` -- and
`Prop::cards(lift, alpha, out)` lays the piece out again every frame at the offset
the wobble asks for. `furniture()` no longer writes into `text` at all: it builds
the cards and marks the box in `taken`, so the code is laid out around the piece
instead of the piece being made of the code. The box stays empty, and the page
keeps its own drawing separate from its own content.

That separation is what makes the two page kinds possible. `PropKind::is_perch()` is
table and shelf, and a plant, a lamp or a picture is something to hit; `Prop::top()`
and `Prop::distance_to` are the only geometry the kitten needs to know about a piece
of furniture, and the furniture never learns about the kitten.

### Dust with a velocity

`Mote` gained `vel` and `life`. Ambient dust wanders the way it always did; a puff
lives half a second to a second and a third. The simulation owns the position and
the app draws at it rather than wobbling the drawn position on its own -- the app
wobbling what it draws means a chase aims at a place the speck is not.

### A paw has one vocabulary

`Page::strike` is the whole of it, and it is one method because from the page's
side, being hit is being hit. Words within reach are knocked away carrying the
syntax colour of the line they came from. Furniture within 1.2 of its box takes
`bump`, which sets `wobble` and leaves the card layout alone. And four short-lived
motes are thrown from the paw, capped at 240 so a cat left alone all afternoon
cannot fill the buffer. `step_props` decays the wobble at 1.7 a second and resets
`phase` to zero when the piece is still again, so a struck table returns exactly to
the layout: a piece that settled a cell off would move the whole room a little with
every swat.

### One drive, not two

Restless and tired are not two things a cat is at once, so there is one number.
`rest` fills at 0.05 a second while she sits, grooms or sleeps, empties at 0.09
while she swats, chases, runs or pounces, and starts at 0.35. `idle = rest > 0.6`
shifts the weight table in `next_mood`: a rested cat reaches for the table, a cat
that has been chasing dust sits down. The same table drops what the page cannot
give -- no furniture means no perch, no dust means no chase, already perched means
no second perch -- and weights the mood she is in out of the next draw, except the
three neutral ones, because a cat that swats a table and then swats the same table
again is not playing.

### A cat gets where it decided to go

The first version of the swat arrived late and swung at nothing, because the mood
clock had been running since the decision. Four fields fix it. `arrived` is false
until she is within a cell of the thing, `patience` (10 seconds) is how long she is
willing to walk at something she cannot reach, and while she travels the mood timer
is held at 0.6 or more. On arrival the mood gets its whole `dwell` back, or `arm()`,
which sets 0.41 seconds and crouches, so the paw lands on the thing she walked to.
`struck` makes that land once per window rather than on every fixed step.

Bat, swat and perch run when the thing is more than five cells away. It is the
difference between a kitten that drifts toward a table and a kitten that decides to
be on the table and is.

### Arriving is not stopping

Two frames a second apart -- 62 and 63 seconds -- had the same cat in the same
place, which is the sort of bug a still frame hides and a run exposes. Wander
picked a point, arrived at it, and stood there for what was left of the mood, up to
four seconds. A patrolling cat does not park. `elsewhere()` now picks the next mark
when a walk or a lap reaches the one it had, and the same run moves 469, 490 and
510 pixels across three samples half a second apart.

### Verification

`cargo test --offline --all`: 2153 tests (1087 in `nn-app`, 966 in `nn-core`, 100 in
`nn-mcp`), 0 failed. Ten of them are new. `left_alone_it_finds_something_to_do_on_the_page`
runs a page and a kitten for three minutes with no input and asserts the whole
vocabulary happened: she swatted, perched and chased, the furniture shook, the words
moved and the dust was thrown. `a_swat_is_aimed_at_the_furniture_and_lands_on_it`
and `it_sits_on_the_table_it_climbed` pin the aim; `a_chase_follows_dust_that_keeps_moving`
pins that a chase follows rather than arrives; `the_sitting_clock_fills_while_she_rests_and_empties_while_she_plays`
pins the single drive. In `scene.rs`: furniture never lands on the rug, a prop's box
is left empty by the code, a bump shakes and settles, a strike throws dust that
dies, and dust is thrown further than words. `cargo clippy --offline --all-targets`:
0 warnings. `cargo fmt` was not run.

Frames at 0, 6, 20, 62, 62.5, 63, 66, 90 and 100 seconds were rendered and looked
at: a kitten standing among punctuation, walking, on a tabletop mid-patrol, sitting
on a shelf's edge with her paws over it, dust specks in the air, and a knocked-over
word lying turned on the rug.

`docs/nekopp-features.md` gained a row for the dust, a furniture row that sways and
a kitten row that climbs, and two paragraphs on what she picks and what a paw does.
`python3 docs/build-pdf.py` then `--check`: up to date, 312,308 bytes, 73 pages A4,
112 bookmarks -- one page more than the pass before, the same 112 bookmarks.

## Eightieth pass: a page with gravity, and a cat that climbs it

"no gravity effect, I want the cat to jump onto platforms of code, knock out text
which should fall, do things when staying in the same place, clean, sleep with zzz
etc ...". The seventy-ninth pass gave her somewhere to be and something to do;
what it did not give her was a page with weight. Props swayed and words drifted,
but a row of code was a colour, not a thing, so a cat could not be on one, a word
could not land on one, and the only vertical movement in the mode was a jump that
went nowhere. A page you can climb is a different mode from a page things slide
across.

### A line of code is a place

`Shelf` is a run of characters on one row, seen as an edge to stand on: `row`, `y`
(its top, in cells) and a `span` across the page, with `covers(x)` and
`nearest_point(x)` as the whole of its geometry. `Page::shelves` is rebuilt by
`shelves_from` whenever the page is laid out, out of the same `solid` grid the
layout already uses, and only above the rug: the gutter is a margin and the rug is
the floor, so neither is a ledge, and a cat can stand only on what the code
printed.

Runs are merged across gaps of up to `SHELF_GAP` (2.6 cell widths), because
`let x = 1;` is one line to walk along and eight separate things to fall off. The
rug is punctuation lying about on the floor and the floor is `FLOOR` under it, one
row above the app's own label line -- the same line the label has always been kept
clear, which is why a word that falls off the page lands in the same place a cat
does.

`shelves` is public and passed to the kitten as part of `Senses`, together with
`floor`. The kitten does not know what a shelf is made of; it asks `surface(x,
from)` -- the height of the highest edge under `x` that is no higher than `from` --
and gets a number. Three callers ask: a falling word, a falling cat, and the shadow
under her.

### One gravity, two bodies

`GRAVITY` is 26 cells per second squared, and `pub` because words and cat share the
number: a page whose letters fall faster than its kitten is a page drawn at two
scales. `BOUNCE` is 0.22, small because paper on paper does not bounce, and enough
that a knocked word gives a hop when it lands -- which is how a fall reads as a
fall rather than as a cut.

`surface_from` is a free function rather than a method because its callers hold a
mutable borrow of the list it is asking about: a word is moved while it asks what
is under it. It looks for shelves and for the flat tops of perches only -- a word
does not come to rest on a lamp's shade -- and returns the floor when nothing
qualifies, so no falling thing ever comes to rest in the air.

Tokens are dropped onto their ledge when the page is laid out, too, at
`surface_from`. A page whose loose words hang in the air over gravity has the
gravity switched off for the thing the reader notices first.

### The paw takes one thing

`knock_out` used to sort everything under the paw by x and lift it in one piece. A
paw is taller than a line and lands between two, so what it takes is the ends of
two different lines, and sorting by x alone interleaved them: the token that came
out was two half-words spliced. It now buckets by row into a `BTreeMap`, sorts each
row by x, splits it into runs, and lifts the longest run. A word knocked off a page
is still a word, and the page loses one thing per paw rather than a strip.

### Six lines up, and no jumping at holes

`CLIMB` is how high a bound reaches: six lines. It is used three times -- the leap
guard, the window `nearest_shelf_above` looks in, and `nearest_perch` -- so what she
can jump to and what she can jump from agree.

The arc is fixed and the time grows with the height
(`LEAP_TIME + rise * 0.05`), with `hop` scaled by `1.0 + rise * 0.1` up to 2.6. A
cat scaling a table does not fly over it by the height of the table: what the
reader sees is a bound, not a parabola scaled to the furniture. `leap` leaves `pos`
on the ground and animates `hop`, so the shadow the app draws can stay on the ledge
she left -- it is dropped to `surface(x, pos.y)` and shrunk with the height, which
is the difference between a jump and a cat that grew.

The bug that came out of the first run was a cat leaping at a hole. A landing used
to take the line out, so the place she was aiming at could be gone before she got
there, and she would spring at the gap again and again: bouncing in place at the
same cell for ten seconds. `move_by` now asks `senses.surface(goal[0], goal[1] +
0.05) >= goal[1] - 0.05` before it leaves the ground -- a place is leapt at only
while the page still holds it up -- except for dust and loose words, which a bat or
a chase catches on the way up. `nearest_perch` also refuses a perch under her own
paws (`prop.top() - at[1] > 0.6`) and `nearest_shelf_above` looks up rather than
across, so the perching weight can be raised to 2 idle and 6 active without her
spending the mood walking to something she can already reach.

### Land, gather, spring

`next_leap` is the pause between touch-down and the next bound: `0.3 + rng(0.0,
0.2)` seconds, during which she crouches (`crouch = 1.0`) and the leap is not
taken. It is one number and it is the whole of "a cat looks before it jumps". A
wave of jumps with no gap between them reads as a bouncing ball, whatever the arc
looks like.

### A landing is not a swing

`Page::strike` takes, `Page::land` does not: the same `bump` and a smaller `puff`,
and no `knock_out`. A landing that took the line out from under the paws would
leave her standing on nothing, and a page of ledges a cat cannot stand on is a page
with no ledges. What falls off a line falls to a paw, not to a landing. The caller
picks between them from `kitten.landed` and the strike flag, and `land` is what
makes the two dozen jumps of a long run free of holes she punched herself.

### Zzz and sparkles

`Floating` is a glyph the page holds up for a moment: character, style, position,
drift, `age` and `life`, and `alpha()` that arrives over a sixth of a second and
fades out. `Effect` on the kitten says which -- `Zzz` from `crown()`, `Lick` from
`muzzle()` -- and `Page::float` is capped at 32 so an idle afternoon cannot fill
the buffer. They are drawn as glyph cards from the page's own atlas, not as a
sprite, so a `z` is the same `z` as the code under it.

The same `Effect` drives the wash animation, which is the answer to "do things when
staying in the same place": a mood that keeps her still now has a paw moving in
front of her face, and the sparkle that comes off it is what tells a reader she is
washing rather than sitting.

### Verification

`cargo test --offline --all`: 2163 tests (1087 in `nn-app`, 976 in `nn-core`, 100
in `nn-mcp`), 0 failed. Ten of them are new. In `kitten.rs`:
`it_jumps_onto_a_line_of_code_and_sits_on_it` runs a perch on a line rather than a
prop, `it_steps_off_the_end_of_a_line_and_drops_to_the_one_below` pins that the
page is what holds her up, `she_gathers_herself_before_the_next_jump` that the
crouch comes before the bound, `she_does_not_jump_at_a_line_that_is_not_there_any_more`
that a hole is not leapt at, `a_sleeping_cat_puts_a_zzz_over_its_head`,
`washing_puts_a_paw_in_front_of_its_face`, and
`it_does_not_swat_the_furniture_it_is_standing_on`. In `scene.rs`:
`a_paw_on_a_line_takes_characters_out_of_it`,
`a_paw_between_two_lines_takes_one_word_not_two` (the splice), and
`a_word_knocked_off_a_line_falls_to_the_one_below`. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run.

The bench was given a `NN_TRACE` line and a `playground_seed` flag and run for 120
seconds at 1280 by 800 on seed 24301: 13 jumps, 49 characters taken off the page,
11 loose words, and the moods perch 24.6 s, wander 33.1 s, zoomies 19.7 s, chase
15.9 s, bat 11.8 s, groom 6.7 s, swat 4.7 s, sit 3.0 s, clean 0.6 s. The trace is
how the jumping-at-a-hole loop was found: a table of jump times with several
leaps inside a few seconds is a loop, whatever the frames look like. The page lost
49 of its 642 characters over the two minutes and ended with 11 words lying about.
The instrumentation was removed before the commit.

Frames were rendered and looked at: 3.6, 11.9 and 12.5 seconds zoomed on the cat
(mid-air between two lines with her shadow a speck below, then standing on a line
of code between `self.naps` and `kitten().purr`), 120 seconds full page (gaps
knocked through two comment lines with the surviving runs resting on their rows)
and 200 seconds zoomed (asleep in a loaf, `z` above the head, the hint reading
"asleep · Esc leaves").

`docs/nekopp-features.md` gained a ledges row, a tokens row whose words fall and
land, Zzz on the sleep row, a sparkles row, and two paragraphs on the page as a
place and on a landing not being a swing; the old claim that the words drift with
no gravity pulling them anywhere is gone. `python3 docs/build-pdf.py` then
`--check`: up to date, 314,273 bytes, 73 pages A4, 112 bookmarks -- the same 73
pages and 112 bookmarks, 1,965 bytes heavier than the pass before.

## Eighty-first pass: light on the page

"also where are my shader effects ? I want to see things like a rainbow trail of
colors when the cat jumps. particules, make it visually amusing and entertaining".
The eightieth pass gave the page weight; what it did not give it was a way of
saying that something happened quickly. One line of code looks like the next and a
jump leaves nothing behind, so the reader's eye has no reason to follow her. The
answer is light: a band of colour along the arc she flew, a ring where she lands,
confetti where a paw takes something.

### The light belongs to the simulation

The obvious place for a spark is the renderer -- it is the thing that draws, and a
particle is a drawing. It is the wrong place. A renderer that owns state makes the
mode's look depend on the frame rate and on how many frames a reader happened to
skip, and it makes the effect untestable without a window. So `spark.rs` is in
`nn-core` and owns all of it: `Spark` is a position, a velocity, a hue, a size, how
fast it grows, its age and its life, and `alpha()` is the whole of its shape in
time -- in fast, out over its life squared. `Page::step_sparks` steps them in the
same fixed loop as the tokens, `Page::glows` hands the renderer a `Glow` per spark,
and the renderer holds no state at all before the first draw. The tests that came
with it -- a ring widening as it fades, confetti fanning out across the wheel, a
spark gone when its second is up, the page never holding more than its share
(`MAX_SPARKS`, 512, oldest dropped first) -- need no GPU, and a frame at a given
time is the frame the run produced.

The simulation does not know what colour anything is. A spark carries a `hue`, a
number from 0 to 1, and the shader's `rainbow(hue)` turns it into one. That is what
makes a trail a rainbow instead of a palette: the hue steps a ninth of a turn per
piece, so the band runs through the wheel in the order a rainbow has it, and one
shader covers every effect -- blob, ring, star -- with the shape chosen by the
`z` channel of the instance colour and the seed in `w` turning a star's rays.

Light adds. The glow pipeline is `One`/`One` with no depth write, drawn last, so
overlapping sparks are brighter than one and there is no order to get wrong -- and
so three pieces of three hues in the same place are white. That single fact drove
the tuning: the hue step is small and the pieces are laid apart, because a trail
that cycles the wheel every two cells mixes itself to grey.

### Two bugs the frames found

The first was invisible in the code and obvious on screen: the glow instance buffer
was filled but never uploaded. `self.dots.upload(...)` and the glow upload sat on
different lines of the same block, an edit matched only one of them, and the draw
read whatever the buffer held -- identity matrices, so a field of white squares. A
grep for `upload(` found it in a minute; what it cost was the hour before that,
looking at the shader.

The second was mine and geometric: `smoothstep(0.26, 0.0, edge)` has its edges
reversed. `smoothstep` with `edge0 > edge1` is undefined in the spec and in
practice clamps to something that is 1 everywhere inside the ring's radius, so a
ring drew as a disc. `(1.0 - smoothstep(0.0, 0.22, edge)) * (1.0 - 0.25 * d)` with
`edge = abs(d - 0.82)` is a ring: thin at the rim, brightest where it is thickest.

### A trail is laid by distance

The first working trail was emitted every twenty-two thousandths of a second and
looked like a smear. The reason took a while, because the frames I was reading were
the wrong frames: the bench stepped the page at 0.05 s per step while the mode on
screen runs at a frame of its own, and a coarse step is a different run of the same
seed: one jump in the first twenty seconds where the fine step has three in the
first half-minute, and a completely different cat. The bench now steps at 120 a second, which is what the app does, and
`playground_time=9.45` means the frame a reader would have seen.

With honest frames the shape of the problem was clear. A temporal rule breaks at
both ends: a piece every few hundredths of a second piles up into a white column
when she pounces in place, and thins into a dotted line when she falls off a ledge
at ten cells a second. What a trail is, is a line in space, so the clock counts
cells instead of seconds -- `TRAIL_STEP` of 0.42 between pieces, measured from her
middle last frame to her middle this frame, so a walk, a pounce and a fall all lay
their light the same way. Each piece is left with the negative of her own velocity,
so it streams out behind her rather than gathering under her, and every fourth one
is a star so the band has something in it that turns. A test pins the property
rather than the numbers: the same seed watched at 120 and at 30 frames a second
puts the same band in the air (thirteen pieces and eleven).

The rest is numbers chosen from frames: a ring grows to three and a half cells and
fades in half a second; confetti is bright and small and falls at a fifth of the
page's gravity (an eighth of it); the halo that follows her is dim, because it is there to say she is
lit, not to cover the colours she has left behind. The glow also keeps more of its
brightness than the text when the page dims -- light outlasts the words it was
thrown over, which is what makes a page left alone still look alive.

### Verification

`cargo test --offline --all`: 2169 tests (1087 in `nn-app`, 982 in `nn-core`, 100
in `nn-mcp`), 0 failed. Six are new: in `scene.rs`
`a_ring_on_the_paper_widens_as_it_fades`, `confetti_fans_out_across_the_wheel`,
`a_spark_is_gone_when_its_second_is_up` and
`the_page_never_holds_more_than_its_share_of_light`; in `playground/mod.rs`
`a_jump_leaves_a_rainbow_behind_it` (a hundred and twenty seconds of one seed, and
at least four of the six buckets of the wheel show up along her trails) and
`the_trail_lies_along_the_path_and_not_along_the_clock`. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run.

Frames were rendered on seed 24301 at 1280 by 800 and looked at: 9.45 seconds,
mid-fall off a ledge, a magenta head coming off her crown through blue, cyan,
green, yellow and back to magenta at the tail with a star in it; 9.30 and 26.40
(the same band, shorter, because the fall has only started); 4.70, a pounce in
place, where the trail is three cells of colour rather than a smudge. The stills are
in `/tmp/pg` and are not committed.

`docs/nekopp-features.md` gained a light row in the Playground table and a paragraph
on why the light belongs to the page and not to the drawing. `python3
docs/build-pdf.py` then `--check`: up to date, 315,130 bytes, 73 pages A4, 112
bookmarks -- the same 73 pages and 112 bookmarks, 857 bytes heavier than the pass
before.

## Eighty-second pass: a cat that runs through the text

### Three bugs before it ran at all

The first version of the dash picked the farthest thing on the page. On seed 24301
that is a shelf nine lines up, so she ran to the point underneath it and stood
there for the mood's whole ten seconds of patience: the trace read `x33.0 → x10.7`
in three seconds and then `sp0.0` for six. A run is horizontal -- what a dash aims
at has to be on the level she is standing on, which is the far end of the ledge
under her, or a loose word at about her height, and the surface lookup decides
whether it is a place at all. `run_target` is that rule and nothing else.

The second was arithmetic. `move_by` reports a rake at her feet, and her feet are on
top of the glyphs she runs along: a card's centre is half a line below the shelf it
makes. The rake was placed at `pos[1] + 0.2` with a box of half a line, so the
swathe covered from a third of a line under her feet to a third of a line over
them, and missed every character of the line she was taking apart -- one rake in
ninety seconds, four characters. It is `pos[1] - 0.5` now, and the same ninety
seconds takes fifty.

The third was the timer. A mood that is going somewhere tops its own clock up while
it travels, which is right for a walk that is late and wrong for a run: the dash
that had arrived at its target but not within a cell of it stayed a dash for ten
seconds of standing still. A run ends when its clock does. Arriving early only means
the paw comes down on what she found.

### A rake is not a swing

The paw is a deliberate thing and takes the longest word under it. A shoulder at
eight cells a second has no manners: it takes whatever is in a swathe of a character
and a half at her own row, every two characters of travel, and throws the words it
finds along her heading. Three consequences, each of which is why a number is what
it is. The swathe is narrow because a rake happens a dozen times a dash -- wide, and
one run would take the page apart rather than mark it. The pieces are reported
*behind* her, because a rake under her own paws would split the shelf she is standing
on and drop her mid-run. And the gap it leaves is narrower than `SHELF_GAP` (2.6
cells), so the line stays a ledge with holes in it, which is the only way a cat can
keep destroying the thing it is running on.

The light follows the same rule: a piece every step of travel *scaled by 1.35* and
smaller than a jump's, laid at her paws and not at her middle. Twenty pieces of the
size of a leg in a row is a cloud; twenty smaller pieces a third further apart is a
speed. A ring and a burst mark the moment she sets off and a ring the moment she
stops, so the run has edges.

### The trail clock had to stop resetting

`the_trail_lies_along_the_path_and_not_along_the_clock` broke, and it was right to.
The old code reset the clock to a step whenever a piece was due, so a frame that
flew more than one step threw the remainder away: at 120 frames a second a jump laid
twenty-one pieces, at thirty it laid five. The clock carries over now and a frame
lays as many pieces as it owes, each one placed at the fraction of the frame's path
where she crossed that step -- by distance, so a slow renderer draws the same band
as a fast one. The test measures density (pieces per cell flown) rather than a count,
because two runs of one seed do not have the same jumps and what has to hold is the
rule, not the run. Both rates are within a fifth of each other and within a quarter
of the exact density.

### What makes her tired

The mood table became a function, `weights`, so a test can ask what a cat wants
without sampling a distribution. After a dash -- and only after a dash -- sitting,
grooming and cleaning are worth 9, 9 and 12 rather than 1, 1 and 1, and another dash
is worth nothing. That is the whole of the characterisation: there is no script, and
the animal reads as an animal because what it has just done changes what it wants
next. On seed 24301 over five minutes: 18 dashes, 5161 of 36000 frames at a run
(14%), and 52 characters off the page, which leaves 593 of 645 -- a page that is
being played in and not one that is being eaten.

### Verification

`cargo test --offline --all`: 2173 tests (1087 in `nn-app`, 986 in `nn-core`, 100 in
`nn-mcp`), 0 failed. Four are new: in `scene.rs`
`a_run_takes_characters_out_of_a_line_and_leaves_it_standing` (the line is still one
ledge afterwards, and the word came away downwind); in `kitten.rs`
`a_dash_is_faster_than_a_run` (a real run across a real shelf, topping out above a
run and never above a dash) and `after_a_dash_the_cat_wants_to_wash_itself` (the
weights, not the generator); in `playground/mod.rs`
`a_dash_leaves_a_trail_and_a_line_full_of_holes` (four minutes of one seed: something
dashing, trail behind it, a ring where it began or ended, and characters gone).
`cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not run.

Frames were rendered on seed 24301 at 1280 by 800 and looked at, from 6.1 to 7.5
seconds at two tenths intervals: the ring and the confetti where she sets off, the
streak lying along line 29 as `def sleep(self, seconds=0):` loses `secon`, then
`se c`, then `s l f   c d 0)`, the loose characters tumbling over the line below,
and the ring where she stops. The streak first came out as a rainbow cloud over the
line above her (too big, too high) and was cut down and dropped to her paws. The
stills are in `/tmp/pg` and are not committed.

`docs/nekopp-features.md` gained a run row in the Playground table and two paragraphs
-- on why a run aims at its own level and what a rake is not, and on why a wash
follows a gallop. `python3 docs/build-pdf.py` then `--check`: up to date, 317,246
bytes, 74 pages A4, 112 bookmarks -- one page more than the pass before.

## Eighty-third pass: a page that mends itself

### Why the fun was throttled

The complaint was that the cat was boring: not enough activity, not enough mayhem.
Measured, it was true and it was the mode's own fault. On seed 24301 over five
minutes the old build took 52 characters off the page in 18 dashes, 14% of frames
at a gallop. That is not a tuning accident, it is the shape of the design: the page
could not be damaged much because nothing ever put it back, so the animal had to be
kept too calm to damage it. Every fix in this pass follows from removing that
constraint -- the page prints back what it loses, and only then can the cat be let
off the lead.

### Two bugs in where a run goes

The dash was running along the rug. A rake refuses characters under 0.9 alpha,
because the page's text is printed at 1 and its scenery is not, and the rug is
drawn in characters at 0.3 -- so a run along the floor took nothing at all. It
looked fine in the trace and did nothing in the stills: 84% of rakes landed on a
row that could not be taken. `run_target` had asked the surface lookup whether the
point it was aiming at was at her own level, which on the floor is trivially true
for every x on the page. Both fast moods now ask `floor_spring` first: standing on
the rug, they climb onto the nearest line over their heads before they go anywhere,
and `lap_target` runs along her own piece of a line rather than a point on another
piece at the same height, which she would run at and never reach.

The second bug was self-inflicted and only showed up once the swathe got wider. The
rake is reported at `pos[1] - 0.5`, the row the glyphs she is standing on are
printed in, and a swathe of 1.1 cells either side reached a quarter of a cell past
her centre: she took the glyphs under her own paws, the shelf under her went, and
she fell mid-run. Half the runs ended on the rug for that reason. The rake now
happens a character and a half behind her and reaches nearly three characters
wide, so the furrow ends under her tail; one rake every 1.5 cells overlaps the
last, and the line stays a ledge until she has left it.

### The mending

`Page` keeps `missing`, the characters it has lost with the cell each stood in, and
`mending`, the ones on their way back. `mend` is called in the same fixed-step loop
as everything else: three characters a second are taken out of the memory at random
and printed at nothing, and each fades in over four tenths of a second; when it is
whole it rejoins the text and the ledges are laid out again. Random rather than in
order, so a furrow fills in as patches and not as a zipper. The memory is capped at
900 characters, so an afternoon costs a fixed amount; past the cap the oldest loss
is forgotten and the page ends up with the lines the cat has not been at lately.

The rate is the whole balance and it is deliberately near the destruction rate. At
3 a second the page is never quite empty and never quite whole: on seed 0x5EED over
five minutes the least it ever held was 586 of 645 characters. Faster and the
damage is never seen -- which is what the old build looked like -- slower and a
long session leaves a page of gaps with a cat walking about in it.

### Both ends of a run

A run used to end with a paw coming down on one word. Now the frame it starts and
the frame it ends are both `blast`: everything within 2.8 cells of her paws comes
off at once, each run of characters thrown along the line from her to where it
stood, so the middle of the hole is empty and the edges look like a burr. It fires
on the change of `raking`, which is either fast mood, so a lap gets the same two
ends as a dash. On one end the blast lands in a furrow she has just emptied and
takes nothing; on the other it lands in text. Fourteen frames in 240 seconds lose
eight characters or more, the biggest 26 -- against a rake's two or three, which is
how the test tells them apart.

Furniture sheds the same way: `chip` pops one to three cards off a prop within
reach and throws them up and away from the hit, so a table that a body arrives at
loses its middle instead of sitting there being hit.

### More of everything, and the numbers

The zoomies became a run rather than a stroll: `ZOOM` at 1.7 a run, raking and
streaking like a dash, three to five and a half seconds long, worth 8 rather than
3 while she has energy. A dash with more than 1.2 seconds left on its clock takes
another leg straight on instead of stopping where it arrived, and one leg in three
is the line over her head, which is how a run up the page happens at all. The rest
of it is the table: dash 7 to 12, lap 3 to 8, the bar for resting 0.6 to 0.75, sits
and washes 2.4 to 3.6 seconds down to 1.4 to 3.6, and a dash drains 0.2 of her rest
a second rather than 0.14.

Six seeds, three minutes each: 16 to 29 runs (one every seven seconds), 26 to 53%
of frames raking -- 40% on average, against 14% at a gallop before -- and 180 to 680
characters lost and printed back, against 52 lost and none returned.

### Verification

`cargo test --offline --all`: 2182 tests (1087 in `nn-app`, 995 in `nn-core`, 100 in
`nn-mcp`), 0 failed. Nine are new. In `scene.rs`: `the_page_prints_back_what_a_run_took`
(a rake, then the page at its own rate, ending with the line whole and the ghosts
gone), `a_blast_takes_a_radius_where_a_paw_takes_a_word` (the same page hit both
ways, and nothing left standing in the hole), `the_furniture_sheds_what_it_is_made_of_when_it_is_hit_hard`
and `the_page_remembers_only_so_much_of_what_it_lost` (the cap, exactly). In
`kitten.rs`: `a_lap_is_a_run_that_rakes_the_text`, `a_dash_with_time_left_on_its_clock_runs_another_leg`
and `a_run_from_the_rug_climbs_before_it_goes`. In `playground/mod.rs`:
`a_page_under_a_cat_that_never_stops_is_never_emptied` (five minutes, asserted
every frame) and `the_end_of_a_run_blasts_a_hole_in_the_page` (a frame that loses
eight characters, which no rake can do). `cargo clippy --offline --all-targets`: 0
warnings. `cargo fmt` was not run.

Frames were rendered at 1280 by 800 on seeds 24301 and 123456 and looked at, in
strips of nine at half a second: a rainbow through the air, the cat landing with a
hole behind her, `static int laps(int n) { retu   * 1; }` with its middle gone,
loose words and specks of dust and confetti all over the bottom of the page, and
the comment line reading `# three is enough to be goin  o  w th`. The mending is
not visible in a still, which is why it has a test rather than a picture. Stills
are in `/tmp/pg` and are not committed.

`docs/nekopp-features.md` gained three rows in the Playground table -- the lap, the
blast, the mending -- and four paragraphs: the two ends of a run, the zoomies,
why the page prints itself back, and furniture that sheds.
`python3 docs/build-pdf.py` then `--check`: up to date, 319,943 bytes, 75 pages A4,
112 bookmarks.

## Eighty-fourth pass: a cat that goes up the page

The last pass made the cat destructive in a straight line. A page of text is not a
straight line, it is a cliff -- a hundred and twenty columns of it stacked forty rows
high -- and nothing in the animal went up it. `Mood::Perch` still exists and still
takes the nearest ledge above and sits on it, which is one line, once, and reads as a
cat getting on to something; a cat going up a page is a different thing that this
build did not have.

### The ascent

`Mood::Climb` is a run of bounds rather than one. Each is aimed by `foothold_above`,
which is the highest thing one bound can reach: six lines of rise and three of lean,
from the lines of code and the flat tops of the furniture alike, because to a climbing
cat a table and a line are the same job. Highest and not nearest, because a cat that
took the next line up would take thirty bounds to cross the page upwards and stop at
every one; the two rows at the top are excluded, because that is where its ears go and
a foothold there would have it clamped off the ledge it landed on.

The landing point is offset a character or two along the foothold, clamped inside it
and away from its last character, so an ascent zigzags up the page instead of rising
in a column: a cat on a lift is not a cat climbing. Nothing about the mood runs on a
clock -- the clock is refreshed at every foothold -- so what ends it is the page
running out, which is the answer to the question the mode is asked. `topped` is
reported once, on the frame the picker finds nothing, and the page answers with two
rings, a puff of dust, two dozen pieces of confetti and a card off anything in reach.

It is worth 12 in the mood table, which is what a dash is worth: on a full page with
energy left, going up is as likely as running, and a climb is the bigger event of the
two.

### The claws

A climb damages the page with its back paws, so `Page::claw` is the rake's shape and
half its height: one row, two words, thrown down and outwards. Half a line of height
is the whole of the design -- the claw is fired from the row she has left, and the row
she is landing on has to survive, or the ascent has nowhere to put its feet. `kitten`
fires one every 0.9 of a line she rises, from `step_leap`, guarded to stay a line below
the foothold she is going to, so a bound of five lines pulls five times and a furrow
twenty rows long is what a climb leaves behind. Furniture in reach of the same claws is
shaken and sheds a card or two.

### What two measurements caught

The first was a foothold 0.5 of a line up. `foothold_above` accepted anything more than
half a line over her head while the leap itself needs 0.6, so a table top 0.55 above a
line she was standing on was a target she would stand under for ever, waiting for a jump
that never came. The floor of the picker is 0.75 now, above the leap's own threshold.

The second was worse and is why the climb's reach is one bound. A foothold can be over
the next column and not over hers -- a page of code is ragged -- and the first version
walked the difference before jumping. What that produced was a cat walking to the end of
her own line, falling to the floor eight lines below the foothold, walking the rest of
the way along the rug, and then standing under a target that was out of reach from
there: four of six seeds had her falling mid-climb and finishing at half the height. The
fix is to keep `REACH` at three cells and let the ascent end where the page does -- "the
page above her column has run out" is an honest reason to stop, and the next climb starts
from wherever she comes down.

### The mending had to keep up

A climb takes three to five times what a run does, and the page was still printing at a
flat three characters a second with a memory capped at 900. On the largest page in the
seed set it ended its five minutes at 57% of its characters, which is a cat walking about
in gaps. The rate is now proportional to the damage -- three a second plus six times that
in proportion to how much of the page is missing, so seven times at a page with nothing
left -- and the cap is 2400, near enough a whole page. A run's furrow still sits in front
of a reader for the few seconds it takes to read as damage, and a wrecked page catches up
with itself.

### The numbers

Six seeds, five minutes each: 8 to 16 climbs (one every 19 to 37 seconds), 7 to 13 of
them reaching the highest place a cat may stand, 38 to 48 landings, and 970 to 1805
characters taken off the page and printed back. The page ends at 78 to 95% of the
characters it started with, and the worst frame in the set is 79%. Before the rate change
the same seeds ended between 57 and 95%.

A climb is two to four bounds of four and a half to five and a half lines, six to nine
seconds of the cat's time, and about 3% of the frames of a five-minute page -- the same
order as a dash, which is what makes it something a reader sees rather than a feature
they have to go looking for.

### Verification

`cargo test --offline --all`: 2187 tests (1087 in `nn-app`, 1000 in `nn-core`, 100 in
`nn-mcp`), 0 failed. Five are new. In `scene.rs`,
`a_claw_takes_the_line_she_has_left_and_nothing_over_her_head` -- the row is emptied, no
character above the swathe is touched, and everything thrown is falling. In `kitten.rs`,
`a_climb_aims_at_the_highest_foothold_one_bound_can_reach` (nothing standable within
reach is left behind), `a_climb_ends_where_the_page_does_and_nowhere_else` (the mood only
ends with the picker empty), and `the_claws_of_a_climb_are_in_the_line_it_has_left`
(every reported claw under her feet and clear of the foothold). In `playground/mod.rs`,
`a_climb_goes_up_the_page_and_takes_it_apart`. `left_alone_it_finds_something_to_do_on_the_page`
now asks for a climb and for a claw among the things a minute of the page contains.
`cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not run.

The page was then dumped as text and read at the four seconds of a real ascent -- seed
24301, the first climb, 92.74, 93.63, 94.55 and 95.01 seconds -- and the four dumps
diffed against each other. It is the check the numbers cannot make. At 92.74 she stands
at line 24 of 40, at 93.63 at line 28, at 94.55 at line 32, and at 95.01 her body covers
lines 35 to 38 with the ceiling at 37.6, so the ascent ends where the page does. What the
rows do in between is the design: at 93.63 the two rows under her paws have lost `pi` and
`ur(e` out of the middle of `pit"ur"`, at 95.01 line 31 reads `e   (ef,ims3:` where it
began as `efur(ef,ims3:` -- a claw takes the row she has left, not the one she is aiming
at, and it takes it only in the five columns she came up through, the rest of the
sentence still there. Nothing above her head is ever touched. The dumps are in
`/tmp/pg/climb-map.txt` and are not committed.

`docs/nekopp-features.md` gained three rows in the Playground table -- the climb, the
claws, the top -- and the mending row rewritten for the new rate, plus two paragraphs:
what an ascent is, and why a claw goes in the row she has left. The kitten row now lists
going up the page. `python3 docs/build-pdf.py` then `--check`: up to date, 322,208 bytes,
75 pages A4, 112 bookmarks.

## Eighty-fifth pass: a wall of furniture, and a cat that leaves it

The last pass taught the cat to go up the page, and it found the page's ceiling quickly:
the code only reaches so far to the right, so past the longest line there is nothing to
climb and nothing to land on. The right hand margin was dead space with a cat's whole
width of page beside it. This pass furnishes it and gives the top of it somewhere to go.

### The wall

`wall()` lays seven pieces against the right edge: a sofa standing on the rug, then a
shelf, a fern, a table, a topiary, two more shelves. Designs are new -- `SOFA` at 19 by 5,
`FERN` at 11 by 5, `TOPIARY` at 9 by 5, `FLOWERS` at 10 by 5 -- and `PLANT`'s stems were
double-backslash rows that had been drawing nothing. Each piece's top is `height.max(3 +
jitter)` rows over the one below, which measures out at four or five lines: one bound, and
not a lift. `stand()` does the placing for both the wall and the scatter now, marking the
whole box in `taken` and refusing an overlap or a box off the page, so a piece either
stands where it can be stood on or is not placed at all.

They are right-aligned with zero to two columns of play, because a wall is stacked by
someone rather than laid out by a grid engine. `is_perch()` now also says yes for `Sofa`
and `Plant` -- a cat ends up in a plant -- and `PropKind::sway()` scales the wobble
amplitude: a plant and a lamp move four times as far as a shelf. The wall is placed first
and the one or two scattered pieces afterwards, confined by `left_of_wall`, so the room's
loose furniture stands to the left of the wall instead of in the middle of the way up it.

### The swoop

`Mood::Swoop` is the other half of the circuit: the climb exists so there is a top to
leave. A leg is `SWOOP_LEG` = 24 cells across -- a third of the page -- and `swoop_target`
picks, among the ledges 0.8 to 9 lines below her, `x = (pos - 24).clamp(lo, hi)` with
`score = -(|across - 24|) - drop * 0.2`. Nearest-to-a-leg rather than farthest, because
what is being built is a run of leaps across the page and one edge-to-edge jump is a fall
with a cat on it. `SWOOP_MIN` = 7 keeps her from a leg that would land where it took off.

The clock is refreshed on every landing, so what ends a swoop is the page running out and
not a timer, and the top of a climb calls it directly: the `None` arm of `Climb`'s ascent
now starts a `Swoop` if there is one to be had. `move_by` gives a swoop its own leap gate
-- it gathers and jumps instead of walking -- and `step_leap` times the flight by its
length: `SWOOP_TIME` 0.45 s plus 0.02 s a cell, with a hop of `sin(pi t) * (1.2 + across
* 0.055).min(3.2)`, so a long leg is flat and a short one is a bound. It costs 0.18 of the
rest scale, the most tiring thing on the page, because what a cat does after flying the
width of the paper is wash.

### The rainbow, and the landing

The trail of a swoop is the trail of a jump at twice the width: `emote` asks for two
ribbons per piece, the second half a cell over and half the size, and its colour is a
whole turn of the wheel along the arc -- `(trail_hue + jump * 1.15) % 1.0` -- instead of a
step per piece, so one leg is one rainbow from tail to landing. A star in every third
piece rather than every fourth, and a 1.9-cell aura instead of 1.2. What the landing does
is the one exception on the page: every other landing only shakes the paper, because a cat
that took the line out from under its paws would have nowhere to stand, but a swoop coming
down `blast`s four cells out of the row it lands in, throws everything within sixteen, puts
two rings and confetti on the page and shakes the furniture in reach, and then a dash runs
along the wreckage. That is the damage the whole circuit is for.

### The numbers

Six seeds, five minutes each, from the current constants: 3 to 8 swoops, 4 to 15 legs
between them, legs of 7.1 to 32.3 cells across and 0.0 to 8.9 lines down, 17 to 21 dashes,
the highest point 37.0 to 37.6 of a 40 line page, 783 to 2,055 characters taken off the
page, 183 to 218 sparks alive at a swoop landing, and a hue span of 0.99 to 1.00 -- the
whole wheel, on every seed. Seed 99 is the quiet one at 3 swoops: the wall is on the right
of the page and whether she gets up it is a matter of where her wandering has put her, so a
swoop is a thing that happens some minutes and not others. That is the same shape as the
climb, and it is the reason a five minute window is the measurement.

### What the tests caught

Two of the old ones were wrong rather than the code. The furniture test computed the rug as
`rows - RUG_ROWS` -- a row index, 35 -- and compared it with a cell height, 5.0, so every
piece counted as standing on the floor; it is `FLOOR + RUG_ROWS` now. And
`a_dash_with_time_left_on_its_clock_runs_another_leg` waited two seconds for a second leg
from a six-cell line, which on this page aims at the wall fifty-five cells away: the window
was measuring the page's furniture rather than the rule. It now puts her at the end of the
first leg and asks for the second one there, which is what the rule is. The first attempt
at a fix was to cap how far a dash may aim, and that was wrong in a way the suite caught:
with the cap she never crossed to the right hand side of the page at all, and
`left_alone_it_finds_something_to_do_on_the_page` lost its furniture. A run across the page
is the feature, not the bug.

### Verification

`cargo test --offline`: 2,191 tests (1,087 in `nn-app`, 1,004 in `nn-core`, 100 in
`nn-mcp`), 0 failed. Four are new. In `scene.rs`,
`a_wall_of_furniture_stands_up_the_right_of_the_page` -- five or more pieces, a sofa and a
plant among them, each top 3.5 to 6 lines over the last, no piece standing in the one below
it, every piece a perch, and all of it right of 60% of the width. In `kitten.rs`,
`a_swoop_heads_for_the_far_side_and_down` (a leg of 7 to 24 cells, 0.8 to 9 lines down, and
a surface at the far end to land on) and `the_top_of_a_climb_is_where_the_swoop_begins`. In
`playground/mod.rs`, `a_swoop_lays_a_rainbow_across_the_page`: over five minutes it finds
the swoops, the widest band laid is over 30 cells, five of six colours turn up, and more
than ten characters come off the page under a landing. `every_mood_has_a_name_and_a_pose`
gained `Climb`, `Dash` and `Swoop`, which had never been in it.
`cargo clippy --offline --all-targets`: 0 warnings, which is why `stand()` takes its row
and column as one pair and its page size as another. `cargo fmt` was not run.

`docs/nekopp-features.md` has four new rows in the Playground table -- the wall, the swoop,
the rainbow, the landing -- and three rewritten ones (the furniture, the kitten, the light),
the introduction now says what is against the right edge, and two paragraphs follow the
climb: what a wall of furniture is doing on a page, and what a swoop is and why its landing
is the only one allowed to take the line. `python3 docs/build-pdf.py` then `--check`: up to
date, 325,273 bytes, 76 pages A4.

## Eighty-sixth pass: a cat that always wants up, and super jumps across the page

The last pass furnished the right of the page and gave the cat a way off the top of it.
Two things were still wrong with what came out. The descending run of legs was one-way:
having come down into the code there was no way back up except walking the whole width of
the page, so the wall was climbed once and then abandoned. And the cat was still a cat that
did whatever the page offered next -- an animal in a room rather than one with a plan.

### Wanting up

The mood table is the character of the animal, and it now says one thing about her before
it says anything about the page: `weights` scales climbing, perching and the super jumps by
how far up the page she already is, `1.0 + low * 0.6` where `low` is the fraction of the
page under her feet, so going up is worth half again as much on the rug as at the ceiling.
Nothing else changed weight. What that buys, over six seeds of five minutes: climbing 17 to
26% of frames, super jumps 12 to 17%, dashing 13 to 20%, sitting and washing 7 to 12%, dust
1 to 3% -- a vertical animal that still wrecks the page it is climbing.

Two smaller things follow from the same idea. A run of legs that ends on the furniture ends
in a `Climb` rather than a `Dash`, because the wall is what there is to go up and a dash
along a furniture top is a run over an empty box; a run that ends in the code still ends in
a dash, which is what the damage is for. And a pointer holds her: `choose` refreshes the
clock while a reader's mouse is on the page, so the low-half pull cannot take her off the
mouse a second after the mouse stopped moving.

### The super jump

`SWOOP_LEG` went from 24 cells to 34, which is the width of the page less a margin: one
bound from one side of it to the other. `swoop_side` now decides what a leg is allowed to
land on from the direction it is going -- to the right the flat tops of the furniture, to
the left the runs of code -- because the two halves of the page are the two ends of the
jump and a leap that stayed on its own side would be a jump along a shelf, which is what a
dash is for. A leg's height is one-sided too: `drop` must be between 0.8 and 9 lines going
left, and between `-SUPER_RISE` and 9 going right, so leaving the top of the wall is always
leaving it downwards and coming back to the furniture is always coming back upwards. Five
lines is a table's top over a shelf, and under what one bound climbs.

`swoop_target` tries both directions and keeps the better score, `-|across - 34| - drop *
0.2`: a rise therefore beats a fall of the same length, which is the same preference as the
mood table in the small. The direction of a run is remembered in `Kitten::swoop_dir` and
every later leg uses `swoop_side` with it, so a crossing is a crossing and not a bounce: a
cat that picked the best leap from wherever it landed would ping-pong between the wall and
the code for the rest of the session.

What it looks like from the trace of seed 24301: `(21.6, 2.9) -> (55.4, 7.0)` is a leg out
of the code and up onto the sofa, `(31.2, 2.0) -> (63.9, 7.0)` is one of 32.7 cells and five
lines up, `(63.0, 35.9) -> (29.2, 27.0)` is the way back down off the top of the wall at 33.8
cells across and 8.9 down, and `(54.4, 7.1) -> (64.0, 12.0)` is a 9.6-cell hop from one
piece of furniture to the next, which is the wall being taken in leaps rather than climbed.

### What the tests caught

The first version allowed a rise in both directions, and `the_top_of_a_climb_is_where_the
_swoop_begins` failed with a target at 37.0 from a cat standing at 36.0: she was leaving the
top of the wall by jumping *up* into the code above it, which is not leaving. Hence the
one-sided height rule. The first version of the bias was `1.0 + low`, twice as much at the
floor, and `left_alone_it_finds_something_to_do_on_the_page` reported `never chase` -- the
long chained moods ate the decision budget and dust chasing never came up in three minutes.
`the_pointer_reaches_the_kitten_as_a_place_on_the_page` caught the pointer problem on its
own: the cat walked to the mouse, the mood clock ran out on the way, and a new mood had her
off the wall while the reader's mouse sat still.

### The numbers

Six seeds, five minutes each: 12 to 16 climbs, 17 to 24 super jumps, 29 to 42 legs of which
8 to 14 gain ground (the best 5.0 to 6.0 lines), 15 to 23 dashes, the highest point 37.0 on
every seed against a ceiling of 37.6, the mean height 13.1 to 17.1 of 40, and 1,042 to 1,482
characters taken off the page -- which is in the same range as the pass before, so the extra
verticality has cost the damage nothing.

### Verification

`cargo test --offline`: 2,193 tests (1,087 in `nn-app`, 1,006 in `nn-core`, 100 in `nn-mcp`),
0 failed. Two are new: `a_super_jump_goes_back_up_to_the_furniture` (some line of code in
the middle of the page can reach a furniture top in one rightward leg, gaining no more than
`SUPER_RISE` lines and landing on a flat top) and `the_lower_she_is_the_more_she_wants_up`
(the climb, perch and swoop weights sum higher on the rug than in the middle of the page).
`a_swoop_heads_for_the_far_side_and_down` now asks for the direction as well as the leg, and
`cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not run.

`docs/nekopp-features.md` gained a row -- "Wanting up" -- and rewrote the swoop row for the
crossing, the kitten row and the introduction; the two paragraphs from the last pass are now
three, on the drive upward, the crossing both ways, and the landing. `python3
docs/build-pdf.py` then `--check`: up to date, 327,584 bytes, 77 pages A4.

## Eighty-seventh pass: fireworks when the page goes out

The last pass made the mode vertical. What it still lacked was an event: everything on the
page is the cat doing something, and a page being replaced -- which is the one thing that
happens to the mode rather than in it -- was a two-and-a-half-second crossfade that a reader
would catch at the corner of an eye if they were looking. The request was for the page to
change every three minutes in a display of fireworks over the whole of it.

### Three minutes

`REBUILD_AFTER` went from 300.0 to 180.0. Five minutes is the length of a screensaver and the
page is not worth that much of anybody's afternoon: three is long enough for her to take the page
to the ceiling and back -- the pass before last measured the top of the page reached on every
seed inside five minutes, and it is reached inside three -- and short enough that the rebuild is
something a reader is likely to be present for rather than something they come back to. A page
therefore changes twenty times an hour rather than twelve.

`SLEEP_AFTER` is also 180, and the two clocks are different: the sleep clock is the window's
idle clock, the rebuild is the page's. An abandoned window therefore puts a new page up in
fireworks in front of a cat that is already curled up on it, which is exactly the screensaver
reading and needed no special case. The dim at four minutes is untouched -- an idle desk now
dims on its second page rather than its first.

### A volley, not a flash

A rebuild is now a schedule rather than a fade. `Shell` is a private struct -- where it leaves
the paper, where it comes apart, its hue, its fuse, and whether it has gone up -- and
`plan_fireworks` fills `shells` with `SHELLS` of them, twenty-four, spread across the width of
the page with a fifth of a cell of play either side so the volley looks laid out rather than
measured. Each fuse is `slot * SHELL_SPAN` plus up to 0.12 s of jitter, `SHELL_SPAN` being
3.8 s, so a shell leaves every sixth of a second and there is no frame where two go up
together. Bursts break between 45% and 92% of the way up the page: a shell that came apart at
the rug would be a firework in a cellar.

Colours are the whole wheel. Each shell's hue is `trail_hue + slot * 0.85`, so the volley runs
from one end of the colour wheel to the other across the width of the page; inside a burst
`Page::firework` fans its sparks over half the wheel again, which gives one shell a colour with
an edge of its neighbours on it. The opening burst is a single shell out of the middle of the
paper at the first frame of the new page, before any fuse has even gone down.

`step_fireworks` runs on the display's own clock, after the crossover, because a display
outlives the two and a half seconds the old page takes to fade. It takes `self.shells` with
`std::mem::take` and rebuilds it, because bursting needs `&mut self` while the shells are being
walked; what has not come off the fuse goes back into `keep` and what has gone up goes into a
`bursts` list that is fired afterwards, outside the borrow. A shell that reaches zero the
first time is one going up: it leaves a `rocket` streak along the path it is about to take and
gets a fresh fuse of 0.34 s, which is how long the streak takes to arrive.

### What a burst costs

`burst` is four things at once: a shockwave, sixty sparks, a ring of twelve stars on its outer
edge and a chip out of the furniture within nine cells, which puts ninety-one new sparks on the
page per shell and 2,275 over a display. Three spark primitives are new, in `spark.rs`.
`Spark::shockwave` is a `Ring` of 0.5 cells growing at 26 cells a second for 1.4 s, four times a
landing's ring and twice its life, because this wave is not answering a paw: it is the paper
itself changing, and it has to reach the far side. `Spark::rocket` is a `Blob` of 0.85 cells with
a downward pull of -1.4, so the streak rises rather than falls -- what it is following is going
up. `Spark::firework` is the burst itself, thrown at 8 to 18 cells a second, alive for 0.9 to 1.6
s at 0.22 to 0.42 cells, three in ten of them stars.

`MAX_SPARKS` went from 512 to 960 against the renderer's `MAX_GLOWS` of 1,024. An intermediate
version of this pass was sixteen shells over 4.2 s at forty-four sparks a burst, and it peaked
at 171 to 240 glows against a baseline of 106 to 133 for the cat's own traffic over six seeds:
a large jump and not a display, because an explosion in the same traffic class as a jump reads
as a lot of jumping. Twenty-four shells over 3.8 s at sixty sparks gave 358 to 454. It does not
fill the buffer, so the cat still gets half of it, which matters: she is on the page and she
does not stop for the fireworks.

Measured over six seeds, at 60 frames a second for the nine seconds after a rebuild: peak 358
to 454, reached between 3.0 and 4.1 s in; still over 250 until 4.25 to 4.55 s; back to exactly
the pre-rebuild count by seven seconds, so nothing of a display is left when the next page has
settled. Hues span the wheel 0.00 to 1.00 on every seed; the light reaches from 10 cells to 68
across a page of 66, and from 2 to 43 lines on a page of 40, which is sparks flying past both
edges of it rather than stopping at them.

### What the tests caught

`the_page_is_rebuilt_every_five_minutes` became `the_page_is_rebuilt_every_three_minutes`: it
was written against `REBUILD_AFTER` and passed unchanged, which is the point of writing it that
way, and the name and its message were the only things that had gone stale. Nothing else in the
suite needed adapting -- `the_kitten_falls_asleep_after_three_minutes_and_the_page_dims_after
_four` still passes because a rebuild does not touch the idle clock, and `a_rebuild_crosses_over
_instead_of_cutting` still finds both pages drawn on the same frame.

Two tests are new. `a_rebuild_is_a_display_across_the_whole_page` runs three seeds to 180 s and
then five seconds past it, sampling every frame: the peak is over 300 glows, the page is still
lit by more than 200 two seconds in (a volley, not a flash), at least six of eight slices of
the colour wheel are lit, the light spans the width (under 20 cells and over 55) and the
height (under 12 lines and over 26). `the_display_burns_out_in_a_few_seconds` measures the
cat's own traffic over the five seconds before the rebuild, waits seven past it, and asks for
no more than sixty glows over that baseline -- which is the real requirement of a display: it
has to end.

### Verification

`cargo test --offline`: 2,195 tests (1,087 in `nn-app`, 1,008 in `nn-core`, 100 in `nn-mcp`),
0 failed. `cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not run.
`cargo build --offline --release -p nn-app -p nn-mcp`: 2m22s.

`docs/nekopp-features.md` rewrote the Rebuild row and gained "The fireworks" and "The volley";
the Playground introduction now says a page goes out in fireworks every three minutes, and a
paragraph before "Two clocks" describes what a display is made of. The claim about how much of
the time she spends climbing was measured over five minutes and had to be re-measured over
three: climbing, perching and swooping are 42% to 54% of frames, so it now reads "between two
fifths and half" rather than "between a third and a half". Four other "five minutes" in the
file went to three, and one did not: a missing newline had glued the "Wanting up" row to "The
tokens" and put four cells in a two-cell row, which the PDF had been rendering as a squeezed
table. `python3 docs/build-pdf.py` then `--check`: up to date, 328,497 bytes, 77 pages A4.
The corpus line that opens the Rust page -- "a page of code, a kitten, and five minutes of
nothing to do" -- went to three as well, since it is the mode's own tagline.

## The version at 0.3.0

Twenty commits of work since the number last moved, so it moves again: 0.2.0 to 0.3.0. It is
still declared once, in `[workspace.package]`, and everything that reports a version reads it
from there -- `--version`, the About window, the single-instance bus property and the MCP
server's `serverInfo` -- so nothing else had to be found: the line changed, the workspace
rebuilt, and all four moved.

What could not follow the manifest was edited by hand, the same three places as last time: a
debian changelog entry above 0.2.0, which is also what `dpkg-parsechangelog` hands the package
build, the man page's header, and the MCP README's transcript, which carries the version the
binary prints and the counts of an actual run. The reference's title line reads the manifest
already, so `python3 docs/build-pdf.py` was all the PDF needed.

Verified: `npp --version` prints `Neko++ 0.3.0`; `npp-mcp` prints `npp-mcp 0.3.0: 110 sections,
74 shortcuts, 1 of 5 plugins enabled` and answers `initialize` with `"version":"0.3.0"`; the
`search_features` transcript in the README still matches the binary line for line, although the
reference has grown six sections and one shortcut since it was written. `cargo test --offline`:
2,195 tests, 0 failed. `cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not
run. The plugin workspace keeps its own 0.1.0 -- different target, versioned separately,
interface unchanged.

## The links point at this project

The four Help rows opened Notepad++'s pages -- its home page, its repository, its user manual and
its forum -- because until this release there was nothing of this project's to open. The snapshot
published as `Stephane-U/nekopp` changes that, so the rows now name what they open: the
repository, its issue tracker, the reference rendered from `docs/nekopp-features.md`, and the
author's LinkedIn profile for the community row.

The addresses moved into `links.rs`, a module whose whole content is four strings and three
tests, because a link is the one thing in the program a window cannot show you is wrong: it opens
a browser, or it does not. The tests ask that every page is `https` on
`github.com/Stephane-U/nekopp`, that the manual is the file the PDF is built from on `main`, and
that the community row is the About box's string rather than a second copy of it that can drift.
`Action::Forum` and its `forum` identifier did not change: the row is a link, not a concept, and
a keymap or settings file written before the move still names it.

That fixed a state the document had described in two places as deliberate: the labels said
`Neko++ ...` and the destinations were Notepad++'s, which the pass that added the menu called a
known disagreement. Both passages now say what each row opens, the row list follows, and `grep`
finds no `notepad-plus-plus.org`, `npp-user-manual.org` or `community.notepad-plus-plus.org` left
in the source. Two labels changed with the destinations -- `Neko++ Project Page` to `Neko++
Issues`, `Neko++ Community (Forum)` to `(LinkedIn)` -- so the menu does not promise a forum it
does not have.

## Eighty-eighth pass: a name that does not exist yet

`nn nekopp.token`, in a directory where the file was not there, printed
`npp: cannot open ...: No such file or directory` and opened nothing. That was
deliberate and it was wrong. The editor was built around "a file is read", and
reading a file that is not there is an error; but the one failure that is not a
failure is a name the user has not used yet. `vim notes.txt` and `nano notes.txt`
both open an empty page and create the file at the first write, and `npp
notes.txt` is the same act: naming the document you are about to write.

So `Document::open` sits beside `from_file`: it reads, and on an `Err` whose kind
is `NotFound` it builds `Document::new()`, adopts the path and stamps it. Only
absence counts. A directory, or a file with no permission, is still an error,
because replacing something that is there and cannot be shown with a blank page
offers to overwrite what the user never saw. The stamp is what makes the blank
page behave: `FileStamp::of` on an absent file is `None`, the same as on a buffer
that has been written nowhere, so the change monitor does not report the tab as a
file deleted on the frame it appears, and the buffer is not dirty until a key is
pressed.

Two call sites, and they are not the same one twice. `open_path` is every later
route -- a second file, a dialog answer, a drop, another process's request -- and
`State::new` is the file the window is born with, which is the route the command
line takes for its first argument. Both call it, which is why the check on the
binary matters: the first version of this change only reached `open_path`, and
`nn nekopp.token` still printed the old line. Session restore keeps `from_file`,
because a tab the session remembers is a file that existed last run and a stale
entry should report rather than conjure a tab.

The recent list keeps its meaning by being skipped: a buffer whose file has never
been written is not something you opened, so `push_recent` runs only when the path
already existed. An ordinary save reaches `save_active`, which pushes the path
then, so the entry appears when the file does and never before.

Verified on the running binary, not only in tests: `document.list` shows the tab
at the absent path with `modified: false` while the directory stays empty to `ls`,
`document.set` then `document.save` writes 6 bytes and creates the file, and no
`cannot open` line reaches the log. An existing file still opens with its 11
characters, and a directory handed to the same route still prints exactly one
`cannot open`. `cargo test --offline`: 2,201 tests, 0 failed. `cargo clippy
--offline --all-targets`: 0 warnings. `cargo fmt` was not run.
`docs/nekopp-features.md` gained the paragraph after `--wait`, and `python3
docs/build-pdf.py` then `--check`: up to date, 329,787 bytes, 77 pages A4.

## Eighty-ninth pass: the same calls, addressed by URL

`npp --call` speaks JSON over the session bus and `npp-mcp` speaks JSON-RPC over a
pipe, and a publishing script, a notebook or a language whose HTTP client is
already on the shelf has neither. So the call table got a third transport:
`--rest=8085`
serves it on the loopback interface, and `PUT /document` with the new text is the
same act as `document.set` over the bus.

The design that made it small is that a route is a *spelling*, not a second API.
Each row builds the JSON object a `--call` user would have typed and hands it to
`api::parse`, so the rules -- a range in either order, `whole_document` and `range`
are alternatives, an empty `path` names nothing -- are stated once, in a place that
already had tests. The REST face therefore cannot answer a request the other two
would refuse, and the shared parser's own words reach the client: a `POST
/command/sort_asc?whole_document=1&range=1-2` answers "send `whole_document` or
`range`, not both: they choose the same block" with a 400, because that sentence
was written for the other transport and did not need rewriting.

What HTTP adds is the status line and the safety rails. 200 for an answer, 400 for
a malformed request, 404 for a path or a document that is not there, 405 with an
`Allow` header for the wrong verb, 503 when the window did not answer in 5 s. The
body is always the `error` object `--call` answers with, so a client parses one
thing. The listener binds `127.0.0.1` and has no way to bind anything else: this
API replaces the text in a window and saves it over a file, and a desktop editor
is not something to expose on a network without being asked. A request carrying
`Origin` is refused with 403 -- no shell tool sends one, every browser does, and
that single rule is the whole cross-site story. `NPP_REST_TOKEN` adds a bearer
check for a shared machine, and it is an environment variable rather than a
setting because a settings file is world-readable and a token in one would be a
secret written down in the wrong place.

Threading decided the shape of the module. The editor state lives on the event
loop, so the listener is a thread that turns a request into an `api::Call`, posts
it to the loop with `EventLoopProxy`, and waits on a channel for the answer: the
HTTP side knows nothing about a window, and `rest` compiles and tests without one.
Twenty-nine tests, seven of which open a real socket against a canned dispatch,
which is what proves the header parsing, the body split across two writes, the
token and the `Origin` refusal without a display in sight.

Two numbers moved because the feature is documented where the other two transports
are: the reference gained a subsection, so `npp-mcp` reports 111 sections, 29
chapters and 82 subsections, and the three tests that had those counts written into
them were updated with it. `settings.txt` gained `rest` (off) and `rest_port`
(8085), which is how a launch starts the listener without a flag; the switch keeps
its port attached rather than taking the next word, because `npp --rest 8085` is a
listener and a file called `8085`, and guessing would open the wrong thing.

Verified against the running binary with `curl`, not only in tests: `GET /`,
`/status`, `/documents`; `PUT /document` with `c\n a\n b\n`; `POST
/command/sort_asc?whole_document=1`; `GET /document?range=0-4`; `POST
/document/save`, which wrote the sorted text to the file; 404 for `/nope` and for
an unopened path; 405 with `allow: GET, PUT`; 403 for an `Origin`; 401 with
`NPP_REST_TOKEN` set and 200 with the bearer; 400 for `?range=abc`, for a body
that is not UTF-8 and for `whole_document` with `range`. `cargo test --offline`:
2,235 tests, 0 failed. `cargo clippy --offline --all-targets`: 0 warnings.
`cargo fmt` was not run. `python3 docs/build-pdf.py` then `--check`: up to date,
342,134 bytes, 78 pages A4.

## Ninetieth pass: a file that says how the API opens

The REST face was switched on by a flag or by `settings.txt`, and both are awkward
for the machine that most wants it. A harness launches the editor a hundred times a
day and cannot put `--rest` on every launch of a couple of hundred paths; a monitor
wants the API read-only, and that is not a preference the user should have to go
and edit. So the listener now reads its own file: `api.yaml`, or `api.json`, in the
configuration directory.

Two formats, because two readers were already here and a third was not.
`serde_json` is in the tree for the session bus and the MCP pipe, so JSON costs
nothing; no YAML crate is available offline, so the YAML reader is a hundred and
fifty lines that accept the subset a configuration needs -- `key: value`, one level
of nesting, `#` comments outside quotes, quoted or bare scalars -- and refuse the
rest with the line number: a tab, a list, a second nesting level, a key with no
value. That is the honest trade. A configuration file is not a document, and a
reader that refuses what it does not understand is better than a dependency that is
not there.

The two readers meet in the middle. Both flatten to one map of dotted keys --
`rest.port`, `port` -- and one `Config` is built from it, so `rest: {port: 8085}`
and a bare `port: 8085` differ only in spelling, and the nested spelling wins when
a file carries both. The flattening is the line where the format stops mattering,
which is why the twenty tests are mostly about keys and only a few about YAML.

Precedence is by specificity. `--rest=PORT` beats the file, the file beats
`settings.txt`, and `NPP_REST_TOKEN` beats the file's `token`, because a secret
belongs in the environment of a launch rather than on disk. `read_only` comes from
the file alone: it is not a thing to say once on a command line, it is the standing
description of what that listener is for.

A file that exists and does not load is fatal, `eprintln!` and `exit(1)`, and that
is the pass's one real decision. Everywhere else in the program a bad input
degrades and the editor carries on. Here a `port` typo that silently fell back to
8085 would open a listener where nobody looked, and a `token` typo that silently
fell back to no token would open one that authenticates nobody. Configuration is
the one place where guessing is a vulnerability, so a typo stops the launch and
says `~/.config/npp/api.yaml: rest.port takes a port between 1 and 65535, not
"not-a-port"`. A message that wraps is still one message.

A token in a file whose mode bits reach past the owner is still served, and the
startup line adds a `chmod 600` suggestion. Refusing would break the setup of
somebody who knows what the file's permissions are; silence would let a secret sit
in a file other accounts can read. One sentence, and only when it is true.

`read_only: true` is enforced after routing rather than inside a hundred handlers.
`rest::mutating(&Call)` names the three calls that change something --
`document.set`, `document.save`, `command.run` -- and `handle` answers them `403`
with the code `read_only` before the dispatch, which leaves `route` pure and puts
the policy in one place a reviewer reads in a glance.

Verified against the running binary with `curl`, not only in tests: a file and no
flag started a listener on 8090, 401 without the bearer and 200 with it; mode 644
produced the warning and `chmod 600` silenced it; `read_only: true` answered 403
`read_only` to a `PUT` and 200 to a `GET`; `--rest=8091` overrode the file's port;
an `api.json` alone worked and named itself; both files present served 8093 and
said `api.json is there and ignored; api.yaml is the file in use`; a broken file
exited 1 with the message above. `cargo test --offline`: 2,257 tests, 0 failed.
`cargo clippy --offline --all-targets`: 0 warnings. `cargo fmt` was not run.
`python3 docs/build-pdf.py` then `--check`: up to date, 349,655 bytes, 79 pages A4.

## Ninety-first pass: the notice on every door

The licence was already the first of the Free Software Foundation's three steps
and only that one. `LICENSE` holds the unmodified GPLv3 -- 674 lines, 35,149
bytes, identical to the copy Debian ships in `/usr/share/common-licenses/GPL-3`
-- and `Cargo.toml` says `GPL-3.0-only`. Missing were the other two: no source
file carried a header, and nothing in the program said what the terms were.

A hundred and twenty Rust files and seven scripts gained the copyright line and
the permission notice, thirteen lines each, placed above the module doc rather
than after it. That is legal -- a `//` comment is trivia, and `//!` may follow it
-- and it is what the Foundation asks for, which is the whole reason. A copied
file should carry its own terms; an SPDX tag names a licence without stating it,
and a file that travels out of the tree with `GPL-3.0-only` on it tells a reader
nothing about the warranty. The wording says "version 3 of the License", not "or
later", because that is what `Cargo.toml`, `debian/copyright` and the reference
all say, and a licence that drifts from the file that declares it is worse than
either choice. The wrap is 79 columns: narrow enough to read as a header and not
as code, wide enough that the three yes/no questions of the notice fit.

Not every file. XML and JSON fixtures are test data; the PNGs, the icon and the
font are assets, and the font is already accounted for under the SIL Open Font
License. The markdown is prose that `LICENSE` covers, and a licence block at the
top of a document is noise in the rendered page. The four shell scripts, the
Python builder and `debian/rules` keep their shebang on line one -- a header
above `#!/bin/sh` is a script that does not run -- and get the notice underneath,
which is the one shape the sweep has to know about.

The third step is the interactive one, and a window has no terminal to print in.
So the About box now carries the copyright, the licence by name and version, the
sentence that there is no warranty, and the line that points at the terms; the
`--help` text ends with the same four sentences, because a user who only ever
runs the command line never opens a window; and `npp-mcp`, which has no banner of
its own, prints the notice on stderr beside the status line its client ignores.
The Help menu gained `Neko++ License (GPLv3)` between two rules -- the GNU
project's page for the licence, next to the four pages that are about this
project but not among them -- and `Action::License` routes it through the same
`open_page` the other links use, so there is no second way to open a browser.

That row found the test that was already there for it. The menu labels are
translated in twenty-three bundled languages, and the one that says every
bundled language translates every menu label failed the moment the row existed: a
menu bar that says "Fichier" and "Neko++ License" in the same dropdown is the bug
that test exists to catch. The fix is twenty-three rows in the localisation files, in the
alphabetical place `Neko++ Issues` and `Neko++ Online User Manual` leave for it,
and the test proves them rather than a reader checking one file of each script.

The packaging follows the same three steps. `debian/copyright` names the holder
rather than "the Neko++ authors"; `debian/rules` installs `LICENSE` into
`/usr/share/doc/npp/` where the copyright file already points; the AppImage puts
it in `usr/share/licenses/npp/`, which is where an appimage reader looks; and the
manpage gained a LICENCE section and an AUTHOR who is a person with an address
instead of "The Neko++ authors".

Verified end to end, not only in tests. This machine has no pointer injection, so
the row was clicked through the accessibility tree instead: a `python3` client on
the AT-SPI bus found the Help menu, read its eight rows in order, ran the row's
`click` action, and a stub `xdg-open` first on `PATH` recorded
`https://www.gnu.org/licenses/gpl-3.0.html`. The same route opened
`About Neko++`, whose screenshot shows the copyright, the licence by name and
version, the sentence with no warranty in it, the line that points at the terms,
and the LinkedIn address in the theme's link colour. `npp --help` ends with the
notice and `npp-mcp` prints it on stderr beside its status line. `cargo test
--offline`: 2,260 tests, 0 failed -- three of them new, for the About text, the
licence URL and the row between its two rules. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run. `python3
docs/build-pdf.py` then `--check`: up to date, 351,974 bytes, 79 pages A4.

## Ninety-second pass: the line under the top edge

The report was one sentence: resizing the screen changes the line we were at.
It reproduced first try. Twenty-five wheel clicks into a five hundred line file
put line 130 under the top edge while the caret stayed on line one -- the wheel
scrolls the pane and does not move the caret -- and one resize brought the pane
back to line one.

`WindowEvent::Resized` reaches `State::resize`, which calls `resize_view`, and
`resize_view` ended with a call to `ensure_caret_visible` on both panes. That
line was written when the wheel moved the caret: the caret was then the only
thing that knew where the reader was, so putting it back on screen after a
resize was right. Once the wheel scrolls by pixels and leaves the caret where
it is, the same line becomes a reset button. An off-screen caret is the ordinary
state of a reader a hundred lines into a file, and chasing it throws away the
place that reader had scrolled to.

So the view has one anchor now and it is the top row. `EditorView` remembers
`last_rows`, the room the previous frame had, which is what lets the next frame
tell a window that shrank from one that did not. The caret is pulled back in
exactly two cases: it was on screen in the previous frame and the window now has
room for fewer rows -- a shrink, a zoom step, a dock that took a slice of the
pane -- or wrapping was reflowed underneath it. Growing is never followed by a
scroll, and a caret that was already off screen is never chased, because being
off screen is not a mistake the view can correct without losing a place the
reader chose.

The policy also moved out of `resize_view` and into `prepare`. `text_height` and
the origin are set by the renderer a few lines before it calls `prepare`; when
the resize event arrives they still describe the old window, so any row count
measured there was the previous frame's arithmetic about the previous frame's
geometry. `resize` keeps the job it is good at, sizing the text column, and the
draw pass -- the only place that knows how many rows fit -- decides what to do
about the caret.

The second cause was hiding behind the first. `first_line` is a row number and
`prepare` set the wrap width from the window every frame, so narrowing the
window renumbered every row in the document, and row 130 of the new numbering
was a different line of text. `sync_wrap` now remaps through the text rather
than the arithmetic: it reads the line under the top row, sets the wrap width,
and asks for that line's first row in the new numbering. It answers whether it
reflowed, which is the second of the two cases above.

Asking whether the caret is on screen needed a way to ask without moving the
document, which `ensure_caret_visible` cannot be -- it reveals a collapsed block
before it answers. `Editor::caret_row` returns the row the caret would be drawn
at, and `None` inside a fold; `ensure_caret_visible` became its first caller
rather than its own duplicate of the arithmetic, and recomputes the column
locally for the horizontal scroll because that is the only place that needs it.

Five tests in a new module at the end of `view.rs` pin the behaviour: a view
scrolled away from its caret is left alone by a grow and by a shrink, a window
that shrinks keeps the caret it was showing, a window that grows does not scroll
to a caret already in view, and both directions of a wrap change keep the line
under the top edge. Putting the old behaviour back fails three of the five,
which is the only evidence worth having that they test the bug and not the fix.

Then the window, through XTEST, because this machine has no `xdotool`: a five
hundred line file, the pointer over the text, twenty-five wheel clicks to line
130, `XResizeWindow` to 900x520 and then to 1100x800. The gutter said 130 before
both and 130 after both. Word wrap is off by default, so the reflow path was
exercised the other way: `Ctrl+Alt+W` turned it on -- and scrolled to the caret,
correctly, since an explicit command should -- thirty clicks more put line 3 at
the top, and resizing to 700x600 and then to 700x380 left line 3 there while
each line's row count changed underneath it.

`cargo test --offline`: 2,265 tests, 0 failed. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run. The feature reference
gained three sentences in "Editor pane" that state the rule as a promise to the
user, and `python3 docs/build-pdf.py` then `--check` reports it up to date:
352,217 bytes, 79 pages A4.

## Ninety-third pass: a map that survives a long line

A file of training data -- 4.1 MB of JSONL, 1,826 records, the longest line
33,602 characters -- opened with an empty minimap column. Not a faint one:
background from the top of the pane to the bottom, and the column's background is
the editor's own colour darkened by three percent, so the strip read as missing.
The only things still visible were the one-pixel border on its left and the
viewport box, which is how a column can be dragged to scroll a document it no
longer shows.

The size was not the cause. `LARGE_FILE_CHARS` is sixteen mebibytes, and four
megabytes is not a large file by that measure. The longest line was. `build`
scaled the picture against it -- 78 pixels of drawing room over 33,602
characters, 0.0023 of a pixel per character -- and `push_run` drops a run
narrower than half a pixel, which was 216 characters on that file. A JSON record
is a hundred short tokens, every one of them under the floor and every one of
them discarded, so the picture was not coarse, it was not there.

The second rule was hiding behind the first. `MAX_RUNS` was six, and the count
was taken before the width was measured: `runs += 1` sat beside the call rather
than around it, so six discarded slivers closed the band. On a JSONL line the
sixth style change lands inside `"messages"`, before the first `content` string,
which is the only part of a record with any length. Taking the two long lines out
of a copy did not bring the picture back either, so the budget alone was enough
to lose it -- the two rules each had to be wrong for the column to be blank.

There was a cost in the same place. The walk ran to the end of the line, so a
band could pull thirty thousand characters out of the rope to find that nothing
past the last pixel can be drawn; on an unhighlighted file, where a whole line is
one run, every band did exactly that. The ceiling that fixes the picture fixes
this too: nothing past the two hundredth character is read at all.

So the scale has an upper bound now, `MAX_COLUMNS`, at two hundred characters --
0.39 of a pixel each, above the floor `push_run` keeps. A line longer than that
is cut at the right edge instead of squeezed, which is what a thumbnail of a long
line has to be, and `push_run` clips the run to the strip, because without the
clip the same change would paint a rectangle wider than the column over the text
beside it. `runs` counts the runs that were drawn, and the budget is twenty-four
rather than six, since a JSON record reaches its text after about twenty tokens.
Source files come out a little denser for the larger budget, which is the
faithful direction: the picture follows the text more closely than it did.

Four tests in `minimap.rs` know the difference. A record two thousand characters
long still reaches across the strip; forty sub-pixel tokens do not spend the run
budget; a stray minified line does not blank the ordinary lines beside it; and no
run reaches past the column's edge. The first three fail when the old rules go
back in -- `MAX_COLUMNS` at a billion, `MAX_RUNS` at six counted unconditionally,
the walk unbounded -- and the fourth passes either way, because one unbroken run
was always the case that fit. It is the guard against the clip overshooting, not
evidence of the bug, and it is worth saying so.

`cargo test --offline`: 2,269 tests, 0 failed. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run.

Then the window, on `:1`, with a private `XDG_CONFIG_HOME` and `--software`, the
same file before and after the change: mean ink across the column 4.2 pixels of
78 over 5.4 percent of rows -- the viewport box and nothing else -- before, and
74.2 pixels over every row after. The same text hard-wrapped at a hundred and
twenty columns reads 74.2 before and 77.3 after, so the control that always
worked still works and gained the denser budget. `crates/nn-app/src/main.rs`
still reads as a map: green comments, grey code, blue keywords, the ragged right
edge of short lines.

`target/release/bench` on the same file at 1920x1080, three hundred frames a
scenario, before and after: idle 1.78 and 1.79 ms, scroll 15.94 and 16.88 ms,
type 5.90 and 6.70 ms at p50. The denser picture costs about eight tenths of a
millisecond on a keystroke, which leaves it inside the eight milliseconds the
plan promises; `crates/nn-app/src/main.rs` goes from 2.30 to 2.50 ms typing. The
record file's own scrolling costs fifteen milliseconds either way, so that is the
document and not the map -- two thousand characters a line do not come free.

The feature reference gained a paragraph in "Document Map" that states the
ceiling as a promise to the user, and `python3 docs/build-pdf.py` then `--check`
reports it up to date: 352,523 bytes, 79 pages A4.

## Ninety-fourth pass: shaping only what is drawn

Dragging the Document Map through the record file from the last pass -- 4.1 MB of
JSONL, longest line 33,602 characters -- moved the view about once a second. The
map's own trouble was fixed and the pixel budget is now bounded, so the strip was
not the suspect, and the measurements said so plainly. A drag of forty lines a
step, forty frames, `target/release/bench` at 1920x1080, p50 per step: 1.9 ms for
a megabyte of eighty-column source, 1.9 ms for four megabytes of the same, 32 ms
for a file of two-thousand-character lines, 1,024 ms for the JSONL. File size did
nothing, the map did nothing; the length of the longest line did all of it.

It was `LineCache::ensure`. With wrapping off a line is one row, and the row was
shaped whole -- `doc.slice(start..end)` over the entire line, handed to cosmic-text
-- so every wide record in the pane was shaped end to end, thirty-three
thousand characters a frame, to draw the hundred columns the pane can hold.
The cache made it worse rather than better: the entry was keyed by line and
segment, so it was correctly reused, but the first frame that had to revalidate
paid for the whole line again.

So a row is now shaped as a window of columns, and the window is a function of the
horizontal scroll for the whole frame. `line_window` starts at the column under
the left edge aligned down to a multiple of sixty-four and is that much wider than
the pane, which makes two things true at once: the last column in view is always
inside the window, and a scroll of fewer than sixty-four columns reuses the window
it already had, so a drag of a few characters does not reshape every row. A
`CachedLine` remembers the column its shaping began at, `text_areas` draws the
buffer that far into the line, and the cache is only reused when that column is
the column being drawn, because a shaped window of the wrong part of a line is a
picture of the wrong text. Nothing about the key changed, which matters: a window
in the key would have grown the cache by one entry per scroll position.

The marks have the same bound. `push_marks` walks a line character by character to
place whitespace dots, boxes and indent guides, which was another full pass over a
thirty-thousand-character record per frame; the walk now stops at the end of the
window and starts at its first column, with `seen_text` set when it starts late so
the leading indentation of a line scrolled out of view is not counted. The
end-of-line mark is skipped when the walk stopped short, because a mark placed one
column past the last character belongs off screen when the character does.

A window that starts at column 640 cannot always be drawn where its column says.
A tab is not one column wide and neither is a wide character, so the shaped text
is laid out from its own first character, and the tab stops after the cut would be
counted from the wrong place. The first version had this wrong and looked right:
a tab-indented file of hundred-and-twenty-column lines, rendered at three scrolls
and compared pixel for pixel against the build before the change, matched at the
left edge and differed by 79,168 pixels at a scroll of 700 -- the text was there,
in the wrong place. So the front of a window is cut only when what is being cut is
plain single-width text with no tab in it; anything else is shaped from the line's
start and cut at the window's right edge alone. That keeps the cost bounded by
what is on screen either way -- a line scrolled to column 3,000 shapes 3,164
characters, not 33,000 -- and the pixels are now identical at three scrolls on the
tabbed file and at two on the record file, 0 differing pixels in all five.

Five tests in `view.rs`. The window is checked to be aligned and to cover the
columns on screen at two hundred different scrolls, and to stay put across a drag
shorter than its margin; `window_bounds` is checked to clamp at the end of a short
line, to cut a long one, and to refuse to cut the front of a line with a tab or a
wide character in it; and one test builds a cache around a line of thirty thousand
characters and asserts that a window at column 640 leaves a hundred and sixty-four
characters shaped. That last one fails on the old code with thirty thousand
characters and a window of zero, which is the difference being claimed.

`cargo test --offline`: 2,274 tests, 0 failed. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run.

Then the same four files, the same forty-frame drag, after the change, p50 per
step: 1.76 ms, 1.75 ms, 4.82 ms, 5.01 ms. The record file goes from 1,024 ms to
5.01 ms, two hundred times, and it now costs about what two-thousand-character
lines cost -- which is what a bound on shaping should look like, since both are
paying for the screen rather than for the line. The eighty-column files are
unchanged within noise, as they must be: their lines were already shorter than the
window. `idle` and `type` on the JSONL read 1.09 and 5.80 ms, and a keystroke
inside a thirty-thousand-character record is 5.80 ms rather than a second, because
the row being reshaped is the row in the window. The tabbed file scrolls in
2.39 ms, the smallest gain of the set, since it is the one that keeps its left
edge.

The temporary patch to `bench.rs`, which made the scroll scenario jump forty lines
and let a dump set `scroll_x`, was reverted; `git diff` on that file is empty. The
feature reference gained a paragraph under "Rendering" that states the bound as a
promise, including the tab exception, and `python3 docs/build-pdf.py` then `--check`
reports it up to date: 353,459 bytes, 79 pages A4.

## Ninety-fifth pass: a frame when the window comes back

Reported as: copy a selection with `Ctrl+C`, `Alt+Tab` to another application, come
back, and the text is not there. Not normal, and not a copy problem -- the
clipboard is fine, `Action::Copy` only reads the document and writes to arboard,
and nothing in that path touches the view.

What it was: the window is drawn on demand. `render` runs on `RedrawRequested`,
and a redraw is asked for by the events that change the picture -- a resize, a
key, a click, the wheel, a dropped file -- and by the surface recovering from
`Outdated`, `Timeout` or `Lost`. `WindowEvent::Focused` was not handled at all, and
neither was `Occluded`, so a window coming back to the front drew nothing and kept
whatever frame the compositor still held. Under a compositing desktop that frame
can be an older one, from before the selection was made or before the text was
shaped, which is exactly the report: the pane shows a picture of a state the
editor is no longer in.

So a focus gain and a return from occlusion now ask for one frame each:

```rust
WindowEvent::Focused(true) | WindowEvent::Occluded(false) => {
    state.window.request_redraw();
}
```

`Focused(false)` is ignored on purpose -- a window losing the focus is not worth
drawing, and drawing it would put work on the way out of every `Alt+Tab`.

Checked on an isolated Xvfb display, never on the desktop: a 500 KB source file,
`--software`, a click, `Ctrl+A`, `Ctrl+C`, the focus sent to the root window and
back. Before the change the sequence produced no frame at all after the copy --
the picture could not change, whatever had happened to it in the meantime. With a
temporary frame counter in `render`, the focus gain prints one `refocus` and the
frames that follow; the shot taken after the round trip is pixel-identical to the
one taken before it (`compare -metric AE` reports 0), so the redraw draws the
selection the editor still holds rather than an empty pane.

`cargo test --offline`: 2,274 tests, 0 failed. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run. The feature reference gained
three lines under "Rendering" that say the window is drawn on demand and draws one
frame on the way back; `python3 docs/build-pdf.py` then `--check` reports it up to
date: 353,641 bytes, 79 pages A4.
## Ninety-sixth pass: the key that made the switch

Reported as: select text, `Ctrl+C`, `Alt+Tab` away, paste it somewhere else,
`Alt+Tab` back -- and the text is gone from the file.

The ninety-fifth pass had given the window one frame on the way back and the
report came again, so this pass went after it with a record instead of a theory.
Six gestures were run first, on the live desktop, with the user's own settings
file, their 4,1 MB JSON file and a window maximized through the window manager:
drag-select, double-click word, `Shift+Down` from the top, `Ctrl+A`, the same
with the pointer warped out of the window first, and the whole round trip twice,
with `Ctrl+C` in every copying variant. All six came back with 0 differing
pixels. Covering the window with another client for two seconds likewise
produced no frame at all while it was covered, and the text came back correct
without any input once it was uncovered. Nothing in a fresh window reproduced
it, which is why the record was needed: the report only happens in an instance
that has been alive for a while, and only its owner can make it happen.

So `select_log` and `evt` wrote a line to `selection.log` in the configuration
directory: every change to the selection with the entry point that made it, and
every frame, present, focus change, key and button. The file was capped at four
megabytes.

What it showed, in eight lines:

```text
1795530774905 key Character("c")        copy, selection 23..33, 89 characters
1795530775754 focus off                 Alt+Tab away
1795530780180 focus on                  back
1795530780182 key Named(Tab)            the Tab that made the switch
              key len=83 sel=27..27     89 -> 83, caret 23 -> 27
1795530795243 key len=87 sel=31..31     83 -> 87, caret 27 -> 31
1795530831224 key len=91 sel=35..35     87 -> 91, caret 31 -> 35
```

A `Tab` press reached `handle_key` two milliseconds after `Focused(true)`, and
the document was edited by it. The arithmetic says the rest: ten selected
characters replaced by four, then four more on each later round trip, which is
`Key::Named(NamedKey::Tab)`'s own arm inserting `"    "` -- and replacing the
selection, because an insertion replaces what is selected. The user's text was
not deleted by a stale frame or a lost selection. It was typed over.

Why the key arrives at all: `Alt+Tab` is a grab the window manager owns, so the
client never sees the `Tab` -- until the switch commits, the grab is released,
and a `Tab` press still in the queue is delivered to the window that just got the
focus. The key physically predates the switch. From the loop's point of view it
arrives in the same instant.

The fix is to say so:

```rust
if self.switched_in() {
    return;
}
```

`switched_in` is a `Focused(true)` less than `LEAKED_KEY_GRACE` ago, sixty
milliseconds, armed by a focus gain and only by a focus gain -- an occlusion
report says nothing about the keyboard, and a window being uncovered while
someone types must not lose a keystroke. Sixty is enough because the leaked key
arrives in single-digit milliseconds, and the first key of someone who switched
windows on purpose comes hundreds of milliseconds later, after their hand has
moved.

Checked on the live desktop against the built binary: the focus is taken away and
given back, and a `Tab` is pressed in the same breath, from one process, so the
gap is zero. The log then read `key Named(Tab)` / `leaked key ignored` and the
next frame still had `sel=0..89`, the whole selection the test made; a `Tab`
pressed a second later reached the document and inserted its four spaces
(`sel=94..94`), so the guard stops the switch and not the typing. Repeated on the
user's own desktop, the gesture that had been losing their text four times an
hour stopped doing it.

With that confirmed, the diagnostic came out: `select_log`, `evt`,
`select_trace` and the `trace` helper the ninety-fifth pass had left behind are
gone, `selection.log` is not written by anything, and the same check was run
again on the clean build -- the shot taken after the leaked `Tab` is
pixel-identical to the one taken before it (`compare -metric AE` reports 0), the
next `Tab` changes 206,549 pixels, and no log file appears.

The same pass closed one unrelated hole. `render` retried `Outdated`, `Timeout`
and `Lost` -- each arm carrying the note that skipping a frame forever is
indistinguishable from a frozen window -- but `Occluded` returned without a
retry, on the assumption that winit sends a redraw when the window comes back.
The surface does not have to agree: the answer lags the window on a compositing
desktop, so the frame asked for on the way in can come back occluded as well and
nothing asks again. That arm now asks 120 ms later, at most five times in a row,
and any frame that reaches the screen clears the promise. `repaint_at` is a
fifth timer in `next_wake`, so the loop wakes for it instead of sleeping through
the deadline. No run of mine produced an occluded acquire, so this is a hole
closed and not a diagnosis; it is kept because the retry rule the other arms
follow is the right one.

`cargo test --offline`: 2,274 tests, 0 failed. `cargo clippy --offline
--all-targets`: 0 warnings. `cargo fmt` was not run. No feature reference change:
a keystroke swallowed by the window switch is not a feature, so
`docs/build-pdf.py` was not run.

## The version at 0.3.1

A patch number, and the first release since 0.3.0: five commits, three of them
one bug. The version is still declared once, in `[workspace.package]`, and
`npp --version`, the About window, the single-instance bus property and the MCP
server's `serverInfo` all read it from there, so changing the line and rebuilding
moved all four.

By hand, as before: a debian changelog entry above 0.3.0, which is what
`dpkg-parsechangelog -S Version` hands the package build; the man page's header;
and the MCP README's transcript, which carries what the binary prints. The
reference's title line reads the manifest, so `python3 docs/build-pdf.py` was all
the PDF needed -- 353,643 bytes, and `--check` reports it up to date.

Verified: `npp --version` prints `Neko++ 0.3.1`; `npp-mcp` prints `npp-mcp 0.3.1:
111 sections, 74 shortcuts, 1 of 5 plugins enabled` and answers `initialize` with
`"version":"0.3.1"`; the section and shortcut counts did not move, because no
feature was added. `cargo test --offline`: 2,274 tests, 0 failed. `cargo clippy
--offline --all-targets`: 0 warnings. `cargo fmt` was not run. The plugin
workspace keeps its own 0.1.0: different target, versioned separately, interface
unchanged.

## Ninety-seventh pass: the apostrophe that was not a quote

Reported as: syntax colouring is sometimes wrong, "maybe due to comment signs".

It was not the comments. `'` was in `quotes` for eighty-three of the hundred and
forty-seven languages, and `ConfigLexer` read that as "this character opens a
string that may end on a later line". Every use of the apostrophe that is not a
string was therefore the start of one, and the state carried into every line
below it until the next apostrophe anywhere in the file. Rust's
`fn f<'a>(x: &'a str) -> &'a str { x }` turned the rest of the line and every line
under it green; so did YAML's `name: don't panic`, Haskell's `x'` and Julia's
`a'`.

The built-in C lexer had the right rule all along -- an apostrophe has to close
within a few characters or it is not a literal -- but the rule lived in the lexer
rather than in the table, so it only ever applied to the `builtin` language.

### A fifth field

`LangSpec` grew `apostrophe`, and `quotes` now means what its name says: the
characters that can *open* a literal. Six languages stopped listing `'` at all --
Haskell's prime, Julia's adjoint, and Makefile, Dockerfile, NSIS and AutoIt, which
do not quote with it. The other seventy-seven say how a literal opened by one
ends:

| `Apostrophe` | Rule | Languages |
| --- | --- | --- |
| `Char` | one character or one escape, closing within a dozen characters at the outside | 19: Rust, C, C++, C#, Java, Kotlin, Go, Zig, Nim, D, Crystal, Vala, Elm, F#, PureScript, Objective-C, CUDA, HLSL, Assembly |
| `Line` | closes on the line it opened on, or it is punctuation | 42: YAML, TOML, JSON5, JavaScript, TypeScript, Dart, Groovy, Lua, Matlab, APL and the rest |
| `Multiline` | may carry into the next line | 16: Shell, Fish, Nushell, PowerShell, Perl, Ruby, PHP, R, Smalltalk, Elixir, Raku, SQL, MySQL, PostgreSQL, Transact-SQL |

`find_char_literal_end` was tightened at the same time and the built-in lexer
inherited it: it used to accept a closing quote up to five characters away, which
paired the two apostrophes in `<'a, 'b`. It now takes one character, or an escape
scanned to the outside of a `'\u{1F600}'`.

Python is `Line` on purpose. Its multi-line string is a delimiter of three, and a
line-at-a-time lexer cannot count three: docstring bodies are coloured as code
rather than everything after the closing `'''` being coloured as a string. That
is the same trade the header already records for Lua's long brackets.

### Verified

The repository's own Rust, 120 files and 102,589 lines, run through `Highlighter`
with the Rust spec: characters painted as a string fell from 796,733 to 352,381,
and lines painted entirely as a string from 15,898 to 2,090. The 2,090 that
remain are real multi-line literals -- the `"...\` continuations in `bench.rs` and
the help texts in `api_config.rs`.

Nine tests, among them the table-wide one: for all hundred and forty-seven
languages, a line holding one apostrophe either opens a string the language
allows or opens nothing at all.

`cargo test --offline --release`: 2,283 tests, 0 failed. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. No feature
reference change: the colour of a language is not something the PDF documents.

## Ninety-eighth pass: the name after `fn`

Reported as: some Rust functions are not coloured as functions, pointing at
`plugins/xml/src/lib.rs`; "maybe it is a wider issue and it goes beyond Rust".

Three defects, all in the shared lexer, all older than the last pass:

* **A definition is not a call.** The rule was "an identifier immediately
  followed by `(`", and a definition puts its type parameters in between:
  `fn markup<'a>(text: &'a str)` left `markup` plain text. Four functions in that
  one file, every generic method in the repository.
* **A turbofish is not a call either.** `names.collect::<Vec<_>>()` puts `::`
  between the name and the arguments, so `collect` was plain text: eighty-one
  sites in this repository alone.
* **A number took the rest of the line.** The literal scan accepted letters and
  dots, so `&s[4..end.len()]` was one number from `4` to `len`: the range read as
  a number and `len` lost its colour as a method.

The first two are one question -- what follows a name when it is being used as a
function -- and the answer is now three shapes: `(`, a list of type arguments and
then `(`, and `::` before that list. The list is checked rather than assumed,
because `<` is a comparison in every language here: at most sixty characters, and
none of `&&`, `=`, `;`, `{`, `|`, `"`. `if (a < b && c > (d))` is still a
comparison.

A declaration is the one place a name is a function without being called, so the
word before it is checked against the words that declare: `fn`, `def`, `function`,
`func`, `fun`, `sub`, `subroutine`, `procedure`, `proc` and the Lisp and Elixir
spellings. The list names no language, because those words are the same word in a
dozen of them, and it is compared without case so that Pascal's `Procedure` and
VB's `Sub` are read.

The number scan was replaced: digits and underscores, a fraction only where a
digit follows the dot, an exponent, then one suffix run for `0x1f`, `0b1010`,
`1u8` and `1.5f32`. `4..end` is a number, two dots, and a name.

### Verified

A probe over the repository's own sources -- 120 files, 43,345 identifiers that
are followed by one of the three call shapes -- finds 0 that are not painted as
functions, and 0 identifiers painted as a function that are not followed by one
of them, so the new rules add no colour that the text does not ask for. The same
probe against the previous lexer reports 203 misses, which is a floor rather than
a count: lines the apostrophe bug had painted as strings were not examined.
`plugins/xml/src/lib.rs` alone: 5 misses, then 0.

Five tests, and one existing one changed rather than deleted: `fn main` now
expects `main` painted as a function, because that is what it is.

`cargo test --offline --release`: 2,288 tests, 0 failed. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. No feature
reference change.

## The version at 0.3.2

A patch number, the day after 0.3.1: two commits, one of them the colouring pass
that was reported twice, the other the line clippy asked for. The version is
still declared once, in `[workspace.package]`, and `npp --version`, the About
window, the single-instance bus property and the MCP server's `serverInfo` all
read it from there, so changing the line and rebuilding moved all four.

By hand, as before: a debian changelog entry above 0.3.1, which is what
`dpkg-parsechangelog -S Version` hands the package build; the man page's header;
and the MCP README's transcript, which carries what the binary prints. The
reference's title line reads the manifest, so `python3 docs/build-pdf.py` was all
the PDF needed -- 353,641 bytes, and `--check` reports it up to date.

Both packages in `dist/` were rebuilt: `npp_0.3.2_amd64.deb`, whose control file
says 0.3.2, and `npp-x86_64.AppImage`, which holds the same binary as
`target/release/npp` -- the two hash alike. This is the release that carries the
apostrophe and the declared-name colouring, so the AppImage is the first build a
reader who installs rather than compiles will see them in.

The release page carries five assets, where 0.3.1 carried four: the AppImage, the
deb, the docs zip, and the source as both a tarball and a zip. The deb is the new
one and stays -- a Debian user with a package on offer should not have to build
the package to install it.

Verified: `npp --version` prints `Neko++ 0.3.2`; `npp-mcp` prints `npp-mcp 0.3.2:
111 sections, 74 shortcuts, 1 of 5 plugins enabled` and answers `initialize` with
`"version":"0.3.2"`; `dpkg-parsechangelog -S Version` hands the package build
0.3.2; `docs/build-pdf.py --check` reports the PDF up to date at 353,641 bytes.
`cargo test --offline --release`: 2,288 tests, 0 failed. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. The plugin
workspace keeps its own 0.1.0: different target, versioned separately, interface
unchanged.

## Ninety-ninth pass: the copies an open did not need

Asked whether the memory could come down without touching performance. Of the
four and a half bytes a byte of text cost, three bytes were not the text.

Measured on a 131 MiB file of source-like lines -- 137,363,424 characters and
1,560,949 lines -- against an empty window that costs 163 MB on this machine: the
editor peaked at 747 MB and settled at 367 MB. The 380 MB between those two
numbers was the load path, and all of it was copies:

* `std::fs::read` hands `from_bytes` the whole file as bytes.
* `Encoding::decode` returned `String::from_utf8_lossy(..).into_owned()`, which
  copies the file even when its bytes already are valid UTF-8.
* `EolMode::normalise` returned `text.to_owned()` when there was no carriage
  return to rewrite -- which is every file that needs no rewriting.
* `Rope::from_str` copies the text into the rope, which is the one copy that has
  to happen.

So for the width of one call, four copies of the document were alive at once.

### Borrowed, not copied

`decode` and `normalise` return `Cow<str>` now: borrowed for valid UTF-8 with no
BOM and for text holding no `\r`, owned for the code pages, the two UTF-16 byte
orders, and line endings that have to be rewritten. No call site had to change
shape, because `&Cow<str>` is a `&str` where one was expected.

### A mapping that only said "the same"

`Document` kept three vectors over the lines: `rows` at sixteen bytes a line, an
index pair per line, `row_of` at eight and `row_count` at two. With no wrapping
and no collapsed block their whole content is "row `n` is line `n`", which for a
file this size is forty-one megabytes of the letter n.

`from_text` leaves all three empty, `sync_rows` returns before building when
there is nothing to build, and the six accessors that read them read an empty
vector as the identity. Which also means the rebuild that used to walk every line
after every edit of such a document is now a scan of the folded flags.

Wrapping and folding still build the mapping, and dropping back to neither drops
it again -- the test that asserts it turns wrapping on, checks the mapping is
there, turns it off, and checks all three vectors are empty.

### Verified

Same file, same machine, same measurement -- `VmHWM` polled while the editor ran
and `VmRSS` read two and a half seconds in: peak 747 -> 460 MB, steady 367 ->
328 MB. The 132 MB left between those two turned out not to be the read buffer
the next paragraph guessed at; the hundredth pass found it, and took the buffer
anyway.

Nothing else moved: the same screenshot of twelve string literals counts 1,548
string-coloured pixels, before and after. `cargo test --offline --release`: 2,291
tests, 0 failed, three of them new -- one that the two conversions borrow when
they can, one that a code page still decodes rather than borrowing, and the one
about the three vectors. `cargo clippy --offline --release --all-targets`: 0
warnings. `cargo fmt` was not run. No feature reference change: what the editor
does with a file is the same, it just holds one copy of it while doing it.

## The hundredth pass: a file that arrives in bites

The ninety-ninth pass made the whole-file copies borrow instead of copy and left
one standing: `std::fs::read` held the file while the rope was built from it.

`Document::from_file` streams now. A 64 KiB buffer is read, a UTF-8 BOM dropped,
every carriage return folded into the `\n` the document keeps, the terminators
counted for the line-ending decision, and the result appended to a `RopeBuilder`.
A character split across two bites waits in a three-byte tail until the rest of
it arrives. `EolMode::from_counts` is the rule `detect` already applied, lifted
into a function of its own so the counting loader and the walking detector cannot
drift apart.

Anything that is not UTF-8 -- a UTF-16 file, a code page, a binary -- ends the
stream, and the file is re-read whole and decoded as before, because telling
those apart means seeing every byte at once. Only that case still pays for a copy
of the file.

### Verified

A standalone load of the same 131 MiB file peaked at 331 MB before this change
and 162 MB after. It is also 59 ms faster rather than slower -- 927 ms against
868 ms, best of three each -- because the file is copied through one 64 KiB
buffer instead of being read into an allocation the size of the document first.

In the running editor the peak went 460 -> 457 MB and the steady weight 328 ->
326 MB, which is not the improvement those numbers promise. The reason is worth
writing down: instrumenting the open found the remaining 132 MB in
`State::a11y_snapshot`, which copies the *whole document* into a `String` for the
accessibility tree, once per snapshot. That is a memory cost and a time cost at
the same time -- 131 MB allocated and copied per call on this file -- and it is
not fixed there, because what a screen reader is handed is a decision about
behaviour rather than about bytes; the hundred-and-first pass caps it where it is
built.

`cargo test --offline --release`: 2,292 tests, 0 failed, one of them new: eight
file shapes -- plain, empty, shorter than three bytes, CRLF, a lone final CR, a
UTF-8 BOM, a four-byte character split across the 64 KiB boundary, and bytes that
are not UTF-8 -- each streamed through `from_file` and compared with `from_bytes`
on text, line ending and encoding. `cargo clippy --offline --release
--all-targets`: 0 warnings. `cargo fmt` was not run. No feature reference change.

## The hundred-and-first pass: the document a screen reader is handed

`a11y` had a limit all along -- `MAX_TEXT`, 64 KiB, with a line saying the
document was cut -- but it was applied in `tree`, after `State::a11y_snapshot`
had already built a `String` of the whole document. A limit that runs after the
copy limits what goes over D-Bus, not what the editor holds.

`Document::text_upto(chars)` hands back a rope slice rather than the file, and
the snapshot takes `MAX_TEXT` characters of it and remembers that it left the
rest behind, in a `truncated` flag of its own. `exposed_text` keeps its cut as a
second guard, and appends the note when either it or the producer had to stop.

### Verified

On the 131 MiB file the editor's peak went 457 -> 324 MB, which is now its steady
weight as well: there is no transient copy left to find. The 32 MiB file went 242
-> 206 MB the same way. A screen reader still gets the first 64 KiB and the line
that says the document was cut; what it no longer gets is the allocation.

`cargo test --offline --release`: 2,294 tests, 0 failed, two of them new -- the
prefix helper, and the note a truncated snapshot carries. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. No feature
reference change.

## The version at 0.3.3

A patch number for three passes of memory work, three commits, none of them a
change in behaviour: how a file is opened, what the document keeps while it is
open, and what the accessibility tree is handed.

The version is still declared once, in `[workspace.package]`, and everything that
reports one reads it from there. By hand again: a debian changelog entry above
0.3.2, the man page's header, and the MCP README's transcript. The reference's
title line reads the manifest, so `python3 docs/build-pdf.py` was all the PDF
needed -- 353,641 bytes, and `--check` reports it up to date.

Both packages in `dist/` were rebuilt, and the release page carries the five
assets the last one settled on: the AppImage, the deb, the docs zip, and the
source as a tarball and a zip.

Verified: `npp --version` prints `Neko++ 0.3.3`; `npp-mcp` prints `npp-mcp 0.3.3:
111 sections, 74 shortcuts, 1 of 5 plugins enabled` and answers `initialize` with
`"version":"0.3.3"`; `dpkg-parsechangelog -S Version` hands the package build
0.3.3; the deb's control file says 0.3.3 and the AppImage holds the same binary as
`target/release/npp`. `cargo test --offline --release`: 2,294 tests, 0 failed.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The plugin workspace keeps its own 0.1.0: different target, versioned
separately, interface unchanged.

## The hundred-and-second pass: a label under a menu that covers it

The toolbar's tooltips were shown through one gate, `State::modal_open`, and a
dropdown is not a modal. A menu panel is drawn from the menu bar downward, across
the bar of icons, so a pointer resting on `Open...` in the File menu is also, by
its coordinates, resting on the Save button underneath -- and half a second later
the label for that button appeared on top of the menu it belonged to, because the
tooltip is painted after everything else in the frame.

The hover highlight had the answer already. `Menu::hover_at` returns true when the
pointer is inside an open panel and the toolbar is only asked when it returns
false, so a dropdown has never lit a button it covers. Only the tooltip path
bypassed that, because it asked the bar directly on every pointer move.

So the gate became one predicate, `State::tooltips_allowed`: no modal and no menu.
`rest_tooltip` reads it, and the event loop closes a label with `hide_tooltip`
whenever it is false, which is the case a menu opened from the keyboard leaves
behind -- the pointer has not moved, so nothing else would take the label down.

### Verified

Two runs of each case on the release binary, against a binary built from a
worktree at the commit before the change. The counter is the pixels that differ in
the box under the Save All button -- x 88..215, y 52..82, which is where a label
would be -- compared with a frame of the same menu state taken with the pointer
parked off the controls, so the panel and its border cancel out:

| pointer resting on | before | after |
| --- | --- | --- |
| the button, nothing open | 1,806 | 1,806 |
| the File menu's first row, over the bar | 112 | 0 |
| the button, Tools menu open | 1,806 | 78 |

Both runs of each row gave the same numbers. The mark of 112 is the shorter label
`Save` rather than `Save All`, which is the fault exactly as reported: the label of
the button the row sits on. The 78 left in the third row is the button's own hover
highlight -- every differing pixel is in rows 27..54, the bar, and none below it.

`cargo test --offline --release`: 2,294 tests, 0 failed. `cargo clippy
--offline --release --all-targets`: 0 warnings. `cargo fmt` was not run. No test
of its own: the predicate reads a `Menu` and six modal flags inside `State`, and
`main.rs` has no test module to put a seventh state in. No feature reference
change.

## The hundred-and-third pass: a window that reads the display

The furniture of the window -- the menu bar, the toolbar, the tab strip, the
status bar and the panels -- was fifteen-pixel text on a twenty-pixel row, and
those two numbers were constants in `renderer.rs`. Nothing anywhere asked what
the display was. On a 4K panel whose desktop is set to 200%, where a pixel is
half of what the eye expects, every label came out at half the size it should be:
a user reads that as "everything is tiny", and no preference could fix it,
because the preferences had no number for it.

The number exists and the window always had it: `winit` reports a scale factor,
2.0 on such a screen, and the program had never asked for it.

### One pair of numbers, taken together

`UiMetrics` is now the font size and the row height as one value, built from a
density by `UiMetrics::at`. Rounded to whole pixels, because that is the size a
glyph is rasterised at: 18.75 is honoured by the shaper and comes out with stems
between two pixel columns, where 19 is sharp. A density that is zero, negative or
not a number falls back to the plain sizes rather than to nothing.

The document's text follows the same factor, in `apply_view_settings`, so a 4K
screen does not get a doubled menu bar over a fifteen-pixel document. The
preference stays what it was -- a size in pixels at density one -- so the number
the dialog shows is still the number a user typed. Zoom is untouched: it is a
property of the text, not of the window, and this pass does not change that.

### A reshape, not a repaint

The widgets shape their labels when they are built and keep the shaped buffers,
which is what makes the bar cheap to draw. So a density is not a number the
renderer reads at paint time; it is a reason to rebuild every one of them. Each
widget gained a `set_metrics` that hands the new metrics to the buffers it holds
-- `Buffer::set_metrics` re-shapes in place and keeps the text -- and drops the
small caches that would otherwise pin the old size: `MenuBar::shaped`,
`TabBar::stale`, the two shaped-text guards on the status bar, and the tooltip's
(text, columns) pair. Twenty-three widgets, and the toolbar is not among them: it
takes the row height as a parameter and holds no text.

The renderer takes the row height as an argument now instead of reading the
constant. That is the fault the first attempt at this pass shipped: the labels
were built at forty pixels and the bars were still measured at twenty, so the menu
bar came out twenty-six pixels tall with thirty-pixel text spilling out of it.
Two answers to "how tall is a row" is exactly one too many, and the renderer was
holding the wrong one.

Icons follow the row they sit in, a fixed fifteen hundredths of it, and the 16..28
pixel clamp is gone. The clamp only ever bound because the row height was the
constant twenty; left in place it would have capped a 4K display at 28-pixel icons
in a 40-pixel row, which is the same complaint moved one step along.

### Verified

Two runs of each case on the release binary. At density one the frame is
pixel-identical to a build of the commit before this one, twice over: 0 differing
pixels, and the one shot that disagreed at first differed from its own re-run by
the same 1,976 pixels, so the fault was a stale window on the screen rather than
the change. Which is the answer to "will I see anything on my 2560x1440 at 96
dpi": no.

At `WINIT_X11_SCALE_FACTOR=2` every band doubles, measured down the left edge of
the frame: menu bar 26 -> 46 pixels, toolbar 30 -> 54, icons 23 -> 46, tab strip
and text with them. The icons are rasterised at the new size rather than stretched:
a scan across one stroke gives the smooth run 121, 159, 153, 185, 206, 207, 140,
63 where a doubled 23-pixel atlas would have given every value twice. A dropdown
opened at that density highlights the right row for a pointer at (180, 70), and the
preferences dialog draws its rows at forty pixels.

`ui_scale 1.5` in a settings file on that same 96-dpi screen gives a menu bar of
36 pixels and a toolbar of 43, which is 1.5 rounded into whole pixels, so the
escape hatch works for the desktops that report one on a dense panel.

`cargo test --offline --release`: 2,298 tests, 0 failed, four of them new -- the
rounding, the fallback for a nonsense density, the clamped window scale, and its
round trip through the settings file. `cargo clippy --offline --release
--all-targets`: 0 warnings. `cargo fmt` was not run. The feature reference has the
`ui_scale` row and a paragraph on the window, and the PDF was rebuilt.

## The hundred-and-fourth pass: a scale a user can pick, and the widths that go with it

The pass before this one made the window read the display. Two things were left
out of it, and both of them showed on the screen.

The density was only ever what the display said. A 4K panel driven at 100% --
which is how a great many of them are driven, because a desktop that doubles
everything is a desktop where some other program looks wrong -- reports a
density of one, so the window stayed at fifteen-pixel labels and there was
nothing in the preferences to say otherwise. `ui_scale` existed in
`settings.txt` and no dialog offered it.

The other one is the reason this pass is more than a row. The furniture grew and
the boxes did not. `set_metrics` on each widget handed the new metrics to the
buffers it holds and stopped there, and every widget measures its own lengths
from a `char_width` it was built with. So at density two the glyphs doubled and
the character kept the advance of the sparse screen: a tab width computed as
`chars * char_width` stayed where it was while the title it held grew past the
edge, a value column drawn at `char_width * n` no longer met the value it was
drawing, and the preferences dialog came out with its right-hand values running
out of the box. That dialog was in the screenshot that closed the last pass, and
it read "System" for the theme with the m cut off.

### One row, four answers

`Row::UiScale`, "Window Scale", sits under Font Size. Left and right step through
`UiScale::ALL` in `settings.rs`: the display's own density first, then ×1.5, ×1.75
and ×2, and the value column writes the first as the words "Follow the display"
and the others as "x1.5", "x1.75", "x2". Zero is the number the settings file
already used for "ask the display", so the dialog and the file agree on what the
default means and a hand-written `ui_scale` line lands on a row rather than
between two.

Stepping stops at both ends instead of wrapping. The row is a choice among
window sizes that fit, and a press at ×2 that wrapped round to "follow the
display" would resize the window at the moment the user was pressing the key to
stop. The row and the words are in all 23 localisations, which the coverage test
that walks every catalog enforces rather than suggests.

### Lengths counted in characters

Every `set_metrics` in the twenty-three widgets now begins with the same two
lines after its early return: multiply `char_width` by the ratio of the new line
height to the old one, then take the new metrics. It is one line, and it is the
line that makes the arithmetic in the rest of the widget right -- the widget
counts in characters and has to know how wide a character is now. The early
return is what keeps it a no-op on a preference that did not move: an
unconditional multiplication would compound every time the display was asked
about itself.

The lengths that are *not* counted in characters are constants in pixels,
written when the only font was fifteen pixels. A tab's floor and ceiling, the tab
strip's vertical column, the folder and function list's clamp, the completion
popup's, the context menu's, the peeker's, the tooltip's ceiling, the about box's
buttons, the preferences floor and its pickers, the widths of the browser, the
style configurator, the shortcut mapper and the plugin list, and the find
dialog's pair. They all go through one number now, `scale_of(metrics)` in
`renderer.rs`: the line height over the twenty it was written against, one for a
density that is not a number. A constant means what it meant at fifteen pixels
at every density, which is the definition of a constant doing its job.

The preferences floor stays a floor and not a width. Its widest line is still
measured in characters, so a language with longer words still opens a wider box;
the floor only says how narrow it may get.

### The status bar drew over itself

Not caused by the scale, but made common by it, which is the same thing from a
user's chair. The bar draws two runs, one from the left edge and one from the
right, and the left one's clip rectangle ran to the window's edge rather than to
where the right one begins. On a wide window nothing met; on a narrow one at
double size the numbers landed inside the encoding label and the result read as
"lines 5UTF-8". The left run is now clipped where the right run starts, less one
padding, and the cut is rounded down to a whole character -- a clip halfway
through a glyph leaves half of it drawn, and a half-drawn "5" reads as another
label rather than as a truncation.

### Verified

Two routes to the same density, measured against each other. `WINIT_X11_SCALE_FACTOR=2`
on a 96-dpi display, and `ui_scale 2` in a fresh settings file on the same
display with no factor at all, give frames whose horizontal boundaries down the
left edge fall on the same pixel rows: the menu bar ends at 45, the toolbar at
90, the tab strip at 147, and in a 1371-pixel window the status bar starts at
1327. The setting is the same number as the factor, not a second way of
computing one.

The preferences dialog at that density is 880 pixels wide and its rows are 40
pixels apart, which is twice the 440 floor and the 20-pixel row it was built
from, and all seventeen rows and their values are inside it: "Follow the display"
for the new row, "System dialog", "English" and "Edit..." for the last three.
Before this pass the same dialog drew the same shell around text at the same
double size and cut "System" and "Englis" off at the right edge.

A tab strip reads its own floor and ceiling through the same number: a four
character title measures 128 pixels at density one and 256 at density two, a long
one 260 and 520, which is the pair of constants that used to clamp a doubled title
into a sparse screen's box. The find bar at density two draws all seven of its
right-hand buttons -- Find Next, Count, Find All in Current Document, Find All in
All Opened Documents, Mark All, Clear Marks, Close -- at double height with none
of them cut.

At density one the geometry is unchanged, because every factor is one and every
early return holds; the one visible difference is the preferences dialog, which
grew a row. The clip on the status bar only moves when the two runs would have
met, which on a wide window they never did.

`cargo test --offline --release`: 2,302 tests, 0 failed, four of them new -- the
window scale row stepping and stopping at both ends, a tab's width doubling with
the metrics, the preferences box doubling, and the two halves of the status bar
kept apart. `cargo clippy --offline --release --all-targets`: 0 warnings. `cargo
fmt` was not run. The feature reference has the row in the preferences table and
the paragraph on the furniture now points at the dialog, and the PDF was rebuilt.

## The hundred-and-fifth pass: the rows that did not move with the text

The pass before this one made the furniture grow with the display, and left one
thing behind. Six widgets -- the file browser, the plugin list, the preferences
dialog, the print preview, the shortcut mapper and the style configurator -- held
their own copy of the line height, taken from the metrics in their constructor.
Every row they draw, and every highlight over a row, was placed and sized from
that copy. Nothing ever wrote to it again.

So a density change grew the glyphs and left the rows where they were. The
labels are one shaped buffer per widget, and a buffer re-shapes its own lines at
the new metrics, so the text moved to a thirty-pixel pitch while the highlight
over the selected row stayed twenty pixels tall and sat above the row it was
meant to mark. Which is what a user sees when they open Preferences and change
the Window Scale row: the very dialog they are editing draws its selection short.
The same six widgets had the same fault, and the shortcut mapper is the one
somebody would try next.

### One answer to the row height

The copy is gone. Each of the six reads `self.metrics.line_height` through a
private `line_height`, which is the shape `charset.rs` had already chosen, and
`EditorView` gives up the same copy: its `line_height` was only ever assigned
from the metrics in `zoom` and `set_font_size`, so the getter reads the metrics
and the two assignments disappeared with the field.

This is the third time in four passes that a second copy of a number has been
the fault -- the renderer's row height against the labels', and now a widget's
row height against the metrics'. The rule the passes keep re-learning: a value
that can be derived is not a value to store.

### Verified

Measured on the running build, before and after, at a 96-dpi display: opened
Preferences, stepped the Window Scale row to x1.5, and measured the frame.

Before: the labels of the dialog sit on a 30-pixel pitch, which is the doubled
metrics, and the highlight over the selected row is 20 pixels tall at y 245..264
-- the height of the old font, and inside the box of the row above the one it is
meant to mark. After: the highlight is 30 pixels tall at y 180..209 and covers
the Window Scale row exactly. The whole layout moves between the two shots
because the dialog is taller once its rows are, and re-centres itself; the pair
of numbers that matter is 20 against 30, and the row it lands on.

`cargo test --offline --release`: 2,304 tests, 0 failed, two of them new -- a new
density moving the preferences rows and their highlight, and the same for the
shortcut mapper, both of which fail on the code before this pass. `cargo clippy
--offline --release --all-targets`: 0 warnings. `cargo fmt` was not run. The
feature reference needed no change: no command and no preference was added.

## The version at 0.4.0

A minor number, and the first one in the released history: 0.3.0 was followed by
three patches, and this release is a bigger thing than a patch -- four commits
that change what a person sees: a window that reads the display, a Window Scale
row in the preferences for the desktops that report a density of one on a dense
panel, the widths that had to grow with it, and two faults the growing exposed --
a selection highlight left at the old height, and a status bar that drew its two
halves over each other. A new preference is a new thing to choose, so this is
0.4.0 and not 0.3.4: a 4K user is the one who will look for it, and a patch
number would tell them there is nothing to look for.

The version is still declared once, in `[workspace.package]`. What cannot follow
the manifest was edited by hand, the same three places as last time: a debian
changelog entry above the 0.3.3 one, the man page's header, and the MCP README's
transcript. The reference's title line reads the manifest, so `python3
docs/build-pdf.py` was all the PDF needed, and `--check` says so.

Both packages in `dist/` were rebuilt by the two scripts under `packaging/`, and
the release page carries the five assets 0.3.3 settled on: the AppImage, the deb,
the documentation directory as a zip, and the source as a tarball and a zip, both
of them `git archive` with a `nekopp-0.4.0/` prefix.

Verified: `npp --version` prints `Neko++ 0.4.0`; `npp-mcp --version` prints
`npp-mcp 0.4.0: 111 sections, 74 shortcuts, 1 of 5 plugins enabled`;
`dpkg-parsechangelog -S Version` hands the package build 0.4.0; the deb's control
file says 0.4.0, the binary unpacked from it prints `Neko++ 0.4.0`, and the
AppImage's copy hashes the same as `target/release/npp`
(`e14a395bf21c6ffed0b753ec8c98988b1d53f99340fa3480fdce2ba25b14351b`) and prints
the same version when run. `docs/build-pdf.py --check` reports the PDF up to date
at 355,293 bytes. `cargo test --offline --release`: 2,304 tests, 0 failed. `cargo
clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not run.
The plugin workspace keeps its own 0.1.0: different target, versioned separately,
interface unchanged.

## The hundred-and-sixth pass: the page at the window's scale

The three passes before the release taught the window to read the display, gave
the reader a Window Scale row for the desktops that report a density of one, and
moved the panels and the rows with it. One surface was left out, because it is not
a widget: the playground.

The playground replaces the window with a page, and that page carried three
numbers written as constants -- a twenty-pixel line, a forty-pixel glyph atlas and
a thirteen-pixel corner label, with a fourteen-pixel gap to the edge. Nothing told
it that the window had a scale, so on a display that reports 200% the mode drew a
small page in a large window: the one place in the editor that stayed the size it
had been on a 96-dpi screen, next to furniture that had doubled.

### What the page reads instead

The three numbers are now described as the values they are -- what a line, a cell
and a label measure at a scale of one -- and the page multiplies them by the
window's scale. The label is re-shaped at the new size, its gap to the edge goes
with it, and the grid is derived from the window and the line height by a
`cells()` that is a function of both rather than a pair of constants in `resize`.

The atlas is the part that cannot be multiplied after the fact. Glyphs are
rasterised once into a texture, so a page drawn at twice the size needs the texture
built at twice the cell or the glyphs come out soft, stretched from the smaller
set. `Scene` now keeps the bind group's layout and its sampler, and `set_atlas`
builds a texture, a view and a bind group from a new atlas without touching the
three pipelines around it; `upload_atlas` is the upload that was already inside
`Scene::new`, lifted out so both paths use it.

A label is shaped once per text, so `set_scale` clears `hint_text`: setting the
metrics alone leaves the old shaping in place and the corner keeps the small
words.

### The scale is not a suggestion

`sane` refuses a zero, a negative and a NaN and returns one, because a scale of
zero would divide the grid by nothing. The bench passes one on purpose: its dumps
are compared with each other, and a scale would move every number in them without
saying anything about the page.

### Verified

Measured on the running build, X display :1 at 96 dpi unless said otherwise,
window 1100 by 750 unless said otherwise, the page opened with `Ctrl+Alt+K` and
shot through `/tmp/scalecheck.py`. The measure is the pitch of the line-number
gutter down the left edge of the page, which is one row of the page.

| Window scale | Window | Gutter pitch | Rows on the page |
|---|---|---|---|
| ×1 (`ui_scale 1`) | 1100×750 | 20 px | 39 |
| ×2 (`ui_scale 2`) | 1100×750 | 39 px | 19 |
| ×3 (`ui_scale 3`) | 1100×750 | 58 px | 11 |
| Follow the display, display at 200% | 2560×1371 | 40 px | 33 |

The two mechanisms differ in exactly the way they should. A preferred scale does
not grow the window it is applied to, so twice the line height is half the rows:
39 becomes 19, and 58 pixels leaves 11. A display that reports twice the density
doubles the window as well, so the pitch doubles and the row count stays: 33 rows
on a page 1371 pixels tall, which is the 34 the arithmetic gives for 685 logical
rows at a twenty-pixel line. The glyphs are
rasterised at the size they are drawn at: the atlas is 40 pixels at scale one, 80
at two and 120 at three, and the letters at ×3 are sharp-edged in the shot rather
than interpolated.

The cramped look of a page at around fifty columns is older than this pass. The
bench dumps the same page at 550 by 375 with a scale of one -- `bench "" 550 375 1
/tmp/small.ppm playground playground_time=1.0` -- and the furniture crowds the text
there too; what changed is that a small window can now ask for a small page on
purpose.

`cargo test --offline --release`: 2,307 tests, 0 failed, three of them new -- the
grid halving with the line height, the atlas and the label at the scale they draw
at, and a nonsense scale drawing the plain page. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. The feature
reference gained one paragraph, under the page's own section, and the PDF was
built again.

## The hundred-and-seventh pass: the plan for a terminal front-end

Every other section here records something that was built and can be measured. This
one records a decision taken before any code moved, because the decision is the
expensive part: whether Neko++ gets a second front-end, drawn in characters, and how
that front-end stays in step with the first one. `docs/tui-plan.md` is the document;
this is why it says what it says.

The question arrived in the order a person actually thinks about it. Is it possible?
Yes, and the reason is that `nn-core` never learned about a window -- ropey, memchr
and regex are its whole dependency list, and it holds the document, the caret, undo,
the lexers, folding, search, the encodings, the WebAssembly host and the playground
simulation. `npp-mcp` is already a binary that edits text through it with no
renderer attached. Would it cost the windowed version anything? Not at runtime; only
in build time and in the discipline of a shared seam. Can the two grow together?
Only if the seam moves first.

### The seam, and why it is the whole plan

`Action` (241 variants) and its dispatch (275 sites), the menu tree (5,295 lines) and
the preference rows (2,343) all live in the windowed program, welded to a `State`
with 78 window-shaped fields. A second front-end that copied them would answer the
same questions twice and drift at the first new command -- the thing the plan exists
to prevent. So phase 1 pulls the decisions into a shared library and leaves the two
front-ends holding drawings: one dispatch `match`, one menu tree, one settings list
with labels and choices. A new command then fails to compile until both front-ends
have it, and the handful of rows a terminal cannot express is a constant with a
reason each and a test that keeps the list short.

That phase is also the only one that can hurt the window, and its gate is a picture
rather than a promise. The bench already writes a PPM and two runs of the same flags
hash the same -- `a163442c3c14d3954aff272bd0937cdc` twice at 1280 by 720 with `menu
tabmenu` -- so a refactor that moves ten thousand lines can be proven not to have
moved a pixel before anyone looks at a frame rate.

### What the numbers decided

Three measurements from the conversation that shaped the plan rather than decorating
it. A REST client over the existing API is the wrong shape for an editor: 8.3 ms a
call, one frame of the window it talks to, 26.7 ms to change one character in a
one-megabyte document, and no way to insert at a point at all, because an empty range
is refused. A terminal is cheap to draw into: a full screen of styled text -- 37,116
bytes -- is parsed by kitty inside one refresh of a 120 Hz display (4.65 ms) and by
alacritty in 0.89 ms, so what decides a TUI's speed is the number of bytes it writes,
which is why the diff writer is not a later optimisation. And the windowed version
without a GPU is real but expensive: ten seconds of the animated page cost 1.35
CPU-seconds on the 1080 Ti and 30.01 on llvmpipe, which is why the playground gets a
plain character tier first.

### Verified

No code changed: `git status` was clean before this pass and the diff is documents
only -- the plan, this section and a row in the README's table. `cargo test --offline --release`: 2,307 tests, 0 failed. `cargo clippy
--offline --release --all-targets`: 0 warnings. `cargo fmt` was not run. The feature
reference and its PDF are untouched, because nothing a person can see has changed
yet; the plan says which pass will change it.

## The hundred-and-eighth pass: the editor in a terminal

The plan of the hundred-and-seventh pass put the extraction first: move the
decisions out of the windowed program, then build the front-end that would use
them. The order changed, and the reason is worth recording, because it is a rule
about refactoring rather than a preference. A refactor with no consumer guesses
at the API it should expose; a shell built first tells it what to expose. So the
terminal front-end came first, as a new crate that touches nothing the window
uses, and the extraction follows knowing what the second consumer actually needs.

### What was built

`crates/nn-tui`, one binary, `npp-tui`: the screen, the caret, the colours,
typing, opening, saving, and a bar at the bottom. It is its own crate rather than
a `--tui` switch in `npp` for the reason `npp-mcp` is: a front-end that cannot
create a window should not link wgpu, and the packaging scripts already build
`--bin npp` on its own.

There is no terminal library. Raw mode, the window size and a read with a
deadline are five calls -- `tcgetattr`, `tcsetattr`, `ioctl`, `poll`, `read` --
and a dependency to keep current would have been larger than the code that
replaces it. The cost is four `unsafe` blocks in one file that say what they
assume.

The screen is a grid of cells and a difference. A cell carries one tone, of
fifteen, each of which sets a foreground *and* a background: half a pair is how a
terminal ends up with black on black. A frame repaints a row from the first
column that differs to the last, and says nothing about a row that is the same, so
a keystroke costs a few hundred bytes rather than a screenful.

Everything an editor does with text stayed in `nn-core`: the caret and its
motions, undo and redo, auto-indent, the lexers and the highlighter, encodings and
line endings, `Document::is_dirty`. The shell re-lexes from the edited line, and
by the whole file only after undo, which can have changed any of them. Page Up and
Page Down work without a special case because the core takes "a screenful" from
the editor's viewport, and a cell is one line high: the shell sets the viewport to
the window size in cells and the motions follow.

Three details that a first version gets wrong. A tab is drawn as the spaces it
stands for, in the tone of the text around it. A wide character takes two cells
and is dropped rather than split in the last column, because a glyph in the last
column makes a terminal wrap and scroll the whole screen. And a combining mark is
not drawn at all, which is a gap rather than a decision: it needs a cell that
knows it is carrying an accent.

The bar gives way on the left. At forty columns a path and a line number cannot
both fit, and the two numbers a person is looking at while typing are the line and
the column, so the path is what gets cut.

### Verified

Measured in the release build, on X display :1, `npp-tui` running in kitty 0.48:
the reference file opened with its line numbers, its comment green and its
keywords blue; three presses of Down put the caret on line 3; typing replaced that
line; `Ctrl+S` wrote it, and the file on disk went from `//` to `// still fast`
with the `[+]` marker leaving the bar. A second shot after the save shows `saved`
where the path was.

The frame cost, from the test `a_keystroke_costs_a_fraction_of_a_screen...`, which
prints what it measures: the first paint of a 120 by 40 window costs 10,425 bytes;
a keystroke in a 500-line file costs 306 of them, in about 40 microseconds of
drawing. That is three percent of a screen per keystroke, which is the property the
whole module exists for.

`cargo test --offline --release`: 2,331 tests, 0 failed, 24 of them new -- the
diff writer, the key parser, the tones, and the layout (the gutter, the tab stops,
the wide character, the scrolling that follows the caret, the two presses that
discard unsaved work, and the save that writes a file). `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. The feature
reference and its PDF are untouched: a shell with no menus is not yet something to
document for a user, and the plan says the chapter arrives with the menus.

What is not there yet, in the plan's own terms: the menu bar and its dropdowns,
the preference dialog, find, tabs, the split view, plugins, the clipboard, and the
playground. All of them read from the shared library that phase 1 has not built
yet.

## The hundred-and-ninth pass: the vocabulary moves out

Phase 1 of the plan, first pass. The command vocabulary -- `Action` and its four
companion enums, the name each command is written as in the settings file, the
bindable list and the text-command table -- left `crates/nn-app/src/menu.rs` for
a new crate, `crates/nn-shared`.

The crate is the answer to a sentence rather than to a wish for tidiness. The
plan's requirement is that any feature added to the windowed version is converted
to the terminal version where it is possible. A rule like that survives on
structure: a second list of actions and a second `match` that runs them would
drift at the first new command, and the drift would be invisible until somebody
compared the two programs row by row. So the list lives in one crate, and each
front-end keeps what it is actually alone in.

`nn-shared` has one dependency, `nn-core`, and only for the payloads a command
carries: an `Encoding` to re-read a file's bytes as, a `Hash` to compute, a
`CodePage` to convert to. There is no wgpu in its tree and no terminal either: it
cannot open a window, read a key or measure a font, which is what makes it safe
for both front-ends to depend on.

### `pub use` rather than a rename

`menu.rs` keeps four lines of re-export, so the 275 `Action::` sites, the import
lists in `main.rs`, `bench.rs`, `keymap.rs`, `a11y.rs` and the API's command
dispatch all keep compiling unchanged. A refactor that is provably not a
behaviour change should not also be a rename: the compiler proves the first
thing, and a rename only adds lines to review.

### The one thing that did not come along

`Action::from_name` reads a settings-file name and answers the command. Four
families are prefix-shaped -- `encoding_set_*`, `encoding_convert_*`,
`language_built_in_*`, `theme_*` -- and three of them resolve against lists that
are the vocabulary's own business: `CodePage::ALL`, the encodings, and
`nn_core::lang_by_key`. A theme is not. `ThemeChoice::ALL` is fifty palette names
whose only other use is choosing colours, and a command has no reason to know
them.

So the theme prefix moved out of the shared function and into
`menu::action_from_name`, which the two settings parsers -- the `key` line reader
and the API's command call -- now go through. That split is visible in the tests:
`Action::from_name("theme_dracula")` is `None` and `action_from_name` gives
`Theme(12)`, while `save` resolves the same way in both. Nothing a user can
observe changed, because both callers were already inside the windowed program;
what changed is that the shared vocabulary no longer pulls a list of themes
behind it, and phase 1's next pass can move the settings rows without dragging
the menus with them.

### Verified

The gate for this phase is a picture, not an opinion: `bench` over a matrix of
thirty-one flag combinations at 1280 by 720, dumped to PPM, hashed before the
move and again after it. All thirty-one hashes are identical. The matrix covers
the plain document, the menu bar with each dropdown, the preferences dialog in
both themes, wrap and fold on a file with long lines, the function list, the find
list, the completion popup, bookmarks, block select, the multi-caret rows, the
peeker, the print preview, three tabs, and a run with every one of them at once.
Two runs of the same flags hash the same, so the comparison is against a stable
picture rather than against a lucky one.

`cargo test --offline --release`: 2,331 tests, 0 failed -- the same count as
before the move, because the four tests that belonged to `Action` (the bindable
list, the text-command table, the duplicates check, the encoding names) moved
with it rather than being rewritten, and one theme test grew two assertions.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched: nothing a person sees
changed.

## The hundred-and-tenth pass: the tree follows the vocabulary

Phase 1, second pass. The menus themselves -- `Item`, `Menu`, `PluginMenu`, the
public row view the accessibility tree reads, and the fifteen builders that
return one dropdown each -- moved into `nn-shared`, beside the vocabulary that
now names what the rows run.

A menu row is a decision, not a drawing: this label, this chord, this command, or
these children. The window's share of a menu starts where the pixel does -- the
shaped buffers, the panel arithmetic, the arrow, the hover -- and that share is
what stayed in `nn-app`. The terminal front-end will read the same fifteen
builders and write labels into cells, and neither front-end can add a row the
other does not have, because there is only one place a row can be added.

The two seams the move exposed are worth recording, because both are the same
question asked twice: what does a menu know that the crate that holds it cannot?

### The theme list, again

`settings_items` builds the Theme submenu from `ThemeChoice::ALL`, using each
choice's label and passing its position as `Action::Theme(n)`. That is the same
fifty-name list the previous pass kept out of the vocabulary, and it is still the
settings' own business. So the builder takes the labels as an argument:
`settings_items(&[...])`, one entry per theme, in the order the preference row
steps through them. The window passes `ThemeChoice::ALL`'s labels through a
three-line helper; `Action::Theme(n)` is an index into the slice it was given, so
the menu and the cycling row agree by construction rather than by both being
remembered.

The alternative -- moving `ThemeChoice` into the shared crate -- was rejected for
now because the type is not only a list: `palette()`, `theme()` and `is_light()`
turn a choice into colours, and those three reach into the forty-seven palettes
in `nn-app`. Half a type is worse than a parameter, and the parameter is one line
at the only call site.

### What could not follow

Two things still need the catalogue of translations: the Plugins menu, whose
rows are composed from names the plugin supplies and verbs the catalogue
translates, and the two passes that write a column after the tree is built --
`relabel`, which fills each row's label from the catalogue, and `set_shortcuts`,
which fills each row's shortcut column from the keymap. Both stayed in `nn-app`
with the type they operate on now public, field by field: a row's label, its
English text, its chord, its command, whether it is a separator, whether its
label is a literal, and its children.

Translation is the next thing to move and the reason the settings rows wait:
`i18n::is_literal` decides whether a word is a name by asking whether the theme
catalogue holds it, so the catalogue and the list of names have to move together
or not at all.

### Verified

The same gate as the previous pass, and it matters more here, because the rows
this pass moved are the rows the pictures are made of: `bench` over thirty-one
flag combinations at 1280 by 720, hashed before and after. All thirty-one are
identical -- including the eight dropdown dumps (`menu`, `menu_edit`,
`menu_tab`, `menu_view`, `menu_lines`, `menu_settings`, `menu_run`,
`context`), the preferences dialog in both themes, and the run with every flag at
once, which is the one that would notice a row added to a submenu or a separator
in the wrong place.

`cargo test --offline --release`: 2,331 tests, 0 failed, the same count: the
tests that walk the assembled menu stayed where the assembled menu is, because
they drive a `MenuBar` and check what it draws rather than what the builder
returned. `cargo clippy --offline --release --all-targets`: 0 warnings.
`cargo fmt` was not run. The feature reference and its PDF are untouched: no
label, no position and no behaviour changed, which is what the hashes say.

## The hundred-and-eleventh pass: the keys move too

Phase 1, third pass: `keymap.rs` left `nn-app` for `nn-shared`. A chord is a
decision -- this key means that command -- and the terminal front-end has to draw
the same chords beside the same rows, so the table belongs beside the rows.

The move cost one API change. `Keymap::parse_line` applies a `key <action>
<chord>` line from the settings file and used to turn the action's name into an
`Action` itself; the name can also be `theme_dracula`, which is a setting rather
than a command, and the list of themes is not the keymap's to know. So `parse_line`
now takes the resolver as a parameter, and the one production caller -- the
settings reader -- passes the same `menu::action_from_name` it uses for everything
else in the file. That keeps one resolver per front-end instead of one inside each
piece of the crate, and it is what lets the keymap be shared without dragging the
themes behind it.

Like the two passes before it, this one's consumer arrives with phase 3 rather
than now: nothing in the terminal shell looks a key up yet, because a terminal
cannot send every chord the shared table lists -- `Ctrl+Shift+Q` and `Ctrl+Q` are
the same byte on a legacy terminal -- and a shell with no menus has no shortcut
column to fill. The claim this pass makes is narrower and checkable: the windowed
program's behaviour is unchanged, and the table is now in the crate the terminal
already depends on.

### Verified

`bench` over the same thirty-one flag combinations, hashed before and after: all
thirty-one identical, which is what proves the shortcut column still reads the
same table through the re-export. `cargo test --offline --release`: 2,331 tests, 0
failed -- `keymap`'s own twenty-nine moved with it, and the three that apply a
settings line now pass `Action::from_name` as their resolver. `cargo clippy
--offline --release --all-targets`: 0 warnings. `cargo fmt` was not run. The
feature reference and its PDF are untouched.

## The hundred-and-twelfth pass: the terminal draws the window's menus

Phase 3, first pass, and the first time the extraction pays for itself in
something a person can see. `npp-tui` now has a menu bar: `F9` opens it, Left and
Right walk the thirteen titles, Down opens a dropdown, Up and Down walk its rows,
Right and Left step into and out of a submenu, Enter runs the row, and Escape
backs out one level at a time. The rows are not written in the shell. They come
from `nn_shared::menu`, which is the same tree the windowed front-end shapes
glyphs from, and the chords in the right-hand column come from
`nn_shared::keymap`, which is the window's own table. A command added to a menu is
on both bars before either program is run.

The bar itself moved too, which is the belt to that braces. The thirteen menus
were built inside `MenuBar::new`; they are now `nn_shared::menu::menus`, taking a
`Contents` bundle for the four lists that come from outside -- recent files, saved
run commands, user languages, themes -- and leaving the Plugins rows empty for the
front-end that owns the plugin host. The three indices that are reached by
position (Language, Run, Plugins) are constants there rather than in either
front-end, so a bar with thirteen titles in one order is one statement rather than
two.

The shell has six of the two hundred and forty-one commands. The other two
hundred and thirty-five say so: `New: not in the terminal yet`, on the bar, in
the alert colour. That is deliberate. A menu that swallows a press teaches a user
that the program is broken, and a list of what is missing is the shell's own work
list -- one that a reader can check against the window's menus item by item,
because both come from the same place.

### What the drawing cost

A dropdown is a box: `┌─┐│└┘` from the box-drawing block, the labels in the theme's
menu colours, separators as a rule across the inside, `▸` on a row that opens a
submenu, and the chord right-aligned. The box is cut at the last column rather
than allowed to wrap: a glyph in the last column makes a real terminal scroll the
whole screen, which is the one mistake that cannot be walked back from inside the
program.

The colours are the windowed theme's own menu colours, copied into four new tones
-- `Menu`, `MenuHover`, `MenuBorder` -- so a menu looks like a menu in either
front-end.

### Two bugs the picture found

The first was mine and the tests caught it: `items_at(depth)` read every level's
index as an index into the *bar*, so the third level of a submenu chain showed a
different menu's rows -- Encoding's third level came out as Language's. A level
names a row of the level above it, not a menu, and the fix is one loop. The test
that found it walks Encoding, Character sets, a script, and picks a code page.

The second only a real terminal could show. Closing a dropdown left its box
painted on the screen. The cause was not the menu: it was that a frame writes only
the cells that differ from the previous frame, the previous frame's grid is reused
rather than cleared, and a blank cell and a cell painted in the document's colours
are the same cell -- so nothing ever repainted the places the box had been. Two
changes, both of them the same idea: the editor now paints every row edge to edge
in its gutter and page colours, and a fresh frame (the first one, and the first
after a resize) writes every column rather than diffing against an empty grid. The
background is now the program's rather than the terminal's, which is also what
makes the window and the shell look like the same editor.

### Verified

In kitty 0.48 on display :1: `F9` draws the bar with all thirteen titles and lights
the first, Down opens the File dropdown with `New` highlighted, `Ctrl+N` in its
chord column, the separators in place and the arrows on its submenus; Enter
on `New` closes the menu and says `New: not in the terminal yet` on the bar in
amber; closing the menu leaves no trace of the box. Screenshots `/tmp/menu_bar.png`,
`/tmp/menu_file.png`, `/tmp/menu_unsupported.png`, `/tmp/menu_closed2.png`.

The frame cost, printed by the shell's own measuring test: the first paint of a
120 by 40 window is now 15,554 bytes rather than 10,425 -- the whole screen is
written once so the background is the editor's -- and a keystroke in a 500-line
file is still 306 bytes, drawn in about 65 microseconds. Painting every row edge
to edge therefore costs nothing per keystroke: the cells that changed were already
being written, and the ones that did not change compare equal.

`cargo test --offline --release`: 2,342 tests, 0 failed, eleven of them new -- the
thirteen titles in order, the chords coming from the shared keymap, the cursor
skipping separators, a submenu opening rather than running, a command three levels
down, the box drawn round its rows, and a box wider than the screen being cut. The
windowed front-end is untouched: all thirty-one `bench` dumps still hash to the
same values. `cargo clippy --offline --release --all-targets`: 0 warnings. `cargo
fmt` was not run. The feature reference and its PDF are untouched -- the terminal
is not shipped as a program yet.

## The hundred-and-thirteenth pass: the theme list leaves the window

Phase 1, fourth pass: the list of themes left `nn-app` for `nn-shared`. Which
themes exist, what each is called in Preferences and the keyword it is stored as
are decisions the two front-ends have to make the same way -- a terminal that
offered forty of the window's fifty would be a different editor -- so the list
belongs in the crate they share.

What moved is the choice: the fifty-variant enum, `ALL` in the order the
preferences row cycles it, the names and keywords, `named()` (where the named
themes start in that order), `key`, `label`, `from_key`, `next` and `previous`.
What stayed is the colours. A `ThemeChoice` names a palette; `themes::palette_of`
reads its colours out of the catalogue, `themes::theme_of` resolves the choice
into a `Theme` against what the desktop said, and `themes::is_light_of` answers
the one question the window asked of the resolved theme. The three were methods
until this pass and are functions now, for the reason inherent impls fail across
a crate boundary: the type is no longer the window's to extend.

That split is the point rather than an accident of the move. The catalogue is
forty-seven palettes of ten colours each, and a colour is a number only a screen
can use; the choice is a name, and both front-ends need it. So `nn-shared` holds
the names and `nn-app` holds the values, and the only thing that can go wrong is
the two lists disagreeing about which name sits where. That is what the new test
in `themes.rs` is for: it asserts `CATALOGUE.len() == ThemeChoice::NAMES.len()`,
then walks every choice and requires `palette_of` to be `Some` exactly for the
named forty-seven and its name and keyword to match the choice's. It replaces
the test that stood in `settings.rs` and assumed the three built-ins were a
prefix of the order -- a weaker claim, and one that would have kept passing while
a palette inserted in the middle renumbered every colour after it.

Two smaller changes follow from the same idea. `i18n::is_literal` asks the shared
list whether a label is a proper noun, rather than asking the catalogue: a theme
called `Light` is as untranslatable as one called `Nord`, and now the built-ins
are covered too. And the palette itself keeps its own `name` and `key` fields
beside its colours, which is duplication -- but the alternative is editing
forty-seven colour literals to change what they are called, and the test above is
the guard that a name written twice arrives twice.

### Verified

`bench` over the same thirty-one flag combinations, hashed before and after: all
thirty-one identical, among them `light`, `light_prefs`, `prefs`,
`prefs_settings` and `menu_settings` -- the five that draw a theme's colours and
the preferences row that cycles them, so the dump is what says the colours still
come from the same place. `cargo test --offline --release`: 2,346 tests, 0 failed,
four of them new in `nn-shared::theme`; the ones in `settings.rs` now call the
three functions and the weaker of them was replaced by the stronger test above.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched -- the terminal is not
shipped as a program yet, and nothing a user of the window can see has changed.

## The hundred-and-fourteenth pass: the preference rows move too

Phase 1, fifth pass: `Row` and `ROWS` left `nn-app` for `nn-shared::prefs`. The
preferences dialog is one dialog drawn twice -- the same seventeen settings, in
the same order, with the same words on them -- and its order is the part a user
knows by heart. Tab Width is the fourth row. A terminal that listed its settings
in a different order would be a second dialog with the same name, so the list
goes where both can reach it.

What moved is a hundred lines with no dependencies at all: the seventeen
variants, `ALL` in the order the dialog draws them, the label each is shown with,
and the three questions the keyboard asks of a row -- is this a switch, does it
open another dialog, does it open a list rather than stepping. That last group is
the reason the move is worth making now rather than folding it into the settings
themselves: the *shape* of a row is what a front-end needs to decide what Left
and Right and Enter do, and it is entirely independent of what the row holds. So
the questions moved while the answers -- `Settings`, its defaults, its file
format, the list of palettes a picker shows -- stayed behind, and the terminal can
draw a dialog it can navigate before it can draw one it can fill in.

Three tests came with the list, and one of them is new ground. `ALL` covers every
row exactly once and is `ROWS` long, which is what stops a setting being added to
the enum and forgotten in the order -- the failure a user would see is a row that
cannot be reached. The labels are all different, where the window's own test only
asked that they not be empty; two rows reading alike are two rows to try before
you find the one you meant. And the three classifications are mutually exclusive,
because the dialog reads them to decide what a keypress means, so a row that
answered yes twice would be a key that does nothing.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical,
`prefs` and `prefs_settings` and `light_prefs` among them -- the three dumps that
draw this dialog, its rows and its labels, which is what says the words still come
from the same list in the same order. `cargo test --offline --release`: 2,349
tests, 0 failed, `nn-shared` now at forty of them and the window's untouched.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched.

## The hundred-and-fifteenth pass: the colour type moves, its shader does not

Phase 1, sixth pass, and the one with the widest blast radius so far: `Rgb` left
`nn-app` for `nn-shared::color`. Three bytes is three bytes in a terminal. The
theme catalogue is a list of them, the settings file writes them as hex, and a
terminal drawing in true colour carries the same values into an escape sequence
that the window carries into a vertex buffer. There was no reason for the type to
belong to the front-end with a shader, only a reason for its methods to.

That reason cost this pass its shape. `Rgb` had six methods: `new` and
`luminance`, which are facts about a colour and moved with it, and `to_linear`,
`to_linear_alpha`, `to_cosmic` and `to_wgpu`, which are facts about a GPU and
could not. Rust will not let a crate write an inherent `impl` for a type it does
not own, so those four became `theme::ToGpu` -- a trait in the window, so that
the name of the type could stay where every call site already had it. Twenty-nine
files gained one import and no call changed. That is the whole argument for the
trait: 225 call sites would otherwise have become `theme::linear(colour)` and
every one of them a place to get the argument order wrong.

The split is also what the terminal needed to hear. `nn-shared::color` carries
`luminance` and says in its own doc why the conversions are absent: the terminal
has no shader to feed. When it draws a theme, it will map these three bytes onto
whatever colour depth the terminal reports, which is a different problem from
linearising them, and one the window must never be asked about.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical.
That is a stronger claim this time than in the passes before it -- a colour that
reached the shader through a different path would have shown up as a one-bit
difference in any of the thirty-one dumps, and every one of them is a picture
built out of some part of the theme. `cargo test --offline --release`: 2,351
tests, 0 failed, two new in `nn-shared::color` for the ends of the brightness
scale and the order of the three channel weights. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run -- the twenty-nine
import lines were written by hand and sorted by eye, so a reader will find them
in the order the files keep. The feature reference and its PDF are untouched.

## The hundred-and-sixteenth pass: the theme and its catalogue move

Phase 1, seventh pass, and the biggest single move since the shell: `Theme`,
`Palette`, the forty-seven-entry catalogue and everything that builds a theme out
of them left `nn-app` for `nn-shared`. A theme is data. Forty-nine colours, a
blend, a shade, a hex reader and a Notepad++ importer -- none of it needs a
window, and the terminal needs all of it, because a terminal drawing Tokyo Night
has to draw the same Tokyo Night.

The two modules went to two places. `nn-shared::theme` gained `Theme` itself, the
two hand-written palettes, `from_palette`, `from_stylers`, `parse_hex` and
`shade`, joining the choice that moved in the hundred-and-thirteenth pass: one
module answers "which theme" and "what colour is a keyword in it". `nn-shared::
catalogue` is the window's `themes` module under a name that reads better beside
`theme` than `themes` did. The blend, `mix`, went to `color`, where the thing it
takes and returns lives.

What is left in `nn-app` is one trait, one function and two re-exports. `ToGpu`
converts sRGB to what a shader wants; `srgb_to_linear` is the arithmetic under
it; and `pub use nn_shared::theme::Theme`, `pub use nn_shared::color::{mix, Rgb}`
keep every `crate::theme::` path in the crate working. `nn-app/src/themes.rs` is
now four lines of documentation and a glob re-export. That is the shape these
passes leave behind: the file stays as a name, and the name stops being a place
where anything is decided.

Two test modules met head-on and had to be told apart. `nn-shared::theme` now
holds the choice's four tests and the theme's seven, and Rust allows one `mod
tests` per file, so the second is `theme_tests` -- a name that says which half of
the module it belongs to. The three tests for `srgb_to_linear` went back to the
window with the function: a test that cannot reach the code it tests is a test
that will be deleted by the next person to touch the file.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical.
Forty-seven palettes, two built-in themes, an importer and a blend all changed
crate in this pass, so the dumps are the whole of the argument -- every colour in
all thirty-one pictures comes out of the code that moved, and every one is
unchanged. `cargo test --offline --release`: 2,351 tests, 0 failed, the same total
as the pass before: seven theme tests in `nn-shared`, three conversion tests in
`nn-app`, none lost and none invented. `cargo clippy --offline --release
--all-targets`: 0 warnings. `cargo fmt` was not run. The feature reference and its
PDF are untouched.

## The hundred-and-seventeenth pass: the sixteen colours a user may change

Phase 1, eighth pass: `Slot`, `record` and `effective_theme` left `styles.rs` for
`nn-shared::theme`. A style slot is a promise about the theme -- "the keyword
colour is here" -- and the sixteen of them are what the settings file writes its
`style` lines against. A terminal that let its user recolour a keyword and wrote
a different word into the same file would corrupt the window's settings, so the
sixteen names and the way an override is folded into a theme are the same
decisions in both front-ends.

The move is the mirror of the two before it, and it is the reason those came
first: `Slot::get` and `Slot::set` take a `&Theme` and a `Rgb`, and both of those
already live in the shared crate. Without that, this pass would have had to take
the theme along or invent a `SlotView` trait to bridge the two -- and a trait
invented to move sixteen enum variants is a decision disguised as plumbing.

What stayed is the dialog: swatches, glyph buffers, a caret, a rect to centre
itself in. Nine of the file's tests left with the code they test -- the keys
round-tripping, every slot reaching its own field and no other, `record` keeping
the list in dialog order, an override landing on top of every one of the
forty-nine palettes. A tenth stayed behind, because it opens the dialog and steps
a swatch, which is a thing only a window does.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical. The
argument is `menu_settings`, `prefs` and `light_prefs`: these are the dumps that
draw a theme, and a slot that reached the wrong field would have recoloured one of
them. `cargo test --offline --release`: 2,351 tests, 0 failed -- unchanged again,
nine moved and none lost. `cargo clippy --offline --release --all-targets`: 0
warnings. `cargo fmt` was not run. The feature reference and its PDF are
untouched.

## The hundred-and-eighteenth pass: the first setting value

Phase 1, ninth pass, and a small one on purpose: this is the last type standing
between `Settings` and the shared crate. `TabLayout` moved from `tabbar.rs` to
`nn-shared::settings`, three arrangements with three keywords, a label and a
`next` -- the same shape as `ThemeChoice`, and shared for the same reason. A tab
strip is not a window: `tmux` has one, and a terminal front-end that can put its
tabs down the side has to read the same word out of the same file and write the
same word back.

The module is new and holds one type, which is worth saying out loud rather than
burying: naming it `settings` before `Settings` arrives is what makes the next
pass a move rather than another new module, and it puts the file where a reader
looking for "the values a setting can hold" will find it first.

One doc link had to be let go. `TabLayout::MultiLine`'s comment pointed at
`MAX_ROWS`, a private constant of the drawing code that stays behind, and a
shared crate cannot link to a number it does not own. The comment now says "up to
three rows", which is the whole of what the constant said.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical,
`menu_tab` and `tabs3` and `menu_view` among them -- the three dumps that draw a
strip, which is where a wrong layout would show. `cargo test --offline
--release`: 2,351 tests, 0 failed -- four `TabLayout` tests moved and none lost.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched.

## The hundred-and-nineteenth pass: the settings themselves

Phase 1, tenth pass, and the one this series was walking towards: `Settings`, with
`UiScale`, `FileDialog` and the ten numeric bounds the dialog offers, left
`nn-app` for `nn-shared::settings`. It is the type both front-ends have to agree
about completely -- the same file, the same keywords, the same defaults, the same
clamp on every number -- and until this pass the terminal could draw the
preference rows but had no settings to put in them.

The eight passes before this one are why it could be a move rather than a rewrite.
`Settings` names a `ThemeChoice`, a `TabLayout`, a `Keymap`, a `Vec<(Slot, Rgb)>`,
a `BackupMode` and a `RunList`; every one of those but the last two -- which are
`nn_core`'s and were reachable all along -- moved in a pass of its own, each with
the same gate. Nothing in this pass had to invent a bridging trait, and nothing
had to be split: the file crossed whole.

One function had to follow it. `menu::action_from_name` resolves the name on a
`key` line, `theme_dracula` included, and the parser calls it for every line it
reads; a parser in the shared crate cannot call a resolver in the window. It
moved to `nn-shared::menu`, beside the theme list it consults, and the window
re-exports it, so `crate::menu::action_from_name` still says what it said. That
is the shape of this boundary in general: the vocabulary moves, and the file that
used to own it keeps the name.

`nn-app/src/settings.rs` is now a documentation comment and one `pub use` of
nineteen items. The two test modules that met are `tests` and `layout_tests`, and
the fifty-seven settings tests crossed with the code.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical.
Thirty of those runs read a settings file, and `prefs`, `prefs_settings`,
`light_prefs`, `menu_settings`, `menu_tab`, `wrap`, `tabs3` and `light` are the
ones whose pixels depend on what was parsed -- a default, a bound or a keyword
that changed sides would have moved at least one of them. `cargo test --offline
--release`: 2,351 tests, 0 failed, the same total for the fifth pass running:
sixty-one tests changed crate and none was lost. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. The feature
reference and its PDF are untouched -- the settings file itself, its keywords and
its defaults are all exactly as they were, which is the point.

## The hundred-and-twentieth pass: what a row shows and what a key does to it

Phase 1, eleventh pass, and the first that is not a move: two functions, gathered
out of the window's dialog and written against `Settings` instead of against a
`Preferences`. `value_of` answers what a row shows on its right, and `adjust`
answers what Left, Right and Enter do to it. Between them they are every fact the
preferences dialog knows about a setting.

They had to be shared rather than copied, because the interesting part of them is
not the drawing. `adjust` is where the five step sizes and the ten bounds live:
autosave moves in five-second steps, the playground in whole minutes, tab width
between one and sixteen, the scale through a short list of named multipliers
rather than a number. Two copies of that would be two settings files that
disagree -- a terminal writing `ui_scale 1.25` where the window could never step
to it. `value_of` is the same argument one layer up: `12 min`, `x1.5`, `Follow
the display`, `Never` are strings a user reads in both programs.

The translation stayed outside. `value_of` takes a `&dyn Fn(&str) -> String` and
asks it for the *word* -- `On`, `Never`, `Edit...` -- not for the finished line,
so `30 s` keeps its unit and the number keeps its formatting. That also let the
terminal pass `|text| text.to_string()` and be done, which is exactly right for a
front-end with no catalogue yet. A whole-line translation was the first shape
tried and is wrong for a reason worth writing down: it would send `12 min`
through a dictionary as one string, and no translator would ever match it.

The window's two methods are now three lines each. Fifteen of its tests still
walk the shared functions through the dialog, so the behaviour is covered twice
over, and two new tests in `nn-shared::prefs` exercise them directly with the
identity translator -- one that a value reads the same either way, one that a step
stops at both ends.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical,
`prefs`, `prefs_settings` and `light_prefs` among them -- those are the dumps that
draw the values these two functions produce, so a changed string or a changed
bound would have altered at least one. `cargo test --offline --release`: 2,353
tests, 0 failed, the two new ones counted. `cargo clippy --offline --release
--all-targets`: 0 warnings, after the window's test module stopped importing the
bounds that are no longer its business. `cargo fmt` was not run. The feature
reference and its PDF are untouched.

## The hundred-and-twenty-first pass: the preferences dialog in the terminal

Phase 3, next pass, and the first one in this series that adds behaviour rather
than moving it. `crates/nn-tui/src/prefs.rs` is a box the shell draws over the
grid: seventeen rows in the order the window lists them, the same labels, the
same values, the same steps -- because `Row`, `value_of` and `adjust` are the
shared ones from the two passes before this. Up and Down move the lit row, Left,
Right and Enter step it, Escape closes it. Nothing in the file decides what a
setting is.

It has to be worth asking why the box exists at all in a program whose settings
file is text. The answer is the same one Midnight Commander gives for its own
dialog: a user who opens the window's preferences and the terminal's should not
be reading two different documents. Two of the seventeen rows are the window's
alone -- the font size and the UI scale -- and they are drawn with the reason
rather than hidden: "the terminal's font" and "follows the display". A row that
disappeared from one of the two lists would be a worse answer than a row that
says it does not apply here.

Those two rows are also where this pass found a bug worth writing down. The first
draft drew them as unhonoured and stepped them anyway, because `prefs::adjust`
knows nothing about which front-end is asking. Pressing Right on "the terminal's
font" then wrote `font_size 16` into the file the window reads: a key pressed in
a terminal would have resized text in a window. The fix is a guard on the match
arms -- an unhonoured row takes the key and does nothing with it -- and the test
that caught it was already written, expecting exactly that.

The dialog is modal the blunt way: `handle` returns before the editor sees
anything while the box is up, and the eleven keys it does not use are swallowed
rather than passed through. On close the app saves the settings and says so on
the status bar, which is the moment the file is worth writing and the moment a
user looks for confirmation.

The terminal's translator is `|text| text.to_string()`. There is no catalogue in
`nn-tui` yet; the first phase's last two passes bring one over.

### Verified

The window is untouched by this pass -- the only crate that changed is `nn-tui`,
which the windowed binary does not link -- and the gate was run anyway: `bench`
over the same thirty-one flag combinations, all thirty-one identical.

The dialog was driven, not eyeballed. `npp-tui` was started under a pseudo
terminal with a forty-row window and given the bytes a keyboard sends: `ESC [ 20
~` for F9, six `ESC [ C` for the six menus to the right of File, `ESC [ B` to open
Settings' dropdown on its first row, `\r` to pick Preferences. The stream the
program wrote contains `Preferences`, `Tab Width` and the values beside them. Two
runs of the same binary then answer the restart question the pass was written to
ask: with a settings file holding `theme light` and `tab_width 6` the box draws
`Theme    Light` and `Tab Width    6`, and with an empty `HOME` the same binary
draws `Theme    System` and `Tab Width    4`. Stepping Tab Width to 6, leaving and
reopening shows 6 again, and stepping the font size row leaves `font_size 16` in
the file.

`cargo test --offline --release`: 2,359 tests, 0 failed -- the 2,353 of the pass
before plus six for the dialog. `cargo clippy --offline --release --all-targets`:
0 warnings, after a test stopped asking for a `mut` it does not need. `cargo fmt`
was not run. The feature reference and its PDF are untouched.

## The hundred-and-twenty-second pass: the translations

Phase 1, twelfth pass, and the last of the two things the terminal was waiting on
before it could draw a menu bar in French: `i18n` left `nn-app` for `nn-shared`,
whole, with its eight hundred lines of catalogue, the twenty-three bundled
localizations, the two-schema reader and the language list.

It moved for the reason every other module in this series moved, said about words
instead of types: a translation is a promise about a *word*, and both front-ends
draw the same words. A terminal that read `french.xml` its own way would show
"Fichier" beside the window's "Fichier" -- or, worse, fall back to English where
the window does not, and a user comparing the two would conclude that one of the
programs was broken.

The file crossed with one change and one leaving. `is_literal` asked
`nn_shared::theme::ThemeChoice::ALL` from the other side; it now asks `crate::`.
And the coverage test -- "no bundled language may leave a word of the window in
English" -- stayed behind, in `nn-app`, because the list it walks is
`Preferences::labels()`, `StatusBar::labels()` and six more, and those widgets are
the only thing that knows what the window draws. `nn-app/src/i18n.rs` is now a
documentation comment, one `pub use` of six items and that single test, which is
the shape the boundary takes when the *data* is shared and the *inventory* is not.

The bundled files did not move. `include_str!` resolves relative to the source
file, and `crates/nn-shared/src/i18n.rs` sits at the same depth as
`crates/nn-app/src/i18n.rs` did, so all twenty-three paths still read
`../../../localization/…` and the repository keeps one `localization/` directory
rather than a copy in each crate.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical. That
is the whole argument for a move of this kind -- the dozens of strings this module
produces are drawn in `prefs`, `light_prefs`, `menu_settings`, `menu_run`,
`menu_edit` and the rest, so a catalogue that read the same files differently
would have altered at least one picture. `cargo test --offline --release`: 2,359
tests, 0 failed, the same total as the pass before: the twenty-two i18n tests
changed crate, and the coverage test merely changed sides of the boundary.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched -- and so are the twenty-three
localization files themselves.

## The hundred-and-twenty-third pass: the terminal speaks the user's language

Phase 3, next pass, and the first the terminal's user can see: `npp-tui` now reads
its words through the catalogue that crossed to `nn-shared` in the pass before.
The bar's thirteen titles, every row of every dropdown, the preference dialog's
title, its seventeen labels and the words on their right -- `On`, `Never`, `Edit…`
-- all come through one lookup, so a user who sets `language french.xml` in a
terminal sees `Fichier` where the window beside it sees `Fichier`.

Both widgets do the same thing with it, and it is worth naming because it is the
whole of the change: a label is translated *where it is drawn and where it is
measured*. A dropdown's width comes from its longest row, so a translated label
that were drawn translated and measured in English would leave a French menu with
its arrows and shortcut column in the wrong place -- wrong by exactly as much as
"Largeur de tabulation" is longer than "Tab Width". `Menus` therefore asks `tr` in
four places: the titles, the label width, the drawing, and the column the box is
placed at, which is computed from the box before it. `Prefs` asks in three: the
label column's width, the drawn label, and the value, where the word is what goes
through the catalogue and the number keeps its unit.

The Language row stopped being a placeholder. It steps the list the shared
catalogue reports -- the twenty-three bundled files plus anything dropped in the
configuration directory -- with English first and every entry under its *English*
name, so a reader looking for Spanish finds it under `S` whatever it calls itself.
The window's dialog is built the same way from the same list.

When the language is read is a decision and not an accident. `apply_language` runs
once at startup, before the first frame, and again when the preferences dialog
closes -- not on every key. The window retranslates live while its picker is open;
this dialog steps its rows with the arrow keys, and a list that rebuilt itself
under the finger would move the entry being stepped towards. A file that will not
parse leaves the terminal in English and says so on the status bar, which is what
the window does for the same reason: a cosmetic fault must not cost the user the
program.

One derive had to go. `Prefs` was `Copy`, with a bool and an index in it; a
catalogue is a `HashMap` and a list of languages is a `Vec`, so it is `Clone` now,
and the field that made it `Copy` is the field that made it worth sharing.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical, which
is what this pass is entitled to claim and no more -- `nn-tui` is not linked into
the windowed binary, so the gate is re-run for the standing rule rather than for
evidence.

The terminal was driven through a pseudo terminal, twice with the same bytes and
different settings. With `language french.xml` in the settings file, F9 and six
Rights put `Fichier` at the left of the bar, and F9, six Rights, Down, Enter draw
the box titled `Préférences` with `Thème` and `Largeur de tabulation` in it. With
no `language` line the same bytes draw `Preferences`, `Theme` and `Tab Width`.

`cargo test --offline --release`: 2,362 tests, 0 failed -- 2,359 plus three: a
catalogue through the dialog's title, labels and words, a catalogue through the
bar's titles and rows, and the Language row stepping a list that was handed to it.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched.

## The hundred-and-twenty-fourth pass: the dialog's values reach the screen

Phase 3, next pass, and the one that turns the preference dialog from a thing that
writes a file into a thing that changes what the user is looking at. Two rows do
it: the theme and the tab width -- the two the screen was hard-coding.

The theme first, because it is the one that touches every cell. `Tone::sgr`
returned a `&'static str`, one per tone, which is exactly what a hard-coded
palette can offer and exactly what a theme cannot. It is now `Palette::of(theme)`,
eighteen escapes built in one go, and `Palette::sgr(tone)` is a slice of that. The
shape matters for what the terminal was built to be measured on: the renderer
writes one escape per run of cells in the same tone, so a palette lookup is a
lookup and nothing else -- no colour is formatted per cell, and the eight
allocations happen when somebody changes theme and at no other time.

`Frame` gained the palette and a setter that compares. A cell carries a tone and no
colour, so a new theme changes every cell on the screen without touching one of
them -- which means the diff the renderer computes would find nothing to write and
the old colours would stay. `set_palette` therefore marks the frame fresh, and the
next frame is written edge to edge. That is the one place where being cheap about
bytes and being correct disagree, and it happens once per theme.

The colours come from the shared theme, resolved by the shared code --
`catalogue::theme_of` then `theme::effective_theme` -- so the sixteen `style`
overrides a user can write by hand work in a terminal too, without a second
implementation of what an override means. `System` resolves with `None`, which the
window passes when the desktop will not say: a terminal has nobody to ask, so
`System` is dark here, and the doc comment says so rather than leaving the next
reader to guess.

The tab width was a `const TAB: usize = 4` in the shape of an `advance` function
that took a column and a character. It now takes the setting, which is the same
number the window hands its own view, and the constant is gone. The rule is
unchanged -- a tab advances to the next stop rather than by a width -- which is why
this is one extra argument and not a rewrite of the layout.

Both are applied by one function, `apply_settings`, called before the first frame
and again when the dialog closes; it is the language pass's `apply_language` with
the theme beside it, for the same reason: the display is built once from what the
settings say, not looked up per cell.

One test is worth confessing to. `the_theme_the_settings_name_is_the_one_the_screen_uses`
first took its "before" palette from the app it had just built -- which reads the
settings file of whoever runs the tests, and on this machine that file says Tokyo
Night, so the assertion compared Tokyo Night to Dark and failed. Both sides are now
set explicitly. A test that reads the developer's configuration is a test that
passes on the machine where it was written and fails on the reviewer's.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical. Only
`nn-tui` changed and the windowed binary does not link it, so this is the standing
rule rather than evidence -- and this pass is the first in the series where that
sentence is worth repeating, because a palette is the kind of change that could
plausibly have reached the window through `nn-shared::theme`.

The terminal was driven through a pseudo terminal with the same document and two
settings files. With `theme light` the bytes it wrote carry the light page --
forty-three backgrounds of `48;2;255;255;255` and forty of `48;2;240;240;240` for
the gutter -- and with no theme line the same run carries `48;2;30;30;30` and
`48;2;37;37;38` instead.

`cargo test --offline --release`: 2,366 tests, 0 failed -- 2,362 plus four: the
palette's two (every tone gets an escape that sets both colours, and the palette is
the theme's colours), the tab width reaching the grid at eight columns, and a frame
that starts the picture again when the palette changes. `cargo clippy --offline
--release --all-targets`: 0 warnings. `cargo fmt` was not run. The feature
reference and its PDF are untouched.

## The hundred-and-twenty-fifth pass: the lists behind the two long rows

Phase 3, next pass: the Theme and Language rows stopped pretending to be switches.
`Row::opens_picker` has said since the shared rows crossed to `nn-shared` that
these two are not stepped but listed -- fifty themes and two dozen languages, in a
box you read -- and the terminal was the front-end ignoring it, stepping fifty
values with the arrow keys instead. Enter on either row now opens a list.

The list is the dialog's own, not a second thing: `Picker` holds the row it belongs
to, what it offers, and which entry is lit; it is titled with the row's label, it
opens on the value in force rather than at the top, and Enter writes the choice
into the settings through the same `ThemeChoice::ALL` and the same language list
the rest of the dialog uses. Escape closes it and writes nothing, which is what
makes it safe to open one to look -- a list you cannot browse without committing to
is a list people learn not to open.

The keys are the interesting part, and the shape they took is the shape a modal
takes: while a list is up, every key goes to it and the dialog behind it does not
move. That is the same rule the dialog itself follows over the document, one level
down, and it took a test to notice that I had got it wrong -- splitting "Right or
Enter" into "Enter opens a list" and "Right steps" quietly dropped Enter from the
fifteen rows that have no list, so Enter on Word Wrap stopped toggling it. The
existing test for the font size row did not catch it, because that row is one the
terminal refuses; the new one does.

Two smaller things came with it. The hint line is now the window's own sentence --
`Up/Down  Left/Right  Enter open  Esc close` -- which the catalogue already
carries, so the last English line in a French dialog is gone and pass 123's
apology about it is spent. And the box border moved into `draw_box`, because there
are two boxes now and a duplicated corner is a corner that drifts.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical; only
`nn-tui` changed.

Driven through a pseudo terminal: F9, six Rights, Down, Enter, Enter opens the
list on a fresh configuration, and the twelve rows it draws include `System`,
`Ayu Dark` and `Catppuccin Mocha`. One Down and Enter, then Escape, and the
settings file the run wrote says `theme light` -- the second entry of the list,
chosen through it rather than stepped to.

`cargo test --offline --release`: 2,372 tests, 0 failed -- 2,366 plus six: the
list opens on the row's values, a choice is written and the dialog stays up, an
Escape writes nothing, the language list offers English first and choosing it
writes `None`, the list is drawn over the dialog, and Enter still steps a row that
has none. `cargo clippy --offline --release --all-targets`: 0 warnings, after one
test stopped assigning a field of a `Default::default()`. `cargo fmt` was not run.
The feature reference and its PDF are untouched.

## The hundred-and-twenty-sixth pass: the chords and the commands

Phase 3, next pass, and the one the plan called the one dispatch: a keystroke is
now a *chord*, the chord is looked up in the keymap the settings carry, and the
action it names is run. Five hardcoded chords left the shell -- `Ctrl+S`, `Ctrl+Q`,
`Ctrl+Z`, `Ctrl+Y`, `Ctrl+A` -- and the seventy-four bindings the default keymap
carries arrived in their place, because the keymap is the window's list and the
terminal was reading a private one.

The lookup is deliberately narrow and the reason is worth stating: only chords and
function keys are asked about. A character is text, and the arrows, Home, End and
the two paging keys are navigation this shell answers itself. A binding on one of
those would take typing away from a user who rebound a command to a letter without
thinking about it, and the person it would surprise is the one who did it.

`Ctrl+Q` stayed the shell's own, beside F9. The window binds `exit` to
`Ctrl+Shift+Q`, and a terminal cannot send that: `Ctrl`, `Shift` and a letter is
not one of the bytes a keyboard puts on the wire, so a shell that could only be
left with the window's chord would have no way out. Every terminal editor quits on
`Ctrl+Q` for the same reason it uses `Ctrl+S` to save, and the quirks list in this
series now has three entries: `F9` for the bar, `Ctrl+Q` to leave, and the note that
neither is a second implementation of anything.

Then the commands. Five are the line operations the engine already had --
duplicate, move up, move down, delete, select the line -- and the other thirty-four
are one arm, because they are one table over in the engine: `is_text_command` says
whether `nn_core::ops::run` knows the name, and the shell hands it the selected
lines as text and the same `Options` the window passes. Upper case, lower case,
indent, unindent, the five sorts, dedupe, trim, join, the comment commands, the
case conversions: one arm and two options. That is what the extraction was for.

Two facts about the engine came out of writing the tests, and both are worth
having written down because they are the sort of thing a front-end author guesses
wrong. `Indent` inserts a tab character, not `tab_width` spaces -- the column a tab
stands for is a display question and the file keeps the character. And an edit
replaces the lines it was given, so the selection is gone afterwards: a test that
ran four text commands over one selection was really running three of them over a
caret, and a caret means "the caret's line", in this shell as in the window.

The suite also stopped reading the machine it runs on. `app()` in the tests now
resets the settings to their defaults before the first frame, because the keymap
comes out of the settings file and `Ctrl+Q` had begun to mean whatever the
developer's own file said it meant. That is the second time in two passes that a
test written against the real configuration failed for being right; the pattern is
now written down here twice.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical; only
`nn-tui` changed.

Driven through a pseudo terminal: a file holding `x = 1` was opened, `Ctrl+D` sent,
then `Ctrl+S`, and the file on disk afterwards holds the line twice -- two
keystrokes, one keymap lookup each, one save. The shell's help was updated to say
where the chords come from and that a chord bound to a command the shell has not
got says so on the bar.

`cargo test --offline --release`: 2,377 tests, 0 failed -- 2,372 plus five: a
chord running the command the settings bind it to, an unbound chord doing nothing,
the text commands over a selection, the line commands moving and deleting, and a
command the shell has not got still saying so. `cargo clippy --offline --release
--all-targets`: 0 warnings. `cargo fmt` was not run. The feature reference and its
PDF are untouched.

## The hundred-and-twenty-seventh pass: the find bar

Phase 3, next pass, and the first of phase 4's widgets to arrive early: the shell
can search. `crates/nn-tui/src/find.rs` is a module of its own, the way the menus
and the preferences dialog are, and it holds the pattern, the two marks that change
what a pattern finds, the hits and which hit the bar is standing on.

The windowed bar carries eleven chips on a row of their own. A terminal row is a
much smaller thing, so this one keeps the two marks that decide what a search
*finds* -- match case and whole word -- and leaves regex, extended, wrap, in
selection, direction, bookmarks and the eight buttons to the passes that bring
replace and find-in-files with them. What it does keep is the window's wording for
the count, `3/17` and `no match`, so the two front-ends say the same thing about
the same pattern rather than each inventing a sentence.

The keys are Midnight Commander's, because a terminal has no mouse to click a chip
with: Tab moves between the field and the two marks, Space or Enter flips the mark
the focus is standing on, Enter or Down in the field walks the matches forward and
Up walks them back, and Escape closes. A mark is a thing you stand on and press,
which is the same rule the preference dialog follows one pass earlier. There is no
`Alt` in the key parser and inventing one for two marks would have been the tail
wagging the dog.

The marks and the counter are pinned to the right edge, the field takes what is
left, and the counter keeps a fixed nine cells whatever it says, so that a count
going from `0/3` to `no match` does not shove the marks about while a pattern is
being typed. A pattern longer than the room keeps its head and loses its tail; the
column a terminal would wrap on is never written.

Closing is not cancelling: Escape drops the hits and keeps the pattern and the
selection, so the caret stays on the match the last Enter walked to and `F3` works
afterwards. Dropping the hits on close is what makes `F3` honest -- it searches the
document as it is now rather than as it was when the bar was up, which matters
because the shell does not track edits against a hit list.

The performance rule is the interesting part, and it is the one this project would
want written down even if nothing else were. A search is a pass over the whole
document, so a keystroke that runs one is a keystroke a reader feels in a large
file. `refresh` therefore searches on every letter only while the document is small
-- the same `is_large` the highlighter and the window use -- and otherwise marks
the hits stale and waits for Enter -- or `F3` once the bar has closed -- to force
the pass. The bar says `press Enter` in that state rather than `no match`, because
saying "no match" before searching would be a claim it has not earned. The hits are
capped at ten thousand: a one-letter pattern in a large file matches everywhere,
and the count is there to tell a reader whether to keep typing, not to number them
all.

The bar also hands back the keys it does not want, which is what keeps it a row
rather than a mode: `Ctrl+S` saves and `Ctrl+Q` quits with a pattern half typed,
and a keystroke it ignores reaches the shell exactly as if the bar were not there.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical; only
`nn-tui` changed.

Driven through a pseudo terminal, no X server involved. `Ctrl+F` drew `Find:`, the
caret block, `Aa`, `W` and the counter; typing `alpha` in a two-match file made it
read `0/2`; Enter selected the first match with the bar still up and the counter
went to `1/2` (the stream carries the one changed digit, `39;139H1`, which is the
diff writer doing its job); a second Enter put the selection on the third line and
the bar read `2/2` with the status line `Ln 3, Col 11`. Escape repainted row
thirty-nine from the bar to text -- the document got its row back -- and `F3` then
stepped to the first match on its own. In a file of `Alpha alpha ALPHA`, Tab moved
the focus onto `Aa` (the mark redrawn in the caret's colours and nothing else
written) and Space flipped it: the count went from `0/3` to `0/1`, one cell. Tab
again and Space on `W` with the pattern `alph` gave `no match`, which is what the
whole-word rule means there. Tab never edited the document: the file was saved
afterwards and had not gained a space.

`cargo test --offline --release`: 2,384 tests, 0 failed -- 2,377 plus seven: a
pattern counted and walked with wrap in both directions, the two marks changing
what is found, Escape closing on the match and `F3` then searching afresh, an empty
pattern counting nothing and a chord going back to the shell, the marks and the
counter holding their columns as a pattern outgrows the room, the bar taking a row
from the text and giving it back, and `F3` walking matches with the bar closed
while `Ctrl+S` still saves. `cargo clippy --offline --release --all-targets`: 0
warnings. `cargo fmt` was not run. The feature reference and its PDF are untouched.

## The hundred-and-twenty-eighth pass: the setting that says long lines wrap

Phase 3, next pass, and the last of the settings the screen was not honouring.
`settings.wrap` has been in the file, in the preferences dialog and in the View
menu since the settings were extracted; the terminal drew a line as one row and
cut whatever did not fit at the right-hand edge, whatever the setting said.

A line now takes as many rows as it needs. The number is written on its first
row and no other, so the strip reads as one line rather than as several wearing
the same number, and a continuation row keeps the text strip's tone. The caret
is a cell, and a line is one cell wider for it: a line that fills its rows
exactly gets one more for the block that follows its last character, which is
the case where the block would otherwise fall off the bottom.

The state that had to grow is the scroll. `scroll_line` was the whole of it
while a line was a row: the top of the screen was a line, and the caret was on
it or below it. That is no longer true -- the top of the screen can be the
middle of a line -- so two fields came with the change. `scroll_seg` names the
segments of the top line above the screen, which is what holds a caret sitting
deeper in a line than the window is tall. `visible_until` remembers the first
line the last frame had no room for, because with wrapping on the row a line
lands on is the sum of every line before it, and adding that sum up per frame is
a walk over the document that the frame before this one has already done. A
caret that leaves the bottom moves the view by the one line the last frame ran
out of; a caret that jumps ten thousand lines puts its own line at the top.

The row arithmetic is one function, `cell_of`, answering where a display column
goes: a segment and a remainder with wrapping on, a subtraction against
`scroll_col` with it off. Both the drawing and the caret block ask it, so there
is no second opinion about where a character is. With wrapping off nothing about
the picture changed -- the same cells, the same bytes, the same 306-byte
keystroke in 69 microseconds that pass 126 measured -- which is what the switch
being off has to mean.

A tab is drawn cell by cell already, and that turned out to matter here: a tab
can straddle the column a wrapped row ends on, and a run of cells that is placed
on its own crosses the boundary the way the rest of the line does. A wide glyph
sitting exactly on the boundary is dropped instead of being torn in two, which
is what the terminal's own last column does with one.

`Action::ToggleWrap` is no longer one of the commands the shell says it has not
got: it flips the setting, writes it, and puts the scroll back to the caret's
line. Two roads reach it -- the View menu's Word wrap row, and the preferences
dialog's Wrap row, which has been there since pass 121. A third would be the
chord the settings bind to it, `Ctrl+Alt+W`, and that is the one a terminal
cannot send: this parser reads a control byte as Ctrl and a letter, and Ctrl,
Alt and a letter do not fit in the bytes a keyboard puts on the wire. That is
the fourth entry in the quirks list, and it is the reason the note below drives
the menu rather than the chord.

One thing was deliberately left to a later pass, and it belongs to the engine
rather than to this shell. Up and Down still move by document line, and Page Up
and Page Down by `viewport.height` lines, because those are `nn-core`'s motions
and the window shares them. With wrapping on that means Down leaves a long line
at its first continuation row rather than walking down through the rest of it.
Moving by visual row needs the goal column kept across a wrapped motion, which
is engine work that both front-ends get -- the window's own wrapped motion comes
from the shaper, so there is nothing there to reuse -- and it is a pass of its
own. The shell says so in the note rather than guessing at half of it.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

Driven through a pseudo terminal. A document holding one 300-character line and
a short one was opened in a fresh home and on a 140-column screen: with the
setting off the long line was cut at the edge and the short line was one row
below it; F9, three Rights, eleven Downs, Enter on View's Word wrap wrote `wrap
on` to the settings file, said `long lines wrap`, and the next frame drew the
same line as three rows. LineEnd on it put the caret block on the third row at
column 27 -- the twenty-fifth column of the text area, which is where the 301st
character of the line falls -- with the status reading `Ln 1, Col 301`; with
wrapping off the same key scrolled the line sideways and put the block against
the right-hand edge instead. The preferences dialog's Wrap row reached the same
setting and wrote the same line. Thirty lines of three hundred characters with
twenty Downs after them drew no panic and no lost caret.

`cargo test --offline --release`: 2,388 tests, 0 failed -- 2,384 plus four: a
long line wrapping onto the row below and being cut when the setting is off, a
wrapped row carrying the caret and the tone of its text, a line taller than the
window scrolling to hold the caret, and a walk down and back up a document where
every line is six rows tall. The keystroke measurement still prints 15,554 bytes
for the first paint, 306 for a keystroke, in 69 microseconds. `cargo clippy
--offline --release --all-targets`: 0 warnings. `cargo fmt` was not run. The
feature reference and its PDF are untouched.

## The hundred-and-twenty-ninth pass: a row at a time

The last pass ended on a note it could not answer: Up and Down moved by document
line, so with wrapping on a Down left a long line at its first continuation row
instead of walking down through the rest of it, and a page moved a screenful of
lines that might be three screens tall. This pass takes the note up.

The motion stayed in the shell, which is the opposite of what the note guessed.
The note said it belonged to `nn-core`, where both front-ends would get it, and
the reason it does not is a disagreement about what a row is. The document's row
model counts characters -- `set_wrap`, `row_of_line`, `segments_of_line` all
divide a character index by a width -- while the terminal's drawing counts cells,
and a cell is not a character: a tab covers one to eight of them, a wide glyph
two. A motion written against character rows would put the caret a row away from
where it is drawn in any file holding a tab, and the window has the same
disagreement from the other side, because its rows come from the shaper. So the
document keeps its character rows, the shell walks rows of cells, and the two do
not meet until the document's model counts display columns -- a change to make in
`nn-core` on the day both front-ends can be moved together, not one of them now.

The walk is `move_by_rows`, and it is deliberately short. It takes the caret's
line and the row inside it, steps one row, crosses a line boundary at either end,
and lands on the display column it was aiming for; `char_at_display` answers
which character stands at a display column, and `row_count` how many rows a line
takes once its tabs are expanded. The column it aims for is `desired_row`, which
is a single number on `App` that a run of vertical moves keeps and every other
key clears: Down, Down, Up therefore comes back to the column it started on,
rather than drifting left every time a short line interrupts the run. A page is
the same walk repeated once per row of the text area -- not once per line, which
is what it used to be and what a page has to mean when a line is three rows tall.
With wrapping off, and with more than one caret, the key goes to the core's own
motion, because there a row is a line and there is nothing to compute.

Counting the rows of a line is a pass over that line, which is the cost this pass
had to watch. The count is taken once per line the walk crosses, not once per row
it steps, so a page down a document of three-hundred-character lines counts
thirteen lines rather than thirty-nine, and a Down inside one line counts that
line twice
-- once to find the row it is in, once for the move. The keystroke measurement in
the tests grew a second number for it: 15,554 bytes for the first paint and 306
for a keystroke, unchanged, and a Down across wrapped rows in 8.6 microseconds
against the 48.5 a keystroke's own redraw takes -- a fifth of a frame, which is
the same order the line-based motion had. The bound that remains is worth saying
out loud: a Down on a line of a million characters costs a pass over that line,
exactly as the window's shaper does when it lays the same line out. Making that
go away is the same engine change as making the rows display columns, and it will
arrive with it.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

Driven through a pseudo terminal, on a 140-column screen, over a document holding
one 300-character line above a short one, from a fresh home with wrapping on. At
the start the status read `Ln 1, Col 1`; one Down put the caret on the second row
of the line, `Ln 1, Col 139`, with the digit 8 of the line under the caret block;
a second Down, `Ln 1, Col 277`, on the third row; a third, `Ln 2, Col 1`, on the
line below; two Downs and an Up, `Ln 1, Col 139`, the column it had before. Thirty
lines of three hundred characters: Page Down read `Ln 14` with line 14 at the top
of the gutter, Page Down again `Ln 27`, Page Up back to `Ln 1, Col 1`. With
wrapping off the same Down was the core's and moved a line.

`cargo test --offline --release`: 2,393 tests, 0 failed -- 2,388 plus five: a
walk down the rows of one wrapped line, the column inside a row surviving a short
line, an Up from a line's first row arriving on its last row, a page walking rows
and a tab covering its cells, and a vertical move still being a line with wrapping
off; the sixth test to change is the keystroke measurement, which now measures a
wrapped Down as well. `cargo clippy --offline --release --all-targets`: 0
warnings. `cargo fmt` was not run. The feature reference and its PDF are
untouched.

## The hundred-and-thirtieth pass: the bar that had lost its arrows

Pass 129 gave the shell a vertical move of its own and put it where it had to be
-- before the find bar, because a key that moves the caret is a key the text area
answers. The bar is asked after it, so with wrapping on its Up and Down never
arrived: a reader with a pattern typed and a long line in front of them pressed
Down to walk to the next match and watched the caret step a row down the line
instead.

The fix is one clause. `move_by_rows` declines while the bar is up, because a bar
that takes those two keys is a reason to decline rather than a reason to move by a
row; the order in `handle` is unchanged, which is the point -- the guard belongs to
the move, not to the shell's dispatch, and neither the menu nor the preferences
dialog needs one, since both are answered before it.

The test that catches it draws a frame first. Without one the move has no width to
divide by and declines for the wrong reason, which is how the first version of the
test passed while the bug was still there; the numbers it now asserts are the two
matches a Down walks to, `201` and `409`, against the `38` a row of the same line
would have landed on.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

`cargo test --offline --release`: 2,394 tests, 0 failed -- 2,393 plus one: a
wrapped document with two matches on one line, Down twice, the selection at 201
and then 409. Run against the guard removed, the same test fails with 38, so it is
the guard it is testing. `cargo clippy --offline --release --all-targets`: 0
warnings. `cargo fmt` was not run. The feature reference and its PDF are
untouched.

## The hundred-and-thirty-first pass: the replacement and its two commands

The find bar grew a second row: the replacement, and the two commands the window
puts beside it. `Ctrl+H` is what opens it in the window and what opens it here,
except that a terminal does not deliver that key -- `0x08` is what a keyboard puts
on the wire for it, and this shell's parser has read that byte as Backspace since
the first pass, as every terminal editor does. So the road is the menu, Search's
`Replace...` row, which is the fifth entry in the quirks list: a chord the window
has and the terminal cannot send, with a menu row that reaches the same command.

The row holds a field and two words. Tab reads down the bar -- the pattern's row
first, then the replacement's, because that is the order they are drawn in -- so
it walks Query, the two marks, the replacement, `Replace`, `Replace All` and back
round. A mark has always been a thing you stand on and press, and a command is the
same thing with a word instead of a letter: Space or Enter presses it, and while
Tab stands on one the whole word is drawn in the caret's colours. The words are
the window's, and they are measured where they are drawn rather than assumed to be
seven and eleven cells wide, because a translation is free to be longer; a window
too narrow for them keeps the field and drops the two commands instead of writing
them over each other.

Enter is what the focus is standing on, which is the window's own rule: in the
pattern field it walks on, in the replacement field it replaces the match under
the caret and walks on, on a command it presses it. `Ctrl+R` does the same from
anywhere on the bar, because the window answers that chord inside its bar too and
a terminal can send it. Two of the window's three commands are here: `Replace`
and `Replace All` have somewhere to stand, and `Replace All in All Opened
Documents` has not, since this shell has one document rather than a strip of tabs.
That one arrives with the tabs.

`Replace All` deliberately ignores the count's ceiling. The bar stops counting at
ten thousand matches because a count is there to tell a reader whether to keep
typing; a command that says it replaced everything and stopped at the ten
thousandth match would be lying, so it asks the document for every match and
hands them to the editor as one list. The editor records one edit for the whole
list, which is what makes a single `Ctrl+Z` take the whole replacement back --
and why `Replace All` over a large file is one undo group, not a thousand.

The bar edits through the editor rather than through the shell, so the shell would
never have heard about it: the frame would have kept the colours of the text that
was replaced. One flag and one line close that hole -- `take_replaced`, read at
the one place that hands a key to the bar -- and the line is re-lexed next frame.
`Replace` costs one pass over the document per press, because it recounts the
matches afterwards; the window's does the same, and it is a pass the reader asked
for by pressing. Typing in either field still searches nothing.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

Driven through a pseudo terminal on a 140-column screen, from a fresh home, over a
file holding `one two one` and `three one`. F9, Right twice to Search, Down onto
its first row, eight Downs to `Replace...`, Enter: the bar took two rows, the
document gave one up, and the frame read `Find:` above `Replace:` with the two
commands at the right-hand end of the second row. Typing `one` counted `0/3`. Tab
three times, `1`, Enter: line 1 became `1 two one`, and the counter moved on to
`1/2`. A second Enter: `1 two 1`. A fresh run to `Replace All` -- Tab twice more,
Enter -- left `1 two 1` and `three 1` with the counter saying `no match`.

`cargo test --offline --release`: 2,400 tests, 0 failed -- 2,394 plus six: the
replace row and the order Tab walks it, Enter replacing the match under the caret
and moving on, `Replace` and `Ctrl+R` doing the same, `Replace All` as one undo
step, the second row drawn under the first with the commands measured against the
right edge, and the shell growing the bar by a row and redrawing the line it
replaced. The keystroke measurement is unchanged at 15,554 bytes for the first
paint and 306 for a keystroke, and a Down across wrapped rows still costs about
ten microseconds. `cargo clippy --offline --release --all-targets`: 0 warnings.
`cargo fmt` was not run. The feature reference and its PDF are untouched.

## The hundred-and-thirty-second pass: Go to line, and the field it opens in

The window has one prompt widget and every command that wants a line typed before
it runs goes through it: a label, the text after the label, a caret. The shell had
none, so `Go to line` -- the one command it is missing whose whole answer is a
number -- was the reason to build one. The label is a table entry and the field is
the same field for every entry, because `Open`, `Save As` and `Find in Files` want
the same row with a different question in front of it; those arrive as rows of the
same list rather than as three more widgets.

`Ctrl+G` gets there, and it does arrive: `0x07` is what a keyboard sends for it and
this parser reads that byte as Ctrl and a letter, which is why this chord is one
the quirks list does not have to excuse, unlike `Ctrl+H` for replace and `Ctrl+Alt`
for wrap. The field takes the editing keys -- a character where the caret stands,
Backspace, Delete, Left, Right, Home, End -- and it is modal while it is up, as the
window's is: a letter belongs to the prompt rather than to the document behind it,
and a chord is dropped rather than passed on, so `Ctrl+Q` cannot leave from under a
half-typed line number.

Enter runs what was typed and Escape leaves without running anything. The number
is counted from one, as the status bar counts, and clamped by the editor at both
ends: `99` in a thirty-line file arrives on its last line, `0` on the first, and a
word arrives nowhere -- the window runs nothing for a line number that is not a
number, and the shell keeps that silence rather than improving on it, because two
front-ends that disagree about when to be quiet are two front-ends to learn twice.

One row, one owner. The strip above the status line used to belong to the find bar
and nothing else, so `Layout` called it `find_row`; it is now `bar_row`, and its
owner is the prompt while the prompt is up and the find bar otherwise. That is the
whole of the layout change, and it is why a frame with a prompt up shows one line
fewer of the document and gets it back when the prompt closes.

Nothing here costs a keystroke anything: the field edits a `String` of three
characters, no search runs, no line is re-lexed, and the row that was already
being painted carries a different label. The next pass that wants a whole
document pass -- find in files -- will be the one to watch, not this one.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

Driven through a pseudo terminal on a 140-column screen, from a fresh home, over a
file of thirty numbered lines. `Ctrl+G` drew `Go to line:` on the row above the
status line with the status still reading `Ln 1, Col 1`; `12` appeared after the
label as it was typed; Enter closed the prompt, moved the status to `Ln 12, Col
1`, and line 12 was drawn as `12 line 12` -- and the prompt's row went back to
the document. A fresh run with Escape instead of Enter left `Ln 1, Col 1`; typing
`abc` and pressing Enter left it there too; `99` arrived at `Ln 31, Col 1`, which
is the empty line a file ending in a newline has at its end.

`cargo test --offline --release`: 2,406 tests, 0 failed -- 2,400 plus six: a line
number counted from one, a number past the end arriving on the last line, a word
and an Escape running nothing, the field's editing keys with a chord dropped
rather than passed on, the label and the caret holding their places on the row,
and the shell opening the prompt on `Ctrl+G` and putting the caret on the line
named.
The keystroke measurement is unchanged at 15,554 bytes for the first paint and 306
for a keystroke. `cargo clippy --offline --release --all-targets`: 0 warnings.
`cargo fmt` was not run. The feature reference and its PDF are untouched.

## The hundred-and-thirty-third pass: Open and Save As

The shell could read one file, the one named on its command line. `Open` and
`Save As` are the two rows of the File menu a reader reaches for first, and both
said `not in the terminal yet`. They are here, and they are what the prompt of the
last pass was built for.

The prompt changed shape on the way. It used to run its own command, which meant a
field that held an editor; it now hands the answer back -- `take_answer`, read once
-- and the shell runs it. That is why `Go to line` moved into `run_prompt` beside
the two new ones: what a path means, what a number means, and what the bar says
afterwards are all questions for the piece that can open a file and has a status
line, and keeping them out of the widget is what lets the widget be tested without
a document.

`Ctrl+O` arrives. `Ctrl+Shift+S` does not: Ctrl and Shift and a letter is not one of
the bytes a keyboard puts on the wire, so `Save As` is the File menu's row, and that
is the sixth entry in the quirks list. `Open` makes the path absolute before it
touches it, because a relative name means a different file the next time the shell
is started from somewhere else -- the window learned that, and
`nn_core::paths::absolute` is what it was learned into.

Opening is refused while the document in front has unsaved changes. The window
opens a second tab and loses nothing; this shell has one document, so the honest
answer is the bar's: `unsaved changes: Ctrl+S saves them first`. That refusal is
what the tabs pass will lift. A name that is not there is not an error, though: it
opens a blank page for that name, whose first save creates the file. That is
`Document::open`'s rule, the window's rule, and now the command line's too --
`npp-tui notes.txt` in a directory without `notes.txt` used to print an error and
exit, which was the one place where the two front-ends disagreed about a name they
had both been given.

`Save As` writes, adopts the path, clears the mark and swaps the lexer with the
extension, because a buffer saved as `x.json` should colour itself as JSON from the
next line on and not keep the colours of the name it used to have. Everything that
described the old document -- its scroll, the remembered row, the half-armed quit
warning -- is dropped in one place, `load`, which `with_document` now goes through
as well, so "what belongs to the document that was open" is a list written once.

The cost of both commands is a file read and a file write, once each, and one
highlighting pass for the document that replaces the old one. Nothing here is paid
per keystroke: the field edits three characters, and the frame it draws is a row
that was already being painted.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

Through a pseudo terminal on a 140-column screen, from a fresh home, on a file
holding one line `first`. `Ctrl+O` drew `Open:`, a typed `/tmp/second.txt` and Enter
put that file on the screen as three lines and the bar said `opened
/tmp/second.txt`. Typing a character first and then asking for the same file said
`unsaved changes: Ctrl+S saves them first` and left the buffer alone. F9, Down onto
the File dropdown's first row and eight Downs found `Save As...`: the prompt read
`Save as:`, `/tmp/saved-copy.txt` and Enter wrote the six bytes of `first` to disk
and said `saved /tmp/saved-copy.txt`. Started on a name that was not there, the
shell drew an empty first line instead of printing an error; typing `hello` and
`Ctrl+S` created the file with those five bytes in it.

`cargo test --offline --release`: 2,410 tests, 0 failed -- the prompt's five became
four, since the widget no longer runs anything, and the shell gained five: `Ctrl+O`
opening the file that was typed, opening refused while the document is dirty, a
name that is not there opening a blank page, `Save As` writing the file and adopting
its name, and the language following the extension -- the last one proven by the
tone of `fn`, which is a keyword under the code lexer and is not under the JSON
one. The keystroke measurement is unchanged at 15,554 bytes for the first paint and
306 for a keystroke. `cargo clippy --offline --release --all-targets`: 0 warnings.
`cargo fmt` was not run. The feature reference and its PDF are untouched.

## The hundred-and-thirty-fourth pass: New, and reading the file again

Two more rows of the File menu, and they are the last two that cost nothing but a
call the shell already had. `New` starts a blank page with no name, which is
`Document::new` and `load` and nothing else. `Reload from Disk` reads the file
again with `Document::from_file` -- not `Document::open`, because a file that has
been deleted since it was opened is an error worth reporting rather than a blank
page to hand back.

The difference between the two is what they do with unsaved work, and it is the
window's own difference. `New` is refused while the document is dirty, because the
window opens a tab and loses nothing and this shell has one buffer: the honest
answer is `unsaved changes: Ctrl+S saves them first`, the same sentence `Open`
gives, and it is the second place where the missing tabs show. `Reload` goes
ahead, because "throw away what I have and read the file" is what the command
means and the window's reload does exactly that.

The one liberty is in the wording. The window's tab loses its `[+]` mark when a
reload drops unsaved edits, which is a fact a reader can see; the terminal's bar is
the only place that fact can land, so a reload that dropped something says
`reloaded <path> (unsaved changes were dropped)` and one that dropped nothing says
`reloaded <path>`. Same behaviour, said out loud once because the strip that would
have said it is not here yet.

Both commands are one file read at most, and neither touches a keystroke that was
not already being painted.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed.

Through a pseudo terminal on a 140-column screen, from a fresh home. F9 and Down
onto the File dropdown's first row, then Enter, turned a file holding `one` and
`two` into a blank first line whose bar read `[No Name]`; the same command with a
character typed first said `unsaved changes: Ctrl+S saves them first` and left the
character in place. With the file on disk rewritten to `three` and a character
typed into the buffer, five Downs to `Reload from Disk` and Enter drew `three`, said
`reloaded /tmp/rel.txt (unsaved changes were dropped)`, and left the file on disk
holding `three` alone; the same command on a clean buffer said `reloaded
/tmp/rel.txt`.

`cargo test --offline --release`: 2,412 tests, 0 failed -- 2,410 plus two: New
giving a blank page and refusing while dirty, and Reload reading the file again,
refusing on a buffer with no file, saying what it dropped, and reporting a file
deleted since it was opened as an error rather than as an empty page. The keystroke
measurement is unchanged at 15,554 bytes for the first paint and 306 for a
keystroke. `cargo clippy --offline --release --all-targets`: 0 warnings. `cargo
fmt` was not run. The feature reference and its PDF are untouched.

## The hundred-and-thirty-fifth pass: the shape of a tab

The window keeps one editor per tab. The terminal shell kept one editor in the
shell itself, and that is the reason the last three passes had to write `Open`,
`New` and `Save As` so carefully: two of them refuse while the document is dirty,
because losing an edit to a second document is what the window's tabs exist to
prevent. The missing thing was never a command. It was a place for a second
document to live.

So the fields moved out of the shell and into a `Tab`: the editor, its
highlighter, and the five numbers that describe where that document is on screen
-- the first line, the first column, the wrapped segment above the window, the
first line the last frame had no room for, and the row a run of vertical moves
aims at. `App` now holds `tabs: Vec<Tab>` and `active: usize`, and what stayed on
the shell is what belongs to the window rather than to a document: the size of the
last frame, the bar, the preferences dialog, the find bar, the prompt, the
settings, the palette, the message and the quit arming.

The rest of the pass is one rename per call site, with three helpers to keep it
readable: `self.tab()`, `self.tab_mut()` and `self.editor()`, which say which
document a line is about without spelling out `self.tabs[self.active]` a hundred
and twenty times. Where two of them would be borrowed at once the path is written
out instead -- `self.find.handle(key, &mut self.tabs[self.active].editor)` --
because `self.editor()` borrows all of the shell mutably, and two fields have to be
named separately for the compiler to see they do not overlap. That sentence is the
whole cost of the shape, and it appears twice.

Two things were simplified on the way rather than moved. `with_document` used to
build a placeholder editor and replace it in `load`; it now builds a `tabs` list
holding one `Tab::new(doc)` and is done, with the first highlight pass living in
`Tab::new` beside the large-file rule it obeys. `load` is down to the assignment
and disarming the quit.

Nothing else changed, and that is the point: one tab exists so far and nothing
switches it, so this pass should be invisible on screen. Its value is that the next
one -- a strip, and a key that moves between two documents -- becomes a command
rather than a restructuring.

The cost of the indirection is an index and a bounds check per field access, on a
path that reaches a document pointer it already had. The keystroke measurement is
where a cost like that would show, and it did not.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical;
only `nn-tui` changed, and its output is byte for byte what it was.

`cargo test --offline --release`: 2,412 tests, 0 failed -- the same count as the
pass before, because a restructuring that changes no behaviour should add no test.
The tests read the front document through `app.tab().editor`, or through one new
helper, `caret(&app)`, which is the line and column in one call and keeps the eight
assertions that used to name both fields on one line each. The keystroke
measurement is unchanged at 15,554 bytes for the first paint and 306 for a
keystroke, 47 µs in this run. `cargo clippy --offline --release --all-targets`:
0 warnings. `cargo fmt` was not run. The feature reference and its PDF are
untouched.

Through a pseudo terminal on a 140-column screen, from a fresh home: opening
`crates/nn-tui/Cargo.toml`, two Downs and one character typed, drew the file with
`[+]` after the path in the bar and `Ln 2, Col 2` at the right, so the editor, its
caret and its dirty mark all still answer through the tab.

## The hundred-and-thirty-sixth pass: two documents you can walk between

The last pass gave a document somewhere to live; this one fills it. `New` and
`Open` now push a tab instead of refusing while the buffer in front is dirty, which
is the window's rule and the one the tabs were built for, and `Close`, `Close All`,
`Next Tab` and `Previous Tab` answer through the commands they always were. Nothing
in the shell holds a document any more except `tabs`, so switching is one integer
assignment: the scroll, the caret and the highlighter of the document left behind
are where they were, because they belong to it.

The chords are the window's, since the settings file is shared and a user who
rebinds `Next Tab` should get the new key in both front-ends. `Next Tab` and
`Previous Tab` are `Ctrl+PageDown` and `Ctrl+PageUp`, which a terminal sends as
`ESC [ 6 ; 5 ~` and `ESC [ 5 ; 5 ~`: two sequences `keys.rs` did not know, now
named `CtrlPageUp` and `CtrlPageDown` and turned back into the same `Chord` the
keymap holds, so the lookup is the ordinary one. `Close` is `Ctrl+W`, which arrives
as byte `0x17` and needed nothing at all. `Close All` is `Ctrl+Shift+W`, which does
not arrive and cannot: a terminal sends the same `0x17` for Ctrl+W, Ctrl+Shift+W
and Ctrl+Alt+W, so the chord is unreachable here, as `Ctrl+Shift+Q` and `Ctrl+H`
already are. It is on the File menu, five rows past `Save As...`, which is the same
road every other unreachable chord takes.

Two things are decided rather than translated. A file that is already open is
switched to instead of being opened twice -- two tabs on one file is two buffers
writing over each other, and the last save would win with nothing said about it --
and the name on the command line is now made absolute for the same reason the typed
one is, so `notes.txt` at the shell and `notes.txt` typed into the prompt are known
to be one file. Closing the last tab leaves a blank page rather than an empty strip,
which is the window's rule too: an editor with no document has nowhere to type.

What is missing is the strip, and the bar says so in its own way: while more than
one document is open it carries the count after the path, `[2/3]`, which is the row
the window draws as a tab strip. That is the one place this pass is visible, and it
is deliberately crude -- it lands in the same place the `[+]` dirty mark does, and
the next pass replaces both with a strip.

The cost is nothing per keystroke. Switching tabs does not touch the document, the
count is one small `format!` per frame while more than one is open, and the frame
writer still diffs what it is given, so the measurement below -- one document, no
count -- is unchanged byte for byte.

### Verified

`bench` over the same thirty-one flag combinations: all thirty-one identical; only
`nn-tui` changed.

`cargo test --offline --release`: 2,416 tests, 0 failed -- 2,412 plus four, from
six new tests, one rewritten in place and one gone. The six: `New` giving a blank
page in its own tab
with the text before it kept, a second document opened over a dirty buffer without
costing it the edit or the caret, the page chords walking and wrapping (and saying
so when there is only one document), a file already open being switched to, then
`Close` refusing a dirty buffer and keeping the neighbour once it is saved, and
`Close All` naming the count that stopped it. The test that asserted the old
refusal -- opening is refused while the document has unsaved changes -- is gone,
because that is the rule this pass replaced; the one that asserted `New`'s refusal
now asserts where the tab it makes lands. `keys.rs` reads the two new sequences in
the test it already had for held modifiers. The keystroke measurement is unchanged
at 15,554 bytes for the first paint and 306 for a keystroke.
`cargo clippy --offline --release --all-targets`: 0 warnings. `cargo fmt` was not
run. The feature reference and its PDF are untouched.

Through a pseudo terminal on 140 columns, from a fresh home, on a file holding
`one`: Down and one character typed put an `X` on the second line, then F9, Down,
Down, Enter and the prompt answered with `/tmp/tab-b.txt` drew `two` and `three`
with `opened /tmp/tab-b.txt` in the bar. `Ctrl+PageUp` drew the first document
again as `one` and `X`, its caret at `Ln 2, Col 2`, `[+]` after the path and `[1/2]`
at the end of it. `Ctrl+W` closed the second document, saying
`closed /tmp/tab-b.txt` and leaving the first with its edit. The File menu's
`Close All`, with a character typed into the second buffer, said
`unsaved changes in 1 document: Ctrl+S saves them first` and closed nothing; with
both saved it said `closed 2 documents` and left one blank page.

