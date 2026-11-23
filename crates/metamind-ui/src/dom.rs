//! Retained element tree with CSS styling, a flexbox-subset layout and painting.
//!
//! Layout supports `display: flex|block|none`, `flex-direction`, `flex-grow`, `flex-shrink`,
//! `flex-basis`, `width/height` (px, %, auto) with min/max, padding, margin, border, `gap`,
//! `row-gap`/`column-gap`, `flex-wrap: wrap` on a row, `align-items`, `align-content`,
//! `justify-content`, `position: absolute` with insets, and `overflow`.
//! Not supported: grid, wraps in a column, floats, inline layout of mixed elements.

use crate::scene::{Cmd, Rect, Scene};
use crate::text::{Fonts, Span, TextStyle};
use theme_engine::{Color, Computed, Element, Env, LengthCx, Theme};

pub type NodeId = usize;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Dim {
    #[default]
    Auto,
    Px(f32),
    Pct(f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    #[default]
    Stretch,
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Style {
    pub display_none: bool,
    pub row: bool,
    pub grow: f32,
    pub shrink: f32,
    pub width: Dim,
    pub height: Dim,
    pub min_w: f32,
    pub max_w: f32,
    pub min_h: f32,
    pub max_h: f32,
    /// top, right, bottom, left
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    pub border: [f32; 4],
    pub border_color: [Color; 4],
    pub background: Color,
    pub radius: f32,
    pub gap: f32,
    /// Between lines of a wrapped row (`row-gap`). A `gap` shorthand sets both this and `gap`.
    pub gap_cross: f32,
    /// `flex-wrap: wrap` on a row. A column ignores it, as the browser does for a fixed height.
    pub wrap: bool,
    pub align: Align,
    pub align_content: Align,
    pub justify: Align,
    pub clip: bool,
    pub scroll: bool,
    pub absolute: bool,
    pub inset: [Option<f32>; 4],
    pub text: TextStyle,
    pub center_text: bool,
    /// `text-overflow: ellipsis`: text that does not fit ends in `…` instead of being cut.
    pub ellipsis: bool,
    pub opacity: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            display_none: false,
            row: false,
            grow: 0.0,
            shrink: 1.0,
            width: Dim::Auto,
            height: Dim::Auto,
            min_w: 0.0,
            max_w: f32::MAX,
            min_h: 0.0,
            max_h: f32::MAX,
            padding: [0.0; 4],
            margin: [0.0; 4],
            border: [0.0; 4],
            border_color: [Color::TRANSPARENT; 4],
            background: Color::TRANSPARENT,
            radius: 0.0,
            gap: 0.0,
            gap_cross: 0.0,
            wrap: false,
            align: Align::Stretch,
            align_content: Align::Start,
            justify: Align::Start,
            clip: false,
            scroll: false,
            absolute: false,
            inset: [None; 4],
            text: TextStyle {
                size: 14.0,
                line_height: 20.0,
                color: Color::rgb(220, 220, 220),
                bold: false,
                italic: false,
                mono: false,
                wrap: true,
                family: None,
            },
            center_text: false,
            ellipsis: false,
            opacity: 1.0,
        }
    }
}

/// The ranges of children that fit on each line of a wrapping row. A child wider than the whole
/// line gets a line to itself, which is what a browser does: it is never cut in half to fit.
fn break_lines(mains: &[f32], main_avail: f32, gap: f32) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0.0f32;
    for (i, &w) in mains.iter().enumerate() {
        let add = if i == start { w } else { w + gap };
        if i > start && main_avail.is_finite() && used + add > main_avail {
            lines.push((start, i));
            start = i;
            used = w;
        } else {
            used += add;
        }
    }
    if !mains.is_empty() {
        lines.push((start, mains.len()));
    }
    lines
}

/// `x` bounded by `min` and `max` without the panic `f32::clamp` raises when a stylesheet puts
/// `min` above `max`. A theme with `max-height: calc(100% - 60px)` resolves the percentage against
/// nothing and comes out negative; that used to take the whole window down with a panic inside a
/// layout pass instead of simply asking for no height.
fn bounded(x: f32, min: f32, max: f32) -> f32 {
    x.max(min).min(max)
}

fn dim(c: &Computed, name: &str, cx: &LengthCx) -> Dim {
    match c.get(name).map(str::trim) {
        None | Some("auto") => Dim::Auto,
        Some(v) => match v.strip_suffix('%').and_then(|p| p.trim().parse::<f32>().ok()) {
            Some(p) => Dim::Pct(p),
            None => theme_engine::parse_length(v, cx).map_or(Dim::Auto, Dim::Px),
        },
    }
}

fn align(v: Option<&str>, default: Align) -> Align {
    match v.map(str::trim) {
        Some("center") => Align::Center,
        Some("flex-start" | "start") => Align::Start,
        Some("flex-end" | "end") => Align::End,
        Some("space-between") => Align::SpaceBetween,
        Some("stretch") => Align::Stretch,
        _ => default,
    }
}

