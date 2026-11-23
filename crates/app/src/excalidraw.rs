//! `.excalidraw` boards drawn inside notes: `![[Tablero.excalidraw]]` shows the board, not the
//! name of the file.
//!
//! `excalidraw-core` reads a scene into a display list — boxes, ellipses, diamonds, polylines, text
//! runs and image references in scene coordinates. Nothing here interprets the scene again: this
//! module maps those coordinates onto the note's column and turns each op into the commands
//! `metamind-ui` can paint, the same way the Mermaid and graph views do.
//!
//! # Size
//!
//! A board is scaled to the column it sits in, and never enlarged. The column's width is only
//! known after layout, so the height is reserved for the reading column's default width
//! ([`BOARD_W`], the `--file-line-width` default) and the paint pass then shrinks the board to the
//! width it really got, keeping the aspect: a board is never cut off and a narrower pane leaves it
//! centred with the space it no longer needs.
//!
//! # Fonts
//!
//! Excalidraw's `fontFamily` numbers map onto the faces this build has: 1 (the hand-drawn face,
//! Excalidraw's "Virgil") and 2 (its normal sans) both paint in the app's text font — no
//! hand-lettered face ships with MetaMind — and 3 (the code face) paints in the monospace font.
//! A number a newer Excalidraw writes paints in the text font. Sizes, line height and the file's
//! own colours are used as they are stored.
//!
//! # What is not drawn
//!
//! The limits are the ones `excalidraw-core` reports, plus the ones this painter adds:
//!
//! - Hand-drawn ("sketchy") strokes and hatched fills are straight paint; the flag is read and
//!   ignored. Rounded corners are square and a rotated box keeps its corners.
//! - A rotated shape is drawn as a polygon and a rotated text run stays upright: the paint
//!   commands have no transform, so the rotation is carried into the outline, not the glyphs.
//! - Arrowheads are the simple mark the file asks for (a V, a bar, a dot, a small circle), not
//!   Excalidraw's filled head, and elbow routing is not re-computed: the stored points are used.
//! - A freedraw stroke has one width; `pressures` is not used.
//! - An image element is a labelled placeholder: the bytes are a data URL inside the scene, and
//!   this build does not decode them for an embed.
//! - Elements the display list skipped (`frame`, `embed`, a deleted element, an image with no
//!   file) are reported in one line under the board by [`skipped_note`].

use excalidraw_core::{
    Arrowhead, DisplayList, DrawOp, FontFamily, ImageRef, Polyline, Rgba, Scene, Shape, Style, TextAlign,
    TextRun, VerticalAlign,
};
use metamind_ui::{Cap, Cmd, Color, Dom, El, Fonts, NodeId, Rect, TextStyle};
use std::sync::Arc;

/// The width a board is laid out for: the default `--file-line-width` of the note column.
const BOARD_W: f32 = 700.0;
/// Room above and below the board so its border does not touch the note's text.
const PAD: f32 = 8.0;
/// An ellipse is painted as this many straight sides.
const ELLIPSE_SIDES: usize = 64;
/// Scene units between two dots of the surface grid, in the embed and in the editor.
pub(crate) const GRID: f64 = 20.0;
/// The grid coarsens instead of painting more dots than this.
const MAX_DOTS: f32 = 12_000.0;

/// A board ready to draw: the display list and the box its ops occupy.
pub struct Board {
    pub list: DisplayList,
    /// Everything the ops paint, in scene coordinates: `(x, y, width, height)`.
    pub bounds: (f32, f32, f32, f32),
}

impl Board {
    /// Read a `.excalidraw` file into a board. The error is the reason to show the embed as text.
    pub fn parse(src: &str) -> Result<Board, String> {
        let scene = Scene::parse(src).map_err(|e| e.to_string())?;
        let list = scene.display_list();
        let bounds = match list.bounds() {
            Some(b) => (b.x as f32, b.y as f32, b.width as f32, b.height as f32),
            None => (0.0, 0.0, 0.0, 0.0),
        };
        Ok(Board { list, bounds })
    }

    /// The scale the board is drawn at inside a column `column_w` wide: fit the width, not more.
    fn scale(&self, column_w: f32) -> f32 {
        if self.bounds.2 <= 0.0 {
            return 1.0;
        }
        (column_w / self.bounds.2).clamp(0.0, 1.0)
    }
}

