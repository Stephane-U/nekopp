# Neko++ Feature Reference

Every feature the editor has, where it lives, and what it does. The inventory is
taken from the source: the menu tree in `crates/nn-app/src/menu.rs`, the default
keymap in `crates/nn-app/src/keymap.rs`, and one section per subsystem, each
written against the module that implements it.

## About This Document

Neko++ is a Notepad++ workalike for Linux, written in Rust, with a GPU renderer
and a software fallback behind the same interface. The binary is `npp`. The
workspace is `nn`; `nn-core` holds the document model, the lexers, the search
engine and the pure logic, and `nn-app` holds the window, the renderer, the
widgets and the commands.

This document is the source of truth for the feature inventory, and it is also
served, section by section, by the `npp-mcp` Model Context Protocol server that
ships beside it (`crates/nn-mcp`). That is why the headings are stable and why
every menu row appears literally, with its label and its shortcut: a reader
looking for "Sort Lines By Length Ascending" should find that exact string.

Three rules were followed while writing it.

- A row that exists in the menu is listed, even when the row does something
  small. The menu is the contract.
- A feature that is deliberately absent is listed too, in "Deliberate
  Omissions", with the reason. An editor that silently lacks a row is harder to
  trust than one that says why.
- Where the editor deviates from Notepad++, the deviation is named. A clone that
  pretends to be identical is a clone whose users cannot tell a bug from a
  decision.

Counts quoted in this document are counts of things in the source, not
estimates: 147 built-in languages, 41 single-byte code pages in 13 character-set
groups, 50 theme choices, 73 default key bindings, 23 bundled localisations, 16
style slots, and 13 top-level menus.

## Overview

Neko++ opens text files, edits them, and puts them back on disk. That sentence
describes a hundred editors, so the interesting part is what sits underneath it.

### The shape of the program

Two crates, one binary.

- `nn-core` is the editor without a window: a rope-backed document, the caret
  and selection model, undo, search, folding, lexers, code pages, printing
  layout and the WebAssembly plugin interpreter. It has no GPU, windowing or
  event-loop dependency, which is what lets it be tested properly.
- `nn-app` is everything that needs a screen: the wgpu renderer, the widgets
  drawn in an overlay pass, the menu bar, the dialogs, the file browser, the
  session, the settings, and the dispatcher that turns a menu click into an
  editor call.

The split is not decoration. Every behaviour with a decision in it lives in a
library module with tests; `main.rs` is a thin shell that owns the winit event
loop and the GPU surface, and it is the only file in the tree without a test
module.

### Rendering

Text is shaped by cosmic-text and drawn through glyphon on wgpu. Shaped lines
are cached per line and invalidated by edit, so a frame that scrolls reuses
every glyph run it can. Rectangles -- the caret, selections, the current-line
band, the gutter, fold markers -- go through an instanced quad renderer. Icons
are vector paths flattened and rasterised once into an alpha atlas rather than
loaded as bitmaps.

The GPU is the primary path, not the only one. `npp --software` selects the
software rasteriser for machines with no usable adapter, remote sessions and
containers; both backends draw the same scene.

### Footprint and speed

The design target is an editor that starts in well under a second and stays
responsive on files far larger than a screen. Three decisions carry most of
that: the document is a rope, so an insert in the middle of a 200 MB file does
not copy it; highlighting and folding are incremental and stop as soon as their
state converges with what is already stored; and the view keeps a per-line
shaping cache instead of re-shaping the visible page every frame.

### Compatibility

Notepad++ is the reference for behaviour and for the shape of the menus. The
editor reads Notepad++'s `config.xml`, `shortcuts.xml`, `stylers.xml` and
`userDefineLang.xml`, and it reads Notepad++'s localisation files. It writes
`shortcuts.xml` back. Compatibility is behavioural, not binary: plugins are
WebAssembly, not DLLs, and the reason is in "Plugins".

## Installing and Running

Neko++ is a single binary. There is no data directory to install beside it, no
runtime to satisfy and no service to start; the translations and the Hangul font
are compiled in.

### Command line

```
usage: npp [switches] [file ...]

A file named - reads the document from standard input.

switches:
  -n, --new-instance start a new process even if one is running
  -w, --wait         block until the document is closed
  -l, --line         put the caret on this line (one-based)
  -c, --column       put the caret on this column (one-based)
  -s, --software     draw with the software rasterizer, not the GPU
  -h, --help         print this text
  -V, --version      print the version
```

The parser follows the conventions a shell user expects. `--` ends the switches,
so a file called `-w` can still be opened. A lone `-` means standard input, and
the document it produces is marked as unnamed until it is saved. Short switches
cluster (`-nw` is `-n -w`), a switch that takes a number accepts it attached
(`-l20`) or separate (`-l 20`), and `--line=7` works as well as `--line 7`.
`--line` and `--column` are one-based because compiler errors and `grep -n` are
one-based; the editor converts once, in one place.

`--wait` is what makes `npp` usable as `$EDITOR`: the process stays alive until
every document it opened is closed, so a tool that shells out to an editor can
wait for the edit to finish.

### One editor per session

A second launch does not open a second window by default. The first process owns
the session bus name `org.npp.Editor`, and a later launch calls `Open` on it with
absolute paths, the requested caret position, and whether the caller wants to be
told when the document closes. The second process then exits.

The bus is optional. A bare TTY, a container or a CI run with no session bus gets
a standalone editor rather than an error. `--new-instance` skips the handoff and
starts a second full editor, which is what a user wants when comparing two files
side by side on two monitors.

### Files the editor writes

Configuration lives where the platform says it should:

| Platform | Directory |
|---|---|
| Linux | `$XDG_CONFIG_HOME/npp` or `~/.config/npp` |
| Windows | `%APPDATA%\npp` |
| macOS | `~/Library/Application Support/npp` |

| File | Contents |
|---|---|
| `settings.txt` | every preference, the keymap, the Run menu, the print setup |
| `session.txt` | the recent file list, the open tabs, their carets and colours |
| `plugins/` | one `.wasm` file per plugin |
| `backup/` | timestamped copies, when verbose backups are on |
| `snapshot/` | the unsaved buffers written by the autosave timer |

## The Window

The window is a menu bar, a toolbar, a tab strip, the editor pane, optional
docked panels, and a status bar. Every piece of furniture can be turned off,
because a text editor's job is the text.

### Menu bar

Thirteen menus: File, Edit, Search, View, Encoding, Language, Settings, Tools,
Macro, Run, Plugins, Window, Help. The bar is laid out from the monospace
advance width, so a dropdown is exactly as wide as its longest label plus its
shortcut, and the shortcut column is read from the keymap rather than stored
with the row -- a rebinding cannot leave the menu advertising a chord the editor
does not honour.

Menus can be reached from the keyboard: Alt and the menu's first letter opens
it, the arrow keys walk the rows, Enter opens a submenu and Escape closes one
level. Pressing the same chord twice cycles between two menus that share a
letter rather than closing and reopening one.

### Toolbar

A row of icon buttons under the menu bar, in Notepad++'s order and its nine
groups: file, clipboard, history, search, zoom, scroll sync, view toggles,
panels, monitoring, and the macro recorder. `Ctrl+Alt+T` is not it -- the
toolbar's switch is in Preferences, and it is remembered.

The buttons are: New, Open, Save, Save All, Close, Close All, Print, Cut, Copy,
Paste, Undo, Redo, Find, Replace, Zoom In, Zoom Out, Sync Vertical Scroll, Sync
Horizontal Scroll, Word Wrap, Show All Characters, Show Indent Guide, User
Defined Language, Document Map, Document List, Function List, Folder as
Workspace, Monitoring, Start Recording, Stop Recording, Playback, Run Multiple
Macros, Save Current Macro.

Each button has a tooltip that appears after the pointer rests on it, and the
tooltip is the only text in the bar that is translated -- the icons are not.

### Tab strip

One tab per document, with the title, a modified marker, an optional pin and an
optional colour. Three arrangements, cycled by "Tab Layout" (`Ctrl+Alt+B`):
Single Row, Multi-line, and Vertical down the left edge.

Tabs can be pinned, so "Close All but Pinned Documents" keeps them; coloured
with five preset colours; reordered by "Move Tab Forward" and "Move Tab
Backward"; and moved to the start or the end. A right-click opens the tab
strip's context menu. With more tabs than fit, the strip clips rather than
shrinks, and the document peeker (`Ctrl+Tab`) is the way to find one.

### Status bar

Left: the caret's line and column, the selection length, the document length,
the encoding and the line-ending mode. Right: the language, the read-only state,
and any one-off message -- "Column mode on", "Hid 4 block(s)", a save failure.
Messages replace the right-hand labels until the next action.

### Editor pane

The text area itself: a gutter with line numbers and fold markers, the current
line band, the selection, the caret and any additional carets, the minimap on
the right edge when it is on, and the whitespace and end-of-line marks when they
are on.

### Layout

Without a split there is one pane filling the window. A vertical split puts two
panes side by side with a draggable divider; a horizontal split stacks them.
Each pane has its own tab strip, its own active document, its own scroll
position and its own caret. The arithmetic that decides which rectangle belongs
to which pane is a pure function in `layout.rs` with tests, because a click that
lands in the wrong document is the kind of bug nobody notices until it edits the
wrong file.

### Overlays

Modal and semi-modal widgets are drawn in an overlay pass after the text: the
menu dropdowns, the completion popup, the document peeker, the preferences
dialog, the shortcut editor, the style configurator, the file browser, the
context menus, the path prompt, the find bar, the About box, the tooltip, the
character panel, the change history panel, the function list, the folder tree,
the results panel and the print preview.

## The Menus

The menu is Notepad++'s, row for row, because a user who knows where a command
lives should not have to learn a second arrangement. Each subsection below lists
every row, with its default shortcut in parentheses. Submenus are indented.

A row with no shortcut can still be bound: "Shortcut Mapper" lists every
bindable command, and the keymap file accepts any chord for any of them.

### File

The document's lifecycle. Recent files are labelled with the file name alone --
a dropdown is a fixed width, and a full path would either be truncated or make
the menu enormous -- and there are at most ten of them.

"Open Containing Folder" is a submenu because on Linux the three rows really are
three programs: the desktop's file manager, `cmd` (a terminal) and PowerShell.
All three open a terminal or a file manager in the document's directory, and the
row is disabled for an unsaved document. "Open in Default Viewer" hands the file
to the desktop's own handler rather than to the editor. "Reload from Disk"
re-reads the file and discards unsaved changes after asking.

The save family is ordinary except for one row: "Save a Copy As" writes the
buffer to a path and leaves the tab pointing at the original, which is what a
user wants when exporting a fragment without renaming the document.

"Close Multiple Documents" holds the five ways to close a set: everything but
the active document, everything but the pinned ones, everything to the left,
everything to the right, and everything with no unsaved changes. "Move to
Recycle Bin" uses the XDG trash, so the file is recoverable.

"Load Session" and "Save Session" write the same file the editor uses for its
own resume, which makes a session a document rather than a hidden state.

- New (`Ctrl+N`)
- Open... (`Ctrl+O`)
- The recent files, one row each (up to ten), by file name
- Clear Recent
- Open Containing Folder
  - Explorer
  - cmd
  - PowerShell
  - Folder as Workspace
- Open in Default Viewer
- Open Folder as Workspace...
- Reload from Disk (`Ctrl+Alt+R`)
- Save (`Ctrl+S`)
- Save As... (`Ctrl+Shift+S`)
- Save a Copy As...
- Save All (`Ctrl+Alt+S`)
- Rename...
- Close (`Ctrl+W`)
- Close All (`Ctrl+Shift+W`)
- Close Multiple Documents
  - Close All but Active Document
  - Close All but Pinned Documents
  - Close All to the Left
  - Close All to the Right
  - Close All Unchanged
- Move to Recycle Bin
- Load Session...
- Save Session...
- Print... (`Ctrl+P`)
- Print Now (`Ctrl+Alt+Shift+P`)
- Exit (`Ctrl+Shift+Q`)

### Edit

Notepad++'s largest menu, and almost all of it is submenus, because a flat list
would be sixty rows long. Undo and redo group by edit rather than by keystroke:
typing a word is one step, and so is one paste.

"Begin/End Select" and its column-mode twin mark an anchor and let the caret
move freely until the second press closes the selection; that is how a selection
larger than the screen is made without holding a button.

"Insert" writes the local time in one of three shapes: `HH:MM:SS`, the long
form, and a `strftime` pattern the user supplies once and reuses. The local time
comes from the C library's time zone data rather than a date crate.

"Copy to Clipboard" copies the full path, the file name, the directory, or every
open file's name or path -- one per line, in tab order.

"Indent" and "Unindent" move the selected lines by one tab stop. "Convert Case
to" has eight rows because the eight are genuinely different: forced uppercase,
lowercase, Proper Case and its blend (which leaves words that already contain a
capital alone), Sentence case and its blend, inverted case, and random case for
testing a case-sensitive parser.

