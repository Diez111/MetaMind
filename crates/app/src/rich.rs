//! Typeset blocks that are painted with custom display-list commands: LaTeX math and Mermaid.
//!
//! The element only reserves the height (computed with font-independent metrics when the tree
//! is built); the commands are produced after layout, when real font metrics are available.

use md_engine::{NodeKind, Tree};
use metamind_ui::{Cmd, Color, Dom, El, Fonts, NodeId, Rect, TextStyle};

#[derive(Clone, Debug)]
pub enum Kind {
    Math { src: String, display: bool },
    Mermaid(String),
}

/// A `$..$` of a block, typeset inside the flow of its own line of text.
///
/// The text keeps the formula's place as blanks ([`Inline::filler`]); once the layout has
/// measured the line, [`paint`] draws the formula into that gap, on the line's baseline.
#[derive(Clone, Debug)]
pub struct Inline {
    /// Source range of the whole `$..$`, from the start of the block's text.
    pub start: usize,
    pub end: usize,
    /// The formula between the dollars.
    pub src: String,
    /// What stands in for it in the text: blanks as wide as the formula is drawn.
    pub filler: String,
    /// Size of the run it sits in, as a multiple of the text size (a heading's scale).
    pub scale: f32,
    /// How much taller than its line the formula is, as a multiple of the line height.
    pub lh: f32,
    /// A `$$..$$` block formula: it takes a line of its own, at the size of a block.
    pub display: bool,
}

/// The formulas of one text node, in the order their blanks appear in it.
#[derive(Clone, Debug)]
pub struct InlineItem {
    pub node: NodeId,
    pub slots: Vec<Inline>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub node: NodeId,
    pub kind: Kind,
    /// Pan of the drawing inside its box, in pixels: what a drag in the full-screen view writes.
    pub pan: (f32, f32),
    /// Extra zoom over the fit the box gives the drawing; `1.0` is fitted.
    pub zoom: f32,
}

const BLOCK_SIZE: f32 = 20.0;
const PAD: f32 = 8.0;
const BASELINE: f32 = 0.99;
/// Width a diagram is fitted to before layout: the note column (`--file-line-width` 700) minus
/// the 28 px of padding the reading view puts on each side. The whole width is used, not a
/// fixed 760 px: percentages in the column resolve against it, and a diagram that can grow is
/// drawn as large as its column allows.
const DIAGRAM_MAX_W: f32 = 644.0;
/// Smallest text size a diagram is scaled to. Shrinking below this turns node labels into an
/// unreadable smudge; a diagram that would need less keeps this size (running past the right
/// edge of its column) and is opened full screen when clicked.
const MIN_FONT_SIZE: f32 = 8.0;
/// Text size of a block of prose (`--font-text-size`), to size a formula's place in the flow
/// before the real font metrics are known.
const TEXT_SIZE: f32 = 16.0;
/// `line-height` of a line of prose in `base.css`, in em.
const LINE_EM: f32 = 1.7;
/// Advance of U+00A0 in the text faces, in em (4.2 px at the 16 px body size). The blanks that
/// hold a formula's place are made of it: no ink, and no line break, so a formula never lands
/// half on each of two lines.
const NBSP_EM: f32 = 0.2625;
/// Spacing of a formula set inside a line of prose: tighter than a display block.
const INLINE_METRICS: math_core::Metrics =
    math_core::Metrics { padding_x: 3.0, padding_y: 1.0, run_gap: 0.0 };
/// Smallest fraction of [`BLOCK_SIZE`] a display formula is drawn at: past this the glyphs stop
/// being readable, so a formula that does not fit is allowed to run under the edge of the pane
/// rather than turn into a band of a few pixels tall.
const MATH_MIN_SCALE: f32 = 0.55;

/// The widget a top-level block is drawn as, if it is one. `src` is the text of the block's
/// root node and `root_start` that node's start within its top-level block.
pub fn block_kind(tree: &Tree, src: &str, root_start: u32) -> Option<Kind> {
    match tree.root().kind {
        NodeKind::MathBlock { content } => {
            let text = src.get(
                content.start.checked_sub(root_start)? as usize
                    ..content.end.checked_sub(root_start)? as usize,
            )?;
            Some(Kind::Math { src: text.trim().to_string(), display: true })
        }
        NodeKind::CodeBlock { .. } => {
            let text = mermaid_source(tree, src, root_start)?;
            mermaid_core::render(text, &mut mermaid_core::Approx).ok()?;
            Some(Kind::Mermaid(text.to_string()))
        }
        _ => None,
    }
}

