//! Builds the element tree for the current application state, and maps editor geometry
//! (carets, selections, clicks) between source offsets and what is displayed.

use crate::icons;
use crate::shell::{App, Focus, Panel, PromptKind, RightPanel, ViewMode};
use editor_core::deco::{decorate, Deco, Style as Deco_};
use editor_core::Sel;
use md_engine::NodeKind;
use metamind_ui::{Dom, El, Fonts, NodeId, Rect, Span};
use theme_engine::{parse_color, Color, Element};
use vault_core::path;

/// Colors the editor needs for syntax styling, read from the theme's CSS variables.
#[derive(Clone, Debug)]
pub struct Palette {
    pub accent: Color,
    pub muted: Color,
    pub faint: Color,
    pub code_bg: Color,
    pub code: Color,
    pub highlight: Color,
    pub selection: Color,
    pub caret: Color,
    pub tag: Color,
    pub star: Color,
    pub link: Color,
    pub error: Color,
    pub heading_scale: [f32; 6],
    /// Syntax colors indexed by `highlight_core::Class as usize`.
    pub syntax: [Color; 14],
}

struct Body(bool);
impl Element for Body {
    fn tag(&self) -> &str {
        "body"
    }
    fn id(&self) -> Option<&str> {
        None
    }
    fn has_class(&self, c: &str) -> bool {
        c == if self.0 { "theme-dark" } else { "theme-light" }
    }
    fn attr(&self, _: &str) -> Option<&str> {
        None
    }
    fn parent(&self) -> Option<Self> {
        None
    }
    fn prev_sibling(&self) -> Option<Self> {
        None
    }
    fn is_last_child(&self) -> bool {
        false
    }
}

pub fn palette(app: &App) -> Palette {
    let class = if app.dark { "theme-dark" } else { "theme-light" };
    let c = app.theme.compute(&Body(app.dark), &[class], app.env(), None, None);
    let col = |name: &str, default: Color| c.var(name).and_then(|v| parse_color(&v)).unwrap_or(default);
    let num = |name: &str, default: f32| {
        c.var(name).and_then(|v| v.trim().trim_end_matches("em").parse().ok()).unwrap_or(default)
    };
    let syn = |name: &str, dark: (u8, u8, u8), light: (u8, u8, u8)| {
        let d = if app.dark { dark } else { light };
        col(name, Color::rgb(d.0, d.1, d.2))
    };
    let kw = syn("--code-keyword", (224, 108, 155), (170, 40, 100));
    let string = syn("--code-string", (140, 200, 120), (30, 120, 50));
    let comment = syn("--code-comment", (120, 120, 120), (140, 140, 140));
    let value = syn("--code-value", (170, 130, 240), (110, 60, 190));
    let func = syn("--code-function", (225, 200, 100), (150, 110, 10));
    let important = syn("--code-important", (235, 150, 80), (190, 90, 20));
    let property = syn("--code-property", (90, 200, 215), (10, 120, 150));
    let tag = syn("--code-tag", (230, 100, 100), (190, 40, 40));
    let operator = syn("--code-operator", (200, 130, 190), (150, 70, 140));
    let red = syn("--color-red", (230, 100, 100), (190, 40, 40));
    let mut syntax = [kw; 14];
    {
        use highlight_core::Class as K;
        for (k, v) in [
            (K::Keyword, kw),
            (K::String, string),
            (K::Comment, comment),
            (K::Number, value),
            (K::Function, func),
            (K::Type, important),
            (K::Property, property),
            (K::Tag, tag),
            (K::Operator, operator),
            (K::Important, important),
            (K::Constant, value),
            (K::Variable, property),
            (K::Inserted, string),
            (K::Deleted, red),
        ] {
            syntax[k as usize] = v;
        }
    }
    Palette {
        syntax,
        accent: col("--text-accent", Color::rgb(138, 92, 245)),
        muted: col("--text-muted", Color::rgb(150, 150, 150)),
        faint: col("--text-faint", Color::rgb(110, 110, 110)),
        code_bg: col("--code-background", Color::rgb(42, 42, 42)),
        code: col("--code-normal", Color::rgb(214, 197, 166)),
        highlight: col("--text-highlight-bg", Color { r: 255, g: 208, b: 0, a: 100 }),
        selection: col("--text-selection", Color { r: 138, g: 92, b: 245, a: 90 }),
        caret: col("--caret-color", Color::rgb(220, 220, 220)),
        tag: col("--tag-color", Color::rgb(138, 92, 245)),
        star: col("--star-color", Color { r: 208, g: 160, b: 74, a: 255 }),
        link: col("--link-color", Color { r: 138, g: 162, b: 247, a: 255 }),
        error: syn("--text-error", (251, 70, 76), (233, 49, 71)),
        // A theme may make a heading bigger than the text; none may make it the size of the
        // text, which is what `--h1-size: 1.125em` in Minimal does. Headings keep a floor.
        heading_scale: [
            num("--h1-size", 1.8).max(1.7),
            num("--h2-size", 1.5).max(1.35),
            num("--h3-size", 1.3).max(1.15),
            num("--h4-size", 1.15).max(1.05),
            num("--h5-size", 1.05).max(1.0),
            num("--h6-size", 1.0).max(1.0),
        ],
    }
}

/// One rendered editor block: what is displayed and how it maps back to the source.
#[derive(Clone, Debug, Default)]
pub struct BlockView {
    pub block: usize,
    pub node: NodeId,
    /// Source offset of the block content.
    pub src_start: usize,
    pub src_end: usize,
    pub display: String,
    /// (source offset, display offset, length) of each displayed segment.
    pub map: Vec<(usize, usize, usize)>,
}

impl BlockView {
    pub fn to_display(&self, src: usize) -> usize {
        let mut last = 0;
        for &(s, d, len) in &self.map {
            if src < s {
                return d;
            }
            if src <= s + len {
                return d + (src - s);
            }
            last = d + len;
        }
        last
    }
    pub fn to_source(&self, disp: usize) -> usize {
        for &(s, d, len) in &self.map {
            if disp <= d + len {
                return s + disp.saturating_sub(d);
            }
        }
        self.src_end
    }
}

#[derive(Default)]
pub struct ViewInfo {
    /// The accent of the theme, for the marks painted after layout (the tab indicator).
    pub accent: theme_engine::Color,
    pub blocks: Vec<BlockView>,
    pub editor: Option<NodeId>,
    /// The workspace element: everything the windows show above the status bar. The over-window
    /// overlays live inside it, so they reserve the height the status bar takes.
    pub workspace: Option<NodeId>,
    /// The graph pane and the hidden elements its colors are read from.
    pub graph: Option<NodeId>,
    pub graph_colors: Vec<NodeId>,
    /// The graph's settings panel (or its gear button); input over it does not reach the graph.
    pub graph_panel: Option<NodeId>,
    pub canvas: Option<NodeId>,
    pub canvas_colors: Vec<NodeId>,
    /// The kanban board of the pane, and the hidden elements its colours are read from.
    pub kanban: Option<NodeId>,
    pub kanban_colors: Vec<NodeId>,
    /// Each node that paints an Excalidraw board: the pane's, and the full-screen box's.
    pub excview: Vec<NodeId>,
    pub exc_colors: Vec<NodeId>,
    /// Each pane with its leaf element and, unless focused, its scrolling view.
    pub panes: Vec<(usize, NodeId, Option<NodeId>)>,
    /// Each tab of a stacked pane: (pane, tab, its column or spine, its scrolling view unless
    /// it is the focused editor).
    pub stack: Vec<(usize, usize, NodeId, Option<NodeId>)>,
    /// Math and diagram blocks painted after layout.
    pub rich: Vec<crate::rich::Item>,
    /// `.excalidraw` boards embedded in notes, painted after layout.
    pub exc: Vec<crate::excalidraw::Item>,
    /// Inline formulas, painted into the blanks their own line of text kept for them.
    pub inline: Vec<crate::rich::InlineItem>,
    /// Each note view, to scroll once its blocks are laid out.
    pub scrolled: Vec<Scrolled>,
    /// (block, first node it added) for each block `note_blocks` visited.
    pub marks: Vec<(usize, NodeId)>,
}

/// A note view: tab `tab` of pane `pane`, showing blocks from `marks`.
pub struct Scrolled {
    pub pane: usize,
    pub tab: usize,
    pub view: NodeId,
    /// (block, its first node) for each block shown, in order.
    pub marks: Vec<(usize, NodeId)>,
    /// The first and last blocks of the note are in the view.
    pub start: bool,
    pub end: bool,
    /// How many blocks the note has: an anchor past that is stale.
    pub blocks: usize,
    /// Room reserved below this frame's blocks, for the rest of the note: the part of the
    /// scroller's content that has no node of its own this frame.
    pub below: f32,
}

/// What a note's block heights depend on. A different column width, font, fold or number of
/// blocks, and every height has to be measured again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shape {
    /// Width of the note's column, in half pixels.
    width: u32,
    /// `--font-text-size`, in half pixels.
    font: u32,
    /// `--file-line-width`, in half pixels.
    line_width: u32,
    mode: u8,
    /// The version of the folded ranges (`Tab::hidden`).
    folds: u64,
    blocks: usize,
    readable: bool,
}

/// How tall a note is, block by block.
///
/// The view lays out the blocks around the scroll position and nothing else, so the height it
/// computed for its children is the height of a window, not of the document: the limit of the
/// scroll, the end of the note and the scrollbar thumb all came out short. This is the document's
/// own height: the blocks the view lays out are measured by that layout for free, and the rest are
/// measured a batch at a time in a tree of their own. Until a block has been measured it counts
/// for the average of the ones that have, so the note is never shorter than the part of it known.
///
/// `start[block]` is how much of the note is above that block and `start[blocks]` is how tall the
/// whole thing is; both the limit and the thumb are read from this one place.
#[derive(Clone, Default)]
pub struct Extent {
    shape: Option<Shape>,
    /// Height of each block, once a layout has measured it.
    heights: Vec<Option<f32>>,
    /// First block the measuring pass has still to visit.
    pending: usize,
    start: Vec<f32>,
}

impl Shape {
    /// The same column, font, fold and mode: only the length of the note is different, which is
    /// what an Enter or a deletion does.
    fn same_layout(&self, other: Shape) -> bool {
        let (a, b) = (self, &other);
        a.width == b.width
            && a.font == b.font
            && a.line_width == b.line_width
            && a.mode == b.mode
            && a.folds == b.folds
            && a.readable == b.readable
    }
}

impl Extent {
    /// Describe the note `shape` measures, dropping what was measured for another shape. A note
    /// that only got or lost blocks keeps the heights it had — they are the heights of those
    /// blocks — and is walked again, so an Enter does not make the scrollbar forget the note.
    pub fn fit(&mut self, shape: Shape) {
        if self.shape == Some(shape) {
            return;
        }
        if self.shape.is_some_and(|s| s.same_layout(shape)) {
            self.heights.resize(shape.blocks, None);
        } else {
            self.heights = vec![None; shape.blocks];
        }
        self.shape = Some(shape);
        self.pending = 0;
        self.recount();
    }

    /// The blocks the measuring pass takes this frame, if there are any left.
    fn next_batch(&self) -> Option<std::ops::Range<usize>> {
        let n = self.heights.len();
        if self.pending >= n {
            return None;
        }
        Some(self.pending..(self.pending + MEASURE_BATCH).min(n))
    }

    /// Take the heights of a batch: `starts` is where each block that added nodes begins, and
    /// `total` is what the batch measured, bottom margin of its last block included.
    fn fill(&mut self, batch: std::ops::Range<usize>, starts: &[(usize, f32)], total: f32) {
        // A block that added nothing to the tree (a blank line nobody is on, a folded one) is
        // flat, and its own block in the window is flat too: the view skips it the same way.
        for b in batch.clone() {
            if let Some(h) = self.heights.get_mut(b) {
                *h = Some(0.0);
            }
        }
        for (i, &(block, y)) in starts.iter().enumerate() {
            let end = starts.get(i + 1).map_or(total, |s| s.1);
            if let Some(h) = self.heights.get_mut(block) {
                *h = Some((end - y).max(0.0));
            }
        }
        self.pending = batch.end;
        self.recount();
    }

    /// A block the view just laid out, at its exact height.
    fn put(&mut self, block: usize, height: f32) {
        if let Some(h) = self.heights.get_mut(block) {
            *h = Some(height);
        }
    }

    /// Where each block starts, from the heights measured so far and the estimate for the rest.
    fn recount(&mut self) {
        let known: f32 = self.heights.iter().flatten().sum();
        let count = self.heights.iter().filter(|h| h.is_some()).count();
        let average = if count > 0 { known / count as f32 } else { ESTIMATED_BLOCK };
        self.start.clear();
        self.start.reserve(self.heights.len() + 1);
        let mut y = 0.0;
        self.start.push(0.0);
        for h in &self.heights {
            y += h.unwrap_or(average);
            self.start.push(y);
        }
    }

    /// How much of the note is above `block`.
    pub fn offset(&self, block: usize) -> f32 {
        self.start.get(block).copied().unwrap_or_else(|| self.total())
    }

    /// How tall the note is.
    pub fn total(&self) -> f32 {
        self.start.last().copied().unwrap_or(0.0)
    }
}

/// Blocks a frame measures of a note it has not measured yet: a long note is known in a few
/// frames, and the batch is small enough not to show in the cost of a frame.
const MEASURE_BATCH: usize = 32;
/// What an unmeasured block counts for, before a single one has been measured.
const ESTIMATED_BLOCK: f32 = 34.0;
/// The height the measuring tree is laid out at: its blocks are read from the top, and nothing in
/// a note is sized from the height of the column.
const MEASURE_H: f32 = 100_000.0;

/// What a note's block heights depend on, for the extent that holds them.
fn shape_of(app: &App, tab: &crate::shell::Tab, width: f32) -> Shape {
    let a = &app.config.appearance;
    let half = |px: f64| (px * 2.0).round() as u32;
    let px =
        |key: &str, default: f64| half(a.get(key).and_then(serde_json::Value::as_f64).unwrap_or(default));
    Shape {
        width: half(f64::from(width)),
        font: px("baseFontSize", 16.0),
        line_width: px("lineWidth", 700.0),
        mode: match tab.mode {
            ViewMode::Source => 0,
            ViewMode::LivePreview => 1,
            ViewMode::Reading => 2,
        },
        folds: tab.hidden.0,
        blocks: tab.editor.buf.doc().blocks.len(),
        readable: app.config.app.bool("readableLineLength", false),
    }
}

/// The classes the note's column wears, so the measuring tree measures the same column.
fn view_classes(app: &App, mode: ViewMode) -> String {
    let class = if mode == ViewMode::Reading {
        "view-content markdown-reading-view"
    } else {
        "view-content markdown-source-view cm-editor"
    };
    let readable =
        if app.config.app.bool("readableLineLength", false) { " is-readable-line-width" } else { "" };
    format!("{class}{readable}")
}

/// Measure the next batch of blocks of every note the window showed, at the width its column had.
///
/// The blocks are built in a tree of their own and laid out at that width: the heights are the
/// ones the view would give them, without the view having to lay out the whole document each frame.
/// The poster queue is emptied afterwards, because measuring is not painting: a video card off
/// screen must not have its frame decoded for a measurement nobody sees.
pub fn measure_notes(app: &mut App, fonts: &mut Fonts) {
    let shown: Vec<((usize, usize), f32)> = app.note_widths.iter().map(|(&k, &w)| (k, w)).collect();
    let mut wanted = false;
    for ((pane, tab), width) in shown {
        if width <= 0.0 {
            continue;
        }
        let Some(t) = app.pane_tab(pane, tab) else { continue };
        let shape = shape_of(app, t, width);
        if t.extent.shape != Some(shape) {
            if let Some(t) = app.pane_tab_mut(pane, tab) {
                t.extent.fit(shape);
            }
            continue;
        }
        let Some(batch) = t.extent.next_batch() else { continue };
        let (starts, total) = measure_range(app, fonts, width, t, batch.clone());
        wanted = true;
        if let Some(t) = app.pane_tab_mut(pane, tab) {
            t.extent.fill(batch, &starts, total);
        }
    }
    if wanted {
        app.poster_queue.borrow_mut().clear();
    }
}

/// Where each block of `range` starts in a column `width` wide, and how tall the batch is. The
/// first of the two numbers is relative to the top of the content, the same space `Extent` keeps.
fn measure_range(
    app: &App,
    fonts: &mut Fonts,
    width: f32,
    tab: &crate::shell::Tab,
    range: std::ops::Range<usize>,
) -> (Vec<(usize, f32)>, f32) {
    let mut dom = Dom::new();
    // The theme's variables hang off the root's classes: without them every height is the one of
    // a window with no stylesheet.
    dom.nodes[0].el.classes.push(if app.dark { "theme-dark" } else { "theme-light" }.to_string());
    let mut info = ViewInfo::default();
    let pal = palette(app);
    let class = view_classes(app, tab.mode);
    let view = dom.add(0, El::div().class(&class));
    note_blocks(app, &mut dom, view, &pal, &mut info, tab, tab.mode, false, range, false);
    dom.style(&app.theme, app.env());
    dom.layout(fonts, width, MEASURE_H);
    let top = dom.content_rect(view).y;
    let added = dom.nodes.len();
    let mut starts: Vec<(usize, f32)> = Vec::new();
    let mut last: Option<NodeId> = None;
    // A block that added nothing shares its node with the one after it: it is flat, and the mark
    // belongs to the block that really begins there.
    for (b, n) in info.marks.drain(..).filter(|&(_, n)| n < added) {
        if last == Some(n) {
            starts.pop();
        }
        last = Some(n);
        let node = &dom.nodes[n];
        // The margin above a block is part of the note's height and not of its box.
        starts.push((b, node.rect.y - node.style.margin[0] - top));
    }
    let padding = dom.nodes[view].style.padding[2];
    (starts, (dom.nodes[view].content_h - padding).max(0.0))
}

/// Syntax-highlight spans for a fenced code block, appended to `spans`. `map` is the
/// (source offset, display offset, length) mapping of the block's displayed segments.
fn code_spans(
    tree: &md_engine::Tree,
    block_start: usize,
    src: &str,
    src_start: usize,
    map: &[(usize, usize, usize)],
    pal: &Palette,
    spans: &mut Vec<Span>,
) {
    const MAX: usize = 256 * 1024;
    let root = tree.root();
    let NodeKind::CodeBlock { info, content, .. } = root.kind else { return };
    let (cs, ce) = (block_start + content.start as usize, block_start + content.end as usize);
    let (Some(text), Some(info)) = (
        src.get(cs.saturating_sub(src_start)..ce.saturating_sub(src_start)),
        src.get(
            (block_start + info.start as usize).saturating_sub(src_start)
                ..(block_start + info.end as usize).saturating_sub(src_start),
        ),
    ) else {
        return;
    };
    if text.is_empty() || text.len() > MAX || cs < src_start {
        return;
    }
    let toks = highlight_core::highlight_cached(&highlight_core::lang_of_info(info), text);
    let conv = |abs: usize| {
        map.iter().find(|&&(s, _, len)| abs >= s && abs <= s + len).map(|&(s, d, _)| d + abs - s)
    };
    spans.retain(|s| s.color.is_some() || s.bold || s.italic || s.background.is_some());
    for t in toks.iter() {
        if let (Some(a), Some(b)) = (conv(cs + t.start), conv(cs + t.end)) {
            if b > a {
                let c = pal.syntax[t.class as usize];
                spans.push(Span { start: a, end: b, color: Some(c), scale: 1.0, ..Span::default() });
            }
        }
    }
    spans.sort_by_key(|s| s.start);
}

/// Turn a block's source and decorations into display text and styled spans.
///
/// `math` are the block's inline formulas: each one keeps its place in the text as blanks, which
/// the painter fills once the line's real metrics are known. A display formula (`$$..$$`) is given
/// a line of its own: no text may sit beside it.
///
/// `verbatim` keeps the source as it is: a raw HTML block is shown as code, so the tags that
/// carry meaning inside a line of prose (`<div>`, `<br>`) are content there and must not be
/// stripped.
pub fn block_display(
    src: &str,
    src_start: usize,
    decos: &[Deco],
    math: &[crate::rich::Inline],
    pal: &Palette,
    verbatim: bool,
) -> (String, Vec<Span>, Vec<(usize, usize, usize)>) {
    let end = src_start + src.len();
    let mut cuts: Vec<usize> = vec![src_start, end];
    for d in decos {
        cuts.push(d.start.clamp(src_start, end));
        cuts.push(d.end.clamp(src_start, end));
    }
    for m in math {
        cuts.push(m.start.clamp(src_start, end));
        cuts.push(m.end.clamp(src_start, end));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut display = String::with_capacity(src.len());
    let mut spans = Vec::new();
    let mut map = Vec::new();
    // A display formula that was just placed: the next text starts on a line of its own.
    let mut break_after_math = false;
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (ra, rb) = (a - src_start, b - src_start);
        if !src.is_char_boundary(ra) || !src.is_char_boundary(rb) {
            continue;
        }
        // A formula takes its place in the line as blanks of its own width; the source it stands
        // for is not shown at all here (it comes back when the caret enters the block).
        if let Some(m) = math.iter().find(|m| m.start <= a && m.end >= b) {
            if a == m.start {
                break_after_math = place_math(&mut display, &mut spans, &mut map, m);
            }
            continue;
        }
        let mut span =
            Span { start: display.len(), end: display.len() + (b - a), scale: 1.0, ..Span::default() };
        let mut hidden = false;
        let mut styled = false;
        for d in decos.iter().filter(|d| d.start <= a && d.end >= b) {
            styled = true;
            match d.style {
                Deco_::Hide => hidden = true,
                Deco_::Heading(l) => {
                    span.bold = true;
                    span.scale = pal.heading_scale[(l.clamp(1, 6) - 1) as usize];
                }
                Deco_::Strong => span.bold = true,
                Deco_::Emph => span.italic = true,
                Deco_::Strike => {
                    span.strike = true;
                    span.color = Some(pal.muted);
                }
                Deco_::Highlight => span.background = Some(pal.highlight),
                Deco_::Code => {
                    span.mono = true;
                    span.color = Some(pal.code);
                    span.background = Some(pal.code_bg);
                }
                Deco_::Math => {
                    span.mono = true;
                    span.color = Some(pal.code);
                }
                Deco_::Link | Deco_::WikiLink | Deco_::Embed => {
                    // Colour alone carries the link: an underline under every other word reads as
                    // a spelling error, which is what a link used to look like.
                    span.color = Some(pal.link);
                }
                Deco_::Tag => span.color = Some(pal.tag),
                Deco_::Url
                | Deco_::Markup
                | Deco_::Comment
                | Deco_::BlockId
                | Deco_::Html
                | Deco_::Footnote => span.color = Some(pal.faint),
                Deco_::ListMarker => span.color = Some(pal.muted),
                Deco_::Task { checked } => {
                    span.color = Some(if checked { pal.accent } else { pal.muted });
                    span.mono = true;
                }
                _ => {}
            }
        }
        if hidden {
            continue;
        }
        map.push((a, display.len(), b - a));
        // HTML tags (center, div, span, br) are markup: they carry alignment, not content.
        let raw = &src[ra..rb];
        if break_after_math {
            break_after_math = false;
            // The source's own line break already ended the formula's line unless the formula
            // shared its line with text; then the text starts a new line here.
            if !raw.is_empty() && !raw.starts_with('\n') && !display.ends_with('\n') {
                display.push('\n');
            }
        }
        if !verbatim && raw.contains('<') && raw.contains('>') {
            display.push_str(&strip_html(raw));
        } else {
            display.push_str(raw);
        }
        if styled {
            spans.push(span);
        }
    }
    (display, spans, map)
}

