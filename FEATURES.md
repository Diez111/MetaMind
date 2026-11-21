# Features

The authoritative list of what Rustsidian does today. Nothing outside it is claimed.

`done` = implemented and covered by tests. `partial` = usable, gaps listed. `none` = not implemented.

### Vault and files

| Feature | Status | Notes |
|---|---|---|
| Open an existing vault with zero migration | done | Dot-folders are skipped |
| Read/write `.obsidian/*.json`, unknown keys preserved | done | `app`, `appearance`, `core-plugins`, `community-plugins`, `hotkeys`, `bookmarks`, `workspace`, `graph`. Only `app`, `appearance`, `hotkeys`, `graph` and `core-plugins` are *acted on*; the rest are loaded and preserved but unused |
| Atomic writes | done | temp file + fsync + rename |
| Conflict detection with external edits | done | A changed file is never overwritten |
| File watcher with debounce | done | Linux inotify verified; other platforms untested |
| Metadata cache (links, embeds, tags, headings, blocks, front matter, list items, sections) | done | |
| Link resolution (shortest path, aliases, headings, blocks) | done | |
| Rename with link updates | done | Property-tested over 40,000 random vaults. Links in `.canvas` files are **not** updated by the app yet (`canvas-core` has the function) |
| Persisted index, mmap, mtime/size revalidation, rebuild on corruption | done | |
| Trash | partial | Vault `.trash/` and XDG trash; Windows recycle bin falls back to `.trash/` |
| File recovery snapshots | partial | Snapshots are written on save; there is no UI to browse or restore them |

### Markdown

| Feature | Status | Notes |
|---|---|---|
| CommonMark + GFM blocks and inlines | partial | Not run against the CommonMark spec suite; known simplifications in lazy continuation and HTML blocks |
| Wikilinks, embeds, block ids, callouts, comments, highlights, tags, footnotes, front matter, task lists, tables, math spans | done | Parsed with byte-exact spans |
| Incremental re-parse | done | Property-tested equal to a full parse (60,000 cases) |
| HTML rendering | done | Used by export |
| Math rendering | partial | `math-core`: `$$` blocks typeset (fractions, roots, scripts, big operators with limits, `\left\right`, matrices, aligned, `\text`, `\mathbf/rm/bb/cal`, `\underbrace`/`\overbrace` with the annotation against the brace, `\overset`/`\underset`, `\textcolor`/`\color` with names or `#rrggbb`, `\boxed`, `\xrightarrow`/`\xleftarrow` with a label on either side, `\tag`/`\tag*`, `\not`). The box model carries a colour per run and exposes the block padding and the run gap (`Metrics`). Inline `$..$` is typeset in a row under its paragraph (not in the text flow); unknown commands show their source and come back as `{offset, command}` diagnostics. **`\tag` is reported, not placed**: `Layout::tag` and `Rendered::tag` say what the number is and where the fallback run landed (one em past the last item, since the box model has no line to measure); the app is what puts it against the right margin, and it does not do that yet. `\boxed` reuses the caller's `Metrics` as the frame padding and, when the whole formula is one box, skips the outer padding so the frame is the box edge to edge; the frame is four `Seg`s at a 0.06 em stroke (heavier than a 0.05 em rule) because `Seg` has no style field of its own yet. `\not` grows the box to hold its slash (TeX lets the slash overhang). The symbol table also carries the order/relation/logic marks the rest of the vault writes (`\preccurlyeq`, `\prec`/`\succ` and their variants, `\nmid`/`\nparallel`, `\vdash`/`\dashv`/`\models`, `\veebar`, `\blacksquare`/`\square`, `\checkmark`, `\sqcap`/`\sqcup`, `\leqslant`, `\subsetneq`, `\nsubseteq`, `\llbracket`/`\rrbracket`, `\longmapsto`, `\rightleftarrows`, `\rightsquigarrow`, `\backslash` and the obvious neighbours), so a whole-vault scan (64 236 formulas, all subjects) now reports 0 unknown commands. `\pmod{n}` typesets `(mod n)` with relation spacing and `\bmod` is the bare operator with binary spacing; both set the word in the model's monospace face, which is a deliberate divergence from amsmath's upright roman: the model has one mod style and it is the monospace run. `\check` draws the caron as two `Seg`s, `\texttt` is a monospace run that keeps its spaces, `\phantom` keeps the body's box and drops its ink, `\middle` inside `\left\right` stretches to the tallest chunk, and `\hline` in an `aligned`/`matrix` row draws a rule over that row. The `\big`/`\Big`/`\bigg`/`\Bigg` family is drawn at TeX's 1.2/1.8/2.4/3 times the text size, centred on the math axis, and its `l`/`r`/`m` suffixes give the Open/Close/Rel spacing classes. Inter-atom spaces are the TeXbook's Table 18.4 (thin 3mu, medium 4mu, thick 5mu), with `Inner` atoms for fractions, `\left\right` groups and matrices, so `= \frac{a}{b}` gets the thick space and `a\frac{a}{b}` the thin one; script styles get none, as in TeX. In the app, a `$$` formula wider than its block is re-rendered smaller (down to 0.55 of the block size) instead of running off the edge, and the reader can still click it to edit the source. Widths and ink extents come from the caller's `Measure` (`Measure::extents` defaults to the built-in estimate, which is what `Approx` is: a fallback for callers with no fonts, not a measurement). Still missing: `\substack`, `\array` column specs, `\hline` outside a matrix row (parsed, zero width), and per-glyph font coverage: a symbol the font lacks is drawn from the system's fallback face, so it keeps its meaning but not its weight (a `Measure::covers` hook is what a stroked fallback would need). |
| Code syntax highlighting | done | `highlight-core`, hand-written (no dependencies): rust, js/ts, python, json, yaml, toml, bash, html, css, c/c++, go, java and relatives, sql, markdown, diff, generic fallback. Colors come from the theme's `--code-*` variables. Cached per block text, blocks over 256 KB are left plain. Not incremental inside one huge block |
| Mermaid diagrams | partial | `mermaid-core`: `graph`/`flowchart` (TD/LR/RL/BT, node shapes, edge labels and styles, subgraph frames, layered layout) and `sequenceDiagram` (messages, notes, loop/alt frames, autonumber); other diagram types show the source with a notice |
| Fuzzing with `cargo-fuzz` | none | Property tests with arbitrary input stand in for it |

