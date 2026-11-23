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
groups, 50 theme choices, 74 default key bindings, 23 bundled localisations, 16
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

--call sends a JSON call to the editor that is already running and
prints its answer: `npp --call '{"method":"document.list"}'`.
Nothing opens.

--rest serves the same calls over HTTP on 127.0.0.1, for a client that
would rather fetch a URL: `--rest=8085`, then `GET /status`. The port
goes straight after the switch, so a file called 8085 still opens.

switches:
  -n, --new-instance    start a new process even if one is running
  -w, --wait            block until the document is closed
  -l, --line            put the caret on this line (one-based)
  -c, --column          put the caret on this column (one-based)
  -s, --software        draw with the software rasterizer, not the GPU
  -C, --call            send a JSON call to the running editor and print the answer
      --rest            serve the REST face on 127.0.0.1 (--rest=PORT names the port)
  -h, --help            print this text
  -V, --version         print the version
```

`--call` is the one switch that opens nothing while the editor runs: it hands a
JSON request to the window that is already running and prints the answer.
"Commands over the API" is what it is for.

`--rest` opens a listener instead, and it is the only switch whose port must be
attached: `--rest 8085` is a listener on the default port and a file called
`8085`, because an editor that guesses which of the two a word meant is an editor
that has opened the wrong thing. "The REST face" describes what it serves.

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

A file named on the command line that does not exist yet opens an empty buffer
of that name, and the first save creates it, the way `vim` and `nano` do. The
buffer is not dirty until something is typed, the file only enters the recent
list once it exists, and a path that is there but cannot be read -- a directory,
a file without permission -- is still reported as an error rather than replaced
by a blank page.

### One editor per session

A second launch does not open a second window by default. The first process owns
the session bus name `org.npp.Editor`, and a later launch calls `Open` on it with
absolute paths, the requested caret position, and whether the caller wants to be
told when the document closes. The second process then exits.

The handoff also ends the desktop's startup notification. A launcher that starts
a program puts the pointer into its busy state and waits for the program to say
the launch is over, which a program normally does by opening a window. A launch
that hands its files to a running editor opens no window, so the second process
sends the message itself before it exits. Without it the pointer kept turning
until the desktop's own timeout gave up, seconds after the file had opened,
which is what made the file manager feel slow.

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
| `api.yaml` | what the REST listener serves, as described under "Over HTTP" |
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

The find bar is docked along the bottom by default: the pattern field, then a
row of eleven setting chips, then the action buttons, which wrap into as many
rows as the window's width needs. `Ctrl+H` adds the replacement field and its
three swap buttons above the chips. Notepad++'s dialog has a Transparency
pull-down; a bar docked to the window edge has nothing to fade over, which is why
turning on "Floating Search" in Preferences exists: it draws the same search as
the dialog, transparency included, and leaves the docked bar for those who would
rather not lose the lines to a panel.

The eleven chips are the dialog's checkboxes in the dialog's order: match case,
match whole word only (the slot becomes ". matches newline" once the pattern is
a regular expression, because a whole word is not a thing a regex has), normal,
extended and regular expression search mode, wrap around, in selection, and the
two direction chips. The last two are the mark tab's, which a dialog gets from a
second tab: "B" bookmarks every match as it marks it, and "P" is "Purge for each
search", which clears the marks left by earlier searches before laying down the
new ones. Without "P", marks of different styles accumulate, which is what makes
several searches visible at once.

Both fields remember what was searched for. `Alt+Down` or the little arrow at the
right end of the field opens the list, and the twenty most recent entries are
kept, eight of them visible at a time. The list hangs over the document, so a
click in it counts as a click on the bar even though the list is outside the
bar's own rectangle.

The bar is incremental in the sense that matters -- the count updates as you
type -- and it never moves the caret until you ask for the next match.

Behind the bar sit the buttons Notepad++ 8.9 has: Find Next, Count, Find All in
Current Document, Find All in All Opened Documents, Replace, Replace All, Replace
All in All Opened Documents, Mark All, Clear Marks, and Close. The three
replacement buttons only appear with `Ctrl+H`, the way the dialog disables them
in find mode. "Find All" fills the results panel below the bar instead of
opening the dialog's own list, and clicking a row there opens the file at the
line. Each of the three swap buttons does one of the pull-down's three entries,
because three small buttons cost one click where a pull-down costs two.

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

"Playground" is the fourth, and the only one that hands the whole window to
something else. It is a page of code with a kitten on it; the document is
untouched and `Escape` comes back. It has a chapter of its own, below.

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
- Playground (`Ctrl+Alt+K`)
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

The four link rows open this project's own pages on GitHub, so a label and a
destination no longer disagree. "Neko++ Home" opens the repository, whose README
says what the editor is and how to build it. "Neko++ Issues" opens its issue
tracker, which is where a bug report or a question goes. "Neko++ Online User
Manual" opens the reference document itself, rendered from the same markdown the
PDF is built from. "Neko++ Community (LinkedIn)" opens the author's LinkedIn
profile, the same address the About box carries.

Until this release those rows went to Notepad++'s home page, its repository, its
user manual and its forum, which were the closest thing to documentation for
these features while this project had no pages of its own.

"Neko++ License (GPLv3)" opens the GNU project's page for the licence the source
is under. It sits below a separator of its own because it is not a page about this
project: it is the terms, and a user checking them is asking a different question
from a user looking for the issue tracker. The complete text also travels with the
program -- in `LICENSE` in the source tree, in `/usr/share/doc/npp/` in the
package, and in the AppImage -- because a link needs a network and a licence
should not.

"Debug Info..." reports the renderer, the adapter, the font system, the
configuration paths and the file list, which is what a bug report needs.
"About Neko++" is the box described in "About and Credits".

- Command Line Arguments...
- Neko++ Home
- Neko++ Issues
- Neko++ Online User Manual
- Neko++ Community (LinkedIn)
- Neko++ License (GPLv3)
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

Docked along the bottom of the pane, above the status bar. The pattern comes
first, then a row of settings chips, then the action buttons, which flow left to
right and wrap onto another row when the window is too narrow to hold them in
one. A match counter at the right end of the pattern row reads `n/m`. `Escape`
closes the bar and returns the keyboard to the text.

The buttons are the dialog's own, in the dialog's order: Find Next, Count, Find
All in Current Document, Find All in All Opened Documents, Replace, Replace All,
Replace All in All Opened Documents, Mark All, Clear Marks and Close. The three
replacement buttons only exist once `Ctrl+H` has opened the replacement row,
which is the bar's way of greying out what the dialog disables. The two "Find
All" buttons fill the results panel rather than a list of their own. Long labels
wrap rather than truncate, and a greedy fill that would leave one button alone
under a row packed to the edge moves a button down instead, which is the
difference between a bar that looks laid out and one that looks wrapped.

The settings row is eleven chips. `Aa` is match case. `W` is whole word; in
regex mode the same chip turns into `\n` and means "a dot matches a newline"
instead, because a whole word is not a thing a regular expression has and the
row has no room for a twelfth chip. `N`, `E` and `.*` are the three search
modes, and exactly one of them is lit: Normal takes the pattern literally,
Extended reads `\n`, `\r`, `\t`, `\0`, `\\`, `\xHH` and `\uHHHH` as escapes and
leaves an escape it does not know as the two characters it is, and `.*` hands the
pattern to the regex crate. `∞` is wrap: with it on, a search that runs off the
end continues at the first match; with it off the search stops at the end and
leaves the caret where it is. `▭` confines the search to the selection, read live
from the editor on every refresh and falling back to the whole document once the
selection is empty -- a match has to lie entirely inside the range, because
replacing one that starts before it would write outside the text the user meant
to touch. `↑` and `↓` pick the direction, and exactly one is lit. The last two
come from the dialog's mark tab: `B` bookmarks every line a mark lands on, and
`P` is "Purge for each search", which wipes the marks of every style before the
new ones go down. Purge off is what lets two searches of different styles stay
visible at once; purge on is what stops the document filling with the last
search's colours.

Both fields keep a history, twenty entries deep and eight rows visible, opened by
the arrow at the right end of the field or by `Alt+Down` when that field has the
caret. The list grows upwards over the document, so a press inside it belongs to
the bar even though it lies outside the bar's rectangle.

`Enter` does what the bar is set up to do: Find Next in the direction that is
lit, Replace when the caret is in the replacement field, and Replace All with
`Ctrl` held. `Shift+Enter` takes the opposite direction.

`Ctrl+H` opens the same bar with a replacement row between the pattern and the
settings, plus the dialog's swap pull-down as three small buttons -- exchange the
two fields, copy the pattern into the replacement, copy the replacement into the
pattern -- because three buttons are one click each where a pull-down is two.
`Ctrl+F` puts the bar back to find mode without losing the pattern. The
label column is as wide as the widest label as drawn, not as wide as a constant:
"Replace:" in a proportional face is wider than "Find:", and a translated label
can be half again as long, so a fixed column ends up under the field's text.

`Ctrl+V` pastes into whichever of the two fields has the caret, so a pattern
copied from a file or a terminal does not have to be retyped. The line breaks
and tabs a copied line brings along are dropped: the field is one line and a
pattern carrying a newline would never match.

`Ctrl+A` selects the field and draws the selection, so the next keystroke
replaces the whole pattern instead of landing in the middle of it, and an arrow
key drops the selection again. Re-pointing a search at a new word is then one
chord and one word, not a row of Backspaces.

Case-insensitivity is the default, which is what a user typing a word into a
file expects. The counter turns red when there is no match, which is the only
feedback a modal-less bar can give.

Closing the bar drops its highlights when "Volatile Find" is on, which is the
default: a search you have finished with should not leave the document speckled.
Turning it off keeps the highlights, which is what you want when the point of the
search was to see every occurrence at once.

### The floating Find window

Preferences has one row, "Floating Search", that changes how `Ctrl+F` and
`Ctrl+H` look and nothing else. Off, which is the default, the search is the bar
docked along the bottom. On, it is a sheet over the document with three tabs --
Find, Replace and Mark -- and the caption bar of Notepad++'s dialog: the pattern
and the replacement as `Find what:` and `Replace with:`, a row of checkboxes, two
groups of radio buttons for direction and search mode, a transparency slider, and
the tab's buttons along the bottom.

The tabs are not three searches. Replace is the Find tab plus the replacement
row, and Mark is Find with the marking options in place of the wrap and
selection ones, so switching keeps the pattern, the matches and the history. It
also moves the caret to the field the tab is about: typing a replacement is why
anybody opened the Replace tab.

The checkboxes are the bar's chips under their own names: Match whole word only,
Match case, Wrap around, In selection, Bookmark line, Purge for each search and
". matches newline". The two whole-word-ish boxes share the bar's one chip, and
the sheet dims whichever of them the current mode cannot honour -- Match whole
word only in regex mode, ". matches newline" outside it -- so the row does not
reflow under the pointer when the mode changes.

The frame is dragged by its caption bar. The slider fades the panel and its text
down to a tenth of their opacity and no further, because a dialog you cannot find
is a dialog you have to close to get back. `Escape` closes it and returns the
keyboard to the text, and a click outside it lands in the document: the window is
not modal, and Find Next would be a poor button if it were the only thing you
could press. The old docked bar is still one preference row away, and switching
between the two keeps the search that was typed in either.

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

## Playground

"View > Playground" (`Ctrl+Alt+K`) hands the window to a page: a line-number
gutter, rows of code at the editor's own line height, a wall of household
furniture up the right of it -- a sofa, shelves, a table and plants of three kinds
-- a rug of punctuation along the bottom, and a kitten two lines tall who walks
across the text, follows the pointer, jumps from line to line, bats loose words off
them and -- left alone -- finds things on the page to do: swats the furniture,
climbs the wall a piece at a time, crosses the page in super jumps between the
furniture and the code, chases dust, runs a lap, and eventually falls asleep; every
three minutes the page she is standing on goes out in fireworks and is replaced by
another one. `Escape` gives the
window back. The document is never read and never written, so a screenshot of the
mode cannot show anyone's work and three minutes in the page cost no undo step.

The page is flat, and that is the whole idea. It is drawn with an orthographic
camera, one cell to one cell, so a glyph lands on whole pixels and the second line
of code is the same size as the twentieth: what the reader sees is a document,
not a diorama of one. The kitten is the exception -- its boxes go through the same
camera pre-multiplied by a shear, so a cube has a visible side and top while the
letters it stands on stay square. Everything is measured in lines rather than in
pixels, so the mode fills a 1280 by 800 window with about 40 rows of 116 columns, a
smaller window with fewer of both, and looks the same on any display.

| Piece | Behaviour |
|---|---|
| The page | A gutter of line numbers, rows of code, the bottom five rows a rug of scattered punctuation, and a corner label on a line kept clear for it |
| The ledges | Every intact row of code is a shelf a paw can stand on, and the floor runs under the rug at the bottom of the page, so falling always ends somewhere |
| The furniture | One or two pieces drawn in characters -- a table, a plant, a lamp, a framed picture -- each standing in its own cleared box in the lower half of the page and in the room to the left of the wall, swaying when a paw hits it |
| The wall | Seven or eight pieces against the right edge, one above another: a sofa standing on the rug, then a shelf, a fern, a table, a topiary and two more shelves, each top four or five lines over the last, which is one bound; right-aligned with a column or two of play, and every piece of it somewhere to land |
| The kitten | Walks, sits, crouches, gathers herself and springs onto a line of code, bats a token, swats the furniture, bounds up the wall and onto a plant, goes up the page a foothold at a time, crosses it in super jumps between the furniture and the code, chases dust, runs laps, washes a paw, blinks, flicks its tail; turns to follow the pointer, and lands the paw it aimed |
| Wanting up | Going up is worth half again as much on the rug as at the top of the page: climbing, perching and the super jumps are weighted by how far up she already is, so the low half of the page is a place she leaves rather than a place she lives |
| The tokens | Whole words knocked off a line by a paw, carrying the syntax colour they had where they stood; they fall, bounce once, and come to rest on the next ledge under them or on the floor, with a shadow under each |
| The dust | Specks that wander on their own; one within reach of a paw is thrown in a short arc and fades where it lands |
| The light | A jump leaves a band of colour along the arc she flew, a small halo travels with her, a landing puts a ring on the paper with a burst of confetti, and every fourth piece of trail is a star; a swoop lays the same light twice as wide and a whole turn of the wheel long |
| The run | Now and then she picks the far end of the ledge she is standing on and crosses it at twice her running speed, taking the characters in her way and dragging a streak of light along the line as it comes apart; a run with time left on its clock takes another leg, and one leg in three is the line over her head |
| The lap | Her other fast mood: the whole page is the target and the speed is between a run and a dash, raking and streaking exactly as a dash does, turning onto a new line whenever it reaches the end of the one it is on |
| The blast | A run has two ends and both of them hit: pushing off and stopping from that speed take everything within a radius of her paws at once, each run of characters thrown away from her |
| The climb | She takes the highest foothold one bound can reach -- a line of code or the top of a table, whichever is higher, up to six lines and three across -- and takes another from there, for as long as there is anything over her head, so the ascent zigzags up the page and stops only where the page does |
| The claws | A climb digs in where it leaves: every nine tenths of a line she rises takes a swathe two words wide out of the row her back paws are in, thrown down and outwards, and the furniture in reach sheds what it is made of |
| The top | Reaching the highest place a cat may stand -- two lines short of the top of the page, which is where its ears go -- puts a double ring, a puff of dust and two dozen pieces of confetti on the paper, and the top of a climb is where a swoop begins |
| The swoop | A super jump across the page: up to thirty-four characters in one bound, from the wall of furniture to the code and back again. Out of the wall it is a leg at a time to the left, each a few lines lower than the last, aimed at the end of a line of code, because that is where the writing stops; out of the code it is one leg to the right and up to five lines, landing on the flat top of a piece of furniture. A run keeps one direction and its clock is refreshed at every landing, so a leg is followed by another while there is page left to cross |
| The rainbow | A swoop's trail is twice a jump's: two bands of light, the wider one carrying a whole turn of the colour wheel from the tail of the leg to its landing, and a star in every third piece, so what crosses the page is a ribbon rather than a stroke |
| The landing | The one landing on the page that takes the line it lands on: four characters bitten out of the row, everything within sixteen thrown, two rings, a puff, confetti and the furniture in reach shaking; then a run along what is left |
| The mending | Everything a paw, a run, a claw or a landing takes is remembered and printed back, three characters a second while the page is barely damaged and up to seven times that when much of it is in the air, each one fading in over four tenths of a second, so the page is taken apart and put back rather than emptied |
| Sleep | Three minutes with no input and it curls up where it stands, and `z` floats up off her head |
| Sparkles | Washing and stretching raise a `*` or two off the muzzle, for a second or so |
| Dimming | Four minutes and the page starts to dim, the way a screensaver does |
| Rebuild | Three minutes and the page is rebuilt from the next seed, the old one fading out over two and a half seconds while the new one goes up in fireworks |
| The fireworks | Twenty-four shells across the width of the page on one fuse after another over three and eight tenths of a second: each goes up as a streak from the rug, comes apart eight to eighteen cells a second outwards in sixty sparks across half the colour wheel, and puts a wave of light through everything near it |
| The volley | The shells are coloured from one end of the wheel to the other, so what crosses the page is a rainbow that goes up rather than one that is painted on; the opening burst is a single shell out of the middle of the paper, and a burst also throws twelve stars on its own edge, chips the furniture and raises a puff |

A line of code is a place, not a stripe of colour. Every row the page still holds
is a shelf: a paw can stand on it, a word knocked off it falls onto the shelf
below, and a cat can spring from one to the next. That is what lets the mode read
as a room instead of a scrolling background -- she climbs the page the way it is
laid out, and the page answers by emptying. Gravity is one number, a bounce, and a
surface lookup that asks what is under a point and where it starts, so a word in
the air and the cat in the air are subject to the same rule and no falling thing
can pass through a line that is still there.

A landing and a swing are not the same thing, and the page treats them
differently. A swing takes what is under the paw: the longest run of whole words
on that one line is lifted out, keeping their colours, so a row reads
`let count = 0;` on the left and then a gap and the rest of the line. A landing
only shakes the page and raises dust -- taking a line out from under the paws
would leave her standing on nothing, and a cat that deletes its own ledge is a
bug, not a joke. A bound reaches six lines, and because the arc is fixed while the
height is not, a short hop and a long leap look the same going up and very
different coming down. She crouches first, the way a cat does, and holds it for a
third of a second before she springs.

A run is a third verb, and not a faster walk. A dash picks the far end of the line
she is standing on rather than the most interesting thing on the page, because a
run is horizontal and a target nine lines up is a climb: a cat that aimed at the
farthest thing would walk to the point under it and stand there. Twice her running
speed is fast enough to cross the page in under two seconds, which is what makes it
an event rather than a hurried walk, and she does not stop when she arrives -- the
mood's own clock ends the run, and getting there early only means the paw comes
down on what she found. What she passes comes apart. A swing takes the longest word
under the paw; a shoulder takes whatever is in a swathe nearly three characters wide
at her own row, so a line comes out with a run of holes in it rather than a hole in
it, and the words come away thrown the way she was going. The rake is reported a
character and a half behind her and never under her, because a cat that took the
line out from under its own paws would drop into the page mid-run -- what she leaves
is a furrow, and a furrow is behind. The gap a rake leaves is narrower than the
bridge a shelf is built over, so the line she is running along stays a ledge even as
it empties.

A run has two ends and both of them hit. Leaving the ground at that speed is a
push-off and stopping from it is a fall, so the frames where a run begins and ends
each take everything within a radius of her paws at once, each run of characters
thrown along the line from her to where it stood, which is why the middle of the
hole is empty and the edges look like a burr. That is the page's loudest answer and
it happens twice a run; the furrow between the two is what carries the reader along.

The zoomies are the same animal with the whole page for a target. At one and seven
tenths of a run they rake and streak exactly as a dash does, and because they turn,
one lap crosses four or five lines instead of one. Both of the fast moods start on a
line and not on the rug, because the floor of the room is a rug of punctuation at a
third of the opacity -- scenery and not text -- so a run that started there would
take nothing at all; standing on it, they go up first.

A climb is the one thing on the page that is only up. It is a run of bounds rather
than one, and each is aimed at the highest foothold a single bound can reach -- six
lines, and as much as three across, because the line above her may end before her
column and a climb that only took what was exactly overhead would stop a third of
the way up. Landing on it costs nothing but a ring and a puff: what a climb does to
the page it does with its claws. The foothold is not gone over twice, so an ascent
reaches the top of the page in six or seven bounds and takes about eight seconds
over it, and the place she lands is offset a couple of characters along from where
she stood, which is the difference between a cat going up the page and a cat on a
lift. Where the page above her column runs out, the ascent has run out with it; the
next one starts from wherever she comes down. Two rows at the top are above the
highest place a cat may stand, because that is where its ears go, so the top of a
climb is a cat with its head against the edge of the page and nothing left to take
except the light. It is worth as much as a run in the table that picks her moods,
which is what makes it something a reader sees every minute rather than once.

The claws are the other half of a climb, and they are not a rake and not a swing. A
claw goes where her back paws are -- in the row she has just left -- because the row
she is going to has to still be there, and a cat that clawed the line it was landing
on would have nothing to land on. So the swathe is half a line of height and the
damage is behind her as well as below: every nine tenths of a line she rises pulls
once more, which is what turns four or five bounds into a furrow of missing
characters twenty rows long, with the words falling rather than flying, because the
page is above her and nothing falls up. Furniture in reach of the same claws is
shaken and sheds a card or two. What is left of the page after a climb is a vertical
scar with a cat sitting at the top of it, which the mending then takes half a minute
over.

The wall is what that ascent is for, and it is why the right of the page is margin no
longer. A page of code is a cliff only over the columns the code reaches; past the
longest line there is nothing to climb and nothing to land on, which is why the
right edge used to be kept empty. What stands there now is the furniture of a room
pushed against a wall: a sofa on the rug, then a shelf, a fern, a table, a topiary
and two more shelves, each top four or five lines over the last, which is one bound
and not a lift. They are right-aligned with a column or two of play in them, because
a wall is stacked by someone rather than laid out by a grid, and the plants are in it
because a room has plants and because a cat that has climbed a fern ends up sitting
in it. The wall is placed first and the loose furniture afterwards, so the scatter
stands in the room to the left of it instead of in the way up.

A cat wants up. It is the one thing the mood table says about the animal rather than
about the page: the moods that go up -- climbing, perching on the furniture, and the
super jumps -- are worth more the lower she is, half again as much on the rug as at
the top of the page, so the low half of the page is somewhere she leaves. What that
buys is a mode that is vertical: over a page's three minutes she spends between two
fifths and half of the frames climbing or in the air between two footholds, and the
top of the page is reached on every seed rather than once in a while. The pointer
still wins over all of it -- a mouse that has just moved takes her off the wall --
but a mouse that is already still does not, and the clock does not run out on one.

The wall and the code are the two halves of the page, and a swoop is the leap between
them: up to thirty-four characters in one bound, aimed at whichever half she is not
standing in, because a leap that stayed on its own side would be a jump along a shelf
and that is what a dash is for. Out of the top of the wall it is a leg at a time to the
left, each aimed at the end of a line of code, where the writing stops; out of the code
it is one leg back to the right and up to five lines, because the furniture over her
head is what there is to come down on. Each run keeps one direction -- a cat that
turned round at every landing would be a cat bouncing -- and the clock is refreshed at
every landing, so there is another leg while there is page left to cross. The arc is a
fixed shape and its height grows with its width, so a long leg is a flat, fast crossing
rather than a fall. What follows her is the rainbow of a jump at twice the width and in
two bands, with the colour turning once over the whole length of the leg -- one rainbow
per leg rather than a scatter of them -- and a star in every third piece. The landing
at the end of it is the only one on the page that takes the line it lands on: four
characters gone, everything within sixteen thrown, two rings and confetti on the paper,
the furniture in reach shaking. Every other landing only shakes, because a cat that
took the line out from under its own paws would have nowhere to stand; a swoop is the
end of a flight the wall was climbed for, so it is allowed to bring the line down with
it. A run that comes back to the wall ends in a climb rather than a dash, because the
furniture is what there is to go up.

The furrow is not permanent, and that is what makes the rest of it possible. The
page remembers every character it loses, with the cell it stood in, and prints them
back three a second, each fading in over four tenths of a second; the memory is
capped at two and a half thousand characters, near enough a whole page, so an
afternoon of this costs a fixed amount of memory rather than a growing one. Without
the mending the mode could not afford a cat that wrecked anything, which is what it
used to be: the damage had to be kept rare to keep the page a page, and the result
was a cat sauntering over a document that never changed. The rate is deliberately
near the rate she destroys at. Much faster and the damage is never seen; much slower
and the page slowly turns into gaps -- and one rate cannot serve both a run's furrow
and an ascent's, so the page prints faster the emptier it is, seven times its base
rate at a page with nothing left.

The furniture sheds, too. A prop is drawn in characters like everything else, and a
body that arrives at one takes a card or two out of it and throws them, so a table
loses its middle instead of sitting there being hit.

What follows a run is a wash. The weights that pick her next mood are read off the
mood she is in, and a cat that has just crossed the page at a gallop is a cat with
no play left in it: sitting, grooming and cleaning go from one or two to eight and
ten, and another run is worth nothing at all. That is the whole of the
characterisation -- there is no state machine and no script, only a table of what
each thing is worth right now -- and it is why the mode reads as an animal rather
than as a generator after a few minutes.

The light belongs to the page and not to the drawing, which is what makes it
checkable. The simulation owns every spark: where a piece of trail goes, how long
it lives, how far around the colour wheel it sits. It knows nothing about colour
-- a hue is a number, and one additive pass in the renderer turns that number into
a rainbow -- and because nothing in it reads the clock, a frame at a given time is
reproducible, so a test can count the pieces along a jump without a window. The
trail is laid every part of a cell she flies rather than every few hundredths of a
second: a rule about time piles the light into a white column when she pounces in
place and thins it into a dotted line when she falls off a ledge, while a rule
about distance puts the same band along the same flight at any frame rate. Slow
steps around the wheel are the other half of it: light adds, three pieces of three
hues in one place are white, so a jump turns the wheel once and no more.

The kitten is a few dozen boxes and no skeleton: a body, a head, two ears, four
legs with cream socks, a muzzle, a chin, a nose, two eyes and a tail on a bow
outside the rump. Two lines tall is enough to read as a kitten, and it means the
renderer has one mesh and one draw call whatever the pose is.

She is built in two spaces, not one. The body faces along the page and is turned
a quarter turn, so it is in profile and the length of the cat runs left to right
along the lines. The head is built in its own space with its own small turn, so
the face looks out at the reader. That is the difference between a kitten and a
small dog: a head in profile shows one eye, half a muzzle and no face, so the head
is instead as wide as the body is long and carries the whole of it -- two dark
eyes a fifth of the face with a white glint in each, cream cheeks and chin, a pink
nose, and ears wide at the base and narrow at the tip with the pink of the inside
showing where they stand above the crown.

There are three bodies. Walking, standing, crouching and pouncing share one.
Sitting front-on lowers the hips, folds the hind legs under and brings the tail
down around them. Sleeping replaces the lot with a loaf, the head laid along the
front of it and the tail curled up by the nose. Boxes that stack overlap, and no
box is wider than the one in front of it, because the renderer lights every face:
a top face left showing inside a silhouette is a bright band across the animal,
and a band across a face is a muzzle nobody drew.

What she does is picked from what the page holds. A mood lasts a few seconds and
is drawn by weight from a table that knows the page: swat a piece of furniture,
chase the nearest speck of dust, climb a table or a shelf and perch on its edge,
bat a loose word, run a lap of the rug, pounce, sit, groom, sleep. A line of code
is a better perch than any of the furniture, so she looks up from where she stands
rather than across the page, and a shelf she can already reach is not worth a walk.
follow one drive, rest, which fills while she sits and empties while she plays, so
a cat that has been idle reaches for a table and a cat that has been chasing dust
sits down. A page with nothing to perch on offers no perch, and a cat already
sitting on a table does not pick another one.

A mood that is going somewhere keeps going until it gets there: the timer waits
while she travels and only starts when she arrives, because a cat that gives up
halfway across the page has not done the thing it set out to do. Arriving is not
stopping either -- a walk or a lap that reaches its mark picks the next mark -- so
a patrolling cat keeps moving instead of standing in the middle of the text. When
the mood does start, the paw lands on the thing she walked to, and the page
answers. One method is the whole vocabulary of a paw: words within reach are
knocked away carrying the colour of the line they came from, furniture within a
line of its box sways and settles back exactly where the layout put it, and dust
is thrown in a short arc and dies where it lands. One mood lands one paw.

The material is a corpus compiled into the binary, in eight flavours: Rust, C,
Java, Python, JSON, XML, UN/EDIFACT and CSV. The first six are coloured by the
editor's own lexers, because a page should be printed in the highlighting the rest
of the program uses; EDI and CSV have no lexer here and are split on their
delimiters instead. The open document is never the material. Starting the mode on
the file the user was reading would make it look like a view of that document
rather than a page beside it, and the page's vocabulary is meant to be a different
language every three minutes.

The page changing is the one event in the mode that has nothing to do with the cat,
so it is the one event allowed to be loud. A rebuild fires twenty-four shells across
the width of the page, one every sixth of a second for three and eight tenths of a
second: a streak up from the rug, then a burst sixty sparks wide across half the
colour wheel, a wave of light through everything within a line and a half of it, a
ring of twelve stars on its edge, a puff of dust and a chip out of whatever piece of
furniture it went off beside. The colours run from one end of the wheel to the other
across the volley, so what goes over the paper is a rainbow that goes up rather than
one that is painted on, and the whole of it is bracketed by one shell out of the
middle of the page at the first frame of the new one. The old page fades over two
and a half seconds and the new one rises into its place; the display runs six or
seven, four hundred sparks at its peak against the hundred and thirty the cat holds
on her own, and then it is over and the page is something else.

Two clocks matter and they are different. The page's clock is part of the rules:
it decides when the page is rebuilt and when the kitten sleeps, and it advances
only while the mode is open, so a minute in the page is a minute of the page. The
window's idle clock counts input of any kind -- a key, a click, a wheel -- and it
decides whether the mode may open on its own.

That is the second way in. With "Playground After" set to a number of minutes, a
window that has seen no input for that long opens the page by itself and the mode
behaves as a screensaver; the default is never, because a screen that turns into a
page of code while somebody is reading it is a screen that has taken their
document away. "Playground" is the switch that owns both ways in. Off,
`Ctrl+Alt+K` is inert and the mode never opens by itself, which is also the answer
for a machine that is shared or recorded.

Inside the page only `Escape` is a command. Every other key is a sign that
somebody is still at the desk: it wakes the kitten, keeps the page bright and
cancels the screensaver, and it goes no further, so typing cannot reach a document
that is not being drawn. The line in the corner names the flavour and the page
number, says when the kitten is asleep, and always ends with "Esc leaves"; a
screen reader is given the same sentence as one label, which is the whole
accessibility story of a mode with no widgets in it.

The drawing is three instanced pipelines into one depth buffer -- glyph cards for
the page, boxes for the kitten, camera-facing dots for dust and shadows -- with
the cards sorted back to front. Text is not lit: a document whose syntax colours
depended on where a kitten's lamp happened to be would be a document that changes
colour as a cat walks past it. Only the boxes are lit, and the whole picture dims
together when the screensaver dims it. The glyphs come from an atlas rasterised
once at startup at twice the height they are drawn at, and are tinted per instance
from the syntax style the lexer gave them.

The mode is not in Notepad++, and it is the one place where this editor adds a
whole subsystem rather than matching one: Notepad++ is a program somebody uses all
day, and the alternative to three minutes of kitten is three minutes of something
else.

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
| Floating Search | On or off | Off |
| Playground | On or off: whether the page can be opened at all | On |
| Playground After | Minutes of no input before the page opens by itself, up to 60; zero shows "Never" | Never |
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

## Commands over the API

Every door onto a command -- the menu, the keymap, `npp --call`, the MCP server --
opens onto one table. `nn_core::ops` names the commands whose whole effect is a
function of the text: sort, dedupe, reverse, join and split, trim, case, comment,
tabs and spaces, indent. Thirty-seven of them. The menu row for "Sort Lines
Ascending", the keybinding that does the same thing, `command.run` over the API
and `run_command` over MCP all reach the same function, so a change lands in one
place and cannot be true on one surface and false on another. The two rows whose
menu is a submenu or a prompt -- `sort_lines` with its key and order,
`split_lines` with its delimiter -- are in the table too, named the way a caller
would name them.

A command whose effect depends on where the caret is (`duplicate_line`,
`move_line_up`, `delete_line`) is not in that table: there is no text in, text out
for those. They are reachable in a window, through `command.run`.

### Without a window

`run_command` over MCP runs one command over a string or over a file and answers
with the result. It needs no window, no display and no session bus, so it works in
a container, over ssh, and in a client that has never started the editor.

```
{"command": "sort_asc", "text": "c\na\nb\n"}           -> "a\nb\nc\n"
{"command": "tabs_to_spaces", "path": "a.rs", "write": true}
```

`path` without `write` reads the file and changes nothing, which is what a caller
checking a result wants; `write` puts the result back. `text` and `path` are
alternatives, not a pair. The options a command reads are the ones `list_commands`
names beside it: `sort_key`, `sort_order`, `case_sensitive`, `width`, `delimiter`,
`prefix`, `block_open`, `block_close`, `seed`.

### In the window

`npp --call '<json>'` hands one call to the editor that is already running and
prints its answer as one JSON object on stdout. Nothing opens and nothing takes
the focus: the process is a client, not a launch.

```
npp --call '{"method":"document.list"}'
npp --call '{"method":"command.run","command":"sort_asc","whole_document":true}'
```

`-C` and `--call=...` are the same switch. The methods:

- `app.status` -- the version, how many documents are open, which one is active.
- `document.list` -- every open document in tab order: its path, its language,
  whether it has unsaved changes, and which one is active.
- `document.get` -- one document's text, caret, selection, line ending and size.
  `path` names one, leaving it out means the active tab, and `range` asks for a
  span of characters instead of the whole.
- `document.set` -- replaces a document's text, or a `range` of it, and answers
  with the path and the new size.
- `document.save` -- writes it to the file it came from.
- `command.run` -- runs any command in the menu, by its action name.
- `help` -- the list above, from the binary itself.

A failed call answers `{"error": {"code": ..., "message": ...}}` and exits 0: the
call reached the editor and was refused, which is an answer. A call that finds no
editor exits 1 and says why on stderr.

`command.run` runs the command the way the menu does -- over the selection, or
over the line the caret is on -- because that is what makes one name mean the same
thing everywhere. A caller that means a block says so: `whole_document` selects
everything first, and `range` takes `[start, end]` in characters. They are
alternatives; sending both is refused rather than guessed at. Without either,
`sort_asc` on a document with no selection sorts the one line the caret is on,
which is worth reading twice before wondering why nothing changed.

The answer is thin -- a path, a character count, or the text when `return_text`
asks for it -- because a caller that knows what it sent rarely needs it echoed.

A call is answered by the window's own event loop, not by the process that
connected: the client queues the request over D-Bus, the window runs it between
frames, and the client collects the answer with a second call. A blocking method
would stall the message loop that has to deliver its own reply. The client asks
every 5 ms and gives up after 5 s.

### Over HTTP

`--rest` serves the same call table over HTTP on `127.0.0.1`, for a client that
would rather fetch a URL than speak JSON-RPC on a pipe or find a session bus. The
port is `8085` unless `--rest=PORT` says otherwise, and the `rest_port` key sets
the default for a launch that names none.

| Method and path | Call | Reads |
|---|---|---|
| `GET /` | -- | the route list, as JSON |
| `GET /help` | `app.help` | -- |
| `GET /status` | `app.status` | -- |
| `GET /documents` | `document.list` | -- |
| `GET /document` | `document.get` | `path`, `range` |
| `PUT /document` | `document.set` | `path`, `range`; body is the new text |
| `POST /document/save` | `document.save` | `path` |
| `POST /command/<name>` | `command.run` | `return_text`, `whole_document`, `range` |

```sh
curl -s http://127.0.0.1:8085/status
curl -s -X PUT --data-binary @notes.txt http://127.0.0.1:8085/document
curl -s -X POST 'http://127.0.0.1:8085/command/sort_asc?whole_document=1'
```

`path` names a document and leaving it out means the active tab, exactly as in
`--call`. `range` is `start-end`, half-open character offsets, the same pair the
JSON spelling takes. A `PUT` body is the document's new text; with
`Content-Type: application/json` it is `{"text": ..., "range": [start, end]}`
instead, which is how a caller sends a span without percent-encoding it into a
query.

A route does not validate anything. It builds the request object a `--call` user
would have typed and hands it to the same parser, so the one rule about whether
`whole_document` and `range` can travel together is stated once, in the place that
already had tests for it, and the REST face cannot answer a request the other two
faces would refuse. What HTTP adds is the status line: `200` when the call
answered, `400` for a malformed request, `404` for a path or a document that is
not there, `405` with an `Allow` header for the wrong verb, `500` for a call that
failed, `503` when no window answered in 5 s. The body is always the `error`
object `--call` answers with, so a client parses one thing.

`--rest` is the switch for one launch. The listener can also be described in
`api.yaml` or `api.json` in the configuration directory, which is what a machine
that runs a harness wants: the file is written once and every launch of the editor
serves the API without a wrapper script.

```yaml
# ~/.config/npp/api.yaml
rest:
  enabled: true
  port: 8085
  token: "a long random string"
  read_only: false