/// Give a formula its place in the display: the blanks that stand for it, the span that makes the
/// line tall enough to hold it, and the source range it maps back to. Returns whether the formula
/// is a display one, whose line has to end before any text that follows it.
fn place_math(
    display: &mut String,
    spans: &mut Vec<Span>,
    map: &mut Vec<(usize, usize, usize)>,
    m: &crate::rich::Inline,
) -> bool {
    // A display formula is never set beside words: it starts a line of its own.
    if m.display && !display.is_empty() && !display.ends_with('\n') {
        display.push('\n');
    }
    let at = display.len();
    display.push_str(&m.filler);
    spans.push(Span { start: at, end: display.len(), scale: m.scale, lh_scale: m.lh, ..Span::default() });
    map.push((m.start, at, m.end - m.start));
    m.display
}

/// The display text of a run of source that holds HTML: the tags that only carry alignment are
/// dropped, since the words between them are what a reader wants.
fn strip_html(raw: &str) -> String {
    [
        ("<center>", ""),
        ("</center>", ""),
        ("<div>", ""),
        ("</div>", ""),
        ("<span>", ""),
        ("</span>", ""),
        ("<br>", "\n"),
        ("<br/>", "\n"),
        ("<hr>", "\n---\n"),
    ]
    .iter()
    .fold(raw.to_string(), |s, (tag, with)| s.replace(tag, with))
}

/// Whether a line of the display holds nothing but the blanks of a display formula.
fn is_filler_line(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|c| c == '\u{a0}')
}

/// Drop the empty lines a container leaves at the end of its body (`>` alone on the last line):
/// they are not content, and an empty paragraph makes a callout taller than the one beside it.
fn trim_empty_lines(display: &mut String) {
    let end = display.trim_end_matches(['\n', ' ', '\t', '\r']).len();
    display.truncate(end);
}

/// One line of a rendered block, ready to become an element of its own.
pub(crate) struct BlockLine {
    /// What the line shows, without the line break.
    pub display: String,
    pub spans: Vec<Span>,
    pub map: Vec<(usize, usize, usize)>,
    /// Source range the line shows.
    pub start: usize,
    pub end: usize,
    /// Bullet or number for a list item, how many levels the item is nested under, and the
    /// source range the marker itself came from.
    pub marker: Option<(String, usize, usize, usize)>,
}

/// What a list line starts with, when it is a list item: the glyph drawn in the gutter and where
/// the item's own text starts.
fn list_marker(line: &str) -> Option<(String, usize, usize)> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    let task = |tail: &str| {
        let b = tail.as_bytes();
        (b.len() >= 4 && b[0] == b'[' && b[2] == b']' && b[3] == b' ').then(|| {
            if b[1] == b' ' {
                "☐"
            } else {
                "☑"
            }
        })
    };
    for bullet in ["- ", "* ", "+ "] {
        if let Some(tail) = rest.strip_prefix(bullet) {
            let glyph = task(tail).unwrap_or("•");
            let n = if task(tail).is_some() { 6 } else { 2 };
            return Some((glyph.to_string(), indent + n, indent));
        }
    }
    // `1. `, `10) `: the number is the marker, as it is in the source.
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && digits <= 9 {
        let tail = &rest[digits..];
        if let Some(after) = tail.strip_prefix('.').or_else(|| tail.strip_prefix(')')) {
            if after.starts_with(' ') {
                return Some((rest[..digits + 1].to_string(), indent + digits + 2, indent));
            }
        }
    }
    None
}

/// Cut a block's rendered text at its line breaks. Spans and the source map are clipped and
/// shifted per line, so every line can be drawn as its own element and still map back to the
/// source offsets it came from.
fn cut_lines(display: &str, spans: &[Span], map: &[(usize, usize, usize)]) -> Vec<BlockLine> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for text in display.split('\n') {
        let (lo, hi) = (at, at + text.len());
        at = hi + 1;
        let mut line = BlockLine {
            display: text.to_string(),
            spans: Vec::new(),
            map: Vec::new(),
            start: map
                .iter()
                .find(|&&(_, d, len)| d <= lo && d + len > lo)
                .map_or(0, |&(s, d, _)| s + (lo - d)),
            end: 0,
            marker: None,
        };
        for sp in spans.iter().filter(|sp| sp.start < hi && sp.end > lo) {
            let mut sp = sp.clone();
            sp.start = sp.start.max(lo) - lo;
            sp.end = sp.end.min(hi) - lo;
            line.spans.push(sp);
        }
        for &(src, d, len) in map.iter().filter(|&&(_, d, len)| d < hi && d + len > lo) {
            let (a, b) = (d.max(lo), (d + len).min(hi));
            line.map.push((src + (a - d), a - lo, b - a));
        }
        if let Some(&(s, _, len)) = line.map.last() {
            line.end = s + len;
        } else {
            line.end = line.start;
        }
        if let Some((glyph, len, indent)) = list_marker(text) {
            let marker_start = line.start;
            // The item's own text is what wraps; the marker is drawn apart, in a gutter.
            line.display = text[len..].to_string();
            let drop = |v: &mut Vec<(usize, usize, usize)>| {
                v.retain_mut(|e| {
                    if e.1 + e.2 <= len {
                        return false;
                    }
                    let cut = len.saturating_sub(e.1);
                    e.0 += cut;
                    e.1 = e.1.saturating_sub(len);
                    e.2 = e.2.saturating_sub(cut);
                    e.2 > 0
                });
            };
            drop(&mut line.map);
            line.spans.retain_mut(|sp| {
                if sp.end <= len {
                    return false;
                }
                sp.start = sp.start.saturating_sub(len);
                sp.end -= len;
                true
            });
            line.start = line.map.first().map_or(marker_start + len, |e| e.0);
            line.marker = Some((glyph, indent.div_ceil(2), marker_start, line.start));
        }
        out.push(line);
    }
    out
}

/// Whether any line of a rendered block is a list item: only then is it worth splitting the
/// block, since a single text run cannot hang the wrapped lines under the item's text.
fn has_list_lines(display: &str) -> bool {
    display.split('\n').any(|l| list_marker(l).is_some())
}

/// Whether a rendered block is drawn as a list: one of its lines opens with a bullet or a number.
///
/// A heading is never a list, even when its text starts with `10.`. Its live preview hides the
/// `## ` marker, so the number comes first in the display, and drawing it as a gutter marker put
/// it in a different text run: regular weight, body size and a baseline a few pixels above the
/// heading's. The number is part of the heading and keeps the heading's style.
fn draws_list_lines(root: &NodeKind, display: &str) -> bool {
    !matches!(root, NodeKind::Heading { .. }) && has_list_lines(display)
}

/// Markdown wraps its paragraphs where the writer's editor wrapped them. On screen a paragraph is
/// one run of text, so those breaks are single spaces and the layout wraps at the width it really
/// has — otherwise a hard-wrapped sentence shows as three short, ragged lines. A blank line, a
/// list item, a heading, a quote or a fence starts a line for real, and so does a hard break (a
/// trailing space or backslash). `keep_first` guards a callout's title, which is its own line.
///
/// The text gets shorter, so this also returns where each old byte went, and `remap` moves the
/// spans and the source map after it.
fn join_soft_breaks(display: &str, keep_first: bool) -> Option<(String, Vec<usize>)> {
    if !display.contains('\n') {
        return None;
    }
    let mut lines = Vec::new();
    let mut at = 0;
    for line in display.split('\n') {
        lines.push((at, line));
        at += line.len() + 1;
    }
    // Whether the break before each line survives, and how much indent a joined line loses.
    let mut kept = vec![true; lines.len()];
    let filler = |l: &str| is_filler_line(l.trim_matches([' ', '\t']));
    for i in 1..lines.len() {
        let line = lines[i].1;
        let prev = lines[i - 1].1;
        let t = line.trim_start_matches([' ', '\t']);
        let starts = t.is_empty() || list_marker(line).is_some() || t.starts_with(['#', '>', '|', '`', '~']);
        // A display formula's line is its own line: the text around it never joins it.
        kept[i] = prev.ends_with(' ')
            || prev.ends_with('\\')
            || starts
            || (i == 1 && keep_first)
            || filler(line)
            || filler(prev);
    }
    if kept.iter().all(|&k| k) {
        return None;
    }
    let mut out = String::with_capacity(display.len());
    let mut table = Vec::with_capacity(display.len() + 1);
    for (i, &(_, line)) in lines.iter().enumerate() {
        let text = if kept[i] { line } else { line.trim_start_matches([' ', '\t']) };
        if i > 0 {
            table.push(out.len());
            if kept[i] {
                out.push('\n');
            } else if !out.ends_with(' ') {
                // One space where the break was: the indent of the next line is gone.
                out.push(' ');
            }
            for _ in 0..(line.len() - text.len()) {
                table.push(out.len());
            }
        }
        for ch in text.chars() {
            for _ in 0..ch.len_utf8() {
                table.push(out.len());
            }
            out.push(ch);
        }
    }
    table.push(out.len());
    Some((out, table))
}

/// Move spans and the source map of a block through a `join_soft_breaks` table.
fn remap_lines(spans: &mut Vec<Span>, map: &mut Vec<(usize, usize, usize)>, table: &[usize]) {
    let at = |i: usize| table.get(i).copied().unwrap_or_else(|| table.last().copied().unwrap_or(0));
    spans.retain_mut(|sp| {
        (sp.start, sp.end) = (at(sp.start), at(sp.end));
        sp.start < sp.end
    });
    map.retain_mut(|e| {
        let (d, end) = (at(e.1), at(e.1 + e.2));
        e.1 = d;
        e.2 = end.saturating_sub(d);
        e.2 > 0
    });
}
/// Length of the `#`s and spaces that open a heading, or 0 for any other line.
fn heading_marker(src: &str) -> usize {
    let ws = src.len() - src.trim_start_matches(' ').len();
    let hashes = src[ws..].bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return 0;
    }
    let mut n = ws + hashes;
    n += src[n..].bytes().take_while(|&b| b == b' ').count();
    n
}

/// Display offset of a source offset inside one line, for recolouring a range (the callout
/// title) once the line's own spans have been shifted.
fn display_of(map: &[(usize, usize, usize)], src: usize) -> usize {
    map.iter().find(|&&(s, _, len)| src >= s && src <= s + len).map_or(0, |&(s, d, _)| d + src - s)
}

/// The size multiplier of a block's own text: a heading sets its words larger than the body, and
/// a formula inside it has to grow with them.
fn block_scale(decos: &[Deco], pal: &Palette) -> f32 {
    decos
        .iter()
        .filter_map(|d| match d.style {
            Deco_::Heading(l) => Some(pal.heading_scale[(l.clamp(1, 6) - 1) as usize]),
            _ => None,
        })
        .fold(1.0, f32::max)
}

/// Draw a rendered block that holds list items one line at a time: the bullet or number in a
/// gutter and the item's text beside it, so the wrapped lines of an item start under its text
/// and not under the marker. A callout gets a box and one child per line, which also keeps the
/// title, the paragraphs and the items of the same box aligned.
fn list_block(
    dom: &mut Dom,
    view: NodeId,
    block: usize,
    lines: Vec<BlockLine>,
    callout: Option<(Color, usize, usize)>,
    math: &[crate::rich::Inline],
    info: &mut ViewInfo,
) -> NodeId {
    let parent = match callout {
        Some((color, _, _)) => dom.add(
            view,
            El::div().class("cm-callout-block").style(&format!(
                "border-left-color: rgb({0},{1},{2}); background-color: rgba({0},{1},{2},0.08)",
                color.r, color.g, color.b
            )),
        ),
        None => view,
    };
    let mut next = 0usize;
    for BlockLine { display, mut spans, map, start, end, marker } in lines {
        // Read before the line's text goes into its block: the blanks are what says how many
        // formulas it holds.
        let shown = display.clone();
        // A display formula owns its line: it is drawn by the block painter, exactly as a `$$`
        // block of the note would be, instead of as a run of blanks in a line of text.
        if let Some(slot) = display_slot(&shown, math, &mut next) {
            let kind = crate::rich::Kind::Math { src: slot.src, display: true };
            let node = crate::rich::add(dom, parent, kind, &mut info.rich);
            info.blocks.push(BlockView {
                block,
                node,
                src_start: start,
                src_end: end,
                map: vec![(start, 0, 0)],
                ..BlockView::default()
            });
            continue;
        }
        if let Some((color, ts, te)) = callout {
            let (d0, d1) = (display_of(&map, ts), display_of(&map, te));
            for sp in spans.iter_mut().filter(|sp| sp.start >= d0 && sp.end <= d1) {
                sp.color = Some(color);
            }
        }
        let node = match marker {
            Some((glyph, depth, first, last)) => {
                let row = dom.add(
                    parent,
                    El::div()
                        .class("cm-line cm-list-item")
                        .style(&format!("padding-left: {:.2}em", depth as f32 * 1.4)),
                );
                let m = dom.add(row, El::div().class("list-item-marker").text(glyph.clone()));
                info.blocks.push(BlockView {
                    block,
                    node: m,
                    src_start: first,
                    src_end: last,
                    display: glyph.clone(),
                    map: vec![(first, 0, (last - first).min(glyph.len()))],
                });
                dom.add(row, El::div().class("list-item-text").text(display.clone()).spans(spans))
            }
            None => dom.add(
                parent,
                El::div()
                    .class("cm-line")
                    .class_if(callout.is_some(), "cm-callout-line")
                    .text(display.clone())
                    .spans(spans),
            ),
        };
        info.blocks.push(BlockView {
            block,
            node,
            src_start: start,
            src_end: end,
            display: shown.clone(),
            map,
        });
        attach_math(&shown, math, &mut next, node, info);
    }
    parent
}

/// Record the formulas whose blanks a text node holds, in the order they appear in it.
fn attach_math(
    display: &str,
    math: &[crate::rich::Inline],
    next: &mut usize,
    node: NodeId,
    info: &mut ViewInfo,
) {
    let n = crate::rich::slots_in(display).min(math.len().saturating_sub(*next));
    if n == 0 {
        return;
    }
    info.inline.push(crate::rich::InlineItem { node, slots: math[*next..*next + n].to_vec() });
    *next += n;
}

/// The display formula whose blanks make up a whole line of a container, if that is what the line
/// is. Stepping past the formula it takes is left to [`attach_math`] for every other line, so a
/// line is counted once.
fn display_slot(line: &str, math: &[crate::rich::Inline], next: &mut usize) -> Option<crate::rich::Inline> {
    if !is_filler_line(line) || crate::rich::slots_in(line) != 1 {
        return None;
    }
    let slot = math.get(*next).filter(|m| m.display).cloned()?;
    *next += 1;
    Some(slot)
}

/// Mark the characters that carry meaning on their own: the stars that rate a note and the `·`
/// some dashboards use as a bullet. Purely typographic, so it never adds or removes text.
fn mark_symbols(display: &str, spans: &mut Vec<Span>, pal: &Palette) {
    let mut extra: Vec<Span> = Vec::new();
    let mut line_start = 0usize;
    let mut prev = '\0';
    for (i, c) in display.char_indices() {
        if c == '\n' {
            line_start = i + c.len_utf8();
        }
        if c == '★' || c == '☆' {
            match extra.last_mut() {
                Some(s) if s.end == i && (prev == '★' || prev == '☆') => s.end = i + c.len_utf8(),
                _ => extra.push(Span {
                    start: i,
                    end: i + c.len_utf8(),
                    color: Some(pal.star),
                    ..Span::default()
                }),
            }
        }
        if c == '·' && display[line_start..i].trim().is_empty() {
            extra.push(Span { start: i, end: i + c.len_utf8(), color: Some(pal.muted), ..Span::default() });
        }
        prev = c;
    }
    // Only where nothing else already styles the character: a star inside a link keeps the
    // link's colour.
    extra.retain(|e| !spans.iter().any(|s| s.start < e.end && e.start < s.end));
    if extra.is_empty() {
        return;
    }
    spans.extend(extra);
    spans.sort_by_key(|s| (s.start, s.end));
}

fn sidebar_files(app: &App, dom: &mut Dom, parent: NodeId, viewport_h: f32) {
    let header = dom.add(parent, El::div().class("nav-header"));
    dom.add(
        header,
        El::div()
            .class("nav-header-title")
            .text(app.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("Bóveda").to_string()),
    );
    for (icon, cmd) in [
        ("note-plus", "file-explorer:new-file"),
        ("sort", "file-explorer:sort-menu"),
        ("collapse", "file-explorer:collapse-all"),
    ] {
        let b = icon_button(dom, header, icon, cmd, false);
        if cmd == "file-explorer:sort-menu" {
            dom.nodes[b].el.attrs.push(("aria-label".into(), "Cambiar orden".into()));
        }
    }
    let target = app.explorer_drag.as_ref().and_then(|d| d.target.as_deref());
    let list = dom.add(
        parent,
        El::div()
            .class("nav-files-container")
            .class_if(target == Some(""), "is-being-dragged-over")
            .id("files"),
    );
    // Virtualised: fixed-height rows, only those in view are built; spacers keep the scroll range.
    let rows = app.tree_rows();
    let h = crate::shell::TREE_ROW_HEIGHT;
    let scroll = app.scrolls.get("files").copied().unwrap_or(0.0);
    let first = ((scroll / h).floor() as usize).min(rows.len());
    let last = (first + (viewport_h / h).ceil() as usize + 2).min(rows.len());
    let active = app.tab().map(|t| t.path.as_str());
    if first > 0 {
        dom.add(list, El::div().style(&format!("height: {}px", first as f32 * h)));
    }
    for r in &rows[first..last] {
        let pad = 6 + r.depth * 14 + if r.folder { 0 } else { 18 };
        let el = if r.folder {
            El::div()
                .class("nav-folder-title")
                .class_if(target == Some(r.path.as_str()), "is-being-dragged-over")
                .action(format!("toggle-folder:{}", r.path))
        } else {
            El::div()
                .class("nav-file-title")
                .class_if(active.is_some_and(|a| a.eq_ignore_ascii_case(&r.path)), "is-active")
                .action(format!("open:{}", r.path))
        };
        let row = dom.add(list, el.style(&format!("padding-left: {pad}px")));
        if r.folder {
            let chevron = if r.collapsed { "chevron-right" } else { "chevron-down" };
            dom.add(row, icons::el(chevron).class("nav-folder-collapse-indicator"));
            dom.add(row, icons::el("folder"));
        } else {
            dom.add(row, icons::el(icons::for_file(&r.path)));
        }
        dom.add(row, El::div().class("nav-file-title-content").text(r.label.clone()));
    }
    if last < rows.len() {
        dom.add(list, El::div().style(&format!("height: {}px", (rows.len() - last) as f32 * h)));
    }
    if rows.is_empty() {
        empty_pane(dom, list, "folder", "Esta bóveda no tiene notas todavía.");
    }
}

/// Characters that fit, with room to spare, in the fields of this app: one line of 13 px text in a
/// 250 px sidebar is about thirty of them, and the palette's 560 px box about sixty.
const SIDE_FIELD_CHARS: usize = 28;
const SEARCH_FIELD_CHARS: usize = 28;
const PROMPT_FIELD_CHARS: usize = 54;

/// The tail of a prompt query that fits in the field of a window `w` pixels wide, with the
/// ellipsis that says it was cut and the caret at the end. The field is what the box, its margins,
/// its padding, its icon and its borders leave, and a character of the 16 px field takes about
/// 8 px: 9 px each is a budget the narrowest window still meets.
fn prompt_chars(w: f32) -> usize {
    (((w - 96.0).max(120.0) / 9.0) as usize).min(PROMPT_FIELD_CHARS)
}

fn sidebar_search(app: &App, dom: &mut Dom, parent: NodeId) {
    section_header(dom, parent, "search", "Buscar");
    let shown = if app.search_query.is_empty() && app.focus != Focus::Search {
        "Escribe una búsqueda y pulsa Intro".to_string()
    } else if app.focus == Focus::Search {
        with_caret(&app.search_query, SEARCH_FIELD_CHARS)
    } else {
        app.search_query.clone()
    };
    let field = dom.add(
        parent,
        El::div()
            .class("search-input-container")
            .class_if(app.focus == Focus::Search, "is-editing")
            .text(shown)
            .action("focus:search"),
    );
    if app.focus == Focus::Search {
        // The ring on the field the keyboard is in, the same one every other field wears.
        dom.nodes[field].focus = true;
    }
    if app.focus == Focus::Search && app.search_query.is_empty() {
        search_options(dom, parent, "search:insert:");
    }
    let list = dom.add(parent, El::div().class("nav-files-container").id("search-results"));
    if let Some(e) = &app.search_error {
        empty_pane(dom, list, "warning", format!("Error en la búsqueda: {e}"));
        return;
    }
    if app.search_results.is_empty() && !app.search_query.trim().is_empty() {
        // A search that found nothing is an answer, not an empty pane.
        empty_pane(dom, list, "search", format!("Nada coincide con «{}».", app.search_query.trim()));
    }
    for r in &app.search_results {
        dom.add(
            list,
            El::div()
                .class("search-result-file-title")
                .text(path::strip_md(&r.path).to_string())
                .action(format!("open:{}", r.path)),
        );
        for s in &r.snippets {
            dom.add(
                list,
                El::div()
                    .class("search-result-file-match")
                    .text(s.clone())
                    .action(format!("open:{}", r.path)),
            );
        }
    }
}

/// Operators offered while a search field is empty, as (inserted text, description).
pub const SEARCH_OPTIONS: &[(&str, &str)] = &[
    ("path:", "coincidir con la ruta del archivo"),
    ("file:", "coincidir con el nombre del archivo"),
    ("tag:", "buscar etiquetas"),
    ("line:", "buscar palabras en la misma línea"),
    ("section:", "buscar palabras bajo el mismo encabezado"),
    ("[", "coincidir con una propiedad"),
];