fn mermaid_source<'a>(tree: &Tree, src: &'a str, root_start: u32) -> Option<&'a str> {
    let NodeKind::CodeBlock { info, content, .. } = tree.root().kind else { return None };
    let rel = |v: u32| v.checked_sub(root_start).map(|v| v as usize);
    let lang = highlight_core::lang_of_info(src.get(rel(info.start)?..rel(info.end)?)?);
    if lang != "mermaid" {
        return None;
    }
    src.get(rel(content.start)?..rel(content.end)?)
}

/// Why a mermaid fence is shown as plain code, if it is one that cannot be drawn.
pub fn notice(tree: &Tree, src: &str, root_start: u32) -> Option<String> {
    let text = mermaid_source(tree, src, root_start)?;
    match mermaid_core::render(text, &mut mermaid_core::Approx) {
        Ok(_) => None,
        Err(mermaid_core::Error::Unsupported(k)) => {
            Some(format!("Los diagramas Mermaid `{k}` no son compatibles; se muestra el código"))
        }
        Err(mermaid_core::Error::Syntax(e)) => Some(format!("Mermaid: {e}; se muestra el código")),
    }
}

/// The inline formulas of a block, with the blanks that hold their place in its text.
///
/// `scale` is the size multiplier of the block's own text (a heading's), so a formula set in a
/// heading is as big as the words around it.
pub fn inline(tree: &Tree, block_text: &str, scale: f32) -> Vec<Inline> {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    tree.iter()
        .filter_map(|(_, n)| match n.kind {
            NodeKind::InlineMath { content, display } => {
                let src = content.of(block_text);
                Some((n.span.start as usize, n.span.end as usize, src.trim().to_string(), display))
            }
            // A `$$..$$` block that is not the root of its top-level block (it sits in a callout,
            // a list item, a footnote) is never seen by `block_kind`: it is typeset here, in the
            // flow of its container, and given a line of its own.
            NodeKind::MathBlock { content } => Some((
                n.span.start as usize,
                n.span.end as usize,
                math_block_source(content.of(block_text)),
                true,
            )),
            _ => None,
        })
        .filter(|(_, _, src, _)| !src.is_empty())
        .map(|(start, end, src, display)| slot(start, end, &src, scale, display))
        .collect()
}

/// The formula of a `$$..$$` block: its lines with the marks of the container it sits in (the `>`
/// of a quote) removed, joined into one expression. A display formula is set as a whole, so the
/// line breaks the container put in front of them are not part of it.
fn math_block_source(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim_start();
        let t = t.strip_prefix('>').map_or(t, |r| r.trim_start()).trim();
        if t.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(t);
    }
    out
}

/// The place one formula keeps in the text it sits in: its blanks, and how tall it makes the
/// line. `start` and `end` are the formula's byte range in the drawn text, for the callers that
/// track it; drawing only needs the order of the blanks.
fn slot(start: usize, end: usize, src: &str, scale: f32, display: bool) -> Inline {
    // A display formula is set at the size a block of its own uses, whatever the text around it.
    let scale = if display { BLOCK_SIZE / TEXT_SIZE } else { scale };
    let size = TEXT_SIZE * scale;
    let l = math_core::render_full(src, size, display, &INLINE_METRICS, &mut math_core::Approx).layout;
    // The blanks are counted in a character's advance, and the face that draws them is
    // not the one the box was measured with: keep air, so the formula can never run
    // under what follows it. `paint` shrinks it if the guess was too small anyway. A display
    // formula owns its line and is centred there, so its blanks only have to be wide enough to
    // keep the line from wrapping in the middle of it.
    let want =
        if display { l.width.min(DIAGRAM_MAX_W).max(NBSP_EM * size * 2.0) } else { l.width * 1.05 + 3.0 };
    let count = ((want / (NBSP_EM * size).max(0.5)).round() as usize).max(1);
    // A line of prose is `LINE_EM` tall; a formula taller than that pushes the line down
    // instead of running over the line above and below. The height comes from the plain
    // box, not from the one with inline padding: `Approx` reports the padded box shorter
    // than the formula the fonts really set (26 px against 27 for a sum with limits, 17
    // against 24 for a fraction), and a line that under-measures its formula draws it
    // over the line below.
    let tall = math_core::render(src, size, display, &mut math_core::Approx).height()
        + if display { 2.0 * PAD } else { 0.0 };
    let lh = ((tall / TEXT_SIZE) / LINE_EM).max(1.0);
    Inline { start, end, src: src.to_string(), filler: "\u{a0}".repeat(count), scale, lh, display }
}

