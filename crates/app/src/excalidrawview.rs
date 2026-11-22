//! Excalidraw boards opened in the main pane: a `.excalidraw` file shown as a board you can draw
//! on, next to the read-only `![[Tablero.excalidraw]]` embed in the note.
//!
//! The scene model, the display list and the writer are `excalidraw-core`'s; this module owns what
//! a viewer needs and a file does not: the camera (a centre in scene coordinates plus a zoom), the
//! tool, the pointer state and the text box being typed into. It has the shape of the canvas view
//! (`canvasview.rs`) — the app's two infinite boards — and paints through the painter of
//! `excalidraw.rs`, so a board looks the same in its note and in its tab.
//!
//! # Framing
//!
//! The first paint fits the elements, not the origin: the camera goes to the middle of
//! `Scene::bounds` and the zoom to what fits that box with a margin, so a board whose first
//! element sits at `x: 40, y: 60` is neither clipped at the top nor floating in a corner. A board
//! with nothing on it starts at the origin, at 100 %, ready to draw. The fit is applied only while
//! [`ExcView::auto_fit`] is set: panning, zooming or drawing clears it and *Ajustar* sets it again.
//!
//! # The surface
//!
//! Excalidraw saves `appState.viewBackgroundColor`, and it is almost always pure white. On a dark
//! theme that page is what dazzles, so a *white* page is painted in the theme's surface colour with
//! a faint dot grid on it; a page the user coloured is painted as it is, in either theme. On a dark
//! surface the near-black default stroke of every Excalidraw file would vanish, so the painter
//! swaps the light and dark ends of a colour there (see `excalidraw::Pen`).
//!
//! # What a board saves
//!
//! `Ctrl+S` and leaving the view write `Scene::to_json`, the schema `Scene::parse` reads, with the
//! keys Excalidraw writes for a new element (`version`, `versionNonce`, `updated`) filled in by
//! `new_element` and `bump`. A text box is the one thing written by approximation: its width is
//! measured while it is being typed (the paint pass is the only place holding the font) and
//! estimated from the character count otherwise.
//!
//! # Not covered
//!
//! Rotation, grouping, arrow bindings, sketchy strokes and image decoding are the limits of
//! `excalidraw-core` and of the painter it feeds; the toolbar has no frame, embed or image tool,
//! undo is the editor's (the board writes as it goes), and a selection is one element at a time.

use crate::excalidraw::{paint_op, Pen};
use crate::shell::{App, Focus};
use excalidraw_core::{new_element, ElementType, Rgba, Scene};
use metamind_ui::{Cmd, Dom, El, Fonts, NodeId, Rect, TextStyle};
use theme_engine::Color;
use vault_core::fsio::{self, Stamp};
use vault_core::{path, WriteError};

/// Hidden elements the board colours are read from: the same twelve the canvas view reads.
pub const COLOR_CLASSES: [&str; 12] = crate::canvasview::COLOR_CLASSES;

/// The eight stroke colours of the toolbar, in Excalidraw's own palette.
pub const STROKES: [&str; 8] =
    ["#1e1e1e", "#e03131", "#2f9e44", "#1971c2", "#f08c00", "#9c36b5", "#0c8599", "#868e96"];
/// The eight background colours; the first is Excalidraw's "no fill".
pub const FILLS: [&str; 8] =
    ["transparent", "#ffc9c9", "#b2f2bb", "#a5d8ff", "#ffec99", "#eebefa", "#c3fae8", "#e9ecef"];
/// The three stroke widths the toolbar offers.
pub const WIDTHS: [f64; 3] = [1.0, 2.0, 4.0];

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 8.0;
/// Room kept around the board by *Ajustar*, so no element touches the edge of the pane.
const FIT_MARGIN: f32 = 48.0;
/// Scene units between two dots of the grid.
const GRID: f64 = 20.0;
/// The grid coarsens instead of painting more dots than this.
const MAX_DOTS: f32 = 12_000.0;
/// Screen pixels a wheel line scrolls the board by.
const WHEEL_PAN: f32 = 42.0;
/// How far the pointer travels before a press becomes a drag.
const DRAG_START: f32 = 3.0;
/// Smaller than this in both directions, a shape drawn by accident is dropped on release.
const MIN_SIZE: f64 = 2.0;
/// Advance of the text font, as a fraction of its size, for a box nobody has measured.
const CHAR_W: f64 = 0.55;

/// The drawing tools of the left toolbar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Rect,
    Ellipse,
    Diamond,
    Arrow,
    Line,
    Freedraw,
    Text,
}

impl Tool {
    /// The toolbar, top to bottom.
    pub const ALL: [Tool; 8] = [
        Tool::Select,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Diamond,
        Tool::Arrow,
        Tool::Line,
        Tool::Freedraw,
        Tool::Text,
    ];