fn search_options(dom: &mut Dom, parent: NodeId, action: &str) {
    let list = dom.add(parent, El::div().class("search-options"));
    dom.add(list, El::div().class("search-options-title").text("Opciones de búsqueda"));
    for (op, desc) in SEARCH_OPTIONS {
        let row = dom.add(list, El::div().class("suggestion-item").action(format!("{action}{op}")));
        let label = if *op == "[" { "[propiedad]" } else { op };
        dom.add(row, El::div().class("suggestion-title").text(label));
        dom.add(row, El::div().class("suggestion-aux").text(*desc));
    }
}

fn right_sidebar(app: &App, dom: &mut Dom, parent: NodeId, info: &mut ViewInfo) {
    let head = dom.add(parent, El::div().class("sidebar-tab-header"));
    for (icon, cmd, panel) in [
        ("link", "backlinks:open", RightPanel::Backlinks),
        ("arrow-up-right", "outgoing-links:open", RightPanel::Outgoing),
        ("list", "outline:open", RightPanel::Outline),
        ("ai", "ai:open", RightPanel::Ai),
    ] {
        if app.disabled_by(cmd).is_some() {
            continue;
        }
        icon_button(dom, head, icon, cmd, app.right_panel == panel);
    }
    // The copilot is useful with no note open: it answers without the note as context.
    let tab = app.tab();
    if app.right_panel != RightPanel::Ai {
        let Some(tab) = tab else {
            dom.add(parent, El::div().class("pane-empty").text("No hay ningún archivo abierto."));
            return;
        };
        if let Some(name) = app.disabled_by(match app.right_panel {
            RightPanel::Outgoing => "outgoing-links:open",
            RightPanel::Outline => "outline:open",
            _ => "backlinks:open",
        }) {
            dom.add(parent, El::div().class("pane-empty").text(format!("{name} está desactivado.")));
            return;
        }
        match app.right_panel {
            RightPanel::Outline => outline(tab, dom, parent),
            RightPanel::Outgoing => outgoing(app, tab, dom, parent),
            _ => backlinks(app, dom, parent),
        }
        return;
    }
    if let Some(name) = app.disabled_by("ai:open") {
        dom.add(parent, El::div().class("pane-empty").text(format!("{name} está desactivado.")));
        return;
    }
    ai_panel(app, dom, parent, info);
}

/// The AI copilot: the active provider, the quick actions, the conversation and the input.
fn ai_panel(app: &App, dom: &mut Dom, parent: NodeId, info: &mut ViewInfo) {
    let panel = dom.add(parent, El::div().class("ai-panel"));
    let bar = dom.add(panel, El::div().class("nav-buttons-container ai-toolbar"));
    let cfg = crate::ai::AiConfig::from_config(&app.config);
    let profile = cfg.active_profile();
    let pill = dom.add(
        bar,
        El::div()
            .class("ai-model ai-profile")
            .text(format!("{} · {}", profile.name, profile.model))
            .action("ai:profile-menu"),
    );
    dom.nodes[pill].el.attrs.push(("aria-label".into(), "Cambiar de perfil de IA".into()));
    let b = icon_button(dom, bar, "trash", "ai:clear", false);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Borrar la conversación".into()));
    let b = icon_button(dom, bar, "settings", "ai:settings", false);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Ajustes de la IA".into()));
    ai_body(app, dom, panel, info);
}

/// The copilot as a sheet over the workspace, for a compact window that has no column to give it:
/// a header of its own names the panel and the model in use and carries the way out, and the body
/// is the conversation the sidebar shows.
fn ai_sheet(app: &App, dom: &mut Dom, parent: NodeId, info: &mut ViewInfo) {
    // The sheet comes in from the right, on its own clock and the same curve as the sidebars. A
    // reader who asked for reduced motion never starts it, so the first frame is the last one.
    let off = (1.0 - app.anims.t("ai-sheet")) * app.window_w.max(1.0);
    // The sheet takes its height from the window the same way the workspace does: the engine does
    // not resolve `position: relative` on an ancestor, so an absolute child with `bottom: 0` would
    // stretch past the note and under the bottom navigation.
    let body_h = (app.window_h - STATUS_BAR_H - if app.compact { NAV_H } else { 0.0 }).max(0.0);
    let sheet = dom.add(
        parent,
        El::div()
            .class("ai-sheet")
            .style(&format!("top: 0; left: {off:.1}px; right: {:.1}px; height: {body_h:.0}px", -off)),
    );
    let head = dom.add(sheet, El::div().class("ai-sheet-header"));
    let mark = dom.add(head, El::div().class("ai-sheet-mark"));
    dom.add(mark, icons::el("ai"));
    dom.add(head, El::div().class("ai-sheet-title").text("Copiloto"));
    let cfg = crate::ai::AiConfig::from_config(&app.config);
    let profile = cfg.active_profile();
    let label = format!("{} · {}", profile.name, profile.model);
    // The model is the part that gives way: the title and the close button keep their room, so
    // at 360 px the profile ends in an ellipsis instead of pushing the way out of the sheet.
    let model = dom.add(head, El::div().class("ai-sheet-model").text(label.clone()));
    dom.nodes[model].el.attrs.push(("aria-label".into(), format!("Perfil de IA: {label}")));
    let close = dom.add(head, El::div().class("ai-sheet-close").action("ai:close"));
    dom.nodes[close].el.attrs.push(("aria-label".into(), "Cerrar el copiloto".into()));
    dom.add(close, icons::el("x"));
    ai_body(app, dom, sheet, info);
}

/// What both shapes of the copilot share: the quick actions, the conversation and the input.
/// `parent` is the flex column of the sidebar panel or of the sheet, so the input lands on the
/// last line of either one and never over the status bar.
fn ai_body(app: &App, dom: &mut Dom, parent: NodeId, info: &mut ViewInfo) {
    let quick = dom.add(parent, El::div().class("ai-quick"));
    for (label, action) in
        [("Explicar selección", "ai:explain-selection"), ("Resumir nota", "ai:summarize-note")]
    {
        dom.add(quick, El::div().class("ai-quick-button").text(label).action(action));
    }
    let list = dom.add(parent, El::div().class("ai-messages").id("ai-messages"));
    if app.ai.messages.is_empty() {
        empty_pane(dom, list, "ai", "Pregunta sobre la nota abierta. Las fórmulas se escriben con $...$.");
    }
    for m in &app.ai.messages {
        ai_message(dom, list, m, info);
    }
    if app.ai.streaming {
        let typing = dom.add(list, El::div().class("ai-typing"));
        // The lit dot moves on, so the three dots read as waiting even without CSS animations.
        let on = (vault_core::fsio::now_ms() / 250 % 3) as usize;
        for i in 0..3 {
            dom.add(typing, El::div().class("ai-typing-dot").class_if(i == on, "is-on"));
        }
    }
    if let Some(e) = &app.ai.error {
        dom.add(list, El::div().class("ai-error").text(e.clone()));
    }
    // What the copilot did to the vault this turn: the reader sees the function, what it was
    // asked and whether it worked. The writes are applied without asking, so this line is where
    // the turn explains itself.
    if !app.ai.tool_log.is_empty() {
        let log = dom.add(parent, El::div().class("ai-tool-log"));
        for (name, summary, ok) in &app.ai.tool_log {
            let row = dom
                .add(log, El::div().class("ai-tool-row").class_if(!ok, "is-error").action("ai:tools-open"));
            dom.add(row, icons::el(if *ok { "check" } else { "warning" }).class("ai-tool-icon"));
            dom.add(row, El::div().class("ai-tool-name").text(name.clone()));
            dom.add(row, El::div().class("ai-tool-summary").text(summary.clone()));
        }
    }
    let input = dom.add(parent, El::div().class("ai-input-container"));
    let empty = app.ai.input.is_empty();
    let text = if empty && !app.ai.editing {
        "Pregunta sobre la nota…".to_string()
    } else if app.ai.editing {
        with_caret(&app.ai.input, SIDE_FIELD_CHARS)
    } else {
        app.ai.input.clone()
    };
    dom.add(
        input,
        El::div()
            .class("ai-input")
            .class_if(empty && !app.ai.editing, "is-placeholder")
            .class_if(app.ai.editing, "is-editing")
            .text(text)
            .action("ai:focus-input"),
    );
    let send =
        dom.add(input, El::div().class("ai-send").class_if(app.ai.streaming, "is-busy").action("ai:send"));
    dom.nodes[send].el.attrs.push(("aria-label".into(), "Enviar".into()));
    dom.add(send, icons::el("send"));
}

fn ai_message(dom: &mut Dom, parent: NodeId, m: &crate::ai::Message, info: &mut ViewInfo) {
    let role = format!("ai-msg-{}", m.role);
    let row = dom.add(parent, El::div().class("ai-msg").class(&role));
    let head = dom.add(row, El::div().class("ai-msg-role"));
    dom.add(head, icons::el(if m.role == "user" { "edit" } else { "ai" }));
    let name = if m.role == "user" { "Tú" } else { "IA" };
    dom.add(head, El::div().class("ai-msg-role-name").text(name));
    let body = dom.add(row, El::div().class("ai-msg-content"));
    ai_markup(dom, body, &m.content, info);
}

/// A message body: prose in the flow, `$$...$$` formulas as display blocks and `$...$` ones
/// typeset inside their line.
fn ai_markup(dom: &mut Dom, parent: NodeId, text: &str, info: &mut ViewInfo) {
    for chunk in crate::ai::split_display_math(text) {
        match chunk {
            crate::ai::Chunk::Text(t) => {
                if t.trim().is_empty() {
                    continue;
                }
                let (shown, slots) = crate::rich::inline_text(&t, 1.0);
                let node = dom.add(parent, El::div().class("ai-msg-line").text(shown));
                if !slots.is_empty() {
                    info.inline.push(crate::rich::InlineItem { node, slots });
                }
            }
            crate::ai::Chunk::Display(src) => {
                if src.is_empty() {
                    continue;
                }
                crate::rich::add(dom, parent, crate::rich::Kind::Math { src, display: true }, &mut info.rich);
            }
        }
    }
}

fn outline(tab: &crate::shell::Tab, dom: &mut Dom, parent: NodeId) {
    section_header(dom, parent, "list", "Esquema");
    let outline = dom.add(parent, El::div().class("pane-section"));
    for h in tab.meta.iter().flat_map(|(_, m)| &m.headings) {
        dom.add(
            outline,
            El::div()
                .class("tree-item-self")
                .style(&format!("padding-left: {}px", 8 + (h.level as usize - 1) * 12))
                .text(h.text.to_string()),
        );
    }
}

fn outgoing(app: &App, tab: &crate::shell::Tab, dom: &mut Dom, parent: NodeId) {
    section_header(dom, parent, "arrow-up-right", "Enlaces salientes");
    let list = dom.add(parent, El::div().class("pane-section"));
    let mut seen = std::collections::HashSet::new();
    for l in tab.meta.iter().flat_map(|(_, m)| &m.links) {
        let target = app.store.index.resolve(&l.target, &tab.path).and_then(|id| app.store.index.entry(id));
        let key = target.map_or_else(|| l.target.to_lowercase(), |f| f.path.to_lowercase());
        if l.target.is_empty() || !seen.insert(key) {
            continue;
        }
        let (label, action) = match target {
            Some(f) => (path::strip_md(&f.path).to_string(), format!("open:{}", f.path)),
            None => (l.target.to_string(), format!("create:{}", l.target)),
        };
        dom.add(
            list,
            El::div()
                .class("tree-item-self")
                .class_if(target.is_none(), "is-unresolved")
                .text(label)
                .action(action),
        );
    }
    if seen.is_empty() {
        empty_pane(dom, list, "arrow-up-right", "No hay enlaces salientes.");
    }
}

fn backlinks(app: &App, dom: &mut Dom, parent: NodeId) {
    let bar = dom.add(parent, El::div().class("nav-buttons-container"));
    let b = icon_button(dom, bar, "collapse", "backlinks:toggle-collapse", app.backlinks_collapsed);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Contraer resultados".into()));
    let b = icon_button(dom, bar, "context", "backlinks:toggle-context", app.backlinks_more);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Mostrar más contexto".into()));
    let b = icon_button(dom, bar, "sort", "backlinks:toggle-sort", app.backlinks_desc);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Cambiar orden".into()));
    let b = icon_button(dom, bar, "search", "backlinks:toggle-filter", app.backlinks_filter.is_some());
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Mostrar filtro de búsqueda".into()));
    if let Some(f) = &app.backlinks_filter {
        let (text, faint) = match (f.is_empty(), app.backlinks_editing) {
            (_, true) => (with_caret(f, SIDE_FIELD_CHARS), false),
            (true, false) => ("Filtrar…".to_string(), true),
            (false, false) => (f.clone(), false),
        };
        let field = dom.add(
            parent,
            El::div()
                .class("graph-input backlinks-filter")
                .class_if(app.backlinks_editing, "is-editing")
                .action("backlinks:edit-filter"),
        );
        dom.add(field, icons::el("search"));
        dom.add(field, El::div().class("graph-input-text").class_if(faint, "is-placeholder").text(text));
    }
    let needle = app.backlinks_filter.as_deref().unwrap_or("").to_lowercase();
    let files: Vec<(&String, Vec<&String>)> = app
        .linked
        .as_ref()
        .map(|m| m.files.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|(p, snippets)| {
            if needle.is_empty() || p.to_lowercase().contains(&needle) {
                return Some((p, snippets.iter().collect()));
            }
            let hits: Vec<&String> = snippets.iter().filter(|s| s.to_lowercase().contains(&needle)).collect();
            (!hits.is_empty()).then_some((p, hits))
        })
        .collect();
    let header = dom.add(parent, El::div().class("nav-header"));
    dom.add(header, El::div().class("nav-header-title").text("Menciones vinculadas"));
    dom.add(header, El::div().class("tree-item-flair").text(files.len().to_string()));
    let back = dom.add(parent, El::div().class("pane-section"));
    if files.is_empty() {
        empty_pane(
            dom,
            back,
            "link",
            if needle.is_empty() {
                "Ninguna nota enlaza a esta todavía.".to_string()
            } else {
                format!("Ningún retroenlace coincide con «{needle}».")
            },
        );
    }
    let ordered: Box<dyn Iterator<Item = _>> =
        if app.backlinks_desc { Box::new(files.iter().rev()) } else { Box::new(files.iter()) };
    for (p, snippets) in ordered {
        let row = dom.add(back, El::div().class("tree-item-self").action(format!("open:{p}")));
        let p = p.as_str();
        dom.add(row, El::div().class("tree-item-inner").text(path::strip_md(p).to_string()));
        dom.add(row, El::div().class("tree-item-flair").text(snippets.len().to_string()));
        if app.backlinks_collapsed {
            continue;
        }
        for s in snippets {
            dom.add(
                back,
                El::div()
                    .class("search-result-file-match")
                    .class_if(app.backlinks_more, "is-wrapped")
                    .text(s.to_string())
                    .action(format!("open:{p}")),
            );
        }
    }
    let header = dom.add(parent, El::div().class("nav-header").action("backlinks:toggle-unlinked"));
    dom.add(header, icons::el(if app.show_unlinked { "chevron-down" } else { "chevron-right" }));
    dom.add(header, El::div().class("nav-header-title").text("Menciones sin vínculo"));
    if !app.show_unlinked {
        return;
    }
    let list = dom.add(parent, El::div().class("pane-section"));
    let files = app.mentions.as_ref().map(|m| m.files.as_slice()).unwrap_or(&[]);
    if files.is_empty() {
        empty_pane(dom, list, "link", "No hay menciones sin vínculo.");
    }
    for (p, snippets) in files {
        let row = dom.add(list, El::div().class("tree-item-self bookmark").action(format!("open:{p}")));
        dom.add(row, El::div().class("tree-item-inner").text(path::strip_md(p).to_string()));
        dom.add(
            row,
            El::div()
                .class("tree-item-flair clickable-icon")
                .text("Vincular")
                .action(format!("backlinks:link:{p}")),
        );
        for s in snippets {
            dom.add(
                list,
                El::div().class("search-result-file-match").text(s.clone()).action(format!("open:{p}")),
            );
        }
    }
}

/// The editor area: panes laid out by `node`, whose position in the layout tree is `path`.
#[allow(clippy::too_many_arguments)]
fn split_layout(
    app: &App,
    dom: &mut Dom,
    parent: NodeId,
    node: &crate::split::Layout,
    path: &mut Vec<usize>,
    pal: &Palette,
    info: &mut ViewInfo,
    viewport_h: f32,
    width: f32,
) {
    let crate::split::Layout::Split { row, kids } = node else {
        if let crate::split::Layout::Leaf(i) = node {
            pane(app, dom, parent, *i, pal, info, viewport_h, width);
        }
        return;
    };
    let class = if *row { "workspace-split mod-vertical" } else { "workspace-split mod-horizontal" };
    let split = dom.add(parent, El::div().class(class));
    let (dim, handle) = if *row { ("width", "mod-vertical") } else { ("height", "mod-horizontal") };
    let at = path.iter().map(usize::to_string).collect::<Vec<_>>().join(".");
    for (k, (kid, ratio)) in kids.iter().enumerate() {
        if k > 0 {
            // The divider being dragged turns accent-colored, so the drag has visible feedback.
            let dragging = app.pane_drag.as_ref().is_some_and(|d| {
                d.k == k - 1 && d.path.iter().map(usize::to_string).collect::<Vec<_>>().join(".") == at
            });
            let el = El::div()
                .class("workspace-leaf-resize-handle")
                .class(handle)
                .class_if(dragging, "is-dragging");
            dom.add(split, el.action(format!("pane-resize:{}:{at}", k - 1)));
        }
        let cell = El::div().class("workspace-split-cell").style(&format!("{dim}: {:.3}%", ratio * 100.0));
        let cell = dom.add(split, cell);
        path.push(k);
        let w = if *row { width * ratio } else { width };
        split_layout(app, dom, cell, kid, path, pal, info, viewport_h, w);
        path.pop();
    }
}

/// Pane `pane`, about `width` wide: its tab bar and the active tab, or all its tabs when
/// stacked. Only the focused pane shows the graph or canvas.
#[allow(clippy::too_many_arguments)]
fn pane(
    app: &App,
    dom: &mut Dom,
    parent: NodeId,
    pane: usize,
    pal: &Palette,
    info: &mut ViewInfo,
    viewport_h: f32,
    width: f32,
) {
    let focused = pane == app.pane;
    let leaf = dom.add(parent, El::div().class("workspace-leaf").class_if(focused, "mod-active"));
    let tabs = dom.add(leaf, El::div().class("workspace-tab-header-container"));
    let tab_header =
        |dom: &mut Dom, icon: &str, title: &str, dirty, active, focus: String, close: (String, &str)| {
            let tab = dom.add(
                tabs,
                El::div()
                    .class("workspace-tab-header")
                    .class_if(active, "is-active")
                    .id(&format!("tab-{pane}-{}", dom.nodes[tabs].children.len()))
                    .action(focus),
            );
            dom.add(tab, icons::el(icon).class("workspace-tab-header-inner-icon"));
            dom.add(tab, El::div().class("workspace-tab-header-inner-title").text(title.to_string()));
            if dirty {
                dom.add(tab, icons::el("dot").class("workspace-tab-header-status"));
            }
            // A pinned tab shows a pin in place of the close button; clicking it unpins.
            let x = dom.add(tab, El::div().class("workspace-tab-header-inner-close-button").action(close.0));
            dom.add(x, icons::el(close.1));
        };
    let (open, active) = app.pane_tabs(pane);
    let covered = focused
        && (app.show_graph
            || app.canvas_active()
            || app.base_active()
            || app.exc_active()
            || app.kanban_active()
            || app.calendar_view_active());
    for (i, t) in open.iter().enumerate() {
        tab_header(
            dom,
            icons::for_file(&t.path),
            t.title(),
            t.editor.buf.is_modified(),
            i == active && !covered,
            format!("tab:{i}"),
            if t.pinned { (format!("toggle-pin:{i}"), "pin") } else { (format!("close-tab:{i}"), "x") },
        );
    }
    if let Some(g) = app.graph_view.as_ref().filter(|_| focused) {
        let (focus, close) = ("graph:focus".to_string(), ("graph:close".to_string(), "x"));
        tab_header(dom, "graph", &g.title(), false, app.show_graph, focus, close);
    }
    if let Some(c) = app.canvas_view.as_ref().filter(|_| focused) {
        let (focus, close) = ("canvas:focus".to_string(), ("canvas:close".to_string(), "x"));
        tab_header(dom, "canvas", &c.title(), false, app.canvas_active(), focus, close);
    }
    if let Some(b) = app.base_view.as_ref().filter(|_| focused) {
        let (focus, close) = ("base:focus".to_string(), ("base:close".to_string(), "x"));
        tab_header(dom, "table", &b.title(), false, app.base_active(), focus, close);
    }
    if let Some(e) = app.exc_view.as_ref().filter(|_| focused) {
        let (focus, close) = ("exc:focus".to_string(), ("exc:close".to_string(), "x"));
        tab_header(dom, "edit", &e.title(), e.dirty, app.exc_active(), focus, close);
    }
    if let Some(k) = app.kanban_view.as_ref().filter(|_| focused) {
        let (focus, close) = ("kanban:focus".to_string(), ("kanban:close".to_string(), "x"));
        tab_header(dom, "table", &k.title(), k.dirty, app.kanban_active(), focus, close);
    }
    if let Some(c) = app.calendar_view.as_ref().filter(|_| focused) {
        let (focus, close) = ("calview:focus".to_string(), ("calview:close".to_string(), "x"));
        tab_header(dom, "calendar", &c.title(), false, app.calendar_view_active(), focus, close);
    }
    let new_tab =
        dom.add(tabs, El::div().class("workspace-tab-header-new-tab").action("file-explorer:new-file"));
    dom.add(new_tab, icons::el("plus"));
    // A divided window lets each pane go from its own bar: the button is the command the tab
    // menu offers, and it only exists while there is more than one panel to choose between.
    if app.panes.len() > 1 {
        let close = dom.add(
            tabs,
            El::div().class("workspace-tab-header-close-pane").action(format!("pane-close:{pane}")),
        );
        dom.nodes[close].el.attrs.push(("aria-label".into(), "Cerrar panel".into()));
        dom.add(close, icons::el("x"));
    }
    if app.panes[pane].stacked && !covered {
        stack(app, dom, leaf, pane, focused, pal, info, width);
        info.panes.push((pane, leaf, None));
        return;
    }
    if !focused {
        let mut own = ViewInfo::default();
        editor(app, dom, leaf, pal, &mut own, open.get(active).map(|t| (pane, active, t)), false);
        info.rich.append(&mut own.rich);
        info.exc.append(&mut own.exc);
        info.scrolled.append(&mut own.scrolled);
        info.panes.push((pane, leaf, own.editor));
        return;
    }
    match &app.graph_view {
        Some(g) if app.show_graph => graph(g, dom, leaf, info, viewport_h),
        _ if app.canvas_active() => {
            crate::canvasview::pane(app.canvas_view.as_ref().unwrap(), dom, leaf, info)
        }
        _ if app.base_active() => crate::baseview::pane(app.base_view.as_ref().unwrap(), dom, leaf),
        _ if app.exc_active() => crate::excalidrawview::pane(app.exc_view.as_ref().unwrap(), dom, leaf, info),
        _ if app.kanban_active() => {
            crate::kanbanview::pane(app.kanban_view.as_ref().unwrap(), dom, leaf, info, app.compact)
        }
        _ if app.calendar_view_active() => {
            crate::calendarview::pane(app, dom, leaf);
        }
        _ if crate::dashboard::is_dashboard(app) => crate::dashboard::pane(app, dom, leaf, width),
        _ if crate::finance::is_finance(app) => {
            crate::finance::pane(app, dom, leaf, width);
        }
        _ if crate::gantt::is_gantt(app) => {
            crate::gantt::pane(app, dom, leaf);
        }
        _ => editor(app, dom, leaf, pal, info, app.tab().map(|t| (pane, app.active, t)), true),
    }
    info.panes.push((pane, leaf, None));
}