/// A board read from the vault: the picture, or the reason there is none. The reason is shown in
/// place of the board, next to the `![[…]]` line the note keeps.
#[derive(Clone)]
pub enum Loaded {
    Board(Arc<Board>),
    Failed(String),
}

/// A board placed in the note, painted once layout has run.
#[derive(Clone)]
pub struct Item {
    pub node: NodeId,
    pub board: Arc<Board>,
    /// The twelve theme colours the surface and the grid are read from: the same hidden
    /// `.exc-view` elements the board's tab uses, so both paint the same page.
    pub colors: Vec<NodeId>,
}

/// Reserve the space of a board `parent` and hand it to the paint pass.
pub fn add(dom: &mut Dom, parent: NodeId, board: Arc<Board>, items: &mut Vec<Item>) -> NodeId {
    let h = (board.bounds.3 * board.scale(BOARD_W) + 2.0 * PAD).ceil();
    let node = dom.add(parent, El::div().class("exc-board").style(&format!("height: {h}px")));
    let swatches = dom.add(node, El::div().class("exc-view-colors"));
    let colors = crate::excalidrawview::COLOR_CLASSES
        .iter()
        .map(|k| dom.add(swatches, El::div().class("exc-view").class(k)))
        .collect();
    items.push(Item { node, board, colors });
    node
}

/// What the board left out, in one line, or `None` when it drew everything.
pub fn skipped_note(list: &DisplayList) -> Option<String> {
    if list.skipped.is_empty() {
        return None;
    }
    let mut seen: Vec<(String, usize)> = Vec::new();
    for (_, why) in &list.skipped {
        let name = match why {
            excalidraw_core::Skip::Deleted => "eliminado".to_string(),
            excalidraw_core::Skip::Type(t) => t.clone(),
            excalidraw_core::Skip::EmptyText => "texto vacío".to_string(),
            excalidraw_core::Skip::NoGeometry => "sin geometría".to_string(),
            excalidraw_core::Skip::MissingFile(_) => "imagen faltante".to_string(),
        };
        match seen.iter_mut().find(|(k, _)| *k == name) {
            Some((_, n)) => *n += 1,
            None => seen.push((name, 1)),
        }
    }
    let parts: Vec<String> = seen.iter().map(|(k, n)| format!("{n} × {k}")).collect();
    Some(format!(
        "Excalidraw: {} elemento(s) sin dibujar ({}); los marcos y embeds no tienen imagen aquí",
        list.skipped.len(),
        parts.join(", ")
    ))
}

/// Fill in the draw list of every board after layout. `dark` is the theme: a board's plain white
/// page is painted as the theme's soft grey with the dot grid on it, exactly as its tab does, so
/// the embed is never a white flash inside a dark note.
pub fn paint(dom: &mut Dom, items: &[Item], fonts: &mut Fonts, dark: bool) {
    for it in items {
        let r = dom.content_rect(it.node);
        let base = dom.nodes[it.node].style.text.clone();
        let mut theme = [Color::rgb(128, 128, 128); 12];
        for (slot, &n) in theme.iter_mut().zip(&it.colors) {
            let s = &dom.nodes[n].style;
            *slot = s.text.color.with_alpha(s.opacity);
        }
        let page = surface(it.board.list.background, &theme, dark);
        let mut cmds = Vec::new();
        let (bx, by, bw, bh) = it.board.bounds;
        if bw > 0.0 && bh > 0.0 {
            let s = it.board.scale(r.w.min(BOARD_W));
            // Fit-width, centred in whatever space the block reserved.
            let ox = r.x + ((r.w - bw * s) / 2.0).max(0.0) - bx * s;
            let oy = r.y + ((r.h - bh * s) / 2.0).max(0.0) - by * s;
            let area = Rect::new(ox + bx * s, oy + by * s, bw * s, bh * s);
            cmds.push(Cmd::PushClip(r));
            cmds.push(Cmd::Fill { rect: area, color: page, radius: 0.0 });
            // The grid belongs to the dark surface, where an empty board would otherwise be a
            // flat grey slab: the page the user writes on has to read as a surface.
            if dark && it.board.list.background.is_none_or(is_plain_white) {
                grid(area, s, ox, oy, page, dark, &mut cmds);
            }
            let pen = Pen::new(s, ox, oy, &base, luma(page) < 0.5);
            for op in &it.board.list.ops {
                paint_op(&mut cmds, op, &pen, fonts);
            }
            // A hairline, so a white board on a white page still reads as a picture.
            let edge = base.color.with_alpha(0.22);
            cmds.push(Cmd::Lines { segs: edges(&outline(area)), width: 1.0, color: edge });
            cmds.push(Cmd::PopClip);
        }
        dom.nodes[it.node].draw = cmds;
    }
}