    /// The name in the `exc:tool:<id>` action.
    pub fn id(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Diamond => "diamond",
            Tool::Arrow => "arrow",
            Tool::Line => "line",
            Tool::Freedraw => "freedraw",
            Tool::Text => "text",
        }
    }

    pub fn from_id(id: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Rect => "square",
            Tool::Ellipse => "circle",
            Tool::Diamond => "diamond",
            Tool::Arrow => "arrow-up-right",
            Tool::Line => "minus",
            Tool::Freedraw => "edit",
            Tool::Text => "type",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Seleccionar",
            Tool::Rect => "Rectángulo",
            Tool::Ellipse => "Elipse",
            Tool::Diamond => "Rombo",
            Tool::Arrow => "Flecha",
            Tool::Line => "Línea",
            Tool::Freedraw => "Trazo libre",
            Tool::Text => "Texto",
        }
    }

    /// The element this tool draws, if it draws one.
    fn kind(self) -> Option<ElementType> {
        Some(match self {
            Tool::Select | Tool::Text => return None,
            Tool::Rect => ElementType::Rectangle,
            Tool::Ellipse => ElementType::Ellipse,
            Tool::Diamond => ElementType::Diamond,
            Tool::Arrow => ElementType::Arrow,
            Tool::Line => ElementType::Line,
            Tool::Freedraw => ElementType::Freedraw,
        })
    }
}

enum Drag {
    /// Space held, or the select tool on empty space: the board follows the pointer.
    Pan { last: (f32, f32) },
    /// Moving the selected element.
    Move { id: String, last: [f64; 2] },
    /// Drawing a new shape, from the point the press started at.
    Draw { id: String, from: [f64; 2] },
    /// Adding points to a freedraw stroke.
    Free { id: String },
    /// A press that does nothing until the second half of a double click.
    Idle,
}

struct Press {
    drag: Drag,
    at: (f32, f32),
    moved: bool,
}

/// A `.excalidraw` board opened as a tab.
pub struct ExcView {
    pub path: String,
    pub scene: Scene,
    stamp: Stamp,
    /// The scene point shown at the centre of the pane.
    pub cam: [f32; 2],
    pub zoom: f32,
    /// The camera follows the board until the user pans, zooms or draws.
    pub auto_fit: bool,
    pub tool: Tool,
    /// `#rrggbb` of the stroke the next element gets.
    pub stroke: String,
    /// `#rrggbb`, or `transparent` for no fill.
    pub fill: String,
    pub width: f64,
    pub selected: Option<String>,
    /// The text element being typed into.
    pub editing: Option<String>,
    /// Its text, as typed.
    buf: String,
    /// The element being typed into was created by this press: escaping it leaves no element.
    fresh: bool,
    /// The space bar is held: a press pans the board instead of drawing.
    space_held: bool,
    pub dirty: bool,
    press: Option<Press>,
    seed: u64,
}

impl ExcView {
    /// Read a board. The error is what the tab shows instead of the board.
    pub fn open(path: &str, text: &str, stamp: Stamp) -> Result<ExcView, String> {
        Ok(ExcView {
            path: path.to_string(),
            scene: Scene::parse(text).map_err(|e| e.to_string())?,
            stamp,
            cam: [0.0; 2],
            zoom: 1.0,
            auto_fit: true,
            tool: Tool::Select,
            stroke: STROKES[0].to_string(),
            fill: FILLS[0].to_string(),
            width: WIDTHS[1],
            selected: None,
            editing: None,
            buf: String::new(),
            fresh: false,
            space_held: false,
            dirty: false,
            press: None,
            seed: fsio::now_ms() | 1,
        })
    }

    pub fn title(&self) -> String {
        path::stem(&self.path).to_string()
    }

    /// The elements the board paints, for the header's count.
    pub fn count(&self) -> usize {
        self.scene.elements.iter().filter(|e| e.painted()).count()
    }

    /// What the board could not show, in one line, or `None` when it painted everything.
    pub fn skipped_note(&self) -> Option<String> {
        crate::excalidraw::skipped_note(&self.scene.display_list())
    }

    pub fn to_world(&self, rect: &Rect, x: f32, y: f32) -> [f64; 2] {
        [
            (self.cam[0] + (x - rect.x - rect.w / 2.0) / self.zoom) as f64,
            (self.cam[1] + (y - rect.y - rect.h / 2.0) / self.zoom) as f64,
        ]
    }

    pub fn to_screen(&self, rect: &Rect, x: f64, y: f64) -> (f32, f32) {
        (
            rect.x + rect.w / 2.0 + (x as f32 - self.cam[0]) * self.zoom,
            rect.y + rect.h / 2.0 + (y as f32 - self.cam[1]) * self.zoom,
        )
    }

    /// Put the elements in the middle of `rect`, with a margin around them.
    fn fit(&mut self, rect: &Rect) {
        if !self.auto_fit {
            return;
        }
        match self.scene.bounds() {
            Some(b) => {
                let zx = (rect.w - 2.0 * FIT_MARGIN) / (b.width as f32).max(1.0);
                let zy = (rect.h - 2.0 * FIT_MARGIN) / (b.height as f32).max(1.0);
                // Fit shrinks a wide board, never enlarges a small one: at 100 % the text is
                // crisp, and `+` is there for anyone who wants it bigger.
                self.zoom = zx.min(zy).clamp(MIN_ZOOM, 1.0);
                self.cam = [(b.x + b.width / 2.0) as f32, (b.y + b.height / 2.0) as f32];
            }
            None => {
                // An empty board: the origin in the middle, ready for the first shape.
                self.zoom = 1.0;
                self.cam = [0.0; 2];
            }
        }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.auto_fit = false;
    }

    /// Zoom back to 100 %, keeping the middle of the pane where it is.
    pub fn zoom_reset(&mut self) {
        self.zoom = 1.0;
        self.auto_fit = false;
    }

    /// Ask for the camera to follow the board again: the next paint fits it.
    pub fn zoom_fit(&mut self) {
        self.auto_fit = true;
    }