"Line Operations" sorts, dedupes, trims, joins, splits and reorders lines; it is
described in "Line Operations".

"Comment/Uncomment" uses the active language's own comment syntax, so `--` in a
SQL file and `#` in a shell script, with block comments where the language
defines a pair.

"Auto-Completion" reaches word completion from the document, function
completion from the symbol scan, parameter hints, and path completion against
the filesystem.

"EOL Conversion" and "Blank Operations" are the two groups a user reaches for
when a file arrives from Windows or from a form: CRLF/LF/CR, and trimming,
converting end-of-line runs to spaces, and converting between tabs and spaces.

"On Selection" opens the selected text as a file, opens its folder, redacts it
in place with a fixed-width mask, or searches the internet for it -- with an
editable engine template.

Multi-select is the multi-caret feature: add every occurrence of the selection,
or the next one, under one of four matching rules, then edit all of them at
once. "Column Mode" makes the next drag a rectangle; "Column Editor" inserts
text or a number series at one column of every selected line.

The last rows are the panels: the character panel, the clipboard history (ten
entries, newest first, consecutive duplicates collapsed), and read-only, which
can be set for the current document or for every document and cleared again.

- Undo (`Ctrl+Z`)
- Redo (`Ctrl+Y`)
- Cut (`Ctrl+X`)
- Copy (`Ctrl+C`)
- Paste (`Ctrl+V`)
- Delete
- Select All (`Ctrl+A`)
- Begin/End Select
- Begin/End Select in Column Mode
- Insert
  - Date Time (short)
  - Date Time (long)
  - Date Time (customized)
- Copy to Clipboard
  - Copy Current Full File path
  - Copy Current Filename
  - Copy Current Dir. Path
  - Copy All Filenames
  - Copy All File Paths
- Indent
  - Increase Line Indent (`Ctrl+]`)
  - Decrease Line Indent (`Ctrl+[`)
- Convert Case to
  - UPPERCASE (`Ctrl+Shift+U`)
  - lowercase (`Ctrl+U`)
  - Proper Case
  - Proper Case (blend)
  - Sentence case
  - Sentence case (blend)
  - iNVERT cASE
  - ranDOm CasE
- Line Operations
  - Duplicate Current Line (`Ctrl+D`)
  - Remove Duplicate Lines
  - Remove Consecutive Duplicate Lines
  - Split Lines
  - Join Lines
  - Move Up Current Line (`Ctrl+Shift+Up`)
  - Move Down Current Line (`Ctrl+Shift+Down`)
  - Remove Empty Lines
  - Remove Empty Lines (Containing Blank characters)
  - Insert Blank Line Above Current
  - Insert Blank Line Below Current
  - Reverse Line Order
  - Randomize Line Order
  - Sort Lines Lexicographically Ascending
  - Sort Lines Lex. Ascending Ignoring Case
  - Sort Lines In Locale Order Ascending
  - Sort Lines As Integers Ascending
  - Sort Lines As Decimals (Comma) Ascending
  - Sort Lines As Decimals (Dot) Ascending
  - Sort Lines By Length Ascending
  - Sort Lines Lexicographically Descending
  - Sort Lines Lex. Descending Ignoring Case
  - Sort Lines In Locale Order Descending
  - Sort Lines As Integers Descending
  - Sort Lines As Decimals (Comma) Descending
  - Sort Lines As Decimals (Dot) Descending
  - Sort Lines By Length Descending
- Comment/Uncomment
  - Toggle Single Line Comment (`Ctrl+Q`)
  - Single Line Comment
  - Single Line Uncomment
  - Block Comment
  - Block Uncomment
- Auto-Completion
  - Function Completion (`Ctrl+Shift+Space`)
  - Word Completion (`Ctrl+Space`)
  - Function Parameters Hint (`Ctrl+Shift+Alt+Space`)
  - Function Parameters Previous Hint
  - Function Parameters Next Hint
  - Path Completion (`Ctrl+Alt+Space`)
- EOL Conversion
  - Windows (CR LF)
  - Unix (LF)
  - Macintosh (CR)
- Blank Operations
  - Trim Trailing Space
  - Trim Leading Space
  - Trim Leading and Trailing Space
  - EOL to Space
  - Trim both and EOL to Space
  - TAB to Space
  - Space to TAB (All)
  - Space to TAB (Leading)
- On Selection
  - Open File
  - Open Containing Folder in Explorer
  - Redact Selection
  - Search on Internet
  - Change Search Engine...
- Multi-select All
  - Ignore Case & Whole Word
  - Match Case Only
  - Match Whole Word Only
  - Match Case & Whole Word
- Multi-select Next
  - Ignore Case & Whole Word
  - Match Case Only
  - Match Whole Word Only
  - Match Case & Whole Word
- Undo the Latest Added Multi-Select
- Skip Current & Go to Next Multi-select
- Column Mode...
- Column Editor...
- Character Panel (`Ctrl+Alt+C`)
- Clipboard History
- Read-Only in Neko++
  - Read-Only on Current Document
  - Read-Only for All Documents
  - Clear Read-Only for All Documents

### Search

The find bar is one line: a field, three toggles (match case, whole word,
regular expression), a match counter and the buttons. It is incremental in the
sense that matters -- the count updates as you type -- and it never moves the
caret until you ask for the next match.

"Find (Volatile) Next" and its previous sibling search without changing what is
remembered as the search. "Select and Find Next" starts a selection at the
caret and extends it to the next match, which is how a list of occurrences is
built one keystroke at a time.

"Find in Files" is one field with one optional suffix: `pattern in directory`.
The directory half is only taken when it names a real directory, so a pattern
that happens to contain " in " still searches for itself. The default directory
is the active file's folder, because find-in-files means find in the files
around this one. Hits stream into the results panel as the walk proceeds, and
clicking one opens the file at the line. "Find in Projects" runs the same search
over every project root.

"Go to Matching Brace" jumps between the bracket under the caret and its
partner, using the highlighter's styles so a brace inside a string does not
count. "Select All In-between {} [] or ()" selects the text between the
innermost pair around the caret.

"Mark..." highlights every match of the search text. The five mark styles are
separate colours so several searches can coexist: "Style All Occurrences of
Token" and "Style One Token" paint matches, "Clear Style" removes one style or
all of them, "Jump Up" and "Jump Down" walk the marks of one style, or of
every style in document order when "Find Mark Style" is chosen. "Copy Styled
Text" copies the text inside one style's marks, one match per line, and "All
Styles" copies every style in turn. The name says markup; what it copies is the
matches, because a clipboard is text rather than a colour.

The bookmark rows operate on lines rather than on text: toggle, next, previous,
clear, cut the bookmarked lines, copy them, paste over them, remove them, remove
the ones that are not bookmarked, and invert the set. Bookmarks survive edits to
other lines because they belong to lines, not to offsets.

"Find characters in range..." asks for one code point, in decimal or in `0x`
hex, and selects its next occurrence after the caret. It is the way to find a
stray character that has no name to type -- a non-breaking space, a soft hyphen,
a zero-width joiner -- without writing a regex for it.

- Find... (`Ctrl+F`)
- Find in Files... (`Ctrl+Shift+F`)
- Find Next (`F3`)
- Find Previous (`Shift+F3`)
- Select and Find Next
- Select and Find Previous
- Find (Volatile) Next
- Find (Volatile) Previous
- Replace... (`Ctrl+H`)
- Incremental Search (`Ctrl+Alt+I`)
- Search Results Window
- Next Search Result
- Previous Search Result
- Go to... (`Ctrl+G`)
- Go to Matching Brace (`Ctrl+B`)
- Select All In-between {} [] or ()
- Mark...
- Change History
  - Go to Next Change
  - Go to Previous Change
  - Clear Change History
- Style All Occurrences of Token
  - 1st Style
  - 2nd Style
  - 3rd Style
  - 4th Style
  - 5th Style
- Style One Token
  - 1st Style
  - 2nd Style
  - 3rd Style
  - 4th Style
  - 5th Style
- Clear Style
  - Clear 1st Style
  - Clear 2nd Style
  - Clear 3rd Style
  - Clear 4th Style
  - Clear 5th Style
  - Clear all Styles
- Jump Up
  - 1st Style
  - 2nd Style
  - 3rd Style
  - 4th Style
  - 5th Style
  - Find Mark Style
- Jump Down
  - 1st Style
  - 2nd Style
  - 3rd Style
  - 4th Style
  - 5th Style
  - Find Mark Style
- Copy Styled Text
  - All Styles
  - 1st Style
  - 2nd Style
  - 3rd Style
  - 4th Style
  - 5th Style
  - Find Mark Style
- Bookmark
  - Toggle Bookmark (`Ctrl+F2`)
  - Next Bookmark (`F2`)
  - Previous Bookmark (`Shift+F2`)
  - Clear All Bookmarks
  - Cut Bookmarked Lines
  - Copy Bookmarked Lines
  - Paste to (Replace) Bookmarked Lines
  - Remove Bookmarked Lines
  - Remove Non-Bookmarked Lines
  - Inverse Bookmarks
- Find characters in range...

### View

Tab navigation, tab colour and the tab strip's own ordering live here, because
they are about how the documents are arranged rather than what is in them.

"Always on Top", "Toggle Full Screen Mode" and "Distraction Free Mode" are three
separate switches: one changes the window's stacking, one its size, one how much
furniture is drawn. "Post-It" sets two of them at once -- always on top and no
chrome -- because a note kept beside your work has to stay visible and has to be
small.

"View Current File in" offers Firefox, Chrome, Edge and IE. On Linux the four
labels are deliberate: none of them can be honoured as a *specific* browser, so
all four open the desktop's default handler, and the labels stay because a user
looking for the row should find it where Notepad++ put it. A missing browser is
not a dead row.

"Show Symbol" controls what is drawn in place of invisible characters: spaces
and tabs, end-of-line, non-printing characters, control characters and the
Unicode end-of-line glyph, all of them at once, the indent guide, and the wrap
symbol at a soft line break.

"Zoom" changes the font size in steps; the status bar shows the percentage when
it is not 100. "Synchronize Across Views" makes one pane's zoom apply to both.

"Move/Clone Current Document" moves the active tab to the other pane or copies
it there, and the last two rows open the document in a second *process* rather
than a second pane.

The "Tab" submenu holds the nine numbered tabs, the four navigation rows, the
four reordering rows and the five colours plus "Remove Color". The numbered
rows are a Notepad++ habit that survives because it is faster than counting.

"Word wrap" wraps long lines at the pane's width; the wrap is soft, so nothing
is written to the file. "Focus on Another View" moves the keyboard to the other
pane. "Hide Lines" folds away every foldable block the selection touches --
nothing is deleted, and "Unfold All" brings it back.

The folding rows are described in "Folding". "Summary..." reports lines, words
and characters for the document, and for the selection when there is one.

"Project Panels" opens one of three project trees. "Folder as Workspace" opens
the directory tree. "Document Map" is the minimap. "Document List" is the list
of open documents. "Function List" is the symbol panel.

"Synchronize Vertical Scrolling" and its horizontal twin tie the two panes
together, which is what makes a split useful for comparing two versions of a
file. "Monitoring (tail -f)" reloads the active document whenever it changes on
disk without asking, for a log file that is still being written.

- Always on Top
- Toggle Full Screen Mode (`F11`)
- Post-It
- Distraction Free Mode (`F12`)
- View Current File in
  - Firefox
  - Chrome
  - Edge
  - IE
- Show Symbol
  - Show Space and Tab
  - Show End of Line
  - Show Non-Printing Characters
  - Show Control Characters & Unicode EOL
  - Show All Characters
  - Show Indent Guide
  - Show Wrap Symbol
- Zoom
  - Zoom In (Ctrl+Mouse Wheel Up) (`Ctrl++`)
  - Zoom Out (Ctrl+Mouse Wheel Down) (`Ctrl+-`)
  - Restore Default Zoom (`Ctrl+0`)
  - Synchronize Across Views
- Move/Clone Current Document
  - Move to Other View (`Ctrl+Alt+Shift+Right`)
  - Clone to Other View (`Ctrl+Alt+Shift+Left`)
  - Move to New Instance
  - Open in New Instance
- Tab
  - 1st Tab
  - 2nd Tab
  - 3rd Tab
  - 4th Tab
  - 5th Tab
  - 6th Tab
  - 7th Tab
  - 8th Tab
  - 9th Tab
  - First Tab
  - Last Tab
  - Next Tab (`Ctrl+PageDown`)
  - Previous Tab (`Ctrl+PageUp`)
  - Move to Start
  - Move to End
  - Move Tab Forward
  - Move Tab Backward
  - Apply Color 1
  - Apply Color 2
  - Apply Color 3
  - Apply Color 4
  - Apply Color 5
  - Remove Color