/// `tab` in `parent`. Only the focused pane's editor takes input.
fn editor(
    app: &App,
    dom: &mut Dom,
    parent: NodeId,
    pal: &Palette,
    info: &mut ViewInfo,
    tab: Option<(usize, usize, &crate::shell::Tab)>,
    focused: bool,
) {
    let Some((pane, tab_i, tab)) = tab else {
        empty_state(app, dom, parent);
        return;
    };
    let mode_label = match tab.mode {
        ViewMode::Source => "fuente",
        ViewMode::LivePreview => "vista previa",
        ViewMode::Reading => "lectura",
    };
    let header = dom.add(
        parent,
        El::div()
            .class("view-header")
            .class_if(!app.config.appearance.bool("showViewHeader", true), "is-hidden"),
    );
    icon_button(dom, header, "arrow-left", "app:go-back", false);
    icon_button(dom, header, "arrow-right", "app:go-forward", false);
    dom.add(
        header,
        El::div().class("view-header-title").text(format!("{}  ·  {mode_label}", path::strip_md(&tab.path))),
    );
    let (mode_icon, source_icon) = (if tab.mode == ViewMode::Reading { "edit" } else { "book" }, "code");
    icon_button(dom, header, source_icon, "editor:toggle-source", tab.mode == ViewMode::Source);
    icon_button(dom, header, mode_icon, "markdown:toggle-preview", false);
    icon_button(dom, header, "sidebar-right", "app:toggle-right-sidebar", app.right_open);
    let class = view_classes(app, tab.mode);
    let mut view_el = El::div().class(&class);
    if focused {
        view_el = view_el.id("editor");
    }
    let view = dom.add(parent, view_el);
    info.editor = Some(view);
    let doc = tab.editor.buf.doc();
    // Virtualised: only blocks around the scroll position, capped. A few above it are laid out
    // so the view can scroll up smoothly.
    let top = tab.top_block.min(doc.blocks.len().saturating_sub(1));
    let first = top.saturating_sub(8);
    let last = (top + 90).min(doc.blocks.len());
    // The part of the note the view does not build keeps its room, and that room is what makes
    // the scroll limit the note's and not the window's. The heights come from `Extent`, measured
    // from the blocks themselves; a note not measured yet has no room and scrolls as it always did.
    let room = |h: f32| if h > 0.5 { h } else { 0.0 };
    let above = room(tab.extent.offset(first));
    if above > 0.0 {
        dom.add(view, El::div().class("cm-offscreen").style(&format!("height: {above:.1}px")));
    }
    info.marks.clear();
    note_blocks(app, dom, view, pal, info, tab, tab.mode, focused, first..last, false);
    let below = room(tab.extent.total() - tab.extent.offset(last));
    if below > 0.0 {
        dom.add(view, El::div().class("cm-offscreen").style(&format!("height: {below:.1}px")));
    }
    let added = dom.nodes.len();
    let mut marks: Vec<(usize, NodeId)> = Vec::new();
    // A block that added nothing shares its mark with the next one.
    for (b, n) in info.marks.drain(..).filter(|&(_, n)| n < added) {
        if marks.last().is_some_and(|m| m.1 == n) {
            marks.pop();
        }
        marks.push((b, n));
    }
    info.scrolled.push(Scrolled {
        pane,
        tab: tab_i,
        view,
        marks,
        start: first == 0,
        end: last == doc.blocks.len(),
        blocks: doc.blocks.len(),
        below,
    });
}

/// The block the caret of the focused note is in, if the note has one.
fn caret_block(app: &App) -> Option<usize> {
    let t = app.tabs.get(app.active)?;
    let head = t.editor.buf.sels.last().map_or(0, |s| s.head);
    t.editor.buf.doc().block_index_at(head)
}

/// The mark of the block a caret is in: the block itself, or the one after it when the caret is on
/// a block that added no line of its own (a blank line nobody is drawing a caret on).
fn caret_mark(marks: &[(usize, NodeId)], block: usize) -> Option<usize> {
    marks.iter().position(|&(b, _)| b >= block).or_else(|| marks.len().checked_sub(1))
}

/// Scroll each note view to its tab's position, now that its blocks have their heights, and
/// anchor the tab to the block at the top of the view.
pub fn scroll_views(app: &mut App, dom: &mut Dom, info: &ViewInfo) {
    // A stored scroll that no longer fits its content (a list that lost rows, a page that got
    // shorter) would leave the box showing blank space past its end, with the thumb already at
    // the bottom: the map keeps the same limit the bar is drawn from.
    let stored: Vec<(String, f32)> = app
        .scrolls
        .iter()
        .filter_map(|(id, &y)| {
            let n = dom.by_id(id)?;
            let max = dom.max_scroll(n);
            (y > max + 0.5).then(|| (id.clone(), max))
        })
        .collect();
    for (id, max) in stored {
        if let Some(n) = dom.by_id(&id) {
            dom.set_scroll(n, max);
        }
        app.scrolls.insert(id, max);
    }
    // The caret of the focused note, for the frame the keyboard moved it out of the view.
    let caret = if info.scrolled.iter().any(|s| s.pane == app.pane && s.tab == app.active) {
        caret_block(app)
    } else {
        None
    };
    for s in &info.scrolled {
        let (pane, tab) = (s.pane, s.tab);
        let Some(t) = app.pane_tab(pane, tab) else { continue };
        if s.marks.is_empty() {
            if let Some(t) = app.pane_tab_mut(pane, tab) {
                t.scroll_to_block(0);
                t.caret_jump = false;
            }
            continue;
        }
        // An anchor past the end of the note (an edit that shortened it, a saved session from a
        // longer version) would land on the last block laid out and open the note at its end,
        // with its first lines scrolled off the top. The last block is as far as it can go.
        let stale = t.top_block >= s.blocks;
        let top_block = if stale { s.blocks.saturating_sub(1) } else { t.top_block };
        let Some(t) = app.pane_tab(pane, tab) else { continue };
        let top = dom.content_rect(s.view).y;
        // Where each block of the note begins, in the note's own pixels: the room the view left
        // for the blocks it did not build is part of the count, so this is the document's offset
        // and not the window's. The margin above a block belongs to the note and not to its box.
        let pos: Vec<f32> =
            s.marks.iter().map(|&(_, n)| dom.nodes[n].rect.y - dom.nodes[n].style.margin[0] - top).collect();
        let anchor = s.marks.iter().position(|&(b, _)| b >= top_block).unwrap_or(s.marks.len() - 1);
        let max = dom.max_scroll(s.view);
        let want = pos[anchor] + t.scroll_px;
        let mut y = want;
        let mut jumped = false;
        if s.start {
            // The first line of the note is never scrolled above the top of the view.
            y = y.max(0.0);
        }
        if s.end {
            y = y.min(max);
        }
        // The keyboard moved the caret: the block it is in has to be on screen, and a count of
        // blocks was never the way to know where that is. A block taller than the view shows from
        // its top, which is where the first line of what the caret is on sits anyway.
        if t.caret_jump && pane == app.pane && tab == app.active {
            jumped = true;
            if let Some(k) = caret.and_then(|c| caret_mark(&s.marks, c)) {
                let (block_top, block_h) = (pos[k], dom.nodes[s.marks[k].1].rect.h);
                let box_h = dom.content_rect(s.view).h;
                if y > block_top || block_top + block_h > y + box_h {
                    y = if block_h >= box_h { block_top } else { (block_top + block_h - box_h).max(0.0) };
                }
            }
        }
        if y != want {
            // Ran into the start or end of the note.
            if let Some(t) = app.pane_tab_mut(pane, tab) {
                t.scroll_pending = 0.0;
            }
        }
        let y = y.clamp(0.0, max);
        dom.set_scroll(s.view, y);
        // The blocks this frame laid out, at the height the layout gave them: an edit, a theme or
        // an image that landed moves the note's own height with it without waiting for the pass.
        let view = &dom.nodes[s.view];
        let end_of_window = (view.content_h - view.style.padding[2] - s.below).max(0.0);
        let mut laid_out: Vec<(usize, f32)> = Vec::with_capacity(s.marks.len());
        for (k, &(b, _)) in s.marks.iter().enumerate() {
            let end = pos.get(k + 1).copied().unwrap_or(end_of_window);
            laid_out.push((b, (end - pos[k]).max(0.0)));
        }
        let k = pos.iter().rposition(|&p| p <= y).unwrap_or(0);
        if let Some(t) = app.pane_tab_mut(pane, tab) {
            if stale {
                t.scroll_to_block(top_block);
            }
            for (b, h) in laid_out {
                t.extent.put(b, h);
            }
            t.extent.recount();
            t.top_block = s.marks[k].0;
            t.scroll_px = y - pos[k];
            if jumped {
                t.caret_jump = false;
            }
        }
    }
    // The AI conversation follows the answer as it streams in.
    if app.ai.scroll_to_end {
        if let Some(n) = dom.by_id("ai-messages") {
            let max = dom.max_scroll(n);
            dom.set_scroll(n, max);
            app.scrolls.insert("ai-messages".to_string(), max);
        }
        app.ai.scroll_to_end = false;
    }
    // The palette's list follows the keyboard. It scrolls to the row the selection moved to and
    // leaves the offset alone the rest of the time, so the wheel keeps what the reader asked for.
    if let Some(p) = &mut app.prompt {
        if let Some(sel) = p.scroll_to.take() {
            if let Some(list) = dom.by_id("prompt-results") {
                if let Some(&row) = dom.nodes[list].children.get(sel) {
                    let view = dom.content_rect(list);
                    let r = dom.nodes[row].rect;
                    let cur = dom.nodes[list].scroll_y;
                    let want = if r.y < view.y {
                        cur - (view.y - r.y)
                    } else if r.y + r.h > view.y + view.h {
                        cur + (r.y + r.h - view.y - view.h)
                    } else {
                        cur
                    };
                    if (want - cur).abs() > 0.5 {
                        let y = want.clamp(0.0, dom.max_scroll(list));
                        dom.set_scroll(list, y);
                        app.scrolls.insert("prompt-results".to_string(), y);
                    }
                }
            }
        }
    }
}

/// The mark under the tab that has the focus, drawn after layout because it is measured from the
/// boxes the tab bar really got. It slides from the tab that had the focus to the one that took
/// it, so the change of tab reads as one thing moving instead of two tabs blinking.
pub fn tab_indicator(app: &App, dom: &Dom, info: &ViewInfo) -> Option<metamind_ui::Cmd> {
    let (pane, to) = (app.pane, app.active);
    let node = dom.by_id(&format!("tab-{pane}-{to}"))?;
    let n = &dom.nodes[node];
    // Only the tab that is really the active one wears the mark: a pane covered by the graph or
    // a board keeps its tabs but none of them is the one in front.
    if !n.el.classes.iter().any(|c| c == "is-active") {
        return None;
    }
    // A window over the app covers the tab bar: its mark has no business on top of the panel.
    if app.settings.is_some()
        || app.prompt.is_some()
        || app.diagram_zoom.is_some()
        || app.video_fullscreen
        || app.more_open
    {
        return None;
    }
    let t = app.anims.t("tab");
    let (x, w) = match app.tab_from.filter(|_| t < 1.0) {
        Some((fp, fi)) if (fp, fi) != (pane, to) => {
            let from = &dom.nodes[dom.by_id(&format!("tab-{fp}-{fi}"))?].rect;
            (from.x + (n.rect.x - from.x) * t, from.w + (n.rect.w - from.w) * t)
        }
        _ => (n.rect.x, n.rect.w),
    };
    // Inset from the corners of the tab, two pixels tall, sitting on the line that closes the bar,
    // and never past the bar: a tab scrolled half out of it used to leave its mark hanging over
    // whatever was drawn beside the tab strip.
    let h = 2.0;
    let mut cur = Some(node);
    let mut bar = None;
    while let Some(id) = cur {
        if dom.nodes[id].el.classes.iter().any(|c| c == "workspace-tab-header-container") {
            bar = Some(dom.nodes[id].rect);
            break;
        }
        cur = dom.nodes[id].parent;
    }
    let bar = bar?;
    let (lo, hi) = (bar.x, bar.x + bar.w);
    let x0 = (x + 5.0).max(lo);
    let x1 = (x + w - 5.0).min(hi);
    if x1 - x0 < 4.0 {
        return None;
    }
    Some(metamind_ui::Cmd::Fill {
        rect: metamind_ui::Rect::new(x0, n.rect.y + n.rect.h - h, x1 - x0, h),
        color: info.accent,
        radius: 1.0,
    })
}

/// Add the note's blocks `range` to `view` as `mode` shows them. Without `cursor` the lines
/// under the selection are rendered too. `print` lays the note out for export: folds are open
/// and there are no fold markers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn note_blocks(
    app: &App,
    dom: &mut Dom,
    view: NodeId,
    pal: &Palette,
    info: &mut ViewInfo,
    tab: &crate::shell::Tab,
    mode: ViewMode,
    cursor: bool,
    range: std::ops::Range<usize>,
    print: bool,
) {
    let buf = &tab.editor.buf;
    let doc = buf.doc();
    let (first, last) = (range.start, range.end);
    let sels: &[Sel] = if mode == ViewMode::Reading || !cursor { &[] } else { &buf.sels };
    let deco_mode = if mode == ViewMode::Source {
        editor_core::deco::Mode::Source
    } else {
        editor_core::deco::Mode::LivePreview
    };
    let decos = decorate(doc, first..last, sels, deco_mode);
    for (i, b) in doc.blocks[first..last].iter().enumerate() {
        info.marks.push((first + i, dom.nodes.len()));
        let root = b.tree.root();
        // A callout marked Closed by the source is shown folded, and one that is open folds, the
        // moment the reader clicks its title: `callout_toggled` is that override, per note.
        let callout_key = (tab.path.clone(), b.start);
        let callout_folded = matches!(root.kind, NodeKind::Callout { fold: md_engine::Fold::Closed, .. })
            != app.callout_toggled.contains(&callout_key);
        let (s, e) = (b.start + root.span.start as usize, b.start + root.span.end as usize);
        let class = match root.kind {
            NodeKind::Blank => {
                // Keep an empty line editable when the cursor is on it.
                if !sels.iter().any(|x| x.head >= b.start && x.head <= b.end()) {
                    continue;
                }
                "cm-line"
            }
            NodeKind::CodeBlock { .. } | NodeKind::MathBlock { .. } => "cm-line cm-codeblock",
            // Raw HTML (an `<svg>` block, a `<div>` layout) is shown as code: it is source, not
            // prose, and a line of it wrapped as prose breaks tags in the middle.
            NodeKind::HtmlBlock => "cm-line cm-codeblock cm-htmlblock",
            NodeKind::BlockQuote => "cm-line cm-quote",
            NodeKind::Callout { .. } => "cm-line cm-callout",
            NodeKind::FrontMatter { .. } => "cm-line cm-frontmatter",
            NodeKind::ThematicBreak
                if mode != ViewMode::Source && !sels.iter().any(|x| x.head >= s && x.head <= e) =>
            {
                "cm-line cm-hr"
            }
            // A `%%…%%` block is Obsidian's comment: hidden while reading *and* in the live
            // preview, unless the caret is inside it. The Excalidraw plugin keeps a whole scene
            // there, and showing it is a wall of JSON in the middle of the note.
            NodeKind::CommentBlock
                if mode != ViewMode::Source && !sels.iter().any(|x| x.head >= s && x.head <= e) =>
            {
                continue
            }
            _ => "cm-line",
        };
        // Folded text: blocks inside a fold are skipped; a fold inside a block hides part of it.
        let hidden: &[(usize, usize)] = if print { &[] } else { &tab.hidden.1 };
        if hidden.iter().any(|&(hs, he)| hs < s && he >= e) {
            continue;
        }
        let folded_here = hidden.iter().any(|&(hs, _)| hs >= s && hs <= e);
        let rendered = match mode {
            ViewMode::Source => false,
            ViewMode::Reading => true,
            // A heading is rendered whatever the caret is doing: `## 11.` next to a rendered `10.`
            // is the same level drawn two ways, and the `#`s are markup, not content.
            ViewMode::LivePreview => {
                matches!(root.kind, NodeKind::Heading { .. })
                    || !sels.iter().any(|x| x.start() <= e && x.end() >= s)
            }
        };
        let src = buf.slice(s, e);
        if rendered
            && matches!(root.kind, NodeKind::FrontMatter { .. })
            && crate::properties::render(app, dom, view, mode, &src)
        {
            continue;
        }
        let mut block_decos: Vec<Deco> = decos.iter().filter(|d| d.start < e && d.end > s).copied().collect();
        for &(hs, he) in hidden.iter().filter(|&&(hs, he)| hs < e && he > s) {
            block_decos.push(Deco { start: hs.max(s), end: he.min(e), style: Deco_::Hide });
        }
        if rendered && matches!(root.kind, NodeKind::Table { .. }) {
            table(dom, view, pal, &src, s, &b.tree, b.start, first + i, &block_decos, info);
            continue;
        }
        // A `dataview` fence shows its result in place of the source; when the query cannot be
        // read the source stays and a notice says why.
        let mut dataview_error = None;
        if rendered {
            if let Some(query) = crate::dataview::query_source(&b.tree, &src, root.span.start) {
                match crate::dataview::block(app, dom, view, &query) {
                    Ok(()) => continue,
                    Err(e) => dataview_error = Some(format!("Dataview: {e}")),
                }
            }
        }
        if rendered {
            if let Some(kind) = crate::rich::block_kind(&b.tree, &src, root.span.start) {
                // A double click on a diagram swaps the drawing for its source, in place.
                let as_source = matches!(kind, crate::rich::Kind::Mermaid(_))
                    && app.mermaid_source.contains(&(tab.path.clone(), b.start));
                if !as_source {
                    let node = crate::rich::add(dom, view, kind, &mut info.rich);
                    let bv = BlockView {
                        block: first + i,
                        node,
                        src_start: s,
                        src_end: e,
                        map: vec![(s, 0, 0)],
                        ..BlockView::default()
                    };
                    info.blocks.push(bv);
                    continue;
                }
            }
        }
        let mut callout: Option<(Color, usize, usize)> = None;
        // Whether the callout was given a title of its own, which is what can be clicked to fold
        // it: an untitled one shows its type, which is part of the body's own line.
        let mut callout_titled = false;
        if rendered && matches!(root.kind, NodeKind::BlockQuote | NodeKind::Callout { .. }) {
            hide_quote_markers(&src, s, &mut block_decos);
            if let NodeKind::Callout { kind, title, .. } = root.kind {
                let first_end = s + src.find('\n').unwrap_or(src.len());
                let (ks, ke) = (b.start + kind.start as usize, b.start + kind.end as usize);
                let (ts, te) = (b.start + title.start as usize, b.start + title.end as usize);
                // Show the title, or the callout type when there is no title.
                let (shown_s, shown_e) = if te > ts { (ts, te) } else { (ks, ke) };
                let hide = |a: usize, b: usize| Deco { start: a, end: b, style: Deco_::Hide };
                block_decos.push(hide(s, shown_s));
                block_decos.push(hide(shown_e, first_end));
                block_decos.push(Deco { start: shown_s, end: shown_e, style: Deco_::Strong });
                // The body of a folded callout is not on screen, whatever the source says: the
                // reader's toggle is what decides, and it is the same state the click writes.
                if callout_folded {
                    block_decos.push(hide(first_end, e));
                }
                callout_titled = te > ts;
                callout = Some((callout_color(&buf.slice(ks, ke)), shown_s, shown_e));
            }
        }
        // The `#`s of a heading are markup, not content: they stay out of the way unless the
        // caret is inside the heading, which is also when its source is shown to edit it.
        let marker = if matches!(root.kind, NodeKind::Heading { setext: false, .. }) {
            heading_marker(&src)
        } else {
            0
        };
        if marker > 0 {
            // The block shows its source exactly while the caret is in it, which is also when
            // the `#`s have to come back: hiding them there would leave a heading with no way
            // to see or edit its level.
            let style = if rendered { Deco_::Hide } else { Deco_::Markup };
            block_decos.push(Deco { start: s, end: s + marker, style });
        }
        // Inline math is typeset in the flow of its own line, never in a row of its own: each
        // `$..$` gets blanks the width of the formula in the text, and the painter fills them in
        // once the line has real metrics. While the caret is in the block its source shows, as
        // tables and fences do.
        // A closed callout shows only its title: the inline math of the hidden content
        // would still be collected and painted below the truncated text, so skip it.
        let mut math = if rendered && !class.ends_with("cm-hr") && !callout_folded {
            crate::rich::inline(&b.tree, &buf.slice(b.start, b.end()), block_scale(&block_decos, pal))
        } else {
            Vec::new()
        };
        // The formulas' ranges are block-local; the display is built in document offsets.
        for m in &mut math {
            m.start += b.start;
            m.end += b.start;
        }
        math.retain(|m| m.start >= s && m.end <= e && m.end > m.start);
        let (mut display, mut spans, mut map) = if class.ends_with("cm-hr") {
            (String::new(), Vec::new(), Vec::new())
        } else {
            block_display(&src, s, &block_decos, &math, pal, matches!(root.kind, NodeKind::HtmlBlock))
        };
        // A container's body may end with an empty line (a `>` alone): it is not content, and an
        // empty paragraph underneath a callout makes its box taller than the one beside it.
        if rendered && matches!(root.kind, NodeKind::Callout { .. } | NodeKind::BlockQuote) {
            trim_empty_lines(&mut display);
            spans.retain(|sp| sp.start < display.len());
            map.retain(|&(_, d, len)| d < display.len() && len > 0);
        }
        if marker > 0 && !rendered {
            // Shown to edit it: at the size of the text, not of the heading.
            for sp in spans.iter_mut().filter(|sp| sp.end <= marker) {
                sp.scale = 1.0;
            }
        }
        // The source's soft wraps are single spaces on screen: one paragraph, wrapped by the
        // layout at the width it has.
        if rendered {
            if let Some((one_line, table)) = join_soft_breaks(&display, callout.is_some()) {
                display = one_line;
                remap_lines(&mut spans, &mut map, &table);
            }
        }
        mark_symbols(&display, &mut spans, pal);
        if folded_here {
            let at = display.len();
            display.push_str("  ⋯");
            spans.push(Span {
                start: at,
                end: display.len(),
                color: Some(pal.faint),
                scale: 1.0,
                ..Span::default()
            });
        }
        // A rendered list is drawn item by item: a single text run cannot hang the wrapped
        // lines under the item's text, and the bullet would be the source's own `-`. A callout
        // with a title of its own is drawn that way too, list or not: the title has to be a line
        // of its own to fold and unfold it, while a click on the body still places the caret.
        // A container that holds a display formula is split as well: the formula gets a line of
        // its own, and no text may be laid out beside it.
        let display_math = display.split('\n').any(is_filler_line);
        let split_callout = display.contains('\n') && (callout_titled || display_math);
        if rendered && !folded_here && (draws_list_lines(&root.kind, &display) || split_callout) {
            let mut callout_lines = cut_lines(&display, &spans, &map);
            // A closed callout shows its title and nothing else: `list_block` gives every line
            // its own child node, which the block's hide decorations would not reach.
            if callout_folded {
                callout_lines.truncate(1);
            }
            let callout_node = list_block(dom, view, first + i, callout_lines, callout, &math, info);
            // A click on the callout's title folds or unfolds it, in the reading view as well as
            // in live preview: the action sits on the title line (which is the whole box while
            // the body is hidden) and never on the body, where a click has to place the caret.
            if callout.is_some() {
                let title = if callout_folded {
                    Some(callout_node)
                } else {
                    dom.nodes[callout_node].children.first().copied()
                };
                if let Some(n) = title.filter(|&n| n != view) {
                    dom.nodes[n].el.action = Some(format!("callout:toggle:{}", b.start));
                }
            }
        } else {
            let mut el =
                El::div().class(class).attr("data-block", &(first + i).to_string()).text(display.clone());
            if let NodeKind::Heading { level: level @ 1.., .. } = root.kind {
                el = el.attr("aria-level", &level.to_string());
            }
            let mut bv = BlockView { block: first + i, node: 0, src_start: s, src_end: e, display, map };
            if let Some((color, ts, te)) = callout {
                let (d0, d1) = (bv.to_display(ts), bv.to_display(te));
                for sp in spans.iter_mut().filter(|sp| sp.start >= d0 && sp.end <= d1) {
                    sp.color = Some(color);
                }
                el = el.style(&format!(
                    "border-left-color: rgb({0},{1},{2}); background-color: rgba({0},{1},{2},0.1)",
                    color.r, color.g, color.b
                ));
            } else if matches!(root.kind, NodeKind::Callout { .. }) {
                let kind = match root.kind {
                    NodeKind::Callout { kind, .. } => {
                        buf.slice(b.start + kind.start as usize, b.start + kind.end as usize)
                    }
                    _ => "".into(),
                };
                let c = callout_color(&kind);
                el = el.style(&format!("border-left-color: rgb({},{},{})", c.r, c.g, c.b));
            }
            code_spans(&b.tree, b.start, &src, s, &bv.map, pal, &mut spans);
            bv.node = dom.add(view, el.spans(spans));
            // A folded callout whose title and body share one run is the whole box: there is no
            // title node to put the handle on, and the body is hidden anyway.
            if callout.is_some() && callout_folded {
                dom.nodes[bv.node].el.action = Some(format!("callout:toggle:{}", b.start));
            }
            if !math.is_empty() {
                info.inline.push(crate::rich::InlineItem { node: bv.node, slots: math.clone() });
            }
            if !print && matches!(root.kind, NodeKind::Heading { level, .. } if level > 0) {
                let folded = tab.folds.is_folded(b.start);
                dom.add(
                    bv.node,
                    El::div()
                        .class("cm-fold-indicator")
                        .class_if(folded, "is-collapsed")
                        .text(if folded { "▸" } else { "▾" })
                        .attr("aria-label", if folded { "Desplegar encabezado" } else { "Plegar encabezado" })
                        .action(format!("fold:{}", b.start)),
                );
            }
            info.blocks.push(bv);
        }
        if mode != ViewMode::Source {
            embeds(app, dom, view, &tab.path, &b.tree, &buf.slice(b.start, b.end()), rendered, info);
        }
        if rendered {
            if let Some(n) = dataview_error {
                dom.add(view, El::div().class("rich-notice").text(n));
            }
            if let Some(n) = crate::rich::notice(&b.tree, &src, root.span.start) {
                dom.add(view, El::div().class("rich-notice").text(n));
            }
        }
    }
}