```

```json
{"rest": {"enabled": true, "port": 8085, "read_only": true}}
```

| Key | Values | Default |
|---|---|---|
| `enabled` | `true`, `false` | off unless `--rest` says otherwise |
| `port` | 1 to 65535 | the `rest_port` setting, then 8085 |
| `token` | a string; empty means no check | none |
| `read_only` | `true`, `false` | `false` |

The keys may sit inside a `rest:` section or at the top of a file that holds
nothing else, and the nested spelling wins when a file has both. YAML is read as
the subset a configuration needs -- `key: value`, one level of nesting, `#`
comments, quoted or bare values -- and a list or a second level is refused with the
line number rather than half-read; JSON is read as JSON, nesting and all. The two
file names are looked for in that order, so `api.yaml` wins and the ignored
`api.json` is named on stderr rather than silently skipped.

What beats what is specificity. `--rest=PORT` wins over the file, the file wins
over `settings.txt`, and `NPP_REST_TOKEN` wins over the file for the token, because
a secret is better in the environment than on disk. A configuration file that
exists and does not load stops the launch with the file name, the line and the
reason: a typo in `port` must not turn into a listener somewhere unexpected, and a
typo in `token` must never turn into a listener with no token at all. A token in a
file other users can read is served, and warned about on the same startup line.

