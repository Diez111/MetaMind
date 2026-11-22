//! Kanban boards opened in the main pane: a Markdown file with the Obsidian Kanban plugin's
//! settings, shown as columns of cards you can drag around.
//!
//! # The format
//!
//! A board is a plain `.md` file whose first lines carry the plugin's settings block and whose
//! `##` headings are columns and whose list items are cards:
//!
//! ```text
//! %% kanban: settings
//! {"kanban-plugin": "basic"} %%
//! # Título del tablero
//!
//! ## Pendiente
//! - [ ] Primera tarea
//!
//! ## Hecho
//! - [x] Terminada
//! ```
//!
//! [`Board::parse`] reads that shape and keeps everything it does not understand: the settings
//! block goes back out verbatim, and any line under a column that is not a card is kept after the
//! cards. [`Board::render`] writes the same shape back, so a board read and written without an
//! edit is the same file byte for byte (see the roundtrip tests). A `.kanban` (or `.board`) file is
//! treated the same as a `.md` that carries the settings.
//!
//! # The view
//!
//! Like the canvas and the Excalidraw board (`canvasview.rs`, `excalidrawview.rs`) the board is a
//! display list painted into one node of the pane, after layout: the cards wrap to the width of
//! their column whatever the zoom of the window, the columns scroll sideways when they do not fit,
//! and nothing is ever clipped mid-card. The paint pass is where the font is in hand, so it is also
//! the place that lays the cards out; the geometry it leaves behind ([`KanbanView::geom`]) is what
//! the next press, drag or wheel is hit-tested against.
//!
//! # Editing
//!
//! A double click on a card edits it in place; `Enter` commits, `Escape` commits as well (the same
//! choice the Excalidraw text box makes), `Backspace` edits, and a click elsewhere commits. The
//! `+` at the foot of a column adds a card and starts typing in it. Text arrives from
//! `App::text_input`, so an input method types into a card the way it types into a text box.
//!
//! # Saving
//!
//! The same scheme as the rest of the boards: leaving the view, `Ctrl+S`, `save_all` and the end
//! of a drag write the file. A drag writes it on the drop, so the order on disk is the order on
//! screen as soon as the card lands.

use crate::shell::{App, Focus};
use metamind_ui::{Cmd, Dom, El, Fonts, NodeId, Rect, Span, TextStyle};
use theme_engine::Color;
use vault_core::fsio::Stamp;
use vault_core::{path, WriteError};

/// Hidden elements the board colours are read from: the same twelve the canvas view reads.
pub const COLOR_CLASSES: [&str; 12] = crate::canvasview::COLOR_CLASSES;

/// The width of a column. Fixed, so the cards of a board are the same size everywhere.
pub const COL_W: f32 = 280.0;
/// Room kept around the columns.
const EDGE: f32 = 14.0;
/// Between two columns.
const GAP: f32 = 12.0;
/// The header of a column: its name, its count and its menu.
const HEAD_H: f32 = 34.0;
/// A card is inset this much inside its column.
const INSET: f32 = 8.0;
/// Padding inside a card.
const CARD_PAD: f32 = 9.0;
/// Between two cards.
const CARD_GAP: f32 = 7.0;
/// The side of the checkbox a card leads with.
const CHECK: f32 = 15.0;
/// A card is never shorter than this, even with one short line.
const MIN_CARD_H: f32 = 34.0;
/// The `+` at the foot of a column.
const FOOT_H: f32 = 28.0;
/// How far the pointer travels before a press on a card becomes a drag.
const DRAG_START: f32 = 4.0;
/// Screen pixels a wheel line scrolls the board by.
const WHEEL_STEP: f32 = 44.0;
/// How long the card just dropped takes to fade in.
pub const DROP_MS: f32 = 120.0;
/// Card text.
const CARD_TEXT: f32 = 13.0;
const CARD_LINE: f32 = 18.5;

/// The settings block a board written by the plugin opens with.
pub const SETTINGS: &str = "%% kanban: settings\n{\"kanban-plugin\": \"basic\"} %%";

/// A new board: the three columns of the plugin's own template, empty and ready to fill.
pub const TEMPLATE: &str =
    "%% kanban: settings\n{\"kanban-plugin\": \"basic\"} %%\n# Tablero\n\n## Pendiente\n\n## Haciendo\n\n## Hecho\n";

/// Whether a file is a board by its extension: the plugin writes `.md`, but a `.kanban` (or the
/// older `.board`) is the same thing without the Markdown name.
pub fn is_board_path(p: &str) -> bool {
    matches!(path::extension(p).to_ascii_lowercase().as_str(), "kanban" | "board")
}

/// Whether what a file holds is a board: a note with the plugin's settings block at the top.
pub fn is_board_text(text: &str) -> bool {
    text.lines().take(8).any(is_settings_start)
}

fn is_settings_start(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("%%") && t.to_ascii_lowercase().replace(' ', "").contains("kanban:")
}

// ---- the model ----

/// A card: one list item of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// Stable while the board is open: the selection, the fade and the drag follow it.
    pub id: u64,
    pub done: bool,
    pub text: String,
    /// The file wrote a plain list item (no checkbox): it goes back out the same way.
    pub bare: bool,
}

/// A column: one `##` heading and the cards under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub cards: Vec<Card>,
    /// Lines under the cards that are not cards, kept so the file keeps what it had.
    pub extra: Vec<String>,
    /// Folded on screen: the header stays, the cards are hidden. A view state, not saved.
    pub collapsed: bool,
}

/// A board file, as the view sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    /// The `%% kanban: settings … %%` block, verbatim, when the file has one.
    pub settings: Option<String>,
    /// The `# ` title, when the file has one.
    pub title: Option<String>,
    /// Lines before the first column that are neither the settings nor the title.
    pub before: Vec<String>,
    pub columns: Vec<Column>,
    /// Lines after the last column.
    pub after: Vec<String>,
    next_id: u64,
}

impl Default for Board {
    fn default() -> Board {
        Board {
            settings: Some(SETTINGS.to_string()),
            title: None,
            before: Vec::new(),
            columns: Vec::new(),
            after: Vec::new(),
            next_id: 1,
        }
    }
}

/// The card a line holds, with the text after the marker.
fn card_line(line: &str) -> Option<(bool, bool, String)> {
    let t = line.trim();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "))?;
    let rest = rest.trim_start();
    for (mark, done) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(text) = rest.strip_prefix(mark) {
            if text.is_empty() || text.starts_with(' ') {
                return Some((done, false, text.trim().to_string()));
            }
        }
    }
    // A list item with no checkbox is still an item of a column: it is a card as well, and it
    // goes back out as the plain item it was.
    (!rest.is_empty()).then(|| (false, true, rest.trim().to_string()))
}

/// The text of a `##` heading, or `None` when the line is not one.
fn heading_column(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("##")?;
    if rest.starts_with('#') || !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim().to_string())
}

fn heading_title(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix('#')?;
    (rest.starts_with(' ') && !rest.starts_with("##")).then(|| rest.trim().to_string())
}

