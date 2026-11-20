use crate::color::Color;
use crate::parse::{Align, Face, Node};
use crate::syms::Kind;

pub trait Measure {
    fn width(&mut self, text: &str, size: f32, face: Face) -> f32;
}

/// Font-independent widths, good enough to size boxes before real fonts are available.
pub struct Approx;

impl Measure for Approx {
    fn width(&mut self, text: &str, size: f32, face: Face) -> f32 {
        let mut w = 0.0;
        for c in text.chars() {
            w += match c {
                ' ' => 0.28,
                'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '′' | '|' => 0.28,
                'f' | 't' | 'r' | '(' | ')' | '[' | ']' => 0.36,
                'm' | 'w' => 0.85,
                'M' | 'W' => 0.9,
                '0'..='9' => 0.58,
                'A'..='Z' => 0.7,
                'a'..='z' => 0.55,
                c if (c as u32) > 0x2000 => 0.8,
                _ => 0.6,
            };
        }
        w * size * if face == Face::Bold { 1.08 } else { 1.0 }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub face: Face,
    /// Set by `\textcolor`/`\color`; `None` means the painter's own text colour.
    pub color: Option<Color>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Seg {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub width: f32,
    /// Set by `\textcolor`/`\color`; `None` means the painter's own text colour.
    pub color: Option<Color>,
}

/// Spacing around and inside a laid-out formula.
///
/// `padding_x`/`padding_y` are the breathing room the box adds around its own glyphs: the returned
/// `Layout` is the padded box, so painters can draw it edge to edge. `run_gap` is extra space in
/// pixels between the top-level items of the formula (between a fraction and the `+` after it, not
/// between the glyphs of one item); `0.0` keeps the textbook spacing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Metrics {
    pub padding_x: f32,
    pub padding_y: f32,
    pub run_gap: f32,
}

impl Metrics {
    /// No padding, no extra gap: the box hugs the ink.
    pub const ZERO: Metrics = Metrics { padding_x: 0.0, padding_y: 0.0, run_gap: 0.0 };
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics { padding_x: 6.0, padding_y: 5.0, run_gap: 0.0 }
    }
}

/// Positioned output. `y` of a glyph is its baseline measured from the top of the box.
#[derive(Clone, Default, Debug)]
pub struct Layout {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub glyphs: Vec<Glyph>,
    pub segs: Vec<Seg>,
}

impl Layout {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

#[derive(Clone, Default)]
struct B {
    w: f32,
    a: f32,
    d: f32,
    g: Vec<Glyph>,
    s: Vec<Seg>,
}

impl B {
    fn put(&mut self, o: B, dx: f32, dy: f32) {
        for mut g in o.g {
            g.x += dx;
            g.y += dy;
            self.g.push(g);
        }
        for mut s in o.s {
            s.x0 += dx;
            s.x1 += dx;
            s.y0 += dy;
            s.y1 += dy;
            self.s.push(s);
        }
    }
    fn seg(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32) {
        self.s.push(Seg { x0, y0, x1, y1, width, color: None });
    }
}

#[derive(Clone, Copy)]
struct St {
    size: f32,
    display: bool,
    script: bool,
}

const AXIS: f32 = 0.25;

pub struct Ctx<'a> {
    pub m: &'a mut dyn Measure,
    metrics: Metrics,
    /// Nesting level of the node lists being laid out; the metrics only touch level 0.
    depth: usize,
}

