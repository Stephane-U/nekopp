# What we want to achieve

A text and code editor for Linux, Windows and macOS. It does what Notepad++ does, on every
desktop, and it is fast enough that you stop thinking about it.

---

## What it must do

**Edit.** Tabs, split view, multiple cursors, column mode, bookmarks, code folding, word wrap,
zoom, session restore.

**Handle any text.** Any encoding, any line ending, files up to hundreds of megabytes. If a file
changes on disk, we notice and reload it.

**Highlight.** Every language Notepad++ covers, plus user-defined languages, plus a function
list.

**Search.** Find, replace, find in files, regular expressions, mark and count.

**Transform.** Line operations, case conversion, sorting, comment toggling, tabs to spaces.

**Look like a real application.** Menus, toolbar, status bar, preferences, themes, remappable
shortcuts.

**Extend.** Macros and plugins.

**Move in.** It reads Notepad++'s own configuration files, so someone switching over does not
have to reconfigure anything.

---

## What it must feel like

- Opens instantly.
- Never stutters — at any resolution, at any refresh rate.
- Uses no CPU when you are not touching it.
- Looks right on a 4K screen at 150% scaling.
- Never loses a byte.

---

## What "done" looks like

| | Target |
|---|---|
| Startup | under 250 ms |
| Install size | under 30 MB |
| Memory, five files open | under 90 MB |
| Scrolling | 60 fps at 1080p, 1440p and 4K; 120 fps wherever the display and GPU allow |
| Keystroke to pixel | under 8 ms |
| 100 MB file | opens in about a second |
| Idle CPU | zero |
| Data loss | never |

---

## What is out of scope

- Running without a GPU as a first-class mode. A fallback exists; it is not a promise.
- Loading Notepad++'s compiled plugins. New plugins are sandboxed and portable.
- Mobile, web, or a browser build.
- Being called Notepad++.

---

## How we will know we got there

The team uses it as their only editor from the moment it is usable, and every annoyance they hit
becomes a bug. Quality in an editor comes from use, not from time spent building.

---

*The engineering plan — architecture, stack, phases, risks — is in `ENGINEERING.md`. This
document is the one to read if you want to know what we are building and whether we are done.*