impl Board {
    /// Read a board. Nothing is rejected: what is not a column, a card or the settings is kept
    /// as it came.
    pub fn parse(text: &str) -> Board {
        let mut board = Board { next_id: 1, ..Board::default() };
        board.settings = None;
        board.title = None;
        let mut in_settings = false;
        let mut seen_column = false;
        for raw in text.lines() {
            let line = raw.trim_end();
            if in_settings {
                if let Some(s) = &mut board.settings {
                    s.push('\n');
                    s.push_str(line);
                }
                if line.trim_end().ends_with("%%") && !line.trim_end().eq("%%") {
                    in_settings = false;
                }
                continue;
            }
            if is_settings_start(line) {
                in_settings = true;
                board.settings = Some(line.to_string());
                // A one-line block closes on itself: `%% kanban: settings {"plugin":"basic"} %%`.
                let t = line.trim_end();
                if t.ends_with("%%") && t.trim_start().len() > 4 {
                    in_settings = false;
                }
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            if let Some(name) = heading_column(line) {
                seen_column = true;
                board.columns.push(Column { name, cards: Vec::new(), extra: Vec::new(), collapsed: false });
                continue;
            }
            if !seen_column {
                if board.title.is_none() {
                    if let Some(title) = heading_title(line) {
                        board.title = Some(title);
                        continue;
                    }
                }
                board.before.push(line.to_string());
                continue;
            }
            let card = card_line(line);
            if !seen_column {
                // A card with no column over it is kept as it came, before the first column.
                board.before.push(line.to_string());
                continue;
            }
            let col = board.columns.len() - 1;
            match card {
                Some((done, bare, text)) => {
                    let id = board.next_id;
                    board.next_id += 1;
                    board.columns[col].cards.push(Card { id, done, text, bare });
                }
                None => board.columns[col].extra.push(line.to_string()),
            }
        }
        board
    }

    /// The file this board saves as: the canonical shape [`Board::parse`] reads back exactly.
    pub fn render(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        if let Some(s) = &self.settings {
            out.extend(s.lines().map(str::to_string));
        }
        if let Some(t) = &self.title {
            out.push(format!("# {t}"));
        }
        if !self.before.is_empty() {
            out.push(String::new());
            out.extend(self.before.iter().cloned());
        }
        for col in &self.columns {
            out.push(String::new());
            out.push(format!("## {}", col.name));
            for card in &col.cards {
                out.push(if card.bare {
                    format!("- {}", card.text)
                } else {
                    format!("- [{}] {}", if card.done { "x" } else { " " }, card.text)
                });
            }
            out.extend(col.extra.iter().cloned());
        }
        if !self.after.is_empty() {
            out.push(String::new());
            out.extend(self.after.iter().cloned());
        }
        let mut text = out.join("\n");
        text.push('\n');
        text
    }

    /// The file, for a board opened from an empty or missing one.
    pub fn template() -> Board {
        Board::parse(TEMPLATE)
    }

    pub fn cards(&self) -> usize {
        self.columns.iter().map(|c| c.cards.len()).sum()
    }

    pub fn done(&self) -> usize {
        self.columns.iter().flat_map(|c| &c.cards).filter(|c| c.done).count()
    }

    /// Where a card lives, as (column, index in that column).
    pub fn locate(&self, id: u64) -> Option<(usize, usize)> {
        for (i, col) in self.columns.iter().enumerate() {
            if let Some(j) = col.cards.iter().position(|c| c.id == id) {
                return Some((i, j));
            }
        }
        None
    }

    pub fn card(&self, id: u64) -> Option<&Card> {
        self.columns.iter().flat_map(|c| &c.cards).find(|c| c.id == id)
    }

    pub fn card_mut(&mut self, id: u64) -> Option<&mut Card> {
        self.columns.iter_mut().flat_map(|c| &mut c.cards).find(|c| c.id == id)
    }

    /// Append a card to column `col` and answer its id.
    pub fn push_card(&mut self, col: usize, text: impl Into<String>) -> Option<u64> {
        if col >= self.columns.len() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.columns[col].cards.push(Card { id, done: false, text: text.into(), bare: false });
        Some(id)
    }

    pub fn remove_card(&mut self, id: u64) -> Option<Card> {
        let (col, at) = self.locate(id)?;
        Some(self.columns[col].cards.remove(at))
    }

    /// Move a card to `col`, at index `idx` of the list it will be in. True when it moved.
    pub fn move_card(&mut self, id: u64, col: usize, idx: usize) -> bool {
        let Some((from, at)) = self.locate(id) else { return false };
        if col >= self.columns.len() {
            return false;
        }
        if from == col && at == idx {
            return false;
        }
        let card = self.columns[from].cards.remove(at);
        let idx = idx.min(self.columns[col].cards.len());
        self.columns[col].cards.insert(idx, card);
        self.locate(id) != Some((from, at))
    }

    pub fn add_column(&mut self, name: impl Into<String>) -> usize {
        self.columns.push(Column {
            name: name.into(),
            cards: Vec::new(),
            extra: Vec::new(),
            collapsed: false,
        });
        self.columns.len() - 1
    }

    /// Swap a column with its neighbour. True when there was one to swap with.
    pub fn move_column(&mut self, i: usize, right: bool) -> bool {
        let j = if right { i + 1 } else { i.wrapping_sub(1) };
        if i >= self.columns.len() || j >= self.columns.len() || j == i {
            return false;
        }
        self.columns.swap(i, j);
        true
    }
}

// ---- the view ----

/// What is being typed into, and the text typed so far.
#[derive(Clone, Debug, PartialEq)]
enum Edit {
    /// An existing card.
    Card(u64),
    /// A card the press just made: left empty, it is taken away again.
    NewCard(u64),
    Column(usize),
    NewColumn(usize),
}

struct Editing {
    target: Edit,
    buf: String,
}

/// What a press landed on, so the app can open a menu or a link.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    None,
    Card(u64),
    Check(u64),
    ColumnMenu(usize),
    ColumnFold(usize),
    ColumnName(usize),
    AddCard(usize),
    ColumnBody(usize),
    /// A `[[link]]` inside a card.
    Link(String),
}

/// A card, or the header it hangs from, in window coordinates after the last paint.
#[derive(Clone)]
struct CardGeom {
    id: u64,
    /// The whole card.
    rect: Rect,
    /// The checkbox at its left.
    check: Rect,
    /// Where its text starts and how wide it wraps.
    text: Rect,
}

/// What a pointer is over, for the highlight of the board.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum Hover {
    #[default]
    None,
    Card(u64),
    /// The checkbox of a card.
    Check(u64),
    /// A column: its header, its empty space or its menu.
    Column(usize),
    /// The `+` at the foot of a column.
    Add(usize),
}

#[derive(Clone)]
struct ColGeom {
    /// The column: header, cards and the `+` at the foot.
    rect: Rect,
    head: Rect,
    fold: Rect,
    menu: Rect,
    add: Rect,
    cards: Vec<CardGeom>,
}

enum Press {
    /// A card grabbed. It moves when the pointer does.
    Card { id: u64, origin: (usize, usize), at: (f32, f32) },
    /// Empty space: the board follows the pointer.
    Pan { last: (f32, f32) },
}


/// A board opened as a tab.
pub struct KanbanView {
    pub path: String,
    pub board: Board,
    stamp: Stamp,
    /// How far the board is scrolled, in screen pixels.
    pub scroll: (f32, f32),
    /// The content size the last paint measured, for the scroll limits.
    content: (f32, f32),
    /// The pane the last paint used.
    rect: Rect,
    pub selected: Option<u64>,
    editing: Option<Editing>,
    /// The board as text instead of as columns: the `<>` toggle.
    pub show_source: bool,
    pub dirty: bool,
    press: Option<Press>,
    /// The geometry captured when the card was grabbed: a drag reads the drop slot from it, so
    /// moving the card in the model never moves the slot under the pointer.
    press_geom: Vec<ColGeom>,
    /// The card being dragged.
    dragging: Option<u64>,
    /// The card just dropped, fading in while the editor's clock runs.
    pub drop: Option<u64>,
    drop_started: bool,
    /// What the pointer is over, so the board answers before it is pressed.
    hover: Hover,
    geom: Vec<ColGeom>,
    /// The pane's text style, kept for the hit-test of a link.
    text_style: TextStyle,
    card_style: TextStyle,
}

impl KanbanView {
    pub fn open(path: &str, text: &str, stamp: Stamp) -> KanbanView {
        let card_style = TextStyle {
            size: CARD_TEXT,
            line_height: CARD_LINE,
            color: Color::rgb(200, 200, 200),
            bold: false,
            italic: false,
            mono: false,
            wrap: true,
            family: None,
        };
        KanbanView {
            path: path.to_string(),
            board: Board::parse(text),
            stamp,
            scroll: (0.0, 0.0),
            content: (0.0, 0.0),
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            selected: None,
            editing: None,
            show_source: false,
            dirty: false,
            press: None,
            press_geom: Vec::new(),
            dragging: None,
            drop: None,
            drop_started: false,
            hover: Hover::default(),
            geom: Vec::new(),
            text_style: TextStyle {
                size: 13.0,
                line_height: 18.0,
                color: Color::rgb(200, 200, 200),
                bold: false,
                italic: false,
                mono: false,
                wrap: true,
                family: None,
            },
            card_style,
        }
    }


    pub fn title(&self) -> String {
        path::stem(&self.path).to_string()
    }

