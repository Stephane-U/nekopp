# Neko++ plugins

A plugin is a WebAssembly module the editor offers in the `Plugins` menu, one
row per command. Having the file in the plugins directory is what installs it;
switching it on is a separate step, and a newly installed plugin starts
disabled. `Plugins > Manage Plugins...` lists every plugin the editor found,
says whether it is on, and toggles it. The module cannot open a file, spawn a
process or reach the network: the host functions below are its entire surface,
so a plugin is only as dangerous as an editing command can be.

This directory is a separate Cargo workspace because the plugins are built for
`wasm32-unknown-unknown`, not for the host. The editor's own `cargo build` at
the repository root therefore needs no wasm toolchain, and a user who only
wants to edit text can ignore this directory entirely.

## Building and installing

```sh
rustup target add wasm32-unknown-unknown   # once
./build.sh
```

`build.sh` runs `cargo build --release --target wasm32-unknown-unknown` and
copies every `npp_*.wasm` it produces into the directory the editor reads
plugins from:

```
${XDG_CONFIG_HOME:-$HOME/.config}/npp/plugins
```

`Plugins > Open Plugins Folder` in the editor opens that same directory, which
is one way to check the install without guessing at paths. The directory is
scanned when the editor starts and again on `Plugins > Reload Plugins`, so a
rebuild is picked up without restarting (the plugins that were on stay on).

## Switching one on

Installing is not enabling. A plugin that is present but switched off offers no
commands: the menu lists it as `Load <Name>` and nothing else happens. Open
`Plugins > Manage Plugins...`, which shows every plugin in the directory with
`Enabled` or `Disabled` beside it, move to a row with the arrow keys and press
Space. The row flips, the status bar says `Loaded <Name>` or `Unloaded <Name>`,
and the `Plugins` menu grows or loses that plugin's commands immediately --
there is no restart and no reload in between. `Esc` closes the list.

The choice is remembered in `settings.txt` in the same directory, one line per
enabled plugin:

```
plugin JSON
plugin Surround
```

The file records names, not paths, so the list survives a plugin being rebuilt
and reinstalled. A name in the file that no longer matches a `.wasm` on disk is
ignored rather than reported as an error, which is what makes deleting a plugin
a matter of deleting its file.

## Writing one

Add a crate to `members` in `Cargo.toml`, depend on `nn-plugin` (`path =
"../sdk"`), and export your commands with the macro:

```rust
use nn_plugin::host;

fn shout() {
    let text = host::text();
    host::replace(&text.to_uppercase());
}

nn_plugin::plugin! {
    name: "Shout",
    commands: [
        ("Uppercase the document", shout),
    ],
}
```

The crate needs `crate-type = ["cdylib"]`. Name the file `npp_<something>.wasm`
if you want `build.sh` to install it for you; nothing else depends on the file
name. A plugin with several commands gets several menu rows, shown as
`Name - Command`.

The SDK is in [`sdk/`](sdk/src/lib.rs) and its `host` module is the typed
version of the ABI table below: `text`, `line`, `lines`, `range`, `replace`,
`insert`, `replace_range`, `caret`, `set_caret`, `selection`, `select`, `log`,
`line_count`, `line_start`, `line_len`, `line_of_pos`.

## The ABI

Four exports, all of which the SDK writes for you:

| export | signature | meaning |
|---|---|---|
| `npp_name` | `() -> i32` | pointer to a NUL-terminated UTF-8 name |
| `npp_command_count` | `() -> i32` | how many commands the plugin offers |
| `npp_command_name` | `(i32) -> i32` | pointer to command `i`'s name |
| `npp_run` | `(i32) -> ()` | run command `i` |

Fifteen imports, all from the module `npp`, all `i32` in and out. A pointer is
an offset into the plugin's own linear memory; a length is a byte count.