/// Excalidraw writes `#ffffff` for a page nobody coloured: on a dark theme that is the theme's
/// surface, not the user's choice (the same rule the board's tab follows).
fn is_plain_white(bg: Rgba) -> bool {
    bg.r > 240 && bg.g > 240 && bg.b > 240
}

/// The page a board is painted on: the board's own colour, unless it is the plain white that
/// dazzles on a dark theme.
pub(crate) fn surface(saved: Option<Rgba>, c: &[Color; 12], dark: bool) -> Color {
    match saved {
        Some(bg) if dark && is_plain_white(bg) => mix(c[0], c[2], 0.06),
        Some(bg) => Color { r: bg.r, g: bg.g, b: bg.b, a: 255 },
        None if dark => mix(c[0], c[2], 0.06),
        None => Color::rgb(255, 255, 255),
    }
}

/// What the empty part of a board is painted with. Excalidraw itself only has the grid on/off
/// switch; a ruled page and dots are the two papers a hand-drawn board wants most, so they are
/// modes of their own here and are saved in `appState.gridMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SurfaceMode {
    /// Nothing: the page is a flat colour.
    None,
    /// A dot every `size` scene pixels.
    #[default]
    Dots,
    /// A line every `size` scene pixels, both ways.
    Grid,
    /// Horizontal rules every `size` scene pixels, with a faint margin line every eight.
    Ruled,
}

impl SurfaceMode {
    pub const ALL: [SurfaceMode; 4] =
        [SurfaceMode::None, SurfaceMode::Dots, SurfaceMode::Grid, SurfaceMode::Ruled];

    pub fn id(self) -> &'static str {
        match self {
            SurfaceMode::None => "none",
            SurfaceMode::Dots => "dots",
            SurfaceMode::Grid => "grid",
            SurfaceMode::Ruled => "ruled",
        }
    }

    pub fn from_id(id: &str) -> Option<SurfaceMode> {
        SurfaceMode::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            SurfaceMode::None => "Sin fondo",
            SurfaceMode::Dots => "Puntos",
            SurfaceMode::Grid => "Cuadrícula",
            SurfaceMode::Ruled => "Hoja rayada",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SurfaceMode::None => "square",
            SurfaceMode::Dots => "dot",
            SurfaceMode::Grid => "table",
            SurfaceMode::Ruled => "list",
        }
    }
}

/// The background of a board: which paper, and how far apart its lines are (scene pixels).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Surface {
    pub mode: SurfaceMode,
    pub size: f32,
}

impl Default for Surface {
    fn default() -> Surface {
        Surface { mode: SurfaceMode::Dots, size: GRID as f32 }
    }
}

/// How far apart the three sizes the toolbar offers put the lines.
pub const SURFACE_SIZES: [f32; 3] = [10.0, 20.0, 40.0];

impl Surface {
    /// The surface a file asks for. A file written by Excalidraw has no `gridMode`: its `gridSize`
    /// (a number, or `"small"`/`"medium"`/`"large"` in older files) only says the grid was on.
    pub fn from_app_state(s: &excalidraw_core::AppState) -> Surface {
        let named = |name: &str| match name {
            "small" => Some(SURFACE_SIZES[0]),
            "medium" => Some(SURFACE_SIZES[1]),
            "large" => Some(SURFACE_SIZES[2]),
            _ => None,
        };
        let size = s
            .grid_size
            .as_ref()
            .and_then(|v| v.as_f64().map(|n| n as f32).or_else(|| v.as_str().and_then(named)))
            .filter(|n| *n >= 4.0 && *n <= 200.0)
            .unwrap_or(GRID as f32);
        let mode = s.grid_mode.as_deref().and_then(SurfaceMode::from_id).unwrap_or_default();
        Surface { mode, size }
    }

    /// Write the surface back so the board reopens the way it was left.
    pub fn write(&self, s: &mut excalidraw_core::AppState) {
        s.grid_mode = Some(self.mode.id().to_string());
        s.grid_size = Some(serde_json::json!(self.size));
    }