/// The inline formulas of a plain text, such as an AI reply, with the blanks that hold their
/// place in it. A `$` that never closes, a `$$` and a formula split over two lines are left as
/// text: only the formulas a reader would call inline are typeset.
pub fn inline_text(text: &str, scale: f32) -> (String, Vec<Inline>) {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let mut display = String::with_capacity(text.len());
    let mut slots: Vec<Inline> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with('$') && !rest.starts_with("$$") && !text[..i].ends_with('\\') {
            if let Some(len) = rest[1..].find('$').map(|p| p + 1).filter(|&len| len > 1) {
                let src = rest[1..len].trim();
                if !src.is_empty() && !rest[1..len].contains('\n') {
                    let start = display.len();
                    let s = slot(start, start, src, scale, false);
                    display.push_str(&s.filler);
                    slots.push(Inline { end: display.len(), ..s });
                    i += len + 1;
                    continue;
                }
            }
        }
        let c = rest.chars().next().unwrap_or('$');
        display.push(c);
        i += c.len_utf8();
    }
    (display, slots)
}

/// How many formulas a text node holds, read from the blanks left for them.
pub fn slots_in(text: &str) -> usize {
    nbsp_runs(text).len()
}

/// The byte ranges of each run of U+00A0, the blanks a formula's place is made of.
fn nbsp_runs(text: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, c) in text.char_indices() {
        match (c == '\u{a0}', out.last_mut()) {
            (true, Some(last)) if last.1 == i => last.1 = i + c.len_utf8(),
            (true, _) => out.push((i, i + c.len_utf8())),
            _ => {}
        }
    }
    out
}

fn approx_height(kind: &Kind) -> f32 {
    match kind {
        Kind::Math { src, display } => {
            math_core::render(src, BLOCK_SIZE, *display, &mut math_core::Approx).height() + 2.0 * PAD
        }
        Kind::Mermaid(src) => match mermaid_core::render(src, &mut mermaid_core::Approx) {
            Ok(d) => d.height * diagram_scale(&d, DIAGRAM_MAX_W) + 2.0 * PAD,
            Err(_) => 0.0,
        },
    }
}

/// Scale a diagram is drawn at in a block `w` wide: fill the width, never enlarge past its
/// natural size, and never shrink the text below [`MIN_FONT_SIZE`]. Past that floor the drawing
/// is wider than the block (the column cannot scroll sideways), which is what the full-screen
/// view opened by a click is for.
pub(crate) fn diagram_scale(d: &mermaid_core::Diagram, w: f32) -> f32 {
    let fit = (w / d.width.max(1.0)).min(1.0);
    fit.max((MIN_FONT_SIZE / mermaid_core::TEXT_SIZE).min(1.0))
}

pub fn add(dom: &mut Dom, parent: NodeId, kind: Kind, info: &mut Vec<Item>) -> NodeId {
    // The commands are produced after layout, with the real font metrics, from an estimate that
    // is always above what they need (see `inline` for the numbers). A block is never thinner
    // than a line of prose, though, so a formula cannot come out as a band a few pixels tall.
    let h = approx_height(&kind).max(TEXT_SIZE * LINE_EM).ceil();
    let mut el = El::div().class("cm-line rich-block").style(&format!("height: {h}px; width: 100%"));
    if matches!(kind, Kind::Mermaid(_)) {
        // A click opens the diagram on its own, at the width of the window.
        el = el.class("mermaid-block").action("mermaid:zoom");
    }
    let node = dom.add(parent, el);
    info.push(Item { node, kind, pan: (0.0, 0.0), zoom: 1.0 });
    node
}