/// Hide the `>` that starts each line of a quote or callout.
fn hide_quote_markers(src: &str, src_start: usize, decos: &mut Vec<Deco>) {
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let ws = line.len() - line.trim_start_matches(' ').len();
        if ws <= 3 && line[ws..].starts_with('>') {
            let n = ws + 1 + line[ws + 1..].starts_with(' ') as usize;
            decos.push(Deco { start: src_start + at, end: src_start + at + n, style: Deco_::Hide });
        }
        at += line.len();
    }
}

/// Default colour for a callout type.
fn callout_color(kind: &str) -> Color {
    let (r, g, b) = match kind.trim().to_ascii_lowercase().as_str() {
        "abstract" | "summary" | "tldr" | "tip" | "hint" | "important" => (83, 223, 221),
        "success" | "check" | "done" => (68, 207, 110),
        "question" | "help" | "faq" | "warning" | "caution" | "attention" => (233, 151, 63),
        "failure" | "fail" | "missing" | "danger" | "error" | "bug" => (251, 70, 76),
        "example" => (168, 130, 255),
        "quote" | "cite" => (158, 158, 158),
        _ => (2, 122, 255),
    };
    Color { r, g, b, a: 255 }
}

/// Width of the note column a table is laid out in: `--file-line-width` (700 px) minus the
/// table's own left and right padding (28 px each), which is what its percentages resolve
/// against.
const TABLE_W: f32 = 644.0;
/// A table column narrower than this cannot show a header like "Duración" on one line: the
/// header wraps in the middle of the word. Every column gets this much before width is shared
/// by weight.
const MIN_COL_W: f32 = 80.0;

/// Width in pixels of each column of a grid `usable` wide, given the preferred width of every
/// column (the longest cell). A column never falls below [`MIN_COL_W`] while there is room for
/// every column to have it; the rest of the width is shared by weight. A table with more columns
/// than that allows shares the width equally, since honouring the minimum would overflow.
fn column_widths(weights: &[f32], usable: f32) -> Vec<f32> {
    let total: f32 = weights.iter().sum();
    let cols = weights.len() as f32;
    if total <= 0.0 || cols == 0.0 {
        return Vec::new();
    }
    if cols * MIN_COL_W >= usable {
        return vec![usable / cols; weights.len()];
    }
    weights.iter().map(|w| MIN_COL_W + (usable - cols * MIN_COL_W) * w / total).collect()
}

/// A table drawn as a grid. Every cell is its own text block, so clicks map to the cell's source.
#[allow(clippy::too_many_arguments)]
fn table(
    dom: &mut Dom,
    view: NodeId,
    pal: &Palette,
    src: &str,
    src_start: usize,
    tree: &md_engine::Tree,
    block_start: usize,
    block: usize,
    decos: &[Deco],
    info: &mut ViewInfo,
) {
    struct Cell {
        start: usize,
        display: String,
        spans: Vec<Span>,
        map: Vec<(usize, usize, usize)>,
        end: usize,
        center: bool,
    }
    let mut rows: Vec<(bool, Vec<Cell>)> = Vec::new();
    for (rid, rn) in tree.children(0) {
        let NodeKind::TableRow { header } = rn.kind else { continue };
        let mut cells = Vec::new();
        for (_, cn) in tree.children(rid) {
            let NodeKind::TableCell { align } = cn.kind else { continue };
            let (a, b) = (block_start + cn.span.start as usize, block_start + cn.span.end as usize);
            let Some(raw) = src.get(a.saturating_sub(src_start)..b.saturating_sub(src_start)) else {
                continue;
            };
            let start = a + (raw.len() - raw.trim_start().len());
            let text = raw.trim();
            let cell_decos: Vec<Deco> =
                decos.iter().filter(|d| d.start < start + text.len() && d.end > start).copied().collect();
            let (display, spans, map) = block_display(text, start, &cell_decos, &[], pal, false);
            cells.push(Cell {
                start,
                end: start + text.len(),
                display,
                spans,
                map,
                center: align == md_engine::Align::Center,
            });
        }
        rows.push((header, cells));
    }
    let cols = rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    // Column widths follow the longest cell, so short columns stay narrow. The text is not
    // measured here; 7.6 px per character plus the cell padding is a deliberate over-estimate,
    // and the cap keeps one long cell from leaving the other columns with no room.
    let weights: Vec<f32> = (0..cols)
        .map(|c| {
            let chars =
                rows.iter().filter_map(|(_, r)| r.get(c)).map(|cell| cell.display.chars().count()).max();
            chars.unwrap_or(0).clamp(3, 40) as f32 * 7.6 + 36.0
        })
        .collect();
    let widths = column_widths(&weights, TABLE_W);
    let total: f32 = widths.iter().sum();
    let grid = dom.add(view, El::div().class("markdown-table"));
    let mut band = false;
    for (header, cells) in rows {
        // Zebra striping: the style engine has no `:nth-child`, so the band is named here.
        if !header {
            band = !band;
        }
        let row = dom.add(
            grid,
            El::div().class("table-row").class_if(header, "is-header").class_if(!header && band, "is-band"),
        );
        for (c, cell) in cells.into_iter().enumerate() {
            let node = dom.add(
                row,
                El::div()
                    .class("table-cell")
                    .class_if(cell.center, "is-center")
                    .style(&format!("width: {:.3}%", widths[c] / total * 100.0))
                    .text(cell.display.clone())
                    .spans(cell.spans),
            );
            info.blocks.push(BlockView {
                block,
                node,
                src_start: cell.start,
                src_end: cell.end,
                display: cell.display,
                map: cell.map,
            });
        }
    }
}

const EMBED_MAX_BLOCKS: usize = 30;

/// The blocks of an embedded note, formatted like live preview without a cursor. Tables and
/// embeds inside an embed are shown as their source.
fn embedded_blocks(dom: &mut Dom, parent: NodeId, text: &str, action: &str, pal: &Palette) {
    let doc = md_engine::parse(text);
    let n = doc.blocks.len().min(EMBED_MAX_BLOCKS);
    let decos = decorate(&doc, 0..n, &[], editor_core::deco::Mode::LivePreview);
    for b in &doc.blocks[..n] {
        let root = b.tree.root();
        let (s, e) = (b.start + root.span.start as usize, b.start + root.span.end as usize);
        let class = match root.kind {
            NodeKind::Blank | NodeKind::CommentBlock | NodeKind::FrontMatter { .. } => continue,
            NodeKind::CodeBlock { .. } | NodeKind::MathBlock { .. } => "embed-line cm-codeblock",
            NodeKind::BlockQuote | NodeKind::Callout { .. } => "embed-line cm-quote",
            _ => "embed-line",
        };
        let Some(src) = text.get(s..e) else { continue };
        let mut block_decos: Vec<Deco> = decos.iter().filter(|d| d.start < e && d.end > s).copied().collect();
        if class.ends_with("cm-quote") {
            hide_quote_markers(src, s, &mut block_decos);
        }
        let verbatim = matches!(root.kind, NodeKind::HtmlBlock);
        let (display, mut spans, map) = block_display(src, s, &block_decos, &[], pal, verbatim);
        code_spans(&b.tree, b.start, src, s, &map, pal, &mut spans);
        dom.add(parent, El::div().class(class).text(display).spans(spans).action(action));
    }
    if doc.blocks.len() > n {
        dom.add(parent, El::div().class("embed-line").text("…").action(action));
    }
}
const IMAGE_MAX_WIDTH: f32 = 640.0;
/// The width of the note column (`--file-line-width` in `base.css`), the room a poster may take
/// before the editor has drawn a frame to measure.
const LINE_WIDTH: f32 = 700.0;

/// Images, note embeds and `.excalidraw` boards referenced by a block, shown below it. A board is
/// drawn only where the note is rendered: while the caret is on the line its `![[…]]` stays
/// editable, the same rule the dataview and Mermaid blocks follow.
#[allow(clippy::too_many_arguments)]
fn embeds(
    app: &App,
    dom: &mut Dom,
    view: NodeId,
    source: &str,
    tree: &md_engine::Tree,
    block_text: &str,
    rendered: bool,
    info: &mut ViewInfo,
) {
    for (_, n) in tree.iter() {
        let (target, subpath, size) = match n.kind {
            NodeKind::WikiLink { target, subpath, alias, embed: true } => {
                (target.of(block_text).to_string(), subpath.of(block_text), alias.of(block_text))
            }
            NodeKind::Image { dest, .. } => {
                let d = dest.of(block_text);
                if d.contains("://") {
                    continue;
                }
                (path::percent_decode(d), "", "")
            }
            _ => continue,
        };
        let Some(id) = app.store.index.resolve(&target, source) else { continue };
        let Some(f) = app.store.index.entry(id) else { continue };
        let rel: &str = &f.path;
        if f.kind == vault_core::FileKind::Markdown {
            let Some(text) = app.media.borrow_mut().note(&app.store, rel) else { continue };
            use vault_core::cache::parts;
            let meta = f.meta_parts(parts::HEADINGS | parts::BLOCKS).unwrap_or_default();
            let section = crate::media::embed_section(&text, &meta, subpath);
            let action = format!("open:{rel}");
            let embed = dom.add(view, El::div().class("markdown-embed").action(action.clone()));
            dom.add(
                embed,
                El::div().class("markdown-embed-title").text(path::stem(rel)).action(action.clone()),
            );
            embedded_blocks(dom, embed, section, &action, &palette(app));
        } else if ["png", "jpg", "jpeg", "gif", "webp", "svg"]
            .iter()
            .any(|e| path::extension(rel).eq_ignore_ascii_case(e))
        {
            let Some(img) = app.media.borrow_mut().image(&app.store, rel) else { continue };
            let (iw, ih) = img.size();
            let (iw, ih) = (iw.max(1) as f32, ih.max(1) as f32);
            // `![[image.png|100]]` and `|100x80` set the size.
            let mut dims = size.split('x').map(|v| v.trim().parse::<f32>().ok().filter(|v| *v > 0.0));
            let (w, h) = match (dims.next().flatten(), dims.next().flatten()) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, ih * w / iw),
                _ => (iw, ih),
            };
            let k = (IMAGE_MAX_WIDTH / w).min(1.0);
            dom.add(
                view,
                El::div()
                    .class("image-embed")
                    .attr("aria-label", path::file_name(rel))
                    .style(&format!("width: {:.0}px; height: {:.0}px", w * k, h * k))
                    .image(img),
            );
        } else if path::extension(rel).eq_ignore_ascii_case("excalidraw") {
            // While the caret is on the line the `![[…]]` stays, so it can still be edited.
            if rendered {
                board_embed(app, dom, view, rel, info);
            }
        } else if crate::video::is_video(rel) && app.video.is_open() && app.video.path() == rel {
            // While the player is full screen it lives in the overlay, not in the note.
            if !app.video_fullscreen {
                crate::video::widget(app, dom, view, crate::video::EMBED_SPACE);
            }
        } else {
            // Video, audio and anything else this build does not decode: a card that names the
            // file. A video also shows its own first frame, with the play button on top of it: the
            // frame is painted first and its children over it, which is the whole z-order this
            // engine has.
            let stamp = app.store.index.get(rel).map(|f| (f.mtime, f.size));
            // A compact window keeps the card with its button and decodes nothing: the poster is
            // for a screen with room to spare, not for the one that already has to squeeze.
            let posters = crate::posters::wanted(app.compact);
            let poster = stamp.filter(|_| posters).and_then(|st| app.posters.image(rel, st));
            if poster.is_none() && posters {
                if let (Some(st), Some(f)) = (stamp, app.store.index.get(rel)) {
                    // The tree is built with a shared borrow: the request waits its turn.
                    let path = app.store.abs(&f.path);
                    app.poster_queue.borrow_mut().push((path, rel.to_string(), st));
                }
            }
            if let Some(img) = poster {
                // The engine gives an image the box it is put in: the poster keeps its own shape,
                // shrunk to the room the note column really has, so it is never a stretched frame
                // and never spills past the column. The last frame's column is the only measure
                // there is while the tree is being built, and it is exact once the first one is
                // drawn, which is always before a poster lands.
                let (iw, ih) = (img.0.width() as f32, img.0.height() as f32);
                let line = app
                    .config
                    .appearance
                    .get("lineWidth")
                    .and_then(serde_json::Value::as_f64)
                    .map_or(LINE_WIDTH, |n| n as f32);
                let column = if app.editor_w > 1.0 { app.editor_w.min(line) } else { line };
                // As wide as the card under it: one block, one left edge. The engine stretches an
                // image to its box, so the box keeps the frame's own shape.
                let k = (column.max(160.0) / iw).min(1.0);
                let play = El::div().class("media-card-play").action(format!("video:open:{source}|{rel}"));
                let frame = dom.add(
                    view,
                    El::div()
                        .class("media-card-poster")
                        .style(&format!("width: {:.0}px; height: {:.0}px", iw * k, ih * k))
                        .image(img),
                );
                let button = dom.add(frame, play);
                dom.add(button, icons::el("play").class("media-card-play-icon"));
                let card = dom.add(view, El::div().class("media-card"));
                let body = dom.add(card, El::div().class("media-card-body"));
                dom.add(body, El::div().class("media-card-title").text(path::file_name(rel)));
                dom.add(
                    body,
                    El::div()
                        .class("media-card-action")
                        .text("Reproducir en MetaMind")
                        .action(format!("video:open:{source}|{rel}")),
                );
                continue;
            }
            let card = dom.add(view, El::div().class("media-card"));
            let icon = dom.add(card, El::div().class("media-card-icon"));
            dom.add(icon, icons::el("play"));
            let body = dom.add(card, El::div().class("media-card-body"));
            dom.add(body, El::div().class("media-card-title").text(path::file_name(rel)));
            dom.add(
                body,
                El::div()
                    .class("media-card-action")
                    .text(if crate::video::is_video(rel) {
                        "Reproducir en MetaMind"
                    } else {
                        "Abrir con el reproductor del sistema"
                    })
                    .action(if crate::video::is_video(rel) {
                        format!("video:open:{source}|{rel}")
                    } else {
                        format!("open-external:{rel}")
                    }),
            );
        }
    }
    // A YouTube link in the block is a video of its own: the card plays it in the application when
    // `yt-dlp` can resolve the stream, and offers the browser when it cannot. Code blocks keep
    // their text, so an example is never turned into a player.
    youtube_embed(app, dom, view, tree, block_text);
}

/// The YouTube card of a block, if the block links to one.
fn youtube_embed(app: &App, dom: &mut Dom, view: NodeId, tree: &md_engine::Tree, block_text: &str) {
    if matches!(tree.root().kind, NodeKind::CodeBlock { .. }) {
        return;
    }
    if let Some(link) = crate::youtube::find(block_text) {
        youtube_card(app, dom, view, &link);
    }
}

/// The card a YouTube link shows under its line: a player while that video is the open one, an
/// offer to play it otherwise. Without `yt-dlp` the card says why it cannot and hands the link to
/// the browser instead of failing silently.
fn youtube_card(app: &App, dom: &mut Dom, view: NodeId, link: &crate::youtube::Link) {
    if app.video.is_open() && app.video.path() == link.url {
        // While the player is over the window it lives there, not in the note.
        if !app.video_fullscreen {
            crate::video::widget(app, dom, view, crate::video::EMBED_SPACE);
        }
        return;
    }
    let card = dom.add(view, El::div().class("media-card media-card-youtube"));
    let icon = dom.add(card, El::div().class("media-card-icon"));
    dom.add(icon, icons::el("play"));
    let body = dom.add(card, El::div().class("media-card-body"));
    dom.add(body, El::div().class("media-card-title").text(format!("YouTube · {}", link.id)));
    if app.youtube.available() {
        // While `yt-dlp` runs, the card says so instead of looking dead.
        let waiting = app.youtube.resolving() == Some(link.url.as_str());
        dom.add(
            body,
            El::div()
                .class("media-card-action")
                .class_if(waiting, "is-busy")
                .text(if waiting { "Preparando el video…" } else { "Reproducir en MetaMind" })
                .action(format!("youtube:open:{}", link.url)),
        );
        dom.add(
            body,
            El::div()
                .class("media-card-note media-card-secondary")
                .text("Abrir en el navegador")
                .action(format!("open-url:{}", link.url)),
        );
    } else {
        dom.add(
            body,
            El::div()
                .class("media-card-action")
                .text("Abrir en el navegador")
                .action(format!("open-url:{}", link.url)),
        );
        dom.add(
            body,
            El::div()
                .class("media-card-note")
                .text("No está instalado yt-dlp: el video solo se puede ver en el navegador."),
        );
    }
}

/// A `.excalidraw` board drawn where its embed is, or the reason it could not be: the raw
/// `![[…]]` line stays above it, so a board this build cannot read is never lost.
fn board_embed(app: &App, dom: &mut Dom, view: NodeId, rel: &str, info: &mut ViewInfo) {
    let board = app.media.borrow_mut().board(&app.store, rel);
    // The embed is the way into the board: a press anywhere on it opens the file in its own tab,
    // the same `open:` a link uses, with the pointer cursor and the status-bar hint that go with
    // an action. The board is read-only here — drawing happens in the tab.
    let holder = dom.add(
        view,
        El::div()
            .class("exc-embed")
            .attr("aria-label", "Abrir el tablero en el editor")
            .action(format!("exc:open:{rel}")),
    );
    let name = path::file_name(rel);
    let notice = |dom: &mut Dom, text: String| {
        dom.add(holder, El::div().class("rich-notice").text(text));
    };
    let Some(board) = board else {
        notice(dom, format!("Excalidraw: {name} no está en la bóveda"));
        return;
    };
    match &*board {
        crate::excalidraw::Loaded::Failed(e) => {
            notice(dom, format!("Excalidraw: no se pudo leer {name} ({e}); se muestra el texto"))
        }
        crate::excalidraw::Loaded::Board(b) if b.list.ops.is_empty() => {
            notice(dom, format!("Excalidraw: {name} es un tablero vacío"))
        }
        crate::excalidraw::Loaded::Board(b) => {
            crate::excalidraw::add(dom, holder, b.clone(), &mut info.exc);
            if let Some(note) = crate::excalidraw::skipped_note(&b.list) {
                notice(dom, note);
            }
        }
    }
}