- Word wrap (`Ctrl+Alt+W`)
- Focus on Another View
- Hide Lines
- Fold All (`Ctrl+Alt+A`)
- Unfold All (`Ctrl+Alt+Shift+F`)
- Fold Current Level
- Unfold Current Level
- Fold Level
  - 1
  - 2
  - 3
  - 4
  - 5
  - 6
  - 7
  - 8
- Unfold Level
  - 1
  - 2
  - 3
  - 4
  - 5
  - 6
  - 7
  - 8
- Summary...
- Project Panels
  - Project Panel 1
  - Project Panel 2
  - Project Panel 3
- Folder as Workspace
- Document Map
- Document List
- Function List (`Ctrl+Alt+L`)
- Synchronize Vertical Scrolling
- Synchronize Horizontal Scrolling
- Monitoring (tail -f)

### Encoding

Two halves with different meanings, and confusing them corrupts files. The top
rows *reinterpret* the bytes already in the file: the file is unchanged, the
same bytes are read again under a different assumption, which is the only way to
recover a file whose encoding was guessed wrong. The "Convert to" rows *re-encode*
the text in memory, so the next save writes different bytes.

ANSI means Windows-1252, which is what Notepad++ means by it. UTF-8 is available
with and without a byte-order mark, and UTF-16 in both byte orders. Everything
else is under "Character sets", grouped by script the way Notepad++ groups them:
13 groups and 41 code pages, from ISO 8859-1 through the OEM and Windows code
pages to KOI8-R and TIS-620. A file that does not decode cleanly is reported,
not silently mangled.

- ANSI
- UTF-8
- UTF-8-BOM
- UTF-16 BE BOM
- UTF-16 LE BOM
- Character sets
  - Arabic
    - ISO 8859-6
    - OEM 720
    - Windows-1256
  - Baltic
    - ISO 8859-4
    - ISO 8859-13
    - OEM 775
    - Windows-1257
  - Celtic
    - ISO 8859-14
  - Cyrillic
    - ISO 8859-5
    - KOI8-R
    - KOI8-U
    - Macintosh
    - OEM 855
    - OEM 866
    - Windows-1251
  - Central European
    - OEM 852
    - Windows-1250
  - Eastern European
    - ISO 8859-2
  - Greek
    - ISO 8859-7
    - OEM 737
    - OEM 869
    - Windows-1253
  - Hebrew
    - ISO 8859-8
    - OEM 862
    - Windows-1255
  - North European
    - OEM 861
    - OEM 865
  - Thai
    - TIS-620
  - Turkish
    - ISO 8859-3
    - ISO 8859-9
    - OEM 857
    - Windows-1254
  - Western European
    - ISO 8859-1
    - ISO 8859-15
    - OEM 850
    - OEM 858
    - OEM 860
    - OEM 863
    - OEM 437
    - Windows-1252
  - Vietnamese
    - Windows-1258
- Convert to ANSI
- Convert to UTF-8
- Convert to UTF-8-BOM
- Convert to UTF-16 BE BOM
- Convert to UTF-16 LE BOM

### Language

"None (Normal Text)" is the first row, because plain text is a language choice.
The 147 built-in languages are grouped under their first letter, which turns a
menu taller than the screen into a dozen short ones. Three of them are not
grouped, because Notepad++ added them after the grouped block was written and
never moved them in: KIXtart sits after J, and XML and YAML after V.

The language of a document decides its comment syntax, its keywords, its
folding rules, its function-list scan and its completion source. It is chosen
automatically from the file's extension and can be overridden from this menu;
the override is remembered per document, not per extension.

"User Defined Language" manages Notepad++'s `userDefineLang.xml`: define a
language in the editor, open the folder the files live in, or visit the
collection. Installed languages appear under "User-Defined"; the submenu exists
even when it is empty, so the menu does not change shape when the first one is
installed.

- None (Normal Text)
- User Defined Language
  - Define your language...
  - Open User Defined Language folder...
  - Notepad++ User Defined Languages Collection
- (none installed)
- User-Defined
  - @local:udl

### Settings

"Preferences..." is the settings dialog described in "Preferences". "Style
Configurator..." edits the sixteen style slots described in "Themes and the
Style Configurator".

"Theme" lists all 50 choices: System, which follows the desktop's colour-scheme
preference and falls back to dark when the desktop has no opinion; the two
built-in palettes, Light and Dark; and 47 named palettes. They are described in
"Themes and the Style Configurator".

"Shortcut Mapper" is a submenu rather than a dialog with Import and Export
buttons along its bottom edge, because this editor has no such dialog. The three
rows are the same three operations in the same place, one level down:
"Open Shortcut Mapper..." edits the bindings, "Import Notepad++ Config..." reads
a Notepad++ installation's `config.xml` and `shortcuts.xml`, and "Export
Shortcuts..." writes a `shortcuts.xml` a Notepad++ installation can load.

"Import" takes plugins and style themes: a `.wasm` file copied into the plugins
directory, or a `stylers.xml` whose colours become the current theme.

"Edit Popup ContextMenu" edits the editor's right-click menu.

- Preferences... (`Ctrl+,`)
- Style Configurator...
- Theme
  - System
  - Light
  - Dark
  - Ayu Dark
  - Ayu Light
  - Ayu Mirage
  - Catppuccin Frappe
  - Catppuccin Latte
  - Catppuccin Macchiato
  - Catppuccin Mocha
  - Darcula
  - Dracula
  - Everforest
  - Everforest Light
  - GitHub Dark
  - GitHub Dark Dimmed
  - GitHub Light
  - Gruvbox Dark
  - Gruvbox Light
  - Horizon
  - Iceberg
  - Kanagawa
  - Kanagawa Dragon
  - Kanagawa Lotus
  - Material Ocean
  - Material Palenight
  - Molokai
  - Monokai
  - Monokai Pro
  - Moonlight
  - Night Owl
  - Nightfox
  - Nord
  - Oceanic Next
  - One Dark
  - One Light
  - Panda Syntax
  - Quiet Light
  - Rose Pine
  - Rose Pine Dawn
  - Rose Pine Moon
  - Shades of Purple
  - Solarized Dark
  - Solarized Light
  - Synthwave 84
  - Tokyo Night
  - Tokyo Night Day
  - Tokyo Night Storm
  - Vitesse Dark
  - Zenburn
- Shortcut Mapper
  - Open Shortcut Mapper...
  - Import Notepad++ Config...
  - Export Shortcuts...
- Import
  - Import plugin(s)...
  - Import style theme(s)...
- Edit Popup ContextMenu

### Tools

Four digests, each with the same three sources. MD5, SHA-1, SHA-256 and SHA-512
are implemented in `nn-core` rather than pulled from a crate: they are the only
hashes the editor needs, they are short, and a dependency that exists to compute
them would be larger than the code.

"Generate..." hashes text the user types. "Generate from files..." hashes a file
chosen from the dialog. "Generate from selection into clipboard" hashes the
selection and puts the digest on the clipboard without touching the document,
which is the one a user actually wants when verifying a download.

- MD5
  - Generate...
  - Generate from files...
  - Generate from selection into clipboard
- SHA-1
  - Generate...
  - Generate from files...
  - Generate from selection into clipboard
- SHA-256
  - Generate...
  - Generate from files...
  - Generate from selection into clipboard
- SHA-512
  - Generate...
  - Generate from files...
  - Generate from selection into clipboard

### Macro

"Start Recording" records the *editor calls* a user's keystrokes produce, not the
keystrokes themselves: a recorded `Insert("a")` means "insert an a at every
caret", so a macro recorded on a US layout replays correctly on an AZERTY one.
Replay goes through the same methods the keyboard does, so a macro cannot do
anything a person could not, and it inherits undo grouping and caret clamping.

Recording replaces the previous macro rather than appending, because a recorder
that silently appended would be impossible to reason about. "Save Current
Recorded Macro..." writes a macro to the Run menu as a command, and "Run a Macro
Multiple Times..." asks for a count.

- Start Recording (`Ctrl+Shift+R`)
- Stop Recording
- Playback (`Ctrl+Shift+P`)
- Save Current Recorded Macro...
- Run a Macro Multiple Times...

### Run

"Run..." takes a command line, substitutes `$(NAME)` macros for facts about the
current document, and hands the result to the shell. The macro names are
Notepad++'s, so an existing command line pastes in and works:
`$(FULL_CURRENT_PATH)`, `$(CURRENT_DIRECTORY)`, `$(FILE_NAME)`, `$(NAME_PART)`,
`$(EXT_PART)`, `$(CURRENT_WORD)`, `$(CURRENT_LINE)`, `$(CURRENT_COLUMN)`,
`$(SELECTION)`, `$(NPP_DIRECTORY)`, and `$(SYS.NAME)` for an environment
variable. An unknown macro is left as literal text rather than expanded to an
empty string, because a typo that silently becomes nothing runs a command
against the wrong file.

Commands can be saved, and a saved command appears as its own row in this menu,
labelled with the command itself: a name would be a second thing to invent.

"Validate shortcuts.xml" reads a shortcuts file and reports which rows this
build does not know, which is what makes an import trustworthy.

- Run... (`F5`)
- The saved commands, one row each (shown once something is saved)
- Validate shortcuts.xml

### Plugins

"Manage Plugins..." comes first, because it is the only row here that switches a
plugin on: it lists every plugin in the folder with `Enabled` or `Disabled`
beside it, and Space toggles the row under the cursor. A newly installed plugin
starts disabled, so the manager is where a user goes first. "Open Plugins
Folder..." follows, because it is how a user gets from "no plugins" to having
one, and "Reload Plugins" sits beside it: it re-reads the folder, so a `.wasm`
file copied in by hand is discovered without a restart.

Those three rows are always there. Under them, past a separator, sits one
submenu per *running* plugin, named exactly as the plugin names itself and
holding the commands it offers: `JSON` opens onto `Pretty Print`, `Minify` and
`Validate`. That is the shape a Notepad++ user already knows, and it is the one
that scales -- a plugin may have one command or a dozen, and a dozen more rows
in this dropdown would bury the three that manage it. Moving onto a submenu
opens it and the arrows then walk the commands inside, so reaching one is a step
to the right and a few down rather than a dialog, and Enter runs the row under
the pointer.

A plugin that is switched off gets no submenu. The folder holds what is
installed, the manager says which of those are running, and a submenu that opens
onto nothing would be a third place to look for the same state. Switching a
plugin on adds its submenu at once, inside the same session and without a
reload; switching it off takes the submenu away. The same commands are reachable
over the protocol as well, as described in "Plugins over MCP".

The rows inside a submenu are literal. A plugin's name and its commands are
written by the plugin, in its own words, so no catalog renames them and nothing
is looked up in the translation tables. A command called "Undo" is that plugin's
Undo and not the editor's, and translating a stranger's vocabulary would be a
guess at what it does. The three management rows above the separator belong to
the editor and are translated like any other.

With nothing in the folder the menu still has a row saying so: an empty dropdown
reads as a bug rather than as an empty list. A folder that holds plugins gets no
such row -- saying "none installed" about five installed plugins would be worse
than saying nothing.

Plugins are described in "Plugins".

- Manage Plugins...
- Open Plugins Folder...
- Reload Plugins
- A submenu per running plugin, holding its commands (once one is switched on)
- No Plugins Installed (when the folder is empty)

### Window

The Window menu is about which document is where, not about the text. "Sort By"
orders the tab strip by name, path, type, content length or modification time,
each in both directions -- ten rows, and they sort tabs rather than lines.

"Windows..." lists every open document with its title and marks the active one,
which is the way to find a file when the strip has more tabs than fit. "Recent
Window" switches to the document that was active before this one.

- Sort By
  - Name A to Z
  - Name Z to A
  - Path A to Z
  - Path Z to A
  - Type A to Z
  - Type Z to A
  - Content Length Ascending
  - Content Length Descending
  - Modified Time Ascending
  - Modified Time Descending
- Windows...
- Recent Window

### Help

"Command Line Arguments..." opens this build's own usage text, so the switches
are discoverable from inside the program.

The four link rows are labelled "Neko++ ..." because this program is Neko++, and
a Help menu that names a different product reads as a leftover. Their
destinations are still Notepad++'s pages: they are the closest thing to
documentation for the features implemented here, and the addresses are the
owner's to change. The label and the destination disagreeing is a known,
deliberate state.

"Debug Info..." reports the renderer, the adapter, the font system, the
configuration paths and the file list, which is what a bug report needs.
"About Neko++" is the box described in "About and Credits".

- Command Line Arguments...
- Neko++ Home
- Neko++ Project Page
- Neko++ Online User Manual
- Neko++ Community (Forum)
- Debug Info...
- About Neko++

## Editing and the Caret Model

The editor is not a text box with extra keys. It has a selection model, several
carets, a rectangular mode, bracket pairing and an undo history that groups the
way a person thinks about edits.

### Carets and selections