impl Style {
    /// Resolve typed style values from a computed CSS style.
    pub fn from_computed(c: &Computed, parent_font: f32, root_font: f32) -> Style {
        let d = Style::default();
        let base = LengthCx { font_size: parent_font, root_font_size: root_font, ..LengthCx::default() };
        let size = c.length("font-size", &base).unwrap_or(parent_font);
        let cx = LengthCx { font_size: size, ..base };
        let len = |name: &str| c.length(name, &cx);
        let sides = |prefix: &str, suffix: &str| {
            ["top", "right", "bottom", "left"]
                .map(|s| len(&format!("{prefix}-{s}{suffix}")).unwrap_or(0.0).max(0.0))
        };
        let line_height = match c.get("line-height").map(str::trim) {
            Some(v) if v.parse::<f32>().is_ok() => v.parse::<f32>().unwrap_or(1.5) * size,
            Some(v) => theme_engine::parse_length(v, &cx).unwrap_or(size * 1.5),
            None => size * 1.5,
        };
        let family_list = c.get("font-family").map(str::trim).filter(|f| !f.is_empty());
        let family = family_list.unwrap_or("").to_ascii_lowercase();
        let overflow = |axis: &str| c.get(axis).map(str::trim).unwrap_or("visible").to_string();
        let (ox, oy) = (overflow("overflow-x"), overflow("overflow-y"));
        let display = c.get("display").map(str::trim).unwrap_or("block");
        Style {
            display_none: display == "none",
            row: display.contains("flex")
                && !c.get("flex-direction").is_some_and(|v| v.trim().starts_with("column")),
            grow: c
                .number("flex-grow")
                .or_else(|| c.get("flex").and_then(|f| f.split_whitespace().next()?.parse().ok()))
                .unwrap_or(0.0),
            shrink: c.number("flex-shrink").unwrap_or(1.0),
            width: dim(c, "width", &cx),
            height: dim(c, "height", &cx),
            min_w: len("min-width").unwrap_or(0.0),
            max_w: len("max-width").unwrap_or(f32::MAX),
            min_h: len("min-height").unwrap_or(0.0),
            max_h: len("max-height").unwrap_or(f32::MAX),
            padding: sides("padding", ""),
            margin: sides("margin", ""),
            border: sides("border", "-width"),
            border_color: ["top", "right", "bottom", "left"]
                .map(|s| c.color(&format!("border-{s}-color")).unwrap_or(Color::TRANSPARENT)),
            background: c.color("background-color").unwrap_or(Color::TRANSPARENT),
            radius: len("border-radius").unwrap_or(0.0),
            gap: len(
                if display.contains("flex")
                    && !c.get("flex-direction").is_some_and(|v| v.trim().starts_with("column"))
                {
                    "column-gap"
                } else {
                    "row-gap"
                },
            )
            .unwrap_or(0.0),
            gap_cross: len(
                if display.contains("flex")
                    && !c.get("flex-direction").is_some_and(|v| v.trim().starts_with("column"))
                {
                    "row-gap"
                } else {
                    "column-gap"
                },
            )
            .unwrap_or(0.0),
            wrap: c.get("flex-wrap").is_some_and(|v| v.trim().starts_with("wrap")),
            align: align(c.get("align-items"), Align::Stretch),
            align_content: align(c.get("align-content"), Align::Start),
            justify: align(c.get("justify-content"), Align::Start),
            clip: ox != "visible" || oy != "visible",
            scroll: matches!(oy.as_str(), "auto" | "scroll"),
            absolute: matches!(c.get("position").map(str::trim), Some("absolute" | "fixed")),
            inset: ["top", "right", "bottom", "left"].map(len),
            text: TextStyle {
                size,
                line_height,
                color: c.color("color").unwrap_or(d.text.color),
                bold: c.font_weight() >= 600,
                italic: c.get("font-style").is_some_and(|v| v.trim() == "italic"),
                mono: family.contains("mono") || family.contains("code") || family.contains("courier"),
                wrap: !matches!(c.get("white-space").map(str::trim), Some("nowrap" | "pre")),
                family: family_list.map(Into::into),
            },
            center_text: c.get("text-align").is_some_and(|v| v.trim() == "center"),
            ellipsis: c.get("text-overflow").is_some_and(|v| v.trim() == "ellipsis"),
            opacity: c.number("opacity").unwrap_or(1.0).clamp(0.0, 1.0),
        }
    }
}

/// Element description used to build the tree.
#[derive(Clone, Debug, Default)]
pub struct El {
    pub tag: &'static str,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub text: Option<String>,
    pub spans: Vec<Span>,
    pub inline_style: Option<String>,
    pub action: Option<String>,
    /// Painted over the content box, stretched to fit.
    pub image: Option<crate::scene::Image>,
}

