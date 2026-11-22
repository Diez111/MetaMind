//! Pieces the diagram kinds share: a drawing surface that knows its own box, the layered
//! placement of a graph of boxes, and the way the links between them are drawn.

use crate::{Diagram, Error, Fill, Item, Measure};

/// Room between two boxes that sit side by side in a layer, and between two layers.
const GAP: f32 = 28.0;
const LAYER_GAP: f32 = 58.0;

/// How far one line of text sits below the one before it.
pub(crate) fn lead(size: f32) -> f32 {
    size * 1.35
}

/// Which way the layers of a layered layout advance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Dir {
    Down,
    Right,
}

impl Dir {
    /// The direction named by a `direction` line, if it is one. `default` is what mermaid uses
    /// when the source says nothing.
    pub fn parse(text: &str, default: Dir) -> Result<Dir, Error> {
        match text.trim().to_ascii_uppercase().as_str() {
            "" => Ok(default),
            "TB" | "TD" | "BT" => Ok(Dir::Down),
            "LR" | "RL" => Ok(Dir::Right),
            other => Err(Error::Syntax(format!("dirección desconocida `{other}`"))),
        }
    }
}

/// What is painted at one end of a link.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Head {
    None,
    /// Filled triangle: an association or a message.
    Arrow,
    /// The same triangle, drawn hollow: a dependency.
    Open,
    /// Inheritance.
    Triangle,
    /// Filled diamond: composition.
    Diamond,
    /// Hollow diamond: aggregation.
    DiamondOpen,
}

/// A link between two boxes of a laid out graph.
#[derive(Clone, Debug)]
pub(crate) struct Link {
    pub from: usize,
    pub to: usize,
    pub label: String,
    /// Written next to the `from` end, then the `to` one (cardinalities, multiplicity).
    pub from_label: String,
    pub to_label: String,
    pub head_from: Head,
    pub head_to: Head,
    pub dashed: bool,
    pub thick: bool,
    /// Only keeps the two boxes a layer apart: nothing is painted.
    pub hidden: bool,
}

impl Link {
    pub fn new(from: usize, to: usize) -> Self {
        Link {
            from,
            to,
            label: String::new(),
            from_label: String::new(),
            to_label: String::new(),
            head_from: Head::None,
            head_to: Head::None,
            dashed: false,
            thick: false,
            hidden: false,
        }
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }
}

/// A drawing in progress: its items plus the box that holds them, so that the diagram can be
/// sized to its content and moved so that it starts at the margin.
pub(crate) struct Draw {
    items: Vec<Item>,
    lo: (f32, f32),
    hi: (f32, f32),
    /// The boxes the labels already took, in diagram coordinates. A label is text nobody can
    /// read if another one lands on it, so every new one looks for a free spot first.
    labels: Vec<(f32, f32, f32, f32)>,
}

/// A finished drawing whose top left corner is `(0, 0)`.
pub(crate) struct Block {
    pub w: f32,
    pub h: f32,
    pub items: Vec<Item>,
}

impl Default for Draw {
    fn default() -> Self {
        Draw::new()
    }
}

impl Draw {
    pub fn new() -> Self {
        Draw { items: Vec::new(), lo: (f32::MAX, f32::MAX), hi: (f32::MIN, f32::MIN), labels: Vec::new() }
    }

    pub fn grow(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.lo = (self.lo.0.min(x0), self.lo.1.min(y0));
        self.hi = (self.hi.0.max(x1), self.hi.1.max(y1));
    }

