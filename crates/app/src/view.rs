//! Builds the element tree for the current application state, and maps editor geometry
//! (carets, selections, clicks) between source offsets and what is displayed.

use crate::icons;
use crate::shell::{App, Focus, Panel, PromptKind, RightPanel, ViewMode};
use editor_core::deco::{decorate, Deco, Style as Deco_};
use editor_core::Sel;
use md_engine::NodeKind;
use rustsidian_ui::{Dom, El, Fonts, NodeId, Rect, Span};
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
    pub blocks: Vec<BlockView>,
    pub editor: Option<NodeId>,
    /// The graph pane and the hidden elements its colors are read from.
    pub graph: Option<NodeId>,
    pub graph_colors: Vec<NodeId>,
    /// The graph's settings panel (or its gear button); input over it does not reach the graph.
    pub graph_panel: Option<NodeId>,
    pub canvas: Option<NodeId>,
    pub canvas_colors: Vec<NodeId>,
    /// Each pane with its leaf element and, unless focused, its scrolling view.
    pub panes: Vec<(usize, NodeId, Option<NodeId>)>,
    /// Each tab of a stacked pane: (pane, tab, its column or spine, its scrolling view unless
    /// it is the focused editor).
    pub stack: Vec<(usize, usize, NodeId, Option<NodeId>)>,
    /// Math and diagram blocks painted after layout.
    pub rich: Vec<crate::rich::Item>,
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
/// the painter fills once the line's real metrics are known.
pub fn block_display(
    src: &str,
    src_start: usize,
    decos: &[Deco],
    math: &[crate::rich::Inline],
    pal: &Palette,
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
                let at = display.len();
                display.push_str(&m.filler);
                spans.push(Span {
                    start: at,
                    end: display.len(),
                    scale: m.scale,
                    lh_scale: m.lh,
                    ..Span::default()
                });
                map.push((m.start, at, m.end - m.start));
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
        display.push_str(&src[ra..rb]);
        if styled {
            spans.push(span);
        }
    }
    (display, spans, map)
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
    for i in 1..lines.len() {
        let line = lines[i].1;
        let prev = lines[i - 1].1;
        let t = line.trim_start_matches([' ', '\t']);
        let starts = t.is_empty() || list_marker(line).is_some() || t.starts_with(['#', '>', '|', '`', '~']);
        kept[i] = prev.ends_with(' ') || prev.ends_with('\\') || starts || (i == 1 && keep_first);
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
) {
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
}