| import | signature | meaning |
|---|---|---|
| `log` | `(ptr, len)` | append a line to the plugin log |
| `text_len` | `() -> i32` | the document's length in bytes |
| `text_read` | `(offset, ptr, len) -> i32` | copy document bytes into plugin memory |
| `text_replace` | `(ptr, len) -> i32` | replace the whole document |
| `text_insert` | `(pos, ptr, len) -> i32` | insert at a byte offset |
| `text_replace_range` | `(pos, remove, ptr, len) -> i32` | replace `remove` bytes at `pos` |
| `caret` | `() -> i32` | the caret's byte offset |
| `set_caret` | `(pos)` | move the caret |
| `line_count` | `() -> i32` | how many lines the document has |
| `line_start` | `(line) -> i32` | the byte offset of line `line`'s first byte |
| `line_len` | `(line) -> i32` | line `line`'s length in bytes, without its terminator |
| `line_of_pos` | `(pos) -> i32` | which line byte `pos` is on |
| `sel_start` | `() -> i32` | the selection's lower byte offset |
| `sel_end` | `() -> i32` | the selection's upper byte offset |
| `set_sel` | `(start, end)` | select from `start` to `end` |

`text_read` returns how many bytes it copied, which is fewer than asked for at
the end of the document. The three writing calls return zero on success and
non-zero on refusal -- a read-only document, or an offset past the end -- so a
plugin can tell "done" from "the editor said no" without a second call.

Every offset and length is a **byte** offset into the document's UTF-8. That is
the unit a plugin can measure for the data it is handed, and the editor does the
character conversion at the boundary. The one thing that does not count bytes is
a line index, as the name says.

`crates/nn-core/src/wasm/abi.rs` holds the same table in the compiler's view,
and a test there fails if the documentation and the host implementation drift
apart.

## What is here

| crate | commands |
|---|---|
| [`surround`](surround/src/lib.rs) | Surround with Quotes, Surround with Brackets, Surround with Parentheses, Select Quoted Text |
| [`align`](align/src/lib.rs) | Align on Equals, Align on Colon, Align on Comma |
| [`halfwidth`](halfwidth/src/lib.rs) | To Half Width, To Full Width |
| [`json`](json/src/lib.rs) | Pretty Print, Minify, Validate |
| [`xml`](xml/src/lib.rs) | Pretty Print, Linearize, Check Well-Formedness, Escape Entities, Unescape Entities |

`align` and `halfwidth` work on the selection when there is one and on the
whole document otherwise, which is what makes them usable from a menu that has
no way to ask which was meant. `surround` wraps the selection; with nothing
selected it inserts the pair and leaves the caret between them. `json` follows
the same rule as `align` and `halfwidth`, because JSON is a document on its own
and a selection is often just the part of it being worked on. `xml` prints and
checks the whole document -- an element's indentation depends on the nesting
around it -- and touches only the selection for the two entity commands, since
that is the text being pasted into a document.

## Provenance

These plugins are reimplementations of editing operations that Notepad++
plugins have offered for years: column alignment, surrounding a selection with
a pair of characters, full-width/half-width conversion, and formatting and
checking JSON and XML. They are written from the behaviour, not from anyone's
source. They read the document through a fifteen-function, byte-offset ABI
rather than through Scintilla's messaging API, so the code, the tables and the
call order are different by construction; no line of anyone else's source was
copied.

The JSON and XML plugins are clean-room implementations written from the
formats themselves -- RFC 8259 for JSON, the XML 1.0 recommendation for XML.
They are not ports of XML Tools or JSON Viewer, the GPL plugins that offer
commands with these names. Those two were used only as a description of which
commands a user expects to find in the menu: the parsers, the error messages
and the edits in `plugins/json` and `plugins/xml` are this repository's own
work, and neither crate contains a line of their source. Both keep the bytes
they were given -- object key order, the spelling of `1e10`, an entity
reference, a self-closing tag -- because the result is written back over the
user's document, and a formatter that re-encodes what it did not have to has
edited the file.

This matters because of licences. Most Notepad++ plugins are GPL-licensed, and
this repository has no `LICENSE` file, so it has no declared licence under which
it could distribute GPL-derived code. Nothing GPL-derived was copied here: the
plugin ABI in `crates/nn-core/src/wasm/` and the five plugins in this directory
are original work inside this repository.

If you port a real plugin from source rather than reimplementing it, read its
licence first. A copyleft plugin cannot be combined with this repository's
absence of a licence without the author deciding what that licence is first.