### Editor

| Feature | Status | Notes |
|---|---|---|
| Source mode, live preview | partial | Inline markup is hidden/revealed around the cursor. Quotes, callouts (coloured by type, foldable ones collapsed when written `-`) and tables are rendered while the cursor is outside them and turn back into source when it enters. Table columns are sized by estimate, not measurement; right alignment is not applied; callouts have no icons and no fold toggle. Math and Mermaid are shown as source |
| Reading mode | partial | Live preview without a cursor; not a separate rendered document |
| Multi-cursor, undo tree, smart lists, auto-pair, indent, formatting toggles, move/duplicate line | done | |
| Folding | done | Headings, list items, quotes/callouts and code blocks. Arrow next to each heading, `Toggle fold`, `Fold all`, `Unfold all`. Folds follow edits and undo and are restored with the session. While any fold is active the whole note is rescanned after each edit |
| Table editing | partial | In live preview with the cursor in a table: Tab/Shift+Tab/Enter move between cells and re-align the source; palette commands add, move, delete, align and sort rows and columns, and insert a table. Editing happens in the table's source, not in the rendered grid |
| Vim mode | partial | On when `vimMode` is set in `.obsidian/app.json`, or with `Toggle Vim key bindings`. Subset documented in `editor-core/src/vim.rs`; the mode is shown in the status bar; the caret does not change shape |
| IME | partial | Composing (pre-edit) text is shown underlined at the caret with the input method's cursor, and the candidate window is placed at the caret. fcitx/ibus untested |
| RTL, ligatures | partial | Whatever `cosmic-text` shaping provides; not tested |
| Link and command suggestions | partial | Typing `[[` lists notes and files (fuzzy) with a preview of the selected note; Enter or Tab inserts the link in the vault's link format. `/` at the start of a word lists commands and runs the chosen one. Arrows move, Escape dismisses. Not offered inside code blocks. No heading or block (`#`, `^`) completion |
| Hover preview | partial | Ctrl+hover over a link in the editor shows the target note rendered like an embed (formatting, headings, code, quotes; first 30 blocks). Click the title or body to open it. No scrolling, images or nested previews |
| Drop files from outside the app | done | Copied into `attachmentFolderPath` (`/`, `./`, `./sub` or a vault folder) with a counter on name clashes, then linked at the cursor as `![[file]]` (or a Markdown link) in the vault's link format; files already in the vault are linked in place |
| Spellcheck | done | Bundled en_US dictionary (SCOWL); misspellings underlined in the editor (not in code, links, tags or front matter), suggestions and "add to dictionary" from the right-click menu, `spellcheck` setting. English only |
| Paste files and images | done | Files copied in a file manager are imported like dropped files; a copied image (PNG, or a Windows bitmap) is saved as `Pasted image <time>.png` in the attachment folder and embedded. Linux needs `wl-paste` or `xclip` for this (not `xsel`); macOS reads only the first of several copied files |
| Properties editor | partial | Front matter shown as a table in live preview and reading view: text, number, checkbox, date, list pills (tags, aliases, cssclasses). Edit, add (`properties:add`), rename, remove; edits are buffer edits (undo works) and only the changed entry is rewritten. `properties:toggle-source` or the `propertiesInDocument` setting shows raw source. No type picker or `types.json`; flow lists become block lists when edited; nested values are read-only |
| Embeds of notes and images | partial | `![[Note]]`, `![[Note#Heading]]`, `![[Note#^block]]` show the note or section below the paragraph (first 30 blocks; tables and nested embeds inside it as source). `![[image.png\|100x80]]` and `![](image.png)` show PNG, JPEG, GIF (first frame), WebP and SVG images, decoded on the UI thread at first display; remote images are not shown |
| Embeds of audio, video, PDF | none | |