    /// Space bar held: a press pans instead of drawing.
    pub fn set_space(&mut self, on: bool) {
        self.space_held = on;
    }

    /// The middle button grabbed the board: it follows the pointer until the button is released.
    pub fn start_pan(&mut self, x: f32, y: f32) {
        self.press = Some(Press { drag: Drag::Pan { last: (x, y) }, at: (x, y), moved: false });
    }

    /// Whether the pointer is panning the board.
    pub fn panning(&self) -> bool {
        matches!(self.press.as_ref().map(|p| &p.drag), Some(Drag::Pan { .. }))
    }

    // ---- text ----

    fn start_edit(&mut self, id: &str, fresh: bool) {
        let Some(e) = self.scene.element(id) else { return };
        if !matches!(e.kind, ElementType::Text) {
            return;
        }
        self.buf = e.text();
        self.editing = Some(id.to_string());
        self.selected = Some(id.to_string());
        self.fresh = fresh;
    }

    /// Write the text being typed into its element. An empty text disappears: a click with the
    /// text tool that never got a character leaves nothing behind.
    pub fn commit_edit(&mut self) -> bool {
        let Some(id) = self.editing.take() else { return false };
        let buf = std::mem::take(&mut self.buf);
        let fresh = std::mem::replace(&mut self.fresh, false);
        if buf.trim().is_empty() {
            self.scene.remove(&id);
            if self.selected.as_deref() == Some(id.as_str()) {
                self.selected = None;
            }
            self.dirty |= !fresh;
            return true;
        }
        if let Some(e) = self.scene.element_mut(&id) {
            e.text = Some(buf.clone());
            if e.original_text.is_none() {
                e.original_text = Some(buf.clone());
            }
            if e.width < MIN_SIZE && e.height < MIN_SIZE {
                // Not painted since it was created, so nothing measured it: estimate the box.
                let size = e.font_size.unwrap_or(20.0);
                let lines = buf.lines().count().max(1) as f64;
                let longest = buf.lines().map(|l| l.chars().count()).max().unwrap_or(0);
                e.width = (longest as f64 * size * CHAR_W).max(1.0);
                e.height = (size * e.line_height.unwrap_or(1.25) * lines).max(1.0);
            }
            e.bump(fsio::now_ms(), self.seed);
        }
        self.dirty = true;
        true
    }

    /// Type into the text element being edited. False when nothing is being edited.
    pub fn type_text(&mut self, text: &str) -> bool {
        let Some(id) = self.editing.clone() else { return false };
        self.buf.push_str(text);
        if let Some(e) = self.scene.element_mut(&id) {
            let text = self.buf.clone();
            e.text = Some(text);
        }
        self.dirty = true;
        true
    }

    /// Backspace in the text being edited.
    pub fn backspace(&mut self) -> bool {
        let Some(id) = self.editing.clone() else { return false };
        if self.buf.pop().is_none() {
            return true;
        }
        if let Some(e) = self.scene.element_mut(&id) {
            let text = self.buf.clone();
            e.text = Some(text);
        }
        self.dirty = true;
        true
    }

    /// A newline in the text being edited.
    pub fn newline(&mut self) -> bool {
        self.type_text("\n")
    }

    // ---- editing ----