/// A diagram on its own, full screen: `w` x `h` of room for the painter to fit it to. It clips,
/// so a diagram dragged or zoomed past the box stays inside it, and it carries its own action:
/// a click on it must not reach the backdrop behind and close the view.
pub fn add_zoom(dom: &mut Dom, parent: NodeId, src: &str, info: &mut Vec<Item>, z: Zoom) -> NodeId {
    let el = El::div()
        .class("rich-block diagram-zoom")
        .style(&format!("width: {:.0}px; height: {:.0}px; overflow: hidden", z.w, z.h))
        .action("mermaid:drag");
    let node = dom.add(parent, el);
    info.push(Item { node, kind: Kind::Mermaid(src.to_string()), pan: z.pan, zoom: z.zoom });
    node
}

/// The box the full-screen view gives a diagram, and how far the reader has moved and zoomed it.
#[derive(Clone, Copy, Debug)]
pub struct Zoom {
    pub w: f32,
    pub h: f32,
    pub pan: (f32, f32),
    pub zoom: f32,
}

/// A diagram shown as its own source, in the same box the drawing would take (double click).
pub fn add_zoom_source(dom: &mut Dom, parent: NodeId, src: &str, w: f32, h: f32) -> NodeId {
    dom.add(
        parent,
        El::div()
            .class("rich-block diagram-zoom diagram-zoom-source")
            .style(&format!("width: {w:.0}px; height: {h:.0}px; overflow-y: auto"))
            .action("mermaid:drag")
            .text(src.to_string()),
    )
}

struct Fm<'a> {
    fonts: &'a mut Fonts,
    base: &'a TextStyle,
}

impl math_core::Measure for Fm<'_> {
    fn width(&mut self, text: &str, size: f32, face: math_core::Face) -> f32 {
        let st = face_style(self.base, size, face);
        self.fonts.measure(text, &[], &st, None).0
    }
}

struct Dm<'a>(Fm<'a>);

impl mermaid_core::Measure for Dm<'_> {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let face = if bold { math_core::Face::Bold } else { math_core::Face::Roman };
        math_core::Measure::width(&mut self.0, text, size, face)
    }
}

fn mix(bg: Color, fg: Color, t: f32) -> Color {
    let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color { r: l(bg.r, fg.r), g: l(bg.g, fg.g), b: l(bg.b, fg.b), a: 255 }
}

fn background(dom: &Dom, mut n: NodeId) -> Color {
    loop {
        let c = dom.nodes[n].style.background;
        if c.a == 255 {
            return c;
        }
        match dom.nodes[n].parent {
            Some(p) => n = p,
            None => return Color { r: 30, g: 30, b: 30, a: 255 },
        }
    }
}

struct Pen {
    bg: Color,
    fg: Color,
    s: f32,
    ox: f32,
    oy: f32,
}

impl Pen {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (self.ox + x * self.s, self.oy + y * self.s)
    }
    fn fill(&self, f: mermaid_core::Fill) -> Color {
        match f {
            mermaid_core::Fill::Node => mix(self.bg, self.fg, 0.12),
            mermaid_core::Fill::Accent => mix(self.bg, self.fg, 0.05),
            // The shades a diagram uses to tell neighbouring shapes apart: the discs of a pie,
            // the levels of a mindmap.
            mermaid_core::Fill::Tone(n) => mix(self.bg, self.fg, 0.06 + 0.06 * f32::from(n.min(6))),
            _ => self.bg,
        }
    }
}

/// Fills a polygon one pixel row at a time, the way the diamond of a flowchart is filled: the
/// painter has no polygon of its own, but a row of commands one device pixel tall comes to the
/// same shape.
fn fill_polygon(out: &mut Vec<Cmd>, color: Color, poly: &[(f32, f32)]) {
    if color.a == 0 || poly.len() < 3 {
        return;
    }
    let top = poly.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let bottom = poly.iter().map(|p| p.1).fold(f32::MIN, f32::max);
    if !top.is_finite() || !bottom.is_finite() {
        return;
    }
    let mut y = top.floor();
    while y <= bottom {
        let row = y + 0.5;
        let mut xs: Vec<f32> = Vec::new();
        for w in poly.windows(2) {
            let ((ax, ay), (bx, by)) = (w[0], w[1]);
            if (ay <= row && by > row) || (by <= row && ay > row) {
                xs.push(ax + (row - ay) / (by - ay) * (bx - ax));
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            if pair[1] > pair[0] {
                out.push(Cmd::Fill {
                    rect: Rect::new(pair[0], y, pair[1] - pair[0], 1.0),
                    color,
                    radius: 0.0,
                });
            }
        }
        y += 1.0;
    }
}

pub(crate) fn dashed(pts: &[(f32, f32)], on: f32, off: f32) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    let mut draw = true;
    let mut left = on;
    for w in pts.windows(2) {
        let (mut x, mut y) = w[0];
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-3 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mut rem = len;
        while rem > 0.0 {
            let step = rem.min(left);
            if draw {
                out.push([x, y, x + ux * step, y + uy * step]);
            }
            x += ux * step;
            y += uy * step;
            rem -= step;
            left -= step;
            if left <= 0.0 {
                draw = !draw;
                left = if draw { on } else { off };
            }
        }
    }
    out
}

