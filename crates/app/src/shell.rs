//! Application state, commands and input handling. Headless: the window layer feeds it keys,
//! text and mouse events and asks it for a view.

use editor_core::deco::Mode;
use editor_core::{EditKind, Editor, Motion, Sel};
use search_core::{Corpus, Fuzzy, Query, RankWeights, Sort};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use theme_engine::Theme;
use vault_core::config::VaultConfig;
use vault_core::fsio::{self, Recovery, Stamp};
use vault_core::{path, FileKind, VaultStore, WriteError};

pub const BASE_CSS: &str = include_str!("base.css");
pub const CONTRAST_CSS: &str = include_str!("contrast.css");

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct Key {
    /// `a`..`z`, digits, punctuation, or a named key: `Enter`, `Escape`, `Tab`, `Backspace`,
    /// `Delete`, `ArrowLeft`, `ArrowRight`, `ArrowUp`, `ArrowDown`, `Home`, `End`, `PageUp`,
    /// `PageDown`, `F1`…
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Key {
    pub fn new(key: &str) -> Key {
        Key { key: normalize_key(key), ctrl: false, shift: false, alt: false }
    }
    /// Parse `Ctrl+Shift+P`. `Mod` is accepted as an alias of `Ctrl` (the name used in `hotkeys.json`).
    pub fn parse(s: &str) -> Option<Key> {
        let mut k = Key { key: String::new(), ctrl: false, shift: false, alt: false };
        let parts: Vec<&str> = if s == "+" { vec!["+"] } else { s.split('+').collect() };
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "mod" | "cmd" | "meta" | "control" if !last => k.ctrl = true,
                "shift" if !last => k.shift = true,
                "alt" | "option" if !last => k.alt = true,
                "" => return None,
                _ => k.key = normalize_key(part.trim()),
            }
        }
        (!k.key.is_empty()).then_some(k)
    }
    /// A key sequence such as `Ctrl+K Ctrl+S`.
    pub fn parse_sequence(s: &str) -> Option<Vec<Key>> {
        s.split_whitespace().map(Key::parse).collect()
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        parts.push(if self.key.len() == 1 { self.key.to_uppercase() } else { self.key.clone() });
        write!(f, "{}", parts.join("+"))
    }
}

fn normalize_key(k: &str) -> String {
    if k.chars().count() == 1 {
        return k.to_lowercase();
    }
    match k.to_ascii_lowercase().as_str() {
        "esc" | "escape" => "Escape",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "arrowleft" | "left" => "ArrowLeft",
        "arrowright" | "right" => "ArrowRight",
        "arrowup" | "up" => "ArrowUp",
        "arrowdown" | "down" => "ArrowDown",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "space" => " ",
        _ => return k.to_string(),
    }
    .to_string()
}

