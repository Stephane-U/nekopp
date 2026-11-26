# The terminal front-end

A second way to run Neko++: the same editor, drawn in characters, for a machine with
no GPU, a session over SSH, or a reader who prefers a terminal. Not a cut-down
editor and not a separate program that happens to edit text -- the same engine,
the same menus, the same preferences, the same commands, with the pixels replaced
by cells.

This document is the plan. It was agreed before any code moved, and it is the
answer to three questions asked in that order: is it possible, would it cost the
windowed version anything, and can the two stay in step.

## The one requirement that shapes everything

> Any new feature added to the windowed version is converted to the terminal
> version where it is possible.

That sentence decides the architecture. Today the decisions live in the windowed
program:

| Today | Where | Size |
|---|---|---|
| `Action`, 241 variants, and the dispatch | `crates/nn-app/src/menu.rs`, `main.rs` | 5,295 + 10,808 lines, 275 dispatch sites |
| The preference rows | `crates/nn-app/src/prefs.rs` | 2,343 lines |
| The window state | `State` in `main.rs` | 78 fields |

A second front-end that copied those would drift at the first new command: two
lists of actions, two dispatches, two answers to what "enabled" means. So the
decisions move into a shared library and the drawings stay behind:

| Shared library | Windowed front-end | Terminal front-end |
|---|---|---|
| Document set, tabs, session, undo, folding, highlighter styles | glyph shaping, GPU, window | character grid, escape sequences |
| `Action` table, one dispatch, enable and check rules | dialogs' pixels | dialogs' boxes |
| Menu tree, keymap, settings list with labels, choices and defaults | icon toolbar, tooltips, a11y tree | menu bar and dropdowns, preference dialog |
| Status messages, clipboard, `--call` and REST face | window scale | terminal capability probe |

Then there is exactly one `match` that runs a command. A new action is a compile
error until it is handled, and a new setting is a row in one list that both
front-ends draw. Parity stops being a habit and becomes something the compiler
enforces.

The few rows that genuinely cannot be expressed in a terminal are named in one
constant with a reason each, and a test asserts that the list has no other
members. Adding to it is a decision; forgetting to convert a feature is not.

## The terminal front-end

**The screen.** A cell grid, alternate screen, cursor parked, and redraw by
difference: only the rows and cells that changed are written. That is the whole
performance story of a TUI -- see the measurements at the end -- and it is worth
more than any colour trick.

**The menus.** The windowed bar and its thirteen dropdowns, drawn as rows of
cells: `F9` or a click opens the bar, arrows traverse, a letter jumps to a row,
`Escape` closes. Rows keep their shortcut column, their enabled and disabled
state, their check marks and their separators, because the tree is the same tree.

**The preferences.** The same dialog with the same 35 settings, drawn in cells
over the text, with the same rows, the same choices and the same "apply on
change". The rows come from the shared list, so a setting added for the window is
a setting added here without a second edit.

**Kept on purpose.** Tabs, split view, multiple carets, column mode, bookmarks,
folding, word wrap, find and replace with its options, find in files, the function
list, the folder panel, the plugin list, macros, styles, the encoding and line
ending menus, the character panel, session restore, backups, the `--call` and REST
faces, and the plugin host.

**Given up, as agreed.** The icon toolbar and its tooltips. Also unavoidable: the
kitten's three-dimensional shear, font selection, window scale, drag and drop from
a file manager, and the accessibility tree (the terminal and the screen reader
above it own that). Print preview becomes the existing PDF export plus a page
count, because a terminal cannot show a sheet of paper.

**The playground.** Two tiers behind one preference:

- *Plain*, the default and the one described as the software mode: the page, the
  furniture and the kitten drawn in ASCII and box-drawing characters, with the
  same simulation underneath -- `nn_core::playground` already holds it, and it
  already works in cells.
- *Colour*, when the terminal reports 24-bit colour and is not on the slow list:
  the same shapes with per-cell RGB and a higher frame rate.

No image protocols (kitty graphics, sixel) in this version. That is the only thing
that would be genuinely accelerated in a terminal, and the mode does not need it
to be fun.

## Phases

Each phase is one or more engineering passes, in the sense `ENGINEERING.md` uses:
a note, a `Verified` line, tests, and the reference updated when a person can see
the change.

| Phase | Work | Size | Gate |
|---|---|---|---|
| 0 | This document; the parity rule; a yardstick for a TUI (bytes per frame, keystroke latency) | 1-2 passes | written down and measured |
| 1 | Extract the shared library out of `main.rs`, `menu.rs`, `prefs.rs`. No behaviour change in the window | 8-14 passes | `bench` PPM dumps byte-identical across a flag matrix; 2,307 tests green |
| 2 | The terminal shell: grid and diff writer, keys, open, save, status line | 10-15 passes | typing and reopening a file, measured |
| 3 | Menu bar and dropdowns; the preference dialog from the shared rows | 6-10 passes | all 35 settings reachable and applied |
| 4 | The rest of the furniture, one pass per widget: tabs, split view, find bar and dialog, find in files, completion, function list, folder panel, plugins, macros, styles | 12-20 passes | the same checks each widget got in the window |
| 5 | The playground in characters, plain first, colour behind a preference | 3-6 passes | CPU per second against the windowed mode's own number |

The shell is its own crate and binary, `nn-tui` and `npp-tui`, rather than a
`--tui` switch inside `npp`: a front-end that cannot create a window should not
link wgpu, which is the same argument that made `npp-mcp` its own crate. A `--tui`
alias in `npp` can forward to it later if that turns out to be what people type.