impl El {
    pub fn new(tag: &'static str) -> El {
        El { tag, ..El::default() }
    }
    pub fn div() -> El {
        El::new("div")
    }
    pub fn class(mut self, classes: &str) -> El {
        self.classes.extend(classes.split_whitespace().map(str::to_string));
        self
    }
    pub fn class_if(self, cond: bool, classes: &str) -> El {
        if cond {
            self.class(classes)
        } else {
            self
        }
    }
    pub fn id(mut self, id: &str) -> El {
        self.id = Some(id.to_string());
        self
    }
    pub fn attr(mut self, k: &str, v: &str) -> El {
        self.attrs.push((k.to_string(), v.to_string()));
        self
    }
    pub fn text(mut self, t: impl Into<String>) -> El {
        self.text = Some(t.into());
        self
    }
    pub fn spans(mut self, spans: Vec<Span>) -> El {
        self.spans = spans;
        self
    }
    pub fn image(mut self, image: crate::scene::Image) -> El {
        self.image = Some(image);
        self
    }
    pub fn style(mut self, css: &str) -> El {
        self.inline_style = Some(css.to_string());
        self
    }
    /// Command (with optional argument) dispatched when the element is clicked.
    pub fn action(mut self, a: impl Into<String>) -> El {
        self.action = Some(a.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub el: El,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub style: Style,
    pub computed: Computed,
    /// Border box in window coordinates (after ancestors' scrolling).
    pub rect: Rect,
    pub scroll_y: f32,
    /// Height of the content, for scroll limits and scrollbars.
    pub content_h: f32,
    pub hover: bool,
    pub focus: bool,
    pub active: bool,
    /// Text drawn instead of `el.text` when it had to be shortened to fit (see `Style::ellipsis`).
    pub shown: Option<String>,
    /// Extra rectangles painted under the text, relative to the content box (selections, carets
    /// are painted above via `overlays_top`).
    pub overlays: Vec<(Rect, Color)>,
    pub overlays_top: Vec<(Rect, Color)>,
    /// Extra commands in window coordinates, painted above the node's own content and below
    /// its children. Filled in after layout (custom-drawn views such as the graph).
    pub draw: Vec<Cmd>,
}

#[derive(Clone, Debug)]
pub struct Dom {
    pub nodes: Vec<Node>,
    /// Colour of the scrollbar thumb, from the theme (`--scrollbar-thumb`). `None` keeps the
    /// painter's own grey, which is what the welcome screen and the tests get.
    pub scrollbar_thumb: Option<Color>,
}

#[derive(Clone, Copy)]
struct ElRef<'a>(&'a Dom, NodeId);

impl Element for ElRef<'_> {
    fn tag(&self) -> &str {
        self.0.nodes[self.1].el.tag
    }
    fn id(&self) -> Option<&str> {
        self.0.nodes[self.1].el.id.as_deref()
    }
    fn has_class(&self, class: &str) -> bool {
        self.0.nodes[self.1].el.classes.iter().any(|c| c == class)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.0.nodes[self.1].el.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn parent(&self) -> Option<Self> {
        self.0.nodes[self.1].parent.map(|p| ElRef(self.0, p))
    }
    fn prev_sibling(&self) -> Option<Self> {
        let p = self.0.nodes[self.1].parent?;
        let sibs = &self.0.nodes[p].children;
        let i = sibs.iter().position(|&c| c == self.1)?;
        (i > 0).then(|| ElRef(self.0, sibs[i - 1]))
    }
    fn is_last_child(&self) -> bool {
        self.0.nodes[self.1].parent.is_some_and(|p| self.0.nodes[p].children.last() == Some(&self.1))
    }
    fn hover(&self) -> bool {
        self.0.nodes[self.1].hover
    }
    fn focus(&self) -> bool {
        self.0.nodes[self.1].focus
    }
    fn active(&self) -> bool {
        self.0.nodes[self.1].active
    }
}

impl Default for Dom {
    fn default() -> Self {
        Dom::new()
    }
}

impl Dom {
    /// A tree with a `body` root (node 0).
    pub fn new() -> Dom {
        let mut d = Dom { nodes: Vec::new(), scrollbar_thumb: None };
        d.push(None, El::new("body"));
        d
    }

    fn push(&mut self, parent: Option<NodeId>, el: El) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            el,
            parent,
            children: Vec::new(),
            style: Style::default(),
            computed: Computed::default(),
            rect: Rect::default(),
            scroll_y: 0.0,
            content_h: 0.0,
            hover: false,
            focus: false,
            active: false,
            shown: None,
            overlays: Vec::new(),
            overlays_top: Vec::new(),
            draw: Vec::new(),
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        id
    }

    pub fn add(&mut self, parent: NodeId, el: El) -> NodeId {
        self.push(Some(parent), el)
    }