    pub fn to_markdown(&self) -> String {
        self.board.render()
    }

    /// The column header a menu hangs from, as window coordinates.
    pub fn column_anchor(&self, i: usize) -> (f32, f32) {
        self.geom
            .get(i)
            .map(|g| (g.head.x + 24.0, g.head.y + g.head.h))
            .unwrap_or((self.rect.x + EDGE, self.rect.y + EDGE))
    }

    /// The column as the last paint laid it out, in window coordinates. A test (or the menu) can
    /// ask where a card ended up without knowing how the board is laid out.
    pub fn column_rect(&self, i: usize) -> Option<Rect> {
        self.geom.get(i).map(|g| g.rect)
    }

    /// The card at (column, index) as the last paint laid it out, in window coordinates.
    pub fn card_rect(&self, col: usize, idx: usize) -> Option<Rect> {
        self.geom.get(col)?.cards.get(idx).map(|c| c.rect)
    }

    /// The checkbox of the card at (column, index).
    pub fn check_rect(&self, col: usize, idx: usize) -> Option<Rect> {
        self.geom.get(col)?.cards.get(idx).map(|c| c.check)
    }

    /// The `+` at the foot of a column.
    pub fn add_rect(&self, col: usize) -> Option<Rect> {
        self.geom.get(col).map(|g| g.add)
    }

    /// The chevron that folds a column.
    pub fn fold_rect(&self, col: usize) -> Option<Rect> {
        self.geom.get(col).map(|g| g.fold)
    }

    /// Where a `[[link]]` of a card sits, in window coordinates, from the last paint.
    pub fn link_rect(
        &mut self,
        fonts: &mut Fonts,
        col: usize,
        idx: usize,
        target: &str,
    ) -> Option<Rect> {
        let card = self.geom.get(col)?.cards.get(idx)?;
        let text = self.board.card(card.id)?.text.clone();
        let style = self.card_style.clone();
        let spans = card_spans(&text, &chip_ranges(&text), &[Color::rgb(120, 120, 120); 12], false);
        let (s, e) = chip_ranges(&text).into_iter().find_map(|(s, e, chip)| match chip {
            Chip::Link(t) if t == target => Some((s, e)),
            _ => None,
        })?;
        let r = fonts.range_rects(&text, &spans, &style, Some(card.text.w), s, e).first().copied()?;
        Some(Rect::new(card.text.x + r.x, card.text.y + r.y, r.w.max(1.0), r.h))
    }

    /// Whether a card or a column name is being typed right now.
    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// The column under a point, from the last paint.
    pub fn column_at(&self, x: f32, y: f32) -> Option<usize> {
        self.geom.iter().position(|g| x >= g.rect.x && x < g.rect.x + g.rect.w && y >= g.head.y)
    }

    // ---- typing ----

    pub(crate) fn stop_edit(&mut self) -> bool {
        let Some(editing) = self.editing.take() else { return false };
        let text = editing.buf.trim().to_string();
        let target = editing.target;
        let mut changed = false;
        match target {
            Edit::Card(id) | Edit::NewCard(id) => {
                if text.is_empty() {
                    if matches!(target, Edit::NewCard(_)) {
                        // A card that was never given a text is not a card.
                        changed = self.board.remove_card(id).is_some();
                    }
                } else if let Some(card) = self.board.card_mut(id) {
                    if card.text != text {
                        card.text = text;
                        changed = true;
                    }
                }
            }
            Edit::Column(i) => {
                if let Some(col) = self.board.columns.get_mut(i) {
                    if !text.is_empty() && col.name != text {
                        col.name = text;
                        changed = true;
                    }
                }
            }
            Edit::NewColumn(i) => {
                if let Some(col) = self.board.columns.get_mut(i) {
                    if !text.is_empty() && col.name != text {
                        col.name = text;
                        changed = true;
                    }
                }
            }
        }
        self.dirty |= changed;
        changed
    }

    /// What is typed right now, if it is the card `id`.
    fn editing_card(&self, id: u64) -> Option<&str> {
        match self.editing.as_ref().map(|e| (&e.target, e.buf.as_str())) {
            Some((Edit::Card(i) | Edit::NewCard(i), buf)) if *i == id => Some(buf),
            _ => None,
        }
    }

    fn editing_column(&self, i: usize) -> Option<&str> {
        match self.editing.as_ref().map(|e| (&e.target, e.buf.as_str())) {
            Some((Edit::Column(j) | Edit::NewColumn(j), buf)) if *j == i => Some(buf),
            _ => None,
        }
    }

    fn start_card(&mut self, id: u64, fresh: bool) {
        if self.editing.is_some() {
            self.stop_edit();
        }
        let text = self.board.card(id).map(|c| c.text.clone()).unwrap_or_default();
        let target = if fresh { Edit::NewCard(id) } else { Edit::Card(id) };
        self.editing = Some(Editing { target, buf: text });
        self.selected = Some(id);
    }

    fn start_column(&mut self, i: usize, fresh: bool) {
        if self.editing.is_some() {
            self.stop_edit();
        }
        let name = self.board.columns.get(i).map(|c| c.name.clone()).unwrap_or_default();
        // A column that was just made starts with its placeholder name in the header, not in the
        // box: the first key typed is the first letter of its name.
        let (target, buf) = if fresh {
            (Edit::NewColumn(i), String::new())
        } else {
            (Edit::Column(i), name)
        };
        self.editing = Some(Editing { target, buf });
        self.selected = None;
    }

    /// Add a card at the end of a column and start typing in it.
    pub fn add_card(&mut self, col: usize) -> Option<u64> {
        let id = self.board.push_card(col, "")?;
        self.dirty = true;
        self.start_card(id, true);
        self.scroll_to(col);
        Some(id)
    }

    /// Bring a column into view.
    fn scroll_to(&mut self, col: usize) {
        let Some(g) = self.geom.get(col).cloned() else { return };
        let left = g.rect.x;
        let right = left + g.rect.w;
        if left < self.rect.x {
            self.scroll.0 -= self.rect.x - left + EDGE;
        } else if right > self.rect.x + self.rect.w {
            self.scroll.0 += right - (self.rect.x + self.rect.w) + EDGE;
        }
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let max_x = (self.content.0 - self.rect.w).max(0.0);
        let max_y = (self.content.1 - self.rect.h).max(0.0);
        self.scroll.0 = self.scroll.0.clamp(0.0, max_x);
        self.scroll.1 = self.scroll.1.clamp(0.0, max_y);
    }

    // ---- input ----

   /// What the pointer is over, from the last paint.
    fn hit(&self, x: f32, y: f32) -> Hit {
        for (i, g) in self.geom.iter().enumerate() {
            if !g.rect.contains(x, y) {
                continue;
            }
            if g.head.contains(x, y) {
                if g.fold.contains(x, y) {
                    return Hit::ColumnFold(i);
                }
                if g.menu.contains(x, y) {
                    return Hit::ColumnMenu(i);
                }
                return Hit::ColumnName(i);
            }
            if g.add.contains(x, y) {
                return Hit::AddCard(i);
            }
            for card in &g.cards {
                if card.check.contains(x, y) {
                    return Hit::Check(card.id);
                }
                if card.rect.contains(x, y) {
                    return Hit::Card(card.id);
                }
            }
            return Hit::ColumnBody(i);
        }
        Hit::None
    }

    /// The `[[link]]` under a point inside card `id`.
    fn link_at(&mut self, fonts: &mut Fonts, id: u64, x: f32, y: f32) -> Option<String> {
        let text = self.board.card(id)?.text.clone();
        let chips = chip_ranges(&text);
        if chips.iter().all(|(_, _, chip)| !matches!(chip, Chip::Link(_))) {
            return None;
        }
        let g = self.geom.iter().flat_map(|c| &c.cards).find(|c| c.id == id)?;
        let style = self.card_style.clone();
        let off = fonts.hit(&text, &[], &style, Some(g.text.w), x - g.text.x, y - g.text.y);
        chips.into_iter().find_map(|(s, e, chip)| match chip {
            Chip::Link(target) if s <= off && off < e => Some(target),
            _ => None,
        })
    }