    /// Step to the next size, wrapping: the toolbar's small/medium/large switch.
    pub fn next_size(self) -> Surface {
        let next = SURFACE_SIZES
            .iter()
            .position(|n| (n - self.size).abs() < 0.5)
            .map(|i| SURFACE_SIZES[(i + 1) % SURFACE_SIZES.len()])
            .unwrap_or(SURFACE_SIZES[1]);
        Surface { size: next, ..self }
    }

    /// `S`, `M` or `L` for the button that cycles the size.
    pub fn size_label(self) -> &'static str {
        match SURFACE_SIZES.iter().position(|n| (n - self.size).abs() < 0.5) {
            Some(0) => "S",
            Some(2) => "L",
            _ => "M",
        }
    }
}

/// The faint dot grid of a board surface, in scene multiples of [`GRID`], clipped to `rect`.
/// `s` is the scale, `(ox, oy)` where scene `(0, 0)` lands: the embed's fit and the editor's
/// camera both paint through here, so a board's surface looks the same in both.
pub(crate) fn grid(rect: Rect, s: f32, ox: f32, oy: f32, board: Color, dark: bool, cmds: &mut Vec<Cmd>) {
    surface_paper(rect, s, ox, oy, board, dark, Surface::default(), cmds);
}

/// Paints the paper a board sits on: dots, a grid, ruled lines, or nothing.
pub(crate) fn surface_paper(
    rect: Rect,
    s: f32,
    ox: f32,
    oy: f32,
    board: Color,
    dark: bool,
    paper: Surface,
    cmds: &mut Vec<Cmd>,
) {
    if paper.mode == SurfaceMode::None || s <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let mut step = paper.size.max(4.0) * s;
    while (rect.w / step) * (rect.h / step) > MAX_DOTS {
        step *= 2.0;
    }
    if step < 6.0 {
        return;
    }
    let ws = step / s;
    let left = (rect.x - ox) / s;
    let top = (rect.y - oy) / s;
    let (right, bottom) = ((rect.x + rect.w - ox) / s, (rect.y + rect.h - oy) / s);
    let faint = if dark { Color::rgb(255, 255, 255) } else { Color::rgb(0, 0, 0) };
    match paper.mode {
        SurfaceMode::None => {}
        SurfaceMode::Dots => {
            let radius = if step > 26.0 { 1.5 } else { 1.0 };
            let mut items = Vec::new();
            let mut y = (top / ws).ceil() * ws;
            while y <= bottom {
                let mut x = (left / ws).ceil() * ws;
                while x <= right {
                    items.push([ox + x * s, oy + y * s, radius]);
                    x += ws;
                }
                y += ws;
            }
            cmds.push(Cmd::Circles { items, color: mix(board, faint, if dark { 0.2 } else { 0.12 }) });
        }
        SurfaceMode::Grid => {
            let mut segs = Vec::new();
            let mut x = (left / ws).ceil() * ws;
            while x <= right {
                let sx = ox + x * s;
                segs.push([sx, rect.y, sx, rect.y + rect.h]);
                x += ws;
            }
            let mut y = (top / ws).ceil() * ws;
            while y <= bottom {
                let sy = oy + y * s;
                segs.push([rect.x, sy, rect.x + rect.w, sy]);
                y += ws;
            }
            cmds.push(Cmd::Lines {
                segs,
                width: 1.0,
                color: mix(board, faint, if dark { 0.16 } else { 0.1 }),
            });
        }
        SurfaceMode::Ruled => {
            let rule = mix(board, faint, if dark { 0.18 } else { 0.12 });
            let margin = mix(board, faint, if dark { 0.3 } else { 0.2 });
            let mut segs = Vec::new();
            let mut y = (top / ws).ceil() * ws;
            while y <= bottom {
                let sy = oy + y * s;
                segs.push([rect.x, sy, rect.x + rect.w, sy]);
                y += ws;
            }
            // A margin line every eight rules, at the left edge: it reads as paper, not as a table.
            let mut marks = Vec::new();
            let mut y = (top / ws).ceil() * ws;
            let mut n = 0;
            while y <= bottom {
                if n % 8 == 0 {
                    let sy = oy + y * s;
                    marks.push([rect.x + 18.0 * s.min(1.0), sy, rect.x + 18.0 * s.min(1.0), sy + ws * s]);
                }
                y += ws;
                n += 1;
            }
            cmds.push(Cmd::Lines { segs, width: 1.0, color: rule });
            cmds.push(Cmd::Lines { segs: marks, width: 1.0, color: margin });
        }
    }
}