    fn next_seed(&mut self) -> u64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(fsio::now_ms());
        self.seed
    }

    /// Add the element a tool draws, empty, at `at`, and select it.
    fn create(&mut self, kind: ElementType, at: [f64; 2]) -> String {
        let seed = self.next_seed();
        let mut e = new_element(kind, at[0], at[1], 0.0, 0.0, seed, fsio::now_ms());
        e.stroke_color = self.stroke.clone();
        e.stroke_width = self.width;
        if !matches!(e.kind, ElementType::Arrow | ElementType::Line | ElementType::Freedraw) {
            e.background_color = self.fill.clone();
        }
        if matches!(e.kind, ElementType::Freedraw) {
            e.points = Some(vec![[0.0, 0.0]]);
        }
        let id = e.id.clone();
        self.scene.elements.push(e);
        self.selected = Some(id.clone());
        self.dirty = true;
        self.auto_fit = false;
        id
    }

    /// Delete the selected element.
    pub fn delete_selected(&mut self) -> bool {
        self.editing = None;
        self.buf.clear();
        self.fresh = false;
        let Some(id) = self.selected.take() else { return false };
        self.scene.remove(&id);
        self.dirty = true;
        true
    }

    // ---- pointer ----

    pub fn press(&mut self, rect: &Rect, x: f32, y: f32) {
        let w = self.to_world(rect, x, y);
        if self.editing.is_some() {
            let inside = self
                .editing
                .as_deref()
                .and_then(|id| self.scene.element(id))
                .is_some_and(|e| e.contains(w[0], w[1]));
            if inside {
                return;
            }
            self.commit_edit();
        }
        if self.space_held {
            self.start_pan(x, y);
            return;
        }
        match self.tool {
            Tool::Select => match self.scene.hit(w[0], w[1]).map(|e| e.id.clone()) {
                Some(id) => {
                    self.selected = Some(id.clone());
                    self.press = Some(Press { drag: Drag::Move { id, last: w }, at: (x, y), moved: false });
                }
                None => {
                    self.selected = None;
                    self.start_pan(x, y);
                }
            },
            Tool::Text => {
                let id = self.create(ElementType::Text, w);
                self.start_edit(&id, true);
                self.press = Some(Press { drag: Drag::Idle, at: (x, y), moved: false });
            }
            tool => {
                let Some(kind) = tool.kind() else { return };
                let id = self.create(kind.clone(), w);
                let drag = if matches!(kind, ElementType::Freedraw) {
                    Drag::Free { id }
                } else {
                    Drag::Draw { id, from: w }
                };
                self.press = Some(Press { drag, at: (x, y), moved: false });
            }
        }
    }

    pub fn drag(&mut self, rect: &Rect, x: f32, y: f32) {
        let w = self.to_world(rect, x, y);
        let Some(press) = &mut self.press else { return };
        if !press.moved && (x - press.at.0).abs() + (y - press.at.1).abs() < DRAG_START {
            return;
        }
        press.moved = true;
        match &mut press.drag {
            Drag::Idle => {}
            Drag::Pan { last } => {
                self.cam[0] -= (x - last.0) / self.zoom;
                self.cam[1] -= (y - last.1) / self.zoom;
                *last = (x, y);
                self.auto_fit = false;
            }
            Drag::Move { id, last } => {
                let (dx, dy) = (w[0] - last[0], w[1] - last[1]);
                *last = w;
                if let Some(e) = self.scene.element_mut(id) {
                    e.x += dx;
                    e.y += dy;
                }
                self.dirty = true;
            }
            Drag::Draw { id, from } => {
                let (x0, y0) = (from[0], from[1]);
                if let Some(e) = self.scene.element_mut(id) {
                    e.x = x0.min(w[0]);
                    e.y = y0.min(w[1]);
                    e.width = (w[0] - x0).abs();
                    e.height = (w[1] - y0).abs();
                    if e.points.is_some() {
                        // The points carry the direction; the box itself stays positive.
                        e.points = Some(vec![[0.0, 0.0], [w[0] - x0, w[1] - y0]]);
                    }
                }
                self.dirty = true;
            }
            Drag::Free { id } => {
                if let Some(e) = self.scene.element_mut(id) {
                    let (dx, dy) = (w[0] - e.x, w[1] - e.y);
                    let pts = e.points.get_or_insert_with(Vec::new);
                    if pts.last().is_none_or(|last| (dx - last[0]).hypot(dy - last[1]) >= 2.0) {
                        pts.push([dx, dy]);
                        e.width = e.width.max(dx);
                        e.height = e.height.max(dy);
                    }
                }
                self.dirty = true;
            }
        }
    }

    /// The button came up. True when the board changed and has to be written.
    pub fn release(&mut self) -> bool {
        let Some(p) = self.press.take() else { return false };
        let drawn = match &p.drag {
            Drag::Draw { id, .. } | Drag::Free { id } => Some(id.clone()),
            _ => None,
        };
        let touched = match &p.drag {
            Drag::Move { id, .. } | Drag::Draw { id, .. } | Drag::Free { id } => Some(id.clone()),
            _ => None,
        };
        if let Some(id) = &touched {
            let tiny = self.scene.element(id).is_some_and(|e| e.width < MIN_SIZE && e.height < MIN_SIZE);
            if drawn.is_some() && tiny {
                // A click with a shape tool is not a zero-sized shape.
                self.scene.remove(id);
                if self.selected.as_deref() == Some(id.as_str()) {
                    self.selected = None;
                }
                return false;
            }
        }
        if p.moved {
            let nonce = self.next_seed();
            let now = fsio::now_ms();
            if let Some(id) = touched {
                if let Some(e) = self.scene.element_mut(&id) {
                    e.bump(now, nonce);
                }
            }
        }
        p.moved && self.dirty
    }

    pub fn wheel(&mut self, rect: &Rect, x: f32, y: f32, lines: f32, zoom: bool) {
        if zoom {
            // Ctrl + wheel: zoom about the pointer, so what is under it stays under it.
            let before = self.to_world(rect, x, y);
            self.zoom = (self.zoom * 1.1f32.powf(-lines)).clamp(MIN_ZOOM, MAX_ZOOM);
            self.cam[0] = before[0] as f32 - (x - rect.x - rect.w / 2.0) / self.zoom;
            self.cam[1] = before[1] as f32 - (y - rect.y - rect.h / 2.0) / self.zoom;
        } else {
            // The wheel scrolls the board; the step is in screen pixels, the camera in scene units.
            self.cam[1] += lines * WHEEL_PAN / self.zoom;
        }
        self.auto_fit = false;
    }

    /// The second half of a double click: on a text, edit it; on empty space with the text tool,
    /// start a text there.
    pub fn double_click(&mut self, rect: &Rect, x: f32, y: f32) -> bool {
        if self.editing.is_some() {
            return true;
        }
        let w = self.to_world(rect, x, y);
        match self.scene.hit(w[0], w[1]).map(|e| (e.id.clone(), e.kind.clone())) {
            Some((id, ElementType::Text)) => {
                self.start_edit(&id, false);
                true
            }
            Some(_) => false,
            None if self.tool == Tool::Text => {
                let id = self.create(ElementType::Text, w);
                self.start_edit(&id, true);
                true
            }
            None => false,
        }
    }

    // ---- painting ----

    /// The page colour: the board's own, unless it is the white that dazzles on a dark theme.
    fn surface(&self, c: &[Color; 12], dark: bool) -> Color {
        let saved = self.scene.app_state.view_background_color.as_deref().and_then(Rgba::parse);
        match saved {
            // Pure white is Excalidraw's default, not a choice: on a dark theme it becomes the
            // theme's own very soft grey.
            Some(bg) if dark && bg.r > 240 && bg.g > 240 && bg.b > 240 => mix(c[0], c[2], 0.06),
            Some(bg) => Color { r: bg.r, g: bg.g, b: bg.b, a: 255 },
            None if dark => mix(c[0], c[2], 0.06),
            None => Color::rgb(255, 255, 255),
        }
    }

    /// The faint dot grid, so the board reads as a surface and not as a blank pane.
    fn grid(&self, cmds: &mut Vec<Cmd>, rect: Rect, board: Color, dark: bool) {
        let z = f64::from(self.zoom);
        if z <= 0.0 {
            return;
        }
        let mut step = GRID * z;
        while (f64::from(rect.w) / step) * (f64::from(rect.h) / step) > f64::from(MAX_DOTS) {
            step *= 2.0;
        }
        if step < 6.0 {
            return;
        }
        let ws = step / z;
        let left = f64::from(self.cam[0]) - f64::from(rect.w) / 2.0 / z;
        let top = f64::from(self.cam[1]) - f64::from(rect.h) / 2.0 / z;
        let (right, bottom) = (left + f64::from(rect.w) / z, top + f64::from(rect.h) / z);
        let radius = if step > 26.0 { 1.5 } else { 1.0 };
        let mut items = Vec::new();
        let mut y = (top / ws).ceil() * ws;
        while y <= bottom {
            let mut x = (left / ws).ceil() * ws;
            while x <= right {
                let (sx, sy) = self.to_screen(&rect, x, y);
                items.push([sx, sy, radius]);
                x += ws;
            }
            y += ws;
        }
        let faint = if dark { Color::rgb(255, 255, 255) } else { Color::rgb(0, 0, 0) };
        cmds.push(Cmd::Circles { items, color: mix(board, faint, if dark { 0.2 } else { 0.12 }) });
    }

    pub fn paint(
        &mut self,
        rect: Rect,
        c: &[Color; 12],
        text: &TextStyle,
        dark: bool,
        primary: bool,
        fonts: &mut Fonts,
    ) -> Vec<Cmd> {
        if primary {
            self.fit(&rect);
        }
        let board = self.surface(c, dark);
        let mut cmds = vec![Cmd::PushClip(rect), Cmd::Fill { rect, color: board, radius: 0.0 }];
        self.grid(&mut cmds, rect, board, dark);
        let ox = rect.x + rect.w / 2.0 - self.cam[0] * self.zoom;
        let oy = rect.y + rect.h / 2.0 - self.cam[1] * self.zoom;
        let pen = Pen::new(self.zoom, ox, oy, text, luma(board) < 0.5);
        for op in &self.scene.display_list().ops {
            paint_op(&mut cmds, op, &pen, fonts);
        }
        self.selection(&mut cmds, &pen, rect, c);
        self.caret(&mut cmds, &pen, rect, c, fonts);
        cmds.push(Cmd::PopClip);
        cmds
    }

    /// The dashed frame and the corner dots around the selected element.
    fn selection(&self, cmds: &mut Vec<Cmd>, pen: &Pen, rect: Rect, c: &[Color; 12]) {
        let Some(e) = self.selected.as_deref().and_then(|id| self.scene.element(id)) else { return };
        let (x, y) = pen.p(e.x, e.y);
        let (w, h) = ((e.width as f32 * pen.s).max(2.0), (e.height as f32 * pen.s).max(2.0));
        let frame = Rect::new(x - 1.0, y - 1.0, w + 2.0, h + 2.0);
        let outside = frame.x + frame.w < rect.x
            || frame.x > rect.x + rect.w
            || frame.y + frame.h < rect.y
            || frame.y > rect.y + rect.h;
        if outside {
            return;
        }
        let pts = [
            (frame.x, frame.y),
            (frame.x + frame.w, frame.y),
            (frame.x + frame.w, frame.y + frame.h),
            (frame.x, frame.y + frame.h),
            (frame.x, frame.y),
        ];
        cmds.push(Cmd::Lines { segs: crate::rich::dashed(&pts, 5.0, 4.0), width: 1.5, color: c[5] });
        cmds.push(Cmd::Circles {
            items: vec![
                [frame.x, frame.y, 2.5],
                [frame.x + frame.w, frame.y, 2.5],
                [frame.x + frame.w, frame.y + frame.h, 2.5],
                [frame.x, frame.y + frame.h, 2.5],
            ],
            color: c[5],
        });
    }

    /// The caret of the text being typed, and the box that text really takes: the paint pass is
    /// the one place with the font in hand, so the width a text element saves is measured here.
    fn caret(&mut self, cmds: &mut Vec<Cmd>, pen: &Pen, rect: Rect, c: &[Color; 12], fonts: &mut Fonts) {
        let Some(id) = self.editing.clone() else { return };
        let buf = self.buf.clone();
        let Some(e) = self.scene.element_mut(&id) else { return };
        let mut style = pen.base.clone();
        style.size = (e.font_size.unwrap_or(20.0) as f32 * pen.s).max(1.0);
        style.line_height = style.size * e.line_height.unwrap_or(1.25) as f32;
        style.wrap = false;
        style.mono = false;
        let widest = buf.lines().map(|l| fonts.measure(l, &[], &style, None).0).fold(0.0, f32::max);
        let height = fonts.measure(&buf, &[], &style, None).1;
        if pen.s > 0.0 {
            e.width = (widest / pen.s) as f64;
            e.height = (height / pen.s) as f64;
        }
        let (x, y) = pen.p(e.x, e.y);
        let line = buf.rsplit('\n').next().unwrap_or("").to_string();
        let advance = fonts.measure(&line, &[], &style, None).0;
        let caret = Rect::new(x + advance, y, 1.5, style.line_height.max(4.0));
        if caret.x + caret.w >= rect.x && caret.x <= rect.x + rect.w {
            cmds.push(Cmd::Fill { rect: caret, color: c[5], radius: 0.0 });
        }
    }

    /// The board as the file wants it.
    pub fn to_json(&self) -> String {
        self.scene.to_json()
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: 255 }
}