    /// A press on the board. Returns what it landed on.
    pub fn press(&mut self, rect: Rect, x: f32, y: f32, fonts: &mut Fonts) -> Hit {
        self.rect = rect;
        let hit = self.hit(x, y);
        let same = match (self.editing.as_ref().map(|e| e.target.clone()), &hit) {
            (Some(Edit::Card(a) | Edit::NewCard(a)), Hit::Card(b) | Hit::Check(b)) => a == *b,
            (Some(Edit::Column(i) | Edit::NewColumn(i)), Hit::ColumnName(j)) => i == *j,
            _ => false,
        };
        if !same {
            self.stop_edit();
        }
        self.dragging = None;
        match hit.clone() {
            Hit::Check(id) => {
                if let Some(card) = self.board.card_mut(id) {
                    card.done = !card.done;
                    self.dirty = true;
                }
                self.selected = Some(id);
                self.press = None;
            }
            Hit::Card(id) => {
                if let Some(target) = self.link_at(fonts, id, x, y) {
                    self.press = None;
                    return Hit::Link(target);
                }
                self.selected = Some(id);
                let origin = self.board.locate(id).unwrap_or((0, 0));
                self.press_geom = self.geom.clone();
                self.press = Some(Press::Card { id, origin, at: (x, y) });
            }
            Hit::AddCard(i) => {
                self.press = None;
                self.add_card(i);
            }
            Hit::ColumnFold(i) => {
                if let Some(col) = self.board.columns.get_mut(i) {
                    col.collapsed = !col.collapsed;
                }
                self.press = None;
            }
            Hit::ColumnMenu(i) => {
                let _ = i;
                self.press = None;
            }
            Hit::ColumnName(_) | Hit::ColumnBody(_) | Hit::None => {
                self.selected = None;
                self.press = Some(Press::Pan { last: (x, y) });
            }
            Hit::Link(_) => self.press = None,
        }
        hit
    }

    /// The second half of a double click: edit the card under the pointer, rename a column, or
    /// add a card where the pointer is.
    pub fn double_click(&mut self, rect: Rect, x: f32, y: f32) -> bool {
        self.rect = rect;
        match self.hit(x, y) {
            Hit::Card(id) => {
                self.start_card(id, false);
                true
            }
            Hit::ColumnName(i) => {
                self.start_column(i, false);
                true
            }
            Hit::ColumnBody(i) | Hit::AddCard(i) => {
                self.add_card(i);
                true
            }
            _ => false,
        }
    }

    /// The pointer moved with the button down. True when the board changed.
    pub fn drag(&mut self, x: f32, y: f32) -> bool {
        match self.press.as_ref() {
            Some(Press::Pan { last }) => {
                let (lx, ly) = *last;
                self.scroll.0 -= x - lx;
                self.scroll.1 -= y - ly;
                self.clamp_scroll();
                if let Some(Press::Pan { last }) = &mut self.press {
                    *last = (x, y);
                }
                false
            }
            Some(Press::Card { id, origin, at }) => {
                let (id, origin, at) = (*id, *origin, *at);
                if self.dragging.is_none() && (x - at.0).abs() + (y - at.1).abs() < DRAG_START {
                    return false;
                }
                self.dragging = Some(id);
                let (col, idx) = self.slot_at(id, origin, x, y);
                let moved = self.board.move_card(id, col, idx);
                self.dirty |= moved;
                moved
            }
            None => false,
        }
    }

