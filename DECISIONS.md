# Decisions

Every major tradeoff, with the measurement behind it where one could be taken. Measurements were
made on the development machine: WSL2 Ubuntu, 12 logical cores, 7 GB RAM, no GPU passthrough
(Vulkan and GL both resolve to llvmpipe), rustc 1.99.1, `--release`. Spike sources are in `spikes/`.

## 1. Build toolchain on the development machine

**Situation.** No `cc`, no `libc6-dev`, no `pkg-config`, no sudo. Downloading a standalone C
toolchain (Zig) was not permitted in the session.

**Decision.** Link with `rust-lld` (shipped inside the Rust toolchain) and supply the one missing
piece, the glibc program entry point, as a 15-line `global_asm!` `_start` that calls
`__libc_start_main`. Dev symlinks (`libc.so` → `libc.so.6` and friends) live in
`~/.local/share/walnut-toolchain/lib`. The wiring is in a git-ignored `.cargo/config.toml`; CI and
normal machines use the default linker and need none of this.

**Consequence.** Any dependency with a C or C++ build step cannot be compiled here. That rules out
local measurement of `rquickjs`, `deno_core`, `tree-sitter`, vendored `mlua`, `zstd-sys`, and
`ring`. Where the spec asked for a benchmark between a pure-Rust and a C-backed option, only the
pure-Rust side has numbers; the other side is marked "not measured".

## 2. Spike: UI and CSS engine

| Measurement | Result |
|---|---|
| `tiny-skia` + `cosmic-text`: warm full 1920×1080 frame (200 rects + 60 shaped lines, full repaint) | 2.7 ms |
| Same, first frame (cold glyph cache) | 7.4 ms |
| `cosmic-text` shaping 60 wrapped lines | 3.7 ms |
| `FontSystem::new()` (scans 89 system fonts) | 47 ms |
| `wgpu` 30 `Instance` + adapter enumeration | 434 ms |
| Binary: tiny-skia + cosmic-text + taffy | 4.3 MB unstripped |
| Binary: wgpu + winit probe | 7.5 MB unstripped |

**Decision.** The UI toolkit (`metamind-ui`) renders through a `RenderBackend` trait with a display
list in between. The **software backend (tiny-skia) is the default presenter and the first frame
always comes from it**. Reasons, all measured:

- GPU initialisation alone costs about three times the whole 150 ms cold-start budget on this
  machine. A CPU first frame costs single-digit milliseconds.
- A full-screen CPU repaint is 2.7 ms, and the toolkit repaints only damaged rectangles, so typing
  latency (8 ms budget) is met without a GPU.
- wgpu roughly doubles the binary.

wgpu is reserved for what needs it: the 50,000-node graph view (instanced quads) and, when a real
GPU adapter is present, an optional presenter initialised on a background thread after the first
frame. **None of that is built yet**; the binary contains no wgpu code.

This deviates from "GPU UI on wgpu" as the default path. The spec's own rule 3 (budgets outrank
convenience) decides it.

**Font start-up.** A 47 ms font scan is a third of the cold-start budget. As built, the font
database loads on a worker thread while the vault index loads on the main thread, so it does not
add to start-up time. (A cached font list was considered and not needed.)

**CSS.** A browser engine is excluded by the constraints. `theme-engine` implements a CSS subset
(selectors with combinators, pseudo-classes for state, cascade with specificity and `!important`,
custom properties with `var()` fallbacks, `calc()` on lengths, `@media (prefers-color-scheme)`,
transitions) and maps the community theme variables and class names onto the native widget tree.
Layout was planned on `taffy`; see §7 for why the toolkit has its own flexbox subset instead.
What is outside the CSS subset is listed in `THEME_COMPAT.md`.

## 3. Spike: JS runtime for `compat-js`

| Candidate | Pure Rust | Measured here | Notes |
|---|---|---|---|
| `boa_engine` 0.22 | yes | context init 0.84 ms; plugin-shaped bundle (classes, async, Map, regex, template strings) 2.8 ms; hot loop (200k object allocs + `fib(24)`) 638 ms; binary 16.4 MB unstripped | Correct on everything tried. Slow on hot loops. Large. |
| `rquickjs` (QuickJS-NG) | no (C) | not measured, no C compiler | Published figures put QuickJS at roughly 10–30× faster than boa on this kind of loop and under 1 MB of code. |
| `deno_core` (V8) | no (C++) | not measured | V8 adds roughly 30–40 MB and a long build. Fastest by far; fails the size goal even as an optional module. |