A selection is an anchor and a caret. The anchor is where the selection started;
the caret is where it ends, so `anchor > caret` means the user selected
backwards. Keeping the direction means "extend the selection" behaves correctly
without a separate flag, and it is what lets Select and Find grow a selection one
match at a time.

More than one caret is allowed. A caret is added above or below with
`Ctrl+Alt+Up` and `Ctrl+Alt+Down`, or with Alt and a click; every caret has its
own selection, and typing, deleting, pasting and indenting apply at all of them.
Multi-select (`Edit > Multi-select All` / `Next`) adds a caret at every match of
the selection under one of four rules: ignore case and whole word, match case
only, whole word only, or both. "Undo the Latest Added Multi-Select" removes the
last caret without touching the text, and "Skip Current & Go to Next
Multi-select" drops the current match and takes the next.

Every caret is clamped to a character boundary and to the line, and the
selections are normalised and merged when they overlap. That work is in
`selection.rs` and it is tested, because overlapping carets are how a
multi-caret editor corrupts a document.

### Motions

Left, right, up, down, word left, word right, line start, line end, document
start, document end, page up and page down. Each can extend the selection
instead of moving the caret. A vertical motion keeps the column where it can,
so walking down through short lines and back into long ones returns to the
column the user was on rather than to the end of the short line.

`Home` goes to the start of the line and `End` to just past its last character;
`Ctrl+Home` and `Ctrl+End` go to the ends of the document. There is no smart-home
row: a key that goes somewhere different depending on the text under the caret is
a key whose second press has to be remembered.

### The pointer

A press in the text area puts the caret on the character under the pointer, and a
drag from that press selects from where it landed to wherever the pointer is
now. Both come from one conversion: a window position has the pane's chrome --
the menu bar, the toolbar, the tab strip, the scroll offset -- taken off it
before it becomes a line and a column, so the caret appears under the pointer and
not a few lines below it. Ctrl with a press adds a caret instead of moving the
one there is, Shift with a press extends the selection, and Alt with a press or a
drag starts a rectangle.

A click has three sizes. One press takes a character, two take the run of text
under the pointer, three take the whole line the pointer is on. A *run* is a
word, a run of spaces, or a run of punctuation: a double click on the `.` of
`foo.bar` takes the dot, and one on the spaces in `a = b` takes the spaces. A
press past the end of a line counts as a press on the last character of that
line, and on an empty line it selects nothing rather than clearing what was
already selected.

Two presses count as one click when they land within four pixels and half a
second of each other, and a third has to pass the same test against the second.
A drag that begins with a double or triple click pulls the selection out by that
same unit, so dragging from a word takes whole words and dragging from a line
takes whole lines. A line selected with three presses carries its line ending, so
copying it and pasting gives a line rather than a line glued to its neighbour.

### Typing

Typing a printable character inserts it at every caret and coalesces with the
previous keystroke into one undo step. Anything that is not typing --
a paste, a line operation, a menu command -- breaks the group, so a word is one
step and the operation before it is its own.

Brackets and quotes pair. Typing `(` inserts `()` and leaves the caret between
them; typing the closer when it is already there steps over it instead of
doubling it; typing a quote next to a word character does not pair, because
`don't` is more common than `don''t`. Auto-indent copies the current line's
indent, and adds one level after a line that ends in `{`.

`Insert` toggles overwrite mode: the next character typed replaces the one under
the caret instead of pushing it right. The mode is per document and it is
recorded by macro recording, because a macro that replayed an overwrite as an
insert would be a lie about what happened.

### Rectangular selections

`Alt` and a drag makes a rectangle rather than a linear selection. `Column Mode`
in the Edit menu turns every drag into a rectangle until it is turned off again,
which is for a user who is about to edit a table and does not want to hold Alt.
The rectangle is stored as an anchor line/column and a caret line/column, and
typing into one inserts at the same column on every line, padding with spaces
when a line is too short.

"Column Editor..." asks for text and inserts it at the caret's column on every
selected line, which is the padding-free version of the same operation.

### Deleting

Backspace and Delete remove the selection when there is one and one character
otherwise, at every caret. `Ctrl+L` deletes the current line, `Ctrl+Shift+L`
selects it. Deleting a selection that spans several carets is done as one edit,
so one undo restores it.

### Clipboard

Cut, copy and paste use the system clipboard. A copy with three carets puts the
three runs on the clipboard, one per line, in caret order. A paste puts the whole
clipboard text at *every* caret, which is what a multi-caret paste means: the
same text in several places. It does not distribute the clipboard's lines across
the carets, because that behaviour claims to know which line the user meant.

The clipboard history keeps the last ten entries. A copy identical to the newest
entry does not add a second one, because copying the same line twice is a mistake
rather than a history entry. The list is a popup, and choosing a row pastes it.

### Read-only

Read-only can be set for the current document or for every open document, and
cleared for all of them. A read-only document refuses edits at the editor level
rather than at the window level, so a plugin cannot write to it either: the
plugin bridge is told the document is read-only and returns a refusal to the
plugin.

## Line Operations

Thirty operations that all take a block of whole lines and return replacement
text. They apply to the lines the selection touches, or to the caret's line when
there is no selection, and each is one undo step.

| Group | Rows |
|---|---|
| Structure | Duplicate Current Line, Remove Duplicate Lines, Remove Consecutive Duplicate Lines, Split Lines, Join Lines, Move Up Current Line, Move Down Current Line |
| Emptiness | Remove Empty Lines, Remove Empty Lines (Containing Blank characters), Insert Blank Line Above Current, Insert Blank Line Below Current |
| Order | Reverse Line Order, Randomize Line Order |
| Sort, ascending | Lexicographic, Lexicographic ignoring case, Locale order, Integers, Decimals (comma), Decimals (dot), Length |
| Sort, descending | The same seven, reversed |

Splitting asks for a delimiter and splits every selected line at it; joining
concatenates the selected lines, dropping each line's terminator so a wrapped
paragraph becomes one line. "Remove Empty Lines (Containing Blank characters)"
removes lines that hold only spaces and tabs, which the plain row keeps.

The fourteen sorts are an enumeration rather than an options dialog because the
menu offers exactly these fourteen: there is no way for a user to ask for
"integers, descending, case-insensitively", so the type does not allow it.
Locale order uses the C library's collation, which is what makes `ä` sort next
to `a` in a German locale and at the end of the alphabet in a Swedish one.
Integer sort compares numeric values, so `10` follows `9` rather than `1`.
Decimal sort understands a comma or a dot as the separator, chosen by the row.
Length sorts by character count. Every sort is stable, so equal keys keep their
original order and a second sort by a different key behaves like a multi-key
sort.

## Find and Replace

Search is a compiled matcher over a rope. Compiling once and reusing the object
is what makes the match counter cheap enough to update on every keystroke.

### The matcher

Four combinations of two independent flags -- case sensitivity and whole word --
plus a regular expression mode. The literal path searches bytes and maps them
back to character indices, so it is fast on a large document without converting
the whole file to a string first. Whole-word matching requires a non-word
character on both sides, computed from Unicode word properties rather than from
ASCII. The regex mode compiles with the `regex` crate, is case-insensitive when
the case toggle is off, and reports an invalid pattern in the find bar instead
of failing silently: "bad regex" in the status area is the whole error UI.

Replacement text is literal -- `$1` is not a backreference -- because the
replacement is typed into the same field and a backreference that silently did
nothing would be worse than one that is not offered. "Replace All" performs one
edit, so a user who replaces two hundred occurrences can undo it in one step.

### The find bar

A single-line field, the three toggles, a match counter showing `n of m`, and
the four buttons. The bar is docked along the bottom of the pane, above the
status bar. `Enter` finds the next match, `Shift+Enter` the previous, `Escape`
closes the bar and returns the keyboard to the text.

Case-insensitivity is the default, which is what a user typing a word into a
file expects. The counter turns red when there is no match, which is the only
feedback a modal-less bar can give.

Closing the bar drops its highlights when "Volatile Find" is on, which is the
default: a search you have finished with should not leave the document speckled.
Turning it off keeps the highlights, which is what you want when the point of the
search was to see every occurrence at once.

### Incremental search

`Ctrl+Alt+I` opens the find bar in incremental mode: as the pattern grows, the
first match from the anchor is selected and scrolled into view, and the caret
returns to where it started if the search is cancelled. The anchor is the caret
and scroll position at the moment the search began, so `Escape` is a true undo of
the navigation.

### Search results

Find-in-files writes one row per matching line into a docked panel: the file, the
line number, and the line's text with the match marked. Clicking a row opens the
file and puts the caret on the match. The panel has its own height, grows with
the results, and is capped so it cannot take the whole window. "Search Results
Window" toggles it, and "Next Search Result" / "Previous Search Result" walk it
from the keyboard.

### Searching a directory tree

The walk is depth-first and sorts each directory before descending, so the same
tree gives the same result order twice and the results panel does not jump around
between runs. Five bounds keep it from hanging the program or drowning the user:
a maximum depth, a maximum number of hits, a maximum file size, dot-directories
and a small list of always-skipped ones (`.git`, `node_modules`, `target`), and
one visit per file by canonical path, so overlapping project roots do not report
the same file twice. A directory that cannot be read is skipped rather than
aborting the walk, and a file that is not valid UTF-8 is skipped rather than
searched: it is not text this editor can show, so it is not text it should report
matches in.

The optional extension filter is a glob with `*` and `?` and nothing else,
hand-written because a glob crate would be a third-party parser in the path of
every file in the tree.

## Bookmarks

A bookmark belongs to a line, not to an offset. That is the whole design: an edit
above a bookmarked line moves the line's number and the bookmark moves with it,
and an edit that deletes the line deletes the bookmark with it. Storing offsets
would have been simpler and would have left bookmarks pointing into the middle of
words after the first insert.

| Row | Effect |
|---|---|
| Toggle Bookmark | Sets or clears the mark on every selected line |
| Next Bookmark / Previous Bookmark | Moves the caret to the next or previous marked line, wrapping at the ends |
| Clear All Bookmarks | Removes every mark in the document |
| Cut Bookmarked Lines | Removes the marked lines and puts them on the clipboard |
| Copy Bookmarked Lines | Copies the marked lines, leaving the document alone |
| Paste to (Replace) Bookmarked Lines | Replaces each marked line with the clipboard's lines |
| Remove Bookmarked Lines | Deletes the marked lines |
| Remove Non-Bookmarked Lines | Deletes everything else |
| Inverse Bookmarks | Marks the unmarked lines and unmarks the marked ones |

The marks are drawn in the gutter as a filled square, in their own margin
between the line numbers and the fold markers. They are not written to the file:
a bookmark is a view of the session, not a change to the document, so bookmarks
do not make a document dirty and do not survive a close.

## Folding

Fold levels are derived, not stored. A folder walks the document once and writes
a level per line, using Scintilla's bit layout, so the renderer can ask the same
questions it would ask of Scintilla: is this line a block header, is it blank,
how deep is it.

The brace folder reads the style array the highlighter already produced, so a
`{` inside a string or a comment does not open a block. That is the whole reason
folding runs after highlighting rather than during lexing: a fold computed from
raw text would open a block at a brace in a string literal, and the user would
see a file folded in the wrong places.

Folding is incremental in the same way highlighting is. An edit re-folds from
the edited line and stops as soon as a line comes out with the level and depth it
already had, because a line's level depends only on the depth it starts at and
its own text. Typing inside a function therefore costs a handful of lines, not
the whole file.

| Row | Effect |
|---|---|
| Fold Current Block (`Ctrl+Alt+F`) | Toggles the fold on the caret's own block |
| Fold All (`Ctrl+Alt+A`) | Folds every block in the document |
| Unfold All (`Ctrl+Alt+Shift+F`) | Unfolds everything, at every level |
| Fold Current Level | Folds every sibling at the depth the caret sits at |
| Unfold Current Level | The same, unfolding |
| Fold Level 1-8 | Folds everything deeper than the chosen level |
| Unfold Level 1-8 | Unfolds everything up to the chosen level |
| Hide Lines | Folds every foldable block the selection touches |

A foldable line draws a box with a minus in the fold margin, and a folded one
gains a vertical stroke; the block's other lines are not drawn. The caret cannot
stay inside a folded block: a motion into one expands it, which is what stops a
keystroke from editing text nobody can see.

"Hide Lines" is the closest reversible equivalent of Notepad++'s row of the same
name. Notepad++ hides exactly the selected lines; this build's fold model is
block-based, so the selection's foldable blocks are collapsed instead. Nothing is
deleted, and "Unfold All" brings it back. That is a deviation and it is stated
here rather than papered over.

## Macros

A macro is a recorded sequence of *editor calls*, not of keystrokes. A recorded
`Insert("a")` means "insert an a at every caret"; a recorded key would mean
"press the key labelled A", which replays wrongly on a different keyboard
layout.

The recorded steps are: insert text, overwrite one character, backspace, delete
forward, delete the selection, move the caret (with or without extending),
select all, undo and redo. Those are the operations a keystroke produces, and
they are enough to record ordinary editing without recording the mouse, the
menus, or anything that depends on where a window is.