/// `b` blended into `a` by `t`.
pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: 255 }
}

/// How bright a colour is, 0..1: on a dark page the strokes are adapted.
pub(crate) fn luma(c: Color) -> f32 {
    (0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)) / 255.0
}

/// Scene coordinates to the note's (or the board view's) pixels.
pub(crate) struct Pen<'a> {
    pub(crate) s: f32,
    pub(crate) ox: f32,
    pub(crate) oy: f32,
    pub(crate) base: &'a TextStyle,
    /// The board is painted on a dark surface: a near-black stroke would vanish, so the light and
    /// dark ends swap, the way Excalidraw's own dark theme draws a light board.
    pub(crate) adapt: bool,
}

impl<'a> Pen<'a> {
    pub(crate) fn new(s: f32, ox: f32, oy: f32, base: &'a TextStyle, adapt: bool) -> Pen<'a> {
        Pen { s, ox, oy, base, adapt }
    }

    pub(crate) fn p(&self, x: f64, y: f64) -> (f32, f32) {
        (self.ox + x as f32 * self.s, self.oy + y as f32 * self.s)
    }
    pub(crate) fn pts(&self, pts: &[(f64, f64)]) -> Vec<(f32, f32)> {
        pts.iter().map(|&(x, y)| self.p(x, y)).collect()
    }
    /// An element colour as this board paints it.
    pub(crate) fn colour(&self, c: Rgba, opacity: f64) -> Color {
        let c = if self.adapt { flipped(c) } else { c };
        solid(c, opacity)
    }
}

/// Light and dark end of a colour swapped, its hue kept: what a dark board needs so that the
/// near-black strokes Excalidraw writes by default stay readable.
fn flipped(c: Rgba) -> Rgba {
    let luma = 0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b);
    if (60.0..=195.0).contains(&luma) {
        return c;
    }
    let keep = |v: u8| (f32::from(v) * 0.35).round().clamp(0.0, 255.0) as u8;
    if luma < 60.0 {
        Rgba { r: 255 - keep(c.r), g: 255 - keep(c.g), b: 255 - keep(c.b), a: c.a }
    } else {
        Rgba { r: keep(c.r), g: keep(c.g), b: keep(c.b), a: c.a }
    }
}

pub(crate) fn paint_op(out: &mut Vec<Cmd>, op: &DrawOp, pen: &Pen, fonts: &mut Fonts) {
    match op {
        DrawOp::Rect(shape, style) => paint_shape(out, pen, shape, style, ShapeKind::Rect),
        DrawOp::Ellipse(shape, style) => paint_shape(out, pen, shape, style, ShapeKind::Ellipse),
        DrawOp::Diamond(shape, style) => paint_shape(out, pen, shape, style, ShapeKind::Diamond),
        DrawOp::Polyline(line) => paint_polyline(out, pen, line),
        DrawOp::TextRun(run) => paint_text(out, pen, run, fonts),
        DrawOp::ImageRef(image) => paint_image(out, pen, image),
    }
}

#[derive(Clone, Copy)]
enum ShapeKind {
    Rect,
    Ellipse,
    Diamond,
}

fn paint_shape(out: &mut Vec<Cmd>, pen: &Pen, shape: &Shape, style: &Style, kind: ShapeKind) {
    let corners = local_corners(shape, kind);
    let pts = pen.pts(&corners);
    if let Some(fill) = style.fill {
        let color = pen.colour(fill, style.opacity);
        match kind {
            // A box that is not rotated is one rectangle: no polygon scanlines needed.
            ShapeKind::Rect if shape.angle == 0.0 => out.push(Cmd::Fill {
                rect: Rect::new(
                    pen.ox + shape.x as f32 * pen.s,
                    pen.oy + shape.y as f32 * pen.s,
                    (shape.width as f32 * pen.s).max(0.0),
                    (shape.height as f32 * pen.s).max(0.0),
                ),
                color,
                radius: 0.0,
            }),
            _ => fill_polygon(out, &pts, color),
        }
    }
    if let Some(stroke) = style.stroke {
        stroke_path(out, &pts, true, style, pen, stroke);
    }
}