**Decision.** `compat-js` is written against a small `JsEngine` trait. The engine built and tested
in this repository is **boa**, because it is the only one that compiles here and it keeps the
module free of C. `rquickjs` is the recommended production engine on size and speed, and the trait
exists so it can be swapped in; that swap is unverified work, not a claim. `deno_core` is rejected
on size.

Because boa is 16 MB, `compat-js` is a separate `cdylib` (`libmetamind_compat_js.so` /
`metamind_compat_js.dll`) loaded with `dlopen` only when a JS plugin is enabled. It does not count
against, and cannot affect, the 15 MB core budget.

## 4. Spike: editor buffer and the CodeMirror 6 adapter

| Measurement (`ropey` 1.6) | 1 MB note | 5 MB note |
|---|---|---|
| Build rope from string | 1.6 ms | 5.8 ms |
| Insert one char mid-document | 177 ns | 227 ns |
| char → line, fetch line | 1.1 µs | 1.0 µs |
| Snapshot (clone) | 63 ns | 86 ns |

**Decision.** `ropey` is the buffer. Edits and snapshots are far below the 8 ms typing budget, so
the budget is spent elsewhere: reparsing and repainting. Both are bounded to the edited region
(`md-engine` reparses the enclosing block run; the UI repaints damaged lines only). Cheap snapshots
make the undo tree and background reparsing straightforward.

**CM6 adapter.** JS community plugins reach the editor through CodeMirror 6 objects. Re-implementing
CM6 is out of scope; the adapter exposes a compatible *surface* backed by `editor-core`:
`EditorState` (doc, selection, `field()`), `Transaction`/`ChangeSet`, `StateField`, `StateEffect`,
`Compartment`, `EditorView` (`dispatch`, `state`, coords queries), `ViewPlugin`, `Decoration`
(mark, widget, replace, line) mapped to `editor-core` decorations, and `keymap`. Unsupported:
custom `Facet` combinators beyond the built-in ones, the Lezer syntax tree, language packages,
tooltips/panels/gutters extensions. `API_COMPAT.md` carries the exact list.

## 5. Other choices

Rows marked *(not built)* are decisions for components that do not exist yet.

| Area | Choice | Why |
|---|---|---|
| Markdown parser | Own parser in `md-engine` | Incremental region reparse, wikilink syntax and byte-exact source spans are needed together; no existing crate offers all three. |
| Search | Native parallel scan; `tantivy` not built | See §6. |
| Scripting *(not built)* | Rhai | Pure Rust, sandboxed by construction, no C. `mlua` vendored needs a C compiler and could not be measured here. |
| WASM runtime *(not built)* | `wasmi` | Pure-Rust interpreter with fuel metering, small. `wasmtime` (even with Winch/Pulley) could not be built here to compare. |
| Async | No runtime; `std::thread` + channels, `rayon` for indexing and search | Idle CPU must be 0%; a blocking event loop with worker threads is the simplest way to guarantee it. |
| YAML | Own front-matter reader/writer in `md-engine` | Front matter edits must preserve key order and untouched values, which serde-based YAML crates do not guarantee. It covers the YAML subset note properties use (scalars, quoted strings, flow and block lists, nested maps, block scalars); anchors, tags and multi-document streams are not supported. |
| Syntax highlighting | Own tokenizer in `highlight-core` | `tree-sitter` needs a C compiler; `syntect` was not tried to keep the binary small (the whole rich-block work added 0.24 MB). |

## 6. Search backend

The native backend keeps a length-preserving lowercased copy of every note in memory (56 MB for
the 50,000-note synthetic vault, about 11 MB at 10,000 notes) and scans it in parallel with
`memchr::memmem`. Vault search is substring-based, not token-based, which a scan answers
exactly; a token index such as tantivy would need n-gram fields to do the same.

| Query, 50,000 notes | Time (best of 5) |
|---|---|
| `filler text` (matches every note) | 14 ms |
| `note 4242` | 8 ms |
| `tag:#tag7 realistic` | 6 ms |
| `line:(paragraph 3 alias)` (every line is a candidate) | 29 ms |
| `task-todo:open -tag:#tag1` | 15 ms |
| `/note \d+7 linking/` | 18 ms |
| `[aliases:N77]` | 3 ms |
| Quick switcher keystroke, 50,000 paths (`nucleo-matcher`) | 1.5–4.7 ms |