    /// The column and the slot a pointer asks for, read from the geometry captured on the press:
    /// the card being dragged is not in that list, so the slot is the index it lands on.
    fn slot_at(&self, id: u64, origin: (usize, usize), x: f32, y: f32) -> (usize, usize) {
        if self.press_geom.is_empty() {
            return origin;
        }
        let col = self
            .press_geom
            .iter()
            .enumerate()
            .find(|(_, g)| x >= g.rect.x && x < g.rect.x + g.rect.w)
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                // Between two columns, or past the last one: the nearest.
                self.press_geom
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let da = (x - (a.rect.x + a.rect.w / 2.0)).abs();
                        let db = (x - (b.rect.x + b.rect.w / 2.0)).abs();
                        da.total_cmp(&db)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(origin.0)
            });
        let others: Vec<&CardGeom> =
            self.press_geom[col].cards.iter().filter(|c| c.id != id).collect();
        let mut idx = others.len();
        for (i, card) in others.iter().enumerate() {
            if y < card.rect.y + card.rect.h / 2.0 {
                idx = i;
                break;
            }
        }
        (col, idx)
    }

    /// Note what the pointer is over. True when the highlight changed, so the frame is redrawn.
    pub fn hover(&mut self, x: f32, y: f32) -> bool {
        let over = match self.hit(x, y) {
            Hit::Card(id) => Hover::Card(id),
            Hit::Check(id) => Hover::Check(id),
            Hit::AddCard(i) => Hover::Add(i),
            Hit::ColumnMenu(i) | Hit::ColumnFold(i) | Hit::ColumnName(i) | Hit::ColumnBody(i) => {
                Hover::Column(i)
            }
            Hit::Link(ref target) => self
                .board
                .columns
                .iter()
                .flat_map(|c| &c.cards)
                .find(|card| {
                    chip_ranges(&card.text)
                        .into_iter()
                        .any(|(_, _, chip)| matches!(chip, Chip::Link(t) if t == *target))
                })
                .map_or(Hover::None, |card| Hover::Card(card.id)),
            Hit::None => Hover::None,
        };
        if self.hover == over {
            return false;
        }
        self.hover = over;
        true
    }

    /// The button came up. True when a card was dropped, so the app writes the file.
    pub fn release(&mut self) -> bool {
        let dragged = self.dragging.take();
        self.press = None;
        self.press_geom.clear();
        match dragged {
            Some(id) => {
                self.drop = Some(id);
                self.drop_started = false;
                true
            }
            None => false,
        }
    }

    /// Wheel over the board: down scrolls the columns, sideways (Shift) walks them.
    pub fn wheel(&mut self, rect: Rect, x: f32, y: f32, lines: f32, shift: bool) -> bool {
        if !rect.contains(x, y) {
            return false;
        }
        self.rect = rect;
        if shift {
            self.scroll.0 += lines * WHEEL_STEP;
        } else {
            self.scroll.1 += lines * WHEEL_STEP;
        }
        self.clamp_scroll();
        true
    }

    /// True while a drag is under way.
    pub fn dragging(&self) -> bool {
        self.dragging.is_some()
    }

    // ---- painting ----

    pub fn paint(
        &mut self,
        rect: Rect,
        c: &[Color; 12],
        text: &TextStyle,
        fonts: &mut Fonts,
        fade: f32,
    ) -> Vec<Cmd> {
        self.rect = rect;
        self.text_style = text.clone();
        self.card_style = TextStyle {
            size: CARD_TEXT,
            line_height: CARD_LINE,
            color: c[2],
            wrap: true,
            ..text.clone()
        };
        let mut cmds = vec![Cmd::PushClip(rect)];
        cmds.push(Cmd::Fill { rect, color: c[0], radius: 0.0 });
        if self.show_source {
            self.geom.clear();
            self.paint_source(&mut cmds, rect, c, fonts);
        } else {
            self.paint_board(&mut cmds, rect, c, fonts, fade);
        }
        cmds.push(Cmd::PopClip);
        cmds
    }

    fn paint_source(&mut self, cmds: &mut Vec<Cmd>, rect: Rect, c: &[Color; 12], fonts: &mut Fonts) {
        self.clamp_scroll();
        let hint_style = TextStyle {
            size: 11.5,
            line_height: 16.0,
            color: c[3],
            wrap: false,
            ..self.text_style.clone()
        };
        cmds.push(Cmd::Text {
            x: rect.x + 18.0,
            y: rect.y + 12.0,
            width: Some(rect.w - 36.0),
            text: "Vista de código · doble clic para volver al tablero".to_string(),
            spans: Vec::new(),
            style: hint_style,
        });
        let style = TextStyle {
            size: 12.5,
            line_height: 19.0,
            color: c[2],
            mono: true,
            wrap: true,
            ..self.text_style.clone()
        };
        let src = self.to_markdown();
        let w = (rect.w - 36.0).max(40.0);
        let x = rect.x + 18.0;
        let y = rect.y + 34.0 - self.scroll.1;
        let (_, h) = fonts.measure(&src, &[], &style, Some(w));
        self.content = (rect.w, h + 60.0);
        cmds.push(Cmd::Text { x, y, width: Some(w), text: src, spans: Vec::new(), style });
    }

    fn paint_board(
        &mut self,
        cmds: &mut Vec<Cmd>,
        rect: Rect,
        c: &[Color; 12],
        fonts: &mut Fonts,
        fade: f32,
    ) {
        self.clamp_scroll();
        let top = rect.y + EDGE - self.scroll.1;
        let start_x = rect.x + EDGE - self.scroll.0;
        let col_bg = mix(c[0], c[1], 0.55);
        let card_bg = mix(c[0], c[1], 0.14);
        let head_style = TextStyle {
            size: 13.5,
            line_height: 18.0,
            color: c[2],
            bold: true,
            wrap: false,
            ..self.text_style.clone()
        };
        let count_style = TextStyle {
            size: 11.5,
            line_height: 15.0,
            color: c[3],
            wrap: false,
            ..self.text_style.clone()
        };
        let mut geom: Vec<ColGeom> = Vec::with_capacity(self.board.columns.len());
        let mut bottom = top;
        for i in 0..self.board.columns.len() {
            let col = self.board.columns[i].clone();
            let x = start_x + i as f32 * (COL_W + GAP);
            let head = Rect::new(x, top, COL_W, HEAD_H);
            let fold = Rect::new(x + 8.0, top + 8.0, 20.0, 20.0);
            let menu = Rect::new(x + COL_W - 30.0, top + 7.0, 22.0, 22.0);
            let mut y = top + HEAD_H;
            let mut cards: Vec<CardGeom> = Vec::new();
            if !col.collapsed {
                for card in &col.cards {
                    let cg = self.card_geom(card, x, y, c, fonts);
                    y = cg.rect.y + cg.rect.h + CARD_GAP;
                    cards.push(cg);
                }
            }
            let add = Rect::new(x + INSET, y, COL_W - 2.0 * INSET, FOOT_H);
            let height = if col.collapsed { HEAD_H } else { (add.y + add.h) - top };
            let rect_col = Rect::new(x, top, COL_W, height);
            bottom = bottom.max(top + height);

            // A column the pointer is over answers: its chevron and its dots take the accent.
            let near = matches!(self.hover, Hover::Column(j) | Hover::Add(j) if j == i);
            let control = if near { c[2] } else { c[3] };
            cmds.push(fill(rect_col, col_bg, 10.0));
            // The header: fold chevron, name, count, menu.
            let (cx, cy) = (fold.x + fold.w / 2.0, fold.y + fold.h / 2.0);
            let chev: Vec<[f32; 4]> = if col.collapsed {
                vec![[cx - 3.0, cy - 5.0, cx + 2.0, cy], [cx + 2.0, cy, cx - 3.0, cy + 5.0]]
            } else {
                vec![[cx - 5.0, cy - 2.0, cx, cy + 3.0], [cx, cy + 3.0, cx + 5.0, cy - 2.0]]
            };
            cmds.push(Cmd::Lines { segs: chev, width: 1.6, color: control });
            let name = self.editing_column(i).map(str::to_string).unwrap_or_else(|| col.name.clone());
            let name_w = COL_W - 34.0 - 34.0;
            cmds.push(Cmd::Text {
                x: fold.x + fold.w + 6.0,
                y: top + 9.0,
                width: Some(name_w),
                text: name,
                spans: Vec::new(),
                style: head_style.clone(),
            });
            let done = col.cards.iter().filter(|c| c.done).count();
            let count = if done > 0 {
                format!("{done}/{}", col.cards.len())
            } else {
                col.cards.len().to_string()
            };
            let (cw, _) = fonts.measure(&count, &[], &count_style, None);
            cmds.push(Cmd::Text {
                x: menu.x - cw - 4.0,
                y: top + 10.0,
                width: Some(cw + 2.0),
                text: count,
                spans: Vec::new(),
                style: count_style.clone(),
            });
            let dot = Rect::new(menu.x + 5.0, cy - 1.0, 2.6, 2.6);
            cmds.push(Cmd::Circles {
                items: vec![
                    [dot.x, dot.y, dot.w],
                    [dot.x + 6.0, dot.y, dot.w],
                    [dot.x + 12.0, dot.y, dot.w],
                ],
                color: control,
            });
            if let Some(buf) = self.editing_column(i).map(str::to_string) {
                self.caret(
                    cmds,
                    Rect::new(fold.x + fold.w + 6.0, top + 9.0, name_w, head_style.line_height),
                    &buf,
                    &head_style,
                    c[5],
                    fonts,
                );
            }
            // The cards, each one painted whole so a dropped card can fade in on its own.
            for cg in &cards {
                let Some(card) = self.board.card(cg.id).cloned() else { continue };
                let mut own = Vec::new();
                self.paint_card(&mut own, &card, cg, c, fonts, card_bg);
                if self.drop == Some(card.id) && fade < 1.0 {
                    for cmd in &mut own {
                        fade_cmd(cmd, fade);
                    }
                }
                cmds.append(&mut own);
            }
            if !col.collapsed {
                // The `+` at the foot of the column: it takes the accent when it is under the
                // pointer, which is the only affordance a painted button can give before a press.
                let over = matches!(self.hover, Hover::Add(j) if j == i);
                cmds.push(fill(
                    add,
                    if over { mix(col_bg, c[5], 0.22) } else { mix(col_bg, c[5], 0.10) },
                    6.0,
                ));
                cmds.push(Cmd::Text {
                    x: add.x + 10.0,
                    y: add.y + 6.0,
                    width: Some(add.w - 20.0),
                    text: "+ Añadir tarjeta".to_string(),
                    spans: Vec::new(),
                    style: TextStyle {
                        color: if over { c[2] } else { c[3] },
                        ..count_style.clone()
                    },
                });
            }
            geom.push(ColGeom { rect: rect_col, head, fold, menu, add, cards });
        }
        if self.board.columns.is_empty() {
            cmds.push(Cmd::Text {
                x: rect.x + 24.0,
                y: rect.y + rect.h / 2.0 - 10.0,
                width: Some((rect.w - 48.0).max(80.0)),
                text: "Este tablero no tiene columnas: «Nueva columna» empieza uno".to_string(),
                spans: Vec::new(),
                style: count_style,
            });
        }
        self.geom = geom;
        let n = self.board.columns.len() as f32;
        self.content = if n == 0.0 {
            (rect.w, rect.h)
        } else {
            (EDGE * 2.0 + n * COL_W + (n - 1.0) * GAP, (bottom - rect.y) + EDGE)
        };
        self.clamp_scroll();
    }

    /// Lay a card out and answer its geometry. Its height is what its wrapped text takes, so a
    /// long card grows and never spills out of its own box.
    fn card_geom(&self, card: &Card, x: f32, y: f32, c: &[Color; 12], fonts: &mut Fonts) -> CardGeom {
        let style = TextStyle { color: if card.done { c[3] } else { c[2] }, ..self.card_style.clone() };
        let (spans, text) = match self.editing_card(card.id) {
            Some(buf) => (Vec::new(), buf.to_string()),
            None => {
                let chips = chip_ranges(&card.text);
                (card_spans(&card.text, &chips, c, card.done), card.text.clone())
            }
        };
        let check = Rect::new(x + INSET + CARD_PAD, y + CARD_PAD + 1.0, CHECK, CHECK);
        let tx = check.x + CHECK + 8.0;
        let tw = (x + COL_W - INSET - CARD_PAD - tx).max(40.0);
        let (_, th) = if text.is_empty() {
            (0.0, style.line_height)
        } else {
            fonts.measure(&text, &spans, &style, Some(tw))
        };
        let h = (th + 2.0 * CARD_PAD).max(MIN_CARD_H);
        CardGeom {
            id: card.id,
            rect: Rect::new(x + INSET, y, COL_W - 2.0 * INSET, h),
            check,
            text: Rect::new(tx, y + CARD_PAD, tw, th.max(style.line_height)),
        }
    }

    fn paint_card(
        &mut self,
        cmds: &mut Vec<Cmd>,
        card: &Card,
        g: &CardGeom,
        c: &[Color; 12],
        fonts: &mut Fonts,
        card_bg: Color,
    ) {
        let selected = self.selected == Some(card.id);
        let dragging = self.dragging == Some(card.id);
        let editing = self.editing_card(card.id).is_some();
        let hovered = matches!(self.hover, Hover::Card(id) | Hover::Check(id) if id == card.id);
        let border = if selected || dragging {
            c[5]
        } else if hovered {
            mix(c[1], c[2], 0.45)
        } else {
            c[1]
        };
        let bg = if selected || dragging {
            mix(c[0], c[5], 0.10)
        } else if hovered {
            mix(c[0], c[2], 0.06)
        } else {
            card_bg
        };
        let r = g.rect;
        cmds.push(fill(r, border, 7.0));
        let bw = if selected || dragging { 2.0 } else { 1.0 };
        cmds.push(fill(Rect::new(r.x + bw, r.y + bw, r.w - 2.0 * bw, r.h - 2.0 * bw), bg, (7.0 - bw).max(0.0)));
        // The checkbox: a ring to fill, or an accent disc with a tick.
        let ck = g.check;
        let mid = (ck.x + ck.w / 2.0, ck.y + ck.h / 2.0);
        if card.done {
            cmds.push(Cmd::Circles { items: vec![[mid.0, mid.1, ck.w / 2.0]], color: c[5] });
            cmds.push(Cmd::Lines {
                segs: vec![
                    [ck.x + 3.4, ck.y + 7.8, ck.x + 6.3, ck.y + 10.9],
                    [ck.x + 6.3, ck.y + 10.9, ck.x + 11.8, ck.y + 4.2],
                ],
                width: 1.8,
                color: c[0],
            });
        } else {
            let ring = if matches!(self.hover, Hover::Check(id)| Hover::Card(id) if id == card.id) {
                c[5]
            } else {
                c[3]
            };
            cmds.push(Cmd::Circles { items: vec![[mid.0, mid.1, ck.w / 2.0]], color: ring });
            cmds.push(Cmd::Circles { items: vec![[mid.0, mid.1, ck.w / 2.0 - 1.4]], color: bg });
        }
        let style = TextStyle { color: if card.done { c[3] } else { c[2] }, ..self.card_style.clone() };
        let (text, spans) = match self.editing_card(card.id) {
            Some(buf) => (buf.to_string(), Vec::new()),
            None => {
                let chips = chip_ranges(&card.text);
                (card.text.clone(), card_spans(&card.text, &chips, c, card.done))
            }
        };
        if !editing {
            // Tags and dates wear a chip behind them, laid out where the text really put them.
            for (s, e, chip) in chip_ranges(&text) {
                if !matches!(chip, Chip::Tag | Chip::Date) {
                    continue;
                }
                for rect in fonts.range_rects(&text, &spans, &style, Some(g.text.w), s, e) {
                    let h = (rect.h - 3.0).max(8.0);
                    cmds.push(fill(
                        Rect::new(
                            g.text.x + rect.x - 3.0,
                            g.text.y + rect.y + 1.5,
                            rect.w + 6.0,
                            h,
                        ),
                        c[5].with_alpha(0.16),
                        h / 2.0,
                    ));
                }
            }
        }
        if !text.is_empty() {
            cmds.push(Cmd::Text {
                x: g.text.x,
                y: g.text.y,
                width: Some(g.text.w),
                text: text.clone(),
                spans,
                style: style.clone(),
            });
        }
        if let Some(buf) = self.editing_card(card.id).map(str::to_string) {
            self.caret(cmds, g.text, &buf, &style, c[5], fonts);
        }
    }

    /// The caret of a text being typed, at the end of what was typed and where it wrapped. `box_`
    /// is where the text starts and how wide it wraps; `accent` is the theme's cursor colour.
    fn caret(
        &self,
        cmds: &mut Vec<Cmd>,
        box_: Rect,
        buf: &str,
        style: &TextStyle,
        accent: Color,
        fonts: &mut Fonts,
    ) {
        let at = fonts.range_rects(buf, &[], style, Some(box_.w), buf.len(), buf.len());
        let Some(r) = at.last() else { return };
        cmds.push(fill(
            Rect::new(box_.x + r.x, box_.y + r.y, 1.5, r.h.max(style.line_height)),
            accent,
            0.0,
        ));
    }
}