    /// Find the first node with this id attribute.
    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.el.id.as_deref() == Some(id))
    }

    /// Compute styles for every node (parents before children).
    pub fn style(&mut self, theme: &Theme, env: Env) {
        let root_font = 16.0;
        // The cascade is the costly part, and many nodes repeat one (rows, icons, labels).
        // Nodes whose parents are styled alike and that match the same rules with the same
        // inline style get the same result, so it is computed once per such group.
        type Key = (usize, Vec<(u16, u32, u32)>, Option<String>);
        let mut groups: std::collections::HashMap<Key, NodeId> = std::collections::HashMap::new();
        let mut group_of = vec![0; self.nodes.len()];
        for id in 0..self.nodes.len() {
            let parent = self.nodes[id].parent;
            let classes: Vec<&str> = self.nodes[id].el.classes.iter().map(String::as_str).collect();
            let rules = theme.matched_rules(&ElRef(self, id), &classes, env);
            let key =
                (parent.map_or(usize::MAX, |p| group_of[p]), rules, self.nodes[id].el.inline_style.clone());
            if let Some(&same) = groups.get(&key) {
                group_of[id] = same;
                let (computed, style) = (self.nodes[same].computed.clone(), self.nodes[same].style.clone());
                let n = &mut self.nodes[id];
                n.computed = computed;
                n.style = style;
                continue;
            }
            let computed =
                theme.compute_matched(&key.1, parent.map(|p| &self.nodes[p].computed), key.2.as_deref());
            let parent_font = parent.map_or(root_font, |p| self.nodes[p].style.text.size);
            let style = Style::from_computed(&computed, parent_font, root_font);
            group_of[id] = id;
            groups.insert(key, id);
            let n = &mut self.nodes[id];
            n.computed = computed;
            n.style = style;
        }
    }

    fn h_extra(s: &Style) -> f32 {
        s.padding[1] + s.padding[3] + s.border[1] + s.border[3]
    }
    fn v_extra(s: &Style) -> f32 {
        s.padding[0] + s.padding[2] + s.border[0] + s.border[2]
    }

    fn flow_children(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&c| !self.nodes[c].style.display_none && !self.nodes[c].style.absolute)
            .collect()
    }

    /// Preferred border-box size given the widest it may be.
    fn intrinsic(&self, fonts: &mut Fonts, id: NodeId, max_w: f32) -> (f32, f32) {
        let n = &self.nodes[id];
        let s = &n.style;
        let (hx, vx) = (Self::h_extra(s), Self::v_extra(s));
        let fixed_w = match s.width {
            Dim::Px(w) => Some(w),
            Dim::Pct(p) if max_w.is_finite() => Some(max_w * p / 100.0),
            _ => None,
        };
        // An unbounded width stays infinite: `max-width: none` is f32::MAX, and a finite bound
        // would let a grandchild's percentage resolve against it.
        let bound = fixed_w.unwrap_or(max_w).min(s.max_w);
        let avail = if bound >= f32::MAX { f32::INFINITY } else { (bound - hx).max(0.0) };
        let (mut w, mut h) = (0.0f32, 0.0f32);
        if let Some(text) = &n.el.text {
            let wrap = s.text.wrap.then_some(avail.min(f32::MAX));
            let (tw, th) = fonts.measure(text, &n.el.spans, &s.text, wrap);
            (w, h) = (tw, th);
        } else {
            let kids = self.flow_children(id);
            let gaps = s.gap * kids.len().saturating_sub(1) as f32;
            // Each child's preferred width, kept so a row can measure the height its children
            // really get once they are squeezed (wrapped text needs another line).
            let mut preferred: Vec<(f32, f32)> = Vec::with_capacity(kids.len());
            for &c in &kids {
                let m = &self.nodes[c].style.margin;
                let size = self.intrinsic(fonts, c, if s.row { f32::INFINITY } else { avail - m[1] - m[3] });
                let size = self.row_child_size(
                    fonts,
                    c,
                    if s.row && avail.is_finite() { Some(avail) } else { None },
                    size,
                );
                let (cw, ch) = size;
                preferred.push((cw, ch));
                let (cw, ch) = (cw + m[1] + m[3], ch + m[0] + m[2]);
                if s.row {
                    w += cw;
                    h = h.max(ch);
                } else {
                    w = w.max(cw);
                    h += ch;
                }
            }
            if s.row {
                w += gaps;
                if s.wrap && avail.is_finite() {
                    // A wrapping row is as tall as its lines, and as wide as its widest line.
                    let (widest, total) = self.wrap_metrics(&kids, &preferred, avail, s);
                    w = widest;
                    h = total;
                } else if avail.is_finite() && w > avail {
                    h = self.row_height(fonts, &kids, &preferred, avail, w);
                }
            } else {
                h += gaps;
            }
        }
        let w = bounded(fixed_w.unwrap_or(w + hx), s.min_w, s.max_w);
        let h = bounded(
            match s.height {
                Dim::Px(v) => v,
                _ => h + vx,
            },
            s.min_h,
            s.max_h,
        );
        (w, h)
    }

    /// Height a node takes once it is laid out at exactly `w`. `w` already had its percentage
    /// width resolved by the caller, and `intrinsic` would resolve that percentage again against
    /// it: a table cell with `width: 30%` would measure its text at 30% of its own column.
    fn height_at(&self, fonts: &mut Fonts, id: NodeId, w: f32) -> f32 {
        let n = &self.nodes[id];
        if let Some(text) = &n.el.text {
            let avail = (w - Self::h_extra(&n.style)).max(0.0);
            let wrap = n.style.text.wrap.then_some(avail);
            return fonts.measure(text, &n.el.spans, &n.style.text, wrap).1 + Self::v_extra(&n.style);
        }
        self.intrinsic(fonts, id, w).1
    }

    /// Preferred size of a child of a flex row, at the row's inner width. A percentage width is
    /// a commitment, not a suggestion: the row measures that child at the width its percentage
    /// names, so its height is the height its text will really take once it wraps there. Without
    /// this a table cell was measured at the width of its unwrapped text and its row came out the
    /// wrong height.
    fn row_child_size(
        &self,
        fonts: &mut Fonts,
        id: NodeId,
        avail: Option<f32>,
        (w, h): (f32, f32),
    ) -> (f32, f32) {
        let (Some(avail), Dim::Pct(p)) = (avail, self.nodes[id].style.width) else { return (w, h) };
        let cs = &self.nodes[id].style;
        let w = bounded(avail * p / 100.0, cs.min_w, cs.max_w);
        (w, self.height_at(fonts, id, w))
    }

    /// Widths `place` gives the children of a flex row that has less room than they asked for:
    /// only the ones that opted in (grow, clipping or wrapping text) give up space, in proportion
    /// to the width each of them asked for. Whatever needs a child's final width before it is
    /// placed — a row's own height, the height of a text that wraps at that width — has to run
    /// this same arithmetic, or the two disagree and the next row is drawn over the wrapped text.
    fn shrunk_row(&self, kids: &[NodeId], sizes: &mut [(f32, f32)], free: f32) {
        let shrinkable: Vec<usize> = kids
            .iter()
            .enumerate()
            .filter(|(_, &c)| {
                let cs = &self.nodes[c].style;
                cs.shrink > 0.0
                    && !matches!(cs.width, Dim::Px(_))
                    && (cs.grow > 0.0 || cs.clip || (self.nodes[c].el.text.is_some() && cs.text.wrap))
            })
            .map(|(i, _)| i)
            .collect();
        let total: f32 = shrinkable.iter().map(|&i| sizes[i].0).sum();
        if total <= 0.0 {
            return;
        }
        for &i in &shrinkable {
            sizes[i].0 = (sizes[i].0 + free * sizes[i].0 / total).max(0.0);
        }
    }

    /// Height of a flex row that is narrower than its children's preferred sizes. The children
    /// are squeezed exactly as `place` will squeeze them and measured again at the width they
    /// really get: wrapped text needs another line there, and without it the row stays a line
    /// short and the next block is drawn over the text that wrapped.
    fn row_height(
        &self,
        fonts: &mut Fonts,
        kids: &[NodeId],
        preferred: &[(f32, f32)],
        avail: f32,
        total: f32,
    ) -> f32 {
        let mut sizes = preferred.to_vec();
        self.shrunk_row(kids, &mut sizes, avail - total);
        let mut h = 0.0f32;
        for (i, &c) in kids.iter().enumerate() {
            let cs = &self.nodes[c].style;
            let m = cs.margin;
            let (asked, asked_h) = preferred[i];
            // Measured again at its final width: text gains a line, a nested row gains the
            // height of the text it holds.
            let ch = if sizes[i].0 < asked { self.height_at(fonts, c, sizes[i].0) } else { asked_h };
            h = h.max(ch + m[0] + m[2]);
        }
        h
    }

    /// Lay out the tree for a viewport of `w` x `h` logical pixels.
    pub fn layout(&mut self, fonts: &mut Fonts, w: f32, h: f32) {
        self.place(fonts, 0, Rect::new(0.0, 0.0, w, h));
    }

    /// Assign `rect` (border box) to `id` and lay out its children inside it.
    /// The longest prefix of `text` that fits in `avail` once `…` is added, or `None` when the
    /// whole text fits already.
    fn shorten(&self, fonts: &mut Fonts, id: NodeId, text: &str, avail: f32) -> Option<String> {
        let n = &self.nodes[id];
        let (w, _) = fonts.measure(text, &n.el.spans, &n.style.text, None);
        if w <= avail || avail <= 0.0 {
            return None;
        }
        let fits = |fonts: &mut Fonts, upto: usize| {
            let mut s = text[..upto].trim_end().to_string();
            s.push('…');
            fonts.measure(&s, &n.el.spans, &n.style.text, None).0 <= avail
        };
        let chars = text.char_indices().map(|(i, c)| i + c.len_utf8()).collect::<Vec<_>>();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(fonts, chars[mid - 1]) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let mut s = text[..chars.get(lo.saturating_sub(1)).copied().unwrap_or(0)].trim_end().to_string();
        s.push('…');
        Some(s)
    }

    fn place(&mut self, fonts: &mut Fonts, id: NodeId, rect: Rect) {
        self.nodes[id].rect = rect;
        let s = self.nodes[id].style.clone();
        if s.ellipsis && !s.text.wrap {
            if let Some(text) = self.nodes[id].el.text.clone() {
                if self.nodes[id].el.spans.is_empty() {
                    let avail = (rect.w - Self::h_extra(&s)).max(0.0);
                    if let Some(short) = self.shorten(fonts, id, &text, avail) {
                        self.nodes[id].shown = Some(short);
                    }
                }
            }
        }
        let inner = Rect::new(
            rect.x + s.border[3] + s.padding[3],
            rect.y + s.border[0] + s.padding[0],
            (rect.w - Self::h_extra(&s)).max(0.0),
            (rect.h - Self::v_extra(&s)).max(0.0),
        );
        let scroll = self.nodes[id].scroll_y;
        let kids = self.flow_children(id);
        let gaps = s.gap * kids.len().saturating_sub(1) as f32;
        // Base main-axis sizes.
        let main_avail = if s.row { inner.w } else { inner.h };
        let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(kids.len());
        let mut used = gaps;
        let mut grow_total = 0.0;
        for &c in &kids {
            let cs = &self.nodes[c].style;
            let m = cs.margin;
            let cross_avail = if s.row { f32::INFINITY } else { inner.w - m[1] - m[3] };
            let (mut cw, mut ch) = self.intrinsic(fonts, c, cross_avail);
            if let Dim::Pct(p) = cs.height {
                ch = bounded(inner.h * p / 100.0, cs.min_h, cs.max_h);
            }
            if let Dim::Pct(p) = cs.width {
                // A percentage is a suggestion: `min-width` and `max-width` still bound it.
                cw = bounded(inner.w * p / 100.0, cs.min_w, cs.max_w);
            }
            if !s.row && (s.align == Align::Stretch || cs.width != Dim::Auto) && cs.width == Dim::Auto {
                cw = bounded(inner.w - m[1] - m[3], cs.min_w, cs.max_w);
            }
            used += if s.row { cw + m[1] + m[3] } else { ch + m[0] + m[2] };
            grow_total += cs.grow;
            sizes.push((cw, ch));
        }
        // A wrapping row is laid out a line at a time, by `place_wrap`. It is a path of its own
        // on purpose: the single-line path below is what every existing panel goes through, and
        // changing it to also handle lines would risk a regression nobody sees until it is on
        // screen. Absolute children are placed the same way by both.
        if s.row && s.wrap {
            let content_h = self.place_wrap(fonts, inner, &s, &kids, sizes, scroll);
            self.nodes[id].content_h = content_h;
            self.place_absolute(fonts, id, rect);
            return;
        }
        // Distribute free space (grow) or remove overflow (shrink).
        let definite_main = s.row || self.nodes[id].style.height != Dim::Auto || rect.h > 0.0;
        let free = main_avail - used;
        if definite_main && free > 0.0 && grow_total > 0.0 {
            for (i, &c) in kids.iter().enumerate() {
                let g = self.nodes[c].style.grow;
                if g > 0.0 {
                    let add = free * g / grow_total;
                    if s.row {
                        sizes[i].0 += add;
                    } else {
                        sizes[i].1 += add;
                    }
                }
            }
        } else if definite_main && free < 0.0 && !s.scroll {
            // Only children that opted in (grow, clipping or wrapping text) give up space.
            if s.row {
                self.shrunk_row(&kids, &mut sizes, free);
            } else {
                let shrinkable: Vec<usize> = kids
                    .iter()
                    .enumerate()
                    .filter(|(_, &c)| {
                        let cs = &self.nodes[c].style;
                        cs.shrink > 0.0 && !matches!(cs.height, Dim::Px(_)) && (cs.grow > 0.0 || cs.clip)
                    })
                    .map(|(i, _)| i)
                    .collect();
                let total: f32 = shrinkable.iter().map(|&i| sizes[i].1).sum();
                if total > 0.0 {
                    for &i in &shrinkable {
                        sizes[i].1 = (sizes[i].1 + free * sizes[i].1 / total).max(0.0);
                    }
                }
            }
        }
        let used: f32 = gaps
            + kids
                .iter()
                .zip(&sizes)
                .map(|(&c, sz)| {
                    let m = self.nodes[c].style.margin;
                    if s.row {
                        sz.0 + m[1] + m[3]
                    } else {
                        sz.1 + m[0] + m[2]
                    }
                })
                .sum::<f32>();
        let leftover = (main_avail - used).max(0.0);
        let (mut cursor, between) = match s.justify {
            Align::Center => (leftover / 2.0, 0.0),
            Align::End => (leftover, 0.0),
            Align::SpaceBetween if kids.len() > 1 => (0.0, leftover / (kids.len() - 1) as f32),
            _ => (0.0, 0.0),
        };
        for (i, &c) in kids.iter().enumerate() {
            let cs = self.nodes[c].style.clone();
            let m = cs.margin;
            let (mut cw, mut ch) = sizes[i];
            let r = if s.row {
                // Text that was squeezed needs its height recomputed at the new width.
                if self.nodes[c].el.text.is_some() || cw < sizes[i].0 {
                    ch = ch.max(self.height_at(fonts, c, cw));
                }
                let cross = inner.h - m[0] - m[2];
                let y = match s.align {
                    Align::Stretch if cs.height == Dim::Auto && rect.h > 0.0 => {
                        ch = bounded(cross, cs.min_h, cs.max_h);
                        0.0
                    }
                    Align::Center => (cross - ch) / 2.0,
                    Align::End => cross - ch,
                    _ => 0.0,
                };
                let r = Rect::new(inner.x + cursor + m[3], inner.y + m[0] + y - scroll, cw, ch);
                cursor += cw + m[1] + m[3] + s.gap + between;
                r
            } else {
                let cross = inner.w - m[1] - m[3];
                let x = match s.align {
                    Align::Center => (cross - cw) / 2.0,
                    Align::End => cross - cw,
                    _ => 0.0,
                };
                if s.align == Align::Stretch && cs.width == Dim::Auto {
                    cw = bounded(cross, cs.min_w, cs.max_w);
                    // Wrapped text may need more height at the final width.
                    if self.nodes[c].el.text.is_some() && cs.height == Dim::Auto && cs.grow == 0.0 {
                        ch = self.intrinsic(fonts, c, cw).1;
                    }
                }
                let r = Rect::new(inner.x + m[3] + x, inner.y + cursor + m[0] - scroll, cw, ch);
                cursor += ch + m[0] + m[2] + s.gap + between;
                r
            };
            self.place(fonts, c, r);
        }
        // The bottom padding is the room the content leaves when it is scrolled all the way
        // down, the same way the top padding starts it: without it the last row of a long
        // list sits flush against the edge of its scrollport and looks cut.
        self.nodes[id].content_h = if s.row {
            inner.h
        } else if kids.is_empty() {
            0.0
        } else {
            cursor - s.gap - between + s.padding[2]
        };
        self.place_absolute(fonts, id, rect);
    }

    /// Lays out the absolutely positioned children against this node's padding box.
    fn place_absolute(&mut self, fonts: &mut Fonts, id: NodeId, rect: Rect) {
        let s = self.nodes[id].style.clone();
        let abs: Vec<NodeId> = self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&c| self.nodes[c].style.absolute && !self.nodes[c].style.display_none)
            .collect();
        if abs.is_empty() {
            return;
        }
        let pad = Rect::new(
            rect.x + s.border[3],
            rect.y + s.border[0],
            rect.w - s.border[1] - s.border[3],
            rect.h - s.border[0] - s.border[2],
        );
        for c in abs {
            let cs = self.nodes[c].style.clone();
            let [top, right, bottom, left] = cs.inset;
            let avail_w = pad.w - left.unwrap_or(0.0) - right.unwrap_or(0.0);
            let (iw, ih) = self.intrinsic(fonts, c, avail_w);
            let w = match (cs.width, left, right) {
                (Dim::Px(w), _, _) => w,
                (Dim::Pct(p), _, _) => pad.w * p / 100.0,
                (Dim::Auto, Some(_), Some(_)) => avail_w,
                _ => iw.min(avail_w.max(0.0)),
            };
            let h = match (cs.height, top, bottom) {
                (Dim::Px(h), _, _) => h,
                (Dim::Pct(p), _, _) => pad.h * p / 100.0,
                (Dim::Auto, Some(t), Some(b)) => pad.h - t - b,
                _ => ih,
            }
            .min(cs.max_h);
            let x = match (left, right) {
                (Some(l), _) => pad.x + l,
                (None, Some(r)) => pad.x + pad.w - r - w,
                _ => pad.x + (pad.w - w) / 2.0,
            };
            let y = match (top, bottom) {
                (Some(t), _) => pad.y + t,
                (None, Some(b)) => pad.y + pad.h - b - h,
                _ => pad.y,
            };
            self.place(fonts, c, Rect::new(x, y, w, h));
        }
    }

    /// The width of the widest line and the total height of all lines of a wrapping row that is
    /// laid out `avail` wide. Used to measure the row before it is placed, so a popover can size
    /// itself to its wrapped content.
    fn wrap_metrics(&self, kids: &[NodeId], preferred: &[(f32, f32)], avail: f32, s: &Style) -> (f32, f32) {
        let mains: Vec<f32> = kids
            .iter()
            .zip(preferred)
            .map(|(&c, &(cw, _))| {
                let m = self.nodes[c].style.margin;
                cw + m[1] + m[3]
            })
            .collect();
        let lines = break_lines(&mains, avail, s.gap);
        let mut widest = 0.0f32;
        let mut total = 0.0f32;
        for &(a, b) in &lines {
            let w: f32 = mains[a..b].iter().sum::<f32>() + s.gap * (b - a).saturating_sub(1) as f32;
            widest = widest.max(w);
            let mut h = 0.0f32;
            for i in a..b {
                let m = self.nodes[kids[i]].style.margin;
                h = h.max(preferred[i].1 + m[0] + m[2]);
            }
            total += h;
        }
        total += s.gap_cross * lines.len().saturating_sub(1) as f32;
        (widest.min(avail), total)
    }

    /// Places the children of a wrapping row, one line at a time, and returns the height the
    /// content takes (for a scrolling container). Each line grows and shrinks on its own, and the
    /// lines are stacked with `row-gap` between them; `align-content` decides where the block of
    /// lines sits when the container is taller than its content.
    fn place_wrap(
        &mut self,
        fonts: &mut Fonts,
        inner: Rect,
        s: &Style,
        kids: &[NodeId],
        mut sizes: Vec<(f32, f32)>,
        scroll: f32,
    ) -> f32 {
        let main_avail = inner.w.max(0.0);
        let mains: Vec<f32> = kids
            .iter()
            .zip(&sizes)
            .map(|(&c, sz)| {
                let m = self.nodes[c].style.margin;
                sz.0 + m[1] + m[3]
            })
            .collect();
        let lines = break_lines(&mains, main_avail, s.gap);
        let mut heights: Vec<f32> = Vec::with_capacity(lines.len());
        for &(a, b) in &lines {
            let mut h = 0.0f32;
            for i in a..b {
                let m = self.nodes[kids[i]].style.margin;
                h = h.max(sizes[i].1 + m[0] + m[2]);
            }
            heights.push(h);
        }
        let total_h = heights.iter().sum::<f32>() + s.gap_cross * lines.len().saturating_sub(1) as f32;
        let cross_free = (inner.h - total_h).max(0.0);
        let (mut y, line_extra) = match s.align_content {
            Align::Center => (cross_free / 2.0, 0.0),
            Align::End => (cross_free, 0.0),
            Align::SpaceBetween if lines.len() > 1 => (0.0, cross_free / (lines.len() - 1) as f32),
            _ => (0.0, 0.0),
        };
        for (li, &(a, b)) in lines.iter().enumerate() {
            let line_h = heights[li];
            let n = b - a;
            let gaps = s.gap * n.saturating_sub(1) as f32;
            let used = gaps + mains[a..b].iter().sum::<f32>();
            let free = main_avail - used;
            if free > 0.0 {
                let grow_total: f32 = (a..b).map(|i| self.nodes[kids[i]].style.grow).sum();
                if grow_total > 0.0 {
                    for i in a..b {
                        let g = self.nodes[kids[i]].style.grow;
                        if g > 0.0 {
                            sizes[i].0 += free * g / grow_total;
                        }
                    }
                }
            } else if free < 0.0 {
                let shrinkable: Vec<usize> = (a..b)
                    .filter(|&i| {
                        let cs = &self.nodes[kids[i]].style;
                        cs.shrink > 0.0 && !matches!(cs.width, Dim::Px(_)) && (cs.grow > 0.0 || cs.clip)
                    })
                    .collect();
                let total: f32 = shrinkable.iter().map(|&i| sizes[i].0).sum();
                if total > 0.0 {
                    for &i in &shrinkable {
                        sizes[i].0 = (sizes[i].0 + free * sizes[i].0 / total).max(0.0);
                    }
                }
            }
            let used: f32 = gaps
                + (a..b)
                    .map(|i| {
                        let m = self.nodes[kids[i]].style.margin;
                        sizes[i].0 + m[1] + m[3]
                    })
                    .sum::<f32>();
            let leftover = (main_avail - used).max(0.0);
            let (mut cursor, between) = match s.justify {
                Align::Center => (leftover / 2.0, 0.0),
                Align::End => (leftover, 0.0),
                Align::SpaceBetween if n > 1 => (0.0, leftover / (n - 1) as f32),
                _ => (0.0, 0.0),
            };
            for i in a..b {
                let c = kids[i];
                let cs = self.nodes[c].style.clone();
                let m = cs.margin;
                let (cw, mut ch) = sizes[i];
                if self.nodes[c].el.text.is_some() {
                    ch = ch.max(self.height_at(fonts, c, cw));
                }
                let cross = (line_h - m[0] - m[2]).max(0.0);
                let dy = match s.align {
                    Align::Stretch if cs.height == Dim::Auto => {
                        ch = bounded(cross, cs.min_h, cs.max_h);
                        0.0
                    }
                    Align::Center => (cross - ch) / 2.0,
                    Align::End => cross - ch,
                    _ => 0.0,
                };
                let r = Rect::new(inner.x + cursor + m[3], inner.y + y + m[0] + dy - scroll, cw, ch);
                cursor += cw + m[1] + m[3] + s.gap + between;
                self.place(fonts, c, r);
            }
            y += line_h + s.gap_cross + line_extra;
        }
        // `content_h` is the height a *scrolling* container reports so its scrollbar knows how far
        // it can go. A row that does not scroll has no such thing: its own height is the answer,
        // the same way the single-line path treats a row.
        if s.scroll {
            total_h + s.padding[0] + s.padding[2]
        } else {
            inner.h
        }
    }

    fn content_box(&self, id: NodeId) -> Rect {
        let n = &self.nodes[id];
        let s = &n.style;
        Rect::new(
            n.rect.x + s.border[3] + s.padding[3],
            n.rect.y + s.border[0] + s.padding[0],
            n.rect.w - Self::h_extra(s),
            n.rect.h - Self::v_extra(s),
        )
    }

    pub fn paint(&self) -> Scene {
        let mut scene = Scene::default();
        self.paint_node(0, &mut scene, 1.0);
        scene
    }

    fn paint_node(&self, id: NodeId, scene: &mut Scene, opacity: f32) {
        let n = &self.nodes[id];
        let s = &n.style;
        if s.display_none || n.rect.w <= 0.0 || n.rect.h <= 0.0 {
            return;
        }
        let opacity = opacity * s.opacity;
        let fade = |c: Color| if opacity < 0.999 { c.with_alpha(opacity) } else { c };
        let r = n.rect;
        if s.background.a > 0 {
            scene.cmds.push(Cmd::Fill { rect: r, color: fade(s.background), radius: s.radius });
        }
        let edges = [
            (Rect::new(r.x, r.y, r.w, s.border[0]), 0),
            (Rect::new(r.x + r.w - s.border[1], r.y, s.border[1], r.h), 1),
            (Rect::new(r.x, r.y + r.h - s.border[2], r.w, s.border[2]), 2),
            (Rect::new(r.x, r.y, s.border[3], r.h), 3),
        ];
        for (rect, i) in edges {
            if s.border[i] > 0.0 && s.border_color[i].a > 0 {
                scene.cmds.push(Cmd::Fill { rect, color: fade(s.border_color[i]), radius: 0.0 });
            }
        }
        if s.clip {
            scene.cmds.push(Cmd::PushClip(r));
        }
        let cb = self.content_box(id);
        // Centering needs the measured width; the text was measured during layout and is cached,
        // so this is approximate for wrapped text and exact for single lines.
        let text_dx = match &n.el.text {
            Some(text) if s.center_text => {
                ((cb.w - text.chars().count() as f32 * s.text.size * 0.52) / 2.0).max(0.0)
            }
            _ => 0.0,
        };
        for (rect, color) in &n.overlays {
            scene.cmds.push(Cmd::Fill {
                rect: Rect::new(cb.x + text_dx + rect.x, cb.y + rect.y, rect.w, rect.h),
                color: fade(*color),
                radius: 2.0,
            });
        }
        if let Some(image) = &n.el.image {
            scene.cmds.push(Cmd::Image { rect: cb, image: image.clone() });
        }
        if let Some(text) = n.shown.as_ref().or(n.el.text.as_ref()) {
            let mut style = s.text.clone();
            style.color = fade(style.color);
            let spans = if n.shown.is_some() { Vec::new() } else { n.el.spans.clone() };
            scene.cmds.push(Cmd::Text {
                x: cb.x + text_dx,
                y: cb.y,
                width: s.text.wrap.then_some(cb.w),
                text: text.clone(),
                spans,
                style,
            });
        }
        for (rect, color) in &n.overlays_top {
            scene.cmds.push(Cmd::Fill {
                rect: Rect::new(cb.x + text_dx + rect.x, cb.y + rect.y, rect.w, rect.h),
                color: fade(*color),
                radius: 0.0,
            });
        }
        scene.cmds.extend(n.draw.iter().cloned());
        for &c in &n.children {
            // Skip children scrolled out of a clipping parent (cheap virtualisation at paint).
            let cr = self.nodes[c].rect;
            if s.clip && (cr.y > r.y + r.h || cr.y + cr.h < r.y) && !self.nodes[c].style.absolute {
                continue;
            }
            self.paint_node(c, scene, opacity);
        }
        if s.scroll && n.content_h > cb.h + s.padding[2] + 1.0 {
            // Scrollbar thumb. The bottom padding is room to breathe at the end of the list, not
            // content under the fold: a list that fits exactly would otherwise carry a bar for a
            // scroll of a few pixels that shows nothing.
            let track = r.h;
            let thumb = (track * cb.h / n.content_h).max(24.0);
            let max_scroll = n.content_h - cb.h;
            let y = r.y + (track - thumb) * (n.scroll_y / max_scroll).clamp(0.0, 1.0);
            scene.cmds.push(Cmd::Fill {
                rect: Rect::new(r.x + r.w - 7.0, y, 5.0, thumb),
                color: self.scrollbar_thumb.unwrap_or(Color { r: 128, g: 128, b: 128, a: 110 }),
                radius: 2.5,
            });
        }
        if s.clip {
            scene.cmds.push(Cmd::PopClip);
        }
    }

    /// Topmost node at a point.
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_node(0, x, y)
    }

    fn hit_node(&self, id: NodeId, x: f32, y: f32) -> Option<NodeId> {
        let n = &self.nodes[id];
        if n.style.display_none {
            return None;
        }
        let inside = n.rect.contains(x, y);
        if n.style.clip && !inside {
            return None;
        }
        for &c in n.children.iter().rev() {
            if let Some(h) = self.hit_node(c, x, y) {
                return Some(h);
            }
        }
        inside.then_some(id)
    }

    /// Nearest ancestor-or-self with an action.
    pub fn action_at(&self, x: f32, y: f32) -> Option<(NodeId, &str)> {
        let mut cur = self.hit(x, y);
        while let Some(id) = cur {
            if let Some(a) = &self.nodes[id].el.action {
                return Some((id, a));
            }
            cur = self.nodes[id].parent;
        }
        None
    }

    /// Nearest ancestor-or-self that scrolls.
    pub fn scroller_at(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut cur = self.hit(x, y);
        while let Some(id) = cur {
            if self.nodes[id].style.scroll {
                return Some(id);
            }
            cur = self.nodes[id].parent;
        }
        None
    }

    /// Scroll `id` to `y` after layout, moving its descendants to match.
    pub fn set_scroll(&mut self, id: NodeId, y: f32) {
        let dy = self.nodes[id].scroll_y - y;
        self.nodes[id].scroll_y = y;
        let mut stack = self.nodes[id].children.clone();
        while let Some(n) = stack.pop() {
            self.nodes[n].rect.y += dy;
            stack.extend_from_slice(&self.nodes[n].children);
        }
    }

    pub fn max_scroll(&self, id: NodeId) -> f32 {
        (self.nodes[id].content_h - self.content_box(id).h).max(0.0)
    }

    pub fn content_rect(&self, id: NodeId) -> Rect {
        self.content_box(id)
    }
}
