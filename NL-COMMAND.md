# Natural-language command layer

Goal: drive the editor by saying or typing what you want, in your own words,
without leaving the keyboard and without a network round trip.

Two inputs, one output path:

```
typed text ─┐
            ├─→ intent ─→ Command ─→ Editor ─→ (optional) spoken confirmation
microphone ─┘
```

Everything runs locally. No API keys, no telemetry, no model downloads at
startup.

---

## 1. Why three tiers

A single small model in front of every keystroke would be the wrong shape. Most
commands people issue are one of about twenty fixed phrasings, and paying 200 ms
of inference for "save" is indefensible in an editor that renders a frame in
0.5 ms.

So intent resolution is layered, cheapest first:

| Tier | Mechanism | Latency | Handles |
|---|---|---|---|
| 0 | Hand-written parser | < 1 µs | The ~20 phrasings that cover most use |
| 1 | Tiny LLM, grammar-constrained | 150–300 ms | Everything else |
| 2 | Ask the user | — | When tier 1 is not confident |

Tier 0 is not a fallback for a weak model. It is the primary path, and it is
exact. Tier 1 exists so the user never has to learn the tier-0 vocabulary.

The tiers share one output type. Nothing downstream knows which tier produced a
command, which means tier 1 can be added, swapped, or removed without touching
the editor.

---

## 2. The command type

One enum, in `nn-core`, with no dependency on any model or audio crate:

```rust
pub enum Command {
    // Motion
    Move { motion: Motion, extend: bool },
    Goto { line: usize, column: Option<usize> },
    Scroll { lines: i32 },

    // Selection
    Select { target: SelectTarget, extend: bool },
    SelectAll,

    // Text
    Insert(String),
    Delete { target: DeleteTarget },
    Replace { find: String, with: String, all: bool, regex: bool },
    Find { pattern: String, regex: bool, case: bool },

    // Structure
    Comment { on: bool },
    Indent { on: bool },
    Fold { target: FoldTarget, on: bool },

    // Files
    New, Open(PathBuf), Save, SaveAs(PathBuf), Close,

    // History
    Undo, Redo,

    // Presentation
    SetLanguage(Language),
    SetTheme(ThemeName),

    // Speech
    Speak(String),
    StopSpeaking,

    // Escape hatch
    Unknown(String),
}
```

`Unknown` is deliberate. A command layer that silently drops what it did not
understand is worse than one that says "I did not understand that" — and the
user needs to see the failure to correct it.

Every command executes as **one undo group**. "Replace all foo with bar" is one
Ctrl+Z, not four hundred.

---

## 3. Tier 0 — the deterministic parser

A hand-written matcher over normalised input: lowercased, punctuation stripped,
number words converted ("forty two" → 42), filler words dropped ("please",
"could you", "go ahead and").

It covers, at minimum:

```
save | save as <path> | open <path> | new | close
undo | redo
go to line <n> | line <n> | jump to <n>
top | bottom | page up | page down | scroll up <n>
select all | select line | select word | select to line <n>
find <text> | find next | find previous
replace <a> with <b> | replace all <a> with <b>
comment | uncomment | toggle comment
indent | unindent
fold | unfold | fold all | unfold all
read this line | read the selection | stop talking
```

Each pattern is a small function returning `Option<Command>`. They are tried in
order and the first match wins. This is a few hundred lines of ordinary Rust
with ordinary unit tests — no model, no weights, no download.

**This tier ships first and is useful on its own.**

---

## 4. Tier 1 — the tiny local model

### Model

**Qwen2.5-0.5B-Instruct**, GGUF, `Q4_K_M` — roughly 400 MB on disk, ~600 MB
resident. It follows short instructions well for its size and is fast enough on
CPU to stay inside the latency budget.

Fallbacks if it disappoints: Qwen3-0.6B, SmolLM2-360M-Instruct (smaller, weaker),
Llama-3.2-1B-Instruct (better, 2× the cost).

The model is a **download on first use**, not a bundled asset. The editor must
start and run with no model present.

### Runtime

`llama-cpp-2` — Rust bindings to llama.cpp. Chosen over a pure-Rust runtime
because it gives us **GBNF grammar-constrained decoding**, which is the single
most important property here.

### Why the grammar matters

Free-form JSON from a 0.5B model is unreliable: it invents fields, forgets
braces, hallucinates commands. Constrained decoding makes that impossible — the
sampler can only emit tokens that keep the output on a path to a valid command
in the schema. The model's job shrinks from "write JSON" to "pick a command and
fill its slots", which a 0.5B model does well.

The grammar is generated from the `Command` enum, so adding a command cannot
desynchronise the two.

### Prompt shape

```
system: You convert editor requests into one command. Reply with JSON only.
        Commands: <generated schema summary>

user:   File: src/editor.rs (Rust, 412 lines)
        Caret: line 88, column 12
        Selection: none
        Request: "wrap this in a match"
```

Context is deliberately small — language, caret, selection, line count. Enough
to resolve "this" and "here", not enough to blow the prompt budget.

### Confidence

The model returns a command plus a confidence. Below a threshold, the editor
shows the parsed command in the palette and waits for Enter instead of executing.
Destructive commands (`Replace { all: true }`, `Close`, `Delete`) always
confirm, regardless of confidence.

---

## 5. Voice input

```
cpal capture ─→ VAD ─→ whisper.cpp ─→ text ─→ tier 0 / tier 1
```