fn luma(c: Color) -> f32 {
    (0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)) / 255.0
}

// ---- the pane ----

/// The toolbar and the zoom bar of a board, inside `parent` (the painted pane, or the full-screen
/// box). The containers hold an action of their own, so a click on their background is not read as
/// a click on the board underneath.
fn controls(v: &ExcView, dom: &mut Dom, parent: NodeId) {
    let tools = dom.add(parent, El::div().class("exc-toolbar").action("exc:none"));
    for tool in Tool::ALL {
        let b = dom.add(
            tools,
            El::div()
                .class("exc-tool")
                .class_if(v.tool == tool, "is-active")
                .action(format!("exc:tool:{}", tool.id())),
        );
        dom.add(b, crate::icons::el(tool.icon()));
    }
    let palette = dom.add(parent, El::div().class("exc-palette").action("exc:none"));
    for (i, hex) in STROKES.iter().enumerate() {
        dom.add(
            palette,
            El::div()
                .class("exc-swatch")
                .class_if(v.stroke.eq_ignore_ascii_case(hex), "is-active")
                .style(&format!("background-color: {hex}"))
                .action(format!("exc:stroke:{i}")),
        );
    }
    for (i, hex) in FILLS.iter().enumerate() {
        let empty = *hex == "transparent";
        let on = if empty { v.fill == "transparent" } else { v.fill.eq_ignore_ascii_case(hex) };
        let bg = if empty { "transparent".to_string() } else { (*hex).to_string() };
        dom.add(
            palette,
            El::div()
                .class("exc-swatch")
                .class_if(on, "is-active")
                .class_if(empty, "is-empty")
                .style(&format!("background-color: {bg}"))
                .action(format!("exc:fill:{i}")),
        );
    }
    let widths = dom.add(palette, El::div().class("exc-widths"));
    for (i, w) in WIDTHS.iter().enumerate() {
        let b = dom.add(
            widths,
            El::div()
                .class("exc-width")
                .class_if((v.width - w).abs() < 0.01, "is-active")
                .action(format!("exc:width:{i}")),
        );
        dom.add(b, El::div().class("exc-width-dot").style(&format!("height: {}px", (w * 1.6).round())));
    }
    let zoom = dom.add(parent, El::div().class("exc-zoombar").action("exc:none"));
    for (icon, cmd) in [("minus", "exc:zoom-out"), ("plus", "exc:zoom-in")] {
        let b = dom.add(zoom, El::div().class("exc-zoom").action(cmd));
        dom.add(b, crate::icons::el(icon));
    }
    for (label, cmd) in [("Ajustar", "exc:zoom-fit"), ("100%", "exc:zoom-reset")] {
        let b = dom.add(zoom, El::div().class("exc-zoom").action(cmd));
        dom.add(b, El::div().class("exc-zoom-label").text(label));
    }
    dom.add(zoom, El::div().class("exc-zoom-level").text(format!("{} %", (v.zoom * 100.0).round() as i32)));
}