/// The corners of a shape, in scene coordinates, rotated the way Excalidraw stores it.
fn local_corners(shape: &Shape, kind: ShapeKind) -> Vec<(f64, f64)> {
    let (hw, hh) = (shape.width / 2.0, shape.height / 2.0);
    let (cx, cy) = (shape.x + hw, shape.y + hh);
    let local: Vec<(f64, f64)> = match kind {
        ShapeKind::Rect => vec![(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)],
        ShapeKind::Diamond => vec![(0.0, -hh), (hw, 0.0), (0.0, hh), (-hw, 0.0)],
        ShapeKind::Ellipse => (0..ELLIPSE_SIDES)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / ELLIPSE_SIDES as f64;
                (hw * t.cos(), hh * t.sin())
            })
            .collect(),
    };
    let (sin, cos) = shape.angle.sin_cos();
    local.into_iter().map(|(x, y)| (cx + x * cos - y * sin, cy + x * sin + y * cos)).collect()
}

fn paint_polyline(out: &mut Vec<Cmd>, pen: &Pen, line: &Polyline) {
    let pts = pen.pts(&line.points);
    if let Some(stroke) = line.style.stroke {
        if line.pressures.len() == pts.len() && pts.len() > 1 {
            // A pen stroke: the width follows the pressure along the line, as a single ribbon.
            let base = (line.style.stroke_width as f32 * pen.s).max(0.8);
            let points: Vec<[f32; 3]> = pts
                .iter()
                .zip(&line.pressures)
                .map(|(&(x, y), &p)| {
                    let w = base * (0.35 + 1.15 * p as f32);
                    [x, y, w.max(0.6)]
                })
                .collect();
            out.push(Cmd::Stroke { points, color: pen.colour(stroke, line.style.opacity), cap: Cap::Round });
        } else {
            stroke_path(out, &pts, false, &line.style, pen, stroke);
        }
    }
    if pts.len() < 2 {
        return;
    }
    // Arrowheads: the mark the file asked for, at the ends it asked for.
    let colour = line.style.stroke.map_or(pen.base.color, |c| pen.colour(c, line.style.opacity));
    let width = (line.style.stroke_width as f32 * pen.s).max(0.8);
    for (head, end) in [(&line.start_head, 0usize), (&line.end_head, 1usize)] {
        let Some(head) = head else { continue };
        let (tip, from) = if end == 0 { (pts[0], pts[1]) } else { (pts[pts.len() - 1], pts[pts.len() - 2]) };
        paint_head(out, tip, from, head, colour, width, pen);
    }
}

fn paint_head(
    out: &mut Vec<Cmd>,
    tip: (f32, f32),
    from: (f32, f32),
    head: &Arrowhead,
    colour: Color,
    width: f32,
    pen: &Pen,
) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let size = (12.0 * pen.s).max(6.0);
    let spread: f32 = 0.42;
    match head {
        Arrowhead::Arrow => {
            let (c, s) = (spread.cos(), spread.sin());
            let a = (tip.0 - size * (ux * c - ny * s), tip.1 - size * (uy * c + nx * s));
            let b = (tip.0 - size * (ux * c + ny * s), tip.1 - size * (uy * c - nx * s));
            out.push(Cmd::Lines {
                segs: vec![[a.0, a.1, tip.0, tip.1], [b.0, b.1, tip.0, tip.1]],
                width,
                color: colour,
            });
        }
        Arrowhead::Triangle => {
            let (c, s) = (spread.cos(), spread.sin());
            let a = (tip.0 - size * (ux * c - ny * s), tip.1 - size * (uy * c + nx * s));
            let b = (tip.0 - size * (ux * c + ny * s), tip.1 - size * (uy * c - nx * s));
            fill_polygon(out, &[tip, a, b], colour);
        }
        Arrowhead::Bar => {
            let h = size * 0.5;
            out.push(Cmd::Lines {
                segs: vec![[tip.0 - nx * h, tip.1 - ny * h, tip.0 + nx * h, tip.1 + ny * h]],
                width,
                color: colour,
            });
        }
        Arrowhead::Dot => {
            out.push(Cmd::Circles { items: vec![[tip.0, tip.1, (3.5 * pen.s).max(2.0)]], color: colour })
        }
        // A ring: the command fills circles, so the inner radius is painted again below.
        Arrowhead::Circle => {
            let r = (5.0 * pen.s).max(3.0);
            out.push(Cmd::Circles { items: vec![[tip.0, tip.1, r]], color: colour });
        }
        Arrowhead::Other(_) => {}
    }
}