Replay goes through the same editor methods the keyboard does, so a macro cannot
do anything a person could not, and it inherits every guarantee those methods
make: undo grouping, caret clamping, selection normalisation, read-only refusal.

| Row | Effect |
|---|---|
| Start Recording (`Ctrl+Shift+R`) | Begins a new recording, discarding the old one |
| Stop Recording | Ends it, keeping the steps |
| Playback (`Ctrl+Shift+P`) | Replays the steps once |
| Save Current Recorded Macro... | Writes the macro into the Run menu as a command |
| Run a Macro Multiple Times... | Asks for a count and replays that many times |

Recording replaces the previous macro rather than appending. A recorder that
silently appended would be impossible to reason about: the third recording of a
session would contain edits from the first. The status bar shows the step count
while recording, so it is visible that something is being captured.

Saving a macro turns it into a Run-menu command, which is Notepad++'s model: the
saved macro is a command line, and it is stored in the settings file with the
other saved commands.

## Languages and Syntax Highlighting

One table describes 147 languages, rather than 147 lexers. Each entry says how a
line comment starts, whether there is a block comment pair, which characters
quote strings, which words are keywords, and whether keywords match without
regard to case. That is enough for the languages people open in a text editor,
and it means adding a language is a table entry rather than a new module.

The lexer is stateful because constructs span lines: a block comment opened on
line 3 is still a comment on line 40, and a raw string can carry its own quoting
across a hundred lines. The lexer emits one style byte per character, using the
same nine built-in style classes the theme colours: default, keyword, type,
string, comment, number, function, operator and preprocessor.

Highlighting is incremental. Editing a line re-lexes from that line and stops as
soon as the lexer's state at the end of a line matches the state stored for it;
everything below is still correct. A large file therefore costs a handful of
lines per keystroke rather than a full re-scan.

### The languages

Every built-in language, as the Language menu groups them. Three are outside the
letter groups, exactly where Notepad++ puts them: KIXtart after J, and XML and
YAML after V.

| Group | Languages |
|---|---|
| A | ActionScript, Ada, APL, AsciiDoc, ASN.1, ASP, Assembly, Astro, AutoIt, AviSynth, AWK |
| B | BaanC, Batch, BibTeX, Blade, Blitzbasic |
| C | C, C#, C++, Caml, Clojure, CMake, COBOL, Crystal, CSound, CoffeeScript, CSS, CUDA |
| D | D, Dart, Diff, Dockerfile |
| E | Elixir, Elm, Erlang, ErrorList, Escape Sequence (ANSI), ESCRIPT |
| F | F#, Fish, Forth, Fortran (free form), Fortran (fixed form), Freebasic |
| G | GDScript, Gleam, GLSL, Go, GraphQL, Groovy, Gui4Cli |
| H | Haskell, HCL, HLSL, Hollywood, HTML |
| I | INI file, Inno Setup, Intel HEX |
| J | Java, JavaScript, Jinja, JSON, JSON5, JSP, Julia |
| KIXtart | *(not grouped)* |
| K | Kotlin |
| L | LaTeX, Less, LISP, Lua |
| M | Makefile, Markdown, Matlab, Meson, Microsoft Transact-SQL, MMIXAL, MS-DOS Style, Mustache, MySQL |
| N | Nim, Ninja, Nix, Nncrontab, NSIS, Nushell |
| O | Objective-C, Org, OScript |
| P | Pascal, Perl, PHP, PostgreSQL, PostScript, PowerShell, Prolog, Properties, Protocol Buffers, Puppet, Purebasic, PureScript, Python |
| R | R, Racket, Raku, REBOL, Registry, Resource file, reStructuredText, Ruby, Rust |
| S | SAS, Sass, SCSS, Sed, Shell, Scheme, Smalltalk, Solidity, Spice, SQL, Structured Text, Svelte, Swift, S-Record, SystemVerilog |
| T | TCL, Tektronix extended HEX, TeX, Thrift, TOML, txt2tags, TypeScript, Typst |
| V | Vala, Vim script, Visual Basic, Visual Prolog, VHDL, Verilog, Vue |
| XML | *(not grouped)* |
| YAML | *(not grouped)* |
| W | WGSL, Wolfram |
| Z | Zig |

### Comment syntax

"Comment/Uncomment" uses the active language's own markers: `//` and `/* */` in
C, `--` and `{- -}` in Haskell, `#` in a shell script, `<!-- -->` in HTML, `;`
in an INI file. Toggling adds the marker to every selected line whose language
has a line comment and removes it from a line that already carries one, so the
same keystroke goes both ways. Block comment and uncomment use the language's
pair, and a language with no block pair greys the rows out rather than pretending.

### User-defined languages

Notepad++'s `userDefineLang.xml` describes a lexer as data: keyword groups,
delimiter pairs, prefix and suffix modes, colours per style. Neko++ reads that
file. A user-defined language can declare eight keyword groups and eight
delimiter pairs, and Notepad++ colours each separately, so the palette is the
theme plus one colour per style index, built once per document rather than once
per frame.

The one judgement call is contrast. A language file is written against *some*
theme, usually Notepad++'s light default, and applying its colours unchanged on a
dark theme produces text that cannot be read. The palette therefore enforces a
minimum contrast against the theme's background and lifts a colour that fails
it. That is a deviation from Notepad++ and it is deliberate: unreadable text is
not a feature.

A user-defined language claims file extensions, so opening `foo.ini` selects the
user's INI language rather than the built-in one when both claim it. The
installed languages appear under Language > User-Defined, and the submenu exists
even when it is empty.

### The function list

A docked column on the right edge listing the symbols `nn-core`'s scanner found
in the active document, indented by nesting depth. Clicking a row puts the caret
on the definition line.

The scan is not a parser. It reads one line at a time and matches the shapes that
declare a named thing in the languages people open in Notepad++: Rust `fn`, C and
C++ function definitions, Python `def` and `class`, JavaScript `function` and
arrow assignments, shell functions, SQL `CREATE`, and the tag-like shapes of
Markup. It is deliberately cheap: a parser would need a grammar per language, and
the list is a navigation aid rather than a compiler.

### Word completion and parameter hints

`Ctrl+Space` completes from the words already in the document. There is no
dictionary, no language server and no model: the candidate list is the distinct
words of three characters or more that start with the prefix before the caret,
sorted, capped at 200, with ten rows visible. The index is built on demand rather
than maintained, because a completion request is a keystroke rather than a frame
and there is no cache to invalidate. Document completion offers the same list
pooled across every open file, stopping at four megabytes of pooled text: it is a
convenience, and copying a hundred megabytes per keystroke would make it a
liability. Completion is disabled for documents the editor considers large, and
the status bar says so rather than showing an empty popup.

Function completion (`Ctrl+Shift+Space`) lists the function declarations the
symbol scan found, filtered by the same prefix. Parameter hints
(`Ctrl+Shift+Alt+Space`) show a one-line signature of the call the caret sits
inside, with the argument being typed in the accent colour; the previous and next
hints walk the arguments without moving the caret. It is a label rather than a
list, because giving it a selection would invite the arrow keys to move a
highlight that means nothing.

Path completion (`Ctrl+Alt+Space`) completes a filesystem path from the text
before the caret: directories first with a trailing separator, then files, each
group sorted, hidden entries skipped unless the prefix starts with a dot.

## Encodings and Line Endings

A document's encoding decides how its bytes become characters; its line ending
decides what a line break is on disk. The two are independent, and both are shown
in the status bar because guessing wrong about either is the most common way a
text file looks corrupted.

### Encodings

Two families. The Unicode ones -- UTF-8 with and without a byte-order mark, and
UTF-16 in both byte orders -- are variable-width and carry their own BOM where
they have one. The single-byte ones are ASCII in the low half and a 128-entry
table in the high half, so decoding is a table lookup per byte and encoding is
the reverse.

| Encoding | Notes |
|---|---|
| ANSI | Windows-1252, which is what Notepad++ means by ANSI |
| UTF-8 | No byte-order mark |
| UTF-8-BOM | With a byte-order mark, for tools that demand one |
| UTF-16 BE BOM / LE BOM | Both byte orders, always with a mark |
| Character sets | 41 single-byte code pages in 13 script groups |

The 13 character-set groups are Arabic, Baltic, Celtic, Cyrillic, Central
European, Eastern European, Greek, Hebrew, North European, Thai, Turkish,
Western European and Vietnamese; they hold ISO 8859-1 through 8859-15, the OEM
code pages from CP437 to CP869, Windows-1250 through Windows-1258, KOI8-R,
KOI8-U, Macintosh Cyrillic and TIS-620.

Reinterpreting and converting are separate commands on purpose. "ANSI",
"UTF-8" and the character sets *reinterpret*: the file is untouched and the same
bytes are read again under a different assumption, which is how a file whose
encoding was guessed wrong is recovered. "Convert to ..." re-encodes the text in
memory, so the next save writes different bytes. A character that the target
encoding cannot represent is written as a substitution rather than silently
dropped, and the status bar reports how many were replaced.

### Line endings

Documents are normalised to `\n` internally so the rope never has to deal with
multi-byte terminators. The original mode is remembered and restored on save, and
"EOL Conversion" changes it: Windows (CR LF), Unix (LF) and Macintosh (CR).

The status bar shows the mode, and it becomes an indicator rather than a
preference the first time a file is saved: a document opened as CRLF stays CRLF
until the user says otherwise, because silently rewriting every line of somebody
else's file is not a helpful default.

A file with mixed endings takes the majority mode, which is what a file that was
mostly written on one platform and then edited on another looks like. "EOL to
Space" in Blank Operations replaces a line's terminator with a space, which is the
operation wanted when joining wrapped text by hand.

## Themes and the Style Configurator

A theme carries about sixty colours because every widget needs one. Nobody picks
sixty colours; a theme is designed as ten and derived. The catalogue holds 47
palettes, and with System, Light and Dark there are 50 choices in the Theme menu
and the preferences dialog.

| Choice | What it is |
|---|---|
| System | Follows the desktop's colour-scheme preference; dark when the desktop has no opinion |
| Light | The built-in light palette, hand-tuned to Notepad++'s colours |
| Dark | The built-in dark palette, from the Visual Studio Code family |
| 47 named palettes | Ayu (3), Catppuccin (4), Everforest (2), GitHub (3), Gruvbox (2), Kanagawa (3), Material (2), Monokai (2), Night (2), One (2), Rosé Pine (3), Solarized (2), Tokyo Night (3), and 15 more |

The full list: Ayu Dark, Ayu Light, Ayu Mirage, Catppuccin Frappé, Catppuccin
Latte, Catppuccin Macchiato, Catppuccin Mocha, Darcula, Dracula, Everforest,
Everforest Light, GitHub Dark, GitHub Dark Dimmed, GitHub Light, Gruvbox Dark,
Gruvbox Light, Horizon, Iceberg, Kanagawa, Kanagawa Dragon, Kanagawa Lotus,
Material Ocean, Material Palenight, Molokai, Monokai, Monokai Pro, Moonlight,
Night Owl, Nightfox, Nord, Oceanic Next, One Dark, One Light, Panda Syntax,
Quiet Light, Rosé Pine, Rosé Pine Dawn, Rosé Pine Moon, Shades of Purple,
Solarized Dark, Solarized Light, Synthwave '84, Tokyo Night, Tokyo Night Day,
Tokyo Night Storm, Vitesse Dark and Zenburn.

Colours are authored as sRGB bytes, because that is how designers and colour
pickers think. The GPU wants linear values, so the conversion happens once, in
`theme.rs`, rather than in every widget.

"Toggle Theme" (`Ctrl+Alt+T`) switches between the light and the dark palette,
which is the one theme operation that deserves a chord: reading a file in
daylight and at night is the same file.

### The Style Configurator

Sixteen slots: the nine syntax classes (default, keyword, type, string, comment,
number, function, operator, preprocessor) and seven pieces of furniture
(background, gutter, foreground, selection, caret, current line, line number).
The dialog is a list of the slots with a colour swatch each; choosing one opens
a red-green-blue picker, and the change is applied live so the dialog can be
judged against the text behind it.

Overrides are stored sparsely: only slots that differ from the theme's palette are
written to the settings file. That is what makes switching between light and dark
keep the user's edits on top of whichever base they picked. A theme import from
a Notepad++ `stylers.xml` is stored as a path rather than as copied colours, so
an edit to the file takes effect without a second import; a path that no longer
exists falls back to the built-in pair rather than failing to start.

## Preferences

A centred list of rows, each edited in place: left and right on a row step a
choice, Enter on a switch toggles it, and the dialog edits a *copy* of the
settings which is committed or discarded as a whole. That contract is the same
one the shortcut editor uses, and it is what makes Escape mean "forget it".