| Piece | Crate | Notes |
|---|---|---|
| Capture | `cpal` | 16 kHz mono, resampled with `rubato` |
| VAD | `silero-vad` via `ort` | Detects end of utterance; ~1 MB model |
| STT | `whisper-rs` | `ggml-tiny.en.bin`, 77 MB |

`tiny.en` over `base.en` because command utterances are short and the vocabulary
is narrow — the accuracy difference is small, the latency difference is not.
Non-English uses `tiny` multilingual.

**Push-to-talk is the default.** Hold a key, speak, release. It removes the
hardest problem in voice interfaces (knowing when the user is done) and it is
what people actually want in an editor, where they are already at the keyboard.
Always-listening is a later opt-in.

VAD still runs in push-to-talk mode, to trim silence from the ends of the clip
before it reaches Whisper. Shorter audio is faster audio.

---

## 6. Speech output — Pocket TTS

[Kyutai Pocket TTS](https://github.com/kyutai-labs/pocket-tts): 100M parameters,
CPU-only, ~200 ms to first audio chunk, ~6× real-time on an M4 using two cores.
Streaming, with voice cloning and seven languages.

It is a Python package, so it runs as a **sidecar**, not in-process:

```bash
pip install pocket-tts --extra-index-url https://download.pytorch.org/whl/cpu
pocket-tts serve          # keeps the model resident between requests
```

The CPU-only index matters: the default PyPI install pulls ~3 GB of CUDA wheels
for a model that never touches the GPU. The CPU build is ~200 MB.

The editor POSTs text to the local server and plays the returned PCM through
`cpal`. `serve` keeps the model in memory, so only the first utterance pays the
load cost.

### Rules

- **Speech never blocks the UI thread.** A worker thread owns the HTTP client and
  the audio output; the editor drops text into a channel and moves on.
- **Speech is optional and off by default.** An editor that talks unprompted is
  hostile. It speaks when asked, or to confirm a voice command.
- **Barge-in.** Starting to speak stops playback immediately.
- **Degrade silently.** No sidecar running means no speech, not an error dialog.

### Licensing

The model weights are non-commercial (CC-BY-NC-4.0). Fine for personal use;
a blocker for a commercial release. The sidecar boundary means it can be swapped
for a permissively licensed engine (Piper, Kokoro) without touching the editor.

---

## 7. Latency budget

Voice, push-to-talk, 3-second utterance:

| Stage | Cost |
|---|---|
| VAD trim + end detect | 50 ms |
| Whisper `tiny.en` | 150–300 ms |
| Tier 0 match | < 1 ms |
| Tier 1 model (only if tier 0 misses) | 150–300 ms |
| Execute | < 1 ms |
| TTS first chunk | ~200 ms |
| **Total, tier 0 hit** | **~400 ms** |
| **Total, tier 1** | **~700 ms** |

Typed commands skip the first two rows: **under 300 ms**, and under 1 ms for
anything tier 0 recognises.

For comparison, the editor renders a frame in 0.5 ms. The command layer is two
to three orders of magnitude slower than the thing it drives, which is why it
must never sit on the render path. It does not: commands mutate the document,
the document bumps its revision, the next frame notices.

---

## 8. Crate layout

```
crates/
  nn-core/     existing — gains `command.rs` (the enum, no deps)
  nn-app/      existing — gains the command palette UI
  nn-nl/       new — tier 0 parser, tier 1 model runner, GBNF grammar
  nn-voice/    new — capture, VAD, STT, TTS client
```

Both new crates are behind cargo features, off by default:

```toml
[features]
default = []
nl    = ["dep:nn-nl"]
voice = ["dep:nn-voice"]
```

A default build stays a lean text editor with no C++ toolchain, no model files,
and no audio stack. `--features nl,voice` builds the full thing.

---

## 9. Milestones

**M1 — Command palette, typed, no models.**
`Command` enum, tier-0 parser, palette UI (`Ctrl+Shift+P`), one undo group per
command. Ships real value with zero new dependencies. Everything after this is
additive.

**M2 — Tier 1 model.**
`nn-nl`, llama.cpp, GBNF grammar generated from the enum, model download on
first use, confirmation for destructive commands.

**M3 — Voice input.**
`nn-voice`, push-to-talk, VAD, Whisper. Transcribed text goes through the same
two tiers — the voice path adds no new intent logic.

**M4 — Speech output.**
Pocket TTS sidecar, worker thread, barge-in, spoken confirmation of voice
commands.

**M5 — Polish.**
Streaming partial transcripts, command history, "do that again", per-command
keybindings learned from use.

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| 0.5B model misreads intent | Grammar constrains output; tier 0 catches the common cases; confirm before destructive actions |
| Whisper mishears | Show the transcript before executing; push-to-talk keeps clips short |
| Pocket TTS is Python | Sidecar boundary; swap for Piper if it becomes a problem |
| Non-commercial TTS weights | Isolated behind the sidecar; not linked into the binary |
| Feature bloat | Everything behind cargo features, off by default |
| Latency regressions | The command path never touches the render path; benchmarked separately |

---

## 11. What this is not

Not a chat interface. Not an agent that edits files on its own. Not a
replacement for keybindings.

It is a second way to say the thing you already know how to do, for the cases
where you cannot remember the keybinding or the menu path. If it ever becomes
the *only* way to do something, the design has failed.