/// A node that paints a board: the pane's, or the full-screen one.
fn board_node(v: &ExcView, dom: &mut Dom, parent: NodeId, info: &mut crate::view::ViewInfo) -> NodeId {
    let board = dom.add(parent, El::div().class("view-content exc-wrapper"));
    let colors = dom.add(board, El::div().class("exc-view-colors"));
    info.exc_colors =
        COLOR_CLASSES.iter().map(|k| dom.add(colors, El::div().class("exc-view").class(k))).collect();
    info.excview.push(board);
    controls(v, dom, board);
    board
}

/// The board of the pane: its header, the controls and the node the paint pass draws into.
pub fn pane(v: &ExcView, dom: &mut Dom, parent: NodeId, info: &mut crate::view::ViewInfo) {
    let header = dom.add(parent, El::div().class("view-header graph-controls exc-controls"));
    let skipped = v.skipped_note().unwrap_or_default();
    let mut label = format!("{}  ·  {} elementos", v.path, v.count());
    if !skipped.is_empty() {
        label.push_str("  ·  ");
        label.push_str(&skipped);
    }
    dom.add(header, El::div().class("graph-count").text(label));
    for (label, cmd) in
        [("Ajustar", "exc:zoom-fit"), ("100%", "exc:zoom-reset"), ("Pantalla completa", "exc:fullscreen")]
    {
        dom.add(header, El::div().class("graph-control").text(label).action(cmd));
    }
    board_node(v, dom, parent, info);
}

/// The board over the whole window: `window_w` × `viewport_h`, dimming what is behind it.
pub fn fullscreen(
    v: &ExcView,
    dom: &mut Dom,
    window_w: f32,
    viewport_h: f32,
    info: &mut crate::view::ViewInfo,
) {
    let w = (window_w - 40.0).max(200.0);
    let h = (viewport_h - 40.0).max(160.0);
    let backdrop = dom.add(0, El::div().class("modal-container mod-dim exc-fullscreen"));
    let modal = dom
        .add(backdrop, El::div().class("modal mod-exc").style(&format!("width: {w:.0}px; height: {h:.0}px")));
    board_node(v, dom, modal, info);
}