Forty to sixty-five passes in total. Phase 1 is the long pole and the only one
that can hurt the windowed version, which is why its gate is a picture rather than
an opinion: the bench writes a PPM, and two runs of the same flags hash the same
(`a163442c3c14d3954aff272bd0937cdc` twice, 1280 by 720, `menu tabmenu`), so a
refactor that moves ten thousand lines can be proven not to have moved a pixel.

## Status

| Phase | State |
|---|---|
| 0 | done -- this document, and a yardstick that runs as a test rather than as a script |
| 1 | under way -- `nn-shared` holds the command vocabulary, the menu tree, the keymap and the list of themes (passes 109 to 113); the settings rows, the translations and the one dispatcher follow |
| 2 | done, as a shell: the screen, the caret, the colours, typing, saving |
| 3 | under way -- the bar, the thirteen dropdowns and their submenus are drawn and navigable (pass 112); the preference dialog comes next |
| 4 | not started |
| 5 | not started |

Phase 2 came before phase 1, which is the plan's order inverted, for a reason
worth keeping: a refactor with no consumer guesses at the API it should expose, and
the shell is now a consumer that can say what it needs. It touches nothing the
windowed front-end uses -- a new crate, one line in the workspace's member list --
so the inversion costs the window nothing, which is what phase 1's gate is there to
protect.

The shell's numbers, from `crates/nn-tui/src/app.rs`: the first paint of a 120 by
40 window is 10,425 bytes, a keystroke in a 500-line file is 306 of them, and the
drawing takes about 40 microseconds. The pass note in `ENGINEERING.md` has the
detail and the screenshots.

Phase 1 is measured the way the table above asks: `bench` over thirty-one flag
combinations, dumped to PPM and hashed. Passes 109 to 111 moved the command
vocabulary, the menu tree and the keymap into `nn-shared`, and every one of the
thirty-one hashes stayed the same in all three. The script that runs the matrix is not in the tree -- it is four lines
of shell around `bench`, and a second copy of the flag list would be a third
list to keep in step -- so the note records the result and the flags rather than
a tool.

## Why the terminal front-end and not a client of the REST face

Measured on this machine, 120 Hz display, loopback:

| Call | Cost |
|---|---|
| one call after a pause | 0.8-1.2 ms |
| back to back | 8.3 ms each -- one frame |
| one-character edit in a 1 MB document | 26.7 ms |
| whole-document replace, 1 MB | 29.8 ms |

A REST client inherits the window's frame clock and its redraw, and its cost grows
with the file. It also cannot insert at a point: an empty range is refused, so a
keystroke has to replace a character that exists. That face is good for driving a
running editor and wrong for being one.

## What a terminal costs, measured

Full screen of styled text, timed by writing it and then asking the terminal who it
is -- the reply only arrives once everything before it has been parsed:

| Terminal | 19-byte frame | Full screen |
|---|---|---|
| alacritty 0.17 | 0.133 ms | 30,138 bytes in 0.89 ms |
| kitty 0.48 | 8.334 ms -- exactly one refresh at 120 Hz | 37,116 bytes in 4.65 ms |

Kitty answers on its next draw, so its figure is a frame and not a parse cost; a
whole screen of styled text fits inside one refresh either way. There is no text
acceleration to buy. What decides a TUI's speed is how many bytes it writes, which
is why the diff writer is phase 2 and not a later optimisation.

For comparison, the windowed front-end's own frame on a GTX 1080 Ti at 1920 by
1080 is p50 0.45 ms idle, 0.51 ms scrolling and 2.61 ms typing.

And without a GPU at all, the windowed front-end still runs -- it says so:

```
npp: llvmpipe (LLVM 15.0.6, 256 bits) (Vulkan)
npp: software rendering, expect a slower frame rate
```

Ten seconds of the animated page cost 1.35 CPU-seconds on the GPU and 30.01 on
llvmpipe, three cores against a seventh of one; a still document stays smooth,
because a frame-paced call measured the same 8.4 ms on both. That is the case the
terminal front-end answers better, and it is why the playground has a plain tier.

## Risks

| Risk | Answer |
|---|---|
| Moving ten thousand lines breaks the window | Phase 1's gate: identical PPM dumps, then the test suite |
| Two front-ends drift apart | One dispatch, one menu tree, one settings list; the exceptions are a tested constant |
| "Look and feel" is only approximate in cells | Accepted: the structure matches, the pixels cannot |
| Terminal differences (colour, mouse, width) | A capability probe, `--plain`, and a colour depth of 16, 256 or 24-bit |
| The release grows two binaries | Packaging builds `--bin npp` and `--bin npp-tui` separately; the man page gains a section |
| A TUI has no yardstick | Phase 0's: bytes per frame and keystroke latency, recorded per pass like the frame times are |

## Where this sits in `PLAN.md`

`PLAN.md` lists "running without a GPU as a first-class mode" as out of scope, with
a fallback that is not a promise. This is not that: it is a second front-end, and
the fallback stays a fallback. A machine with no GPU gets a first-class editor,
which is the spirit of the line, without the windowed version promising a
software-rasterised mode it does not want to maintain.

## What is not decided here

Nothing about the windowed version changes. Naming, packaging details and the
shape of the colour tier will be settled in their phases, in that order, each with
its own note.
