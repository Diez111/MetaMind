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
    ("command-palette:open", "Paleta de comandos: Abrir", "Ctrl+P"),
    ("switcher:open", "Selector rápido: Abrir", "Ctrl+O"),
    ("global-search:open", "Buscar: Buscar en todos los archivos", "Ctrl+Shift+F"),
    ("file-explorer:open", "Archivos: Mostrar explorador de archivos", ""),
    ("bookmarks:open", "Marcadores: Mostrar marcadores", ""),
    ("bookmarks:bookmark-current-view", "Marcadores: Añadir o quitar marcador de la nota actual", ""),
    ("bookmarks:bookmark-current-search", "Marcadores: Añadir marcador de la búsqueda actual", ""),
    ("tag-pane:open", "Etiquetas: Mostrar etiquetas", ""),
    ("templates:insert-template", "Plantillas: Insertar plantilla", ""),
    ("templates:insert-current-date", "Plantillas: Insertar fecha actual", ""),
    ("templates:insert-current-time", "Plantillas: Insertar hora actual", ""),
    ("file-explorer:new-file", "Crear nota", "Ctrl+N"),
    ("file-explorer:new-folder", "Crear carpeta", ""),
    ("workspace:edit-file-title", "Renombrar archivo", "F2"),
    ("workspace:copy-path", "Copiar ruta del archivo", ""),
    ("workspace:export-pdf", "Exportar a PDF", ""),
    ("file-explorer:reveal-active-file", "Mostrar archivo actual en la navegación", ""),
    ("file-explorer:collapse-all", "Archivos: Contraer o expandir todas las carpetas", ""),
    ("editor:save-file", "Guardar archivo actual", "Ctrl+S"),
    ("kanban:new", "Crear tablero Kanban", ""),
    ("kanban:add-column", "Kanban: Nueva columna", ""),
    ("kanban:add-card", "Kanban: Nueva tarjeta", ""),
    ("kanban:source", "Kanban: Ver código del tablero", ""),
    ("kanban:filter", "Kanban: Filtrar tarjetas", ""),
    ("kanban:filter-clear", "Kanban: Quitar el filtro del tablero", ""),
    ("kanban:help", "Kanban: Cómo se escribe un tablero", ""),
    ("workspace:close", "Cerrar pestaña actual", "Ctrl+W"),
    ("workspace:close-others", "Cerrar las demás pestañas", ""),
    ("workspace:close-all", "Cerrar todas las pestañas", ""),
    ("workspace:split-vertical", "Dividir a la derecha", ""),
    ("workspace:split-horizontal", "Dividir abajo", ""),
    ("workspace:close-pane", "Cerrar el panel", ""),
    ("workspace:move-to-new-window", "Mover pestaña actual a una ventana nueva", ""),
    ("workspace:toggle-stacked-tabs", "Alternar pestañas apiladas", ""),
    ("workspaces:save", "Espacios de trabajo: Guardar espacio actual", ""),
    ("workspaces:load", "Espacios de trabajo: Cargar espacio", ""),
    ("workspaces:delete", "Espacios de trabajo: Eliminar espacio", ""),
    ("workspace:undo-close-pane", "Reabrir pestaña cerrada", "Ctrl+Shift+T"),
    ("workspace:next-tab", "Ir a la pestaña siguiente", "Ctrl+Tab"),
    ("workspace:previous-tab", "Ir a la pestaña anterior", "Ctrl+Shift+Tab"),
    ("app:go-back", "Ir atrás", "Ctrl+Alt+ArrowLeft"),
    ("app:go-forward", "Ir adelante", "Ctrl+Alt+ArrowRight"),
    ("app:toggle-left-sidebar", "Alternar barra lateral izquierda", "Ctrl+\\"),
    ("app:toggle-right-sidebar", "Alternar barra lateral derecha", "Ctrl+Shift+\\"),
    ("backlinks:open", "Retroenlaces: Mostrar retroenlaces", ""),
    ("outgoing-links:open", "Enlaces salientes: Mostrar enlaces salientes", ""),
    ("outline:open", "Esquema: Mostrar esquema", ""),
    ("ai:open", "Copiloto de IA: Mostrar panel", "Ctrl+Shift+A"),
    ("ai:close", "Copiloto de IA: Cerrar panel", ""),
    ("dashboard:open", "Tablero: Abrir o crear el tablero", ""),
    ("more:open", "Menú: Abrir el menú de la ventana", ""),
    ("more:close", "Menú: Cerrar el menú", ""),
    ("markdown:toggle-preview", "Alternar vista de lectura", "Ctrl+E"),
    ("editor:toggle-source", "Alternar vista previa en vivo / código fuente", ""),
    ("editor:toggle-bold", "Alternar negrita", "Ctrl+B"),
    ("editor:toggle-italics", "Alternar cursiva", "Ctrl+I"),
    ("editor:toggle-highlight", "Alternar resaltado", ""),
    ("editor:toggle-code", "Alternar código", ""),
    ("editor:toggle-strikethrough", "Alternar tachado", ""),
    ("editor:toggle-checklist-status", "Alternar estado de la casilla", "Ctrl+L"),
    ("editor:toggle-blockquote", "Alternar cita en bloque", ""),
    ("editor:set-heading-0", "Quitar encabezado", ""),
    ("editor:set-heading-1", "Establecer como encabezado 1", ""),
    ("editor:set-heading-2", "Establecer como encabezado 2", ""),
    ("editor:set-heading-3", "Establecer como encabezado 3", ""),
    ("editor:cut", "Cortar", "Ctrl+X"),
    ("editor:copy", "Copiar", "Ctrl+C"),
    ("editor:paste", "Pegar", "Ctrl+V"),
    ("editor:insert-wikilink", "Añadir enlace interno", ""),
    ("editor:undo", "Deshacer", "Ctrl+Z"),
    ("editor:redo", "Rehacer", "Ctrl+Shift+Z"),
    ("editor:select-all", "Seleccionar todo", "Ctrl+A"),
    ("editor:swap-line-up", "Mover línea arriba", "Alt+ArrowUp"),
    ("editor:swap-line-down", "Mover línea abajo", "Alt+ArrowDown"),
    ("editor:duplicate-line", "Duplicar línea", ""),
    ("editor:toggle-fold", "Alternar plegado en la línea actual", ""),
    ("editor:fold-all", "Plegar todos los encabezados y listas", ""),
    ("editor:unfold-all", "Desplegar todos los encabezados y listas", ""),
    ("editor:insert-table", "Insertar tabla", ""),
    ("editor:table-row-before", "Tabla: Añadir fila antes", ""),
    ("editor:table-row-after", "Tabla: Añadir fila después", ""),
    ("editor:table-row-up", "Tabla: Mover fila arriba", ""),
    ("editor:table-row-down", "Tabla: Mover fila abajo", ""),
    ("editor:table-row-delete", "Tabla: Eliminar fila", ""),
    ("editor:table-col-before", "Tabla: Añadir columna antes", ""),
    ("editor:table-col-after", "Tabla: Añadir columna después", ""),
    ("editor:table-col-left", "Tabla: Mover columna a la izquierda", ""),
    ("editor:table-col-right", "Tabla: Mover columna a la derecha", ""),
    ("editor:table-col-delete", "Tabla: Eliminar columna", ""),
    ("editor:table-col-align-left", "Tabla: Alinear a la izquierda", ""),
    ("editor:table-col-align-center", "Tabla: Centrar", ""),
    ("editor:table-col-align-right", "Tabla: Alinear a la derecha", ""),
    ("editor:table-sort-ascending", "Tabla: Ordenar por columna (A a Z)", ""),
    ("editor:table-sort-descending", "Tabla: Ordenar por columna (Z a A)", ""),
    ("editor:table-format", "Tabla: Alinear columnas en el código fuente", ""),
    ("editor:toggle-vim", "Alternar atajos de Vim", ""),
    ("editor:add-cursor-above", "Añadir cursor arriba", "Ctrl+Alt+ArrowUp"),
    ("editor:add-cursor-below", "Añadir cursor abajo", "Ctrl+Alt+ArrowDown"),
    ("editor:select-next-occurrence", "Seleccionar siguiente aparición", "Ctrl+D"),
    ("editor:follow-link", "Seguir enlace bajo el cursor", "Alt+Enter"),
    ("canvas:new", "Canvas: Crear lienzo nuevo", ""),
    ("bases:new", "Bases: Crear base nueva", ""),
    ("canvas:add-text", "Canvas: Añadir nodo de texto", ""),
    ("canvas:delete-selected", "Canvas: Eliminar nodo seleccionado", ""),
    ("canvas:edit-selected", "Canvas: Editar nodo de texto seleccionado", ""),
    ("canvas:reset-view", "Canvas: Ajustar lienzo al panel", ""),
    ("canvas:focus", "Canvas: Mostrar lienzo", ""),
    ("exc:focus", "Excalidraw: Mostrar el tablero", ""),
    ("exc:zoom-fit", "Excalidraw: Ajustar el tablero al panel", "Ctrl+0"),
    ("exc:zoom-in", "Excalidraw: Acercar", ""),
    ("exc:zoom-out", "Excalidraw: Alejar", ""),
    ("exc:zoom-reset", "Excalidraw: Volver al 100 %", ""),
    ("exc:fullscreen", "Excalidraw: Pantalla completa", "F11"),
    ("exc:delete-selected", "Excalidraw: Eliminar el elemento seleccionado", ""),
    ("graph:open", "Vista de gráfico: Abrir gráfico", "Ctrl+G"),
    ("graph:open-local", "Vista de gráfico: Abrir gráfico local", ""),
    ("graph:toggle-tags", "Vista de gráfico: Alternar etiquetas", ""),
    ("graph:toggle-attachments", "Vista de gráfico: Alternar adjuntos", ""),
    ("graph:toggle-unresolved", "Vista de gráfico: Alternar solo archivos existentes", ""),
    ("graph:toggle-orphans", "Vista de gráfico: Alternar huérfanos", ""),
    ("graph:toggle-arrows", "Vista de gráfico: Alternar flechas", ""),
    ("graph:cycle-depth", "Vista de gráfico: Cambiar profundidad del gráfico local", ""),
    ("graph:reset-view", "Vista de gráfico: Ajustar gráfico al panel", ""),
    ("graph:toggle-panel", "Vista de gráfico: Alternar panel de ajustes", ""),
    ("graph:animate", "Vista de gráfico: Animar disposición", ""),
    ("graph:reset-settings", "Vista de gráfico: Restaurar ajustes predeterminados", ""),
    ("calendar:open", "Calendario: Abrir el calendario", "Ctrl+Shift+C"),
    ("calendar:refresh", "Calendario: Actualizar ahora", ""),
    ("calview:open", "Calendario: Abrir la vista grande (mes, semana, día, agenda)", ""),
    ("calendar:open-day-note", "Calendario: Abrir la nota del día seleccionado", ""),
    ("daily-notes", "Notas diarias: Abrir la nota diaria de hoy", ""),
    ("daily-notes:goto-prev", "Notas diarias: Abrir nota diaria anterior", ""),
    ("daily-notes:goto-next", "Notas diarias: Abrir nota diaria siguiente", ""),
    ("zk-prefixer", "Creador de notas únicas: Crear nota única nueva", ""),
    ("random-note", "Nota aleatoria: Abrir nota aleatoria", ""),
    ("app:delete-file", "Eliminar archivo actual", ""),
    ("theme:toggle-light-dark", "Alternar modo claro/oscuro", ""),
    ("app:reload-theme", "Recargar temas y fragmentos CSS", ""),
    ("app:toggle-perf-overlay", "Alternar superposición de rendimiento", "Ctrl+Shift+F12"),
    ("app:open-settings", "Abrir ajustes", "Ctrl+,"),
    ("properties:add", "Añadir propiedad al archivo", ""),
    ("properties:toggle-source", "Alternar propiedades en el documento: tabla/código", ""),
    ("plugins:list", "Plugins: Listar plugins y scripts", ""),
    ("plugins:reload", "Plugins: Recargar plugins", ""),
    ("app:open-vault", "Abrir otra bóveda", ""),
    ("app:check-for-updates", "Buscar actualizaciones", ""),
    ("app:install-update", "Instalar actualización disponible", ""),
    ("app:quit", "Salir", "Ctrl+Q"),
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
    Calendar,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightPanel {
    Backlinks,
    Outgoing,
    Outline,
    Ai,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Editor,
    Search,
    Prompt,
    /// The calendar pane of the left sidebar has the keys: the arrows move the selected day.
    Calendar,
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
    /// Workspaces to save, load or delete, as `Prompt::target` says.
    Workspace,
    /// A new value for the base table cell `Prompt::target` (`path|column`).
    BaseCell,
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
    /// The row the list has to bring into view on the next layout: set when the selection moves,
    /// so the wheel keeps whatever the reader scrolled to and the keyboard always shows its row.
    pub scroll_to: Option<usize>,
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

/// A press on the inner edge of a sidebar, which drags that sidebar wider or narrower.
#[derive(Clone, Copy, Debug)]
pub struct SideDrag {
    /// The left sidebar is the one being dragged.
    pub left: bool,
    pub start_x: f32,
    /// Width the sidebar had when it was grabbed.
    pub width: f32,
}

pub const TREE_ROW_HEIGHT: f32 = 26.0;
/// Pixels the editor scrolls per wheel line (a wheel notch is three).
pub const SCROLL_LINE: f32 = 40.0;
/// Limits of a sidebar's width, as the drag handle enforces them.
pub const SIDE_W_DEFAULT: f32 = 250.0;
pub const SIDE_W_MIN: f32 = 180.0;
pub const SIDE_W_MAX: f32 = 600.0;
/// The one breakpoint of the shell: 44 px of ribbon + the two sidebars at their default 250 + the
/// ~276 px a line of prose needs. Below it the sidebars stop taking a column and float over the
/// note, which keeps the whole width.
pub const COMPACT_W: f32 = 820.0;
/// Width of a sidebar that floats over the note in a compact window: wide enough for its lists,
/// and never so wide that the note behind it is out of sight.
pub const SIDE_W_COMPACT: f32 = 260.0;
/// Room the note keeps between the two sidebars. When the window cannot hold the note and the
/// widths the sidebars were dragged to, the sidebars give up the difference in proportion
/// instead of squeezing the note to nothing and pushing its header, and its properties, out of
/// the window with it.
pub const NOTE_W_MIN: f32 = 380.0;
const META_DEBOUNCE_MS: u64 = 400;

pub struct Tab {
    pub path: String,
    pub editor: Editor,
    pub stamp: Stamp,
    pub mode: ViewMode,
    /// Index of the top-level block the view is anchored to; only blocks near it are laid out.
    pub top_block: usize,
    /// How far the view is scrolled past the top of `top_block`, in pixels. Rendering moves
    /// the anchor to the block at the top of the view.
    pub scroll_px: f32,
    /// Wheel scrolling still to be applied, eased in over the next frames.
    pub scroll_pending: f32,
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
    /// Where every block of the note starts, measured a batch at a time: the scroll limit and the
    /// scrollbar describe the whole document, not the window of blocks the view lays out.
    pub extent: crate::view::Extent,
    /// The keyboard just moved the caret out of the view: the next layout puts the block it is in
    /// on screen, at the exact line, instead of trusting a guess in blocks.
    pub caret_jump: bool,
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
    /// Show block `block` at the top of the view.
    pub fn scroll_to_block(&mut self, block: usize) {
        self.top_block = block;
        self.scroll_px = 0.0;
        self.scroll_pending = 0.0;
    }
    /// Scroll by `px`; `smooth` eases it in over the next frames.
    pub fn scroll_by(&mut self, px: f32, smooth: bool) {
        if smooth {
            self.scroll_pending += px;
        } else {
            self.scroll_px += px + std::mem::take(&mut self.scroll_pending);
        }
    }
    /// Move part of the pending scroll into the view.
    pub fn step_scroll(&mut self) {
        // A fixed share per frame: fast at first, slowing into place.
        let step =
            if self.scroll_pending.abs() < 1.0 { self.scroll_pending } else { self.scroll_pending * 0.3 };
        self.scroll_px += step;
        self.scroll_pending -= step;
    }

    pub(crate) fn new(path: String, text: &str, stamp: Stamp, vim: bool) -> Tab {
        Tab {
            path,
            editor: Editor::new(text),
            stamp,
            mode: ViewMode::LivePreview,
            top_block: 0,
            scroll_px: 0.0,
            scroll_pending: 0.0,
            last_edit_ms: 0,
            meta: None,
            folds: Default::default(),
            hidden: (0, Vec::new()),
            hidden_dirty: false,
            vim: vim.then(Default::default),
            pinned: false,
            extent: Default::default(),
            caret_jump: false,
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
    /// The `.excalidraw` board open in the main pane, and whether it has the pane.
    pub exc_view: Option<crate::excalidrawview::ExcView>,
    pub show_exc: bool,
    /// The kanban board open in the main pane, and whether it has the pane.
    pub kanban_view: Option<crate::kanbanview::KanbanView>,
    pub show_kanban: bool,
    /// The board covers the window instead of sitting in its pane.
    pub exc_fullscreen: bool,
    /// Tables of the `dataview` blocks drawn so far, per query and vault revision.
    pub(crate) dataview: crate::dataview::Cached,
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
    /// Whether the window is in its compact shape (see [`COMPACT_W`]).
    pub compact: bool,
    /// Whether the first frame already told the app what shape the window has.
    pub compact_seen: bool,
    /// Whether a command asked for a sidebar: a window that opens compact keeps that one open
    /// instead of hiding it, and any command that shows a pane wins over the shape.
    pub sidebar_asked: bool,
    /// What the sidebars were doing before the window went compact, to put them back when it
    /// grows again: a resize is not an instruction to lose what the reader had open.
    pub compact_restore: Option<(bool, bool)>,
    /// Where the bottom navigation's indicator is and where it is going, in destinations.
    pub nav_from: f32,
    pub nav_to: f32,
    /// The ⋮ menu of a compact window, open over the note.
    pub more_open: bool,
    /// Width of each sidebar in pixels, while it is open.
    pub left_w: f32,
    pub right_w: f32,
    /// The sidebar edge being dragged, if any.
    pub side_drag: Option<SideDrag>,
    pub panel: Panel,
    pub focus: Focus,
    /// Folders the explorer shows open. Everything else is collapsed, which is how the tree
    /// starts: the set is the exception, so an empty one means a fully folded tree.
    pub expanded: HashSet<String>,
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
    /// What the last frame repainted: the window diffs its display list and paints only what
    /// changed. `reset` it to force a full repaint.
    pub damage: metamind_ui::DamageTracker,
    /// Copy screenshot was chosen: the window copies the pane from the next frame it draws.
    pub screenshot: bool,
    pub quit: bool,
    /// Scroll offsets of scrollable panes, by element id.
    pub scrolls: HashMap<String, f32>,
    /// Width the note column of each (pane, tab) had in the last frame that drew it. The heights
    /// of the blocks depend on it, so the measuring pass needs the same number.
    pub note_widths: HashMap<(usize, usize), f32>,
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
    /// The drawing tablet, when the machine has one: pressure for the pen strokes.
    pub pen: crate::pen::Pen,
    /// The new-board dialog, while it is open.
    pub kanban_new: Option<crate::kanbanview::NewBoard>,
    /// The MCP servers of this session and their tools.
    pub mcp: crate::mcp::Mcp,
    /// The calendar tab in the main pane, and whether it is the one showing.
    pub calendar_view: Option<crate::calendarview::CalendarView>,
    pub show_calendar_view: bool,
    /// Where the tab indicator comes from while it slides to the tab that just took the focus.
    pub tab_from: Option<(usize, usize)>,
    /// The in-app video player, while a note's video card is playing.
    pub video: crate::video::VideoPlayer,
    /// YouTube links played inside the application: the `yt-dlp` lookup and the run in flight.
    pub youtube: crate::youtube::Jobs,
    /// The month of events in the left sidebar, from the iCal address and the vault.
    pub calendar: crate::calendar::Calendar,
    /// Width and height of the workspace in the last frame drawn: the space an over-window
    /// overlay has, with the status bar already taken out of it.
    pub video_area: (f32, f32),
    /// Width of the note column in the last frame that drew one: a poster is sized to it before
    /// the next layout, instead of to the whole window, which is not the room the note has.
    pub editor_w: f32,
    /// The AI copilot of the right sidebar.
    pub ai: crate::ai::AiPanel,
    /// The models the AI panel's provider listed, fetched in the background from Ajustes → IA.
    pub ai_models: crate::ai::ModelList,
    /// The dashboard: the last week of daily notes, read once per vault revision.
    pub dash: crate::dashboard::Cache,
    /// First frames of the videos in the notes, decoded once each and kept within a budget.
    pub posters: crate::posters::Posters,
    /// Posters the frame asked for: the tree is built with a shared borrow, so the requests are
    /// drained right after it, before the next frame.
    pub poster_queue: std::cell::RefCell<Vec<crate::posters::PosterRequest>>,
    /// UI scale factor (window scale).
    pub scale: f32,
    pub video_waker: Option<Box<dyn Fn() + Send>>,
    pub(crate) seen: Option<crate::anim::Seen>,
    pub(crate) notice_since: Option<std::time::Instant>,
    pub update: crate::update::State,
    /// A long operation shown with a spinner in the status bar.
    pub busy: Option<crate::anim::Busy>,
    /// Paths of closed tabs, most recent last, for "Reopen closed tab".
    closed_tabs: Vec<String>,
    /// Set by "Open another vault"; the window shows the vault chooser when it sees this.
    pub switch_vault: bool,
    /// The open right-click menu.
    pub menu: Option<crate::menu::Menu>,
    /// Logical window width of the last frame, to keep menus on screen.
    pub window_w: f32,
    /// Logical window height of the last frame, for the views that fill it.
    pub window_h: f32,
    /// Source of the Mermaid diagram shown full screen, after a click on its block.
    pub diagram_zoom: Option<String>,
    /// Pan offset (x, y) of the diagram zoom overlay, in pixels of the window.
    pub diagram_zoom_pan: (f32, f32),
    /// Extra zoom of the diagram overlay, on top of the fit its box gives it (1.0 = fit).
    pub diagram_zoom_scale: f32,
    /// The diagram overlay shows its source instead of the drawing (double click).
    pub diagram_zoom_source: bool,
    /// When the overlay was opened: the second press of the click that opened it belongs to the
    /// block it came from, not to the view.
    pub diagram_zoom_since: Option<std::time::Instant>,
    /// The overlay is being dragged: where the pointer went down and the pan from then.
    pub diagram_drag: Option<((f32, f32), (f32, f32))>,
    /// Callout blocks the reader folded or unfolded, by note and block start. A callout marked
    /// `-` in the source is folded until this says otherwise, and one that is open folds when it
    /// says so: the set is the reader's override of the source.
    pub callout_toggled: std::collections::HashSet<(String, usize)>,
    /// Mermaid blocks shown as their source instead of the drawing, by note and block start.
    pub mermaid_source: std::collections::HashSet<(String, usize)>,
    /// The video player is drawn as a modal over the window instead of its note.
    pub video_fullscreen: bool,
    /// Where and when the last press landed, to tell a double click from two clicks.
    pub last_click: Option<(std::time::Instant, (f32, f32))>,
    /// A drag happened since that press, so the next one cannot be its second half.
    pub clicked_dragged: bool,
    /// Off in headless use (tests, screenshots) so they never touch the user's clipboard;
    /// copies then stay inside the application.
    pub system_clipboard: bool,
    pub(crate) clip: String,
}

/// Sort key for explorer order: at each level folders come before files, and names compare
/// case-insensitively. Byte-wise comparison of the keys gives the order.
/// A path lowercased the way the explorer groups folders: `Materias` and `materias` are one
/// folder there, so both name the same branch. `App.expanded` stores paths already folded,
/// so a lookup is one hash and never a walk over the set.
pub(crate) fn folded_path(p: &str) -> String {
    p.chars().flat_map(char::to_lowercase).collect()
}

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

/// Keys that edit text or move the caret in a field. A sheet in front of the note takes them for
/// its own field instead of letting them reach the note underneath.
fn edits_text(key: &str) -> bool {
    matches!(
        key,
        "Enter"
            | "Backspace"
            | "Delete"
            | "Tab"
            | "Home"
            | "End"
            | "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "ArrowDown"
            | "PageUp"
            | "PageDown"
    )
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
    /// Follows the window shape into and out of compact. Going compact closes the sidebars, which
    /// now float over the note; going back restores exactly what was open before, because a resize
    /// is not an instruction to throw away the reader's layout.
    pub fn sync_compact(&mut self) {
        let compact = self.window_w > 0.0 && self.window_w < COMPACT_W;
        if !self.compact_seen {
            // The first frame is not a resize: the window was born this shape. A compact one
            // starts with the note at full width, unless a command already asked for a sidebar.
            self.compact_seen = true;
            self.compact = compact;
            if compact && !self.sidebar_asked {
                self.left_open = false;
                self.right_open = false;
            }
            return;
        }
        if compact == self.compact {
            return;
        }
        self.compact = compact;
        if compact {
            self.compact_restore = Some((self.left_open, self.right_open));
            self.left_open = false;
            self.right_open = false;
        } else {
            // The ⋮ menu hangs from the bottom navigation, which only a compact window has: left
            // open, it would keep the keyboard and the clicks with nothing on screen to close.
            self.more_open = false;
            if let Some((left, right)) = self.compact_restore.take() {
                self.left_open = left;
                self.right_open = right;
            }
        }
    }

    /// Moves the bottom navigation's indicator to `to`, from wherever it is right now: pressing a
    /// destination mid-slide continues from the point on screen, not from the previous one.
    pub fn nav_go(&mut self, to: f32) {
        let t = self.anims.t("bottom-nav");
        self.nav_from += (self.nav_to - self.nav_from) * t;
        self.nav_to = to;
        self.anims.start("bottom-nav", 180.0);
    }

    /// Shows a sidebar because a command asked for one of its panes. In a compact window a
    /// sidebar floats over the note, so the other one makes room.
    pub fn show_side(&mut self, left: bool) {
        self.sidebar_asked = true;
        if left {
            self.left_open = true;
            if self.compact {
                self.right_open = false;
            }
        } else {
            self.right_open = true;
            if self.compact {
                self.left_open = false;
            }
        }
    }

    /// The copilot covers the workspace as a sheet when the window is too narrow to give it a
    /// column: only a compact window does that, and only while the right sidebar shows it.
    pub fn ai_sheet(&self) -> bool {
        self.compact && self.right_open && self.right_panel == RightPanel::Ai
    }

    /// The ⋮ menu is a sheet of the same kind, and only a compact window draws it: the flag alone
    /// must not take the keyboard in a window that has no navigation to hang the list from.
    pub fn more_sheet(&self) -> bool {
        self.compact && self.more_open
    }

    /// Whether the copilot sheet is on screen: open, or still sliding back out after it was
    /// closed. It comes in from the right, and it leaves the same way.
    pub fn sheet_drawn(&self) -> bool {
        self.ai_sheet() || (self.compact && self.anims.t("ai-sheet") < 1.0)
    }

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
            exc_view: None,
            show_exc: false,
            kanban_view: None,
            show_kanban: false,
            exc_fullscreen: false,
            dataview: Default::default(),
            video: Default::default(),
            youtube: crate::youtube::Jobs::new(),
            calendar: crate::calendar::Calendar::new(
                root,
                config.app.str("cal.ical_url").unwrap_or_default(),
            ),
            video_area: (0.0, 0.0),
            editor_w: 0.0,
            ai: Default::default(),
            ai_models: Default::default(),
            dash: Default::default(),
            posters: Default::default(),
            poster_queue: Default::default(),
            scale: 1.0,
            video_waker: None,
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
            compact: false,
            compact_seen: false,
            sidebar_asked: false,
            compact_restore: None,
            nav_from: 0.0,
            nav_to: 0.0,
            more_open: false,
            left_w: SIDE_W_DEFAULT,
            right_w: SIDE_W_DEFAULT,
            side_drag: None,
            panel: Panel::Files,
            focus: Focus::Editor,
            expanded: HashSet::new(),
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
            damage: metamind_ui::DamageTracker::new(),
            screenshot: false,
            quit: false,
            scrolls: HashMap::new(),
            note_widths: HashMap::new(),
            fuzzy: Fuzzy::new(),
            plugins: Default::default(),
            hover_action: None,
            explorer_drag: None,
            suggest: None,
            hover_preview: None,
            suggest_dismissed: None,
            anims: Default::default(),
            pen: Default::default(),
            kanban_new: None,
            mcp: crate::mcp::Mcp::default(),
            calendar_view: None,
            show_calendar_view: false,
            tab_from: None,
            seen: None,
            notice_since: None,
            update: crate::update::State::Idle,
            busy: None,
            closed_tabs: Vec::new(),
            switch_vault: false,
            menu: None,
            window_w: 1280.0,
            window_h: 800.0,
            diagram_zoom: None,
            diagram_zoom_pan: (0.0, 0.0),
            diagram_zoom_scale: 1.0,
            diagram_zoom_source: false,
            diagram_zoom_since: None,
            diagram_drag: None,
            callout_toggled: Default::default(),
            mermaid_source: Default::default(),
            video_fullscreen: false,
            last_click: None,
            clicked_dragged: false,
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
        self.calendar_refresh();
        // The copilot's stream is read here, not only in the window loop: the panel has to work
        // the same with no window on screen (a test, a headless run).
        self.ai.poll();
        // The copilot's tools run on this thread, where the store is: a model that asked for a
        // function gets its answer and asks again inside the same turn.
        self.ai_tools_step();
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

    /// Bring the calendar up to date once per frame: finish a fetch, scan a slice of the notes
    /// for `@{fecha}` tasks, and ask for another body when the pane is on screen and the cache is
    /// old. Nothing here waits for the network.
    pub fn calendar_refresh(&mut self) {
        let root = self.store.root.clone();
        if self.left_open && self.panel == Panel::Calendar {
            if self.config.app.bool("cal.refresh_on_open", true) {
                let url = self.config.app.str("cal.ical_url").unwrap_or_default().to_string();
                self.calendar.maybe_fetch(&root, &url);
            }
            let revision = crate::dataview::revision(self);
            if revision != self.calendar.revision() {
                self.calendar.scan_vault(&self.store, revision);
            }
        }
        self.calendar.tick(&root, &self.store);
        // A models query answers on its own thread: wake the window so the answer is drawn now
        // instead of at the next input or deadline. The waker is the same one the video player
        // uses: it only asks for a frame.
        if self.ai_models.poll() {
            if let Some(waker) = &self.video_waker {
                waker();
            }
        }
        if let Some(report) = self.calendar.take_report() {
            self.notice = Some(match report {
                Ok(0) => "La dirección respondió, pero no hay ningún evento".to_string(),
                Ok(n) => format!("{n} evento{} en el calendario", if n == 1 { "" } else { "s" }),
                Err(message) => message,
            });
        }
    }

    /// Milliseconds until the running calendar fetch has to be looked at again, if any.
    pub fn calendar_due_in(&self) -> Option<u64> {
        self.calendar.due_in()
    }

    /// Milliseconds until the outline/word count of the active note should be refreshed.
    pub fn derived_due_in(&self, now_ms: u64) -> Option<u64> {
        let t = self.tab()?;
        let stale = t.meta.as_ref().is_none_or(|(mv, _)| *mv != t.editor.buf.version);
        stale.then(|| (t.last_edit_ms + META_DEBOUNCE_MS).saturating_sub(now_ms))
    }

    /// Whether folder `path` is open. `expanded` keeps its keys folded (see [`folded_path`]),
    /// so this is one hash lookup, not a scan that lowercases every entry.
    pub fn folder_open(&self, path: &str) -> bool {
        self.expanded.contains(&folded_path(path))
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
        // The folders already shown for the branch being walked, folded the way the sort key is:
        // two spellings of one folder (`Materias` and `materias`, which a case-sensitive disk
        // allows) share the branch, so the second one never lists its ancestors again.
        let mut open_dirs: Vec<String> = Vec::new();
        for (p, kind) in paths {
            let dir = path::parent(p);
            let segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
            let mut acc = String::new();
            let mut folded = String::new();
            let mut hidden = false;
            for (depth, seg) in segs.iter().enumerate() {
                if !acc.is_empty() {
                    acc.push('/');
                    folded.push('/');
                }
                acc.push_str(seg);
                folded.extend(seg.chars().flat_map(char::to_lowercase));
                if hidden {
                    break;
                }
                let collapsed = !self.folder_open(&folded);
                if open_dirs.get(depth).map(String::as_str) != Some(folded.as_str()) {
                    open_dirs.truncate(depth);
                    open_dirs.push(folded.clone());
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
        self.tabs.get(self.active).filter(|_| self.editor_shown())
    }
    pub fn tab_mut(&mut self) -> Option<&mut Tab> {
        let shown = self.editor_shown();
        self.tabs.get_mut(self.active).filter(|_| shown)
    }

    /// Whether the panes show the active note instead of a graph, a canvas, a base or a board.
    pub fn editor_shown(&self) -> bool {
        !self.show_graph
            && !self.canvas_active()
            && !self.base_active()
            && !self.exc_active()
            && !self.kanban_active()
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
        // A video or a song is played, not opened as a note: the tree, a link and the command
        // palette all land here, and "solo se pueden abrir notas Markdown" was the answer to a
        // click on an `.mp4` that the app has a player for.
        if crate::video::is_video(&rel) {
            let note = self.tab().map(|t| t.path.clone()).unwrap_or_default();
            let path = self.store.abs(&rel);
            match self.video.open(&rel, &path, &note, self.card_cap()) {
                Ok(()) => {
                    if let Some(waker) = &self.video_waker {
                        waker();
                    }
                }
                Err(e) => self.notice = Some(e),
            }
            return;
        }
        if path::kind_of(&rel) == FileKind::Canvas {
            return self.open_canvas(&rel);
        }
        if path::kind_of(&rel) == FileKind::Base {
            return self.open_base(&rel);
        }
        if path::extension(&rel).eq_ignore_ascii_case("excalidraw") {
            return self.open_exc(&rel);
        }
        // A `.kanban` (or `.board`) file is a board without the Markdown name.
        if crate::kanbanview::is_board_path(&rel) {
            return self.open_kanban(&rel);
        }
        if let Some(i) = self.tabs.iter().position(|t| t.path.eq_ignore_ascii_case(&rel)) {
            self.active = i;
        } else {
            if path::kind_of(&rel) != FileKind::Markdown {
                self.notice = Some(format!("{rel}: solo se pueden abrir notas Markdown en esta compilación"));
                return;
            }
            match self.store.read(&rel) {
                Ok((text, stamp)) => {
                    // A note carrying the Kanban plugin's settings *is* a board: the tree opens
                    // the board, and the view has its own way into the text.
                    // A note the Excalidraw plugin wrote is a board: the scene lives in its
                    // `%%` block, and showing the note would be a wall of JSON.
                    if excalidraw_core::scene_in_markdown(&text).is_some() {
                        let rel = rel.clone();
                        self.open_exc_note(&rel);
                        self.plugins.push(ext_api::Event::FileOpen { path: rel.clone() });
                        if record {
                            self.history.truncate(self.history_pos + usize::from(!self.history.is_empty()));
                            if self.history.last() != Some(&rel) {
                                self.history.push(rel);
                            }
                        }
                        return;
                    }
                    if crate::kanbanview::is_board_text(&text) {
                        let rel = rel.clone();
                        self.open_board(&rel, &text, stamp);
                        self.plugins.push(ext_api::Event::FileOpen { path: rel.clone() });
                        if record {
                            self.history.truncate(self.history_pos + usize::from(!self.history.is_empty()));
                            if self.history.last() != Some(&rel) {
                                self.history.push(rel);
                            }
                            self.history_pos = self.history.len().saturating_sub(1);
                        }
                        return;
                    }
                    self.tabs.push(Tab::new(rel.clone(), &text, stamp, self.vim));
                    self.active = self.tabs.len() - 1;
                    self.init_new_tab();
                }
                Err(e) => {
                    self.notice = Some(format!("No se pudo abrir {rel}: {e}"));
                    return;
                }
            }
        }
        self.focus = Focus::Editor;
        self.show_graph = false;
        self.show_canvas = false;
        self.show_base = false;
        // Leaving the board for a note writes it, the same as closing its tab.
        if self.show_exc {
            if let Some(v) = &mut self.exc_view {
                v.commit_edit();
            }
            self.exc_save();
        }
        self.show_exc = false;
        self.exc_fullscreen = false;
        if self.show_kanban {
            if let Some(v) = &mut self.kanban_view {
                v.stop_edit();
            }
            self.kanban_save();
        }
        self.show_kanban = false;
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
                    "{} se cambió fuera de MetaMind. No se guardó; tu texto se conserva en esta pestaña.",
                    tab.path
                ));
                false
            }
            Err(WriteError::Io(e)) => {
                self.notice = Some(format!("No se pudo guardar: {e}"));
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

    /// The frame size a player in a note asks for: the card the column shows at 1x.
    pub fn card_cap(&self) -> (u32, u32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        ((640.0 * scale) as u32, (360.0 * scale) as u32)
    }

    /// Called from `render_window` before building the view: reads frames, updates the clock,
    /// and finishes the `yt-dlp` runs that were started from a YouTube card.
    pub fn video_tick(&mut self, now: std::time::Instant) {
        self.video.tick(now);
        let _ = self.video.run_action("");
        if let crate::video::State::Failed(message) = self.video.state() {
            self.notice = Some(message.to_string());
        }
        // A resolved YouTube stream opens the player, in the note whose card asked for it.
        match self.youtube.poll() {
            Some(crate::youtube::Outcome::Ready { watch, url, note }) => {
                self.notice = None;
                if let Err(e) = self.video.open_url(&watch, &url, &note, self.card_cap()) {
                    self.notice = Some(e);
                } else if let Some(waker) = &self.video_waker {
                    waker();
                }
            }
            Some(crate::youtube::Outcome::Failed(message)) => self.notice = Some(message),
            None => {}
        }
        // The overlay asks for a frame that fits the box it has, once per window size.
        if self.video_fullscreen {
            self.fit_video_overlay();
        }
        if !self.video.note().is_empty() {
            let open = self.all_tabs().any(|t| t.path.eq_ignore_ascii_case(self.video.note()));
            if !open && self.video.is_open() {
                self.video.close();
            }
        }
    }

    /// Milliseconds until the player needs another tick (for the wait deadline), if playing.
    pub fn video_due_in(&self) -> Option<u64> {
        if self.video.is_open() && !matches!(self.video.state(), crate::video::State::Paused) {
            Some(250)
        } else {
            None
        }
    }

    /// Milliseconds until the running `yt-dlp` has to be looked at again, if one is running.
    pub fn youtube_due_in(&self) -> Option<u64> {
        self.youtube.busy().then_some(150)
    }

    /// Close the video player, called when the media file is deleted or renamed.
    pub fn close_player(&mut self) {
        self.video.close();
    }

    /// Save every modified tab (called when the window loses focus, on a timer, and on quit).
    pub fn save_all(&mut self) {
        self.canvas_save();
        self.exc_save();
        self.kanban_save();
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
        // A batch of external changes means the index is being rebuilt; show it in the status bar.
        if paths.len() >= 8 {
            self.start_busy(format!("Indexando {} archivos…", paths.len()));
        }
        self.tree = None;
        self.canvas_files_changed(paths);
        self.exc_files_changed(paths);
        self.kanban_files_changed(paths);
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
                self.notice = Some(format!("{p} cambió en el disco y aquí tiene cambios sin guardar."));
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
                        aux: "Intro para crear".into(),
                        action: format!("create:{}", query.trim()),
                    });
                }
                items
            }
            PromptKind::Template => self.template_items(&query),
            PromptKind::Workspace => {
                let target = self.prompt.as_ref().map(|p| p.target.clone()).unwrap_or_default();
                self.workspace_items(&target, &query)
            }
            PromptKind::BaseCell => {
                let target = self.prompt.as_ref().map(|p| p.target.clone()).unwrap_or_default();
                self.base_cell_items(&target, &query)
            }
            PromptKind::Rename | PromptKind::NewFolder => {
                let target = self.prompt.as_ref().map(|p| p.target.clone()).unwrap_or_default();
                let name = query.trim();
                if name.is_empty() {
                    Vec::new()
                } else if kind == PromptKind::Rename {
                    vec![PromptItem {
                        label: format!("Renombrar a \u{201c}{name}\u{201d}"),
                        aux: "Enter".into(),
                        action: format!("rename:{target}|{name}"),
                    }]
                } else {
                    vec![PromptItem {
                        label: format!("Crear carpeta \u{201c}{name}\u{201d}"),
                        aux: if target.is_empty() {
                            "en la raíz de la bóveda".into()
                        } else {
                            format!("en {target}")
                        },
                        action: format!("mkdir:{target}|{name}"),
                    }]
                }
            }
        };
        if let Some(p) = &mut self.prompt {
            p.items = items;
            p.selected = p.selected.min(p.items.len().saturating_sub(1));
            p.scroll_to = Some(p.selected);
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
        // The palette takes the keyboard: a field that was being written keeps its text and stops
        // taking the keys, so what is typed next reaches the palette.
        self.ai.editing = false;
        self.prompt = Some(Prompt {
            kind,
            target: String::new(),
            query: String::new(),
            selected: 0,
            items: Vec::new(),
            scroll_to: Some(0),
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

    /// The first `max` hits for `query`, as `(path, line, snippet)`: what the copilot's
    /// `search_notes` tool reads. The corpus is the same one the search pane uses, built on first
    /// use, so the model and the reader see the same vault.
    pub fn search_excerpts(&mut self, query_text: &str, max: usize) -> Vec<(String, usize, String)> {
        let Ok(query) = Query::parse(query_text) else { return Vec::new() };
        let root = self.store.root.clone();
        let corpus = self.corpus.get_or_insert_with(|| Corpus::build(&self.store.index, &root));
        let hits =
            corpus.search(&self.store.index, &root, &query, Sort::Relevance, RankWeights::default(), max);
        self.store.release_cache_pages();
        let mut out = Vec::new();
        for hit in hits {
            let Some(file) = self.store.index.entry(hit.file) else { continue };
            let path = file.path.to_string();
            let text = self.store.read(&path).map(|(t, _)| t).unwrap_or_default();
            let (line, snippet) = match hit.matches.first() {
                Some(m) => {
                    let at = (m.start as usize).min(text.len());
                    let line = text[..at].matches('\n').count() + 1;
                    (line, search_core::snippet(&text, *m, 60).0.trim().to_string())
                }
                None => (1, String::new()),
            };
            out.push((path, line, snippet));
        }
        out
    }

    // ---- commands ----

    /// Run a command by id. Some take an argument after a colon (`open:path`).
    pub fn run(&mut self, command: &str) {
        let now = fsio::now_ms();
        // A command fired while the ⋮ menu is open belongs to the menu: the list goes first, so
        // what the command opens is not painted under it. The menu's own commands are the ones
        // that drive it.
        if self.more_open && !command.starts_with("more:") {
            self.more_open = false;
        }
        if let Some(name) = self.disabled_by(command) {
            self.notice = Some(format!("{name} está desactivado en Ajustes → Plugins"));
            return;
        }
        if matches!(command.split(':').next(), Some("create" | "file-explorer" | "daily-notes" | "app")) {
            // These can add or remove files; the explorer rows are rebuilt on the next frame.
            self.tree = None;
        }
        // ---- in-app video player ----
        // Toggle a folded callout: clicking its title expands/collapses the content.
        if let Some(block) = command.strip_prefix("callout:toggle:").and_then(|b| b.parse::<usize>().ok()) {
            let note = self.tabs.get(self.active).map(|t| t.path.clone()).unwrap_or_default();
            let key = (note, block);
            if !self.callout_toggled.insert(key.clone()) {
                self.callout_toggled.remove(&key);
            }
            return;
        }
        if command == "video:fullscreen" {
            if self.video.is_open() {
                self.set_video_fullscreen(!self.video_fullscreen);
            }
            return;
        }
        if command == "video:fullscreen-close" {
            self.set_video_fullscreen(false);
            return;
        }
        // A YouTube card: `yt-dlp` resolves the stream in the background and the player opens when
        // it answers (see `video_tick`). The video is drawn where the note is.
        if let Some(url) = command.strip_prefix("youtube:open:") {
            let note = self.tab().map(|t| t.path.clone()).unwrap_or_default();
            match self.youtube.start(url, &note) {
                Ok(()) => self.notice = Some("Preparando el video…".to_string()),
                Err(e) => self.notice = Some(e),
            }
            return;
        }
        // A plain link the app cannot show: the browser opens it.
        if let Some(url) = command.strip_prefix("open-url:") {
            self.open_in_browser(url);
            return;
        }
        if let Some(rest) = command.strip_prefix("video:") {
            if let Some((note, rel)) = rest.strip_prefix("open:").and_then(|r| r.split_once('|')) {
                let path = self.store.abs(rel);
                match self.video.open(rel, &path, note, self.card_cap()) {
                    Ok(()) => {
                        // The frames arrive on a thread of their own: the window has to know.
                        if let Some(waker) = &self.video_waker {
                            waker();
                        }
                    }
                    Err(e) => self.notice = Some(e),
                }
                return;
            }
            if self.video.run_action(command) {
                return;
            }
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
                self.notice = Some(format!("No se pudo añadir \"{word}\" al diccionario: {e}"));
            }
            return;
        }
        if command == "notice:dismiss" {
            self.notice = None;
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
                    self.notice = Some(format!("No se pudo crear {rel}: {e}"));
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
                // Giving the pane back to a note writes the board first.
                if let Some(v) = &mut self.exc_view {
                    v.commit_edit();
                }
                self.exc_save();
                self.show_exc = false;
                self.exc_fullscreen = false;
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
        if let Some(i) = command.strip_prefix("pane-close:").and_then(|i| i.parse::<usize>().ok()) {
            return self.close_pane_at(i);
        }
        if let Some(folder) = command.strip_prefix("toggle-folder:") {
            let key = folded_path(folder);
            if !self.expanded.remove(&key) {
                self.expanded.insert(key);
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
        if self.run_plugin_command(command)
            || self.run_calendar(command)
            || self.run_panes(command)
            || self.run_templates(command)
            || self.run_workspaces(command)
        {
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
                self.nav_go(1.0);
                self.show_side(true);
                self.panel = Panel::Search;
                self.focus = Focus::Search;
            }
            "file-explorer:open" => {
                self.nav_go(0.0);
                self.show_side(true);
                self.panel = Panel::Files;
            }
            "file-explorer:new-file" => {
                let rel = path::join(&self.new_note_dir(), &self.unique_untitled());
                match self.store.create(&rel, "") {
                    Ok(_) => self.open_note(&rel),
                    Err(e) => self.notice = Some(format!("No se pudo crear la nota: {e}")),
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
                    Err(e) => self.notice = Some(format!("No se pudo crear la base: {e}")),
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
                    Err(e) => self.notice = Some(format!("No se pudo crear el lienzo: {e}")),
                }
            }
            "editor:save-file" => {
                if self.exc_active() {
                    self.exc_save();
                    self.notice = None;
                } else if self.canvas_active() {
                    self.canvas_save();
                    self.notice = None;
                } else if self.kanban_active() {
                    self.kanban_save();
                    self.notice = None;
                } else if self.save() {
                    self.notice = None;
                }
            }
            "workspace:close" | "graph:close" if self.show_graph || command == "graph:close" => {
                self.graph_view = None;
                self.show_graph = false;
            }
            "workspace:close" if self.base_active() => self.run("base:close"),
            "workspace:close" | "exc:close" if self.exc_active() || command == "exc:close" => {
                self.exc_close();
            }
            "workspace:close" | "kanban:close" if self.kanban_active() || command == "kanban:close" => {
                self.kanban_close();
            }
            "workspace:close" | "canvas:close" if self.canvas_active() || command == "canvas:close" => {
                self.canvas_save();
                self.canvas_view = None;
                self.show_canvas = false;
                self.show_base = false;
            }
            "workspace:close" => self.close_tab(self.active),
            "workspace:split-vertical" => self.split(true),
            "workspace:split-horizontal" => self.split(false),
            "workspace:close-pane" => self.close_pane(),
            "workspace:move-to-new-window" => self.move_to_window(),
            "workspace:toggle-stacked-tabs" => self.toggle_stacked(),
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
                    .map(|d| folded_path(&d))
                    .collect();
                if self.expanded.is_empty() {
                    self.expanded = folders;
                } else {
                    self.expanded.clear();
                }
            }
            "app:check-for-updates" => {
                self.update.start_check(true);
                self.notice = Some("Buscando actualizaciones…".into());
            }
            "app:install-update" => match self.update.start_install() {
                Ok(()) => self.notice = Some("Descargando la actualización…".into()),
                Err(e) => self.notice = Some(format!("No se pudo actualizar: {e}")),
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
            "workspace:next-tab" | "workspace:previous-tab" if self.exc_active() => {
                // Turning the page writes the board and leaves its tab in the bar, one press away.
                if let Some(v) = &mut self.exc_view {
                    v.commit_edit();
                }
                self.exc_save();
                self.show_exc = false;
                self.exc_fullscreen = false;
            }
            "workspace:next-tab" | "workspace:previous-tab" if self.kanban_active() => {
                // The same for a kanban board: its tab stays, the note comes back.
                if let Some(v) = &mut self.kanban_view {
                    v.stop_edit();
                }
                self.kanban_save();
                self.show_kanban = false;
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
            // In a compact window a sidebar floats over the note, so two of them would cover it
            // whole: opening one closes the other.
            "app:toggle-left-sidebar" => {
                self.sidebar_asked = true;
                self.left_open = !self.left_open;
                if self.compact && self.left_open {
                    self.right_open = false;
                }
            }
            "app:toggle-right-sidebar" => {
                self.sidebar_asked = true;
                self.right_open = !self.right_open;
                if self.compact && self.right_open {
                    self.left_open = false;
                }
            }
            c if c.starts_with("file-explorer:sort:") => {
                self.config.app.set("fileSortOrder", c["file-explorer:sort:".len()..].into());
                if let Err(e) = self.config.app.save() {
                    self.notice = Some(format!("No se pudo guardar el orden: {e}"));
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
            "backlinks:open" | "outgoing-links:open" | "outline:open" | "ai:open" => {
                if command == "ai:open" {
                    self.nav_go(3.0);
                }
                self.show_side(false);
                self.right_panel = match command {
                    "backlinks:open" => RightPanel::Backlinks,
                    "outgoing-links:open" => RightPanel::Outgoing,
                    "ai:open" => RightPanel::Ai,
                    _ => RightPanel::Outline,
                };
                if command == "ai:open" {
                    self.ai.scroll_to_end = true;
                }
            }
            // The way out of the copilot sheet: the panel closes and the sidebar stops taking
            // the keyboard, so the note gets both back.
            "ai:close" => {
                self.right_open = false;
                self.ai.editing = false;
            }
            "ai:focus-input" => {
                self.ai.editing = true;
            }
            // A card of the dashboard opens the note it names.
            c if c.starts_with("dashboard:open:") => {
                let name = &c["dashboard:open:".len()..];
                let found = self
                    .store
                    .index
                    .notes()
                    .find(|(_, f)| vault_core::path::stem(&f.path) == name)
                    .map(|(_, f)| f.path.to_string());
                match found {
                    Some(path) => self.open_note(&path),
                    None => self.notice = Some(format!("No encontré la nota «{name}»")),
                }
            }
            "more:open" => {
                // The list covers the note and anything floating over it, so it takes the keyboard
                // as well: a field behind it keeps its text and stops taking the keys.
                self.more_open = true;
                self.ai.editing = false;
            }
            "more:close" => self.more_open = false,
            // A row of the menu names a command: running it closes the menu behind it.
            c if c.starts_with("more:run:") => {
                let cmd = c["more:run:".len()..].to_string();
                self.more_open = false;
                self.run(&cmd);
            }
            "dashboard:open" => match crate::dashboard::open_or_create(self) {
                Ok(path) => {
                    self.more_open = false;
                    self.open_note(&path);
                }
                Err(e) => self.notice = Some(e),
            },
            "ai:send" => self.ai_send(),
            "ai:clear" => self.ai.clear(),
            "ai:explain-selection" => self.ai_quick(crate::ai::QuickAsk::Selection),
            "ai:summarize-note" => self.ai_quick(crate::ai::QuickAsk::Summary),
            c if c.starts_with("ai:profile:") => {
                let name = &c["ai:profile:".len()..];
                let mut cfg = self.ai_config();
                if cfg.profiles.iter().any(|p| p.name == name) {
                    cfg.active = name.to_string();
                    self.set_ai_config(&cfg);
                    self.notice = Some(format!("Perfil de IA: {name}"));
                }
            }
            "ai:settings" => {
                if self.settings.is_none() {
                    self.run("app:open-settings");
                }
                if let Some(s) = &mut self.settings {
                    s.section = crate::settings::Section::Ai;
                }
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
                            self.notice = Some(format!("Se movió {p} a la papelera"));
                        }
                        Err(e) => self.notice = Some(format!("No se pudo eliminar {p}: {e}")),
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
                // Over a board, Ctrl+Z and Ctrl+Shift+Z are the board's own history, not the
                // note's: the pane shows the board, so the keys have to act on it.
                if self.exc_active() && self.run_exc_editor(command) {
                    return;
                }
                if self.run_calendar_view(command) {
                    return;
                }
                if self.run_graph(command)
                    || self.run_canvas(command)
                    || self.run_base(command)
                    || self.run_kanban(command)
                    || self.run_exc(command)
                {
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
        if self.focus == Focus::Editor && !self.canvas_active() && !self.exc_active() && !self.kanban_active()
        {
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
        if self.calendar_key(&key) {
            return true;
        }
        if self.kanban_new_key(&key) {
            return true;
        }
        // A diagram shown full screen is a modal of its own: Escape, or a click on the backdrop,
        // closes it.
        if self.diagram_zoom.is_some() && key.key == "Escape" {
            self.close_diagram_zoom();
            return true;
        }
        // The same for a video shown over the window: Escape gives the note its player back.
        if self.video_fullscreen && key.key == "Escape" {
            self.set_video_fullscreen(false);
            return true;
        }
        // The ⋮ menu is a sheet too: Escape closes it, and a plain key stays with the list instead
        // of editing the note it hides. Ctrl and Alt go on to the shortcuts.
        if self.more_sheet() && !key.ctrl && !key.alt {
            if key.key == "Escape" {
                self.run("more:close");
            }
            return true;
        }
        // The copilot sheet covers the workspace: Escape gives the note back. The input takes the
        // first press to stop editing, so this one is the second, or one with no field in front.
        // With the palette over the sheet, Escape is the palette's: it is the one on top.
        if self.ai_sheet() && self.prompt.is_none() && key.key == "Escape" {
            self.run("ai:close");
            return true;
        }
        // A key that would edit text belongs to the question, never to the note the sheet hides:
        // it gives the field back its caret, which is what a typed letter does. With the palette
        // over the sheet, the palette keeps its keys.
        if self.ai_sheet() && self.prompt.is_none() && !key.ctrl && !key.alt && edits_text(&key.key) {
            self.ai.editing = true;
            self.suggest = None;
            return true;
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
                    "ArrowDown" => {
                        p.selected = (p.selected + 1).min(p.items.len().saturating_sub(1));
                        p.scroll_to = Some(p.selected);
                    }
                    "ArrowUp" => {
                        p.selected = p.selected.saturating_sub(1);
                        p.scroll_to = Some(p.selected);
                    }
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
            // The calendar pane: the arrows move the selected day, Page Up and Down the month,
            // Enter opens the daily note of the day on screen.
            Focus::Calendar => {
                match key.key.as_str() {
                    "ArrowLeft" => self.calendar.step_day(-1),
                    "ArrowRight" => self.calendar.step_day(1),
                    "ArrowUp" => self.calendar.step_day(-7),
                    "ArrowDown" => self.calendar.step_day(7),
                    "PageUp" => self.calendar.step_month(-1),
                    "PageDown" => self.calendar.step_month(1),
                    "Home" => {
                        let today = crate::calendar::Calendar::today();
                        self.calendar.select(today);
                    }
                    "Enter" => self.run("calendar:open-day-note"),
                    "Escape" => self.focus = Focus::Editor,
                    _ => return false,
                }
                true
            }
            Focus::Editor => {
                if self.canvas_active() {
                    return self.canvas_key(&key);
                }
                if self.kanban_active() && self.kanban_key(&key) {
                    return true;
                }
                if self.exc_active() && self.exc_key(&key) {
                    return true;
                }
                let Some(t) = self.tabs.get_mut(self.active).filter(|_| !self.show_graph) else {
                    return false;
                };
                if t.mode == ViewMode::Reading {
                    match key.key.as_str() {
                        "ArrowDown" => t.scroll_by(SCROLL_LINE, false),
                        "ArrowUp" => t.scroll_by(-SCROLL_LINE, false),
                        "PageDown" => t.scroll_by(SCROLL_LINE * 10.0, !self.reduced_motion),
                        "PageUp" => t.scroll_by(-SCROLL_LINE * 10.0, !self.reduced_motion),
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
        // The event editor is a modal: a letter belongs to the field it is editing, never to the
        // note behind it.
        if self.calendar_text(text) {
            return;
        }
        if self.kanban_new_text(text) {
            return;
        }
        if self.modal_text(text) {
            return;
        }
        // The ⋮ menu covers the note: a letter is not an edit of a note nobody can see.
        if self.more_sheet() {
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
            // A letter with the calendar pane focused is not a note edit; the shortcuts still run.
            Focus::Calendar => {}
            // The sheet covers the note: a printable key belongs to the question, never to a note
            // the reader cannot see. Typing into it is also what puts the caret back in the field.
            Focus::Editor if self.ai_sheet() => {
                self.ai.editing = true;
                self.ai.input.push_str(text);
            }
            Focus::Editor if self.canvas_active() => self.canvas_text(text),
            Focus::Editor if self.kanban_active() => self.kanban_text(text),
            Focus::Editor if self.exc_active() => {
                self.exc_text(text);
            }
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
            t.scroll_to_block(0);
            return;
        };
        if block < t.top_block || (block == t.top_block && t.scroll_px > 0.0) {
            t.scroll_to_block(block);
        } else if block > t.top_block + 18 {
            t.scroll_to_block(block - 18);
        } else {
            return;
        }
        // The anchor is a coarse guess (a block is not a screen): the layout is what knows how
        // tall the blocks above the caret are, so it is the one that brings the caret into view.
        t.caret_jump = true;
    }

    /// Scroll the focused editor by `lines` wheel lines.
    pub fn scroll_editor(&mut self, lines: f32) {
        let p = self.pane;
        let active = self.active;
        self.scroll_tab(p, active, lines);
    }

    /// Ease in pending wheel scrolling, once per frame.
    pub fn step_scrolls(&mut self) {
        for (i, p) in self.panes.iter_mut().enumerate() {
            if i != self.pane {
                p.tabs.iter_mut().for_each(Tab::step_scroll);
            }
        }
        self.tabs.iter_mut().for_each(Tab::step_scroll);
    }

    pub fn scrolling(&self) -> bool {
        let other = self.panes.iter().enumerate().filter(|&(i, _)| i != self.pane).flat_map(|(_, p)| &p.tabs);
        self.tabs.iter().chain(other).any(|t| t.scroll_pending != 0.0)
    }
}
