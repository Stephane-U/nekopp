# npp-mcp

The Neko++ feature reference, served over the Model Context Protocol.

`docs/nekopp-features.md` is embedded into the binary at build time and cut into
sections at startup. The server answers questions about the editor's features,
its menus, and its default keymap from that file and from nothing else, so an
answer cannot disagree with the PDF built from the same markdown.

It also runs the editor's WebAssembly plugins. The folder and the `plugin
<name>` lines in `settings.txt` are the same ones the editor reads, so a plugin
switched off in the manager is switched off here, and one that is on is on in
both places.

The binary is `npp-mcp`. It is a separate crate with no dependency on `nn-app`,
so it builds and runs on a machine that has never started the GUI, and it takes
no arguments: a client starts it and talks over a pipe.

```sh
cargo build --release -p nn-mcp      # writes target/release/npp-mcp
target/release/npp-mcp               # reads stdin, writes stdout
```

## The protocol

JSON-RPC 2.0, one object per line, in both directions. There are no headers, no
length prefixes, and no framing beyond the newline: the server reads a line,
answers with a line, and flushes. Everything that is not protocol goes to stderr,
including the startup line, so stdout is the protocol and nothing else.

The revision is `2024-11-05`. The server implements:

| Method | Answer |
|---|---|
| `initialize` | protocol version, `{tools: {}, resources: {}}`, server name and version, and a short set of instructions |
| `notifications/initialized` | nothing: a notification is never answered |
| `ping` | `{}` |
| `tools/list` | the ten tools with their JSON Schemas |
| `tools/call` | one or more content blocks, plus `isError` |
| `resources/list` | `nekopp://features` and one resource per section |
| `resources/read` | the section's markdown |
| `prompts/list` | an empty list: this server publishes no prompts |

A notification -- a message with no `id` -- never gets a response, not even when
the method is unknown; answering one would put a line on the wire that the client
is not waiting for. A request for an unknown method gets a JSON-RPC error object
with code `-32601`; params that do not fit the method get `-32602`; a line that is
not JSON gets `-32700`; a resource URI that names nothing gets `-32002`. A tool
that fails -- an unknown section, an unknown command -- is not a protocol error:
the call succeeds with `"isError": true` and a text block that says what went
wrong and what to try instead.

## Tools

| Tool | Arguments | Returns |
|---|---|---|
| `search_features` | `query`, optional `limit` (1-50, default 10) | the matching sections, best match first, with headings, URIs, and a line of context |
| `get_section` | `section`: a heading or a slug | that section as markdown, plus its resource URI |
| `list_sections` | none | the table of contents: 28 chapters, 81 subsections |
| `list_shortcuts` | none | all 73 default key bindings with their commands and action names |
| `get_command` | `name`: a menu label, an action name, or a `run_command` name | the shortcut, the action, the menu path, the section that documents it, and whether it runs with no window |
| `list_plugins` | none | every installed plugin: its name, its file, whether it is switched on, and its commands |
| `run_plugin_command` | `plugin`, `command`, `text`, optional `caret`, `selection` | what the plugin made of the text, plus its log when it wrote one |
| `run_command` | `command`, `text` or `path`, optional `write` and the command's options | the text after the command ran, or a note that the file was written; no window involved |
| `live_call` | the API call itself: `method` and whatever it reads | the editor's answer as JSON, from the window that is already open |
| `list_commands` | none | the 37 commands that work on text with no window, with the options each reads |

`run_command` is the headless half: text in, text out, no display and no session
bus, so it works in a container or over ssh. `live_call` is the other half: it
forwards the call to `npp --call` and prints what the window answered, which is
how a client reaches the caret, the selection and the open tabs. It looks for the
editor in `NPP_BIN`, then beside `npp-mcp`, then on PATH.

Search ranks a heading that is the query above a heading that contains it, above
a heading with one of its words, above a mention in the body. Whole words beat
fragments, so "bin" does not pull in every page that says "bindable".

## Resources

Every section is a resource, addressed by its slug:

```
nekopp://features                 the index: every section with its URI
nekopp://features/folding         the Folding chapter
nekopp://features/plugins         the Plugins menu
nekopp://features/plugins-1       the Plugins chapter (the second heading with that name)
```

The slug rule is the PDF's anchor rule (`docs/build-pdf.py`), so a URI here and a
link in the PDF name the same section.

## A session

Five lines in, four lines out: the `notifications/initialized` line is a
notification and is not answered. This is the actual transcript of

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"shell","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_features","arguments":{"query":"column editing","limit":2}}}' \
  | target/release/npp-mcp
```

```json
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"resources":{},"tools":{}},"instructions":"The Neko++ feature reference. `search_features` finds sections, `get_section` returns one as markdown, `list_sections` is the table of contents, `list_shortcuts` is the whole default keymap, and `get_command` answers about a single command. Every section is also a resource: nekopp://features/<slug>, indexed by nekopp://features. `list_plugins` is what the editor's WebAssembly plugins add, and `run_plugin_command` runs one of their commands over text you send.","protocolVersion":"2024-11-05","serverInfo":{"name":"npp-mcp","version":"0.2.0"}}}
```

```json
{"id":2,"jsonrpc":"2.0","result":{"content":[{"text":"2 sections match \"column editing\":\n\n## Editing and the Caret Model -- nekopp://features/editing-and-the-caret-model\n  The editor is not a text box with extra keys. It has a selection model, several\n\n### Edit -- nekopp://features/edit\n  \"Begin/End Select\" and its column-mode twin mark an anchor and let the caret\n","type":"text"}],"isError":false}}
```

The same session against the built binary prints those two response lines and, on
stderr, one startup line: `npp-mcp 0.2.0: 104 sections, 73 shortcuts, 1 of 5
plugins enabled, one JSON object per line on stdio`. The plugin counts are this
machine's -- the folder the editor uses -- so they differ from one installation
to the next.

## Registering it with a client

Any client that speaks MCP over stdio can run it. Claude Desktop reads
`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "nekopp": {
      "command": "/opt/DEV/nn/target/release/npp-mcp",
      "args": []
    }
  }
}
```

VS Code reads `.vscode/mcp.json` with a `servers` object instead of `mcpServers`:

```json
{
  "servers": {
    "nekopp": {
      "type": "stdio",
      "command": "/opt/DEV/nn/target/release/npp-mcp",
      "args": []
    }
  }
}
```

The Vibe CLI reads `[[mcp_servers]]` entries, in `~/.vibe/config.toml` for every
project or in a trusted project's `.vibe/config.toml`, and writes one itself:

```sh
vibe mcp add nekopp --transport stdio --command /opt/DEV/nn/target/release/npp-mcp
```

```toml
[[mcp_servers]]
name = "nekopp"
transport = "stdio"
command = "/opt/DEV/nn/target/release/npp-mcp"
args = []
```

Its seven tools arrive in the `mcp_nekopp` group, as `mcp_nekopp.search_features`,
`mcp_nekopp.get_section` and so on, and `/mcp` in a session lists the server with
the tools it published.

`cargo run --release -p nn-mcp` works anywhere a configuration wants to build
first, but the installed binary is the better command: it starts in a few
milliseconds and does not depend on a toolchain.

## Development

```sh
cargo test --offline --release -p nn-mcp          # 70 tests
cargo clippy --offline --release -p nn-mcp --all-targets
```

The logic is in the library so it can be tested without a pipe:

- `doc`: cuts the markdown into sections, assigns slugs, and finds a section by
  heading or slug.
- `commands`: parses the keymap table and the menu tree, and joins them.
- `search`: the ranking and the snippet selection.
- `plugins`: the plugins this machine has, read through `nn_core::plugin`, and the one command a call runs.
- `server`: the JSON-RPC dispatch, the tools, the resources, and the stdio loop.

The markdown is embedded with `include_str!`, so the file has to exist before
this crate compiles. It does: `docs/nekopp-features.md` is the source of the PDF
as well, and editing it and rebuilding is how the server learns a new feature.