/// Set the hover flag on every element with the hovered action, and on its ancestors.
pub fn mark_hover(dom: &mut Dom, action: Option<&str>) {
    let Some(action) = action else { return };
    let hits: Vec<NodeId> =
        (0..dom.nodes.len()).filter(|&i| dom.nodes[i].el.action.as_deref() == Some(action)).collect();
    for id in hits {
        let mut cur = Some(id);
        while let Some(c) = cur {
            dom.nodes[c].hover = true;
            cur = dom.nodes[c].parent;
        }
    }
}

/// A small square button that shows one icon.
fn icon_button(dom: &mut Dom, parent: NodeId, icon: &str, cmd: &str, active: bool) -> NodeId {
    let b = dom.add(parent, El::div().class("clickable-icon").class_if(active, "is-active").action(cmd));
    dom.add(b, icons::el(icon));
    b
}

fn section_header(dom: &mut Dom, parent: NodeId, icon: &str, title: &str) {
    let header = dom.add(parent, El::div().class("nav-header"));
    dom.add(header, icons::el(icon));
    dom.add(header, El::div().class("nav-header-title").text(title));
}

/// What a field shows while it has the keyboard: the value and the caret after it. A value longer
/// than the field keeps its end — which is where the typing is — and starts with an ellipsis, so
/// the caret never falls off the right edge of a box the reader is typing into.
pub(crate) fn with_caret(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return format!("{text}\u{258f}");
    }
    let tail: String = text.chars().skip(n - max.saturating_sub(1)).collect();
    format!("\u{2026}{tail}\u{258f}")
}

/// A pane with nothing in it, said out loud: a faint icon and one line. Silence reads as a pane
/// that failed to draw; a sentence reads as a state the reader can act on.
fn empty_pane(dom: &mut Dom, parent: NodeId, icon: &str, text: impl Into<String>) {
    let row = dom.add(parent, El::div().class("pane-empty"));
    dom.add(row, icons::el(icon));
    dom.add(row, El::div().class("pane-empty-text").text(text.into()));
}

/// Shown in the main pane when no note is open: what to do next, with the hotkeys.
fn empty_state(app: &App, dom: &mut Dom, parent: NodeId) {
    let wrap = dom.add(parent, El::div().class("empty-state"));
    let card = dom.add(wrap, El::div().class("empty-state-container"));
    dom.add(card, El::div().class("empty-state-logo").image(logo()));
    dom.add(card, El::div().class("empty-state-title").text("No hay ningún archivo abierto"));
    let notes = app.store.index.notes().count();
    let sub = if notes == 0 {
        "Esta bóveda está vacía. Crea tu primera nota para empezar.".to_string()
    } else {
        format!("{notes} notas en esta bóveda. Elige una a la izquierda o:")
    };
    dom.add(card, El::div().class("empty-state-subtitle").text(sub));
    let list = dom.add(card, El::div().class("empty-state-action-list"));
    for (icon, label, cmd) in [
        ("note-plus", "Crear nota", "file-explorer:new-file"),
        ("switcher", "Ir a un archivo", "switcher:open"),
        ("command", "Abrir paleta de comandos", "command-palette:open"),
        ("calendar", "Abrir la nota diaria de hoy", "daily-notes"),
        ("graph", "Abrir vista de gráfico", "graph:open"),
        ("settings", "Abrir ajustes", "app:open-settings"),
        ("vault", "Abrir otra bóveda", "app:open-vault"),
    ] {
        let row = dom.add(list, El::div().class("empty-state-action").action(cmd));
        dom.add(row, icons::el(icon));
        dom.add(row, El::div().class("empty-state-action-label").text(label));
        dom.add(row, El::div().class("empty-state-action-hotkey").text(app.hotkey_for(cmd)));
    }
}

/// The application icon as an image, rasterised once.
pub fn logo() -> metamind_ui::Image {
    static LOGO: std::sync::OnceLock<metamind_ui::Image> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        const SIZE: u32 = 192;
        let mut pm = metamind_ui::Pixmap::new(SIZE, SIZE).expect("icon size is not zero");
        let rgba = crate::icon::rgba(SIZE);
        for (dst, src) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            // The pixmap holds premultiplied color.
            let a = u32::from(src[3]);
            dst.copy_from_slice(&[
                (u32::from(src[0]) * a / 255) as u8,
                (u32::from(src[1]) * a / 255) as u8,
                (u32::from(src[2]) * a / 255) as u8,
                src[3],
            ]);
        }
        metamind_ui::Image(std::sync::Arc::new(pm))
    })
    .clone()
}

fn graph(
    g: &crate::graphview::GraphView,
    dom: &mut Dom,
    parent: NodeId,
    info: &mut ViewInfo,
    viewport_h: f32,
) {
    let header = dom.add(parent, El::div().class("view-header graph-controls"));
    dom.add(
        header,
        El::div().class("graph-count").text(format!(
            "{} nodos  ·  {} enlaces",
            g.graph.nodes.len(),
            g.graph.edges.len()
        )),
    );
    dom.add(
        header,
        El::div()
            .class("graph-control")
            .class_if(g.auto_fit, "is-enabled")
            .text("Ajustar")
            .action("graph:reset-view"),
    );
    let pane = dom.add(parent, El::div().class("view-content graph-view-container"));
    let colors = dom.add(pane, El::div().class("graph-view-colors"));
    info.graph_colors = crate::graphview::COLOR_CLASSES
        .iter()
        .map(|c| dom.add(colors, El::div().class("graph-view").class(c)))
        .collect();
    info.graph = Some(pane);
    graph_panel(g, dom, pane, info, viewport_h);
}

/// The floating settings panel in the top right corner of the graph.
fn graph_panel(
    g: &crate::graphview::GraphView,
    dom: &mut Dom,
    pane: NodeId,
    info: &mut ViewInfo,
    viewport_h: f32,
) {
    use crate::graphview::{DEPTH, DISPLAY_SLIDERS, FORCE_SLIDERS, SECTIONS};
    if !g.panel.open {
        let b = dom
            .add(pane, El::div().class("graph-controls-button clickable-icon").action("graph:toggle-panel"));
        dom.add(b, icons::el("settings"));
        info.graph_panel = Some(b);
        return;
    }
    let panel = dom.add(
        pane,
        El::div()
            .class("graph-controls-panel")
            .id("graph-panel")
            .style(&format!("max-height: {:.0}px", (viewport_h - 110.0).max(120.0)))
            .action("graph:panel"),
    );
    info.graph_panel = Some(panel);
    let edit = g.panel.edit.as_ref();
    let field = |dom: &mut Dom, parent: NodeId, key: &str, value: &str, placeholder: &str| {
        let editing = edit.filter(|e| e.0 == key);
        let (text, faint) = match editing {
            Some((_, t)) => (with_caret(t, SIDE_FIELD_CHARS), false),
            None if value.is_empty() => (placeholder.to_string(), true),
            None => (value.to_string(), false),
        };
        let f = dom.add(
            parent,
            El::div()
                .class("graph-input")
                .class_if(editing.is_some(), "is-editing")
                .action(format!("graph:edit:{key}")),
        );
        dom.add(f, icons::el("search"));
        dom.add(f, El::div().class("graph-input-text").class_if(faint, "is-placeholder").text(text));
        if editing.is_some_and(|e| e.1.is_empty()) {
            search_options(dom, parent, "graph:insert:");
        }
    };
    let toggle = |dom: &mut Dom, parent: NodeId, label: &str, on: bool, action: &str| {
        let row = dom.add(parent, El::div().class("graph-setting"));
        dom.add(row, El::div().class("graph-setting-name").text(label));
        let c = dom.add(row, El::div().class("checkbox-container").class_if(on, "is-enabled").action(action));
        dom.add(c, El::div().class("checkbox-thumb"));
    };
    let slider = |dom: &mut Dom, parent: NodeId, sl: &crate::graphview::Slider| {
        let v = g.slider_value(sl.key);
        dom.add(parent, El::div().class("graph-setting-name graph-slider-label").text(sl.label));
        let row = dom.add(parent, El::div().class("graph-slider"));
        let shown = if sl.step >= 1.0 { format!("{v:.0}") } else { format!("{v:.2}") };
        dom.add(row, El::div().class("graph-slider-value").text(shown));
        let track =
            dom.add(row, El::div().class("graph-slider-track").action(format!("graph:slider:{}", sl.key)));
        // Fill and rest share the space left by the thumb in proportion to the value.
        let t = ((v - sl.min) / (sl.max - sl.min)).clamp(0.0, 1.0);
        dom.add(track, El::div().class("graph-slider-fill").style(&format!("flex-grow: {t:.4}")));
        dom.add(track, El::div().class("graph-slider-thumb"));
        dom.add(track, El::div().class("graph-slider-rest").style(&format!("flex-grow: {:.4}", 1.0 - t)));
    };
    for (i, (id, title)) in SECTIONS.iter().enumerate() {
        let collapsed = g.panel.collapsed[i];
        let sec =
            dom.add(panel, El::div().class("graph-control-section").class_if(collapsed, "is-collapsed"));
        let head = dom.add(sec, El::div().class("graph-control-section-header"));
        let t = dom
            .add(head, El::div().class("graph-control-section-title").action(format!("graph:section:{id}")));
        dom.add(t, icons::el(if collapsed { "chevron-right" } else { "chevron-down" }));
        dom.add(t, El::div().text(*title));
        if i == 0 {
            let b = dom.add(
                head,
                El::div()
                    .class("clickable-icon")
                    .attr("aria-label", "Restaurar ajustes predeterminados")
                    .action("graph:reset-settings"),
            );
            dom.add(b, icons::el("rotate-ccw"));
            let b = dom.add(
                head,
                El::div().class("clickable-icon").attr("aria-label", "Cerrar").action("graph:toggle-panel"),
            );
            dom.add(b, icons::el("x"));
        }
        if collapsed {
            continue;
        }
        let body = dom.add(sec, El::div().class("graph-control-section-body"));
        let s = &g.settings;
        match *id {
            "filter" => {
                field(dom, body, "search", &s.search, "Buscar archivos…");
                let f = &s.filters;
                toggle(dom, body, "Etiquetas", f.show_tags, "graph:toggle-tags");
                toggle(dom, body, "Adjuntos", f.show_attachments, "graph:toggle-attachments");
                toggle(dom, body, "Solo archivos existentes", f.hide_unresolved, "graph:toggle-unresolved");
                toggle(dom, body, "Huérfanos", f.show_orphans, "graph:toggle-orphans");
                if g.local.is_some() {
                    slider(dom, body, &DEPTH);
                }
            }
            "color-groups" => {
                for (gi, cg) in s.color_groups.iter().enumerate() {
                    let row = dom.add(body, El::div().class("graph-color-group"));
                    field(dom, row, &format!("group:{gi}"), &cg.query, "Escribe una consulta…");
                    dom.add(
                        row,
                        El::div()
                            .class("graph-color-swatch")
                            .style(&format!("background-color: #{:06x}", cg.rgb))
                            .attr("aria-label", "Cambiar color")
                            .action(format!("graph:group-color:{gi}")),
                    );
                    let x = dom.add(
                        row,
                        El::div()
                            .class("clickable-icon")
                            .attr("aria-label", "Quitar grupo")
                            .action(format!("graph:group-remove:{gi}")),
                    );
                    dom.add(x, icons::el("x"));
                }
                dom.add(
                    body,
                    El::div().class("graph-button mod-wide").text("Grupo nuevo").action("graph:group-new"),
                );
            }
            "display" => {
                toggle(dom, body, "Flechas", s.show_arrows, "graph:toggle-arrows");
                for sl in &DISPLAY_SLIDERS {
                    slider(dom, body, sl);
                }
                let row = dom.add(body, El::div().class("graph-setting mod-end"));
                dom.add(row, El::div().class("graph-button").text("Animar").action("graph:animate"));
            }
            _ => {
                for sl in &FORCE_SLIDERS {
                    slider(dom, body, sl);
                }
            }
        }
    }
}

/// After layout: draw the graph into its pane.
pub fn add_graph(app: &mut App, dom: &mut Dom, info: &ViewInfo) {
    let (Some(pane), Some(g)) = (info.graph, &mut app.graph_view) else { return };
    let mut colors = [Color::rgb(128, 128, 128); 9];
    for (slot, &n) in colors.iter_mut().zip(&info.graph_colors) {
        let s = &dom.nodes[n].style;
        *slot = s.text.color.with_alpha(s.opacity);
    }
    let mut text = dom.nodes[pane].style.text.clone();
    text.size = 11.0;
    text.line_height = 14.0;
    dom.nodes[pane].draw = g.paint(dom.nodes[pane].rect, &colors, &text);
}

/// Which tabs of a stacked pane `width` wide get a column: `(first, last)`, ending at the active
/// tab when the earlier ones fill the rest. The others fold to spines.
pub fn stack_range(n: usize, active: usize, width: f32) -> (usize, usize) {
    const COLUMN: f32 = 480.0;
    const SPINE: f32 = 32.0;
    if n == 0 {
        return (0, 0);
    }
    let k = ((width - SPINE * n as f32) / (COLUMN - SPINE)).floor().max(1.0) as usize;
    let k = k.min(n);
    let first = (active.min(n - 1) + 1).saturating_sub(k);
    (first, first + k - 1)
}

/// Stacked tabs: side by side, the ones that do not fit folded to spines with their title.
#[allow(clippy::too_many_arguments)]
fn stack(
    app: &App,
    dom: &mut Dom,
    leaf: NodeId,
    pane: usize,
    focused: bool,
    pal: &Palette,
    info: &mut ViewInfo,
    width: f32,
) {
    let (open, active) = app.pane_tabs(pane);
    if open.is_empty() {
        return editor(app, dom, leaf, pal, info, None, focused);
    }
    let (first, last) = stack_range(open.len(), active, width);
    let row = dom.add(leaf, El::div().class("workspace-stack"));
    for (i, t) in open.iter().enumerate() {
        if i < first || i > last {
            let spine = dom.add(row, El::div().class("workspace-stack-spine").action(format!("tab:{i}")));
            dom.add(spine, icons::el(icons::for_file(&t.path)));
            // Spines are too narrow for a line of text, so the title runs down a letter at a time.
            for c in t.title().chars().take(40) {
                dom.add(spine, El::div().class("workspace-stack-spine-letter").text(c.to_string()));
            }
            info.stack.push((pane, i, spine, None));
            continue;
        }
        let col = dom.add(row, El::div().class("workspace-stack-item").class_if(i == active, "is-active"));
        let head = dom.add(col, El::div().class("workspace-stack-header").action(format!("tab:{i}")));
        dom.add(head, icons::el(icons::for_file(&t.path)));
        dom.add(head, El::div().class("workspace-stack-title").text(t.title().to_string()));
        if focused && i == active {
            editor(app, dom, col, pal, info, Some((pane, i, t)), true);
            info.stack.push((pane, i, col, None));
        } else {
            let mut own = ViewInfo::default();
            editor(app, dom, col, pal, &mut own, Some((pane, i, t)), false);
            info.rich.append(&mut own.rich);
            info.exc.append(&mut own.exc);
            info.scrolled.append(&mut own.scrolled);
            info.stack.push((pane, i, col, own.editor));
        }
    }
}

/// The 6px strip on the inner edge of a sidebar that drags it wider or narrower. It is drawn
/// over the panel, so the rows underneath keep their full width, and it only shows on hover.
fn resize_handle(app: &App, dom: &mut Dom, side: NodeId, left: bool) {
    let dragging = app.side_drag.is_some_and(|d| d.left == left);
    let el = El::div()
        .class("sidebar-resize-handle")
        .class(if left { "mod-left" } else { "mod-right" })
        .class_if(dragging, "is-dragging")
        .action(if left { "side-resize:left" } else { "side-resize:right" });
    dom.add(side, el);
}

/// The panels the left sidebar holds: the row of tabs that picks one and the pane itself.
fn left_sidebar(app: &App, dom: &mut Dom, left: NodeId, viewport_h: f32) {
    let head = dom.add(left, El::div().class("sidebar-tab-header"));
    for (icon, cmd, panel) in [
        ("folder", "file-explorer:open", Panel::Files),
        ("search", "global-search:open", Panel::Search),
        ("bookmark", "bookmarks:open", Panel::Bookmarks),
        ("tag", "tag-pane:open", Panel::Tags),
        ("calendar", "calendar:open", Panel::Calendar),
    ] {
        if app.disabled_by(cmd).is_some() {
            continue;
        }
        icon_button(dom, head, icon, cmd, app.panel == panel);
    }
    let cmd = match app.panel {
        Panel::Files => "file-explorer:open",
        Panel::Search => "global-search:open",
        Panel::Bookmarks => "bookmarks:open",
        Panel::Tags => "tag-pane:open",
        Panel::Calendar => "calendar:open",
    };
    if let Some(name) = app.disabled_by(cmd) {
        dom.add(left, El::div().class("pane-empty").text(format!("{name} está desactivado.")));
        return;
    }
    match app.panel {
        Panel::Files => sidebar_files(app, dom, left, viewport_h),
        Panel::Search => sidebar_search(app, dom, left),
        Panel::Bookmarks => crate::panes::bookmarks(app, dom, left),
        Panel::Tags => crate::panes::tags(app, dom, left),
        Panel::Calendar => crate::calendar::panel(app, dom, left, viewport_h),
    }
}

/// The widths the two sidebars really get: the width each one was dragged to, while the note
/// between them keeps `NOTE_W_MIN`. A window too narrow for the three gives the sidebars what is
/// left in proportion, instead of squeezing the note to nothing and pushing its header out of the
/// window with it.
fn side_widths(avail: f32, want: (f32, f32)) -> (f32, f32) {
    let (left, right) = want;
    let room = (avail - crate::shell::NOTE_W_MIN).max(0.0);
    let total = left + right;
    if total <= room || total <= 0.0 {
        return want;
    }
    (left * room / total, right * room / total)
}

/// Width a sidebar takes when it floats over the note: the compact width, or less when the window
/// cannot give it that much and still shows some of the note behind it.
fn float_w(avail: f32, want: f32) -> f32 {
    // A sidebar over the note is a sheet: it takes the space instead of sharing it, which is what
    // a phone-width window has to give. `want` only says whether it is open at all.
    if want > 0.01 {
        avail
    } else {
        0.0
    }
}