Corpus build is 65 ms with a warm page cache; it is built on first use of search, not at start-up,
so it does not count against cold start, and it is dropped when search is disabled.

`regex-lite` was tried first to save binary size: the regex query took 381 ms. The `regex` crate
takes 18 ms and is used instead.

**Not done:** the `tantivy` backend named in the spec. The native backend meets the stated budget,
so tantivy was deferred rather than built unmeasured. The 56 MB corpus at 50,000 notes is the
known cost; an on-disk n-gram index would remove it.

## 7. UI layout: own flexbox subset instead of taffy

The spec names `taffy`. Its 0.15 leaf-measurement API differs substantially from earlier
versions, and the toolkit needs text measurement in the layout pass. Rather than spend the
session on an integration that could not be checked against documentation offline, `metamind-ui`
implements the flexbox subset the shell uses (direction, grow, shrink, fixed and percentage
sizes, min/max, padding, margin, border, gap, align, justify, absolute positioning, overflow).
Cost: no grid, no wrapping flex lines. Moving to taffy later replaces one file (`dom.rs` layout).

## 8. Start-up: what the profile showed

Measured in the real window with `--profile`, 50,000 notes, before and after each change:

| Change | Cold start to first frame |
|---|---|
| First working version | 775 ms |
| File explorer builds only visible rows; link graph computed after the first frame | 655 ms |
| Explorer sort uses precomputed byte keys instead of a per-comparison case fold; watcher registration moved off the start-up path (recursive inotify registration took about 120 ms) | 425–480 ms |
| Index decode pool capped at 4 threads (4 threads: 150 ms, 12 threads: 210 ms; allocator contention) | 346–391 ms |

Typing latency had one comparable trap: outline and word count were recomputed from the whole
note on every frame (57 ms per keystroke at 1 MB). They are now refreshed 400 ms after the last
edit, and a keystroke costs 4.9 ms.

String interning for tags and link targets was tried to cut memory and reverted: the lookups
ran on the sequential part of start-up (50,000-note start rose to 735 ms) and resident memory
went up, not down.

## 9. Lazy metadata from the mapped index

Idle RAM at 10,000 notes was 83–87 MB against 80 MB, because every note's metadata (about 6 KB)
was decoded at open. Now the index stays memory-mapped and each unchanged note holds only an
offset into it (`cache::Meta::Cached`); new or changed notes hold decoded metadata
(`Meta::Owned`). Result: 34 MB idle, and 50,000-note cold start went from 346–391 ms to
188–260 ms because nothing is decoded at open.

- The link graph reads link targets straight from the map without allocating.
- Search decodes only the parts a query reads (`cache::parts`). Decoding whole notes put `tag:`
  and `task-todo:` searches at 32–41 ms; partial decoding brought them to 10–21 ms.
- Saving the index copies cached blobs verbatim instead of re-encoding them.
- The checksum pass, the graph build and metadata searches touch map pages; they are dropped
  afterwards with `madvise(DONTNEED)` and fault back in from the page cache when needed.
- Cost: `FileEntry::meta()` decodes on every call for cached notes, so callers take it once per
  use. A blob that fails to decode (only possible through a bug, since the file is checksummed)
  reads as empty metadata.
- Not handled: on Windows a mapped file cannot be replaced by rename, so saving the index while
  it is mapped will need the entries re-pointed first. There is no Windows build yet.

## 10. Math: shrink a wide display formula instead of clipping it

**Situation.** A `$$` block is laid out at a fixed 20px and centred in its block. A formula as long
as the ones the calculus vault writes (an integral plus a long fraction plus a sum runs to about
twice the reading column) ran off the right edge of the pane: the end of the line, where the result
of the formula usually is, was not drawn at all.

**Decision.** When the laid-out width exceeds the block, the app re-renders the formula at
`avail / width` of the base size, with a floor of 0.55, rather than clipping it or wrapping it:
clipping loses content, and wrapping a formula mid-line has no correct place to break. The second
pass matters: it is the real font metrics at the smaller size that decide the new width, not a
scale factor applied to the box. Below the floor the formula overflows on purpose, because past
that point shrinking stops being readable.

**Consequence.** Long display lines stay complete at the cost of being smaller than their
neighbours. Inline formulas and the inline row (`Kind::MathRow`) are untouched: they were never
clipped, and their own row already has room. The block height is still reserved with the
font-independent estimate before real fonts exist, so a shrunk formula has some air above and
below it.