fn paint_text(out: &mut Vec<Cmd>, pen: &Pen, run: &TextRun, fonts: &mut Fonts) {
    let size = (run.size as f32 * pen.s).max(1.0);
    let mut style = pen.base.clone();
    style.size = size;
    style.line_height = size * (run.line_height as f32).max(1.0);
    style.wrap = false;
    style.bold = false;
    style.italic = false;
    style.mono = matches!(run.family, FontFamily::Code);
    style.color = match run.colour {
        Some(c) => pen.colour(c, run.opacity),
        None => pen.base.color.with_alpha(run.opacity as f32),
    };
    let (x, y) = pen.p(run.x, run.y);
    // The run carries its own box; only the alignment needs a measurement.
    let measured = fonts.measure(&run.text, &[], &style, None).0;
    let box_w = run.width as f32 * pen.s;
    let dx = match run.align {
        TextAlign::Center => (box_w - measured) / 2.0,
        TextAlign::Right => box_w - measured,
        _ => 0.0,
    };
    let box_h = run.height as f32 * pen.s;
    let dy = match run.vertical {
        VerticalAlign::Middle => (box_h - style.line_height) / 2.0,
        VerticalAlign::Bottom => box_h - style.line_height,
        _ => 0.0,
    };
    out.push(Cmd::Text {
        x: x + dx,
        y: y + dy,
        width: None,
        text: run.text.clone(),
        spans: Vec::new(),
        style,
    });
}

/// An image in the scene is a data URL inside it; this build shows where the picture goes.
fn paint_image(out: &mut Vec<Cmd>, pen: &Pen, image: &ImageRef) {
    let at = Rect::new(
        pen.ox + image.x as f32 * pen.s,
        pen.oy + image.y as f32 * pen.s,
        (image.width as f32 * pen.s).max(0.0),
        (image.height as f32 * pen.s).max(0.0),
    );
    if at.w < 2.0 || at.h < 2.0 {
        return;
    }
    let colour = pen.base.color.with_alpha(0.5);
    out.push(Cmd::Fill { rect: at, color: pen.base.color.with_alpha(0.06), radius: 0.0 });
    let pts = outline(at);
    out.push(Cmd::Lines { segs: crate::rich::dashed(&pts, 5.0, 4.0), width: 1.0, color: colour });
    let label = match &image.mime_type {
        Some(m) => format!("imagen incrustada ({m}), no dibujada"),
        None => "imagen incrustada, no dibujada".to_string(),
    };
    let size = (12.0 * pen.s).max(9.0);
    out.push(Cmd::Text {
        x: at.x + 6.0,
        y: at.y + at.h / 2.0 - size * 0.7,
        width: None,
        text: label,
        spans: Vec::new(),
        style: TextStyle { size, line_height: size * 1.3, color: colour, wrap: false, ..pen.base.clone() },
    });
}

/// A path stroked with the style's colour, width and dash pattern.
fn stroke_path(out: &mut Vec<Cmd>, pts: &[(f32, f32)], closed: bool, style: &Style, pen: &Pen, stroke: Rgba) {
    if pts.len() < 2 {
        return;
    }
    let mut pts = pts.to_vec();
    if closed {
        pts.push(pts[0]);
    }
    let width = (style.stroke_width as f32 * pen.s).max(0.5);
    let color = pen.colour(stroke, style.opacity);
    let segs = match style.stroke_style {
        excalidraw_core::StrokeStyle::Dashed => crate::rich::dashed(&pts, 6.0 * pen.s, 4.0 * pen.s),
        excalidraw_core::StrokeStyle::Dotted => crate::rich::dashed(&pts, 1.5 * pen.s, 3.0 * pen.s),
        _ => edges(&pts),
    };
    if !segs.is_empty() {
        out.push(Cmd::Lines { segs, width, color });
    }
}