    /// A box with its outline.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, fill: Fill) {
        self.grow(x, y, x + w, y + h);
        self.items.push(Item::Rect { x, y, w, h, radius, fill, stroke: true });
    }

    /// A strip of colour with no outline, for the band behind a section of a diagram.
    pub fn band(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, fill: Fill) {
        self.grow(x, y, x + w, y + h);
        self.items.push(Item::Rect { x, y, w, h, radius, fill, stroke: false });
    }

    pub fn line(&mut self, pts: Vec<(f32, f32)>, dashed: bool, width: f32) {
        for p in &pts {
            self.grow(p.0, p.1, p.0, p.1);
        }
        self.items.push(Item::Line { pts, dashed, width });
    }

    pub fn arrow(&mut self, x: f32, y: f32, angle: f32, open: bool) {
        self.grow(x - 10.0, y - 10.0, x + 10.0, y + 10.0);
        self.items.push(Item::Arrow { x, y, angle, open });
    }

    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, fill: Fill) {
        self.grow(cx - rx, cy - ry, cx + rx, cy + ry);
        self.items.push(Item::Ellipse { cx, cy, rx, ry, fill });
    }

    pub fn diamond(&mut self, cx: f32, cy: f32, w: f32, h: f32, fill: Fill) {
        self.grow(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
        self.items.push(Item::Diamond { cx, cy, w, h, fill });
    }

    pub fn sector(&mut self, cx: f32, cy: f32, r: f32, from: f32, to: f32, fill: Fill) {
        self.grow(cx - r, cy - r, cx + r, cy + r);
        self.items.push(Item::Sector { cx, cy, r, from, to, fill });
    }

    /// Text centered on `(x, y)`; returns the width it took.
    pub fn text(&mut self, m: &mut dyn Measure, x: f32, y: f32, text: &str, bold: bool, size: f32) -> f32 {
        let w = m.width(text, size, bold);
        let lh = size * 1.3;
        self.grow(x - w / 2.0, y - lh / 2.0, x + w / 2.0, y + lh / 2.0);
        self.items.push(Item::Text { x, y, text: text.to_string(), bold, size });
        w
    }

    /// Text whose left edge sits at `x`.
    pub fn text_left(
        &mut self,
        m: &mut dyn Measure,
        x: f32,
        y: f32,
        text: &str,
        bold: bool,
        size: f32,
    ) -> f32 {
        let w = m.width(text, size, bold);
        self.text(m, x + w / 2.0, y, text, bold, size);
        w
    }

    /// Text whose right edge sits at `x`.
    pub fn text_right(
        &mut self,
        m: &mut dyn Measure,
        x: f32,
        y: f32,
        text: &str,
        bold: bool,
        size: f32,
    ) -> f32 {
        let w = m.width(text, size, bold);
        self.text(m, x - w / 2.0, y, text, bold, size);
        w
    }

    /// A stack of lines centered on `x`, the first one centered on `top`.
    pub fn lines(&mut self, m: &mut dyn Measure, x: f32, top: f32, lines: &[String], bold: bool, size: f32) {
        for (i, l) in lines.iter().enumerate() {
            let y = top + size * 0.65 + i as f32 * lead(size);
            if !l.is_empty() {
                self.text(m, x, y, l, bold, size);
            }
        }
    }

    /// Copies a block into this drawing with its top left corner at `(x, y)`.
    pub fn place(&mut self, x: f32, y: f32, b: &Block) {
        self.grow(x, y, x + b.w, y + b.h);
        for it in &b.items {
            let mut it = it.clone();
            shift(&mut it, x, y);
            self.items.push(it);
        }
    }

    pub fn finish(self, margin: f32) -> Diagram {
        let b = self.into_block(margin);
        Diagram { width: b.w, height: b.h, items: b.items }
    }

    /// Ends the drawing, normalizing it so that its content starts at the margin and `(0, 0)` of
    /// the block is its top left corner.
    pub fn into_block(mut self, margin: f32) -> Block {
        if self.items.is_empty() {
            return Block { w: 0.0, h: 0.0, items: Vec::new() };
        }
        let (dx, dy) = (margin - self.lo.0, margin - self.lo.1);
        for it in &mut self.items {
            shift(it, dx, dy);
        }
        Block {
            w: self.hi.0 - self.lo.0 + 2.0 * margin,
            h: self.hi.1 - self.lo.1 + 2.0 * margin,
            items: self.items,
        }
    }
}