| Row | Values | Default |
|---|---|---|
| Theme | The 50 choices | System |
| Font Size | Pixels, clamped to a sane range | 15 |
| Tab Width | Columns, clamped | 4 |
| Word Wrap | On or off | Off |
| Minimap | On or off | On |
| Toolbar | On or off | On |
| Function List | On or off | Off |
| Volatile Find | On or off | On |
| Backup on Save | None, Simple, Verbose | None |
| Autosave | Seconds between snapshots; zero is off | 7 |
| File Dialog | System dialog (XDG portal) or built-in browser | System dialog |
| Language | English or one of the 23 bundled localisations | English |
| Shortcuts | Opens the shortcut mapper | -- |

Two defaults are worth a sentence. Backup on save is off, because writing `.bak`
files into somebody's source tree without being asked is a surprise and version
control already exists. Autosave is on, because it writes only into the editor's
own directory, and it is the difference between a crash costing an hour of work
and costing nothing.

Backup modes: `Simple` writes `foo.txt.bak` beside the file before overwriting
it; `Verbose` writes a timestamped copy into the application's backup directory
and keeps the last ten per file, deleting the oldest. The document's read-only
state, the zoom, the split and the tab layout are not in this dialog: they belong
to a view rather than to the program, and they are reachable from the View menu.

The file dialog choice exists because neither answer is right everywhere. The
XDG portal is the native chooser and respects the desktop's bookmarks, but it
needs a session bus and a portal service; the built-in browser is a modal list of
the directory's contents, looks the same everywhere, and costs nothing at
startup. A machine with no portal gets the built-in browser and a status message
rather than a dead button.

Both open dialogs take more than one file. The portal is asked for a multiple
selection, so the desktop's own gestures apply; in the built-in browser a click
selects a file, Ctrl+click adds or removes one, Shift+click takes a range and
Ctrl+A takes every file in the listing. Enter opens the whole selection, each
file in its own tab, with the last one in front. Save As stays single, because
one buffer is one file.

Which file is "last" is the one the dialog answered last, because the answer is
opened in order and each file activates the tab it creates. The built-in browser
answers in the order the rows were marked, so the file clicked last is the one in
front. The portal answers in an order of its own choosing -- normally the
listing's, so the last name in the answer wins -- and there is no way to ask a
portal which file was clicked last.

## Sessions, Backups and Crash Recovery

Three mechanisms, for three different failures. A session is what you meant to
come back to; a backup protects the file you had; a snapshot protects the work you
have not saved yet.

### Session

`session.txt` holds the recent file list (ten entries, most recent first) and
the open tabs with their carets, their pinned state and their colour, plus which
tab was active. The format is line-oriented and deliberately dumb: a keyword, a
space, and the rest of the line, with numbers before paths so a path containing
spaces needs no quoting.

The file is written when the editor exits and by "Save Session..." at any time.
Both write the same format, so a named session is a file a user can keep, copy
between machines, or check into a repository, and the resume file is just the
session the editor wrote for itself. A session is therefore a document rather
than a hidden state.

### Backups

A save is the one operation in an editor that destroys data on purpose: the bytes
on disk are replaced by whatever is in the buffer. The backup modes are described
in "Preferences". A backup is taken before the write, and a backup that cannot be
written is reported and then ignored: refusing the save the user asked for
because a directory is full loses more than it protects.

### Snapshots

While the editor runs, a timer writes every modified buffer -- and only the
modified ones -- into the application's snapshot directory, seven seconds apart
by default. The snapshot is a list plus one file per buffer, and it is pruned
when the editor closes cleanly, so its presence at startup means the last run did
not.

On startup a snapshot that is not empty is offered for recovery: each buffer is
restored as an unnamed document with its original path remembered for the first
save. A buffer whose file has changed on disk since the snapshot is offered
anyway and the difference is reported, because the alternative is deciding on the
user's behalf which copy is more valuable.

The distinction from a backup matters. A backup writes into the user's own
directory, which is why it is opt-in; a snapshot writes only into ours, and it is
on by default.

## Split View

Split view is the one feature that changes the shape of everything else: two
documents on screen at once, each with its own scroll position, its own tab
strip, and its own idea of where the caret is.

| Row | Effect |
|---|---|
| Split Vertically (`Ctrl+Alt+Shift+V`) | Two panes side by side |
| Split Horizontally (`Ctrl+Alt+Shift+H`) | Two panes stacked |
| Rotate Split | Swaps the orientation, keeping both panes' contents |
| Remove the Split (`Ctrl+Alt+Shift+U`) | Back to one pane; the other pane's tabs move into this one |
| Move to Other View (`Ctrl+Alt+Shift+Right`) | Moves the active document to the other pane |
| Clone to Other View (`Ctrl+Alt+Shift+Left`) | Opens a second view of the same buffer |
| Focus on Another View | Moves the keyboard to the other pane |
| Synchronize Vertical Scrolling | The panes scroll together vertically |
| Synchronize Horizontal Scrolling | The panes scroll together horizontally |
| Synchronize Across Views (Zoom) | A zoom change applies to both panes |

A clone is a second view of the same document, not a copy: an edit in one appears
in the other, the undo history is shared, and closing one leaves the other open.
That is what makes a split useful for comparing two parts of one long file.

Scroll synchronisation is a preference as well as a menu toggle, and it is off by
default: two panes usually hold two different files, and making one move should
not move the other. When it is on, the panes are tied proportionally rather than
by line number, so two files of different lengths stay roughly aligned.

The layout arithmetic is a pure function with tests, because a click that lands
in the wrong document is the kind of bug nobody notices until it edits the wrong
file.

## Panels and Docks

Every panel can be closed, and none of them is open by default except the
minimap: a text editor's job is the text.

### Folder as Workspace

A tree of one directory, docked on the left, with a filter field. Directories
expand on click, files open on double click, and the tree is rescanned when the
panel is opened or refreshed. The scan is depth-limited and skips hidden entries,
symbolic links and anything that is not readable, because a tree that follows a
symlink loop is a tree that hangs the editor.

A filter is a substring match on the name, applied to the flattened rows rather
than to the tree, so a match deep inside an unexpanded directory still appears.

### Project panels

Three project slots, each holding one named project, each project a tree with as
many roots as it has directories. The tree is the same widget the folder panel
uses. Projects are stored in `projects.conf` and written on every change rather than on
exit, because a project list is a deliberate arrangement and losing it to a
killed process would be a surprise. "Find in Projects" searches every root of
every project, and the search deduplicates by canonical path so a project that
lists both a directory and something inside it does not report twice. That is the
Notepad++ model exactly: a project is a named list of directories, not a build
system.

### Document Map

The minimap: a compressed picture of the whole document in a narrow column on
the right edge, built from the same per-character styles the editor uses. Each
pixel row stands for a band of document rows, and the longest of three sampled
rows in the band is drawn as a handful of style runs, which is enough to read the
shape of a file at a glance. Clicking or dragging in the column scrolls the
editor, which makes it a scrollbar with content.

The picture is cached and rebuilt only when the document changes. Reaching into
a rope is a tree descent per line, so doing it per frame would cost more than
drawing the text itself.

The column is as tall as the document needs and no taller. It grows at two
pixels a row and stops at the bottom of the pane, so a long file still fills the
edge while a short one leaves most of the edge empty. A five-line file used to be
spread over the whole column, which made it read as a five-hundred-line one.

### Document List

Every open document with its title, the active one marked. Choosing a row
switches to it. It is the answer to "which of these forty tabs is the one I
want", and it is the same list as Window > Windows.

### Change History

A docked list of every change made to the current document, newest at the
bottom, built from the undo stack. Each row says when the change happened, where
it was, and what it did; clicking a row reverts the document to the state it was
in after that change, which is what makes the undo stack navigable rather than
linear. "Clear Change History" empties it.

### Character panel

A grid of the 256 code points of Latin-1, one cell each, with a click inserting
the character at the caret. It exists for the characters a keyboard cannot
produce: the box-drawing set, the accented letters of a language the layout is
not set to, the currency signs. The panel stays open after a click, because
picking one character from a grid is rarely the whole errand -- the box-drawing
set is used a row at a time.

### Clipboard history

Ten entries, newest first, one line per row so a multi-line copy cannot push the
popup off the screen. Choosing a row pastes it. Consecutive duplicates are
collapsed.

## Printing

Printing is two halves. The layout and the PDF are pure functions in `nn-core`;
the window half is a preview sheet, a page count and a handoff to the system
printer.

The editor is monospace, so a page is a grid: a fixed number of columns across
and a fixed number of rows down, both derived from the paper size and the font
size rather than hard-coded. Long lines either wrap at the page width or are
clipped, and the preview says which. Tabs are expanded to the document's tab
width before measuring, because a tab is not a character on paper.

Five paper sizes: A4, A5, Letter, Legal and Tabloid, in portrait or landscape.
The preview shows one sheet with the header and footer in their real places.

Headers and footers are templates with three zones each, separated by tabs, and
these fields: `$(PAGE)`, `$(NB_PAGES)`, `$(FILE_NAME)`, `$(FULL_CURRENT_PATH)`,
`$(CURRENT_DATE)` and `$(CURRENT_TIME)`. An unknown token is left as it stands
rather than blanked, because a header reading `$(PAGE_OF)` is a visible mistake
while one reading nothing at all looks like a broken feature. `$(PAGE)` in the
footer is the default, so a printed document is numbered without asking.

There is no print backend in the process. The way to get ink on paper on Linux is
to hand a file to CUPS, and the format CUPS handles best without a driver
dependency is PDF, so `nn-core` writes one. "Print..." opens the preview, where
the paper, the orientation, the line-number gutter and the header and footer are
chosen; "Print Now" sends the current setup straight to the printer.

## Plugins

Notepad++ plugins are DLLs loaded into the process. That is a Windows answer to a
Windows problem: it needs a compiler per platform, it gives the plugin the run of
the address space, and a bad plugin takes the editor down with it. Neko++ takes
the other road.

What that road looks like from the menu bar is one submenu per running plugin,
named by the plugin and holding its commands. The rest of this chapter is what
sits behind that submenu: the file a plugin is, the interface it is written
against, the five that ship, and the switch that decides whether any of it runs.

### What a plugin is

A plugin is a `.wasm` file in the `plugins` directory under the configuration
directory. It is interpreted with a bounded stack, bounded memory and a step
budget, and the only way it can touch the editor is through fifteen host
functions. A plugin that traps, loops or reads out of bounds loses its own command
and nothing else.

The interpreter is written in `nn-core` rather than pulled from a crate: the
build is offline, no WebAssembly runtime is vendored, and the subset a
text-editing plugin needs is small. It implements the MVP instruction set minus
the parts that need a table -- `block`, `loop`, `if`/`else`, `br`, `br_if`,
`br_table`, `return`, `call`, `drop`, `select`, locals, one memory, the integer
and floating-point arithmetic and comparisons, and the conversions between them.
`call_indirect`, globals, tables and the reference types are rejected at load
with an error naming what was found.

There is no validator and no JIT. Values carry their type at run time and an
instruction that finds the wrong one traps, so a malformed module fails on the
instruction that is wrong rather than at load. A command that walks a large
document is interpreted, which is slow enough to notice and fast enough for the
commands plugins actually implement.

### The ABI

A plugin exports its name, its command count, its command names and one `run`
entry point:

| Export | Signature | Meaning |
|---|---|---|
| `npp_name` | `() -> i32` | Pointer to a NUL-terminated UTF-8 name |
| `npp_command_count` | `() -> i32` | How many commands the plugin offers |
| `npp_command_name` | `(i32) -> i32` | Pointer to a command's name |
| `npp_run` | `(i32) -> i32` | Run command `n`; zero is success |

Host functions live in the import module `npp`:

| Import | Signature | Meaning |
|---|---|---|
| `log` | `(ptr, len)` | Append a line to the plugin's log |
| `text_len` | `() -> i32` | Document length in bytes |
| `text_read` | `(offset, ptr, len) -> i32` | Copy bytes out; returns the count |
| `text_replace` | `(ptr, len) -> i32` | Replace the whole document |
| `text_insert` | `(offset, ptr, len) -> i32` | Insert at an offset |
| `text_replace_range` | `(offset, remove, ptr, len) -> i32` | Replace `remove` bytes at `offset` |
| `caret` | `() -> i32` | The caret's byte offset |
| `set_caret` | `(offset)` | Move the caret |
| `line_count` | `() -> i32` | How many lines the document has |
| `line_start` | `(line) -> i32` | Byte offset of a line's first byte |
| `line_len` | `(line) -> i32` | A line's length, without its terminator |
| `line_of_pos` | `(offset) -> i32` | Which line a byte offset is on |
| `sel_start` | `() -> i32` | The selection's lower byte offset |
| `sel_end` | `() -> i32` | The selection's upper byte offset |
| `set_sel` | `(start, end)` | Select from `start` to `end` |