/// One-pixel rows: the display list has no path fill, so a polygon is filled a band at a time.
fn fill_polygon(out: &mut Vec<Cmd>, pts: &[(f32, f32)], color: Color) {
    if pts.len() < 3 {
        return;
    }
    let top = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min).floor();
    let bottom = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max).ceil();
    let mut y = top;
    while y < bottom {
        let band = (y + 1.0).min(bottom);
        let mid = (y + band) / 2.0;
        let mut xs: Vec<f32> = Vec::new();
        for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)).take(pts.len()) {
            let (x0, y0, x1, y1) = (a.0, a.1, b.0, b.1);
            if (y0 <= mid && y1 > mid) || (y1 <= mid && y0 > mid) {
                xs.push(x0 + (mid - y0) * (x1 - x0) / (y1 - y0));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks(2) {
            if let [a, b] = pair {
                if b > a {
                    out.push(Cmd::Fill { rect: Rect::new(*a, y, b - a, band - y), color, radius: 0.0 });
                }
            }
        }
        y = band;
    }
}

fn edges(pts: &[(f32, f32)]) -> Vec<[f32; 4]> {
    pts.windows(2).map(|w| [w[0].0, w[0].1, w[1].0, w[1].1]).collect()
}

fn outline(r: Rect) -> Vec<(f32, f32)> {
    vec![(r.x, r.y), (r.x + r.w, r.y), (r.x + r.w, r.y + r.h), (r.x, r.y + r.h), (r.x, r.y)]
}

fn solid(c: Rgba, opacity: f64) -> Color {
    Color {
        r: c.r,
        g: c.g,
        b: c.b,
        a: (c.a as f32 * opacity.clamp(0.0, 1.0) as f32).round().clamp(0.0, 255.0) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r##"{"type":"excalidraw","version":2,"elements":[
        {"id":"t","type":"text","x":10,"y":10,"width":200,"height":20,"angle":0,"strokeColor":"#1971c2",
         "text":"hola","fontSize":20,"fontFamily":1,"textAlign":"left","lineHeight":1.25},
        {"id":"r","type":"rectangle","x":10,"y":40,"width":100,"height":60,"angle":0,
         "strokeColor":"#1e1e1e","backgroundColor":"#f8f9fa","strokeWidth":2},
        {"id":"a","type":"arrow","x":10,"y":110,"width":100,"height":0,"angle":0,"strokeColor":"#e03131",
         "points":[[0,0],[100,0]],"startArrowhead":null,"endArrowhead":"arrow"}
    ],"appState":{"viewBackgroundColor":"#ffffff"},"files":{}}"##;

    #[test]
    fn board_bounds_cover_every_op() {
        let b = Board::parse(SCENE).unwrap();
        assert_eq!(b.list.ops.len(), 3);
        assert_eq!(b.bounds, (10.0, 10.0, 200.0, 100.0));
    }

    #[test]
    fn scale_fits_the_column_and_never_enlarges() {
        let b = Board::parse(SCENE).unwrap();
        assert_eq!(b.scale(BOARD_W), 1.0, "a board narrower than the column keeps its size");
        let wide = Board::parse(&SCENE.replace("\"width\":200", "\"width\":1400")).unwrap();
        assert!((wide.scale(BOARD_W) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_broken_file_is_an_error_not_a_panic() {
        let e = Board::parse("{not json").err().unwrap();
        assert!(e.contains("invalid Excalidraw scene"), "{e}");
        // A known key with the wrong type is an error, not a silently dropped element.
        assert!(Board::parse(
            r#"{"type":"excalidraw","elements":[{"id":"r","type":"rectangle","x":"left"}]}"#
        )
        .is_err());
        assert!(Board::parse(r#"{"type":"excalidraw","elements":{}}"#).is_err());
    }

    #[test]
    fn skipped_elements_are_named() {
        let src = r##"{"type":"excalidraw","version":2,"elements":[
            {"id":"f","type":"frame","x":0,"y":0,"width":10,"height":10},
            {"id":"r","type":"rectangle","x":0,"y":0,"width":10,"height":10}
        ],"appState":{},"files":{}}"##;
        let b = Board::parse(src).unwrap();
        assert_eq!(b.list.ops.len(), 1);
        let note = skipped_note(&b.list).unwrap();
        assert!(note.contains("1 elemento(s) sin dibujar"), "{note}");
        assert!(note.contains("frame"), "{note}");
        assert!(skipped_note(&Board::parse(SCENE).unwrap().list).is_none());
    }
}