fn extents(text: &str) -> (f32, f32) {
    let (mut a, mut d) = (0.0f32, 0.0f32);
    for c in text.chars() {
        let (ca, cd) = match c {
            '0'..='9' | 'A'..='Z' | 'b' | 'd' | 'f' | 'h' | 'k' | 'l' | 't' | 'i' => (0.72, 0.0),
            'g' | 'j' | 'p' | 'q' | 'y' => (0.46, 0.22),
            'a'..='z' => (0.46, 0.0),
            '(' | ')' | '[' | ']' | '{' | '}' | '|' => (0.76, 0.24),
            '.' | ',' | ';' => (0.1, 0.1),
            '+' | '−' | '=' | '<' | '>' | '±' | '×' | '⋅' | '∈' | '≤' | '≥' | '≠' | '→' | '←' | '≈' | '∼' => {
                (0.55, 0.05)
            }
            'β' | 'ζ' | 'ξ' | 'δ' | 'θ' | 'λ' | 'ϑ' | 'φ' | 'ϕ' | 'ψ' | 'ℓ' => (0.72, 0.2),
            'γ' | 'η' | 'μ' | 'ρ' | 'ς' | 'χ' | 'ϱ' => (0.46, 0.22),
            'α' | 'ε' | 'ϵ' | 'ι' | 'κ' | 'ν' | 'π' | 'ϖ' | 'σ' | 'τ' | 'υ' | 'ω' => (0.46, 0.0),
            '′' => (0.75, -0.4),
            _ => (0.72, 0.1),
        };
        a = a.max(ca);
        d = d.max(cd);
    }
    (a, d)
}

fn kind_of(n: &Node) -> Kind {
    match n {
        Node::Atom { kind, .. } => *kind,
        Node::Script { base, .. } => kind_of(base),
        Node::Group(g) => match g.first() {
            Some(Node::Atom { kind: Kind::Op, .. }) if g.len() == 2 => Kind::Op,
            Some(first) if g.len() == 1 => kind_of(first),
            _ => Kind::Ord,
        },
        // A coloured run keeps the spacing class of what it colours.
        Node::Color { body, .. } => match body.as_slice() {
            [only] => kind_of(only),
            _ => Kind::Ord,
        },
        _ => Kind::Ord,
    }
}

fn brace_of(n: &Node) -> Option<(bool, &[Node])> {
    match n {
        Node::Brace { over, body } => Some((*over, body)),
        Node::Group(g) if g.len() == 1 => brace_of(&g[0]),
        _ => None,
    }
}

fn gap(l: Kind, r: Kind) -> f32 {
    use Kind::*;
    match (l, r) {
        (Ord | Close, Op) | (Op, Ord | Op) => 0.167,
        (Ord | Close, Bin) | (Bin, Ord | Op | Open) => 0.222,
        (Ord | Close | Op, Rel) | (Rel, Ord | Op | Open) => 0.278,
        (Punct, Close | Punct) => 0.0,
        (Punct, _) => 0.167,
        _ => 0.0,
    }
}

const BIG: &[&str] = &["∑", "∏", "∐", "⋃", "⋂", "⨁", "⋀", "⋁", "∫", "∬", "∭", "∮"];
const LIMIT_FUNCS: &[&str] = &["lim", "max", "min", "sup", "inf", "liminf", "limsup", "det", "gcd", "Pr"];