`text_replace`, `text_insert` and `text_replace_range` return zero on success and
non-zero on refusal -- a read-only document, or an offset past the end -- so a
plugin can tell the difference between "done" and "the editor said no" without a
second call. Strings are capped at 256 bytes: a name is a label in a menu, and a
plugin that wants to return a kilobyte of text has misunderstood the interface.

Everything crossing the boundary is a byte offset, because a plugin measures the
UTF-8 it is handed in bytes; the editor counts characters and converts at the
boundary. Line functions take a line index rather than a byte offset, as their
names say, and return bytes again.

The last eight arrived with the commands that need them. A command worth writing
is about a *range* -- surround this selection, align these lines, convert the
width of this run -- and with only whole-document functions each one has to copy
the document in, find its range in the copy, rebuild the string and push it back,
which loses the selection and leaves a full-document replacement in the undo
stack. The list is append-only: a module imports the names it was built against,
and instantiation fails if one is missing, so no name here ever changes meaning.

A plugin gets a scratch copy of the document, the caret and a log rather than
the document itself. The bridge is read back afterwards and the change applied in
one step, which is what makes a plugin's edit a single undo entry instead of a
hundred, and what makes a command that traps halfway through leave the text
untouched.

### Writing a plugin

The supported way to write one is the `nn-plugin` SDK in `plugins/sdk`: `plugin!`
generates the four exports, and `host` is the fifteen imports as typed Rust
functions, so a command is a function from `&str` to `String` plus a decision
about the range it applies to. `plugins/build.sh` builds the workspace for
`wasm32-unknown-unknown` and copies the results into the plugins directory.

Five ship as worked examples: `align` (Align on Equals, on Colon, on Comma),
`halfwidth` (To Half Width, To Full Width), `surround` (quotes, brackets,
parentheses, Select Quoted Text), `json` (Pretty Print, Minify, Validate) and
`xml` (Pretty Print, Linearize, Check Well-Formedness, Escape Entities, Unescape
Entities). They are implementations of the commands described in this reference,
not copies of anyone's source.

The ABI itself is plain WebAssembly with integer imports, so another language
could target it from the tables above. No SDK ships for one, and every plugin in
the tree is Rust, because that is what the editor and its tests are built with.

### Load and unload

Disabling a plugin takes its commands out of the menu and drops its module,
keeping only its name and the file it came from. Enabling reads that file again
and instantiates it; re-reading is deliberate, because a plugin rebuilt while the
editor was open should come back as the new build rather than as the bytes that
were already in memory.

What is persisted is the name. `settings.txt` holds one `plugin <name>` line per
enabled plugin and the names are applied at startup, so the set that is on
survives a restart. The folder is what is installed, the settings file is which
of those are running, and everything else starts off: a `.wasm` that arrived a
minute ago should not be handed the document on trust.

Names rather than paths, because a list of paths breaks when a plugin is rebuilt
and reinstalled. A name that no longer matches a file on disk is ignored rather
than reported as an error, which is what makes deleting a plugin a matter of
deleting its file. "Reload Plugins" re-scans the folder and re-enables by name,
which is also how a `.wasm` copied in by hand is picked up.

A file that has disappeared since it was installed is reported rather than
forgotten: the plugin stays in the list, switched off, and switching it on again
says why it cannot.

Loading and unloading are not menu rows of their own. They were, one
`Load <name>` per disabled plugin and one `Unload <name>` per enabled one, and
the pair made the Plugins dropdown a second, worse copy of the manager: the rows
shifted every time one of them was used, so the row the user was aiming at was
not the row under the pointer, and there were two places to look for the same
state. The manager is the one place now. The dropdown keeps the three rows that
open it and gives the space under them to one submenu per running plugin, which
is where a user who wants a command looks.

### Managing plugins

"Manage Plugins..." opens a list of every plugin in the folder with its state
beside it, `Enabled` or `Disabled`, and a line under the cursor naming the file
the row came from, so two plugins with similar names can be told apart. The
arrows move, Space toggles the row and applies it at once -- the menu behind the
list is rebuilt then, so there is no restart and no reload in between -- and Esc
closes. Everything found is listed, not only what is running, because a manager
that hides the disabled half is a manager for people who have already enabled
them.

"Open Plugins Folder..." opens the directory, and "Import plugin(s)..." in the
Settings menu copies a `.wasm` file into it. Switching a plugin on gives it a
submenu of its own in the Plugins dropdown and puts its commands within reach of
`npp-mcp`; switching it off takes both away again. A file that fails to load is
recorded rather than skipped silently: a plugin whose commands never run is a
bug report waiting to happen, and the log says which file and why.

### Plugins over MCP

A submenu is one door onto a plugin's commands, and it is the one that needs a
window. `npp-mcp` is the other: it serves the plugins this machine has, so those
same commands are reachable from anything that speaks the Model Context
Protocol. It reads the same directory the editor reads and honours the same
`plugin <name>` lines in `settings.txt`: a plugin switched off in the manager is
switched off over the protocol, because a way round a switch is not a feature.

`list_plugins` answers with one row per installed plugin -- its name, its file,
whether it is on, and the commands it offers -- and with the arguments to use
when one of them is runnable. `run_plugin_command` takes `plugin`, `command` and
`text`, runs that command over the text with the interpreter and the host
functions the editor uses, and answers with what the plugin left. Offsets in
`caret` and `selection` are bytes, as everywhere on the plugin side of the
boundary, and a selection may name its ends in either order.

The first content block is the document and nothing else, so a caller that asked
for minified JSON can use the answer as it stands. When the plugin reported
something -- "valid JSON", the line and column of a parse error, a refusal
because the document is read-only -- that goes in a second block, and a command
that only moved the caret or the selection says where it left them: a protocol
client has no caret to look at.

The folder is read again when a call needs something it does not have, so a
plugin switched on since the server started is usable by the next call rather
than by the next process.

## Localisation

Twenty-three localisations ship compiled into the binary: Chinese (simplified and
traditional), Czech, Danish, Dutch, Finnish, French, German, Greek, Hebrew,
Italian, Japanese, Korean, Norwegian, Polish, Portuguese (Brazil and Portugal),
Romanian, Russian, Spanish, Swedish, Turkish and Ukrainian.

A file in the `localization` directory still wins over the bundled one of the
same name. That is what makes a translation editable: copy `german.xml` out, fix
a label, and the fix is picked up without recompiling.

### The file format

Notepad++'s own localisations stopped being readable at version 8: they key on a
`menuId` attribute, and the ids they use are not the labels the editor draws.
Reading them would need a table mapping several hundred ids to English text.

Neko++ reads two schemas instead. A file whose `id` is a number is a Notepad++
file, and it is paired with `english.xml`: reading both gives `"New" ->
"Nouveau"` without a single hard-coded id. That is why a Notepad++ localisation
needs its English counterpart present. A file whose `id` is a word is a direct
mapping -- `<Item id="new" name="Nouveau"/>` translates the action named `new` --
which needs no pair, no id table, and survives a menu item being renamed in
English.

```xml
<NotepadPlus>
  <Native-Langue name="French" filename="french.xml">
    <Menu>
      <Item id="new" name="Nouveau"/>
      <Item id="open" name="Ouvrir..."/>
    </Menu>
  </Native-Langue>
</NotepadPlus>
```

### What is translated

Every menu label, every toolbar tooltip, every dialog row, every status message
with a fixed word in it, and the panel titles. Words that are the same in every
language are not translated: palette names, programming-language names and the
single-character submenu letters are proper nouns, and the case-conversion rows
are samples of the effect rather than words. A completeness test walks the menu
and the toolbar and fails when a translatable label has no entry, so a new
feature cannot quietly ship in English only.

Plugin names and the commands under them are the plugin's own words and are
never translated: the editor has no catalog for a vocabulary it did not write,
and a plugin called "JSON" would be surprised to find itself renamed. Those rows
are marked literal when the menu is built, so the completeness test walks past
them rather than asking the catalogs for an entry that cannot exist.

### Choosing a language

Preferences > Language cycles the bundled files and any file found in the
localization directory. The choice is stored as a file name rather than a path: a
stored absolute path would break the moment the configuration moves. A file that
cannot be parsed leaves the editor in English rather than in a half-translated
state.

## Accessibility

A screen reader on Linux talks AT-SPI over D-Bus. `accesskit_unix` speaks that
protocol; the editor's job is to say *what* is on screen, and
`accesskit_winit`'s job is to deliver it.

The tree is built from a snapshot rather than from the live widgets. Every widget
would otherwise need a font system, a theme and a window to be described, and the
tree is the one thing a screen reader user cannot check by looking. Flattening
the widgets into plain strings and indices makes the whole shape of the tree, the
node ids and the action mapping a pure function that can be tested without a
display.

The tree exposes the window, the menu bar and its menus and items, the tab list
with the selected tab, the multiline text input with its value, selection and
caret, and the status bar. Activating a menu item from the tree runs the same
action a click does, because the tree is built from the same row list the menu
draws. A submenu is described as a submenu rather than as a command that does
nothing.

## Keyboard Shortcuts

Every command the menu can invoke is reachable from the keyboard, and the chord
for each lives in one table rather than in a `match` arm. The menu reads its
shortcut column from the same table, so a rebinding cannot leave the menu lying
about what a key does.

Seventy-three chords are bound by default. The table below is the whole default
keymap, in the order the table lists it -- which is menu order, not alphabetical,
because a list that groups the file commands and then the edit commands is easier
to scan than one that starts with `Ctrl+A`.

| Shortcut | Command | Action name |
|---|---|---|
| `Ctrl+N` | New | `new` |
| `Ctrl+O` | Open... | `open` |
| `Ctrl+S` | Save | `save` |
| `Ctrl+Shift+S` | Save As... | `save_as` |
| `Ctrl+W` | Close | `close_tab` |
| `Ctrl+Shift+Q` | Exit | `exit` |
| `Ctrl+Z` | Undo | `undo` |
| `Ctrl+Y` | Redo | `redo` |
| `Ctrl+X` | Cut | `cut` |
| `Ctrl+C` | Copy | `copy` |
| `Ctrl+V` | Paste | `paste` |
| `Ctrl+A` | Select All | `select_all` |
| `Ctrl+Space` | Word Completion | `complete_word` |
| `Ctrl+Shift+Space` | Function Completion | `complete_function` |
| `Ctrl+Shift+Alt+Space` | Function Parameters Hint | `param_hint` |
| `Ctrl+Alt+Space` | Path Completion | `complete_path` |
| `Ctrl+F` | Find... | `find` |
| `Ctrl+H` | Replace... | `replace` |
| `F3` | Find Next | `find_next` |
| `Shift+F3` | Find Previous | `find_prev` |
| `Ctrl+Shift+F` | Find in Files... | `find_in_files` |
| `Ctrl+G` | Go to... | `goto_line` |
| `Ctrl+F2` | Toggle Bookmark | `toggle_bookmark` |
| `F2` | Next Bookmark | `next_bookmark` |
| `Shift+F2` | Previous Bookmark | `prev_bookmark` |
| `Ctrl+Alt+F` | Toggle Fold on Current Line | `fold_toggle` |
| `Ctrl+Alt+A` | Fold All | `fold_all` |
| `Ctrl+Alt+Shift+F` | Unfold All | `unfold_all` |
| `Ctrl++` | Zoom In (Ctrl+Mouse Wheel Up) | `zoom_in` |
| `Ctrl+-` | Zoom Out (Ctrl+Mouse Wheel Down) | `zoom_out` |
| `Ctrl+0` | Restore Default Zoom | `zoom_reset` |
| `Ctrl+Alt+M` | Document Map | `toggle_minimap` |
| `Ctrl+Alt+L` | Function List | `toggle_function_list` |
| `Ctrl+Alt+W` | Word wrap | `toggle_wrap` |
| `Ctrl+Alt+T` | Toggle Theme | `toggle_theme` |
| `Ctrl+,` | Preferences... | `preferences` |
| `F5` | Run... | `run` |
| `Ctrl+Shift+R` | Start Recording | `toggle_recording` |
| `Ctrl+Shift+P` | Playback | `play_macro` |
| `Ctrl+Shift+U` | UPPERCASE | `upper_case` |
| `Ctrl+U` | lowercase | `lower_case` |
| `Ctrl+Q` | Toggle Single Line Comment | `toggle_comment` |
| `Ctrl+D` | Duplicate Current Line | `duplicate_line` |
| `Ctrl+Shift+Up` | Move Up Current Line | `move_line_up` |
| `Ctrl+Shift+Down` | Move Down Current Line | `move_line_down` |
| `Ctrl+Tab` | Document Peeker | `document_peeker` |
| `Ctrl+PageDown` | Next Tab | `next_tab` |
| `Ctrl+PageUp` | Previous Tab | `prev_tab` |
| `Ctrl+Alt+S` | Save All | `save_all` |
| `Ctrl+P` | Print... | `print` |
| `Ctrl+Alt+Shift+P` | Print Now | `print_now` |
| `Ctrl+Shift+W` | Close All | `close_all` |
| `Ctrl+Alt+R` | Reload from Disk | `reload` |
| `Ctrl+L` | Delete Current Line | `delete_line` |
| `Ctrl+Shift+L` | Select Current Line | `select_line` |
| `Ctrl+]` | Increase Line Indent | `indent` |
| `Ctrl+[` | Decrease Line Indent | `unindent` |
| `Ctrl+Alt+D` | Insert Date Time | `insert_datetime` |
| `Ctrl+Alt+Up` | Add Caret Above | `add_caret_above` |
| `Ctrl+Alt+Down` | Add Caret Below | `add_caret_below` |
| `F11` | Toggle Full Screen Mode | `full_screen` |
| `F12` | Distraction Free Mode | `distraction_free` |
| `Ctrl+Alt+B` | Cycle Tab Layout | `tab_layout` |
| `Ctrl+Alt+H` | Change History Panel | `change_history` |
| `Ctrl+Alt+C` | Character Panel | `char_panel` |
| `Ctrl+Alt+I` | Incremental Search | `incremental_search` |
| `Ctrl+Shift+M` | Mark All | `mark_all` |
| `Ctrl+B` | Go to Matching Brace | `goto_brace` |
| `Ctrl+Alt+Shift+V` | Split Vertically | `split_vertical` |
| `Ctrl+Alt+Shift+H` | Split Horizontally | `split_horizontal` |
| `Ctrl+Alt+Shift+U` | Remove the Split | `unsplit` |
| `Ctrl+Alt+Shift+Right` | Move to Other View | `move_to_other_view` |
| `Ctrl+Alt+Shift+Left` | Clone to Other View | `clone_to_other_view` |