// ---- text of a card ----

#[derive(Clone, Debug, PartialEq)]
enum Chip {
    Tag,
    /// A `@{2026-11-25}` date.
    Date,
    Link(String),
}

/// The tags, the dates and the links of a card, as byte ranges.
fn chip_ranges(text: &str) -> Vec<(usize, usize, Chip)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'[' && text[i..].starts_with("[[") {
            if let Some(rel) = text[i + 2..].find("]]") {
                let end = i + 2 + rel + 2;
                let body = &text[i + 2..end - 2];
                let target = body.split('|').next().unwrap_or(body).trim().to_string();
                out.push((i, end, Chip::Link(target)));
                i = end;
                continue;
            }
        }
        if b == b'@' && text[i..].starts_with("@{") {
            if let Some(rel) = text[i + 2..].find('}') {
                let end = i + 2 + rel + 1;
                out.push((i, end, Chip::Date));
                i = end;
                continue;
            }
        }
        if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'-' | b'_' | b'/'))
            {
                j += 1;
            }
            if j > i + 1 {
                out.push((i, j, Chip::Tag));
                i = j;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// The spans a card paints with: a done card is faint and struck through, a tag or a date is
/// accented, and a link is accented and underlined.
fn card_spans(text: &str, chips: &[(usize, usize, Chip)], c: &[Color; 12], done: bool) -> Vec<Span> {
    let mut spans = Vec::new();
    if done {
        spans.push(Span {
            start: 0,
            end: text.len(),
            color: Some(c[3]),
            strike: true,
            scale: 1.0,
            ..Span::default()
        });
    }
    for (start, end, chip) in chips {
        let underline = matches!(chip, Chip::Link(_));
        spans.push(Span {
            start: *start,
            end: *end,
            color: Some(c[5]),
            underline,
            strike: done,
            scale: 1.0,
            ..Span::default()
        });
    }
    spans
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: 255 }
}

fn fill(rect: Rect, color: Color, radius: f32) -> Cmd {
    Cmd::Fill { rect, color, radius }
}

/// Scale the alpha of every colour a command paints: how the card just dropped fades in.
fn fade_cmd(cmd: &mut Cmd, t: f32) {
    let t = t.clamp(0.0, 1.0);
    let fade = |c: &mut Color| c.a = (f32::from(c.a) * t).round().clamp(0.0, 255.0) as u8;
    match cmd {
        Cmd::Fill { color, .. } | Cmd::Lines { color, .. } | Cmd::Circles { color, .. } => fade(color),
        Cmd::Text { style, spans, .. } => {
            fade(&mut style.color);
            for s in spans {
                if let Some(c) = &mut s.color {
                    fade(c);
                }
            }
        }
        Cmd::Image { .. } | Cmd::PushClip(_) | Cmd::PopClip => {}
    }
}

// ---- the pane ----

/// The header of the board and the node its paint pass draws into.
pub fn pane(v: &KanbanView, dom: &mut Dom, parent: NodeId, info: &mut crate::view::ViewInfo) {
    let header = dom.add(parent, El::div().class("view-header graph-controls kanban-controls"));
    dom.add(
        header,
        El::div().class("graph-count").text(format!(
            "{}  ·  {} columnas  ·  {} tarjetas",
            v.path,
            v.board.columns.len(),
            v.board.cards()
        )),
    );
    let source = if v.show_source { "Ver tablero" } else { "Ver código" };
    for (label, cmd) in
        [("Nueva columna", "kanban:add-column"), ("Nueva tarjeta", "kanban:add-card"), (source, "kanban:source")]
    {
        dom.add(header, El::div().class("graph-control").text(label).action(cmd));
    }
    let leaf = dom.add(parent, El::div().class("view-content kanban-wrapper"));
    let colors = dom.add(leaf, El::div().class("kanban-view-colors"));
    info.kanban_colors =
        COLOR_CLASSES.iter().map(|k| dom.add(colors, El::div().class("kanban-view").class(k))).collect();
    info.kanban = Some(leaf);
}