impl Ctx<'_> {
    fn text(&mut self, t: &str, size: f32, face: Face) -> B {
        let (a, d) = extents(t);
        let w = self.m.width(t, size, face);
        let mut b = B { w, a: a * size, d: d * size, ..B::default() };
        b.g.push(Glyph { x: 0.0, y: 0.0, text: t.to_string(), size, face, color: None });
        b
    }

    fn list(&mut self, nodes: &[Node], st: St) -> B {
        let top = self.depth == 0;
        self.depth += 1;
        let b = self.list_inner(nodes, st, top);
        self.depth -= 1;
        b
    }

    fn list_inner(&mut self, nodes: &[Node], st: St, top: bool) -> B {
        let mut out = B::default();
        let mut prev: Option<Kind> = None;
        let mut x = 0.0;
        for (i, n) in nodes.iter().enumerate() {
            if matches!(n, Node::Space(_)) {
                if let Node::Space(f) = n {
                    x += f * st.size;
                }
                continue;
            }
            let mut k = kind_of(n);
            let next = nodes[i + 1..].iter().find(|n| !matches!(n, Node::Space(_))).map(kind_of);
            if k == Kind::Bin
                && (matches!(prev, None | Some(Kind::Bin | Kind::Rel | Kind::Open | Kind::Punct | Kind::Op))
                    || matches!(next, None | Some(Kind::Rel | Kind::Close | Kind::Punct)))
            {
                k = Kind::Ord;
            }
            if let (Some(p), false) = (prev, st.script) {
                x += gap(p, k) * st.size;
            }
            if top && prev.is_some() {
                x += self.metrics.run_gap.max(0.0);
            }
            let b = self.node(n, st);
            out.a = out.a.max(b.a);
            out.d = out.d.max(b.d);
            let w = b.w;
            out.put(b, x, 0.0);
            x += w;
            prev = Some(k);
        }
        out.w = x;
        out
    }

    fn node(&mut self, n: &Node, st: St) -> B {
        let s = st.size;
        match n {
            Node::Atom { text, kind, face } => {
                if *kind == Kind::Op && BIG.contains(&text.as_str()) {
                    return self.big(text, st);
                }
                self.text(text, s, *face)
            }
            Node::Text(t) => self.text(t, s, Face::Roman),
            Node::Space(f) => B { w: f * s, ..B::default() },
            Node::Group(g) => self.list(g, st),
            Node::Unknown(t) => self.text(t, s, Face::Roman),
            Node::Script { base, sup, sub } => self.script(base, sup.as_deref(), sub.as_deref(), st),
            Node::Frac { num, den, rule, forced } => {
                let display = forced.unwrap_or(st.display);
                self.frac(num, den, *rule, display, st)
            }
            Node::Sqrt { body, index } => self.sqrt(body, index.as_deref(), st),
            Node::Delim { open, close, body } => {
                let b = self.list(body, st);
                self.fenced(b, open, close, st)
            }
            Node::Matrix { rows, open, close, align } => {
                let m = self.matrix(rows, *align, open, st);
                self.fenced_h(m, open, close, st)
            }
            Node::Accent { kind, body } => self.accent(kind, body, st),
            Node::Line { over, body } => {
                let mut b = self.list(body, st);
                let t = 0.05 * s;
                if *over {
                    let y = -(b.a + 0.12 * s);
                    b.seg(0.0, y, b.w, y, t);
                    b.a += 0.16 * s;
                } else {
                    let y = b.d + 0.12 * s;
                    b.seg(0.0, y, b.w, y, t);
                    b.d += 0.16 * s;
                }
                b
            }
            Node::Brace { over, body } => {
                let b = self.list(body, st);
                self.braced(*over, b, None, st)
            }
            Node::Stack { over, small, base } => self.stack(*over, small, base, st),
            Node::Color { color, body } => {
                let mut b = self.list(body, st);
                // Nested colours win; this only fills what is still unstyled.
                for g in &mut b.g {
                    g.color = g.color.or(Some(*color));
                }
                for sg in &mut b.s {
                    sg.color = sg.color.or(Some(*color));
                }
                b
            }
        }
    }

    fn big(&mut self, sym: &str, st: St) -> B {
        let integral = matches!(sym, "∫" | "∬" | "∭" | "∮");
        let f = if st.display {
            if integral {
                1.7
            } else {
                1.45
            }
        } else {
            1.0
        };
        let gs = st.size * f;
        let w = self.m.width(sym, gs, Face::Roman);
        let center = AXIS * st.size;
        let half = if integral { 0.62 * gs } else { 0.5 * gs };
        let mut b = B { w, a: center + half, d: half - center, ..B::default() };
        let base = if integral { 0.25 * gs } else { 0.32 * gs } - center;
        b.g.push(Glyph { x: 0.0, y: base, text: sym.to_string(), size: gs, face: Face::Roman, color: None });
        b
    }

    fn sub_st(&self, st: St) -> St {
        St { size: (st.size * 0.7).max(5.0), display: false, script: true }
    }

    fn script(&mut self, base: &Node, sup: Option<&[Node]>, sub: Option<&[Node]>, st: St) -> B {
        let s = st.size;
        // `\underbrace{x}_{n}`: the annotation belongs under the brace, not beside it.
        if let Some((over, body)) = brace_of(base) {
            let (label, rest) = if over { (sup, sub) } else { (sub, sup) };
            let b = self.list(body, st);
            let out = self.braced(over, b, label, st);
            let side = rest.map(|n| self.list(n, self.sub_st(st)));
            return match (side, over) {
                (None, _) => out,
                (Some(o), true) => self.side_scripts(out, None, Some(o), st),
                (Some(o), false) => self.side_scripts(out, Some(o), None, st),
            };
        }
        let limits = match base {
            Node::Atom { text, kind: Kind::Op, .. } => {
                st.display && BIG.contains(&text.as_str()) && !matches!(text.as_str(), "∫" | "∬" | "∭" | "∮")
            }
            Node::Group(g) => {
                st.display
                    && matches!(g.first(), Some(Node::Atom { kind: Kind::Op, text, .. }) if LIMIT_FUNCS.contains(&text.as_str()))
            }
            _ => false,
        };
        let bb = self.node(base, st);
        let sst = self.sub_st(st);
        let up_b = sup.map(|n| self.list(n, sst));
        let dn_b = sub.map(|n| self.list(n, sst));
        if limits {
            return self.stacked(bb, up_b, dn_b, s, 0.12);
        }
        self.side_scripts(bb, up_b, dn_b, st)
    }

    /// Superscript and subscript squeezed against the base, as big operators do in display style.
    /// `gap` is the clearance between the base and the scripts, in ems.
    fn stacked(&mut self, bb: B, up_b: Option<B>, dn_b: Option<B>, s: f32, gap: f32) -> B {
        let mut out = B::default();
        let w = bb.w.max(up_b.as_ref().map_or(0.0, |b| b.w)).max(dn_b.as_ref().map_or(0.0, |b| b.w));
        let (mut a, mut d) = (bb.a, bb.d);
        if let Some(u) = up_b {
            let y = -(bb.a + gap * s + u.d);
            a = bb.a + gap * s + u.d + u.a;
            let dx = (w - u.w) / 2.0;
            out.put(u, dx, y);
        }
        if let Some(l) = dn_b {
            let y = bb.d + gap * s + l.a;
            d = bb.d + gap * s + l.a + l.d;
            let dx = (w - l.w) / 2.0;
            out.put(l, dx, y);
        }
        let dx = (w - bb.w) / 2.0;
        out.put(bb, dx, 0.0);
        out.w = w;
        out.a = a;
        out.d = d;
        out
    }

    fn side_scripts(&mut self, bb: B, up_b: Option<B>, dn_b: Option<B>, st: St) -> B {
        let s = st.size;
        let mut out = B::default();
        let mut up = 0.0;
        let mut down = 0.0;
        if let Some(u) = &up_b {
            up = (0.38 * s).max(bb.a - 0.32 * s + u.d);
        }
        if let Some(l) = &dn_b {
            down = (0.17 * s).max(bb.d + 0.1 * s).max(l.a - 0.75 * s);
            if bb.d == 0.0 && bb.a > 0.0 {
                down = down.max(0.2 * s);
            }
        }
        if let (Some(u), Some(l)) = (&up_b, &dn_b) {
            let room = (up - u.d) - (l.a - down);
            if room < 0.2 * s {
                down += 0.2 * s - room;
            }
        }
        let bw = bb.w;
        let (mut a, mut d, mut sw) = (bb.a, bb.d, 0.0f32);
        out.put(bb, 0.0, 0.0);
        let pad = 0.03 * s;
        if let Some(u) = up_b {
            a = a.max(up + u.a);
            sw = sw.max(u.w);
            out.put(u, bw + pad, -up);
        }
        if let Some(l) = dn_b {
            d = d.max(down + l.d);
            sw = sw.max(l.w);
            out.put(l, bw + pad, down);
        }
        out.w = bw + sw + pad + 0.02 * s;
        out.a = a;
        out.d = d;
        out
    }

    /// A horizontal brace hugging `body`, with an optional annotation on the far side of it.
    ///
    /// The box grows by the brace depth plus the label height, so a paragraph that stacks several
    /// annotated underbraces keeps them apart instead of overlapping the line below.
    fn braced(&mut self, over: bool, body: B, label: Option<&[Node]>, st: St) -> B {
        let s = st.size;
        let t = 0.05 * s;
        let depth = 0.26 * s;
        let gap = 0.1 * s;
        let lab = label.map(|l| self.list(l, self.sub_st(st)));
        let w = body.w.max(lab.as_ref().map_or(0.0, |l| l.w));
        let (bw_src, bdx) = (body.w, (w - body.w) / 2.0);
        let mut out = B { w, a: body.a, d: body.d, ..B::default() };
        out.put(body, bdx, 0.0);
        let o = if over { -1.0f32 } else { 1.0 };
        let flat = o * ((if over { out.a } else { out.d }) + gap);
        let cusp = flat + o * depth;
        let mid = flat + o * depth * 0.5;
        let (bx0, bx1) = if bw_src > 0.01 { (bdx, bdx + bw_src) } else { (0.0, w) };
        let bw = (bx1 - bx0).max(0.0);
        let cx = (bx0 + bx1) / 2.0;
        let e = (bw * 0.12).min(bw * 0.25).max(0.01);
        let c = (bw * 0.06).min(bw * 0.2).max(0.01);
        out.seg(bx0, flat, bx0 + e, mid, t);
        out.seg(bx0 + e, mid, cx - c, mid, t);
        out.seg(cx - c, mid, cx, cusp, t);
        out.seg(cx, cusp, cx + c, mid, t);
        out.seg(cx + c, mid, bx1 - e, mid, t);
        out.seg(bx1 - e, mid, bx1, flat, t);
        match (lab, over) {
            (None, true) => out.a = out.a.max(-cusp + t),
            (None, false) => out.d = out.d.max(cusp + t),
            (Some(l), true) => {
                let y = cusp - 0.06 * s - l.d;
                let dx = (w - l.w) / 2.0;
                out.a = out.a.max(-y + l.a);
                out.put(l, dx, y);
            }
            (Some(l), false) => {
                let y = cusp + 0.06 * s + l.a;
                let dx = (w - l.w) / 2.0;
                out.d = out.d.max(y + l.d);
                out.put(l, dx, y);
            }
        }
        out
    }

    /// `\overset{small}{base}` and `\underset{small}{base}`.
    fn stack(&mut self, over: bool, small: &[Node], base: &[Node], st: St) -> B {
        let s = st.size;
        let b = self.list(base, st);
        let m = self.list(small, self.sub_st(st));
        let w = b.w.max(m.w);
        let (bdx, mdx) = ((w - b.w) / 2.0, (w - m.w) / 2.0);
        let mut out = B { w, a: b.a, d: b.d, ..B::default() };
        out.put(b, bdx, 0.0);
        let gap = 0.1 * s;
        if over {
            let y = -(out.a + gap + m.d);
            out.a = -y + m.a;
            out.put(m, mdx, y);
        } else {
            let y = out.d + gap + m.a;
            out.d = y + m.d;
            out.put(m, mdx, y);
        }
        out
    }

    fn frac(&mut self, num: &[Node], den: &[Node], rule: bool, display: bool, st: St) -> B {
        let s = st.size;
        let inner = if display {
            St { size: s, display: false, script: false }
        } else {
            St { size: (s * 0.78).max(5.0), display: false, script: false }
        };
        let n = self.list(num, inner);
        let d = self.list(den, inner);
        let t = if rule { 0.045 * s } else { 0.0 };
        let g = if display { 0.2 * s } else { 0.1 * s };
        let axis = AXIS * s;
        let ny = -(axis + t / 2.0 + g + n.d);
        let dy = -axis + t / 2.0 + g + d.a;
        let w = n.w.max(d.w) + 0.2 * s;
        let mut out = B { w, a: -ny + n.a, d: dy + d.d, ..B::default() };
        let (nx, dx) = ((w - n.w) / 2.0, (w - d.w) / 2.0);
        out.put(n, nx, ny);
        out.put(d, dx, dy);
        if rule {
            out.seg(0.04 * s, -axis, w - 0.04 * s, -axis, t);
        }
        out
    }

    fn sqrt(&mut self, body: &[Node], index: Option<&[Node]>, st: St) -> B {
        let s = st.size;
        let bb = self.list(body, st);
        let bar_y = -(bb.a.max(0.6 * s) + 0.12 * s);
        let bottom = bb.d.max(0.05 * s) + 0.04 * s;
        let h = bottom - bar_y;
        let rw = 0.55 * s + 0.06 * h.min(3.0 * s);
        let t = 0.05 * s;
        let mut out = B::default();
        let mut x0 = 0.0;
        if let Some(i) = index {
            let sst = St { size: (s * 0.55).max(5.0), display: false, script: true };
            let ib = self.list(i, sst);
            x0 = (ib.w - 0.25 * rw).max(0.0);
            let iw = ib.w;
            out.put(ib, (x0 + 0.1 * rw - iw * 0.0).max(0.0) - x0 + 0.0, bar_y + h * 0.42);
        }
        let mid = bar_y + h * 0.62;
        out.seg(x0, mid, x0 + rw * 0.18, mid - 0.04 * s, t * 0.8);
        out.seg(x0 + rw * 0.18, mid - 0.04 * s, x0 + rw * 0.42, bottom, t * 1.3);
        out.seg(x0 + rw * 0.42, bottom, x0 + rw, bar_y, t);
        out.seg(x0 + rw, bar_y, x0 + rw + bb.w + 0.08 * s, bar_y, t);
        out.w = x0 + rw + bb.w + 0.08 * s;
        out.a = -bar_y + t;
        out.d = bb.d.max(bottom);
        let bw = bb.w;
        let _ = bw;
        out.put(bb, x0 + rw + 0.02 * s, 0.0);
        out
    }

    fn accent(&mut self, kind: &str, body: &[Node], st: St) -> B {
        let s = st.size;
        let mut b = self.list(body, st);
        let top = -(b.a.max(0.46 * s) + 0.06 * s);
        let cx = b.w / 2.0;
        let t = 0.05 * s;
        match kind {
            "^" => {
                b.seg(cx - 0.22 * s, top - 0.02 * s, cx, top - 0.16 * s, t);
                b.seg(cx, top - 0.16 * s, cx + 0.22 * s, top - 0.02 * s, t);
            }
            "~" => {
                let y = top - 0.1 * s;
                b.seg(cx - 0.25 * s, y + 0.03 * s, cx - 0.1 * s, y - 0.05 * s, t);
                b.seg(cx - 0.1 * s, y - 0.05 * s, cx + 0.1 * s, y + 0.05 * s, t);
                b.seg(cx + 0.1 * s, y + 0.05 * s, cx + 0.25 * s, y - 0.03 * s, t);
            }
            "¯" => b.seg(cx - 0.3 * s, top - 0.05 * s, cx + 0.3 * s, top - 0.05 * s, t),
            "→" => {
                let y = top - 0.08 * s;
                b.seg(cx - 0.3 * s, y, cx + 0.3 * s, y, t);
                b.seg(cx + 0.18 * s, y - 0.07 * s, cx + 0.3 * s, y, t);
                b.seg(cx + 0.18 * s, y + 0.07 * s, cx + 0.3 * s, y, t);
            }
            "¨" => {
                for dx in [-0.1, 0.1] {
                    b.seg(cx + dx * s, top - 0.06 * s, cx + dx * s + 0.01, top - 0.06 * s, 0.08 * s);
                }
            }
            _ => b.seg(cx, top - 0.06 * s, cx + 0.01, top - 0.06 * s, 0.08 * s),
        }
        b.a = -top + 0.22 * s;
        b
    }

    fn matrix(&mut self, rows: &[Vec<Vec<Node>>], align: Align, open: &str, st: St) -> B {
        let s = st.size;
        let cst = St { size: s, display: false, script: false };
        let cells: Vec<Vec<B>> = rows
            .iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .map(|(j, c)| {
                        let mut nodes = c.clone();
                        if align == Align::Aligned
                            && j % 2 == 1
                            && c.first().is_some_and(|n| kind_of(n) == Kind::Rel)
                        {
                            nodes.insert(0, Node::Space(0.278));
                        }
                        self.list(&nodes, cst)
                    })
                    .collect()
            })
            .collect();
        let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut cw = vec![0.0f32; ncols];
        for r in &cells {
            for (j, c) in r.iter().enumerate() {
                cw[j] = cw[j].max(c.w);
            }
        }
        let cases = open == "{" && align == Align::Aligned;
        let sep = |j: usize| -> f32 {
            match align {
                Align::Center => 1.0 * s,
                Align::Aligned if cases => 1.0 * s,
                Align::Aligned => {
                    if j.is_multiple_of(2) {
                        0.0
                    } else {
                        2.0 * s
                    }
                }
            }
        };
        let mut xs = vec![0.0f32; ncols];
        let mut x = 0.0;
        for j in 0..ncols {
            xs[j] = x;
            x += cw[j] + if j + 1 < ncols { sep(j) } else { 0.0 };
        }
        let rg = 0.4 * s;
        let heights: Vec<(f32, f32)> = cells
            .iter()
            .map(|r| r.iter().fold((0.7 * s, 0.2 * s), |(a, d), c| (a.max(c.a), d.max(c.d))))
            .collect();
        let total: f32 =
            heights.iter().map(|(a, d)| a + d).sum::<f32>() + rg * (heights.len().saturating_sub(1)) as f32;
        let mut out = B { w: x, ..B::default() };
        let mut y = -AXIS * s - total / 2.0;
        out.a = AXIS * s + total / 2.0;
        out.d = total / 2.0 - AXIS * s;
        for (r, (a, d)) in cells.into_iter().zip(heights) {
            let base = y + a;
            for (j, c) in r.into_iter().enumerate() {
                let dx = match align {
                    Align::Center => (cw[j] - c.w) / 2.0,
                    Align::Aligned if cases || j % 2 == 1 => 0.0,
                    Align::Aligned => cw[j] - c.w,
                };
                out.put(c, xs[j] + dx, base);
            }
            y += a + d + rg;
        }
        out
    }

    fn fenced_h(&mut self, b: B, open: &str, close: &str, st: St) -> B {
        self.fenced(b, open, close, st)
    }

    fn fenced(&mut self, b: B, open: &str, close: &str, st: St) -> B {
        let s = st.size;
        let ax = AXIS * s;
        let half = (b.a - ax).max(b.d + ax).max(0.5 * s);
        let (top, bot) = (-(ax + half), half - ax);
        let h = bot - top;
        let mut out = B { a: -top, d: bot, ..B::default() };
        let lw = self.delim_box(&mut out, open, 0.0, top, bot, s, false);
        let bw = b.w;
        out.put(b, lw, 0.0);
        let rw = self.delim_box(&mut out, close, lw + bw, top, bot, s, true);
        out.w = lw + bw + rw;
        let _ = h;
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn delim_box(&mut self, out: &mut B, d: &str, x: f32, top: f32, bot: f32, s: f32, right: bool) -> f32 {
        if d.is_empty() {
            return 0.0;
        }
        let h = bot - top;
        let pad = 0.05 * s;
        if h <= 1.3 * s {
            let g = self.text(d, s, Face::Roman);
            let w = g.w;
            let mut g = g;
            g.g[0].y = 0.0;
            out.put(g, x + pad * 0.5, 0.0);
            return w + pad;
        }
        let t = 0.05 * s;
        let dw = (0.22 * s + 0.03 * h).min(0.5 * s);
        let (x0, x1) = (x + pad, x + pad + dw);
        let mid = (top + bot) / 2.0;
        let m = |px: f32| if right { x1 - (px - x0) } else { px };
        match d {
            "(" | ")" => {
                let n = 10;
                let mut prev = None;
                for i in 0..=n {
                    let u = i as f32 / n as f32;
                    let bulge = 1.0 - (2.0 * u - 1.0).powi(2);
                    let (px, py) = if d == "(" {
                        (x1 - dw * bulge * 0.85, top + h * u)
                    } else {
                        (x0 + dw * bulge * 0.85, top + h * u)
                    };
                    if let Some((qx, qy)) = prev {
                        out.seg(qx, qy, px, py, t);
                    }
                    prev = Some((px, py));
                }
            }
            "[" | "]" | "⌊" | "⌋" | "⌈" | "⌉" => {
                let left = matches!(d, "[" | "⌊" | "⌈");
                let (sx, ex) =
                    if left { (x0 + dw * 0.1, x0 + dw * 0.7) } else { (x1 - dw * 0.1, x1 - dw * 0.7) };
                let _ = m;
                if d != "⌊" && d != "⌋" {
                    out.seg(sx, top, ex, top, t);
                }
                if d != "⌈" && d != "⌉" {
                    out.seg(sx, bot, ex, bot, t);
                }
                out.seg(sx, top, sx, bot, t);
            }
            "|" => out.seg(x0 + dw * 0.5, top, x0 + dw * 0.5, bot, t),
            "‖" => {
                out.seg(x0 + dw * 0.3, top, x0 + dw * 0.3, bot, t);
                out.seg(x0 + dw * 0.7, top, x0 + dw * 0.7, bot, t);
            }
            "⟨" | "⟩" => {
                let (a, b) = if d == "⟨" { (x1, x0) } else { (x0, x1) };
                out.seg(a, top, b, mid, t);
                out.seg(b, mid, a, bot, t);
            }
            "{" | "}" => {
                let k = h * 0.08;
                let pts = [
                    (1.0, top),
                    (0.5, top + k),
                    (0.5, mid - k),
                    (0.0, mid),
                    (0.5, mid + k),
                    (0.5, bot - k),
                    (1.0, bot),
                ];
                for w in pts.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let (ax, bx) = if d == "{" {
                        (x0 + dw * a.0, x0 + dw * b.0)
                    } else {
                        (x1 - dw * a.0, x1 - dw * b.0)
                    };
                    out.seg(ax, a.1, bx, b.1, t);
                }
            }
            _ => {
                let g = self.text(d, s, Face::Roman);
                let w = g.w;
                out.put(g, x, 0.0);
                return w + pad;
            }
        }
        dw + 2.0 * pad
    }
}

pub fn layout(nodes: &[Node], size: f32, display: bool, m: &mut dyn Measure) -> Layout {
    layout_with(nodes, size, display, &Metrics::default(), m)
}

/// Lay out with explicit spacing knobs; see [`Metrics`].
pub fn layout_with(
    nodes: &[Node],
    size: f32,
    display: bool,
    metrics: &Metrics,
    m: &mut dyn Measure,
) -> Layout {
    let mut cx = Ctx { m, metrics: *metrics, depth: 0 };
    let b = cx.list(nodes, St { size, display, script: false });
    let px = metrics.padding_x.max(0.0);
    let py = metrics.padding_y.max(0.0);
    let (a, d) = (b.a.max(0.7 * size) + py, b.d.max(0.2 * size) + py);
    let mut l = Layout { width: b.w + 2.0 * px, ascent: a, descent: d, ..Layout::default() };
    l.glyphs =
        b.g.into_iter()
            .map(|mut g| {
                g.x += px;
                g.y += a;
                g
            })
            .collect();
    l.segs =
        b.s.into_iter()
            .map(|mut s| {
                s.x0 += px;
                s.x1 += px;
                s.y0 += a;
                s.y1 += a;
                s
            })
            .collect();
    l
}
