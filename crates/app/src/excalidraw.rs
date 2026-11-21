//! `.excalidraw` boards drawn inside notes: `![[Tablero.excalidraw]]` shows the board, not the
//! name of the file.
//!
//! `excalidraw-core` reads a scene into a display list — boxes, ellipses, diamonds, polylines, text
//! runs and image references in scene coordinates. Nothing here interprets the scene again: this
//! module maps those coordinates onto the note's column and turns each op into the commands
//! `rustsidian-ui` can paint, the same way the Mermaid and graph views do.
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
//! hand-lettered face ships with Rustsidian — and 3 (the code face) paints in the monospace font.
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
use rustsidian_ui::{Cmd, Color, Dom, El, Fonts, NodeId, Rect, TextStyle};
use std::sync::Arc;

/// The width a board is laid out for: the default `--file-line-width` of the note column.
const BOARD_W: f32 = 700.0;
/// Room above and below the board so its border does not touch the note's text.
const PAD: f32 = 8.0;
/// An ellipse is painted as this many straight sides.
const ELLIPSE_SIDES: usize = 64;

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
        let bounds = bounds(list.ops.iter());
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
}

/// Reserve the space of a board `parent` and hand it to the paint pass.
pub fn add(dom: &mut Dom, parent: NodeId, board: Arc<Board>, items: &mut Vec<Item>) -> NodeId {
    let h = (board.bounds.3 * board.scale(BOARD_W) + 2.0 * PAD).ceil();
    let node = dom.add(parent, El::div().class("exc-board").style(&format!("height: {h}px")));
    items.push(Item { node, board });
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
            excalidraw_core::Skip::Deleted => "deleted".to_string(),
            excalidraw_core::Skip::Type(t) => t.clone(),
            excalidraw_core::Skip::EmptyText => "empty text".to_string(),
            excalidraw_core::Skip::NoGeometry => "no geometry".to_string(),
            excalidraw_core::Skip::MissingFile(_) => "missing image".to_string(),
        };
        match seen.iter_mut().find(|(k, _)| *k == name) {
            Some((_, n)) => *n += 1,
            None => seen.push((name, 1)),
        }
    }
    let parts: Vec<String> = seen.iter().map(|(k, n)| format!("{n} × {k}")).collect();
    Some(format!(
        "Excalidraw: {} element(s) not drawn ({}); frames and embeds have no picture here",
        list.skipped.len(),
        parts.join(", ")
    ))
}

/// Fill in the draw list of every board after layout.
pub fn paint(dom: &mut Dom, items: &[Item], fonts: &mut Fonts) {
    for it in items {
        let r = dom.content_rect(it.node);
        let base = dom.nodes[it.node].style.text.clone();
        let mut cmds = Vec::new();
        let (bx, by, bw, bh) = it.board.bounds;
        if bw > 0.0 && bh > 0.0 {
            let s = it.board.scale(r.w.min(BOARD_W));
            // Fit-width, centred in whatever space the block reserved.
            let ox = r.x + ((r.w - bw * s) / 2.0).max(0.0) - bx * s;
            let oy = r.y + ((r.h - bh * s) / 2.0).max(0.0) - by * s;
            let area = Rect::new(ox + bx * s, oy + by * s, bw * s, bh * s);
            cmds.push(Cmd::PushClip(r));
            if let Some(bg) = it.board.list.background {
                cmds.push(Cmd::Fill { rect: area, color: solid(bg, 1.0), radius: 0.0 });
            }
            let pen = Pen { s, ox, oy, base: &base };
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

/// Scene coordinates to the note's.
struct Pen<'a> {
    s: f32,
    ox: f32,
    oy: f32,
    base: &'a TextStyle,
}

impl Pen<'_> {
    fn p(&self, x: f64, y: f64) -> (f32, f32) {
        (self.ox + x as f32 * self.s, self.oy + y as f32 * self.s)
    }
    fn pts(&self, pts: &[(f64, f64)]) -> Vec<(f32, f32)> {
        pts.iter().map(|&(x, y)| self.p(x, y)).collect()
    }
}

fn paint_op(out: &mut Vec<Cmd>, op: &DrawOp, pen: &Pen, fonts: &mut Fonts) {
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
        let color = solid(fill, style.opacity);
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
        stroke_path(out, &pts, false, &line.style, pen, stroke);
    }
    if pts.len() < 2 {
        return;
    }
    // Arrowheads: the mark the file asked for, at the ends it asked for.
    let colour = line.style.stroke.map_or(pen.base.color, |c| solid(c, line.style.opacity));
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
        Some(c) => solid(c, run.opacity),
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
    let color = solid(stroke, style.opacity);
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

/// The box every op paints, in scene coordinates.
fn bounds<'a>(ops: impl Iterator<Item = &'a DrawOp>) -> (f32, f32, f32, f32) {
    let (mut x0, mut y0) = (f32::INFINITY, f32::INFINITY);
    let (mut x1, mut y1) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut add = |x: f32, y: f32| {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    };
    let mut any = false;
    for op in ops {
        any = true;
        match op {
            DrawOp::Rect(s, _) | DrawOp::Ellipse(s, _) | DrawOp::Diamond(s, _) => {
                for (x, y) in local_corners(
                    s,
                    match op {
                        DrawOp::Rect(..) => ShapeKind::Rect,
                        DrawOp::Ellipse(..) => ShapeKind::Ellipse,
                        _ => ShapeKind::Diamond,
                    },
                ) {
                    add(x as f32, y as f32);
                }
            }
            DrawOp::Polyline(p) => {
                for &(x, y) in &p.points {
                    add(x as f32, y as f32);
                }
            }
            DrawOp::TextRun(t) => {
                add(t.x as f32, t.y as f32);
                add((t.x + t.width) as f32, (t.y + t.height) as f32);
            }
            DrawOp::ImageRef(i) => {
                add(i.x as f32, i.y as f32);
                add((i.x + i.width) as f32, (i.y + i.height) as f32);
            }
        }
    }
    if !any || !x0.is_finite() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    (x0, y0, x1 - x0, y1 - y0)
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
        assert!(note.contains("1 element(s) not drawn"), "{note}");
        assert!(note.contains("frame"), "{note}");
        assert!(skipped_note(&Board::parse(SCENE).unwrap().list).is_none());
    }
}