fn emit_diagram(d: &mermaid_core::Diagram, pen: &Pen, base: &TextStyle, dm: &mut Dm, out: &mut Vec<Cmd>) {
    use mermaid_core::Item as I;
    let stroke = mix(pen.bg, pen.fg, 0.55);
    let line = mix(pen.bg, pen.fg, 0.8);
    let lw = |w: f32| (w * pen.s).max(1.0);
    for it in &d.items {
        match it {
            I::Rect { x, y, w, h, radius, fill, stroke: st } => {
                let snap = |v: f32| (v * 2.0).round() / 2.0;
                let (px, py) = pen.p(*x, *y);
                let (px, py) = (snap(px), snap(py));
                let (w, h, r) = (snap(w * pen.s), snap(h * pen.s), radius * pen.s);
                if *fill == mermaid_core::Fill::Background {
                    out.push(Cmd::Fill { rect: Rect::new(px, py, w, h), color: pen.bg, radius: r });
                    continue;
                }
                let inset = if *st { 1.0 } else { 0.0 };
                if *st {
                    out.push(Cmd::Fill { rect: Rect::new(px, py, w, h), color: stroke, radius: r });
                }
                out.push(Cmd::Fill {
                    rect: Rect::new(px + inset, py + inset, w - 2.0 * inset, h - 2.0 * inset),
                    color: pen.fill(*fill),
                    radius: (r - inset).max(0.0),
                });
            }
            I::Diamond { cx, cy, w, h, fill } => {
                let (cx, cy) = pen.p(*cx, *cy);
                let (hw, hh) = (w * pen.s / 2.0, h * pen.s / 2.0);
                let color = pen.fill(*fill);
                let mut y = -hh;
                while y < hh {
                    let half = hw * (1.0 - (y + 0.5).abs() / hh);
                    out.push(Cmd::Fill {
                        rect: Rect::new(cx - half, cy + y, 2.0 * half, 1.0),
                        color,
                        radius: 0.0,
                    });
                    y += 1.0;
                }
                let pts = [(cx - hw, cy), (cx, cy - hh), (cx + hw, cy), (cx, cy + hh), (cx - hw, cy)];
                let segs = pts.windows(2).map(|p| [p[0].0, p[0].1, p[1].0, p[1].1]).collect();
                out.push(Cmd::Lines { segs, width: lw(1.3), color: stroke });
            }
            I::Ellipse { cx, cy, rx, ry, fill } => {
                let (cx, cy) = pen.p(*cx, *cy);
                let r = rx.max(*ry) * pen.s;
                out.push(Cmd::Circles { items: vec![[cx, cy, r]], color: stroke });
                out.push(Cmd::Circles { items: vec![[cx, cy, r - 1.2]], color: pen.fill(*fill) });
            }
            I::Sector { cx, cy, r, from, to, fill } => {
                let (cx, cy) = pen.p(*cx, *cy);
                let r = r * pen.s;
                let sweep = to - from;
                let steps = (sweep.abs() * 20.0).clamp(3.0, 90.0) as usize;
                let mut poly: Vec<(f32, f32)> = Vec::with_capacity(steps + 2);
                poly.push((cx, cy));
                for i in 0..=steps {
                    let t = from + sweep * (i as f32 / steps as f32);
                    poly.push((cx + r * t.cos(), cy + r * t.sin()));
                }
                fill_polygon(out, pen.fill(*fill), &poly);
                let mut segs: Vec<[f32; 4]> =
                    poly.windows(2).map(|p| [p[0].0, p[0].1, p[1].0, p[1].1]).collect();
                if let Some(last) = poly.last() {
                    segs.push([last.0, last.1, cx, cy]);
                }
                out.push(Cmd::Lines { segs, width: lw(1.0), color: stroke });
            }
            I::Line { pts, dashed: dash, width } => {
                let pts: Vec<(f32, f32)> = pts.iter().map(|p| pen.p(p.0, p.1)).collect();
                let segs = if *dash {
                    dashed(&pts, 5.0 * pen.s, 4.0 * pen.s)
                } else {
                    pts.windows(2).map(|p| [p[0].0, p[0].1, p[1].0, p[1].1]).collect()
                };
                out.push(Cmd::Lines { segs, width: lw(*width), color: line });
            }
            I::Arrow { x, y, angle, .. } => {
                let (x, y) = pen.p(*x, *y);
                let (c, s) = (angle.cos(), angle.sin());
                let len = 9.0 * pen.s;
                let mut segs = Vec::new();
                let mut t = 0.0;
                while t <= len {
                    let half = t * 0.42;
                    let (bx, by) = (x - c * t, y - s * t);
                    segs.push([bx - s * half, by + c * half, bx + s * half, by - c * half]);
                    t += 0.75;
                }
                out.push(Cmd::Lines { segs, width: 1.2, color: line });
            }
            I::Text { x, y, text, bold, size } => {
                let (px, py) = pen.p(*x, *y);
                let sz = size * pen.s;
                let face = if *bold { math_core::Face::Bold } else { math_core::Face::Roman };
                let mut st = face_style(base, sz, face);
                st.color = pen.fg;
                let w = math_core::Measure::width(&mut dm.0, text, sz, face);
                out.push(Cmd::Text {
                    x: px - w / 2.0,
                    y: py - st.line_height / 2.0,
                    width: None,
                    text: text.clone(),
                    spans: Vec::new(),
                    style: st,
                });
            }
        }
    }
}