`read_only: true` turns the API into a reader: `document.set`, `document.save` and
`command.run` answer `403` with the code `read_only`, and everything that only
reads keeps working. It is the setting for a dashboard or a monitor that should be
able to see the editor's state without being able to change it.

The listener is loopback and nothing else. A request that carries an `Origin`
header is refused with `403`, because no shell tool sends one and every browser
does: that single rule is what stops a page on another site from replacing the
text in an open window and saving it. `NPP_REST_TOKEN` in the environment of the
launch adds a bearer check (`Authorization: Bearer <token>`), and so does `token`
in the configuration file for the case where the file is the only thing that can
carry it. Neither goes into `settings.txt`: that file is a record of preferences
and a secret is not a preference. Eight requests may be
in flight at once and a body may be 32 MiB; past either, the answer is `413` or
`503` rather than a thread or an allocation per connection.

### Over MCP

`live_call` takes the call itself -- `method` and whatever that method reads --
and forwards it to `npp`, so it answers with exactly what a shell would see. It is
the way to do something that needs the window: work on the selection, save the tab
someone has open, reach `duplicate_line` and its neighbours. It needs an editor to
talk to: `NPP_BIN` first, then a sibling of `npp-mcp`, then PATH.

The two tools are chosen by what the work touches. Text that only needs
transforming goes to `run_command`: no window, no focus, no session. Work that has
to happen in the window someone is looking at goes to `live_call`. `get_command`
says which command needs the window and which does not, and it answers to both
vocabularies: `upper_case` comes back with its shortcut and its menu path because
the keymap binds it, and `sort_asc` comes back with the options it reads and no
menu path because its row is one of fourteen behind a submenu.

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

Seventy-four chords are bound by default. The table below is the whole default
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
| `Ctrl+Alt+K` | Playground | `playground` |
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
| `rest` | `on`, `off`: start the REST listener with the window | `off` |
| `rest_port` | the port `--rest` takes when it names none | `8085` |
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

The source is under the GNU General Public License version 3, and the terms are
applied the way the Free Software Foundation describes: the unmodified text is
`LICENSE` at the top of the tree, and every source file in `crates/`, `plugins/`,
`packaging/`, `debian/` and `docs/` opens with the copyright line and the
permission notice, in the comment syntax of its language. The version is 3, not
"or later", which is what `license = "GPL-3.0-only"` in `Cargo.toml` says and what
`debian/copyright` says, so the three cannot drift apart. It is a workalike
written from scratch and contains no code from Notepad++ or Scintilla.

The About box carries the notice an interactive program is asked to show: the
copyright, the licence by name and version, and the sentence that there is no
warranty, in the box rather than in a terminal a window does not have. It also
names the version. Its one interactive element is the LinkedIn address, drawn in
the theme's link colour and opened with the desktop's handler; the licence itself
is a row in the Help menu, one click away from the paragraph that explains it.