/// After layout: paint the board into its pane, and fade in the card just dropped.
pub fn add_kanban(app: &mut App, dom: &mut Dom, info: &crate::view::ViewInfo, fonts: &mut Fonts) {
    let Some(pane_node) = info.kanban else { return };
    let mut colors = [Color::rgb(128, 128, 128); 12];
    for (slot, &n) in colors.iter_mut().zip(&info.kanban_colors) {
        let s = &dom.nodes[n].style;
        *slot = s.text.color.with_alpha(s.opacity);
    }
    let text = dom.nodes[pane_node].style.text.clone();
    let rect = dom.nodes[pane_node].rect;
    // The card just dropped fades in with the editor's clock; with reduced motion the clock is
    // off and the card is simply there.
    let fresh = app.kanban_view.as_ref().is_some_and(|v| v.drop.is_some());
    let started = app.kanban_view.as_ref().is_some_and(|v| v.drop_started);
    let mut fade = 1.0;
    if fresh {
        if !started {
            app.anims.start("kanban-drop", DROP_MS);
            if let Some(v) = &mut app.kanban_view {
                v.drop_started = true;
            }
        }
        fade = app.anims.t("kanban-drop");
    }
    let Some(v) = &mut app.kanban_view else { return };
    dom.nodes[pane_node].draw = v.paint(rect, &colors, &text, fonts, fade);
    if v.drop.is_some() && fade >= 1.0 {
        v.drop = None;
        v.drop_started = false;
    }
}

// ---- the app ----

impl App {
    pub fn kanban_active(&self) -> bool {
        self.show_kanban
            && self.kanban_view.is_some()
            && !self.show_graph
            && !self.show_canvas
            && !self.show_base
            && !self.show_exc
    }

    /// Show a board read from disk.
    pub(crate) fn open_board(&mut self, rel: &str, text: &str, stamp: Stamp) {
        if let Some(v) = &mut self.kanban_view {
            v.stop_edit();
        }
        self.kanban_save();
        self.kanban_view = Some(KanbanView::open(rel, text, stamp));
        self.show_kanban = true;
        self.show_canvas = false;
        self.show_base = false;
        self.show_graph = false;
        self.show_exc = false;
        self.exc_fullscreen = false;
        self.focus = Focus::Editor;
    }

    pub(crate) fn open_kanban(&mut self, rel: &str) {
        let same = self.kanban_view.as_ref().is_some_and(|v| v.path.eq_ignore_ascii_case(rel));
        if same && self.show_kanban {
            self.focus = Focus::Editor;
            return;
        }
        match self.store.read(rel) {
            Ok((text, stamp)) => self.open_board(rel, &text, stamp),
            Err(e) => self.notice = Some(format!("No se pudo abrir {rel}: {e}")),
        }
    }

    /// A board created from the palette: the plugin's template in the folder new notes go to.
    fn new_board(&mut self) {
        let dir = self.new_note_dir();
        let rel = (1..)
            .map(|n| {
                let name = if n == 1 { "Tablero.md".to_string() } else { format!("Tablero {n}.md") };
                path::join(&dir, &name)
            })
            .find(|p| self.store.index.id(p).is_none() && !self.store.abs(p).exists())
            .unwrap_or_else(|| path::join(&dir, "Tablero.md"));
        match self.store.create(&rel, TEMPLATE) {
            Ok(_) => {
                self.plugins.push(ext_api::Event::Create { path: rel.clone() });
                self.files_changed(&[]);
                self.open_kanban(&rel);
                self.notice = Some(format!("Tablero creado: {rel}"));
            }
            Err(e) => self.notice = Some(format!("No se pudo crear el tablero: {e}")),
        }
    }

    /// Write the board, when there is something to write.
    pub(crate) fn kanban_save(&mut self) {
        let Some(v) = self.kanban_view.as_mut().filter(|v| v.dirty) else { return };
        v.stop_edit();
        let text = v.to_markdown();
        match self.store.write(&v.path, &text, Some(v.stamp)) {
            Ok(stamp) => {
                v.stamp = stamp;
                v.dirty = false;
            }
            Err(WriteError::Conflict { .. }) => {
                self.notice = Some(format!("{} se cambió fuera de MetaMind. No se guardó.", v.path))
            }
            Err(WriteError::Io(e)) => self.notice = Some(format!("No se pudo guardar: {e}")),
        }
    }

    /// Close the board, writing it first.
    pub(crate) fn kanban_close(&mut self) {
        if let Some(v) = &mut self.kanban_view {
            v.stop_edit();
        }
        self.kanban_save();
        self.kanban_view = None;
        self.show_kanban = false;
    }

    /// React to the file changing on disk: a board with no unsaved edit is read again.
    pub(crate) fn kanban_files_changed(&mut self, paths: &[String]) {
        let Some(v) = &mut self.kanban_view else { return };
        if !paths.iter().any(|p| p.eq_ignore_ascii_case(&v.path)) {
            return;
        }
        let Ok((text, stamp)) = self.store.read(&v.path) else { return };
        if v.dirty {
            if text != v.to_markdown() {
                self.notice =
                    Some(format!("{} cambió fuera de MetaMind; tus cambios siguen en la pestaña", v.path));
            }
            return;
        }
        if text != v.to_markdown() {
            v.board = Board::parse(&text);
            v.editing = None;
            v.selected = None;
            self.notice = Some(format!("{} cambió en disco: se recargó", v.path));
        }
        v.stamp = stamp;
    }

    fn kanban_rect(&self, frame: &crate::Frame) -> Option<Rect> {
        if !self.kanban_active() {
            return None;
        }
        frame.info.kanban.map(|n| frame.dom.nodes[n].rect)
    }

    pub fn kanban_click(&mut self, frame: &crate::Frame, fonts: &mut Fonts, x: f32, y: f32) -> bool {
        let Some(rect) = self.kanban_rect(frame) else { return false };
        if !rect.contains(x, y) {
            return false;
        }
        let Some(v) = &mut self.kanban_view else { return false };
        let hit = v.press(rect, x, y, fonts);
        let anchor = match &hit {
            Hit::ColumnMenu(i) => Some(v.column_anchor(*i)),
            _ => None,
        };
        // A press that starts typing is not a command to save: writing now would commit the box
        // the press just opened. Everything else writes at once.
        let typing = v.is_editing();
        if !typing {
            self.kanban_save();
        }
        match hit {
            Hit::ColumnMenu(i) => {
                let (mx, my) = anchor.unwrap_or((x, y));
                self.kanban_column_menu(i, mx, my);
            }
            Hit::Link(target) => self.open_note(&target),
            _ => {}
        }
        true
    }

    pub fn kanban_double_click(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(rect) = self.kanban_rect(frame) else { return false };
        if !rect.contains(x, y) {
            return false;
        }
        let Some(v) = &mut self.kanban_view else { return false };
        if v.show_source {
            v.show_source = false;
            return true;
        }
        v.double_click(rect, x, y)
    }

    pub fn kanban_drag(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(_rect) = self.kanban_rect(frame) else { return false };
        let Some(v) = &mut self.kanban_view else { return false };
        v.drag(x, y);
        true
    }

    /// The button came up: a card that was dragged is written to disk right away.
    pub fn kanban_release(&mut self) {
        let dropped = self.kanban_view.as_mut().is_some_and(|v| v.release());
        if dropped {
            self.anims.start("kanban-drop", DROP_MS);
            self.kanban_save();
        }
    }

    /// Wheel over the board: down scrolls the cards, sideways (Shift or Ctrl) walks the columns.
    /// Track the pointer over the board: the card, checkbox or column under it lights up. True
    /// when that changed, so the window redraws.
    pub fn kanban_hover(&mut self, x: f32, y: f32) -> bool {
        if !self.kanban_active() {
            return false;
        }
        let Some(v) = &mut self.kanban_view else { return false };
        v.hover(x, y)
    }

    pub fn kanban_wheel(&mut self, frame: &crate::Frame, x: f32, y: f32, lines: f32, sideways: bool) -> bool {
        let Some(rect) = self.kanban_rect(frame) else { return false };
        let Some(v) = &mut self.kanban_view else { return false };
        v.wheel(rect, x, y, lines, sideways)
    }