fn face_style(base: &TextStyle, size: f32, face: math_core::Face) -> TextStyle {
    let mut st = base.clone();
    st.size = size;
    st.line_height = size * 1.3;
    st.wrap = false;
    st.bold = face == math_core::Face::Bold;
    st.italic = face == math_core::Face::Italic;
    st.mono = face == math_core::Face::Mono;
    st
}

fn emit(
    l: &math_core::Layout,
    ox: f32,
    oy: f32,
    sc: f32,
    base: &TextStyle,
    color: Color,
    out: &mut Vec<Cmd>,
) {
    for g in &l.glyphs {
        let mut style = face_style(base, g.size * sc, g.face);
        style.color = color;
        out.push(Cmd::Text {
            x: ox + g.x * sc,
            y: oy + g.y * sc - BASELINE * g.size * sc,
            width: None,
            text: g.text.clone(),
            spans: Vec::new(),
            style,
        });
    }
    let mut widths: Vec<f32> = Vec::new();
    for s in &l.segs {
        if !widths.iter().any(|w| (*w - s.width).abs() < 0.01) {
            widths.push(s.width);
        }
    }
    for w in widths {
        let segs = l
            .segs
            .iter()
            .filter(|s| (s.width - w).abs() < 0.01)
            .map(|s| [ox + s.x0 * sc, oy + s.y0 * sc, ox + s.x1 * sc, oy + s.y1 * sc])
            .collect();
        out.push(Cmd::Lines { segs, width: (w * sc).max(1.0), color });
    }
}