/// The main window: ribbon, sidebars, the split panes and the status bar. `viewport_h` is the
/// height above the footer: the status bar is not part of the area a note scrolls in.
fn chrome(app: &App, dom: &mut Dom, ws: NodeId, pal: &Palette, info: &mut ViewInfo, viewport_h: f32) {
    // A phone-sized window gives those 44 px to the note: the bottom navigation already carries
    // the places the ribbon offers, and the ⋮ menu of the bar carries the rest.
    let show_ribbon = app.config.appearance.bool("showRibbon", true) && !app.compact;
    let ribbon = dom.add(ws, El::div().class("workspace-ribbon").class_if(!show_ribbon, "is-hidden"));
    let ribbon_action = |dom: &mut Dom, icon: &str, cmd: &str, active: bool| {
        let b = dom.add(
            ribbon,
            El::div().class("side-dock-ribbon-action").class_if(active, "is-active").action(cmd),
        );
        dom.add(b, icons::el(icon));
    };
    for (icon, cmd, active) in [
        ("sidebar-left", "app:toggle-left-sidebar", app.left_open),
        ("switcher", "switcher:open", false),
        ("graph", "graph:open", app.show_graph),
        ("canvas", "canvas:new", false),
        ("table", "kanban:new", app.kanban_active()),
        ("calendar", "calendar:open", app.left_open && app.panel == Panel::Calendar),
        // The dashboard was only in the command palette: a board nobody can find is a board that
        // does not exist.
        ("layout-dashboard", "dashboard:open", crate::dashboard::is_dashboard(app)),
        ("template", "templates:insert-template", false),
        ("command", "command-palette:open", false),
    ] {
        if app.disabled_by(cmd).is_some() {
            continue;
        }
        ribbon_action(dom, icon, cmd, active);
    }
    for r in &app.plugins.ext.reg.ribbon {
        dom.add(
            ribbon,
            El::div().class("side-dock-ribbon-action").text(r.icon.clone()).action(r.command.clone()),
        );
    }
    dom.add(ribbon, El::div().class("side-dock-ribbon-spacer"));
    for (icon, cmd, active) in [
        ("vault", "app:open-vault", false),
        (if app.dark { "sun" } else { "moon" }, "theme:toggle-light-dark", false),
        ("settings", "app:open-settings", app.settings.is_some()),
    ] {
        ribbon_action(dom, icon, cmd, active);
    }
    let ribbon_w = if show_ribbon { 44.0 } else { 0.0 };
    let avail = (app.window_w - ribbon_w).max(0.0);
    // A compact window has no room for three columns: the sidebars float over the note instead
    // of taking one, and the note keeps the whole width.
    let compact = app.window_w < crate::shell::COMPACT_W;
    let left_w = app.anims.toward("left", app.left_open);
    let right_w = app.anims.toward("right", app.right_open);
    // The width the user dragged stays the target: opening and closing scale it.
    let left_want = app.left_w.clamp(crate::shell::SIDE_W_MIN, crate::shell::SIDE_W_MAX) * left_w.min(1.0);
    let right_want = app.right_w.clamp(crate::shell::SIDE_W_MIN, crate::shell::SIDE_W_MAX) * right_w.min(1.0);
    let (left_col, right_col) =
        if compact { (0.0, 0.0) } else { side_widths(avail, (left_want, right_want)) };
    if left_w > 0.01 && !compact {
        let el = El::div().class("workspace-split mod-left-split").style(&format!("width: {left_col:.1}px"));
        let left = dom.add(ws, el);
        left_sidebar(app, dom, left, viewport_h);
        resize_handle(app, dom, left, true);
    }
    let root = dom.add(ws, El::div().class("workspace-split mod-root"));
    split_layout(
        app,
        dom,
        root,
        &app.layout,
        &mut Vec::new(),
        pal,
        info,
        viewport_h,
        avail - left_col - right_col,
    );
    if compact {
        // Added after the note and over it: a sidebar that floats is painted above the content
        // it covers, and it is what the pointer finds first.
        if left_w > 0.01 {
            let el = El::div().class("workspace-split mod-left-split is-floating").style(&format!(
                "top: 0; bottom: 0; left: {ribbon_w:.1}px; width: {:.1}px",
                float_w(avail, left_want)
            ));
            let left = dom.add(ws, el);
            left_sidebar(app, dom, left, viewport_h);
        }
        if right_w > 0.01 && app.right_panel != RightPanel::Ai {
            let el = El::div()
                .class("workspace-split mod-right-split is-floating")
                .style(&format!("top: 0; bottom: 0; right: 0; width: {:.1}px", float_w(avail, right_want)));
            let right = dom.add(ws, el);
            right_sidebar(app, dom, right, info);
        }
    } else if right_w > 0.01 {
        let el =
            El::div().class("workspace-split mod-right-split").style(&format!("width: {right_col:.1}px"));
        let right = dom.add(ws, el);
        right_sidebar(app, dom, right, info);
        resize_handle(app, dom, right, false);
    }
    // The copilot takes no column and no 260 px band either: in a compact window it covers the
    // workspace. It goes last, so it is painted over the note and over a floating sidebar; the
    // status bar is a child of the window, out of the sheet's reach.
    if compact && app.sheet_drawn() {
        ai_sheet(app, dom, ws, info);
    }
    // The status bar keeps its line under the bottom navigation: it is the window's footer, and
    // the rows below the note are measured with it in the count.
    let status = dom.add(0, El::div().class("status-bar"));
    // What the element under the pointer does, with its hotkey. Rows whose label had to be
    // shortened (the explorer, the tabs) are named in full here: the status bar is this app's
    // tooltip, and a floating one would cover the row it describes.
    let hint = app.hover_action.as_deref().and_then(|a| {
        let name = crate::shell::COMMANDS
            .iter()
            .find(|c| c.0 == a)
            .map(|c| c.1.to_string())
            .or_else(|| {
                a.strip_prefix("close-tab:")
                    .map(|_| "Cerrar pestaña (el clic central también cierra)".to_string())
            })
            .or_else(|| a.strip_prefix("toggle-pin:").map(|_| "Desanclar pestaña".to_string()))
            .or_else(|| {
                // The number beside Plugins in the settings window: how many built-in plugins
                // are on, and what clicking it does.
                a.strip_prefix("settings:badge:")
                    .map(|n| format!("{n} plugins integrados activos · abre la lista de plugins"))
            })
            .or_else(|| {
                // A `.excalidraw` embed in a note: a press opens the board in its own tab, which is
                // the only place it can be drawn on.
                a.strip_prefix("exc:open:").map(|p| format!("Abrir tablero: {p}"))
            })
            .or_else(|| (a == "mermaid:zoom").then(|| "Ampliar el diagrama a pantalla completa".to_string()))
            .or_else(|| (a == "mermaid:zoom-close").then(|| "Cerrar el diagrama (Esc)".to_string()))
            .or_else(|| a.strip_prefix("toggle-folder:").map(|p| format!("Carpeta: {p}")))
            .or_else(|| a.strip_prefix("open:").map(|p| format!("Abrir {p}")))
            .or_else(|| {
                let i = a.strip_prefix("tab:")?.parse::<usize>().ok()?;
                app.tabs.get(i).map(|t| format!("Pestaña: {}", t.path))
            })
            .or_else(|| {
                let side = a.strip_prefix("side-resize:")?;
                let px = if side == "left" { app.left_w } else { app.right_w };
                Some(format!("Arrastra para cambiar el ancho ({px:.0} px)"))
            })?;
        let key = app.hotkey_for(a);
        Some(if key.is_empty() { name } else { format!("{name}  ({key})") })
    });
    dom.add(status, El::div().class("status-bar-item status-bar-hint").text(hint.unwrap_or_default()));
    if let Some(b) = &app.busy {
        let item = dom.add(status, El::div().class("status-bar-item status-bar-busy"));
        dom.add(item, El::div().class("status-bar-spinner").text(b.glyph()));
        dom.add(item, El::div().class("status-bar-busy-label").text(&b.label));
    }
    if let Some(t) = app.tab() {
        let words = t.meta.as_ref().map_or(0, |(_, m)| m.words);
        let backlinks = app
            .store
            .index
            .id(&t.path)
            .and_then(|id| app.cached_graph().map(|g| g.back.get(id as usize).map_or(0, Vec::len)))
            .unwrap_or(0);
        if let Some(label) = t.vim_label() {
            dom.add(status, El::div().class("status-bar-item").text(label));
        }
        dom.add(status, El::div().class("status-bar-item").text(format!("{backlinks} retroenlaces")));
        let goal =
            t.meta.as_ref().and_then(|(_, m)| m.frontmatter.as_ref()?.iter().find(|(k, _)| k == "word-goal"));
        let goal = goal.and_then(|(_, v)| match v {
            md_engine::frontmatter::Value::Number(n) if *n >= 1.0 => Some(*n as u32),
            md_engine::frontmatter::Value::String(s) => s.trim().parse().ok().filter(|n| *n > 0),
            _ => None,
        });
        let count = match goal {
            Some(g) => format!("{words} / {g} palabras"),
            None => format!("{words} palabras"),
        };
        dom.add(status, El::div().class("status-bar-item").text(count));
        // 200 words a minute, a common estimate for silent reading.
        dom.add(
            status,
            El::div().class("status-bar-item").text(format!("{} min de lectura", words.div_ceil(200))),
        );
        dom.add(
            status,
            El::div()
                .class("status-bar-item")
                .text(format!("{} caracteres", t.editor.buf.rope().len_chars())),
        );
    }
    for s in &app.plugins.ext.reg.status {
        dom.add(status, El::div().class("status-bar-item").text(s.text.clone()));
    }
    let notes = app.store.index.notes().count();
    dom.add(
        status,
        El::div().class("status-bar-item").text(format!("{notes} nota{}", if notes == 1 { "" } else { "s" })),
    );
}

/// The player over the window, in the box the workspace has: the same card as in the note, fitted
/// to the whole workspace. The scrim takes every pixel between the tab bar and the status bar, so
/// the player reads as full screen instead of as a modal on top of the note, and the card is
/// centred in it, as wide as its picture and never wider than the scrim. A click on the frame
/// plays or pauses it; a click anywhere outside the card, or Escape, goes back to the player in
/// its note.
///
/// The dim layer is a child of the workspace and not of the window, so the status bar keeps its
/// own height instead of being covered (or borrowed) by the overlay, and the picture is fitted to
/// the space the layout really gave the workspace: a size guessed here is what used to cut the
/// controls off the bottom and is what cropped the picture to the shape of the box.
fn video_fullscreen(app: &App, dom: &mut Dom, parent: NodeId) {
    if !app.video_fullscreen || !app.video.is_open() {
        return;
    }
    let backdrop = dom.add(
        parent,
        El::div().class("modal-container mod-dim video-fullscreen").action("video:fullscreen-close"),
    );
    // The box the card is centred in is the workspace itself. It is set in pixels from the last
    // layout because a percentage here resolves against nothing in this engine.
    let area = if app.video_area.0 > 0.0 && app.video_area.1 > 0.0 {
        app.video_area
    } else {
        (app.window_w.max(320.0), app.window_h.max(240.0))
    };
    let modal = dom.add(
        backdrop,
        El::div().class("modal mod-video").style(&format!("width: {:.0}px; height: {:.0}px", area.0, area.1)),
    );
    crate::video::widget(app, dom, modal, crate::video::overlay_space(area));
}

/// A Mermaid diagram opened full screen by a click on its block: the window behind it dims and
/// the diagram gets the width of the window, so the labels of a wide diagram can be read. Inside
/// the box a click grabs and drags it, the wheel zooms it, and a double click shows its source;
/// the backdrop and Escape close it.
fn diagram_zoom(app: &App, dom: &mut Dom, viewport_h: f32, rich: &mut Vec<crate::rich::Item>) {
    let Some(src) = &app.diagram_zoom else { return };
    // The diagram takes the whole window: it is what the reader asked to see, and the note behind
    // it is of no use while it is open.
    let w = app.window_w.max(160.0);
    let h = viewport_h.max(120.0);
    let backdrop = dom.add(0, El::div().class("modal-container mod-dim").action("mermaid:zoom-close"));
    let modal = dom.add(
        backdrop,
        El::div()
            .class("modal mod-diagram is-full")
            .action("mermaid:zoom-close")
            .style(&format!("width: {w:.0}px; height: {h:.0}px; max-width: none")),
    );
    // The hint lives in a band of its own at the bottom: with the diagram taking the whole box it
    // was laid out past the window's edge and read as a line cut in half.
    let (inner_w, inner_h) = (w - 24.0, (h - 44.0 - 12.0 - 26.0).max(120.0));
    if app.diagram_zoom_source {
        crate::rich::add_zoom_source(dom, modal, src, inner_w, inner_h);
    } else {
        crate::rich::add_zoom(
            dom,
            modal,
            src,
            rich,
            crate::rich::Zoom {
                w: inner_w,
                h: inner_h,
                pan: app.diagram_zoom_pan,
                zoom: app.diagram_zoom_scale,
            },
        );
    }
    let close = dom.add(modal, El::div().class("diagram-zoom-close").action("mermaid:zoom-close"));
    dom.nodes[close].el.attrs.push(("aria-label".into(), "Cerrar el diagrama".into()));
    dom.add(close, icons::el("x"));
    let hint = if app.diagram_zoom_source {
        "Doble clic para ver el diagrama · Esc o clic fuera para cerrar"
    } else {
        "Arrastra para mover · rueda para acercar · doble clic para ver el código · Esc para cerrar"
    };
    dom.add(modal, El::div().class("diagram-zoom-hint").text(hint));
}

/// Prompts, the hover preview, settings, menus and notices, over the window.
/// Width the settings panel is drawn at (`.modal.mod-settings` in `base.css`) plus the margin it
/// keeps off the edges of the window: a narrower window gives it the width the window has.
const SETTINGS_W: f32 = 900.0;
/// Width of the window below which a row of the settings panel puts its control under its label:
/// the category column, the 216 px control column and a label with room for a word do not fit in
/// a phone side by side.
const SETTINGS_ROW_W: f32 = 620.0;
/// Under this width the panel shows one screen at a time: a list of categories, or a page with a
/// way back. Two columns at 500 px leave neither of them readable.
pub const SETTINGS_SINGLE_W: f32 = 700.0;

/// The settings window is built by `settings::modal` at the fixed width of `base.css`; here it
/// learns the width it really has. Under `SETTINGS_W + 24` the panel stops being 900 px wide
/// (`is-compact`) and its category column narrows, and in a phone-sized window the rows fold
/// (`is-narrow`), so the panel and everything in it stays between the edges of the window.
fn fold_settings(app: &App, dom: &mut Dom) {
    if app.window_w >= SETTINGS_W + 24.0 {
        return;
    }
    let Some(n) = dom.nodes.iter_mut().find(|n| n.el.classes.iter().any(|c| c == "mod-settings")) else {
        return;
    };
    n.el.classes.push("is-compact".to_string());
    if app.window_w < SETTINGS_ROW_W {
        n.el.classes.push("is-narrow".to_string());
    }
    // Under a phone's width the panel is one screen at a time: the categories, or the page.
    if app.window_w < SETTINGS_SINGLE_W {
        n.el.classes.push("is-single".to_string());
    }
    // The style the panel already carries is where it lands on the window; the cap on its width
    // goes in front of it.
    let style = n.el.inline_style.take().unwrap_or_default();
    n.el.inline_style = Some(format!("max-width: {:.1}px; {style}", app.window_w - 24.0));
}

fn overlays(app: &App, dom: &mut Dom, workspace: Option<NodeId>, viewport_h: f32, info: &mut ViewInfo) {
    // The player over the window is part of the workspace, not of the window: the status bar keeps
    // the height it has.
    if let Some(ws) = workspace {
        video_fullscreen(app, dom, ws);
    }
    diagram_zoom(app, dom, viewport_h, &mut info.rich);
    if app.exc_fullscreen {
        if let Some(v) = &app.exc_view {
            crate::excalidrawview::fullscreen(v, dom, app.window_w, viewport_h, info);
        }
    }
    // The event editor and the new-board dialog are modals of the window, over everything the
    // panes built.
    crate::calendar::editor(app, dom);
    crate::kanbanview::new_board_dialog(app, dom);
    if let Some(p) = &app.prompt {
        let t = app.anims.t("prompt");
        let mut el = El::div().class("prompt");
        // 560 px of box is more than a narrow window has: there it keeps a margin off both
        // edges and takes the room between them.
        let narrow = app.window_w < crate::shell::COMPACT_W;
        let style = |top: f32, opacity: f32| {
            let mut s =
                if narrow { "left: 12px; right: 12px; width: auto; ".to_string() } else { String::new() };
            s.push_str(&format!("top: {top:.1}px; opacity: {opacity:.2}"));
            s
        };
        if t < 1.0 {
            // Down and in: the box arrives from four pixels above, fading while it lands.
            el = el.style(&style(80.0 - 4.0 * (1.0 - t), 0.55 + 0.45 * t));
        } else if narrow {
            el = el.style(&style(80.0, 1.0));
        }
        let modal = dom.add(0, el);
        let placeholder = match p.kind {
            PromptKind::Commands => "Escribe un comando…",
            PromptKind::Switcher => "Busca o crea una nota…",
            PromptKind::Rename => "Nombre nuevo…",
            PromptKind::NewFolder => "Nombre de la carpeta…",
            PromptKind::Template => "Busca una plantilla…",
            PromptKind::Workspace => match p.target.as_str() {
                "save" => "Nombre del espacio de trabajo…",
                _ => "Busca un espacio de trabajo…",
            },
            PromptKind::BaseCell => "Vacío para quitar la propiedad…",
        };
        let shown = if p.query.is_empty() {
            placeholder.to_string()
        } else {
            with_caret(&p.query, prompt_chars(app.window_w))
        };
        let input = dom.add(modal, El::div().class("prompt-input-container"));
        dom.add(
            input,
            icons::el(match p.kind {
                PromptKind::Commands => "command",
                PromptKind::Switcher => "search",
                PromptKind::Rename => "edit",
                PromptKind::NewFolder => "folder-plus",
                PromptKind::Template => "note",
                PromptKind::Workspace => "list",
                PromptKind::BaseCell => "edit",
            }),
        );
        dom.add(input, El::div().class("prompt-input").text(shown));
        let results = dom.add(modal, El::div().class("prompt-results").id("prompt-results"));
        for (i, item) in p.items.iter().enumerate() {
            let row = dom.add(
                results,
                El::div()
                    .class("suggestion-item")
                    .class_if(i == p.selected, "is-selected")
                    .action(item.action.clone()),
            );
            dom.add(row, El::div().class("suggestion-title").text(item.label.clone()));
            dom.add(row, El::div().class("suggestion-aux").text(item.aux.clone()));
        }
    }
    hover_popover(app, dom, viewport_h);
    crate::settings::modal(app, dom);
    fold_settings(app, dom);
    if let Some(m) = &app.menu {
        // Open toward the inside of the window when there is no room at the pointer.
        let x =
            if m.x + crate::menu::WIDTH > app.window_w { (m.x - crate::menu::WIDTH).max(0.0) } else { m.x };
        let y = if m.y + m.height() > viewport_h { (viewport_h - m.height()).max(0.0) } else { m.y };
        let t = app.anims.t("menu");
        // Fen and up into place: four pixels of travel is enough to read as an entrance and not
        // as a box that was always there. It ends exactly where the pointer asked for it.
        let el = El::div().class("menu").style(&format!(
            "left: {x:.1}px; top: {:.1}px; width: {:.0}px; opacity: {:.2}",
            y - 4.0 * (1.0 - t),
            crate::menu::WIDTH,
            0.5 + 0.5 * t
        ));
        let menu = dom.add(0, el);
        for (i, item) in m.items.iter().enumerate() {
            if item.action.is_empty() {
                dom.add(menu, El::div().class("menu-separator"));
                continue;
            }
            let row = dom.add(
                menu,
                El::div()
                    .class("menu-item")
                    .class_if(m.selected == Some(i), "is-selected")
                    .class_if(item.warning, "is-warning")
                    .class_if(item.disabled.is_some(), "is-disabled")
                    .action(format!("menu:{i}")),
            );
            dom.add(row, icons::el(item.icon));
            dom.add(row, El::div().class("menu-item-title").text(item.label.clone()));
            // A row that cannot run says why in the place the hotkey would take.
            if item.disabled.is_some() {
                dom.add(row, El::div().class("menu-item-hotkey").text(item.disabled.unwrap_or("")));
            } else if !item.hotkey.is_empty() {
                dom.add(row, El::div().class("menu-item-hotkey").text(item.hotkey.clone()));
            }
        }
    }
    if let Some(n) = &app.notice {
        let t = app.anims.t("notice");
        // Some notices inform instead of warning: a plugin the native core cannot run yet is
        // expected, and the toast should not dress it as a failure.
        let info = app.plugins.no_port_notice.as_ref().is_some_and(|m| m == n);
        let mut el = El::div().class("notice").class_if(info, "mod-info").attr("role", "status");
        if t < 1.0 {
            // The toast is anchored to the bottom of the window, so it rises: it starts ten
            // pixels lower and fading, and lands on its own line above the status bar.
            el = el.style(&format!(
                "bottom: {:.1}px; opacity: {:.2}",
                46.0 - 10.0 * (1.0 - t),
                0.35 + 0.65 * t
            ));
        }
        let toast = dom.add(0, el);
        dom.add(toast, icons::el(if info { "sparkles" } else { "warning" }));
        dom.add(toast, El::div().class("notice-text").text(n.clone()));
        dom.add(
            toast,
            El::div().class("notice-close").text("×").attr("aria-label", "Cerrar").action("notice:dismiss"),
        );
    }
    if app.perf {
        dom.add(
            0,
            El::div().class("perf-overlay").text(format!(
                "fotograma {:.2} ms · {} nodos",
                app.last_frame_us as f32 / 1000.0,
                dom.nodes.len()
            )),
        );
    }
}

/// Height of the status bar, from `base.css` (`padding: 4px 14px` around a line of
/// `--font-ui-smaller`). It is drawn on top of the window, so it is not part of the area a note
/// scrolls in: the view of a note, and everything sized from the window height, has to end above
/// it. The last line of a note used to disappear under it.
pub const STATUS_BAR_H: f32 = 27.0;

/// Height of the bottom navigation, drawn only in a compact window, right above the status bar.
pub const NAV_H: f32 = 52.0;

/// The list of an open dropdown, over the rows of the panel it belongs to.
///
/// It runs after the first layout, when the control it belongs to has a rectangle: the list is
/// anchored to it, right-aligned, flipped up when there is no room below, and added as the last
/// child of the panel so it paints over the rows it covers (this engine has no z-index). It is
/// placed inside the panel's own box, which clips: the list never runs out over the veil, the
/// status bar or the note behind them, and it scrolls instead of growing past the edge. Nothing
/// grows: the row keeps its height whether the list is open or not.
///
/// How tall the list wants to be is not guessed from a row size in pixels: it is built once with
/// all the room the panel has and measured. A list of long provider names is taller than one of
/// short theme names, and guessing short cut the last option out of sight.
pub fn settings_menu(app: &App, dom: &mut Dom, fonts: &mut Fonts, w: f32, h: f32) {
    let Some(ui) = app.settings.as_ref() else { return };
    let Some(menu) = ui.menu.as_ref() else { return };
    let Some(anchor) = dom.by_id("sm-anchor") else { return };
    let Some(panel) = dom.nodes.iter().position(|n| n.el.classes.iter().any(|c| c == "mod-settings")) else {
        return;
    };
    let p = dom.nodes[panel].rect;
    let st = dom.nodes[panel].style.clone();
    // The box an absolutely positioned child of the panel is placed in: its padding box.
    let pad = Rect::new(
        p.x + st.border[3],
        p.y + st.border[0],
        (p.w - st.border[1] - st.border[3]).max(0.0),
        (p.h - st.border[0] - st.border[2]).max(0.0),
    );
    let a = dom.nodes[anchor].rect;
    // The rows live in the scrolling page of the panel, not under its title bar: the list stays
    // inside that page, so it never covers the search field or the way to close the window.
    let band = dom.by_id("settings-content").map_or(pad, |n| dom.nodes[n].rect);
    let (inside, gap) = (10.0, 4.0);
    let w_list = a.w.max(240.0).min((pad.w - 2.0 * inside).max(120.0));
    let x = (a.x + a.w - w_list).clamp(pad.x + inside, (pad.x + pad.w - inside - w_list).max(pad.x + inside));
    let room = (band.h - 2.0 * inside).max(48.0);
    let list = dom.add(
        panel,
        El::div().class("setting-menu is-overlay").attr("role", "listbox").style(&format!(
            "position: absolute; left: {:.1}px; top: {:.1}px; width: {w_list:.1}px; max-height: {room:.1}px",
            x - pad.x,
            a.y + a.h + gap - pad.y
        )),
    );
    for (i, (value, text)) in menu.options.iter().enumerate() {
        dom.add(
            list,
            El::div()
                .class("setting-menu-item")
                .class_if(i == menu.at, "is-active")
                .class_if(*value == menu.current, "is-checked")
                .attr("role", "option")
                .text(text.clone())
                .action(format!("settings:pick:{}:{i}", menu.key)),
        );
    }
    // Laid out with the whole panel as its room, the list reports the height it wants, not the
    // height the side it opens on would give it.
    dom.style(&app.theme, app.env());
    dom.layout(fonts, w, h);
    let want_h = dom.nodes[list].rect.h;
    // The room on each side of the control: the list takes the side where it fits whole, and the
    // roomier one when it fits on neither.
    let below = (band.y + band.h - inside - (a.y + a.h) - gap).max(0.0);
    let above = (a.y - band.y - inside - gap).max(0.0);
    let (pick, down) = if want_h <= below {
        (want_h, true)
    } else if want_h <= above {
        (want_h, false)
    } else if below >= above {
        (below, true)
    } else {
        (above, false)
    };
    // A list that fits on neither side still has to be usable: it opens at a couple of rows and
    // scrolls to the rest, never taller than the panel.
    let floor = 80.0_f32.min(want_h);
    let h_list = pick.max(floor).min(room);
    let y = if down { a.y + a.h + gap } else { a.y - gap - h_list };
    let y = y.clamp(band.y + inside, (band.y + band.h - inside - h_list).max(band.y + inside));
    dom.nodes[list].el.inline_style = Some(format!(
        "position: absolute; left: {:.1}px; top: {:.1}px; width: {w_list:.1}px; max-height: {h_list:.1}px",
        x - pad.x,
        y - pad.y
    ));
    // The keyboard can walk past the part of the list that fits in the room it got: the list
    // scrolls so the row the keyboard is on stays in sight. The rows were measured by the pass
    // above, so their pitch and where they start are known without laying it out again.
    let kids = dom.nodes[list].children.clone();
    if h_list + 0.5 < want_h && kids.len() > 1 {
        let top = dom.nodes[kids[0]].rect.y - dom.nodes[list].rect.y;
        let pitch = dom.nodes[kids[1]].rect.y - dom.nodes[kids[0]].rect.y;
        let bottom = top + (menu.at + 1) as f32 * pitch;
        dom.nodes[list].scroll_y = (bottom - h_list).max(0.0);
    }
}