## 11. Calendar: the secret iCal address instead of OAuth

**Situation.** "Connect Google Calendar" usually means the OAuth consent flow, a client id, a
redirect URI, refresh tokens and a browser hop. That is a network of credentials the app would
have to store and rotate, and it is what a subscription already replaces: Google publishes every
calendar as a `.ics` file at a URL under Settings → Integrate calendar → Secret address in iCal
format.

**Decision.** The user pastes that address once. The app GETs it with `ureq` on a worker thread
(timeouts of 15 s, its own user agent), caches the body in `.metamind/cache/calendar.ics`,
revalidates with `If-None-Match`/`If-Modified-Since`, and paints the month from the cache first.
`calendar-core` reads the body: line unfolding, properties with parameters, `DTSTART`/`DTEND` with
`TZID`, `VALUE=DATE` and `DURATION`, and `RRULE` expansion for `DAILY`/`WEEKLY`/`MONTHLY`/`YEARLY`
with `INTERVAL`, `COUNT`, `UNTIL` and, for weekly rules, `BYDAY`. The address is a secret: it is
never put in a notice, a log or the cache metadata, which keeps only a hash of it, and the error
messages of a failed fetch are written by hand instead of being taken from the HTTP client's
`Display`, which carries the URL.

**Consequence.** No OAuth, no credentials, no browser: pasting one URL is the whole setup, and
`.ics` files in the vault (a Google export, an Apple calendar) are read by the same parser with no
extra work. The costs: the calendar is read-only in both directions, a revoked address shows as
HTTP 401/403 until the user pastes a new one, and without a time-zone database a `TZID` is kept as
a label — the wall clock the file writes is the wall clock that is shown, so an event written in
another zone is displayed at its own local time. `BYSETPOS`, `BYMONTHDAY`, `BYMONTH`, `WKST`,
`EXDATE` and `RECURRENCE-ID` are parsed past and ignored: an edited instance of a series shows on
its original day. The task lines a note carries (`@{YYYY-MM-DD}`) are scanned in slices of 1500
notes per frame, so the first scan of a 50,000-note vault does not stall a frame.

## 12. Known misses

- **Search uses a native scan, not tantivy** (§6).
- **The default renderer is the CPU, not wgpu** (§2). No wgpu code exists yet.
- **taffy is not used** (§7).
- Everything listed as `none` in the README checklist.


## 31. `flex-wrap` in the layout engine instead of a second layout library

**Situation.** The Excalidraw palette is twenty-four swatches in a box 76 px wide. They came out in
one row that ran out of the pane, and the toolbar above them was a second absolutely positioned
box with a fixed `top`. The engine documents "no wrapping flex lines".

**Decision.** Implement wrapping in `metamind-ui`: `break_lines` over the children's preferred
widths, per-line grow and shrink, `row-gap` between lines and `align-content`. A wrapping row is
laid out by `place_wrap`, a path of its own: the single-line path is what every existing panel goes
through, and rewriting it to also handle lines would risk a regression that only shows on screen.

**Consequence.** One more rule the CSS a theme can use actually does something, and the palette, the
tags and any future chip row wrap instead of overflowing. Grid and floats are still out.

## 32. The pen comes from `evdev`, not from the window toolkit

**Situation.** winit gives a window a pointer position and three buttons. A stylus has pressure and
tilt, and no toolkit on Linux forwards them to a plain window.

**Decision.** A crate of its own, `pen-input`, reads `/dev/input/event*` with a blocking `read` and
decodes `input_event` by its offsets: no `libc`, no `bindgen`, no `unsafe` (the workspace forbids
it). The app combines the sample with the pointer the toolkit reports, because the pen and the
cursor are the same thing on screen.

**Consequence.** Pressure and the eraser work on a tablet the user can read; everything else keeps
working with the mouse. The cost is a Linux-only feature and a permission the settings panel
explains.

## 33. The copilot writes without asking

**Situation.** The operator asked for an agent connected to the vault, with tools, and chose
"tools + auto-apply" over a confirmation per write.

**Decision.** Writes go through the same `vault_core` calls the editor uses (atomic writes, recovery
snapshots), paths are validated inside the vault, and the panel shows one row per call with what it
did. A turn is capped at four rounds of tools, so a model that keeps asking cannot loop.

**Consequence.** The copilot can create and extend notes without a dialog. The undo and the
recovery snapshots are the way back, which is why every write goes through the store and not
through `std::fs`.
