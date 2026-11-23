# Roadmap

MetaMind aims to be the fastest local-first knowledge base, with database-style structure that
stays plain Markdown on disk. Each level below builds on the one before it and ships as a release
with one headline feature; the performance budgets in [FEATURES.md](FEATURES.md#performance-budgets)
gate every release.

Status today (v0.6.1): editor, links, search, graph, canvas, themes, hotkeys, WASM plugins and Rhai
scripts are in place, and most of Level 1 has landed, including spellcheck, PDF export, pasting
images, split panes, stacked tabs, workspaces, pop-out windows and bases (table, cards, list and
kanban views). Still missing from Level 1: printing, and signed macOS/Windows installers (the
Windows installer is unsigned). There is no sync and no mobile app.

## Level 1: daily driver

Everything someone uses every day in a Markdown notes app.

- Split panes, stacked tabs, saved workspaces, pop-out windows
- Bookmarks, tags pane, explorer sorting and drag-to-move
- Templates with variables; unique-note command
- Slash command menu; `[[` autocomplete with previews; hover previews of links
- Paste and drop images and files into notes; JPEG, GIF, WebP and SVG images
- Unlinked mentions in the backlinks pane
- Session restore: tabs, scroll positions and folds
- Spellcheck, word goal, reading time
- PDF export and print
- Follow the OS light/dark setting; accent colour, font and line-width settings
- Signed and notarized macOS builds; a Windows installer

## Level 2: structure on plain files

Database views over front-matter properties, saved as readable files next to the notes.

- Table, board, calendar, gallery and timeline views
- Filters, sorts, grouping, formulas, relations and rollups stored as properties
- Database views embedded inline in notes
- Block handles: drag to move, turn into, duplicate, colour
- Toggle lists, columns, page icons and covers stored in front matter
- Template buttons and a template gallery
- Canvas: edge editing, live embedded notes, freehand drawing

## Level 3: everywhere

- End-to-end encrypted sync with a self-hostable server, plus Git and WebDAV options
- Android app, then iOS, sharing the Rust core
- Quick capture, share sheet and widgets on mobile

## Level 4: smart and extensible

- Local embeddings for semantic search and related notes; optional model connectors
- Published plugin SDK crate, in-app plugin browser and plugin settings UI
- Theme gallery with live preview; snippet editor; every hotkey binding editable
- Accessibility: screen-reader tree, high contrast, reduced motion, IME pre-edit

## Level 5: collaboration (ceiling)

- CRDT-backed notes with presence and comments
- Shared vaults and web publishing

This level needs hosted infrastructure and ongoing operations, so it is the practical ceiling for
a small open-source project. Levels 1–4 are achievable by a small team.

## The copilot's next step: MCP servers and a local model

The copilot already calls eight functions on the vault (`crates/app/src/ai_tools.rs`) and the
profile can point at any OpenAI-compatible endpoint, including Ollama on this machine (a
`localhost` profile needs no API key). What is missing to make it "connected to everything" is a
**Model Context Protocol client**:

1. `crates/mcp-client`: JSON-RPC 2.0 over stdio (the transport every MCP server ships), `initialize`
   handshake, `tools/list` and `tools/call`. No `unsafe`, no async runtime: a thread per server and
   a channel, the same shape `pen-input` uses.
2. A server list in `Ajustes → IA`: command, arguments, working directory, on/off. Started on first
   use, restarted on crash, killed on exit.
3. The tools of every enabled server are merged with ours in the payload, namespaced
   (`mcp__<server>__<tool>`), and a remote tool's result is truncated like a local one's.
4. Permission: a remote tool runs outside the vault, so the first call to each server asks once
   ("¿Permitir que el servidor X ejecute herramientas?") and the answer is remembered per server.
5. Tests: a mock MCP server (a small Rust binary in `tests/fixtures`) that lists one tool and calls
   it; the loop, the namespacing, a server that dies mid-call, and a tool that returns a huge body.

## Not planned

- Matching the plugin count of large ecosystems; the most-used features ship built in instead
- A browser engine for arbitrary web embeds; common embeds render natively