/// After layout: paint every board node the view built. The camera fits the full-screen box when
/// it is open, and the pane otherwise.
pub fn add_exc(app: &mut App, dom: &mut Dom, info: &crate::view::ViewInfo, fonts: &mut Fonts) {
    if info.excview.is_empty() {
        return;
    }
    let mut colors = [Color::rgb(128, 128, 128); 12];
    for (slot, &n) in colors.iter_mut().zip(&info.exc_colors) {
        let s = &dom.nodes[n].style;
        *slot = s.text.color.with_alpha(s.opacity);
    }
    let dark = app.dark;
    let primary =
        if app.exc_fullscreen { info.excview.last().copied() } else { info.excview.first().copied() };
    let Some(v) = &mut app.exc_view else { return };
    for node in &info.excview {
        let rect = dom.nodes[*node].rect;
        let text = dom.nodes[*node].style.text.clone();
        let draw = v.paint(rect, &colors, &text, dark, Some(*node) == primary, fonts);
        dom.nodes[*node].draw = draw;
    }
}

// ---- the app ----

impl App {
    pub fn exc_active(&self) -> bool {
        self.show_exc && self.exc_view.is_some() && !self.show_graph && !self.show_canvas && !self.show_base
    }

    pub(crate) fn open_exc(&mut self, rel: &str) {
        if let Some(v) = &mut self.exc_view {
            v.commit_edit();
        }
        self.exc_save();
        let same = self.exc_view.as_ref().is_some_and(|v| v.path.eq_ignore_ascii_case(rel));
        if !same {
            let opened = self
                .store
                .read(rel)
                .map_err(|e| e.to_string())
                .and_then(|(text, stamp)| ExcView::open(rel, &text, stamp).map_err(|e| e.to_string()));
            match opened {
                Ok(v) => self.exc_view = Some(v),
                Err(e) => {
                    self.notice = Some(format!("No se pudo abrir {rel}: {e}"));
                    return;
                }
            }
        }
        self.show_exc = true;
        self.show_canvas = false;
        self.show_base = false;
        self.show_graph = false;
        self.exc_fullscreen = false;
        self.focus = Focus::Editor;
    }