    /// Keys the board takes while it is the open view.
    pub(crate) fn kanban_key(&mut self, key: &crate::Key) -> bool {
        let editing = self.kanban_view.as_ref().is_some_and(|v| v.editing.is_some());
        if editing {
            match key.key.as_str() {
                "Escape" | "Enter" | "Tab" => {
                    if let Some(v) = &mut self.kanban_view {
                        let changed = v.stop_edit();
                        v.dirty |= changed;
                    }
                    self.kanban_save();
                }
                "Backspace" => {
                    if let Some(e) = self.kanban_view.as_mut().and_then(|v| v.editing.as_mut()) {
                        e.buf.pop();
                    }
                }
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End" => {}
                _ => return false,
            }
            return true;
        }
        match key.key.as_str() {
            "Delete" | "Backspace" => {
                self.run("kanban:delete-card");
            }
            "Enter" if key.ctrl => {
                self.run("kanban:toggle-done");
            }
            "Enter" => {
                self.run("kanban:edit-card");
            }
            "Escape" => {
                if let Some(v) = &mut self.kanban_view {
                    v.selected = None;
                    v.show_source = false;
                }
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn kanban_text(&mut self, text: &str) {
        if let Some(buf) = self.kanban_view.as_mut().and_then(|v| v.editing.as_mut().map(|e| &mut e.buf)) {
            buf.push_str(text);
        }
    }

    /// The board's own commands, `kanban:*`. True when the command was one of them.
    pub(crate) fn run_kanban(&mut self, command: &str) -> bool {
        let Some(rest) = command.strip_prefix("kanban:") else { return false };
        if rest == "new" {
            self.new_board();
            return true;
        }
        if self.kanban_view.is_none() {
            // A board command with no board open has nothing to do.
            return true;
        }
        let (word, arg) = match rest.split_once(':') {
            Some((w, a)) => (w, a.parse::<usize>().ok()),
            None => (rest, None),
        };
        match word {
            "focus" => {
                self.show_kanban = true;
                self.show_canvas = false;
                self.show_base = false;
                self.show_graph = false;
                self.show_exc = false;
                self.focus = Focus::Editor;
            }
            "close" => self.kanban_close(),
            "source" => {
                if let Some(v) = &mut self.kanban_view {
                    v.show_source = !v.show_source;
                }
            }
            "add-column" => {
                let i = {
                    let Some(v) = &mut self.kanban_view else { return true };
                    let i = v.board.add_column("Nueva columna");
                    v.dirty = true;
                    v.start_column(i, true);
                    i
                };
                if let Some(v) = &mut self.kanban_view {
                    v.scroll_to(i);
                }
            }
            "add-card" => {
                let col = self
                    .kanban_view
                    .as_ref()
                    .and_then(|v| v.selected.and_then(|id| v.board.locate(id)).map(|(c, _)| c))
                    .or_else(|| self.kanban_view.as_ref().map(|v| v.board.columns.len().saturating_sub(1)));
                let Some(col) = col else { return true };
                if let Some(v) = &mut self.kanban_view {
                    if v.board.columns.is_empty() {
                        return true;
                    }
                    v.add_card(col);
                }
            }
            "add-card-at" => {
                if let Some(i) = arg {
                    if let Some(v) = &mut self.kanban_view {
                        v.add_card(i);
                    }
                }
            }
            "edit-card" => {
                let Some(v) = &mut self.kanban_view else { return true };
                if let Some(id) = v.selected {
                    v.start_card(id, false);
                }
            }
            "delete-card" => {
                let gone = self.kanban_view.as_mut().and_then(|v| {
                    v.stop_edit();
                    let id = v.selected.take()?;
                    let card = v.board.remove_card(id)?;
                    v.dirty = true;
                    Some(card)
                });
                if let Some(card) = gone {
                    self.notice = Some(format!("Tarjeta eliminada: {}", card.text));
                }
                self.kanban_save();
            }
            "toggle-done" => {
                if let Some(v) = &mut self.kanban_view {
                    if let Some(id) = v.selected {
                        if let Some(card) = v.board.card_mut(id) {
                            card.done = !card.done;
                            v.dirty = true;
                        }
                    }
                }
                self.kanban_save();
            }
            "toggle-column" => {
                if let Some(i) = arg {
                    if let Some(v) = &mut self.kanban_view {
                        if let Some(col) = v.board.columns.get_mut(i) {
                            col.collapsed = !col.collapsed;
                        }
                    }
                }
            }
            "column-menu" => {
                if let Some(i) = arg {
                    let (x, y) = self
                        .kanban_view
                        .as_ref()
                        .map(|v| v.column_anchor(i))
                        .unwrap_or((0.0, 0.0));
                    self.kanban_column_menu(i, x, y);
                }
            }
            "column-rename" => {
                if let Some(i) = arg {
                    if let Some(v) = &mut self.kanban_view {
                        v.start_column(i, false);
                    }
                }
            }
            "column-fold" => {
                self.run_kanban(&match arg {
                    Some(i) => format!("kanban:toggle-column:{i}"),
                    None => "kanban:toggle-column".to_string(),
                });
            }
            "column-left" => {
                if let Some(i) = arg {
                    if let Some(v) = &mut self.kanban_view {
                        v.dirty |= v.board.move_column(i, false);
                    }
                    self.kanban_save();
                }
            }
            "column-right" => {
                if let Some(i) = arg {
                    if let Some(v) = &mut self.kanban_view {
                        v.dirty |= v.board.move_column(i, true);
                    }
                    self.kanban_save();
                }
            }
            "column-delete" => {
                let gone = self.kanban_view.as_mut().and_then(|v| {
                    if arg.is_some_and(|i| i >= v.board.columns.len()) {
                        return None;
                    }
                    let col = v.board.columns.remove(arg?);
                    v.dirty = true;
                    v.selected = None;
                    Some(col)
                });
                if let Some(col) = gone {
                    self.notice = Some(if col.cards.is_empty() {
                        format!("Columna eliminada: {}", col.name)
                    } else {
                        format!("Columna eliminada: {} y sus {} tarjetas", col.name, col.cards.len())
                    });
                }
                self.kanban_save();
            }
            _ => return false,
        }
        true
    }

    /// The menu of a column: rename it, fold it, move it, empty it or take it away.
    pub(crate) fn kanban_column_menu(&mut self, i: usize, x: f32, y: f32) {
        let Some(v) = self.kanban_view.as_ref() else { return };
        let Some(col) = v.board.columns.get(i) else { return };
        let fold = if col.collapsed { "Expandir columna" } else { "Colapsar columna" };
        let items = vec![
            self.kanban_item("plus", "Añadir tarjeta", format!("kanban:add-card-at:{i}"), false),
            self.kanban_item("edit", "Renombrar columna", format!("kanban:column-rename:{i}"), false),
            self.kanban_item("collapse", fold, format!("kanban:column-fold:{i}"), false),
            crate::menu::MenuItem {
                label: String::new(),
                icon: "",
                action: String::new(),
                hotkey: String::new(),
                warning: false,
            },
            self.kanban_item("arrow-left", "Mover a la izquierda", format!("kanban:column-left:{i}"), false),
            self.kanban_item("arrow-right", "Mover a la derecha", format!("kanban:column-right:{i}"), false),
            crate::menu::MenuItem {
                label: String::new(),
                icon: "",
                action: String::new(),
                hotkey: String::new(),
                warning: false,
            },
            self.kanban_item("trash", "Eliminar columna", format!("kanban:column-delete:{i}"), true),
        ];
        self.show_menu(x, y, items);
    }

    fn kanban_item(
        &self,
        icon: &'static str,
        label: &str,
        action: String,
        warning: bool,
    ) -> crate::menu::MenuItem {
        crate::menu::MenuItem { label: label.to_string(), icon, hotkey: self.hotkey_for(&action), action, warning }
    }

    /// The right-click menu over the board: the column under the pointer, or the board's own.
    pub fn kanban_context_menu(&mut self, x: f32, y: f32) {
        let col = self.kanban_view.as_ref().and_then(|v| v.column_at(x, y));
        match col {
            Some(i) => self.kanban_column_menu(i, x, y),
            None => {
                let items = vec![
                    self.kanban_item("plus", "Nueva columna", "kanban:add-column".to_string(), false),
                    self.kanban_item("code", "Ver código", "kanban:source".to_string(), false),
                ];
                self.show_menu(x, y, items);
            }
        }
    }
}