/// (stable id, name shown in the palette, default hotkey)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("command-palette:open", "Command palette: Open", "Ctrl+P"),
    ("switcher:open", "Quick switcher: Open", "Ctrl+O"),
    ("global-search:open", "Search: Search in all files", "Ctrl+Shift+F"),
    ("file-explorer:open", "Files: Show file explorer", ""),
    ("bookmarks:open", "Bookmarks: Show bookmarks", ""),
    ("bookmarks:bookmark-current-view", "Bookmarks: Bookmark or unbookmark current note", ""),
    ("bookmarks:bookmark-current-search", "Bookmarks: Bookmark current search", ""),
    ("tag-pane:open", "Tags: Show tags", ""),
    ("templates:insert-template", "Templates: Insert template", ""),
    ("templates:insert-current-date", "Templates: Insert current date", ""),
    ("templates:insert-current-time", "Templates: Insert current time", ""),
    ("file-explorer:new-file", "Create new note", "Ctrl+N"),
    ("file-explorer:new-folder", "Create new folder", ""),
    ("workspace:edit-file-title", "Rename file", "F2"),
    ("workspace:copy-path", "Copy file path", ""),
    ("workspace:export-pdf", "Export to PDF", ""),
    ("file-explorer:reveal-active-file", "Reveal current file in navigation", ""),
    ("file-explorer:collapse-all", "Files: Collapse or expand all folders", ""),
    ("editor:save-file", "Save current file", "Ctrl+S"),
    ("workspace:close", "Close current tab", "Ctrl+W"),
    ("workspace:close-others", "Close all other tabs", ""),
    ("workspace:close-all", "Close all tabs", ""),
    ("workspace:split-vertical", "Split right", ""),
    ("workspace:split-horizontal", "Split down", ""),
    ("workspace:move-to-new-window", "Move current tab to new window", ""),
    ("workspace:undo-close-pane", "Reopen closed tab", "Ctrl+Shift+T"),
    ("workspace:next-tab", "Go to next tab", "Ctrl+Tab"),
    ("workspace:previous-tab", "Go to previous tab", "Ctrl+Shift+Tab"),
    ("app:go-back", "Navigate back", "Ctrl+Alt+ArrowLeft"),
    ("app:go-forward", "Navigate forward", "Ctrl+Alt+ArrowRight"),
    ("app:toggle-left-sidebar", "Toggle left sidebar", "Ctrl+\\"),
    ("app:toggle-right-sidebar", "Toggle right sidebar", "Ctrl+Shift+\\"),
    ("backlinks:open", "Backlinks: Show backlinks", ""),
    ("outgoing-links:open", "Outgoing links: Show outgoing links", ""),
    ("outline:open", "Outline: Show outline", ""),
    ("markdown:toggle-preview", "Toggle reading view", "Ctrl+E"),
    ("editor:toggle-source", "Toggle Live Preview/Source mode", ""),
    ("editor:toggle-bold", "Toggle bold", "Ctrl+B"),
    ("editor:toggle-italics", "Toggle italic", "Ctrl+I"),
    ("editor:toggle-highlight", "Toggle highlight", ""),
    ("editor:toggle-code", "Toggle code", ""),
    ("editor:toggle-strikethrough", "Toggle strikethrough", ""),
    ("editor:toggle-checklist-status", "Toggle checkbox status", "Ctrl+L"),
    ("editor:toggle-blockquote", "Toggle blockquote", ""),
    ("editor:set-heading-0", "Remove heading", ""),
    ("editor:set-heading-1", "Set as heading 1", ""),
    ("editor:set-heading-2", "Set as heading 2", ""),
    ("editor:set-heading-3", "Set as heading 3", ""),
    ("editor:cut", "Cut", "Ctrl+X"),
    ("editor:copy", "Copy", "Ctrl+C"),
    ("editor:paste", "Paste", "Ctrl+V"),
    ("editor:insert-wikilink", "Add internal link", ""),
    ("editor:undo", "Undo", "Ctrl+Z"),
    ("editor:redo", "Redo", "Ctrl+Shift+Z"),
    ("editor:select-all", "Select all", "Ctrl+A"),
    ("editor:swap-line-up", "Move line up", "Alt+ArrowUp"),
    ("editor:swap-line-down", "Move line down", "Alt+ArrowDown"),
    ("editor:duplicate-line", "Duplicate line", ""),
    ("editor:toggle-fold", "Toggle fold on the current line", ""),
    ("editor:fold-all", "Fold all headings and lists", ""),
    ("editor:unfold-all", "Unfold all headings and lists", ""),
    ("editor:insert-table", "Insert table", ""),
    ("editor:table-row-before", "Table: Add row before", ""),
    ("editor:table-row-after", "Table: Add row after", ""),
    ("editor:table-row-up", "Table: Move row up", ""),
    ("editor:table-row-down", "Table: Move row down", ""),
    ("editor:table-row-delete", "Table: Delete row", ""),
    ("editor:table-col-before", "Table: Add column before", ""),
    ("editor:table-col-after", "Table: Add column after", ""),
    ("editor:table-col-left", "Table: Move column left", ""),
    ("editor:table-col-right", "Table: Move column right", ""),
    ("editor:table-col-delete", "Table: Delete column", ""),
    ("editor:table-col-align-left", "Table: Align left", ""),
    ("editor:table-col-align-center", "Table: Align center", ""),
    ("editor:table-col-align-right", "Table: Align right", ""),
    ("editor:table-sort-ascending", "Table: Sort by column (A to Z)", ""),
    ("editor:table-sort-descending", "Table: Sort by column (Z to A)", ""),
    ("editor:table-format", "Table: Align columns in the source", ""),
    ("editor:toggle-vim", "Toggle Vim key bindings", ""),
    ("editor:add-cursor-above", "Add cursor above", "Ctrl+Alt+ArrowUp"),
    ("editor:add-cursor-below", "Add cursor below", "Ctrl+Alt+ArrowDown"),
    ("editor:select-next-occurrence", "Select next occurrence", "Ctrl+D"),
    ("editor:follow-link", "Follow link under cursor", "Alt+Enter"),
    ("canvas:new", "Canvas: Create new canvas", ""),
    ("bases:new", "Bases: Create new base", ""),
    ("canvas:add-text", "Canvas: Add text node", ""),
    ("canvas:delete-selected", "Canvas: Delete selected node", ""),
    ("canvas:edit-selected", "Canvas: Edit selected text node", ""),
    ("canvas:reset-view", "Canvas: Fit canvas to pane", ""),
    ("canvas:focus", "Canvas: Show canvas", ""),
    ("graph:open", "Graph view: Open graph view", "Ctrl+G"),
    ("graph:open-local", "Graph view: Open local graph", ""),
    ("graph:toggle-tags", "Graph view: Toggle tags", ""),
    ("graph:toggle-attachments", "Graph view: Toggle attachments", ""),
    ("graph:toggle-unresolved", "Graph view: Toggle existing files only", ""),
    ("graph:toggle-orphans", "Graph view: Toggle orphans", ""),
    ("graph:toggle-arrows", "Graph view: Toggle arrows", ""),
    ("graph:cycle-depth", "Graph view: Change local graph depth", ""),
    ("graph:reset-view", "Graph view: Fit graph to pane", ""),
    ("graph:toggle-panel", "Graph view: Toggle settings panel", ""),
    ("graph:animate", "Graph view: Animate layout", ""),
    ("graph:reset-settings", "Graph view: Restore default settings", ""),
    ("daily-notes", "Daily notes: Open today's daily note", ""),
    ("daily-notes:goto-prev", "Daily notes: Open previous daily note", ""),
    ("daily-notes:goto-next", "Daily notes: Open next daily note", ""),
    ("zk-prefixer", "Unique note creator: Create new unique note", ""),
    ("random-note", "Random note: Open random note", ""),
    ("app:delete-file", "Delete current file", ""),
    ("theme:toggle-light-dark", "Toggle light/dark mode", ""),
    ("app:reload-theme", "Reload themes and snippets", ""),
    ("app:toggle-perf-overlay", "Toggle performance overlay", "Ctrl+Shift+F12"),
    ("app:open-settings", "Open settings", "Ctrl+,"),
    ("properties:add", "Add file property", ""),
    ("properties:toggle-source", "Toggle properties in document: table/source", ""),
    ("plugins:list", "Plugins: List plugins and scripts", ""),
    ("plugins:reload", "Plugins: Reload plugins", ""),
    ("app:open-vault", "Open another vault", ""),
    ("app:check-for-updates", "Check for updates", ""),
    ("app:install-update", "Install available update", ""),
    ("app:quit", "Quit", "Ctrl+Q"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Source,
    LivePreview,
    Reading,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Files,
    Search,
    Bookmarks,
    Tags,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightPanel {
    Backlinks,
    Outgoing,
    Outline,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Editor,
    Search,
    Prompt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptKind {
    Commands,
    Switcher,
    /// A new name for the file or folder in `Prompt::target`.
    Rename,
    /// The name of a new folder inside `Prompt::target`.
    NewFolder,
    Template,
}

#[derive(Clone, Debug)]
pub struct PromptItem {
    pub label: String,
    pub aux: String,
    pub action: String,
}

pub struct Prompt {
    pub kind: PromptKind,
    /// The path a rename or new-folder prompt acts on.
    pub target: String,
    pub query: String,
    pub selected: usize,
    pub items: Vec<PromptItem>,
}

/// One row of the file explorer.
#[derive(Clone, Debug)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    /// Vault path of a file, or the folder path for a folder row.
    pub path: String,
    pub folder: bool,
    pub collapsed: bool,
}

/// A press on an explorer row. Its click action waits for the release, because pressing
/// may start a drag that moves the file instead.
#[derive(Clone, Debug)]
pub struct ExplorerDrag {
    pub source: String,
    pub action: String,
    pub start: (f32, f32),
    pub moving: bool,
    /// Folder the source would move into ("" is the vault root).
    pub target: Option<String>,
}

pub const TREE_ROW_HEIGHT: f32 = 26.0;
const META_DEBOUNCE_MS: u64 = 400;

pub struct Tab {
    pub path: String,
    pub editor: Editor,
    pub stamp: Stamp,
    pub mode: ViewMode,
    /// Index of the first top-level block shown (the editor scrolls block-wise).
    pub top_block: usize,
    pub last_edit_ms: u64,
    /// Metadata of the buffer text, valid for the buffer version it was computed at.
    pub meta: Option<(u64, vault_core::NoteMeta)>,
    pub folds: editor_core::fold::Folds,
    /// Folded source ranges, valid for the buffer version they were computed at.
    pub hidden: (u64, Vec<(usize, usize)>),
    pub hidden_dirty: bool,
    /// Vim state when vim keys are on.
    pub vim: Option<editor_core::vim::Vim>,
    /// Pinned tabs survive "close" commands until unpinned.
    pub pinned: bool,
}

impl Tab {
    pub fn deco_mode(&self) -> Mode {
        if self.mode == ViewMode::Source {
            Mode::Source
        } else {
            Mode::LivePreview
        }
    }
    pub fn title(&self) -> &str {
        path::stem(&self.path)
    }

    pub(crate) fn new(path: String, text: &str, stamp: Stamp, vim: bool) -> Tab {
        Tab {
            path,
            editor: Editor::new(text),
            stamp,
            mode: ViewMode::LivePreview,
            top_block: 0,
            last_edit_ms: 0,
            meta: None,
            folds: Default::default(),
            hidden: (0, Vec::new()),
            hidden_dirty: false,
            vim: vim.then(Default::default),
            pinned: false,
        }
    }
}

pub struct SearchResult {
    pub path: String,
    pub snippets: Vec<String>,
}

pub struct App {
    /// Whether new tabs get vim keys (`vimMode` in `.obsidian/app.json`, or toggled).
    pub vim: bool,
    pub settings: Option<crate::settings::SettingsUi>,
    pub prop_edit: Option<crate::properties::Edit>,
    /// Images and embedded notes shown by the editor view.
    pub media: std::cell::RefCell<crate::media::Media>,
    pub graph_view: Option<crate::graphview::GraphView>,
    /// The graph is shown in the main pane instead of the active note.
    pub show_graph: bool,
    pub canvas_view: Option<crate::canvasview::CanvasView>,
    pub show_canvas: bool,
    pub base_view: Option<crate::baseview::BaseView>,
    pub show_base: bool,
    pub store: VaultStore,
    pub config: VaultConfig,
    pub theme: Theme,
    pub dark: bool,
    /// The OS light/dark preference, when the window system reports one.
    pub system_dark: Option<bool>,
    /// The OS asks for minimal motion: themes see `prefers-reduced-motion`, panels stop animating.
    pub reduced_motion: bool,
    /// The OS asks for more contrast: themes see `prefers-contrast: more`, which `contrast.css` uses.
    pub high_contrast: bool,
    /// Text the input method is composing, not yet part of the note.
    pub preedit: String,
    /// The input method's cursor as a byte offset into `preedit`.
    pub preedit_cursor: Option<usize>,
    /// Tabs of the focused pane.
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub panes: Vec<crate::split::Pane>,
    pub layout: crate::split::Layout,
    /// Index of the focused pane.
    pub pane: usize,
    pub pane_drag: Option<crate::split::PaneDrag>,
    /// Panes shown in their own windows, as (window id, pane). They are not in `layout`.
    pub popouts: Vec<(u32, usize)>,
    pub(crate) next_popout: u32,
    /// The main window's pane focused last, to focus again when that window is.
    pub(crate) last_main: usize,
    /// The focused note last seen by linked panes, and the note and buffer version last mirrored.
    pub(crate) followed: Option<String>,
    pub(crate) mirrored: Option<(String, u64)>,
    pub left_open: bool,
    pub right_open: bool,
    pub panel: Panel,
    pub focus: Focus,
    pub collapsed: HashSet<String>,
    pub prompt: Option<Prompt>,
    pub search_query: String,
    pub search_results: Vec<SearchResult>,
    pub search_error: Option<String>,
    pub(crate) corpus: Option<Corpus>,
    pub(crate) graph: Option<std::sync::Arc<vault_core::LinkGraph>>,
    pub(crate) tags: Option<crate::panes::TagCounts>,
    /// Unlinked mentions of the active note, computed while their section is expanded.
    pub mentions: Option<crate::mentions::Mentions>,
    pub show_unlinked: bool,
    /// Notes linking to the active note, with the text around each link.
    pub linked: Option<crate::mentions::Mentions>,
    pub right_panel: RightPanel,
    pub backlinks_collapsed: bool,
    pub backlinks_desc: bool,
    /// Whole lines around each backlink instead of a few words.
    pub backlinks_more: bool,
    /// The backlinks filter text; `None` while the filter field is hidden.
    pub backlinks_filter: Option<String>,
    pub backlinks_editing: bool,
    pub(crate) tree: Option<Vec<TreeRow>>,
    /// While set, the link graph is not computed (the first frame is presented without it).
    pub defer_graph: bool,
    pub keymap: Vec<(Vec<Key>, String)>,
    pending: Vec<Key>,
    history: Vec<String>,
    history_pos: usize,
    pub notice: Option<String>,
    pub perf: bool,
    pub last_frame_us: u64,
    /// Copy screenshot was chosen: the window copies the pane from the next frame it draws.
    pub screenshot: bool,
    pub quit: bool,
    /// Scroll offsets of scrollable panes, by element id.
    pub scrolls: HashMap<String, f32>,
    pub(crate) fuzzy: Fuzzy,
    recovery: Recovery,
    pub plugins: crate::plugins::PluginState,
    /// The action of the element under the pointer, for `:hover` styling and the status hint.
    pub hover_action: Option<String>,
    /// An explorer row being pressed or dragged.
    pub explorer_drag: Option<ExplorerDrag>,
    pub suggest: Option<crate::suggest::Suggest>,
    pub hover_preview: Option<crate::popover::HoverPreview>,
    /// Where the suggestion closed with Escape stays closed until the cursor leaves it.
    pub(crate) suggest_dismissed: Option<usize>,
    pub anims: crate::anim::Anims,
    pub(crate) seen: Option<crate::anim::Seen>,
    pub(crate) notice_since: Option<std::time::Instant>,
    pub update: crate::update::State,
    /// Paths of closed tabs, most recent last, for "Reopen closed tab".
    closed_tabs: Vec<String>,
    /// Set by "Open another vault"; the window shows the vault chooser when it sees this.
    pub switch_vault: bool,
    /// The open right-click menu.
    pub menu: Option<crate::menu::Menu>,
    /// Logical window width of the last frame, to keep menus on screen.
    pub window_w: f32,
    /// Off in headless use (tests, screenshots) so they never touch the user's clipboard;
    /// copies then stay inside the application.
    pub system_clipboard: bool,
    pub(crate) clip: String,
}

/// Sort key for explorer order: at each level folders come before files, and names compare
/// case-insensitively. Byte-wise comparison of the keys gives the order.
fn tree_key(p: &str) -> String {
    let mut key = String::with_capacity(p.len() + 8);
    let mut segs = p.split('/').peekable();
    while let Some(seg) = segs.next() {
        key.push(if segs.peek().is_some() { '\u{1}' } else { '\u{2}' });
        key.extend(seg.chars().flat_map(char::to_lowercase));
        key.push('\u{0}');
    }
    key
}

pub(crate) fn load_theme(config: &VaultConfig) -> Theme {
    let mut theme = Theme::new();
    theme.add_sheet("base", BASE_CSS);
    if let Some(name) = config.css_theme() {
        if let Ok(css) = std::fs::read_to_string(config.dir.join("themes").join(name).join("theme.css")) {
            theme.add_sheet(&format!("theme:{name}"), &css);
        }
    }
    for snippet in config.enabled_snippets() {
        if let Ok(css) = std::fs::read_to_string(config.dir.join("snippets").join(format!("{snippet}.css"))) {
            theme.add_sheet(&format!("snippet:{snippet}"), &css);
        }
    }
    let a = &config.appearance;
    let mut vars = String::new();
    if let Some(n) = a.get("baseFontSize").and_then(serde_json::Value::as_f64) {
        vars += &format!("--font-text-size: {n}px; ");
    }
    if let Some(f) = a.str("textFontFamily").filter(|f| !f.trim().is_empty()) {
        vars += &format!("--font-text: {f}; ");
    }
    if let Some(n) = a.get("lineWidth").and_then(serde_json::Value::as_f64) {
        vars += &format!("--file-line-width: {n}px; ");
    }
    if let Some(c) = a.str("accentColor").and_then(theme_engine::parse_color) {
        let (h, s, l) = hsl(c);
        vars += &format!("--accent-h: {h:.0}; --accent-s: {s:.0}%; --accent-l: {l:.0}%; ");
    }
    if !vars.is_empty() {
        theme.add_sheet("settings", &format!("body {{ {vars}}}"));
    }
    // Last, so it wins over the theme and snippets while it applies.
    theme.add_sheet("contrast", CONTRAST_CSS);
    theme
}

/// Hue in degrees, saturation and lightness in percent.
fn hsl(c: theme_engine::Color) -> (f32, f32, f32) {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l * 100.0);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s * 100.0, l * 100.0)
}

