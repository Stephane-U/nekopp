# Neko++ plugins

A plugin is a WebAssembly module the editor loads at startup and offers in the
`Plugins` menu, one row per command. The module cannot open a file, spawn a
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
scanned when the editor starts, so a rebuild is picked up by restarting it.

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

`align` and `halfwidth` work on the selection when there is one and on the
whole document otherwise, which is what makes them usable from a menu that has
no way to ask which was meant. `surround` wraps the selection; with nothing
selected it inserts the pair and leaves the caret between them.

## Provenance

These three plugins are reimplementations of editing operations that
Notepad++ plugins have offered for years: column alignment, surrounding a
selection with a pair of characters, and full-width/half-width conversion. They
are written from the behaviour, not from anyone's source. They read the
document through a fifteen-function, byte-offset ABI rather than through
Scintilla's messaging API, so the code, the tables and the call order are
different by construction; no line of anyone else's source was copied.

This matters because of licences. Most Notepad++ plugins are GPL-licensed, and
this repository has no `LICENSE` file, so it has no declared licence under which
it could distribute GPL-derived code. Nothing GPL-derived was copied here: the
plugin ABI in `crates/nn-core/src/wasm/` and the three plugins in this directory
are original work inside this repository.

If you port a real plugin from source rather than reimplementing it, read its
licence first. A copyleft plugin cannot be combined with this repository's
absence of a licence without the author deciding what that licence is first.