    /// Write the board, when there is something to write.
    pub(crate) fn exc_save(&mut self) {
        let Some(v) = self.exc_view.as_mut().filter(|v| v.dirty) else { return };
        v.commit_edit();
        let json = v.to_json();
        match self.store.write(&v.path, &json, Some(v.stamp)) {
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
    pub(crate) fn exc_close(&mut self) {
        if let Some(v) = &mut self.exc_view {
            v.commit_edit();
        }
        self.exc_save();
        self.exc_view = None;
        self.show_exc = false;
        self.exc_fullscreen = false;
    }

    /// React to the file changing on disk: a board with no unsaved edit is read again.
    pub(crate) fn exc_files_changed(&mut self, paths: &[String]) {
        let Some(v) = self.exc_view.as_mut() else { return };
        if v.dirty || !paths.iter().any(|p| p.eq_ignore_ascii_case(&v.path)) {
            return;
        }
        let current = v.to_json();
        if let Ok((text, stamp)) = self.store.read(&v.path) {
            if text != current {
                if let Ok(scene) = Scene::parse(&text) {
                    v.scene = scene;
                    v.editing = None;
                    v.selected = None;
                }
            }
            v.stamp = stamp;
        }
    }

    /// Commands of the board, `exc:*`. True when the command was one of them.
    pub(crate) fn run_exc(&mut self, command: &str) -> bool {
        let Some(rest) = command.strip_prefix("exc:") else { return false };
        if rest == "none" {
            return true;
        }
        if self.exc_view.is_none() {
            // A board command with no board open: Escape closes the full-screen box, nothing else.
            return true;
        }
        match rest {
            "focus" => {
                self.show_exc = true;
                self.show_canvas = false;
                self.show_base = false;
                self.show_graph = false;
            }
            "close" => self.exc_close(),
            "zoom-in" => {
                if let Some(v) = &mut self.exc_view {
                    v.zoom_by(1.25);
                }
            }
            "zoom-out" => {
                if let Some(v) = &mut self.exc_view {
                    v.zoom_by(1.0 / 1.25);
                }
            }
            "zoom-fit" => {
                if let Some(v) = &mut self.exc_view {
                    v.zoom_fit();
                }
            }
            "zoom-reset" => {
                if let Some(v) = &mut self.exc_view {
                    v.zoom_reset();
                }
            }
            "delete-selected" => {
                if let Some(v) = &mut self.exc_view {
                    v.delete_selected();
                }
                self.exc_save();
            }
            "edit-selected" => {
                if let Some(v) = &mut self.exc_view {
                    if let Some(id) = v.selected.clone() {
                        v.start_edit(&id, false);
                    }
                }
            }
            "fullscreen" => {
                if let Some(v) = &mut self.exc_view {
                    v.commit_edit();
                }
                self.show_exc = true;
                self.show_graph = false;
                self.show_canvas = false;
                self.show_base = false;
                self.exc_fullscreen = true;
            }
            "fullscreen-close" => self.exc_fullscreen = false,
            _ => {
                let Some((name, arg)) = rest.split_once(':') else { return false };
                return self.run_exc_part(name, arg);
            }
        }
        true
    }

    /// `exc:tool:<id>`, `exc:stroke:<i>`, `exc:fill:<i>` and `exc:width:<i>`.
    fn run_exc_part(&mut self, name: &str, arg: &str) -> bool {
        let Some(v) = &mut self.exc_view else { return true };
        match name {
            "tool" => match Tool::from_id(arg) {
                Some(tool) => {
                    v.commit_edit();
                    v.tool = tool;
                    true
                }
                None => false,
            },
            "stroke" => match arg.parse::<usize>().ok().and_then(|i| STROKES.get(i)) {
                Some(hex) => {
                    v.stroke = (*hex).to_string();
                    apply_style(v);
                    true
                }
                None => false,
            },
            "fill" => match arg.parse::<usize>().ok().and_then(|i| FILLS.get(i)) {
                Some(hex) => {
                    v.fill = (*hex).to_string();
                    apply_style(v);
                    true
                }
                None => false,
            },
            "width" => match arg.parse::<usize>().ok().and_then(|i| WIDTHS.get(i)) {
                Some(w) => {
                    v.width = *w;
                    apply_style(v);
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// A key press while a board is open.
    pub(crate) fn exc_key(&mut self, key: &crate::Key) -> bool {
        let Some(v) = &mut self.exc_view else { return false };
        if v.editing.is_some() {
            match key.key.as_str() {
                "Escape" => {
                    v.commit_edit();
                    self.exc_save();
                }
                "Backspace" => {
                    v.backspace();
                }
                "Enter" => {
                    v.newline();
                }
                _ => return false,
            }
            return true;
        }
        match key.key.as_str() {
            "Delete" | "Backspace" => {
                self.run_exc("exc:delete-selected");
            }
            "Enter" => {
                self.run_exc("exc:edit-selected");
            }
            "Escape" => {
                if self.exc_fullscreen {
                    self.exc_fullscreen = false;
                } else if let Some(v) = &mut self.exc_view {
                    v.selected = None;
                }
            }
            _ => return false,
        }
        true
    }

    /// Committed text from the keyboard or the input method, while a board is open.
    pub(crate) fn exc_text(&mut self, text: &str) -> bool {
        let Some(v) = &mut self.exc_view else { return false };
        v.type_text(text)
    }

    /// The space bar, held or released. True when the board took it.
    pub fn exc_space(&mut self, held: bool) -> bool {
        let Some(v) = &mut self.exc_view else { return false };
        if v.editing.is_some() && held {
            return false;
        }
        v.set_space(held);
        true
    }

    /// The rect of the board the pointer is over: the full-screen box when it is open.
    pub(crate) fn exc_rect(&self, frame: &crate::Frame) -> Option<Rect> {
        if !self.exc_active() {
            return None;
        }
        let node = if self.exc_fullscreen {
            frame.info.excview.last().copied()
        } else {
            frame.info.excview.first().copied()
        };
        Some(frame.dom.nodes[node?].rect)
    }

    pub fn exc_click(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(rect) = self.exc_rect(frame) else { return false };
        let inside = rect.contains(x, y);
        if let Some(v) = self.exc_view.as_mut().filter(|_| inside) {
            v.press(&rect, x, y);
        }
        true
    }

    pub fn exc_double_click(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(rect) = self.exc_rect(frame).filter(|r| r.contains(x, y)) else { return false };
        let Some(v) = self.exc_view.as_mut() else { return false };
        v.double_click(&rect, x, y)
    }

    pub fn exc_drag(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(rect) = self.exc_rect(frame) else { return false };
        if let Some(v) = &mut self.exc_view {
            v.drag(&rect, x, y);
        }
        true
    }

    pub fn exc_wheel(&mut self, frame: &crate::Frame, x: f32, y: f32, lines: f32, zoom: bool) -> bool {
        let Some(rect) = self.exc_rect(frame).filter(|r| r.contains(x, y)) else { return false };
        if let Some(v) = &mut self.exc_view {
            v.wheel(&rect, x, y, lines / 3.0, zoom);
        }
        true
    }

    /// The middle button over the board grabs it. True when it did, so the window keeps sending
    /// the pointer's movement here instead of reading it as a click.
    pub fn exc_middle_press(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        if !self.exc_rect(frame).is_some_and(|r| r.contains(x, y)) {
            return false;
        }
        let Some(v) = &mut self.exc_view else { return false };
        v.start_pan(x, y);
        true
    }

    pub fn exc_release(&mut self) {
        let Some(v) = &mut self.exc_view else { return };
        if v.release() {
            self.exc_save();
        }
    }
}

/// Give the selected element the style the toolbar now shows, so a colour or a width picked after
/// a shape was drawn still applies to it.
fn apply_style(v: &mut ExcView) {
    let Some(id) = v.selected.clone() else { return };
    let (stroke, fill, width) = (v.stroke.clone(), v.fill.clone(), v.width);
    if let Some(e) = v.scene.element_mut(&id) {
        e.stroke_color = stroke;
        if !matches!(
            e.kind,
            ElementType::Arrow | ElementType::Line | ElementType::Freedraw | ElementType::Text
        ) {
            e.background_color = fill;
        }
        e.stroke_width = width;
        e.bump(fsio::now_ms(), 0);
    }
    v.dirty = true;
}