pub(crate) fn default_keymap(config: &VaultConfig) -> Vec<(Vec<Key>, String)> {
    let mut map: Vec<(Vec<Key>, String)> = Vec::new();
    let overrides = config.hotkeys.value.as_object();
    for (id, _, default) in COMMANDS {
        match overrides.and_then(|o| o.get(*id)).and_then(|v| v.as_array()) {
            // `hotkeys.json` format: [{"modifiers": ["Mod", "Shift"], "key": "P"}]; an empty array unbinds.
            Some(list) => {
                for h in list {
                    let mods: Vec<&str> = h
                        .get("modifiers")
                        .and_then(|m| m.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                        .unwrap_or_default();
                    let Some(key) = h.get("key").and_then(|k| k.as_str()) else { continue };
                    let has = |n: &str| mods.iter().any(|m| m.eq_ignore_ascii_case(n));
                    map.push((
                        vec![Key {
                            key: normalize_key(key),
                            ctrl: has("Mod") || has("Ctrl") || has("Meta"),
                            shift: has("Shift"),
                            alt: has("Alt"),
                        }],
                        id.to_string(),
                    ));
                }
            }
            None => {
                if let Some(seq) = Key::parse_sequence(default).filter(|s| !s.is_empty()) {
                    map.push((seq, id.to_string()));
                }
            }
        }
    }
    map
}

impl App {
    pub fn open(root: &Path) -> io::Result<App> {
        let (store, _) = VaultStore::open(root)?;
        App::with_store(root, store)
    }

    /// An app on a vault whose index is already built.
    pub fn with_store(root: &Path, mut store: VaultStore) -> io::Result<App> {
        let config = VaultConfig::load(root)?;
        store.index.link_format = config.link_format();
        store.trash_mode = config.trash_option();
        let _ = store.save_cache();
        let theme = load_theme(&config);
        let keymap = default_keymap(&config);
        let dark = config.color_scheme().unwrap_or(true);
        Ok(App {
            vim: config.app.bool("vimMode", false),
            settings: None,
            prop_edit: None,
            media: Default::default(),
            graph_view: None,
            show_graph: false,
            canvas_view: None,
            show_canvas: false,
            base_view: None,
            show_base: false,
            recovery: Recovery::new(root),
            store,
            theme,
            dark,
            system_dark: None,
            reduced_motion: false,
            high_contrast: false,
            preedit: String::new(),
            preedit_cursor: None,
            keymap,
            config,
            tabs: Vec::new(),
            active: 0,
            panes: vec![Default::default()],
            layout: crate::split::Layout::Leaf(0),
            pane: 0,
            pane_drag: None,
            popouts: Vec::new(),
            next_popout: 0,
            last_main: 0,
            followed: None,
            mirrored: None,
            left_open: true,
            right_open: false,
            panel: Panel::Files,
            focus: Focus::Editor,
            collapsed: HashSet::new(),
            prompt: None,
            search_query: String::new(),
            search_results: Vec::new(),
            search_error: None,
            corpus: None,
            graph: None,
            tags: None,
            mentions: None,
            show_unlinked: false,
            linked: None,
            right_panel: RightPanel::Backlinks,
            backlinks_collapsed: false,
            backlinks_desc: false,
            backlinks_more: false,
            backlinks_filter: None,
            backlinks_editing: false,
            tree: None,
            defer_graph: false,
            pending: Vec::new(),
            history: Vec::new(),
            history_pos: 0,
            notice: None,
            perf: false,
            last_frame_us: 0,
            screenshot: false,
            quit: false,
            scrolls: HashMap::new(),
            fuzzy: Fuzzy::new(),
            plugins: Default::default(),
            hover_action: None,
            explorer_drag: None,
            suggest: None,
            hover_preview: None,
            suggest_dismissed: None,
            anims: Default::default(),
            seen: None,
            notice_since: None,
            update: crate::update::State::Idle,
            closed_tabs: Vec::new(),
            switch_vault: false,
            menu: None,
            window_w: 1280.0,
            system_clipboard: false,
            clip: String::new(),
        })
    }

    /// The link graph as of the last `refresh_graph` call.
    pub fn cached_graph(&self) -> Option<&vault_core::LinkGraph> {
        self.graph.as_deref()
    }
    /// Bring derived data up to date before a frame: link graph, explorer rows, and the
    /// metadata of the active note. Each is recomputed only when its input changed.
    pub fn refresh_derived(&mut self) {
        self.sync_panes();
        self.base_refresh();
        if !self.defer_graph {
            self.graph = Some(self.store.index.graph());
            self.store.release_cache_pages();
        }
        self.refresh_graph_view();
        if self.left_open && self.panel == Panel::Tags {
            self.tag_counts();
        }
        if self.right_open && self.right_panel == RightPanel::Backlinks && self.tab().is_some() {
            self.refresh_linked();
            if self.show_unlinked {
                self.refresh_mentions();
            }
        }
        if self.tree.is_none() {
            self.tree = Some(self.build_tree());
        }
        // Outline and word count need a pass over the whole note, so they are refreshed only
        // once typing has paused; a keystroke never pays for them.
        let now = fsio::now_ms();
        if let Some(t) = self.tabs.get_mut(self.active).filter(|_| !self.show_graph) {
            t.refresh_folds();
            let v = t.editor.buf.version;
            let stale = t.meta.as_ref().is_none_or(|(mv, _)| *mv != v);
            if stale && (t.meta.is_none() || now.saturating_sub(t.last_edit_ms) >= META_DEBOUNCE_MS) {
                t.meta = Some((v, vault_core::meta::extract(&t.editor.buf.text())));
            }
        }
    }

    /// Milliseconds until the outline/word count of the active note should be refreshed.
    pub fn derived_due_in(&self, now_ms: u64) -> Option<u64> {
        let t = self.tab()?;
        let stale = t.meta.as_ref().is_none_or(|(mv, _)| *mv != t.editor.buf.version);
        stale.then(|| (t.last_edit_ms + META_DEBOUNCE_MS).saturating_sub(now_ms))
    }

    pub fn tree_rows(&self) -> &[TreeRow] {
        self.tree.as_deref().unwrap_or(&[])
    }

    /// The explorer order, as `fileSortOrder` in `app.json` names it.
    pub fn sort_order(&self) -> &str {
        match self.config.app.str("fileSortOrder") {
            Some(
                o @ ("alphabeticalReverse"
                | "byModifiedTime"
                | "byModifiedTimeReverse"
                | "byCreatedTime"
                | "byCreatedTimeReverse"),
            ) => o,
            _ => "alphabetical",
        }
    }

    fn build_tree(&self) -> Vec<TreeRow> {
        let order = self.sort_order();
        let created = |p: &str| -> i64 {
            std::fs::metadata(self.store.abs(p))
                .and_then(|m| m.created())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as i64)
        };
        // Within a directory folders come first, in name order; the setting orders the files.
        // The key's directory part keeps every directory's rows together.
        let mut keyed: Vec<(String, i64, String, &str, FileKind)> = self
            .store
            .index
            .iter()
            .map(|(_, f)| {
                let key = tree_key(&f.path);
                let name_at = key.rfind('\u{2}').map_or(0, |i| i + 1);
                let time = match order {
                    "byModifiedTime" => -f.mtime,
                    "byModifiedTimeReverse" => f.mtime,
                    "byCreatedTime" => -created(&f.path),
                    "byCreatedTimeReverse" => created(&f.path),
                    _ => 0,
                };
                (key[..name_at].to_string(), time, key[name_at..].to_string(), &*f.path, f.kind)
            })
            .collect();
        if order == "alphabeticalReverse" {
            keyed.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| b.2.cmp(&a.2)));
        } else {
            keyed.sort_unstable_by(|a, b| (&a.0, a.1, &a.2).cmp(&(&b.0, b.1, &b.2)));
        }
        let paths: Vec<(&str, FileKind)> = keyed.into_iter().map(|k| (k.3, k.4)).collect();
        let mut rows = Vec::with_capacity(paths.len() + 64);
        let mut open_dirs: Vec<String> = Vec::new();
        for (p, kind) in paths {
            let dir = path::parent(p);
            let segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
            let mut acc = String::new();
            let mut hidden = false;
            for (depth, seg) in segs.iter().enumerate() {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                if hidden {
                    break;
                }
                let collapsed = self.collapsed.contains(&acc);
                if open_dirs.get(depth).map(String::as_str) != Some(acc.as_str()) {
                    open_dirs.truncate(depth);
                    open_dirs.push(acc.clone());
                    rows.push(TreeRow {
                        depth,
                        label: seg.to_string(),
                        path: acc.clone(),
                        folder: true,
                        collapsed,
                    });
                }
                hidden = collapsed;
            }
            if hidden {
                continue;
            }
            open_dirs.truncate(segs.len());
            let label = if kind == FileKind::Markdown { path::stem(p) } else { path::file_name(p) };
            rows.push(TreeRow {
                depth: segs.len(),
                label: label.to_string(),
                path: p.to_string(),
                folder: false,
                collapsed: false,
            });
        }
        rows
    }

    /// The note being edited; `None` while the graph has the main pane.
    pub fn tab(&self) -> Option<&Tab> {
        self.tabs
            .get(self.active)
            .filter(|_| !self.show_graph && !self.canvas_active() && !self.base_active())
    }
    pub fn tab_mut(&mut self) -> Option<&mut Tab> {
        let shown = !self.show_graph && !self.canvas_active() && !self.base_active();
        self.tabs.get_mut(self.active).filter(|_| shown)
    }

    pub fn hotkey_for(&self, command: &str) -> String {
        self.keymap
            .iter()
            .find(|(_, c)| c == command)
            .map(|(k, _)| k.iter().map(Key::to_string).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    }

    // ---- files ----

    pub fn open_note(&mut self, rel: &str) {
        self.open_note_inner(rel, true);
    }

    pub(crate) fn open_note_inner(&mut self, rel: &str, record: bool) {
        let mut rel = path::normalize(rel);
        // Accept a note name or partial path the way a link would be resolved.
        if self.store.index.id(&rel).is_none() {
            if let Some(f) = self.store.index.resolve(&rel, "").and_then(|id| self.store.index.entry(id)) {
                rel = f.path.to_string();
            }
        }
        if path::kind_of(&rel) == FileKind::Canvas {
            return self.open_canvas(&rel);
        }
        if path::kind_of(&rel) == FileKind::Base {
            return self.open_base(&rel);
        }
        if let Some(i) = self.tabs.iter().position(|t| t.path.eq_ignore_ascii_case(&rel)) {
            self.active = i;
        } else {
            if path::kind_of(&rel) != FileKind::Markdown {
                self.notice = Some(format!("{rel}: only Markdown notes can be opened in this build"));
                return;
            }
            match self.store.read(&rel) {
                Ok((text, stamp)) => {
                    self.tabs.push(Tab::new(rel.clone(), &text, stamp, self.vim));
                    self.active = self.tabs.len() - 1;
                    self.init_new_tab();
                }
                Err(e) => {
                    self.notice = Some(format!("Cannot open {rel}: {e}"));
                    return;
                }
            }
        }
        self.focus = Focus::Editor;
        self.show_graph = false;
        self.show_canvas = false;
        self.show_base = false;
        self.plugins.push(ext_api::Event::FileOpen { path: rel.clone() });
        if record {
            self.history.truncate(self.history_pos + usize::from(!self.history.is_empty()));
            if self.history.last() != Some(&rel) {
                self.history.push(rel);
            }
            self.history_pos = self.history.len().saturating_sub(1);
        }
    }

    /// Save the active tab. External edits are never overwritten: a conflict is reported.
    pub fn save(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        if !tab.editor.buf.is_modified() {
            return true;
        }
        let text = tab.editor.buf.text();
        match self.store.write(&tab.path, &text, Some(tab.stamp)) {
            Ok(stamp) => {
                tab.stamp = stamp;
                tab.editor.buf.mark_saved();
                self.plugins.push(ext_api::Event::Modify { path: tab.path.clone() });
                let _ = self.recovery.snapshot(&tab.path, &text, fsio::now_ms());
                if let Some(c) = &mut self.corpus {
                    if let Some(id) = self.store.index.id(&tab.path) {
                        c.update(id, Some(&text));
                    }
                }
                let path = tab.path.clone();
                self.saved_elsewhere(&path, &text, stamp);
                if let Some(b) = &mut self.base_view {
                    b.items = None;
                }
                true
            }
            Err(WriteError::Conflict { .. }) => {
                self.notice = Some(format!(
                    "{} was changed outside Rustsidian. Not saved; your text is kept in this tab.",
                    tab.path
                ));
                false
            }
            Err(WriteError::Io(e)) => {
                self.notice = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// Close one tab, saving it first. The tab stays open when the save fails.
    pub fn close_tab(&mut self, i: usize) {
        if self.tabs.get(i).is_none_or(|t| t.pinned) {
            return;
        }
        let prev = self.active;
        self.active = i;
        if !self.save() {
            return;
        }
        let tab = self.tabs.remove(i);
        self.closed_tabs.push(tab.path);
        self.active = if prev > i { prev - 1 } else { prev.min(self.tabs.len().saturating_sub(1)) };
    }

    /// Save every modified tab (called when the window loses focus, on a timer, and on quit).
    pub fn save_all(&mut self) {
        self.canvas_save();
        for p in 0..self.panes.len() {
            self.in_pane(p, |a| {
                let active = a.active;
                for i in 0..a.tabs.len() {
                    a.active = i;
                    a.save();
                }
                a.active = active;
            });
        }
        self.save_session();
        let _ = self.store.save_cache();
    }

    /// Milliseconds until the autosave is due, if any tab has unsaved edits.
    pub fn autosave_due_in(&self, now_ms: u64) -> Option<u64> {
        self.all_tabs()
            .filter(|t| t.editor.buf.is_modified())
            .map(|t| (t.last_edit_ms + 2000).saturating_sub(now_ms))
            .min()
    }

    /// React to files changed on disk by another program.
    pub fn files_changed(&mut self, paths: &[String]) {
        self.tree = None;
        self.canvas_files_changed(paths);
        self.base_files_changed(paths);
        if let Some(g) = &mut self.graph_view {
            g.dirty = true;
        }
        for p in paths {
            if p.starts_with(".obsidian/") {
                if p.contains("snippets")
                    || p.contains("themes")
                    || p.ends_with("appearance.json")
                    || p.ends_with("hotkeys.json")
                    || p.ends_with("app.json")
                {
                    self.run("app:reload-theme");
                }
                continue;
            }
            if self.store.refresh(p).is_none() {
                continue;
            }
            if let Some(c) = &mut self.corpus {
                if let Some(id) = self.store.index.id(p) {
                    c.update(id, std::fs::read_to_string(self.store.abs(p)).ok().as_deref());
                }
            }
            let open = |t: &&mut Tab| t.path.eq_ignore_ascii_case(p);
            if self.all_tabs_mut().filter(open).any(|t| t.editor.buf.is_modified()) {
                self.notice = Some(format!("{p} changed on disk while it has unsaved edits here."));
            } else if let Ok((text, stamp)) = self.store.read(p) {
                for tab in self.all_tabs_mut().filter(open) {
                    let head = tab.editor.buf.sels[0].head;
                    tab.editor = Editor::new(&text);
                    tab.editor.buf.set_sels(vec![Sel::cursor(head)]);
                    tab.stamp = stamp;
                }
            }
        }
    }

    fn unique_untitled(&self) -> String {
        (0..)
            .map(|n| if n == 0 { "Untitled.md".to_string() } else { format!("Untitled {n}.md") })
            .find(|p| self.store.index.id(p).is_none())
            .unwrap_or_default()
    }

    // ---- prompt (palette, switcher) ----

    pub(crate) fn refresh_prompt(&mut self) {
        let Some(kind) = self.prompt.as_ref().map(|p| p.kind) else { return };
        let query = self.prompt.as_ref().map(|p| p.query.clone()).unwrap_or_default();
        let items: Vec<PromptItem> = match kind {
            PromptKind::Commands => {
                let extra = self.plugin_palette();
                let names: Vec<&str> =
                    COMMANDS.iter().map(|c| c.1).chain(extra.iter().map(|c| c.1.as_str())).collect();
                self.fuzzy
                    .top(&query, &names, 12)
                    .iter()
                    .map(|h| {
                        let (id, name) = match COMMANDS.get(h.index) {
                            Some((id, name, _)) => (*id, name.to_string()),
                            None => {
                                let c = &extra[h.index - COMMANDS.len()];
                                (c.0.as_str(), c.1.clone())
                            }
                        };
                        PromptItem { label: name, aux: self.hotkey_for(id), action: id.to_string() }
                    })
                    .filter(|item| self.disabled_by(&item.action).is_none())
                    .collect()
            }
            PromptKind::Switcher => {
                let mut paths: Vec<&str> = self.store.index.notes().map(|(_, f)| &*f.path).collect();
                paths.extend(
                    self.store
                        .index
                        .iter()
                        .filter(|(_, f)| matches!(f.kind, FileKind::Canvas | FileKind::Base))
                        .map(|(_, f)| &*f.path),
                );
                if query.is_empty() {
                    // Recently opened first.
                    paths.sort_by_key(|p| std::cmp::Reverse(self.history.iter().rposition(|h| h == p)));
                }
                let mut items: Vec<PromptItem> = self
                    .fuzzy
                    .top(&query, &paths, 12)
                    .iter()
                    .map(|h| {
                        let p = paths[h.index];
                        PromptItem {
                            label: path::stem(p).to_string(),
                            aux: path::parent(p).to_string(),
                            action: format!("open:{p}"),
                        }
                    })
                    .collect();
                if !query.trim().is_empty()
                    && !paths.iter().any(|p| path::strip_md(p).eq_ignore_ascii_case(query.trim()))
                {
                    items.push(PromptItem {
                        label: query.trim().to_string(),
                        aux: "Enter to create".into(),
                        action: format!("create:{}", query.trim()),
                    });
                }
                items
            }
            PromptKind::Template => self.template_items(&query),
            PromptKind::Rename | PromptKind::NewFolder => {
                let target = self.prompt.as_ref().map(|p| p.target.clone()).unwrap_or_default();
                let name = query.trim();
                if name.is_empty() {
                    Vec::new()
                } else if kind == PromptKind::Rename {
                    vec![PromptItem {
                        label: format!("Rename to \u{201c}{name}\u{201d}"),
                        aux: "Enter".into(),
                        action: format!("rename:{target}|{name}"),
                    }]
                } else {
                    vec![PromptItem {
                        label: format!("Create folder \u{201c}{name}\u{201d}"),
                        aux: if target.is_empty() {
                            "in the vault root".into()
                        } else {
                            format!("in {target}")
                        },
                        action: format!("mkdir:{target}|{name}"),
                    }]
                }
            }
        };
        if let Some(p) = &mut self.prompt {
            p.items = items;
            p.selected = p.selected.min(p.items.len().saturating_sub(1));
        }
    }

    /// Record the OS color scheme; it applies when the vault's scheme is "Adapt to system".
    pub fn set_system_dark(&mut self, dark: Option<bool>) {
        self.system_dark = dark;
        if self.config.color_scheme().is_none() {
            self.dark = dark.unwrap_or(true);
        }
    }

    pub fn env(&self) -> theme_engine::Env {
        theme_engine::Env {
            dark: self.dark,
            reduced_motion: self.reduced_motion,
            high_contrast: self.high_contrast,
        }
    }

    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
        self.anims.enabled = !on;
    }

    /// Indices of the best fuzzy matches of `query` among `names`.
    pub(crate) fn fuzzy_top(&mut self, query: &str, names: &[&str], limit: usize) -> Vec<usize> {
        self.fuzzy.top(query, names, limit).iter().map(|h| h.index).collect()
    }

    pub(crate) fn open_prompt(&mut self, kind: PromptKind) {
        self.prompt = Some(Prompt {
            kind,
            target: String::new(),
            query: String::new(),
            selected: 0,
            items: Vec::new(),
        });
        self.focus = Focus::Prompt;
        self.refresh_prompt();
    }

    /// Ask for a name, for a rename (`target` is the file or folder) or a new folder (`target`
    /// is the parent folder).
    pub(crate) fn open_name_prompt(&mut self, kind: PromptKind, target: &str, initial: &str) {
        self.open_prompt(kind);
        if let Some(p) = &mut self.prompt {
            p.target = target.to_string();
            p.query = initial.to_string();
        }
        self.refresh_prompt();
    }

    fn close_prompt(&mut self) {
        self.prompt = None;
        self.focus = Focus::Editor;
    }

    // ---- search ----

    pub fn run_search(&mut self) {
        self.search_results.clear();
        self.search_error = None;
        if self.search_query.trim().is_empty() {
            return;
        }
        let query = match Query::parse(&self.search_query) {
            Ok(q) => q,
            Err(e) => {
                self.search_error = Some(e.to_string());
                return;
            }
        };
        // The corpus is built on first use, so vaults that never search never pay for it.
        let root = self.store.root.clone();
        let corpus = self.corpus.get_or_insert_with(|| Corpus::build(&self.store.index, &root));
        let hits =
            corpus.search(&self.store.index, &root, &query, Sort::Relevance, RankWeights::default(), 50);
        self.store.release_cache_pages();
        for h in hits {
            let Some(f) = self.store.index.entry(h.file) else { continue };
            let text = std::fs::read_to_string(root.join(&*f.path)).unwrap_or_default();
            let snippets = h
                .matches
                .iter()
                .take(3)
                .map(|m| search_core::snippet(&text, *m, 40).0.trim().to_string())
                .collect();
            self.search_results.push(SearchResult { path: f.path.to_string(), snippets });
        }
    }

    // ---- commands ----

    /// Run a command by id. Some take an argument after a colon (`open:path`).
    pub fn run(&mut self, command: &str) {
        let now = fsio::now_ms();
        if let Some(name) = self.disabled_by(command) {
            self.notice = Some(format!("{name} is turned off in Settings → Core plugins"));
            return;
        }
        if matches!(command.split(':').next(), Some("create" | "file-explorer" | "daily-notes" | "app")) {
            // These can add or remove files; the explorer rows are rebuilt on the next frame.
            self.tree = None;
        }
        if let Some(i) = command.strip_prefix("suggest:").and_then(|i| i.parse().ok()) {
            return self.accept_suggest(i);
        }
        if let Some(rest) = command.strip_prefix("spell:replace:") {
            let mut it = rest.splitn(3, ':');
            let (Some(a), Some(b), Some(word)) = (it.next(), it.next(), it.next()) else { return };
            let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else { return };
            let Some(t) = self.tabs.get_mut(self.active) else { return };
            if a > b || b > t.editor.buf.len() {
                return;
            }
            let now = fsio::now_ms();
            t.editor.buf.set_sels(vec![Sel::new(a, b)]);
            t.editor.replace_selections(word, EditKind::Other, now);
            t.last_edit_ms = now;
            return;
        }
        if let Some(word) = command.strip_prefix("spell:add:") {
            if let Err(e) = crate::spell::add(word) {
                self.notice = Some(format!("Cannot add \"{word}\" to the dictionary: {e}"));
            }
            return;
        }
        if let Some(p) = command.strip_prefix("open:") {
            self.close_prompt();
            return self.open_note(p);
        }
        if let Some(name) = command.strip_prefix("create:") {
            self.close_prompt();
            let rel = format!("{}.md", path::sanitize_file_name(name, path::SanitizePolicy::Strip));
            if self.store.index.id(&rel).is_none() {
                if let Err(e) = self.store.create(&rel, "") {
                    self.notice = Some(format!("Cannot create {rel}: {e}"));
                    return;
                }
                self.plugins.push(ext_api::Event::Create { path: rel.clone() });
            }
            return self.open_note(&rel);
        }
        if let Some(i) = command.strip_prefix("tab:").and_then(|i| i.parse::<usize>().ok()) {
            if i < self.tabs.len() {
                self.active = i;
                self.show_graph = false;
                self.show_canvas = false;
                self.show_base = false;
                self.focus = Focus::Editor;
            }
            return;
        }
        if let Some(j) = command.strip_prefix("link-pane:").and_then(|i| i.parse::<usize>().ok()) {
            return self.link_pane(j);
        }
        if let Some(i) = command.strip_prefix("toggle-pin:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(t) = self.tabs.get_mut(i) {
                t.pinned ^= true;
            }
            return;
        }
        if let Some(i) = command.strip_prefix("close-tab:").and_then(|i| i.parse::<usize>().ok()) {
            return self.close_tab(i);
        }
        if let Some(folder) = command.strip_prefix("toggle-folder:") {
            if !self.collapsed.remove(folder) {
                self.collapsed.insert(folder.to_string());
            }
            self.tree = None;
            return;
        }
        if self.run_settings(command) || self.run_properties(command) {
            return;
        }
        if self.settings.is_some() {
            self.settings = None;
        }
        if command == "focus:search" {
            self.focus = Focus::Search;
            return;
        }
        if let Some(op) = command.strip_prefix("search:insert:") {
            self.search_query.push_str(op);
            self.focus = Focus::Search;
            return;
        }
        // Pasting into an open prompt types into its query rather than closing it.
        if command == "editor:paste" && self.prompt.is_some() {
            self.run_file_action(command);
            return;
        }
        if self.prompt.is_some() && command != "command-palette:open" && command != "switcher:open" {
            self.close_prompt();
        }
        if command == "prompt:dismiss" {
            return;
        }
        if let Some(rel) = command.strip_prefix("backlinks:link:") {
            self.link_mentions(rel);
            return;
        }
        if self.run_plugin_command(command) || self.run_panes(command) || self.run_templates(command) {
            return;
        }
        if self.run_file_action(command) {
            return;
        }
        let wrap = |app: &mut App, marker: &str| {
            if let Some(t) = app.tab_mut() {
                t.editor.toggle_wrap(marker, now);
                t.last_edit_ms = now;
            }
        };
        match command {
            "command-palette:open" => self.open_prompt(PromptKind::Commands),
            "switcher:open" => self.open_prompt(PromptKind::Switcher),
            "global-search:open" => {
                self.left_open = true;
                self.panel = Panel::Search;
                self.focus = Focus::Search;
            }
            "file-explorer:open" => {
                self.left_open = true;
                self.panel = Panel::Files;
            }
            "file-explorer:new-file" => {
                let rel = path::join(&self.new_note_dir(), &self.unique_untitled());
                match self.store.create(&rel, "") {
                    Ok(_) => self.open_note(&rel),
                    Err(e) => self.notice = Some(format!("Cannot create note: {e}")),
                }
            }
            "bases:new" => {
                let name = (0..)
                    .map(|n| if n == 0 { "Untitled.base".to_string() } else { format!("Untitled {n}.base") })
                    .map(|f| path::join(&self.new_note_dir(), &f))
                    .find(|p| self.store.index.id(p).is_none() && !self.store.root.join(p).exists())
                    .unwrap_or_default();
                self.tree = None;
                match self.store.create(&name, "views:\n  - type: table\n    name: Table\n    order:\n      - file.name\n      - file.mtime\n") {
                    Ok(_) => self.open_base(&name),
                    Err(e) => self.notice = Some(format!("Cannot create base: {e}")),
                }
            }
            "canvas:new" => {
                let name =
                    (0..)
                        .map(|n| {
                            if n == 0 {
                                "Untitled.canvas".to_string()
                            } else {
                                format!("Untitled {n}.canvas")
                            }
                        })
                        .map(|f| path::join(&self.new_note_dir(), &f))
                        .find(|p| self.store.index.id(p).is_none() && !self.store.root.join(p).exists())
                        .unwrap_or_default();
                self.tree = None;
                match self.store.create(&name, "{\"nodes\":[],\"edges\":[]}\n") {
                    Ok(_) => self.open_canvas(&name),
                    Err(e) => self.notice = Some(format!("Cannot create canvas: {e}")),
                }
            }
            "editor:save-file" => {
                if self.save() {
                    self.notice = None;
                }
            }
            "workspace:close" | "graph:close" if self.show_graph || command == "graph:close" => {
                self.graph_view = None;
                self.show_graph = false;
            }
            "workspace:close" if self.base_active() => self.run("base:close"),
            "workspace:close" | "canvas:close" if self.canvas_active() || command == "canvas:close" => {
                self.canvas_save();
                self.canvas_view = None;
                self.show_canvas = false;
                self.show_base = false;
            }
            "workspace:close" => self.close_tab(self.active),
            "workspace:split-vertical" => self.split(true),
            "workspace:split-horizontal" => self.split(false),
            "workspace:move-to-new-window" => self.move_to_window(),
            "workspace:unlink-pane" => self.unlink_pane(),
            "workspace:close-others" => {
                for i in (0..self.tabs.len()).rev().filter(|&i| i != self.active).collect::<Vec<_>>() {
                    self.close_tab(i);
                }
            }
            "workspace:close-all" => {
                for i in (0..self.tabs.len()).rev() {
                    self.close_tab(i);
                }
            }
            "workspace:undo-close-pane" => {
                while let Some(p) = self.closed_tabs.pop() {
                    if self.store.index.id(&p).is_some() {
                        self.open_note(&p);
                        break;
                    }
                }
            }
            "file-explorer:collapse-all" => {
                let folders: HashSet<String> = self
                    .store
                    .index
                    .iter()
                    .flat_map(|(_, f)| {
                        let dir = path::parent(&f.path);
                        dir.match_indices('/')
                            .map(|(i, _)| dir[..i].to_string())
                            .chain((!dir.is_empty()).then(|| dir.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .collect();
                if folders.iter().all(|f| self.collapsed.contains(f)) {
                    self.collapsed.clear();
                } else {
                    self.collapsed = folders;
                }
            }
            "app:check-for-updates" => {
                self.update.start_check(true);
                self.notice = Some("Checking for updates…".into());
            }
            "app:install-update" => match self.update.start_install() {
                Ok(()) => self.notice = Some("Downloading the update…".into()),
                Err(e) => self.notice = Some(format!("Cannot update: {e}")),
            },
            "app:open-vault" => {
                self.save_all();
                self.switch_vault = true;
            }
            "workspace:next-tab" | "workspace:previous-tab" if self.show_graph => self.show_graph = false,
            "workspace:next-tab" | "workspace:previous-tab" if self.base_active() => self.show_base = false,
            "workspace:next-tab" | "workspace:previous-tab" if self.canvas_active() => {
                self.show_canvas = false
            }
            "workspace:next-tab" if !self.tabs.is_empty() => {
                self.active = (self.active + 1) % self.tabs.len()
            }
            "workspace:previous-tab" if !self.tabs.is_empty() => {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len()
            }
            "app:go-back" if self.history_pos > 0 => {
                self.history_pos -= 1;
                let p = self.history[self.history_pos].clone();
                self.open_note_inner(&p, false);
            }
            "app:go-forward" if self.history_pos + 1 < self.history.len() => {
                self.history_pos += 1;
                let p = self.history[self.history_pos].clone();
                self.open_note_inner(&p, false);
            }
            "app:toggle-left-sidebar" => self.left_open = !self.left_open,
            "app:toggle-right-sidebar" => self.right_open = !self.right_open,
            c if c.starts_with("file-explorer:sort:") => {
                self.config.app.set("fileSortOrder", c["file-explorer:sort:".len()..].into());
                if let Err(e) = self.config.app.save() {
                    self.notice = Some(format!("Cannot save the sort order: {e}"));
                }
                self.tree = None;
            }
            "backlinks:toggle-unlinked" => self.show_unlinked = !self.show_unlinked,
            "backlinks:toggle-collapse" => self.backlinks_collapsed ^= true,
            "backlinks:toggle-sort" => self.backlinks_desc ^= true,
            "backlinks:toggle-context" => {
                self.backlinks_more ^= true;
                self.linked = None;
            }
            "backlinks:toggle-filter" => {
                self.backlinks_filter = self.backlinks_filter.is_none().then(String::new);
                self.backlinks_editing = self.backlinks_filter.is_some();
            }
            "backlinks:edit-filter" => self.backlinks_editing = self.backlinks_filter.is_some(),
            "backlinks:open" | "outgoing-links:open" | "outline:open" => {
                self.right_open = true;
                self.right_panel = match command {
                    "backlinks:open" => RightPanel::Backlinks,
                    "outgoing-links:open" => RightPanel::Outgoing,
                    _ => RightPanel::Outline,
                };
            }
            "markdown:toggle-preview" => {
                if let Some(t) = self.tab_mut() {
                    t.mode =
                        if t.mode == ViewMode::Reading { ViewMode::LivePreview } else { ViewMode::Reading };
                }
            }
            "editor:toggle-source" => {
                if let Some(t) = self.tab_mut() {
                    t.mode =
                        if t.mode == ViewMode::Source { ViewMode::LivePreview } else { ViewMode::Source };
                }
            }
            "editor:toggle-bold" => wrap(self, "**"),
            "editor:toggle-italics" => wrap(self, "*"),
            "editor:toggle-highlight" => wrap(self, "=="),
            "editor:toggle-code" => wrap(self, "`"),
            "editor:toggle-strikethrough" => wrap(self, "~~"),
            "editor:follow-link" => self.follow_link(),
            "daily-notes" => self.open_daily_note(),
            "daily-notes:goto-prev" => self.daily_step(false),
            "daily-notes:goto-next" => self.daily_step(true),
            "zk-prefixer" => self.new_unique_note(),
            "random-note" => {
                let notes: Vec<String> = self.store.index.notes().map(|(_, f)| f.path.to_string()).collect();
                if !notes.is_empty() {
                    let pick = (fsio::fold64(&now.to_le_bytes()) % notes.len() as u64) as usize;
                    self.open_note(&notes[pick]);
                }
            }
            "app:delete-file" => {
                if let Some(p) = self.tab().map(|t| t.path.clone()) {
                    match self.store.delete(&p) {
                        Ok(_) => {
                            self.plugins.push(ext_api::Event::Delete { path: p.clone() });
                            self.retain_tabs(|t| !t.path.eq_ignore_ascii_case(&p));
                            self.notice = Some(format!("Moved {p} to trash"));
                        }
                        Err(e) => self.notice = Some(format!("Cannot delete {p}: {e}")),
                    }
                }
            }
            "theme:toggle-light-dark" => self.dark = !self.dark,
            "app:reload-theme" => {
                if let Ok(c) = VaultConfig::load(&self.store.root) {
                    self.config = c;
                }
                self.theme = load_theme(&self.config);
                self.keymap = default_keymap(&self.config);
                self.store.index.link_format = self.config.link_format();
                self.store.trash_mode = self.config.trash_option();
                self.config_changed();
            }
            "app:toggle-perf-overlay" => self.perf = !self.perf,
            "app:quit" => {
                self.save_all();
                self.quit = true;
            }
            _ => {
                if self.run_graph(command) || self.run_canvas(command) || self.run_base(command) {
                    return;
                }
                if self.run_editing(command, now) {
                    return;
                }
                let Some(t) = self.tab_mut() else { return };
                if t.mode == ViewMode::Reading {
                    return;
                }
                let e = &mut t.editor;
                match command {
                    "editor:toggle-checklist-status" => e.toggle_task(now),
                    "editor:toggle-blockquote" => e.toggle_blockquote(now),
                    "editor:undo" => {
                        e.undo();
                    }
                    "editor:redo" => {
                        e.redo();
                    }
                    "editor:select-all" => e.select_all(),
                    "editor:swap-line-up" => e.move_lines(true, now),
                    "editor:swap-line-down" => e.move_lines(false, now),
                    "editor:duplicate-line" => e.duplicate_lines(now),
                    "editor:add-cursor-above" => e.add_cursor_vertical(true),
                    "editor:add-cursor-below" => e.add_cursor_vertical(false),
                    "editor:select-next-occurrence" => e.select_next_occurrence(),
                    c => match c.strip_prefix("editor:set-heading-").and_then(|n| n.parse().ok()) {
                        Some(level) => e.set_heading(level, now),
                        None => return,
                    },
                }
                t.last_edit_ms = now;
            }
        }
    }

    /// The target of the link the cursor is in, if any.
    pub(crate) fn link_under_cursor(&self) -> Option<String> {
        let t = self.tab()?;
        let head = t.editor.buf.sels[0].head as u32;
        let meta = vault_core::meta::extract(&t.editor.buf.text());
        meta.links.iter().find(|l| l.start <= head && head <= l.end).map(|l| l.target.to_string())
    }

    fn follow_link(&mut self) {
        let Some(target) = self.link_under_cursor() else { return };
        let Some(t) = self.tab() else { return };
        match self.store.index.resolve(&target, &t.path) {
            Some(id) => {
                let p = self.store.index.entry(id).map(|f| f.path.to_string()).unwrap_or_default();
                self.open_note(&p);
            }
            None => {
                self.run(&format!("create:{target}"));
            }
        }
    }

    // ---- input ----

    /// Handle a key press. Returns true if it was consumed.
    pub fn key(&mut self, key: Key) -> bool {
        let handled = self.key_inner(key);
        self.after_editor_input();
        handled
    }

    /// Suggestions follow the text before the cursor, and only while the editor has focus.
    fn after_editor_input(&mut self) {
        if self.focus == Focus::Editor && !self.canvas_active() {
            self.update_suggest();
        } else {
            self.suggest = None;
        }
    }

    fn key_inner(&mut self, key: Key) -> bool {
        self.notice = None;
        self.hover_preview = None;
        if let Some(r) = self.menu_key(&key) {
            return r;
        }
        if let Some(r) = self.modal_key(&key) {
            return r;
        }
        // Only set while the editor has focus; see `after_editor_input`.
        if let Some(r) = self.suggest_key(&key) {
            return r;
        }
        // Key sequences: collect keys while they are a prefix of some binding.
        let mut seq = std::mem::take(&mut self.pending);
        seq.push(key.clone());
        if let Some(cmd) = self.keymap.iter().find(|(k, _)| *k == seq).map(|(_, c)| c.clone()) {
            self.run(&cmd);
            return true;
        }
        if self.keymap.iter().any(|(k, _)| k.len() > seq.len() && k[..seq.len()] == seq[..]) {
            self.pending = seq;
            return true;
        }
        let now = fsio::now_ms();
        match self.focus {
            Focus::Prompt => {
                let Some(p) = &mut self.prompt else { return false };
                match key.key.as_str() {
                    "Escape" => self.close_prompt(),
                    "ArrowDown" => p.selected = (p.selected + 1).min(p.items.len().saturating_sub(1)),
                    "ArrowUp" => p.selected = p.selected.saturating_sub(1),
                    "Backspace" => {
                        p.query.pop();
                        p.selected = 0;
                        self.refresh_prompt();
                    }
                    "Enter" => {
                        if let Some(action) = p.items.get(p.selected).map(|i| i.action.clone()) {
                            self.close_prompt();
                            self.run(&action);
                        }
                    }
                    _ => return false,
                }
                true
            }
            Focus::Search => {
                match key.key.as_str() {
                    "Escape" => self.focus = Focus::Editor,
                    "Backspace" => {
                        self.search_query.pop();
                    }
                    "Enter" => self.run_search(),
                    _ => return false,
                }
                true
            }
            Focus::Editor => {
                if self.canvas_active() {
                    return self.canvas_key(&key);
                }
                let Some(t) = self.tabs.get_mut(self.active).filter(|_| !self.show_graph) else {
                    return false;
                };
                if t.mode == ViewMode::Reading {
                    match key.key.as_str() {
                        "ArrowDown" | "PageDown" => {
                            t.top_block = (t.top_block + if key.key == "PageDown" { 10 } else { 1 })
                                .min(t.editor.buf.doc().blocks.len().saturating_sub(1))
                        }
                        "ArrowUp" | "PageUp" => {
                            t.top_block = t.top_block.saturating_sub(if key.key == "PageUp" { 10 } else { 1 })
                        }
                        _ => return false,
                    }
                    return true;
                }
                let before: Vec<usize> = t.editor.buf.sels.iter().map(|s| s.head).collect();
                if t.vim.is_some() {
                    let vim_key = match key.key.as_str() {
                        "Escape" => Some("<Esc>"),
                        "r" | "R" if key.ctrl && t.vim_commands() => Some("<C-r>"),
                        // Outside insert mode these move instead of editing.
                        "Enter" if t.vim_commands() => Some("j"),
                        "Backspace" if t.vim_commands() => Some("h"),
                        "Delete" if t.vim_commands() => Some("x"),
                        "Tab" if t.vim_commands() => Some(""),
                        _ => None,
                    };
                    if let Some(k) = vim_key {
                        if !k.is_empty() {
                            t.vim_key(k, now);
                            t.leave_folds(&before);
                        }
                        self.scroll_to_cursor();
                        return true;
                    }
                }
                if t.mode == ViewMode::LivePreview && !key.ctrl && !key.alt {
                    let op = match key.key.as_str() {
                        "Tab" => Some(crate::editing::TableOp::NextCell(!key.shift)),
                        "Enter" if !key.shift => Some(crate::editing::TableOp::NextRow),
                        _ => None,
                    };
                    if op.is_some_and(|op| t.table_op(op, now)) {
                        self.scroll_to_cursor();
                        return true;
                    }
                }
                let e = &mut t.editor;
                let word = key.ctrl;
                match key.key.as_str() {
                    "ArrowLeft" => {
                        e.move_cursors(if word { Motion::WordLeft } else { Motion::Left }, key.shift)
                    }
                    "ArrowRight" => {
                        e.move_cursors(if word { Motion::WordRight } else { Motion::Right }, key.shift)
                    }
                    "ArrowUp" => e.move_cursors(Motion::Up, key.shift),
                    "ArrowDown" => e.move_cursors(Motion::Down, key.shift),
                    "Home" => {
                        e.move_cursors(if word { Motion::DocStart } else { Motion::LineStart }, key.shift)
                    }
                    "End" => e.move_cursors(if word { Motion::DocEnd } else { Motion::LineEnd }, key.shift),
                    "PageUp" | "PageDown" => {
                        for _ in 0..20 {
                            e.move_cursors(
                                if key.key == "PageUp" { Motion::Up } else { Motion::Down },
                                key.shift,
                            );
                        }
                    }
                    "Enter" => e.newline(now),
                    "Backspace" if word => e.delete_word_backward(now),
                    "Backspace" => e.delete_backward(now),
                    "Delete" if word => e.delete_word_forward(now),
                    "Delete" => e.delete_forward(now),
                    "Tab" if key.shift => e.outdent(now),
                    "Tab" => e.indent(now),
                    "Escape" => {
                        let head = e.buf.sels[0].head;
                        e.buf.set_sels(vec![Sel::cursor(head)]);
                    }
                    _ => return false,
                }
                if matches!(key.key.as_str(), "Enter" | "Backspace" | "Delete" | "Tab") {
                    t.last_edit_ms = now;
                } else {
                    t.leave_folds(&before);
                }
                self.scroll_to_cursor();
                true
            }
        }
    }

    /// Committed text from the keyboard or an input method.
    pub fn text_input(&mut self, text: &str) {
        self.text_input_inner(text);
        self.after_editor_input();
    }

    fn text_input_inner(&mut self, text: &str) {
        if text.is_empty() || text.chars().any(char::is_control) {
            return;
        }
        self.menu = None;
        if self.modal_text(text) {
            return;
        }
        let now = fsio::now_ms();
        match self.focus {
            Focus::Prompt => {
                if let Some(p) = &mut self.prompt {
                    p.query.push_str(text);
                    p.selected = 0;
                }
                self.refresh_prompt();
            }
            Focus::Search => self.search_query.push_str(text),
            Focus::Editor if self.canvas_active() => self.canvas_text(text),
            Focus::Editor => {
                let shown = !self.show_graph;
                if let Some(t) =
                    self.tabs.get_mut(self.active).filter(|t| shown && t.mode != ViewMode::Reading)
                {
                    if t.vim_commands() {
                        let before: Vec<usize> = t.editor.buf.sels.iter().map(|s| s.head).collect();
                        for c in text.chars() {
                            t.vim_key(c.encode_utf8(&mut [0; 4]), now);
                        }
                        t.leave_folds(&before);
                    } else {
                        t.editor.type_text(text, now);
                        t.last_edit_ms = now;
                    }
                    self.scroll_to_cursor();
                }
            }
        }
    }

    /// Keep the block containing the primary cursor on screen.
    pub fn scroll_to_cursor(&mut self) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let head = t.editor.buf.sels.last().map_or(0, |s| s.head);
        let Some(block) = t.editor.buf.doc().block_index_at(head) else {
            t.top_block = 0;
            return;
        };
        if block < t.top_block {
            t.top_block = block;
        } else if block > t.top_block + 18 {
            t.top_block = block - 18;
        }
    }

    pub fn scroll_editor(&mut self, blocks: isize) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            let max = t.editor.buf.doc().blocks.len().saturating_sub(1) as isize;
            t.top_block = (t.top_block as isize + blocks).clamp(0, max.max(0)) as usize;
        }
    }
}