/// Draw the formulas of the flow into the blanks their text kept for them, on the baseline of
/// the line each one sits in.
fn paint_inline(dom: &mut Dom, it: &InlineItem, fonts: &mut Fonts) {
    let Some(text) = dom.nodes[it.node].el.text.clone() else { return };
    let st = &dom.nodes[it.node].style;
    let base = st.text.clone();
    let spans = dom.nodes[it.node].el.spans.clone();
    let color = st.text.color.with_alpha(st.opacity);
    // The text origin, in the scene's own coordinates: the commands of a node are drawn where they
    // are and are not moved by it (see `Dom::paint_node`), so the boxes `range_box` reports -- which
    // are relative to the text -- have to be lifted to the node's content box here. Leaving them
    // relative drew every formula at the top left of the window, where the pane's clip took it out.
    let cb = dom.content_rect(it.node);
    let width = st.text.wrap.then_some(cb.w);
    let mut cmds = Vec::new();
    let mut fm = Fm { fonts, base: &base };
    for (i, (a, b)) in nbsp_runs(&text).into_iter().enumerate() {
        let Some(slot) = it.slots.get(i) else { break };
        let Some(at) = fm.fonts.range_box(&text, &spans, &base, width, a, b) else { continue };
        let size = base.size * slot.scale;
        let metrics = if slot.display { math_core::Metrics::default() } else { INLINE_METRICS };
        let r = math_core::render_full(&slot.src, size, slot.display, &metrics, &mut fm);
        let l = &r.layout;
        let (x, sc) = if slot.display {
            // A display formula owns its line: it is centred in the column, and shrunk to fit
            // when it is wider than the column rather than running under the edge of the pane.
            let avail = cb.w.max(1.0);
            let sc = (avail / l.width.max(1.0)).clamp(MATH_MIN_SCALE, 1.0);
            (cb.x + ((avail - l.width * sc) / 2.0).max(0.0), sc)
        } else {
            // The blanks were counted before the real font metrics were known; if they came up
            // short the formula shrinks into the room it has instead of running under the next
            // word.
            let sc = if l.width > at.rect.w { (at.rect.w / l.width).clamp(0.4, 1.0) } else { 1.0 };
            (cb.x + at.rect.x + (at.rect.w - l.width * sc) / 2.0, sc)
        };
        emit(l, x, cb.y + at.baseline - l.ascent * sc, sc, &base, color, &mut cmds);
    }
    dom.nodes[it.node].draw = cmds;
}

/// Fill in the display lists of the rich blocks and the formulas of the text flow after layout.
pub fn paint(dom: &mut Dom, items: &[Item], inline: &[InlineItem], fonts: &mut Fonts) {
    for it in items {
        let r: Rect = dom.content_rect(it.node);
        let st = &dom.nodes[it.node].style;
        let base = st.text.clone();
        let color = st.text.color.with_alpha(st.opacity);
        let mut cmds = Vec::new();
        let bg = background(dom, it.node);
        let mut fm = Fm { fonts, base: &base };
        if let Kind::Mermaid(src) = &it.kind {
            let mut dm = Dm(fm);
            if let Ok(d) = mermaid_core::render(src, &mut dm) {
                // The box gives the diagram its fit; the reader's own zoom multiplies it and its
                // pan moves it. A block in the note has neither, so it draws fitted and centred.
                let sc = (diagram_scale(&d, r.w) * it.zoom).clamp(0.2, 6.0);
                let ox = r.x + ((r.w - d.width * sc) / 2.0).max(0.0) + it.pan.0;
                let oy = r.y + ((r.h - d.height * sc) / 2.0).max(0.0) + it.pan.1;
                let pen = Pen { bg, fg: color, s: sc, ox, oy };
                emit_diagram(&d, &pen, &base, &mut dm, &mut cmds);
            }
            dom.nodes[it.node].draw = cmds;
            continue;
        }
        match &it.kind {
            Kind::Mermaid(_) => {}
            Kind::Math { src, display } => {
                let avail = r.w.max(1.0);
                let mut size = BLOCK_SIZE;
                let mut l = math_core::render_full(src, size, *display, &math_core::Metrics::default(), &mut fm).layout;
                if l.width > avail {
                    // A display formula wider than its block is shrunk to fit rather than clipped:
                    // clipping loses the end of the line, which is where the result of the
                    // formula usually is. The floor keeps a pathological formula readable
                    // instead of turning it into a smudge, and it is re-rendered at the smaller
                    // size so the real font metrics are the ones that decide the new width.
                    let sc = (avail / l.width).clamp(MATH_MIN_SCALE, 1.0);
                    size = BLOCK_SIZE * sc;
                    l = math_core::render_full(src, size, *display, &math_core::Metrics::default(), &mut fm).layout;
                }
                let x = r.x + ((r.w - l.width) / 2.0).max(0.0);
                let y = r.y + ((r.h - l.height()) / 2.0).max(0.0);
                emit(&l, x, y, 1.0, &base, color, &mut cmds);
            }

        }
        dom.nodes[it.node].draw = cmds;
    }
    for it in inline {
        paint_inline(dom, it, fonts);
    }
}