/// The window's view. `solo` is a pane shown alone in a window of its own; the menus, prompts
/// and notices show in the window whose pane has the keyboard.
/// One row of the ⋮ menu: icon, label and the command it runs.
type NavRow = (&'static str, &'static str, &'static str);

/// The ⋮ menu, over the window: what the ribbon and the palettes offered, in one list that reads
/// in a phone-sized window.
fn more_sheet(app: &App, dom: &mut Dom) {
    // It ends where the note ends: above the bottom navigation and the status bar under it.
    // A wide window has no bottom navigation, so the list goes down to the status bar.
    let nav_h = if app.compact { NAV_H } else { 0.0 };
    let body_h = (app.window_h - STATUS_BAR_H - nav_h).max(0.0);
    let sheet = dom.add(
        0,
        El::div().class("more-sheet").style(&format!("top: 0; left: 0; right: 0; height: {body_h:.0}px")),
    );
    let head = dom.add(sheet, El::div().class("more-sheet-header"));
    dom.add(head, icons::el("ellipsis").class("more-sheet-mark"));
    dom.add(head, El::div().class("more-sheet-title").text("Menú"));
    let close = dom.add(head, El::div().class("ai-sheet-close").action("more:close"));
    dom.nodes[close].el.attrs.push(("aria-label".into(), "Cerrar el menú".into()));
    dom.add(close, icons::el("x"));
    let list = dom.add(sheet, El::div().class("more-list"));
    let groups: [(&str, &[NavRow]); 3] = [
        (
            "Paneles",
            &[
                ("separator-vertical", "Dividir a la derecha", "workspace:split-vertical"),
                ("separator-horizontal", "Dividir abajo", "workspace:split-horizontal"),
                ("x", "Cerrar panel", "workspace:close-pane"),
            ],
        ),
        (
            "Vistas",
            &[
                ("layout-dashboard", "Tablero", "dashboard:open"),
                ("graph", "Gráfico", "graph:open"),
                ("canvas", "Lienzo", "canvas:new"),
                ("calendar", "Calendario", "calendar:open"),
                ("search", "Buscar", "global-search:open"),
            ],
        ),
        (
            "Bóveda",
            &[
                ("folder", "Archivos", "file-explorer:open"),
                ("sparkles", "Copiloto", "ai:open"),
                ("command", "Paleta de comandos", "command-palette:open"),
                ("sun", "Tema claro u oscuro", "theme:toggle-light-dark"),
                ("vault", "Abrir otra bóveda", "app:open-vault"),
                ("settings", "Ajustes", "app:open-settings"),
            ],
        ),
    ];
    for (title, rows) in groups {
        dom.add(list, El::div().class("more-group").text(title));
        for (icon, label, cmd) in rows {
            if app.disabled_by(cmd).is_some() {
                continue;
            }
            // A command with nothing to act on stays in the list, muted: the reader sees the
            // way in and learns why it does nothing today instead of wondering where it went.
            let blocked = app.blocked(cmd).is_some();
            let row = dom.add(
                list,
                El::div()
                    .class("more-row")
                    .class_if(blocked, "is-disabled")
                    .action(format!("more:run:{cmd}")),
            );
            dom.add(row, icons::el(icon).class("more-row-icon"));
            dom.add(row, El::div().class("more-row-label").text(*label));
        }
    }
}

/// The bottom navigation of a compact window: the five places a phone reader moves between,
/// with the accent on the one in force and an indicator that slides from the old one to the new.
/// It is a row of the window, between the note and the status bar: neither of them is painted
/// over, and nothing of it is painted over them.
fn bottom_nav(app: &App, dom: &mut Dom) {
    let bar = dom.add(0, El::div().class("bottom-nav").style(&format!("height: {NAV_H}px")));
    // One destination is in force at a time, and it is the one the reader can see: a sheet, a
    // modal or the menu covers the note, so what it hides is not marked under it.
    let covered = app.ai_sheet() || app.settings.is_some() || app.more_open;
    let left = |panel: Panel| !covered && app.left_open && app.panel == panel;
    // One destination in force, and it is the one on top: the settings panel is painted over the
    // sheet, and the ⋮ menu over both. A destination the reader cannot see is not marked.
    let settings = app.settings.is_some() && !app.more_open;
    let agent = app.ai_sheet() && app.settings.is_none() && !app.more_open;
    let items: [(&str, &str, &str, bool); 5] = [
        ("folder", "Archivos", "file-explorer:open", left(Panel::Files)),
        ("search", "Buscar", "global-search:open", left(Panel::Search)),
        ("calendar", "Calendario", "calendar:open", left(Panel::Calendar)),
        ("sparkles", "Agente", "ai:open", agent),
        ("settings", "Ajustes", "app:open-settings", settings),
    ];
    // Five destinations and the way into the rest of the app: the ⋮ slot at the right end.
    // The engine lays every child out at its own width and only the leftover is shared by
    // `flex-grow`, so an equal share is asked for with the width itself: at 360 px that is 60 px
    // each, and the name of the longest destination is shortened with an ellipsis, not wrapped.
    let width = (app.window_w / (items.len() + 1) as f32).max(1.0);
    // With no destination in force (the note alone) there is nothing to point at.
    if items.iter().any(|i| i.3) {
        // The indicator travels from where it was to where it goes, on the window's animation.
        let t = app.anims.t("bottom-nav");
        let pos = app.nav_from + (app.nav_to - app.nav_from) * t;
        dom.add(
            bar,
            El::div().class("bottom-nav-indicator").style(&format!(
                "left: {:.1}px; width: {:.1}px",
                pos * width + width * 0.25,
                width * 0.5
            )),
        );
    }
    for (icon, label, action, active) in items {
        let item = dom.add(
            bar,
            El::div()
                .class("bottom-nav-item")
                .class_if(active, "is-active")
                .style(&format!("width: {width:.2}px"))
                .action(action),
        );
        dom.add(item, icons::el(icon).class("bottom-nav-icon"));
        dom.add(item, El::div().class("bottom-nav-label").text(label));
    }
    // What the ribbon used to hold, at the corner the thumb reaches.
    let more = dom.add(
        bar,
        El::div()
            .class("bottom-nav-item")
            .class_if(app.more_open, "is-active")
            .style(&format!("width: {width:.2}px"))
            .action(if app.more_open { "more:close" } else { "more:open" }),
    );
    dom.add(more, icons::el("ellipsis").class("bottom-nav-icon"));
    dom.add(more, El::div().class("bottom-nav-label").text("Más"));
}

pub fn build(app: &App, viewport_h: f32, solo: Option<usize>) -> (Dom, ViewInfo) {
    let pal = palette(app);
    let mut info = ViewInfo { accent: pal.accent, ..ViewInfo::default() };
    let mut dom = Dom::new();
    dom.nodes[0].el.classes.push(if app.dark { "theme-dark" } else { "theme-light" }.to_string());
    // A window of its own shows the pane alone, with no status bar under it.
    // In a compact window the window has three rows at the bottom: the note, the navigation and
    // the status bar. The note takes what is left, so the navigation is a row of the layout and
    // not a layer over the note: nothing is painted on top of anything else.
    // The bottom navigation belongs to the window, not to a pane: a pop-out shows a pane alone.
    let with_nav = app.compact && solo.is_none();
    let nav_h = if with_nav { NAV_H } else { 0.0 };
    let body_h = if solo.is_some() { viewport_h } else { (viewport_h - STATUS_BAR_H - nav_h).max(0.0) };
    // `flex-grow` in the workspace would let it take the whole window back, over the navigation:
    // in a compact window its height is the one measured here.
    let grow = if with_nav { "flex-grow: 0; " } else { "" };
    let ws = dom.add(0, El::div().class("workspace").style(&format!("{grow}height: {body_h:.1}px")));
    info.workspace = Some(ws);
    if with_nav {
        // Right after the workspace: the status bar, which `chrome` adds next, keeps its line
        // under it, and the sheets, which are children of the workspace, end above it.
        bottom_nav(app, &mut dom);
    }
    match solo {
        Some(p) => {
            let root = dom.add(ws, El::div().class("workspace-split mod-root"));
            pane(app, &mut dom, root, p, &pal, &mut info, body_h, app.window_w);
        }
        None => chrome(app, &mut dom, ws, &pal, &mut info, body_h),
    }
    let here = match solo {
        Some(p) => app.pane == p,
        None => !app.is_popped(app.pane),
    };
    if here {
        overlays(app, &mut dom, info.workspace, body_h, &mut info);
    }
    for (id, y) in &app.scrolls {
        if let Some(n) = dom.by_id(id) {
            dom.nodes[n].scroll_y = *y;
        }
    }
    if app.more_open {
        // Last, so the menu is painted over the note, over a floating sidebar and over the bar.
        // The command is the same in any window, so a wide one shows the list too; without the
        // bottom navigation it covers the note from the top down to the status bar.
        more_sheet(app, &mut dom);
    }
    mark_hover(&mut dom, app.hover_action.as_deref());
    (dom, info)
}

/// After layout: add selection highlights and carets to the editor blocks.
pub fn add_carets(app: &App, dom: &mut Dom, info: &ViewInfo, fonts: &mut Fonts) {
    add_decorations(dom, info, fonts);
    let Some(tab) = app.tab().filter(|t| t.mode != ViewMode::Reading) else { return };
    let pal = palette(app);
    let show_caret = app.focus == Focus::Editor;
    for bv in &info.blocks {
        let node = &dom.nodes[bv.node];
        let width = dom.content_rect(bv.node).w;
        let style = node.style.text.clone();
        let spans = node.el.spans.clone();
        let mut under = Vec::new();
        let mut over = Vec::new();
        for (a, b) in misspellings(app, dom, bv) {
            // The word being typed is left alone until the caret moves off it.
            if tab.editor.buf.sels.iter().any(|s| s.head >= a && s.head <= b) {
                continue;
            }
            for r in fonts.range_rects(
                &bv.display,
                &spans,
                &style,
                Some(width),
                bv.to_display(a),
                bv.to_display(b),
            ) {
                over.push((Rect::new(r.x, r.y + r.h - 2.5, r.w, 1.5), pal.error.with_alpha(0.8)));
            }
        }
        for sel in &tab.editor.buf.sels {
            if sel.end() < bv.src_start || sel.start() > bv.src_end {
                continue;
            }
            if !sel.is_empty() {
                let (a, b) =
                    (bv.to_display(sel.start().max(bv.src_start)), bv.to_display(sel.end().min(bv.src_end)));
                for r in fonts.range_rects(&bv.display, &spans, &style, Some(width), a, b) {
                    under.push((Rect::new(r.x, r.y, r.w, r.h), pal.selection));
                }
            }
            if show_caret && sel.head >= bv.src_start && sel.head <= bv.src_end {
                let d = bv.to_display(sel.head);
                if let Some(r) = fonts.range_rects(&bv.display, &spans, &style, Some(width), d, d).first() {
                    over.push((Rect::new(r.x, r.y + 2.0, 1.5, r.h - 4.0), pal.caret));
                }
            }
        }
        let n = &mut dom.nodes[bv.node];
        n.overlays.extend(under);
        n.overlays_top.extend(over);
    }
}

/// After layout: inline backgrounds (highlight, inline code), underlines and strikethrough, which
/// text layout does not draw. Every view mode and the PDF export need them.
pub fn add_decorations(dom: &mut Dom, info: &ViewInfo, fonts: &mut Fonts) {
    for bv in &info.blocks {
        let node = &dom.nodes[bv.node];
        let width = dom.content_rect(bv.node).w;
        let style = &node.style.text;
        let spans = &node.el.spans;
        let mut under = Vec::new();
        let mut over = Vec::new();
        for sp in spans.iter() {
            let Some(bg) = sp.background else { continue };
            for r in fonts.range_rects(&bv.display, spans, style, Some(width), sp.start, sp.end) {
                under.push((Rect::new(r.x - 1.0, r.y + 1.0, r.w + 2.0, r.h - 2.0), bg));
            }
        }
        for sp in spans.iter().filter(|s| s.underline || s.strike) {
            for r in fonts.range_rects(&bv.display, spans, style, Some(width), sp.start, sp.end) {
                let y = if sp.strike { r.y + r.h * 0.55 } else { r.y + r.h - 3.0 };
                over.push((Rect::new(r.x, y, r.w, 1.0), sp.color.unwrap_or(style.color).with_alpha(0.7)));
            }
        }
        let n = &mut dom.nodes[bv.node];
        n.overlays = under;
        n.overlays_top = over;
    }
}

/// Misspelled words in an editor block as source ranges. Code, front matter, links, tags and
/// markup are not checked: anything mono or colored is left out.
pub fn misspellings(app: &App, dom: &Dom, bv: &BlockView) -> Vec<(usize, usize)> {
    if !app.config.app.bool("spellcheck", false) {
        return Vec::new();
    }
    let el = &dom.nodes[bv.node].el;
    if el.classes.iter().any(|c| c == "cm-codeblock" || c == "cm-frontmatter") {
        return Vec::new();
    }
    let src = |disp: usize, end: bool| {
        bv.map
            .iter()
            .find(|&&(_, d, len)| if end { disp > d && disp <= d + len } else { disp >= d && disp < d + len })
            .map(|&(s, d, _)| s + disp - d)
    };
    crate::spell::misspelled(&bv.display)
        .into_iter()
        .filter(|&(a, b)| !el.spans.iter().any(|s| s.start < b && a < s.end && (s.mono || s.color.is_some())))
        .filter_map(|(a, b)| Some((src(a, false)?, src(b, true)?)))
        .collect()
}

/// Source offset for a click at window coordinates, if it landed on an editor block.
pub fn offset_at(dom: &Dom, info: &ViewInfo, fonts: &mut Fonts, x: f32, y: f32) -> Option<usize> {
    let hit = dom.hit(x, y)?;
    let bv = info.blocks.iter().find(|b| b.node == hit).or_else(|| {
        // A click below the last block or in the margin goes to the nearest block by y.
        (Some(hit) == info.editor)
            .then(|| {
                info.blocks.iter().min_by_key(|b| {
                    let r = dom.nodes[b.node].rect;
                    ((r.y + r.h / 2.0 - y).abs() * 10.0) as i64
                })
            })
            .flatten()
    })?;
    let node = &dom.nodes[bv.node];
    let cb = dom.content_rect(bv.node);
    let d = fonts.hit(&bv.display, &node.el.spans, &node.style.text, Some(cb.w), x - cb.x, y - cb.y);
    Some(bv.to_source(d))
}

/// Window rect of the primary caret, after layout.
pub fn caret_rect(app: &App, dom: &Dom, info: &ViewInfo, fonts: &mut Fonts) -> Option<Rect> {
    let head = app.tab()?.editor.buf.sels[0].head;
    let bv = info.blocks.iter().find(|b| head >= b.src_start && head <= b.src_end)?;
    let node = &dom.nodes[bv.node];
    let cb = dom.content_rect(bv.node);
    let d = bv.to_display(head);
    let r = *fonts.range_rects(&bv.display, &node.el.spans, &node.style.text, Some(cb.w), d, d).first()?;
    Some(Rect::new(cb.x + r.x, cb.y + r.y, r.w, r.h))
}

/// The input method's uncommitted text, drawn over the caret in the editor's font size.
pub fn add_preedit(app: &App, dom: &mut Dom, info: &ViewInfo, caret: Rect) -> Option<NodeId> {
    if app.preedit.is_empty() {
        return None;
    }
    let head = app.tab()?.editor.buf.sels[0].head;
    let bv = info.blocks.iter().find(|b| head >= b.src_start && head <= b.src_end)?;
    let size = dom.nodes[bv.node].style.text.size;
    let el = El::div().class("ime-preedit").text(app.preedit.clone()).style(&format!(
        "left: {:.1}px; top: {:.1}px; height: {:.1}px; line-height: {:.1}px; font-size: {size:.1}px",
        caret.x, caret.y, caret.h, caret.h
    ));
    Some(dom.add(0, el))
}

/// After layout: the underline that marks text as still composing, and the input method's cursor.
pub fn mark_preedit(app: &App, dom: &mut Dom, id: NodeId, fonts: &mut Fonts) {
    let text = app.preedit.as_str();
    let style = dom.nodes[id].style.text.clone();
    let rects = fonts.range_rects(text, &[], &style, None, 0, text.len());
    let pal = palette(app);
    let mut over: Vec<_> =
        rects.iter().map(|r| (Rect::new(r.x, r.y + r.h - 3.0, r.w, 1.5), pal.caret)).collect();
    let at = app.preedit_cursor.unwrap_or(text.len()).min(text.len());
    if let Some(r) = fonts.range_rects(text, &[], &style, None, at, at).first() {
        over.push((Rect::new(r.x, r.y + 2.0, 1.5, r.h - 4.0), pal.caret));
    }
    dom.nodes[id].overlays_top.extend(over);
}

const SUGGEST_W: f32 = 300.0;
const SUGGEST_ROW_H: f32 = 30.0;
const PREVIEW_W: f32 = 320.0;

/// The editor suggestion list under the caret (above it when there is no room below),
/// with a preview of the selected note beside it.
pub fn add_suggest(app: &App, dom: &mut Dom, caret: Rect, viewport_h: f32) {
    let Some(s) = &app.suggest else { return };
    let h = s.items.len() as f32 * SUGGEST_ROW_H + 14.0;
    let below = caret.y + caret.h + 4.0;
    let y = if below + h > viewport_h { (caret.y - h - 4.0).max(0.0) } else { below };
    let preview = !s.preview.is_empty();
    let w = SUGGEST_W + if preview { PREVIEW_W } else { 0.0 };
    let x = caret.x.min(app.window_w - w).max(0.0);
    // The list that follows the caret rises four pixels into place the first time it appears;
    // on the next keystroke the clock is already at its end and the box only moves with the
    // caret, which is what typing needs.
    let t = app.anims.t("suggest");
    let pop = dom.add(
        0,
        El::div().class("editor-suggest").style(&format!(
            "left: {x:.1}px; top: {:.1}px; opacity: {:.2}",
            y + 4.0 * (1.0 - t),
            0.5 + 0.5 * t
        )),
    );
    let list = dom.add(pop, El::div().class("editor-suggest-list").style(&format!("width: {SUGGEST_W}px")));
    for (i, item) in s.items.iter().enumerate() {
        let row = dom.add(
            list,
            El::div()
                .class("suggestion-item")
                .class_if(i == s.selected, "is-selected")
                .action(format!("suggest:{i}")),
        );
        dom.add(row, El::div().class("suggestion-title").text(item.label.clone()));
        dom.add(row, El::div().class("suggestion-aux").text(item.aux.clone()));
    }
    if preview {
        let pane = dom.add(
            pop,
            El::div()
                .class("editor-suggest-preview")
                .style(&format!("width: {PREVIEW_W}px; max-height: {h}px")),
        );
        for line in s.preview.lines() {
            dom.add(pane, El::div().class("editor-suggest-preview-line").text(line.to_string()));
        }
    }
}

const POPOVER_W: f32 = 380.0;
const POPOVER_H: f32 = 260.0;

/// The note preview opened by Ctrl+hover on a link, below and right of the pointer when it fits.
fn hover_popover(app: &App, dom: &mut Dom, viewport_h: f32) {
    let Some(h) = &app.hover_preview else { return };
    let (px, py) = h.at;
    // A window narrower than the preview gets the preview at its own width: floating 380 px in a
    // 360 px window put the card half outside, where it could not be read or dismissed.
    let w = POPOVER_W.min((app.window_w - 16.0).max(200.0));
    let x = if px + 12.0 + w > app.window_w { (px - 12.0 - w).max(8.0) } else { px + 12.0 };
    let y = if py + 16.0 + POPOVER_H > viewport_h { (py - 8.0 - POPOVER_H).max(0.0) } else { py + 16.0 };
    let t = app.anims.t("hover");
    let pop = dom.add(
        0,
        El::div().class("popover hover-popover").id("hover-popover").style(&format!(
            "left: {x:.1}px; top: {:.1}px; width: {w:.0}px; max-height: {POPOVER_H}px; opacity: {:.2}",
            y + 4.0 * (1.0 - t),
            0.4 + 0.6 * t
        )),
    );
    let action = format!("open:{}", h.path);
    dom.add(
        pop,
        El::div().class("hover-popover-title").text(path::stem(&h.path).to_string()).action(action.clone()),
    );
    if h.text.trim().is_empty() {
        dom.add(pop, El::div().class("hover-popover-empty").text("No hay vista previa de este archivo"));
    } else {
        embedded_blocks(dom, pop, &h.text, &action, &palette(app));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_wraps_join_into_one_paragraph() {
        // A paragraph written with hard wraps is one run of text on screen.
        let src = "one two\nthree four";
        let (out, table) = join_soft_breaks(src, false).expect("the break is soft");
        assert_eq!(out, "one two three four");
        // One entry per old byte plus the end, so spans and the map can follow.
        assert_eq!(table.len(), src.len() + 1);
        assert_eq!(*table.last().unwrap(), out.len());
        assert!(table.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn list_items_headings_and_hard_breaks_keep_their_break() {
        // The indent of a continuation line goes with the break it is part of.
        let (out, _) = join_soft_breaks("a\n- b\n  more", false).expect("one soft break");
        assert_eq!(out, "a\n- b more");
        // A line that starts a heading, a quote, a fence or a table keeps its own break.
        assert_eq!(
            join_soft_breaks("body\ntext\n# heading", false).map(|(s, _)| s),
            Some("body text\n# heading".into())
        );
        assert_eq!(join_soft_breaks("line  \nnext", false), None);
        // A callout's title is its own line even when the body starts on the next one.
        assert_eq!(join_soft_breaks("Title\nbody", true).map(|(s, _)| s), None);
        assert_eq!(join_soft_breaks("Title\nbody", false).map(|(s, _)| s), Some("Title body".into()));
    }

    #[test]
    fn a_table_column_is_never_below_the_minimum_width() {
        // 96 px is what an 8-character header such as "Duración" weighs, and sharing the column
        // by weight alone left it at 77 px, where the word broke in two.
        let w = column_widths(&[500.0, 96.0, 420.0, 60.0], TABLE_W);
        assert!(w.iter().all(|x| *x >= MIN_COL_W - 0.01), "{w:?}");
        assert!((w.iter().sum::<f32>() - TABLE_W).abs() < 0.1, "{w:?}");
        assert!(w[0] > w[3], "width is still shared by weight: {w:?}");
        // More columns than the minimum allows share the width equally instead of overflowing.
        let many = column_widths(&[1.0; 12], TABLE_W);
        assert!((many[0] - TABLE_W / 12.0).abs() < 0.1, "{many:?}");
    }

    #[test]
    fn joining_moves_spans_and_the_source_map() {
        // `two` is an emphasised span on the second source line.
        let src = "one two\nthree four";
        let span = Span { start: 8, end: 18, bold: true, ..Span::default() };
        let mut spans = vec![span];
        let mut map = vec![(0usize, 0usize, src.len())];
        let (out, table) = join_soft_breaks(src, false).expect("joined");
        remap_lines(&mut spans, &mut map, &table);
        assert_eq!(&out[spans[0].start..spans[0].end], "three four");
        assert!(spans[0].bold);
        assert_eq!(map[0].1, 0);
        assert_eq!(map[0].2, out.len());
    }
}