/// Moves an item by `(dx, dy)`.
pub(crate) fn shift(it: &mut Item, dx: f32, dy: f32) {
    match it {
        Item::Rect { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::Diamond { cx, cy, .. } | Item::Ellipse { cx, cy, .. } | Item::Sector { cx, cy, .. } => {
            *cx += dx;
            *cy += dy;
        }
        Item::Line { pts, .. } => {
            for p in pts {
                p.0 += dx;
                p.1 += dy;
            }
        }
        Item::Arrow { x, y, .. } | Item::Text { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
    }
}

/// The point where a line from the center of a box towards `toward` leaves the box.
pub(crate) fn border(center: (f32, f32), size: (f32, f32), toward: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (toward.0 - center.0, toward.1 - center.1);
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return center;
    }
    let (hw, hh) = (size.0 / 2.0, size.1 / 2.0);
    let t = (hw / dx.abs().max(1e-6)).min(hh / dy.abs().max(1e-6));
    (center.0 + dx * t.min(1.0), center.1 + dy * t.min(1.0))
}

/// The boxes of a laid out graph: their centers and the layer each one landed in.
pub(crate) struct Layout {
    pub centers: Vec<(f32, f32)>,
    pub ranks: Vec<usize>,
}

/// Places `sizes.len()` boxes in layers, the links deciding which box goes below which: every
/// link is read from `from` to `to`, so the first one ends up in an earlier layer.
pub(crate) fn place(sizes: &[(f32, f32)], links: &[Link], dir: Dir) -> Layout {
    let n = sizes.len();
    if n == 0 {
        return Layout { centers: Vec::new(), ranks: Vec::new() };
    }
    let edges: Vec<(usize, usize)> =
        links.iter().filter(|l| l.from != l.to).map(|l| (l.from, l.to)).collect();

    // The edges that close a cycle are laid out as if they did not exist, or ranking would not
    // terminate. They are still drawn, routed round the side.
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, &(a, _)) in edges.iter().enumerate() {
        out[a].push(k);
    }
    let mut color = vec![0u8; n];
    let mut dropped = vec![false; edges.len()];
    for root in 0..n {
        if color[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        color[root] = 1;
        while let Some(&(v, k)) = stack.last() {
            if k < out[v].len() {
                let ei = out[v][k];
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                let to = edges[ei].1;
                match color[to] {
                    0 => {
                        color[to] = 1;
                        stack.push((to, 0));
                    }
                    1 => dropped[ei] = true,
                    _ => {}
                }
            } else {
                color[v] = 2;
                stack.pop();
            }
        }
    }

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for (k, &(a, b)) in edges.iter().enumerate() {
        if dropped[k] {
            continue;
        }
        succs[a].push(b);
        preds[b].push(a);
        indeg[b] += 1;
    }

    // Longest path from the roots.
    let mut ranks = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let v = queue[head];
        head += 1;
        for &w in &succs[v] {
            ranks[w] = ranks[w].max(ranks[v] + 1);
            indeg[w] -= 1;
            if indeg[w] == 0 {
                queue.push(w);
            }
        }
    }
    let depth = ranks.iter().copied().max().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for v in 0..n {
        layers[ranks[v]].push(v);
    }

    // Order each layer by the average position of its neighbours, sweeping down and up a few
    // times so that the links come out short.
    let mut pos = vec![0.0f32; n];
    for layer in &layers {
        for (i, &v) in layer.iter().enumerate() {
            pos[v] = i as f32;
        }
    }
    for sweep in 0..6 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> =
            if down { (1..depth).collect() } else { (0..depth.saturating_sub(1)).rev().collect() };
        for l in order {
            let nb = if down { &preds } else { &succs };
            let mut keyed: Vec<(f32, usize)> = layers[l]
                .iter()
                .map(|&v| {
                    let k = if nb[v].is_empty() {
                        pos[v]
                    } else {
                        nb[v].iter().map(|&x| pos[x]).sum::<f32>() / nb[v].len() as f32
                    };
                    (k, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[l] = keyed.into_iter().map(|(_, v)| v).collect();
            for (i, &v) in layers[l].iter().enumerate() {
                pos[v] = i as f32;
            }
        }
    }

    let cross = if dir == Dir::Down { 0 } else { 1 };
    let size = |v: usize, axis: usize| if axis == 0 { sizes[v].0 } else { sizes[v].1 };

    // Coordinates across the layers: side by side, each layer centered.
    let mut c = vec![0.0f32; n];
    for layer in &layers {
        let mut x = 0.0;
        for &v in layer {
            c[v] = x + size(v, cross) / 2.0;
            x += size(v, cross) + GAP;
        }
        let width = x - GAP;
        for &v in layer {
            c[v] -= width / 2.0;
        }
    }
    for pass in 0..4 {
        let down = pass % 2 == 0;
        let order: Vec<usize> =
            if down { (1..depth).collect() } else { (0..depth.saturating_sub(1)).rev().collect() };
        for l in order {
            let nb = if down { &preds } else { &succs };
            let layer = layers[l].clone();
            let want: Vec<f32> = layer
                .iter()
                .map(|&v| {
                    if nb[v].is_empty() {
                        c[v]
                    } else {
                        nb[v].iter().map(|&x| c[x]).sum::<f32>() / nb[v].len() as f32
                    }
                })
                .collect();
            let mut placed: Vec<f32> = Vec::with_capacity(layer.len());
            for (i, &v) in layer.iter().enumerate() {
                let mut x = want[i];
                if i > 0 {
                    x = x.max(placed[i - 1] + (size(layer[i - 1], cross) + size(v, cross)) / 2.0 + GAP);
                }
                placed.push(x);
            }
            let recenter = if placed.is_empty() {
                0.0
            } else {
                placed.iter().zip(&want).map(|(p, w)| w - p).sum::<f32>() / placed.len() as f32
            };
            for (i, &v) in layer.iter().enumerate() {
                c[v] = placed[i] + recenter;
            }
        }
    }

    // Coordinates along the layers: one after the other, each layer as tall as its tallest box.
    let mut f = vec![0.0f32; n];
    let mut at = 0.0;
    for layer in &layers {
        let t = layer.iter().map(|&v| size(v, 1 - cross)).fold(0.0, f32::max);
        for &v in layer {
            f[v] = at + t / 2.0;
        }
        at += t + LAYER_GAP;
    }
    let centers = (0..n).map(|v| if dir == Dir::Down { (c[v], f[v]) } else { (f[v], c[v]) }).collect();
    Layout { centers, ranks }
}

/// Paints the links of a laid out graph. A link that only goes down one layer is a straight line
/// between the two boxes; one that goes back up, or that jumps over a layer, is routed round the
/// side so that it does not cross the boxes in between.
pub(crate) fn draw_links(
    d: &mut Draw,
    m: &mut dyn Measure,
    sizes: &[(f32, f32)],
    lay: &Layout,
    links: &[Link],
    dir: Dir,
) {
    let widest = (0..sizes.len())
        .map(|v| {
            let (cx, cy) = lay.centers[v];
            if dir == Dir::Down {
                cx + sizes[v].0 / 2.0
            } else {
                cy + sizes[v].1 / 2.0
            }
        })
        .fold(f32::MIN, f32::max);
    let mut lane = 0usize;
    for l in links {
        if l.hidden || l.from >= sizes.len() || l.to >= sizes.len() {
            continue;
        }
        let width = if l.thick { 3.0 } else { 1.5 };
        if l.from == l.to {
            let (a, sz) = (lay.centers[l.from], sizes[l.from]);
            let pts = if dir == Dir::Down {
                let (x, y) = (a.0 + sz.0 / 2.0, a.1);
                vec![(x, y - 9.0), (x + 26.0, y - 15.0), (x + 26.0, y + 15.0), (x, y + 9.0)]
            } else {
                let (x, y) = (a.0, a.1 + sz.1 / 2.0);
                vec![(x - 9.0, y), (x - 15.0, y + 26.0), (x + 15.0, y + 26.0), (x + 9.0, y)]
            };
            let n = pts.len();
            d.line(pts.clone(), l.dashed, width);
            let back = angle(pts[n - 2], pts[n - 1]) + std::f32::consts::PI;
            head(d, l.head_to, pts[n - 1], angle(pts[n - 2], pts[n - 1]));
            head(d, l.head_from, pts[0], back);
            mid_label(d, m, &pts, &l.label);
            continue;
        }
        let forward = lay.ranks[l.to] > lay.ranks[l.from] && lay.ranks[l.to] - lay.ranks[l.from] == 1;
        let (a, b) = (lay.centers[l.from], lay.centers[l.to]);
        let pts = if forward {
            vec![border(a, sizes[l.from], b), border(b, sizes[l.to], a)]
        } else {
            lane += 1;
            side_route(a, sizes[l.from], b, sizes[l.to], widest, lane, dir)
        };
        d.line(pts.clone(), l.dashed, width);
        let n = pts.len();
        head(d, l.head_to, pts[n - 1], angle(pts[n - 2], pts[n - 1]));
        head(d, l.head_from, pts[0], angle(pts[1], pts[0]));
        cardinal(d, m, pts[0], angle(pts[1], pts[0]), &l.from_label);
        cardinal(d, m, pts[n - 1], angle(pts[n - 2], pts[n - 1]), &l.to_label);
        mid_label(d, m, &pts, &l.label);
    }
}

fn angle(a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.1 - a.1).atan2(b.0 - a.0)
}

/// The way round the outside: out of the `from` box, along a lane clear of every box, and back
/// into the `to` one.
fn side_route(
    a: (f32, f32),
    sa: (f32, f32),
    b: (f32, f32),
    sb: (f32, f32),
    widest: f32,
    lane: usize,
    dir: Dir,
) -> Vec<(f32, f32)> {
    let off = 18.0 + 14.0 * lane as f32;
    if dir == Dir::Down {
        let lane_x = widest + off;
        vec![(a.0 + sa.0 / 2.0, a.1), (lane_x, a.1), (lane_x, b.1), (b.0 + sb.0 / 2.0, b.1)]
    } else {
        let lane_y = widest + off;
        vec![(a.0, a.1 + sa.1 / 2.0), (a.0, lane_y), (b.0, lane_y), (b.0, b.1 + sb.1 / 2.0)]
    }
}

fn head(d: &mut Draw, head: Head, at: (f32, f32), angle: f32) {
    match head {
        Head::None => {}
        Head::Arrow => d.arrow(at.0, at.1, angle, false),
        Head::Open => d.arrow(at.0, at.1, angle, true),
        Head::Triangle => d.arrow(at.0, at.1, angle, true),
        Head::Diamond | Head::DiamondOpen => {
            let (dx, dy) = (angle.cos() * 9.0, angle.sin() * 9.0);
            let fill = if head == Head::Diamond { Fill::Node } else { Fill::None };
            d.diamond(at.0 - dx, at.1 - dy, 19.0, 11.0, fill);
        }
    }
}

/// The point halfway along a polyline.
fn mid(pts: &[(f32, f32)]) -> (f32, f32) {
    let mut lens = Vec::new();
    let mut total = 0.0;
    for w in pts.windows(2) {
        let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        lens.push(l);
        total += l;
    }
    let mut walk = total / 2.0;
    for (i, l) in lens.iter().enumerate() {
        if walk <= *l || i + 1 == lens.len() {
            let t = if *l > 0.0 { walk / l } else { 0.0 };
            let (a, b) = (pts[i], pts[i + 1]);
            return (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        }
        walk -= l;
    }
    pts[0]
}

/// Writes a link's label over the middle of its line, on the background so that the line does
/// not run through the text.
pub(crate) fn mid_label(d: &mut Draw, m: &mut dyn Measure, pts: &[(f32, f32)], text: &str) {
    if text.is_empty() {
        return;
    }
    let size = 12.0;
    let w = m.width(text, size, false) + 10.0;
    let h = size * 1.5;
    let total = length(pts);
    // The middle of the line, then along it and off to the sides: a fan of links leaving one node
    // shares its middle, and that is exactly where two labels used to land on top of each other.
    let mut best: Option<((f32, f32), f32)> = None;
    for dy in [0.0, -1.0, 1.0, -2.0, 2.0] {
        for frac in [0.5, 0.42, 0.58, 0.34, 0.66, 0.26, 0.74] {
            let (x, y) = at(pts, total * frac);
            let y = y + dy * (h + 2.0);
            let over = d.overlap(x - w / 2.0, y - h / 2.0, w, h);
            if over <= 0.0 {
                best = Some(((x, y), over));
                break;
            }
            if best.is_none_or(|(_, b)| over < b) {
                best = Some(((x, y), over));
            }
        }
        if best.is_some_and(|(_, over)| over <= 0.0) {
            break;
        }
    }
    let (x, y) = best.map_or_else(|| mid(pts), |(p, _)| p);
    d.labels.push((x - w / 2.0, y - h / 2.0, w, h));
    d.band(x - w / 2.0, y - size * 0.75, w, size * 1.5, 3.0, Fill::Background);
    d.text(m, x, y, text, false, size);
}

impl Draw {
    /// How much of a box lands on a label already placed, in square units.
    fn overlap(&self, x: f32, y: f32, w: f32, h: f32) -> f32 {
        let mut sum = 0.0;
        for &(lx, ly, lw, lh) in &self.labels {
            let dx = (x + w).min(lx + lw) - x.max(lx);
            let dy = (y + h).min(ly + lh) - y.max(ly);
            if dx > 0.0 && dy > 0.0 {
                sum += dx * dy;
            }
        }
        sum
    }
}

/// The length of a polyline.
fn length(pts: &[(f32, f32)]) -> f32 {
    pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum()
}

/// The point `walk` units along a polyline from its start.
fn at(pts: &[(f32, f32)], walk: f32) -> (f32, f32) {
    let mut left = walk.max(0.0);
    for w in pts.windows(2) {
        let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        if left <= l && l > 0.0 {
            let t = left / l;
            return (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
        }
        left -= l;
    }
    pts.last().copied().unwrap_or((0.0, 0.0))
}

/// Writes the text of one end of a link beside its endpoint, `angle` pointing inwards.
pub(crate) fn cardinal(d: &mut Draw, m: &mut dyn Measure, at: (f32, f32), angle: f32, text: &str) {
    if text.is_empty() {
        return;
    }
    let size = 11.0;
    let w = m.width(text, size, false);
    let (dx, dy) = (angle.cos(), angle.sin());
    let x = at.0 + dx * (10.0 + w / 2.0) - dy * 10.0;
    let y = at.1 + dy * 10.0 + dx * 10.0;
    d.text(m, x, y, text, false, size);
}

/// Breaks `text` into the lines a box of at most `max` width draws it in.
pub(crate) fn wrap(m: &mut dyn Measure, text: &str, size: f32, max: f32) -> Vec<String> {
    let mut out = Vec::new();
    for hard in text.split('\n') {
        let mut line = String::new();
        for word in hard.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && m.width(&candidate, size, false) > max {
                out.push(std::mem::take(&mut line));
            }
            if line.is_empty() {
                line.push_str(word);
            } else {
                line.push(' ');
                line.push_str(word);
            }
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Cleans a label written in mermaid: quoted, with HTML line breaks, with `\n`.
pub(crate) fn clean(text: &str) -> String {
    let t = text.trim();
    let t = if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') { &t[1..t.len() - 1] } else { t };
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while let Some(i) = rest.find("<br") {
        let after = &rest[i..];
        let end = after.find('>').map(|e| e + 1).unwrap_or(after.len());
        out.push_str(&rest[..i]);
        out.push('\n');
        rest = &after[end..];
    }
    out.push_str(rest);
    out.replace("\\n", "\n").trim().to_string()
}

/// The text of a node written as `id[shape]`, `id(shaped)`, `id((shaped))`, with a `::icon(..)`
/// or a `:::class` after it, or as it is.
pub(crate) fn node_text(text: &str) -> String {
    let mut t = text.trim();
    for marker in ["::icon(", ":::"] {
        if let Some(i) = t.find(marker) {
            t = t[..i].trim();
        }
    }
    // The shape the text is written in: `id[...]`, `id(...)`, `id((...))`, `id{{...}}` and the
    // ones that open with a closing bracket, `id)...(`. The id in front is dropped.
    if let Some(i) = t.find(['(', '[', '{', ')', '}']) {
        let open = t.as_bytes()[i] as char;
        let close = match open {
            '[' => ']',
            '(' => ')',
            '{' => '}',
            ')' => '(',
            _ => '{',
        };
        let inner = &t[i..];
        let opens = inner.chars().take_while(|c| *c == open).count();
        let mut s = inner.trim_start_matches(open);
        for _ in 0..opens {
            s = s.trim_end_matches(close);
        }
        return clean(s);
    }
    clean(t)
}