/// Record the formulas whose blanks a text node holds, in the order they appear in it.
fn attach_math(
    display: &str,
    math: &[crate::rich::Inline],
    next: &mut usize,
    node: NodeId,
    info: &mut ViewInfo,
) {
    let n = crate::rich::slots_in(display).min(math.len() - *next);
    if n == 0 {
        return;
    }
    info.inline.push(crate::rich::InlineItem { node, slots: math[*next..*next + n].to_vec() });
    *next += n;
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
            .text(app.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("Vault").to_string()),
    );
    for (icon, cmd) in [
        ("note-plus", "file-explorer:new-file"),
        ("sort", "file-explorer:sort-menu"),
        ("collapse", "file-explorer:collapse-all"),
    ] {
        let b = icon_button(dom, header, icon, cmd, false);
        if cmd == "file-explorer:sort-menu" {
            dom.nodes[b].el.attrs.push(("aria-label".into(), "Change sort order".into()));
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
}

fn sidebar_search(app: &App, dom: &mut Dom, parent: NodeId) {
    section_header(dom, parent, "search", "Search");
    let caret = if app.focus == Focus::Search { "▏" } else { "" };
    let shown = if app.search_query.is_empty() && app.focus != Focus::Search {
        "Type a query, press Enter".to_string()
    } else {
        format!("{}{caret}", app.search_query)
    };
    dom.add(parent, El::div().class("search-input-container").text(shown).action("focus:search"));
    if app.focus == Focus::Search && app.search_query.is_empty() {
        search_options(dom, parent, "search:insert:");
    }
    let list = dom.add(parent, El::div().class("nav-files-container").id("search-results"));
    if let Some(e) = &app.search_error {
        dom.add(list, El::div().class("search-result-file-match").text(format!("Query error: {e}")));
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
    ("path:", "match path of the file"),
    ("file:", "match file name"),
    ("tag:", "search for tags"),
    ("line:", "search keywords on same line"),
    ("section:", "search keywords under same heading"),
    ("[", "match property"),
];

fn search_options(dom: &mut Dom, parent: NodeId, action: &str) {
    let list = dom.add(parent, El::div().class("search-options"));
    dom.add(list, El::div().class("search-options-title").text("Search options"));
    for (op, desc) in SEARCH_OPTIONS {
        let row = dom.add(list, El::div().class("suggestion-item").action(format!("{action}{op}")));
        let label = if *op == "[" { "[property]" } else { op };
        dom.add(row, El::div().class("suggestion-title").text(label));
        dom.add(row, El::div().class("suggestion-aux").text(*desc));
    }
}

fn right_sidebar(app: &App, dom: &mut Dom, parent: NodeId) {
    let head = dom.add(parent, El::div().class("sidebar-tab-header"));
    for (icon, cmd, panel) in [
        ("link", "backlinks:open", RightPanel::Backlinks),
        ("arrow-up-right", "outgoing-links:open", RightPanel::Outgoing),
        ("list", "outline:open", RightPanel::Outline),
    ] {
        if app.disabled_by(cmd).is_some() {
            continue;
        }
        icon_button(dom, head, icon, cmd, app.right_panel == panel);
    }
    let Some(tab) = app.tab() else {
        dom.add(parent, El::div().class("pane-empty").text("No file is open."));
        return;
    };
    let cmd = match app.right_panel {
        RightPanel::Backlinks => "backlinks:open",
        RightPanel::Outgoing => "outgoing-links:open",
        RightPanel::Outline => "outline:open",
    };
    if let Some(name) = app.disabled_by(cmd) {
        dom.add(parent, El::div().class("pane-empty").text(format!("{name} is turned off.")));
        return;
    }
    match app.right_panel {
        RightPanel::Outline => outline(tab, dom, parent),
        RightPanel::Outgoing => outgoing(app, tab, dom, parent),
        RightPanel::Backlinks => backlinks(app, dom, parent),
    }
}

fn outline(tab: &crate::shell::Tab, dom: &mut Dom, parent: NodeId) {
    section_header(dom, parent, "list", "Outline");
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
    section_header(dom, parent, "arrow-up-right", "Outgoing links");
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
        dom.add(list, El::div().class("pane-empty").text("No outgoing links."));
    }
}

fn backlinks(app: &App, dom: &mut Dom, parent: NodeId) {
    let bar = dom.add(parent, El::div().class("nav-buttons-container"));
    let b = icon_button(dom, bar, "collapse", "backlinks:toggle-collapse", app.backlinks_collapsed);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Collapse results".into()));
    let b = icon_button(dom, bar, "context", "backlinks:toggle-context", app.backlinks_more);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Show more context".into()));
    let b = icon_button(dom, bar, "sort", "backlinks:toggle-sort", app.backlinks_desc);
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Change sort order".into()));
    let b = icon_button(dom, bar, "search", "backlinks:toggle-filter", app.backlinks_filter.is_some());
    dom.nodes[b].el.attrs.push(("aria-label".into(), "Show search filter".into()));
    if let Some(f) = &app.backlinks_filter {
        let (text, faint) = match (f.is_empty(), app.backlinks_editing) {
            (_, true) => (format!("{f}▏"), false),
            (true, false) => ("Filter...".to_string(), true),
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
    dom.add(header, El::div().class("nav-header-title").text("Linked mentions"));
    dom.add(header, El::div().class("tree-item-flair").text(files.len().to_string()));
    let back = dom.add(parent, El::div().class("pane-section"));
    if files.is_empty() {
        dom.add(back, El::div().class("pane-empty").text("No backlinks found."));
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
    dom.add(header, El::div().class("nav-header-title").text("Unlinked mentions"));
    if !app.show_unlinked {
        return;
    }
    let list = dom.add(parent, El::div().class("pane-section"));
    let files = app.mentions.as_ref().map(|m| m.files.as_slice()).unwrap_or(&[]);
    if files.is_empty() {
        dom.add(list, El::div().class("pane-empty").text("No unlinked mentions."));
    }
    for (p, snippets) in files {
        let row = dom.add(list, El::div().class("tree-item-self bookmark").action(format!("open:{p}")));
        dom.add(row, El::div().class("tree-item-inner").text(path::strip_md(p).to_string()));
        dom.add(
            row,
            El::div()
                .class("tree-item-flair clickable-icon")
                .text("Link")
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
            let el = El::div().class("workspace-leaf-resize-handle").class(handle);
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
                El::div().class("workspace-tab-header").class_if(active, "is-active").action(focus),
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
    let covered = focused && (app.show_graph || app.canvas_active() || app.base_active());
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
    let new_tab =
        dom.add(tabs, El::div().class("workspace-tab-header-new-tab").action("file-explorer:new-file"));
    dom.add(new_tab, icons::el("plus"));
    if app.panes[pane].stacked && !covered {
        stack(app, dom, leaf, pane, focused, pal, info, width);
        info.panes.push((pane, leaf, None));
        return;
    }
    if !focused {
        let mut own = ViewInfo::default();
        editor(app, dom, leaf, pal, &mut own, open.get(active).map(|t| (pane, active, t)), false);
        info.rich.append(&mut own.rich);
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
        ViewMode::Source => "source",
        ViewMode::LivePreview => "live preview",
        ViewMode::Reading => "reading",
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
    let class = if tab.mode == ViewMode::Reading {
        "view-content markdown-reading-view"
    } else {
        "view-content markdown-source-view cm-editor"
    };
    let readable =
        if app.config.app.bool("readableLineLength", false) { " is-readable-line-width" } else { "" };
    let class = format!("{class}{readable}");
    let fade = if focused { app.anims.t("view") } else { 1.0 };
    let mut view_el = El::div().class(&class);
    if focused {
        view_el = view_el.id("editor");
    }
    if fade < 1.0 {
        view_el = view_el.style(&format!("opacity: {:.3}", 0.25 + 0.75 * fade));
    }
    let view = dom.add(parent, view_el);
    info.editor = Some(view);
    let doc = tab.editor.buf.doc();
    // Virtualised: only blocks around the scroll position, capped. A few above it are laid out
    // so the view can scroll up smoothly.
    let top = tab.top_block.min(doc.blocks.len().saturating_sub(1));
    let first = top.saturating_sub(8);
    let last = (top + 90).min(doc.blocks.len());
    info.marks.clear();
    note_blocks(app, dom, view, pal, info, tab, tab.mode, focused, first..last, false);
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
    });
}

/// Scroll each note view to its tab's position, now that its blocks have their heights, and
/// anchor the tab to the block at the top of the view.
pub fn scroll_views(app: &mut App, dom: &mut Dom, info: &ViewInfo) {
    for s in &info.scrolled {
        let Some(t) = app.pane_tab_mut(s.pane, s.tab) else { continue };
        if s.marks.is_empty() {
            t.scroll_to_block(0);
            continue;
        }
        let top = dom.content_rect(s.view).y;
        // Offsets of the blocks in the view's content; the first starts at the top, margin
        // included.
        let pos: Vec<f32> = (0..s.marks.len())
            .map(|k| if k == 0 { 0.0 } else { dom.nodes[s.marks[k].1].rect.y - top })
            .collect();
        let anchor = s.marks.iter().position(|&(b, _)| b >= t.top_block).unwrap_or(s.marks.len() - 1);
        let max = dom.max_scroll(s.view);
        let want = pos[anchor] + t.scroll_px;
        let mut y = want;
        if s.start {
            y = y.max(0.0);
        }
        if s.end {
            y = y.min(max);
        }
        if y != want {
            // Ran into the start or end of the note.
            t.scroll_pending = 0.0;
        }
        dom.set_scroll(s.view, y.clamp(0.0, max));
        let k = pos.iter().rposition(|&p| p <= y).unwrap_or(0);
        t.top_block = s.marks[k].0;
        t.scroll_px = y - pos[k];
    }
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
            NodeKind::BlockQuote => "cm-line cm-quote",
            NodeKind::Callout { .. } => "cm-line cm-callout",
            NodeKind::FrontMatter { .. } => "cm-line cm-frontmatter",
            NodeKind::ThematicBreak
                if mode != ViewMode::Source && !sels.iter().any(|x| x.head >= s && x.head <= e) =>
            {
                "cm-line cm-hr"
            }
            NodeKind::CommentBlock if mode == ViewMode::Reading => continue,
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
            ViewMode::LivePreview => !sels.iter().any(|x| x.start() <= e && x.end() >= s),
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
        let mut callout: Option<(Color, usize, usize)> = None;
        if rendered && matches!(root.kind, NodeKind::BlockQuote | NodeKind::Callout { .. }) {
            hide_quote_markers(&src, s, &mut block_decos);
            if let NodeKind::Callout { kind, fold, title } = root.kind {
                let first_end = s + src.find('\n').unwrap_or(src.len());
                let (ks, ke) = (b.start + kind.start as usize, b.start + kind.end as usize);
                let (ts, te) = (b.start + title.start as usize, b.start + title.end as usize);
                // Show the title, or the callout type when there is no title.
                let (shown_s, shown_e) = if te > ts { (ts, te) } else { (ks, ke) };
                let hide = |a: usize, b: usize| Deco { start: a, end: b, style: Deco_::Hide };
                block_decos.push(hide(s, shown_s));
                block_decos.push(hide(shown_e, first_end));
                block_decos.push(Deco { start: shown_s, end: shown_e, style: Deco_::Strong });
                if fold == md_engine::Fold::Closed {
                    block_decos.push(hide(first_end, e));
                }
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
        let mut math = if rendered && !class.ends_with("cm-hr") {
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
            block_display(&src, s, &block_decos, &math, pal)
        };
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
        // lines under the item's text, and the bullet would be the source's own `-`.
        if rendered && !folded_here && has_list_lines(&display) {
            list_block(dom, view, first + i, cut_lines(&display, &spans, &map), callout, &math, info);
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
                        .attr("aria-label", if folded { "Unfold heading" } else { "Fold heading" })
                        .action(format!("fold:{}", b.start)),
                );
            }
            info.blocks.push(bv);
        }
        if mode != ViewMode::Source {
            embeds(app, dom, view, &tab.path, &b.tree, &buf.slice(b.start, b.end()));
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
            let (display, spans, map) = block_display(text, start, &cell_decos, &[], pal);
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
    let total: f32 = weights.iter().sum();
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
                    .style(&format!("width: {:.3}%", weights[c] / total * 100.0))
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
        let (display, mut spans, map) = block_display(src, s, &block_decos, &[], pal);
        code_spans(&b.tree, b.start, src, s, &map, pal, &mut spans);
        dom.add(parent, El::div().class(class).text(display).spans(spans).action(action));
    }
    if doc.blocks.len() > n {
        dom.add(parent, El::div().class("embed-line").text("…").action(action));
    }
}
const IMAGE_MAX_WIDTH: f32 = 640.0;

/// Images and note embeds referenced by a block, shown below it.
fn embeds(app: &App, dom: &mut Dom, view: NodeId, source: &str, tree: &md_engine::Tree, block_text: &str) {
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
        } else {
            // Video, audio and anything else this build does not decode: a card that names the
            // file and hands it to the system player, instead of a line of raw English.
            let card = dom.add(view, El::div().class("media-card"));
            let icon = dom.add(card, El::div().class("media-card-icon"));
            dom.add(icon, icons::el("play"));
            let body = dom.add(card, El::div().class("media-card-body"));
            dom.add(body, El::div().class("media-card-title").text(path::file_name(rel)));
            dom.add(
                body,
                El::div()
                    .class("media-card-action")
                    .text("Abrir con el reproductor del sistema")
                    .action(format!("open-external:{rel}")),
            );
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

/// Shown in the main pane when no note is open: what to do next, with the hotkeys.
fn empty_state(app: &App, dom: &mut Dom, parent: NodeId) {
    let wrap = dom.add(parent, El::div().class("empty-state"));
    let card = dom.add(wrap, El::div().class("empty-state-container"));
    dom.add(card, El::div().class("empty-state-logo").image(logo()));
    dom.add(card, El::div().class("empty-state-title").text("No file is open"));
    let notes = app.store.index.notes().count();
    let sub = if notes == 0 {
        "This vault is empty. Create your first note to get started.".to_string()
    } else {
        format!("{notes} notes in this vault. Pick one on the left, or:")
    };
    dom.add(card, El::div().class("empty-state-subtitle").text(sub));
    let list = dom.add(card, El::div().class("empty-state-action-list"));
    for (icon, label, cmd) in [
        ("note-plus", "Create new note", "file-explorer:new-file"),
        ("switcher", "Go to file", "switcher:open"),
        ("command", "Open command palette", "command-palette:open"),
        ("calendar", "Open today's daily note", "daily-notes"),
        ("graph", "Open graph view", "graph:open"),
        ("settings", "Open settings", "app:open-settings"),
        ("vault", "Open another vault", "app:open-vault"),
    ] {
        let row = dom.add(list, El::div().class("empty-state-action").action(cmd));
        dom.add(row, icons::el(icon));
        dom.add(row, El::div().class("empty-state-action-label").text(label));
        dom.add(row, El::div().class("empty-state-action-hotkey").text(app.hotkey_for(cmd)));
    }
}

/// The application icon as an image, rasterised once.
pub fn logo() -> rustsidian_ui::Image {
    static LOGO: std::sync::OnceLock<rustsidian_ui::Image> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        const SIZE: u32 = 192;
        let mut pm = rustsidian_ui::Pixmap::new(SIZE, SIZE).expect("icon size is not zero");
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
        rustsidian_ui::Image(std::sync::Arc::new(pm))
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
            "{} nodes  ·  {} links",
            g.graph.nodes.len(),
            g.graph.edges.len()
        )),
    );
    dom.add(
        header,
        El::div()
            .class("graph-control")
            .class_if(g.auto_fit, "is-enabled")
            .text("Fit")
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
            Some((_, t)) => (format!("{t}▏"), false),
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
                    .attr("aria-label", "Restore default settings")
                    .action("graph:reset-settings"),
            );
            dom.add(b, icons::el("rotate-ccw"));
            let b = dom.add(
                head,
                El::div().class("clickable-icon").attr("aria-label", "Close").action("graph:toggle-panel"),
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
                field(dom, body, "search", &s.search, "Search files...");
                let f = &s.filters;
                toggle(dom, body, "Tags", f.show_tags, "graph:toggle-tags");
                toggle(dom, body, "Attachments", f.show_attachments, "graph:toggle-attachments");
                toggle(dom, body, "Existing files only", f.hide_unresolved, "graph:toggle-unresolved");
                toggle(dom, body, "Orphans", f.show_orphans, "graph:toggle-orphans");
                if g.local.is_some() {
                    slider(dom, body, &DEPTH);
                }
            }
            "color-groups" => {
                for (gi, cg) in s.color_groups.iter().enumerate() {
                    let row = dom.add(body, El::div().class("graph-color-group"));
                    field(dom, row, &format!("group:{gi}"), &cg.query, "Enter query...");
                    dom.add(
                        row,
                        El::div()
                            .class("graph-color-swatch")
                            .style(&format!("background-color: #{:06x}", cg.rgb))
                            .attr("aria-label", "Change color")
                            .action(format!("graph:group-color:{gi}")),
                    );
                    let x = dom.add(
                        row,
                        El::div()
                            .class("clickable-icon")
                            .attr("aria-label", "Remove group")
                            .action(format!("graph:group-remove:{gi}")),
                    );
                    dom.add(x, icons::el("x"));
                }
                dom.add(
                    body,
                    El::div().class("graph-button mod-wide").text("New group").action("graph:group-new"),
                );
            }
            "display" => {
                toggle(dom, body, "Arrows", s.show_arrows, "graph:toggle-arrows");
                for sl in &DISPLAY_SLIDERS {
                    slider(dom, body, sl);
                }
                let row = dom.add(body, El::div().class("graph-setting mod-end"));
                dom.add(row, El::div().class("graph-button").text("Animate").action("graph:animate"));
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
            info.scrolled.append(&mut own.scrolled);
            info.stack.push((pane, i, col, own.editor));
        }
    }
}

/// The main window: ribbon, sidebars, the split panes and the status bar.
fn chrome(app: &App, dom: &mut Dom, ws: NodeId, pal: &Palette, info: &mut ViewInfo, viewport_h: f32) {
    let show_ribbon = app.config.appearance.bool("showRibbon", true);
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
        ("calendar", "daily-notes", false),
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
    let left_w = app.anims.toward("left", app.left_open);
    if left_w > 0.01 {
        let mut el = El::div().class("workspace-split mod-left-split");
        if left_w < 1.0 {
            el = el.style(&format!("width: {:.1}px", 250.0 * left_w));
        }
        let left = dom.add(ws, el);
        let head = dom.add(left, El::div().class("sidebar-tab-header"));
        for (icon, cmd, panel) in [
            ("folder", "file-explorer:open", Panel::Files),
            ("search", "global-search:open", Panel::Search),
            ("bookmark", "bookmarks:open", Panel::Bookmarks),
            ("tag", "tag-pane:open", Panel::Tags),
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
        };
        if let Some(name) = app.disabled_by(cmd) {
            dom.add(left, El::div().class("pane-empty").text(format!("{name} is turned off.")));
        } else {
            match app.panel {
                Panel::Files => sidebar_files(app, dom, left, viewport_h),
                Panel::Search => sidebar_search(app, dom, left),
                Panel::Bookmarks => crate::panes::bookmarks(app, dom, left),
                Panel::Tags => crate::panes::tags(app, dom, left),
            }
        }
    }
    let root = dom.add(ws, El::div().class("workspace-split mod-root"));
    let right_w = app.anims.toward("right", app.right_open);
    let side = |w: f32| if w > 0.01 { 250.0 * w.min(1.0) } else { 0.0 };
    let ribbon_w = if show_ribbon { 44.0 } else { 0.0 };
    let width = app.window_w - ribbon_w - side(left_w) - side(right_w);
    split_layout(app, dom, root, &app.layout, &mut Vec::new(), pal, info, viewport_h, width);
    if right_w > 0.01 {
        let mut el = El::div().class("workspace-split mod-right-split");
        if right_w < 1.0 {
            el = el.style(&format!("width: {:.1}px", 250.0 * right_w));
        }
        let right = dom.add(ws, el);
        right_sidebar(app, dom, right);
    }
    let status = dom.add(0, El::div().class("status-bar"));
    // What the element under the pointer does, with its hotkey.
    let hint = app.hover_action.as_deref().and_then(|a| {
        let name = crate::shell::COMMANDS
            .iter()
            .find(|c| c.0 == a)
            .map(|c| c.1.to_string())
            .or_else(|| {
                a.strip_prefix("close-tab:").map(|_| "Close tab (middle click also closes)".to_string())
            })
            .or_else(|| a.strip_prefix("toggle-pin:").map(|_| "Unpin tab".to_string()))?;
        let key = app.hotkey_for(a);
        Some(if key.is_empty() { name } else { format!("{name}  ({key})") })
    });
    dom.add(status, El::div().class("status-bar-item status-bar-hint").text(hint.unwrap_or_default()));
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
        dom.add(status, El::div().class("status-bar-item").text(format!("{backlinks} backlinks")));
        let goal =
            t.meta.as_ref().and_then(|(_, m)| m.frontmatter.as_ref()?.iter().find(|(k, _)| k == "word-goal"));
        let goal = goal.and_then(|(_, v)| match v {
            md_engine::frontmatter::Value::Number(n) if *n >= 1.0 => Some(*n as u32),
            md_engine::frontmatter::Value::String(s) => s.trim().parse().ok().filter(|n| *n > 0),
            _ => None,
        });
        let count = match goal {
            Some(g) => format!("{words} / {g} words"),
            None => format!("{words} words"),
        };
        dom.add(status, El::div().class("status-bar-item").text(count));
        // 200 words a minute, a common estimate for silent reading.
        dom.add(status, El::div().class("status-bar-item").text(format!("{} min read", words.div_ceil(200))));
        dom.add(
            status,
            El::div()
                .class("status-bar-item")
                .text(format!("{} characters", t.editor.buf.rope().len_chars())),
        );
    }
    for s in &app.plugins.ext.reg.status {
        dom.add(status, El::div().class("status-bar-item").text(s.text.clone()));
    }
    dom.add(
        status,
        El::div().class("status-bar-item").text(format!("{} notes", app.store.index.notes().count())),
    );
}

/// Prompts, the hover preview, settings, menus and notices, over the window.
fn overlays(app: &App, dom: &mut Dom, viewport_h: f32) {
    if let Some(p) = &app.prompt {
        let t = app.anims.t("prompt");
        let mut el = El::div().class("prompt");
        if t < 1.0 {
            el = el.style(&format!("opacity: {t:.3}; top: {:.1}px", 68.0 + 12.0 * t));
        }
        let modal = dom.add(0, el);
        let placeholder = match p.kind {
            PromptKind::Commands => "Type a command…",
            PromptKind::Switcher => "Find or create a note…",
            PromptKind::Rename => "New name…",
            PromptKind::NewFolder => "Folder name…",
            PromptKind::Template => "Find a template…",
            PromptKind::Workspace => match p.target.as_str() {
                "save" => "Workspace name…",
                _ => "Find a workspace…",
            },
            PromptKind::BaseCell => "Empty to remove the property…",
        };
        let shown = if p.query.is_empty() { placeholder.to_string() } else { format!("{}▏", p.query) };
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
        let results = dom.add(modal, El::div().class("prompt-results"));
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
    if let Some(m) = &app.menu {
        // Open toward the inside of the window when there is no room at the pointer.
        let x =
            if m.x + crate::menu::WIDTH > app.window_w { (m.x - crate::menu::WIDTH).max(0.0) } else { m.x };
        let y = if m.y + m.height() > viewport_h { (viewport_h - m.height()).max(0.0) } else { m.y };
        let t = app.anims.t("menu");
        let el = El::div().class("menu").style(&format!(
            "left: {x:.1}px; top: {:.1}px; width: {:.0}px; opacity: {t:.3}",
            y - 4.0 * (1.0 - t),
            crate::menu::WIDTH
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
                    .action(format!("menu:{i}")),
            );
            dom.add(row, icons::el(item.icon));
            dom.add(row, El::div().class("menu-item-title").text(item.label.clone()));
            if !item.hotkey.is_empty() {
                dom.add(row, El::div().class("menu-item-hotkey").text(item.hotkey.clone()));
            }
        }
    }
    if let Some(n) = &app.notice {
        let t = app.anims.t("notice");
        let mut el = El::div().class("notice").attr("role", "status");
        if t < 1.0 {
            el = el.style(&format!("opacity: {t:.3}; top: {:.1}px", 12.0 * t));
        }
        let toast = dom.add(0, el);
        dom.add(toast, icons::el("warning"));
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
                "frame {:.2} ms · {} nodes",
                app.last_frame_us as f32 / 1000.0,
                dom.nodes.len()
            )),
        );
    }
}