### Rebinding

"Settings > Shortcut Mapper > Open Shortcut Mapper..." lists every bindable
command with its chord. Pressing Enter on a row puts it into capture, and the next
chord that arrives is bound to that row, whatever it is. Capture is a mode rather
than a field, because that is the only way to bind a chord the dialog would
otherwise swallow: `Ctrl+N` has to be capturable while the dialog is open, and it
is also the chord that would open a new tab.

A chord means one thing. Binding a chord that is already taken removes it from
the previous command, so the keymap cannot contain a chord with two meanings and
the editor cannot have a command that fires or does not fire depending on the
order of a linear scan.

Motions, typing and the modal widgets are deliberately not bindable. They are not
commands with a name, and a user who rebinds `Ctrl+D` should not lose
`Ctrl+Left`.

### The keymap file

The keymap is stored in `settings.txt` as one line per binding:

```
key new Ctrl+N
key find Ctrl+F
key move_to_other_view Ctrl+Alt+Shift+Right
```

The left-hand side is the command's stable name -- the same word the settings file
has used since the first version, derived from the enum by hand rather than from
its `Debug` output, because a user's `key` lines have to keep meaning the same
thing across versions. A line naming a command this build does not know is
reported and skipped, so a file written by a newer version still loads the rest.

### Notepad++ interoperability

"Import Notepad++ Config..." reads a Notepad++ installation's `config.xml` for
the theme and the default font, and its `shortcuts.xml` for the shortcuts the
user changed from the defaults. Both are XML, and this crate has no XML parser --
but the subset needed is a handful of elements with quoted attributes, so the
reader is a scanner rather than a parser. It never fails: a file it cannot make
sense of imports nothing, and a file that is not there is not an error.

The import is deliberately partial. Notepad++ has commands this editor does not
and settings with no counterpart here; those are counted and reported rather than
guessed at. "Export Shortcuts..." goes the other way and is partial for the same
reason, and "Validate shortcuts.xml" is what makes the round trip trustworthy: it
reads a file written by either program and lists the rows the other one would not
understand.

## Configuration Files

Everything the editor remembers is a text file with a keyword and a value per
line, because a settings file is small, read once at startup, and worth being
able to fix in an editor when it goes wrong.

### settings.txt

| Keyword | Values | Default |
|---|---|---|
| `theme` | `system`, `light`, `dark` or a palette key | `system` |
| `font_size` | pixels | `15` |
| `tab_width` | columns | `4` |
| `wrap` | `on`, `off` | `off` |
| `minimap` | `on`, `off` | `on` |
| `toolbar` | `on`, `off` | `on` |
| `function_list` | `on`, `off` | `off` |
| `volatile_find` | `on`, `off` | `on` |
| `style <slot> #RRGGBB` | one line per overridden style slot | none |
| `run <command>` | one line per saved Run command | none |
| `tab_layout` | `horizontal`, `multiline`, `vertical` | `horizontal` |
| `backup` | `none`, `simple`, `verbose` | `none` |
| `backup_keep` | count | `10` |
| `autosave_seconds` | seconds, `0` is off | `7` |
| `datetime_format` | a `strftime` pattern | `%Y-%m-%d %H:%M:%S` |
| `search_engine` | URL template with `$(CURRENT_WORD)` | DuckDuckGo |
| `zoom_sync` | `on`, `off` | `off` |
| `sync_scroll_v` | `on`, `off` | `off` |
| `sync_scroll_h` | `on`, `off` | `off` |
| `theme_path` | path to an imported `stylers.xml` | none |
| `file_dialog` | `portal`, `builtin` | `portal` |
| `print_paper` | `a4`, `a5`, `letter`, `legal`, `tabloid` | `a4` |
| `print_landscape` | `on`, `off` | `off` |
| `print_line_numbers` | `on`, `off` | `off` |
| `print_header` | three tab-separated zones | empty |
| `print_footer` | three tab-separated zones | `$(PAGE)` |
| `language` | a file name in `localization/` | English |
| `plugin <name>` | one line per enabled plugin | none |
| `key <command> <chord>` | one line per binding | the default keymap |

Unknown keywords are ignored rather than rejected, so a file written by a newer
version still loads and an older version still starts. A malformed value falls
back to the default for that row alone.

### session.txt

```
recent /home/user/src/main.rs
tab /home/user/src/main.rs 41 9 pinned colour
active 0
```

Paths come last on their line so a path containing spaces needs no quoting. A
tab's numbers are the caret's line and column, zero-based, and the two optional
words are its pinned state and its colour.

### projects.txt

Three project slots, each with a name and a list of roots. Written with the
settings, read at startup, and created empty so the first project panel has a
place to put its first root.

### The plugin directory

One `.wasm` file per plugin, plus an `OFL.txt`-style licence beside anything the
plugin's author ships. A file that fails to load is logged to stderr, because a
plugin that does not appear in the menu has to be explicable.

## Unicode and Script Support

Text is Unicode throughout: the rope stores `char`s, the caret is a character
index, and every boundary is a character boundary rather than a byte offset. The
editor does not have a "Unicode mode"; there is no other mode.

Shaping is per line and chooses between a basic shaper and an advanced one. The
basic shaper picks the one font the attributes name and draws a box for every
glyph that font does not have; the advanced shaper runs the shaping engine, which
is what makes Arabic join, Hebrew read right to left, and Devanagari cluster. A
line is shaped the cheap way when its text is plain left-to-right Latin and the
expensive way otherwise, so the common case pays nothing for the rare one.

Font fallback is per glyph, which is what lets one line mix Latin, Greek,
Cyrillic, Hebrew, Arabic and CJK without a font choice from the user. One script
is shipped with the program: Korean. The one face on a typical Linux machine that
claims any Hangul at all claims U+AC00 and nothing beside it, so every Korean menu
label, tab title and status message drew as a row of boxes. The program therefore
carries `NekoHangul-Regular.ttf`, a subset of NanumGothic holding all 11172
syllables of U+AC00-U+D7A3, both jamo blocks, Latin-1 and general punctuation --
1.3 MB of the original 2 MB. NanumGothic is under the SIL Open Font License 1.1,
and because the licence reserves the name and a subset counts as a modified
version, the face is renamed "Neko Hangul". It is registered before the font
system is built, because fallback lists are computed from the faces present at
construction.

### Sample text

The paragraph below is drawn in this document with the same fallback principle:
the body font carries Latin, Greek and Cyrillic, DejaVu carries Hebrew and Arabic,
and Droid Sans Fallback carries CJK and Hangul. If the glyphs are boxes, the
font is missing; if they are letters, the fallback chain found them.

> Greek: Ξεσκεπάζω την ψυχοφθόρα βδελυγμία.
> Cyrillic: Съешь же ещё этих мягких французских булок, да выпей чаю.
> Hebrew: דג סקרן שט בים מאוכזב ולפתע מצא חברה.
> Arabic: نص حكيم له سر قاطع وذو شأن عظيم مكتوب على ثوب أخضر ومغلف بجلد أزرق.
> Korean: 다람쥐 헌 쳇바퀴에 타고파.
> Japanese: いろはにほへと ちりぬるを わかよたれそ つねならむ.
> Chinese: 视之不见，听之不闻，搏之不得.

### Right-to-left text

Hebrew and Arabic are shaped and drawn right to left, and the glyphs are placed
correctly. The caret, however, is positioned as `column * char_width`, which is
only true for a left-to-right run. On a right-to-left line the caret therefore
sits on the wrong side of the character it is between.

That is why "Text Direction RTL" and "Text Direction LTR" are absent from the
View menu. Notepad++ has both rows; this build does not, because a row that
appears to work and does not is worse than a row that is not there. The text
renders; the caret arithmetic has not been taught about bidirectional runs yet.
This is the largest known gap in the editor and it is stated here rather than
buried.

### Normalisation

Text is stored exactly as it arrives. No Unicode normalisation is applied on load
or on save, because an editor that silently rewrites composed characters changes
files it was asked to display; a document with `e` plus a combining acute stays
two code points.

Decoding is lossy by design. A byte sequence that is not valid UTF-8 in the
declared encoding becomes U+FFFD rather than an error, because a mostly-readable
file that opens is worth more than a refusal to open it. The consequence is
stated plainly: saving such a document writes the replacement characters back,
so a file with a corrupt stretch should be inspected rather than edited. There is
no read-only guard for this case; the editor tells you the encoding in the status
bar and leaves the judgement to you.

## Deliberate Omissions

An editor that silently lacks a row is harder to trust than one that says why.
These are Notepad++ features this build does not have, and the reason.

| Not there | Why |
|---|---|
| Text Direction RTL / LTR | The caret is placed as `column * char_width`, which is wrong for a right-to-left run. The text renders; the caret does not. |
| Updater rows in Help | There is no updater. A row that opened a browser at a download page would be a promise the program cannot keep. |
| A Shortcut Mapper *dialog* with Import and Export buttons | There is no such dialog. The two operations are rows under Settings > Shortcut Mapper, one level down, and they are the same two operations in the same place. |
| Plugin Admin | There is no plugin repository. Plugins are files a user copies in, and "Manage Plugins..." lists what was found and switches each one on or off. |
| A macro editor | A macro is recorded, replayed and saved as a command. Editing one by hand is editing a settings file. |
| Spell checking | It needs a dictionary per language and a hunspell-compatible reader, and neither is vendored. |
| A terminal inside the editor | The editor opens a terminal in the document's directory instead, which is what the row is for. |
| Scintilla's C plugin ABI | Plugins are WebAssembly. The reason is in "Plugins": a DLL has the run of the address space, and a bad one takes the editor down with it. |
| Session *sets* with named workspaces | One session file plus "Save Session..." covers the same ground without a second list to maintain. |
| A full regular-expression replacement syntax | Replacement is literal. `$1` in a replacement field that did nothing would be worse than one that is not offered. |
| Multi-line regex over a document | The regex matcher runs per line, because a match spanning lines cannot be highlighted by a line-oriented renderer. |

Everything else in Notepad++'s menu tree that this document lists is
implemented. If a row is in the menu and this document describes it, it does
something; the rows that do nothing are the ones listed above, and they are not
in the menu.

## About and Credits

Neko++ is a Rust workspace of two crates and one binary. The product name is
Neko++; the program is `npp`; the crate is `nn-app`. `nn` names the workspace and
`npp` names the program, so the two cannot drift apart by accident.

The editor is built on `winit`, `wgpu`, `cosmic-text`, `glyphon`, `ropey`,
`regex`, `accesskit`, `zbus` and the `serde` family. Four things are written out
rather than pulled in, and each has a reason in its module: the MD5/SHA family
(the four hashes are short and a dependency would be larger), the WebAssembly
interpreter (no runtime is vendored, and the sandbox limits should be ours), the
PDF writer (CUPS wants PDF and a full PDF crate is a large dependency for one
page layout), and the XML scanner (the subset Notepad++ writes is a handful of
tags with quoted attributes).

The Korean font is NanumGothic, subset and renamed, under the SIL Open Font
License 1.1; the licence is kept beside the font in `assets/OFL.txt`.

The About box names the version, the renderer, the adapter, the font system and
the configuration paths, because those are the five facts a bug report needs. Its
one interactive element is the LinkedIn address, drawn in the theme's link colour
and opened with the desktop's handler.