### Navigation and views

| Feature | Status | Notes |
|---|---|---|
| File explorer | partial | Virtualised, collapsible, file icons; right-click menus for files, folders, tabs and the editor (new note/folder, rename with link updates, make a copy, delete to trash, copy path or link, show in system explorer, cut/copy/paste). Sort by name, modified or created time (`fileSortOrder`). Drag a file or folder onto a folder (or empty space for the root) to move it, with link updates |
| Quick switcher, command palette | done | |
| Search panel with query operators | done | All operators; an empty search field (search pane and graph filter) lists `path:`, `file:`, `tag:`, `line:`, `section:` and `[property]`; no embedded `query` blocks |
| Back/forward, tabs | done | Open tabs, view mode, scroll, cursor, folds, pinned state and sidebar state are restored when the vault reopens. Pinned tabs show a pin and are skipped by every close command until unpinned; the tab menu also bookmarks the note and copies a screenshot of the open pane as an image (Windows, macOS, and Linux with wl-clipboard or xclip). Split right/down from the tab menu or commands; each pane has its own tabs, dividers drag to resize, the same note in two panes stays in sync, and "Link with" makes panes follow each other's note. The pane layout is restored with the session. "Move to new window" (tab menu or command) shows a tab in a window of its own; closing that window docks its pane back into the main one, and the next start restores it as a split. "Stack tabs" (tab menu or command) shows a pane's tabs side by side, the ones that do not fit folded to spines with their title; it is kept per pane in the session. Workspaces (core plugin) save, load and delete named layouts of tabs, panes and sidebars, in `rustsidian-workspaces.json` (Obsidian's `workspaces.json` is not read). No splits inside a pop-out window, or screen-reader support in pop-out windows; no directional pane focus commands (click a pane to focus it) |
| Outline, backlinks, unlinked mentions, word count | done | Right sidebar tabs for Backlinks, Outgoing links and Outline. Linked mentions show up to three snippets around each link, with Collapse results, Show more context (the whole paragraph), sort order and a search filter over file names and snippets; outgoing links list resolved and unresolved targets. Unlinked mentions match the file name and aliases as whole words, with a Link button per note. Status bar shows reading time (200 words a minute) and progress against a `word-goal` property. |
| Unique note creator | done | Format, folder and template from `zk-prefixer.json`; a name already taken gets a counter |
| Daily notes, random note | done | Daily note uses the local date with the folder, format and template from `daily-notes.json`; commands open the previous and next existing daily note |
| Graph view | partial | Global and local graph (depth 1-3) in the main pane: pan, zoom about the pointer, drag nodes, hover highlight, click to open (unresolved creates the note, tags search). Settings panel with search filter, filter toggles, color groups, arrows, text fade, node size, link thickness, the four forces and Animate, all saved to `graph.json`. No layout cache on disk; dense graphs draw a sample of links |
| Canvas view | done | Text/file/link/group nodes, colors, curved edges with arrows and labels; pan, wheel zoom, move, resize, double-click to edit text, add/delete node, lossless save. No edge editing; file nodes show a text preview |
| Excalidraw boards (`.excalidraw`) | partial | A note's `![[Tablero.excalidraw]]` embed is drawn in the note: in reading view always, and in live preview once the caret leaves the line (the `![[…]]` stays editable while the caret is on it). `excalidraw-core` reads format-2 scenes — the eight element types, unknown fields kept at every level so a board round-trips — into a display list, which paints as boxes, ellipses, diamonds, polylines, arrowheads, text runs and image placeholders. The board fits the note column without being enlarged, its own page colour is painted with a hairline edge, the file's colours and font sizes are used, its font families map onto the app's text and mono faces, and the board is read once per modification time. A board that cannot be parsed keeps its `![[…]]` line with a notice saying why, and elements the display list skipped are listed in one line under the board. Not interactive (no clicking, panning, zooming or editing of a board in the note) and a `.excalidraw` file opened on its own has no viewer; nothing writes these files yet (the `.canvas` export through `canvas-core` remains the only writer, with no form for freedraw, images or unbound arrows). Sketchy strokes and hachure fills are painted straight, rounded corners are square, a rotated shape keeps its corners and a rotated text run stays upright, arrowheads are a simple mark rather than Excalidraw's filled head, elbow routing is not re-computed, freedraw pressure is ignored, and an image element is a labelled placeholder (its data URL is not decoded). Notes in the Excalidraw plugin's `.excalidraw.md` format are ordinary notes here: their compressed scene block is not rendered. Frames and embeds are read but not drawn |
| Bookmarks, tags pane, templates | done | Bookmarks for notes and searches in `bookmarks.json`; tags pane; insert template, date and time with `templates.json` folder and formats |
| Properties view, periodic notes beyond daily, slides, format converter, audio recorder, footnotes view | none | |
| Bases | partial | `.base` files open as table, cards, list or kanban views (`type: kanban` puts cards in a column per `groupBy` value; "Move…" on a card sets the group property). Supported: `filters` (expressions and nested `and`/`or`/`not`), `formulas`, `properties.*.displayName`, and per-view `filters`, `order`, `sort`, `limit` and `groupBy`. Expressions cover note properties, `file.*` fields, `file.hasTag/inFolder/hasLink/hasProperty`, comparison, arithmetic, `if`, `now`, `today`, and common string, list and number methods. Click a property cell in a table to set it in the note (keeps text, number, checkbox and list types; empty removes it). Formula and `file.*` cells are read-only. The cell prompt also offers the values the column already has. Views cannot be edited from the UI; kanban cards move by prompt, not drag, no embedded `base` code blocks, and dates compare as text. |
| `dataview` blocks (`TABLE` queries over front matter) | partial | `dataview-core`: `TABLE [WITHOUT ID] <field> [AS "Header"], …`, `FROM "<folder>"` (subfolders included), `WHERE <field> <op> <literal>` joined with `AND`, and `SORT <field> [ASC\|DESC], …`. Columns are front matter keys (dotted names reach into maps) plus `file.name`, `file.link` and `file.mtime`; without `WITHOUT ID` the first column is a link to the note. Operators: `=`, `!=`, `<`, `<=`, `>`, `>=` against text, a number, `null` or `date(YYYY-MM-DD)`; a missing key, an empty string and an empty list read as `null`. The result table takes the place of the fence in reading view and in live preview while the cursor is outside it, and a broken query keeps its source with a notice. The notes in the folder are scanned once per vault revision and the result is cached. No `LIST`, `TASK` or `CALENDAR` queries, no `FROM` by tag or link, no `GROUP BY`, `FLATTEN`, `LIMIT` or calculations, no inline fields or multi-line expressions, and the result is not editable |

### Theming and customisation

| Feature | Status | Notes |
|---|---|---|
| CSS subset engine | done | See `THEME_COMPAT.md` for the exact subset |
| Themes from `.obsidian/themes/`, snippets from `.obsidian/snippets/`, live reload | partial | Loaded and reloaded on change. **No community theme has been tested**; themes that rely on unsupported CSS will apply only in part |
| Light/dark | done | Follows `appearance.json`; "Adapt to system" follows the OS setting on Windows, macOS and Wayland (X11 reports no preference, so dark is used) |
| Interface icons | done | Lucide geometry (lucide-static v1.77.0, ISC), copied into `crates/app/src/icons.rs`: each path is Lucide's own, on its 24 unit grid with a 2 unit stroke, so the 16 px icon the CSS asks for strokes at 1.33 px. Cubic curves are flattened to polylines at paint time, so the painter still draws plain segments. Rustsidian keeps its own icon names, so the ones Lucide does not have (`note`, `canvas`, `graph`, `switcher`, `command`, …) point at the nearest Lucide icon; the header of that file lists the mapping. Not vendored at run time: no `.svg` is read, and the theme's text color still paints them |
| Commands with stable ids, rebindable through `hotkeys.json`, key sequences | done | Settings window (`Ctrl+,`, ribbon gear) with a search across all sections: General (version, vault, updates), Editor, Files & links, Appearance (scheme, theme, snippets, font size, text font, accent color, readable line width), Interface (show ribbon, show tab title bar), Hotkeys (rebind, reset; first binding only), Core plugins (turning one off hides its ribbon, sidebar and palette entries and blocks its commands). Saved to `app.json`, `appearance.json`, `hotkeys.json` with unknown keys kept. Dropdowns cycle on click |
| Everything else in the customisation section (TOML config, JSON Schema, data-driven layout, theme inspector UI, feature flags, profiles, portable mode, `--config-dir`) | none | The theme engine has an `inspect` API; no UI uses it |

### Export, plugins, platform

| Feature | Status | Notes |
|---|---|---|
| Export HTML, Markdown bundle | done | |
| Export PDF, print | partial | `Export to PDF` writes the note as the reading view lays it out (light theme, A4, vector text with embedded font subsets, page breaks between blocks). No print dialog, page size or margin settings, and links are not clickable in the PDF |
| Native WASM plugins (`.obsidian/plugins/<id>/main.wasm`), `plugins:list`, `plugins:reload`, `community-plugins.json` enable list | partial | wasmi host with fuel/memory limits and a permission model (see `PLUGIN_SDK.md`); example plugin in `examples/plugins/hello-wasm`. No SDK crate, no `rustsidian plugin` scaffold, no plugin settings UI (the app exposes `App::plugin_list` / `set_plugin_enabled` for it) |
| User scripts in Rhai (`.obsidian/scripts/*.rhai`) | partial | `scripting` crate on the same API as WASM plugins: commands, events, vault and editor helpers; sandboxed with operation, size and time limits. No hotkeys from `hotkeys.json` |
| JavaScript community plugin compatibility, CM6 adapter, plugin browser | none | Only the JS engine spike was done |
| Accessibility (`accesskit`), high contrast, reduced motion | partial | Screen-reader tree through `accesskit` (AT-SPI, UI Automation, NSAccessibility): buttons, headings, text and labelled images, with click actions. The caret and selection are not exposed. Reduced motion and high contrast are read from the OS (Windows animation effects and high-contrast themes, GNOME `enable-animations` and `high-contrast`, macOS Reduce motion and Increase contrast) at start and on focus. Reduced motion stops interface animations and turns on `prefers-reduced-motion`; high contrast turns on `prefers-contrast: more` and a black/white palette over any theme. No in-app toggle for either |
| GPU rendering (`wgpu`) | none | Software rendering only, by measured decision (DECISIONS.md §2) |
| Multi-window | partial | A pane can move to a window of its own; closing it docks the pane back, and the session saves panes docked. One vault per process |
| System tray, native file dialogs | none | |
| File associations | partial | `rustsidian <file.md>` (or a `file://` URL) opens the note in the vault that contains it. The Linux `.desktop` entry declares `text/markdown` and `x-scheme-handler/rustsidian`. The Windows installer registers the link scheme and an "Open with" entry for `.md`. Nothing is registered on macOS (no app bundle) |
| Single-instance hand-off | partial | A launch for a vault that is already open (a link, `rustsidian <vault> <note>`) sends the note to that window over loopback with a per-process token, brings the window forward and exits. Other vaults still start their own process |
| `rustsidian://` links (`open?vault=&file=`, `open?path=`, `new?vault=&name=&content=`) | partial | Passed as the first command-line argument; vaults by name come from the recent-vaults list. The scheme is registered by the Linux `.desktop` entry and the Windows installer, not on macOS, and a link for a vault that is already open goes to its window |
| Packaging (MSI, zip, Flatpak, AppImage, deb, rpm) | partial | Release workflow publishes tar.gz/zip archives with checksums for Linux, Windows and macOS, plus a per-user Windows installer (Inno Setup, `packaging/rustsidian.iss`); `.desktop` file and SVG icon included. No MSI, Flatpak, AppImage, deb or rpm. The workflow has not been run |
| `rustsidian doctor`, `--profile`, perf overlay (Ctrl+Shift+F12) | done | |


## Performance budgets

Machine: WSL2 Ubuntu, 12 logical cores, 7 GB RAM, software rendering (no GPU passthrough).
Synthetic vaults from `perf_budget`.

| Budget | Target | Measured | |
|---|---|---|---|
| Release binary, stripped | < 15 MB (aim < 10) | 12.0 MB with the default `wasm-plugins` and `scripting` features (6.5 MB before the plugin stack) | met |
| Cold start to presented frame, 5,000 notes | < 150 ms | 65–98 ms (worst of 5 runs, three sessions) | met |
| Cold start to presented frame, 50,000 notes | < 400 ms | 188–260 ms | met |
| Idle RAM, 10,000 notes | < 80 MB | 34 MB | met |
| Idle CPU | 0% | 0 CPU ticks over 5 s | met |
| Keystroke to pixels, 1 MB note | < 8 ms | 4.9 ms median, 6.2 ms p95, 8.7 ms worst seen | met at p95 |
| Scroll frame, 5 MB note | display rate | 7.4 ms mean (60 Hz needs 16.7 ms; 144 Hz would need 6.9 ms) | met for 60 Hz |
| Full-text search, 50,000 notes | first results < 30 ms | 14–29 ms for plain terms, tags, tasks, properties and regex (regex 15–16 ms); `line:(paragraph 3 alias)` takes 43–51 ms | **missed** for `line:` queries with several terms |
| Quick switcher, 50,000 paths | < 5 ms per keystroke | 1.8–2.0 ms | met |
| Graph, 50,000 nodes at 60 fps | pan/zoom | 21 ms per frame at 50,000 nodes / 600,000 links, 15.5 ms at 10,000 / 120,000 (single runs), 1.7 ms for a small vault; CPU rasterizer, links sampled past 3M line pixels; layout tick 26 ms on a worker thread | **missed** |
| Mermaid render | < 50 ms | flowchart/sequence layout + paint inside the 5-7 ms frame | met |

Caveats, stated plainly:

- "Cold start" means a fresh process with the persisted index present. The very first open of a
  vault has to parse every note: 47 ms at 5,000 notes, 433 ms at 50,000. It runs in the background
  while the start screen shows its progress.
- Start-up and typing figures are for the software renderer and include everything up to the
  frame being handed to the compositor, not the compositor's own latency.
- Idle RAM counts resident pages after the mapped index has been released (`madvise`). Note
  metadata is decoded from the mapped index on demand, so a vault-wide metadata query (graph
  build, `tag:` search) briefly raises it. On Windows there is no release call yet.
- The budgets are measured by `perf_budget` and `scripts/window-budget.sh` on every CI run
  (GitHub-hosted runners). The search table is report-only there, since shared runners vary.