/// The window's view. `solo` is a pane shown alone in a window of its own; the menus, prompts
/// and notices show in the window whose pane has the keyboard.
pub fn build(app: &App, viewport_h: f32, solo: Option<usize>) -> (Dom, ViewInfo) {
    let pal = palette(app);
    let mut info = ViewInfo::default();
    let mut dom = Dom::new();
    dom.nodes[0].el.classes.push(if app.dark { "theme-dark" } else { "theme-light" }.to_string());
    let ws = dom.add(0, El::div().class("workspace"));
    match solo {
        Some(p) => {
            let root = dom.add(ws, El::div().class("workspace-split mod-root"));
            pane(app, &mut dom, root, p, &pal, &mut info, viewport_h, app.window_w);
        }
        None => chrome(app, &mut dom, ws, &pal, &mut info, viewport_h),
    }
    let here = match solo {
        Some(p) => app.pane == p,
        None => !app.is_popped(app.pane),
    };
    if here {
        overlays(app, &mut dom, viewport_h);
    }
    for (id, y) in &app.scrolls {
        if let Some(n) = dom.by_id(id) {
            dom.nodes[n].scroll_y = *y;
        }
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
    if !app.config.app.bool("spellcheck", true) {
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
    let pop = dom.add(0, El::div().class("editor-suggest").style(&format!("left: {x:.1}px; top: {y:.1}px")));
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
    let x = if px + 12.0 + POPOVER_W > app.window_w { (px - 12.0 - POPOVER_W).max(0.0) } else { px + 12.0 };
    let y = if py + 16.0 + POPOVER_H > viewport_h { (py - 8.0 - POPOVER_H).max(0.0) } else { py + 16.0 };
    let pop = dom.add(
        0,
        El::div().class("popover hover-popover").style(&format!(
            "left: {x:.1}px; top: {y:.1}px; width: {POPOVER_W}px; max-height: {POPOVER_H}px"
        )),
    );
    let action = format!("open:{}", h.path);
    dom.add(
        pop,
        El::div().class("hover-popover-title").text(path::stem(&h.path).to_string()).action(action.clone()),
    );
    if h.text.trim().is_empty() {
        dom.add(pop, El::div().class("hover-popover-empty").text("No preview for this file"));
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
