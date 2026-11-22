use crate::color::Color;
use crate::layout::Tag;
use crate::syms::{self, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Roman,
    Italic,
    Bold,
    Mono,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Center,
    Aligned,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    Atom { text: String, kind: Kind, face: Face },
    Group(Vec<Node>),
    Script { base: Box<Node>, sup: Option<Vec<Node>>, sub: Option<Vec<Node>> },
    Frac { num: Vec<Node>, den: Vec<Node>, rule: bool, forced: Option<bool> },
    Sqrt { body: Vec<Node>, index: Option<Vec<Node>> },
    // `\left<delim>`/`\right<delim>`: a fence that stretches with its body.
    Delim { open: String, close: String, body: Vec<Node> },
    // A delimiter of a fixed, larger-than-text size: `\bigl\{`, `\Bigm|`, `\Biggr\rangle`.
    // `scale` is the multiple of the size TeX uses for each command, and `kind` is the spacing
    // class the suffix picks (`l` Open, `r` Close, `m` Rel, plain Ord).
    Big { delim: String, scale: f32, kind: Kind },
    Text(String),
    Space(f32),
    Matrix { rows: Vec<Vec<Vec<Node>>>, open: String, close: String, align: Align },
    Accent { kind: &'static str, body: Vec<Node> },
    Line { over: bool, body: Vec<Node> },
    // `\underbrace{..}` or `\overbrace{..}`: the brace hugs the body and the `_`/`^` annotation
    // is placed against it instead of beside it.
    Brace { over: bool, body: Vec<Node> },
    // `\overset{small}{base}` and `\underset{small}{base}`.
    Stack { over: bool, small: Vec<Node>, base: Vec<Node> },
    // `\textcolor{c}{..}`, and the rest of a group after `\color{c}`.
    Color { color: Color, body: Vec<Node> },
    // `\boxed{..}`: a rectangular frame around the body.
    Boxed(Vec<Node>),
    // `\xrightarrow[below]{above}` and `\xleftarrow[below]{above}`: an extensible arrow with an
    // optional label on each side of the shaft.
    ExtArrow { left: bool, above: Vec<Node>, below: Option<Vec<Node>> },
    // `\pmod{n}`: the `(mod n)` that closes a congruence. `\bmod` is the bare operator and is an
    // ordinary Bin atom.
    Pmod(Vec<Node>),
    // `\phantom{..}`: the body's box, with no ink in it.
    Phantom(Vec<Node>),
    // `\middle<delim>`: a delimiter inside `\left..\right` that stretches with the fence.
    Middle(String),
    // `\hline` inside a matrix/aligned row: a rule above the row. Zero-width elsewhere.
    Hline,
    // `\not` puts a slash through the symbol that follows it (`\not=`, `\not\equiv`).
    Not(Box<Node>),
    // `\tag{..}`: the equation number of a display line. `star` is `\tag*{..}`, which drops the
    // parentheses amsmath adds around a plain `\tag`.
    Tag { text: String, star: bool },
    Unknown(String),
}

/// A command the layout does not know. `offset` is a byte offset into the parsed source.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub offset: usize,
    pub command: String,
}

/// What `parse_full` returns: the tree, the commands that were kept as source text, and the
/// equation number when the source carries a `\tag`.
///
/// In `tag` only `text` and `star` are meaningful: there is no layout yet, so the position and
/// width are zero. [`crate::layout_nodes_with`] fills them in on [`crate::Layout::tag`].
#[derive(Clone, Default, Debug)]
pub struct Parsed {
    pub nodes: Vec<Node>,
    pub diagnostics: Vec<Diagnostic>,
    pub tag: Option<Tag>,
}

#[derive(PartialEq, Debug)]
enum Term {
    End,
    Brace,
    Amp,
    Cr,
    EndEnv(String),
    Right(String),
}

pub fn parse(src: &str) -> Vec<Node> {
    parse_full(src).nodes
}

/// Parse and keep the diagnostics for commands that fell back to their source text.
pub fn parse_full(src: &str) -> Parsed {
    let mut p = Parser::new(src);
    let nodes = p.run();
    Parsed { nodes, diagnostics: p.diags, tag: p.tag }
}

struct Parser {
    c: Vec<char>,
    i: usize,
    depth: usize,
    /// Byte offset of `c[0]` in the source this parser is reading; non-zero for a fragment.
    base: usize,
    diags: Vec<Diagnostic>,
    /// The first `\tag` seen, kept flat so the caller does not have to walk the tree for it.
    tag: Option<Tag>,
}

fn atom(text: impl Into<String>, kind: Kind, face: Face) -> Node {
    Node::Atom { text: text.into(), kind, face }
}

impl Parser {
    fn new(src: &str) -> Self {
        Parser { c: src.chars().collect(), i: 0, depth: 0, base: 0, diags: Vec::new(), tag: None }
    }

    fn run(&mut self) -> Vec<Node> {
        let mut out = Vec::new();
        loop {
            let (mut nodes, term) = self.seq(None);
            out.append(&mut nodes);
            if term == Term::End || self.i >= self.c.len() {
                return out;
            }
        }
    }

    /// Byte offset of the char at `i`, in the coordinates of the whole source.
    fn byte_at(&self, i: usize) -> usize {
        self.base + self.c[..i.min(self.c.len())].iter().map(|c| c.len_utf8()).sum::<usize>()
    }

    /// Keep `src` as source text and record where it was.
    fn unknown(&mut self, at: usize, cmd: String) -> Node {
        self.note(at, cmd.clone());
        Node::Unknown(cmd)
    }

    fn note(&mut self, at: usize, cmd: String) {
        self.diags.push(Diagnostic { offset: self.byte_at(at), command: cmd });
    }

    /// Parse a nested source that came from somewhere inside this one, keeping byte offsets honest.
    fn fragment(&mut self, src: &str, base: usize) -> Vec<Node> {
        let mut p = Parser { base, ..Parser::new(src) };
        let nodes = p.run();
        self.diags.append(&mut p.diags);
        nodes
    }

    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn name(&mut self) -> String {
        let s = self.i;
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.i += 1;
            }
        } else if self.peek().is_some() {
            self.i += 1;
        }
        self.c[s..self.i].iter().collect()
    }

    fn raw_group(&mut self) -> String {
        self.skip_ws();
        if self.peek() != Some('{') {
            return self.name();
        }
        self.i += 1;
        let s = self.i;
        let mut d = 1;
        let mut closed = false;
        while let Some(c) = self.peek() {
            match c {
                '{' => d += 1,
                '}' => {
                    d -= 1;
                    if d == 0 {
                        closed = true;
                        break;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        let r = self.c[s..self.i.min(self.c.len())].iter().collect();
        // Only step over a closing brace that is actually there: an unterminated group leaves the
        // cursor at the end of the input, and moving it one past would index out of bounds on the
        // next token (`\begin{matrix` cut short in a note is enough).
        if closed {
            self.i += 1;
        }
        r
    }

    fn arg(&mut self, face: Option<Face>) -> Vec<Node> {
        self.skip_ws();
        match self.peek() {
            Some('{') => {
                self.i += 1;
                self.seq(face).0
            }
            Some('\\') => {
                let start = self.i;
                self.i += 1;
                let n = self.name();
                vec![self.command(&n, face, start)]
            }
            Some(c) => {
                self.i += 1;
                vec![self.ch(c, face)]
            }
            None => vec![],
        }
    }

    fn ch(&self, c: char, face: Option<Face>) -> Node {
        let s = c.to_string();
        match c {
            'a'..='z' | 'A'..='Z' => atom(s, Kind::Ord, face.unwrap_or(Face::Italic)),
            '0'..='9' | '.' => atom(s, Kind::Ord, face.unwrap_or(Face::Roman)),
            '+' | '*' => atom(if c == '*' { "∗".into() } else { s }, Kind::Bin, Face::Roman),
            '-' => atom("−", Kind::Bin, Face::Roman),
            '=' | '<' | '>' => atom(s, Kind::Rel, Face::Roman),
            ':' => atom(s, Kind::Rel, Face::Roman),
            '(' | '[' => atom(s, Kind::Open, Face::Roman),
            ')' | ']' => atom(s, Kind::Close, Face::Roman),
            ',' | ';' => atom(s, Kind::Punct, Face::Roman),
            '|' => atom(s, Kind::Ord, Face::Roman),
            '\'' => atom("′", Kind::Ord, Face::Roman),
            c if c.is_alphabetic() => atom(s, Kind::Ord, face.unwrap_or(Face::Italic)),
            _ => atom(s, Kind::Ord, face.unwrap_or(Face::Roman)),
        }
    }

    fn seq(&mut self, face: Option<Face>) -> (Vec<Node>, Term) {
        self.depth += 1;
        let r = if self.depth > 60 { (Vec::new(), Term::End) } else { self.seq_inner(face) };
        self.depth -= 1;
        r
    }

    fn seq_inner(&mut self, face: Option<Face>) -> (Vec<Node>, Term) {
        let mut out: Vec<Node> = Vec::new();
        loop {
            let Some(c) = self.peek() else { return (out, Term::End) };
            self.i += 1;
            match c {
                c if c.is_whitespace() => {}
                '{' => {
                    let (g, t) = self.seq(face);
                    out.push(Node::Group(g));
                    if t != Term::Brace {
                        return (out, t);
                    }
                }
                '}' => return (out, Term::Brace),
                '&' => return (out, Term::Amp),
                '^' | '_' => {
                    let arg = self.arg(face);
                    let base = out.pop().unwrap_or(Node::Group(vec![]));
                    let (mut b, mut sup, mut sub) = match base {
                        Node::Script { base, sup, sub } => (*base, sup, sub),
                        other => (other, None, None),
                    };
                    let slot = if c == '^' { &mut sup } else { &mut sub };
                    if slot.is_some() {
                        b = Node::Script { base: Box::new(b), sup: sup.take(), sub: sub.take() };
                        let (s1, s2) = if c == '^' { (Some(arg), None) } else { (None, Some(arg)) };
                        out.push(Node::Script { base: Box::new(b), sup: s1, sub: s2 });
                    } else {
                        *slot = Some(arg);
                        out.push(Node::Script { base: Box::new(b), sup, sub });
                    }
                }
                '~' => out.push(Node::Space(0.33)),
                '%' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '\\' => {
                    let start = self.i - 1;
                    let n = self.name();
                    match n.as_str() {
                        "\\" | "cr" => return (out, Term::Cr),
                        "right" => {
                            let d = self.delim();
                            return (out, Term::Right(d));
                        }
                        "end" => {
                            let e = self.raw_group();
                            return (out, Term::EndEnv(e));
                        }
                        "color" => {
                            // A declaration: it colours the rest of the current group.
                            let color = self.color_spec();
                            let (rest, t) = self.seq(face);
                            match color {
                                Some(color) => out.push(Node::Color { color, body: rest }),
                                None => out.extend(rest),
                            }
                            return (out, t);
                        }
                        _ => {
                            let node = self.command(&n, face, start);
                            out.push(node);
                        }
                    }
                }
                c => out.push(self.ch(c, face)),
            }
        }
    }

    fn delim(&mut self) -> String {
        self.skip_ws();
        match self.peek() {
            Some('\\') => {
                self.i += 1;
                let n = self.name();
                match n.as_str() {
                    "{" | "lbrace" => "{".into(),
                    "}" | "rbrace" => "}".into(),
                    "|" | "Vert" | "lVert" | "rVert" => "‖".into(),
                    "vert" | "lvert" | "rvert" => "|".into(),
                    "langle" => "⟨".into(),
                    "rangle" => "⟩".into(),
                    "lfloor" => "⌊".into(),
                    "rfloor" => "⌋".into(),
                    "lceil" => "⌈".into(),
                    "rceil" => "⌉".into(),
                    _ => String::new(),
                }
            }
            Some('.') => {
                self.i += 1;
                String::new()
            }
            Some(c) => {
                self.i += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    fn color_spec(&mut self) -> Option<Color> {
        let raw = self.raw_group();
        Color::parse(&raw)
    }

    /// `\xrightarrow[below]{above}` and `\xleftarrow[below]{above}`.
    ///
    /// amsmath puts the arrow shaft length under the control of both labels, so the bracket and the
    /// brace are read in that order. The vault also writes the label as a lone trailing `_{..}` or
    /// `^{..}`; that form is accepted too.
    fn ext_arrow(&mut self, left: bool, face: Option<Face>) -> Node {
        // The bracket comes first in the source: `\xrightarrow[below]{above}`.
        let below = self.bracket_arg();
        let mut above = self.arg(face);
        let below = below.or_else(|| leading_underscore(&mut above));
        Node::ExtArrow { left, above, below }
    }

    /// `\not`: the slash applies to the single symbol or command that follows it.
    fn not(&mut self, face: Option<Face>) -> Node {
        let mut base = self.arg(face);
        let node = match base.len() {
            0 => return Node::Space(0.0),
            1 => base.remove(0),
            _ => Node::Group(base),
        };
        Node::Not(Box::new(node))
    }

    fn tag(&mut self) -> Node {
        let star = self.peek() == Some('*');
        if star {
            self.i += 1;
        }
        let text = self.raw_group().trim().to_string();
        if self.tag.is_none() {
            self.tag = Some(Tag { text: text.clone(), star, ..Tag::default() });
        }
        Node::Tag { text, star }
    }

    /// The optional `[...]` argument of `\xrightarrow` and friends, parsed as math.
    fn bracket_arg(&mut self) -> Option<Vec<Node>> {
        self.skip_ws();
        if self.peek() != Some('[') {
            return None;
        }
        self.i += 1;
        let start = self.i;
        while self.peek().is_some_and(|c| c != ']') {
            self.i += 1;
        }
        let sub: String = self.c[start..self.i.min(self.c.len())].iter().collect();
        let base = self.byte_at(start);
        self.i += 1;
        Some(self.fragment(&sub, base))
    }

    fn command(&mut self, n: &str, face: Option<Face>, start: usize) -> Node {
        if let Some((s, kind)) = syms::lookup(n) {
            let f = if syms::is_greek_lower(s) { face.unwrap_or(Face::Italic) } else { Face::Roman };
            return atom(s, kind, f);
        }
        if syms::FUNCS.contains(&n) {
            return Node::Group(vec![atom(n, Kind::Op, Face::Roman), Node::Space(0.17)]);
        }
        match n {
            "," => Node::Space(0.17),
            ":" | ">" => Node::Space(0.22),
            ";" => Node::Space(0.28),
            "!" => Node::Space(-0.17),
            " " => Node::Space(0.28),
            "quad" => Node::Space(1.0),
            "qquad" => Node::Space(2.0),
            "{" => atom("{", Kind::Open, Face::Roman),
            "}" => atom("}", Kind::Close, Face::Roman),
            "|" => atom("‖", Kind::Ord, Face::Roman),
            "%" | "$" | "&" | "#" | "_" => atom(n, Kind::Ord, Face::Roman),
            "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" | "left" => {
                if n == "left" {
                    return self.left(face);
                }
                Node::Space(0.0)
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr"
            | "bigm" | "Bigm" | "biggm" | "Biggm" => {
                let d = self.delim();
                // TeX sets these at 1.2, 1.8, 2.4 and 3 times the size of the surrounding text:
                // the capital doubles the step again, and each extra `g` adds half a step.
                let scale = match (n.starts_with('B'), n.matches('g').count()) {
                    (false, 1) => 1.2,
                    (true, 1) => 1.8,
                    (false, _) => 2.4,
                    (true, _) => 3.0,
                };
                // The suffix is the spacing class: `\bigl(` opens, `\Bigr)` closes, `\Bigm|` is a
                // relation, and a plain `\big(` is an ordinary atom.
                let kind = if n.ends_with('l') {
                    Kind::Open
                } else if n.ends_with('r') {
                    Kind::Close
                } else if n.ends_with('m') {
                    Kind::Rel
                } else {
                    Kind::Ord
                };
                Node::Big { delim: d, scale, kind }
            }
            "middle" => Node::Middle(self.delim()),
            "hline" => Node::Hline,
            "phantom" => Node::Phantom(self.arg(face)),
            "pmod" => Node::Pmod(self.arg(face)),
            "bmod" => atom("mod", Kind::Bin, Face::Mono),
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.arg(face);
                let den = self.arg(face);
                let forced = match n {
                    "dfrac" => Some(true),
                    "tfrac" => Some(false),
                    _ => None,
                };
                Node::Frac { num, den, rule: true, forced }
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.arg(face);
                let den = self.arg(face);
                Node::Delim {
                    open: "(".into(),
                    close: ")".into(),
                    body: vec![Node::Frac { num, den, rule: false, forced: None }],
                }
            }
            "sqrt" => {
                self.skip_ws();
                let index = if self.peek() == Some('[') {
                    self.i += 1;
                    let s = self.i;
                    while self.peek().is_some_and(|c| c != ']') {
                        self.i += 1;
                    }
                    let sub: String = self.c[s..self.i].iter().collect();
                    self.i += 1;
                    let base = self.byte_at(s);
                    Some(self.fragment(&sub, base))
                } else {
                    None
                };
                Node::Sqrt { body: self.arg(face), index }
            }
            "text" | "textrm" | "mbox" | "textnormal" | "textit" | "textbf" | "operatorname" | "mathrm"
            | "mathit" | "mathbf" | "boldsymbol" | "bm" | "mathbb" | "mathcal" | "mathscr" | "mathfrak"
            | "mathsf" | "mathtt" | "texttt" => {
                let f = match n {
                    "mathbf" | "boldsymbol" | "bm" | "textbf" => Face::Bold,
                    "mathit" | "textit" => Face::Italic,
                    "mathtt" => Face::Mono,
                    _ => Face::Roman,
                };
                // `\texttt` keeps its spaces and its monospace face; it is a run, not plain text.
                if n == "texttt" {
                    return atom(self.raw_group(), Kind::Ord, Face::Mono);
                }
                if matches!(
                    n,
                    "text" | "textrm" | "mbox" | "textnormal" | "textit" | "textbf" | "operatorname"
                ) {
                    let t = self.raw_group();
                    return Node::Atom {
                        text: t,
                        kind: if n == "operatorname" { Kind::Op } else { Kind::Ord },
                        face: if n == "text" || n == "mbox" { Face::Roman } else { f },
                    }
                    .into_text(n != "operatorname");
                }
                let mut body = self.arg(Some(f));
                let map: Option<fn(char) -> char> = match n {
                    "mathbb" => Some(syms::blackboard),
                    "mathcal" | "mathscr" => Some(syms::calligraphic),
                    "mathfrak" => Some(syms::fraktur),
                    _ => None,
                };
                if let Some(m) = map {
                    map_text(&mut body, m);
                }
                Node::Group(body)
            }
            "hat" | "widehat" => Node::Accent { kind: "^", body: self.arg(face) },
            "tilde" | "widetilde" => Node::Accent { kind: "~", body: self.arg(face) },
            "bar" => Node::Accent { kind: "¯", body: self.arg(face) },
            "vec" => Node::Accent { kind: "→", body: self.arg(face) },
            "dot" => Node::Accent { kind: "˙", body: self.arg(face) },
            "ddot" => Node::Accent { kind: "¨", body: self.arg(face) },
            "check" => Node::Accent { kind: "ˇ", body: self.arg(face) },
            "overline" => Node::Line { over: true, body: self.arg(face) },
            "underline" => Node::Line { over: false, body: self.arg(face) },
            "underbrace" => Node::Brace { over: false, body: self.arg(face) },
            "overbrace" => Node::Brace { over: true, body: self.arg(face) },
            "overset" | "stackrel" => {
                let small = self.arg(face);
                let base = self.arg(face);
                Node::Stack { over: true, small, base }
            }
            "underset" => {
                let small = self.arg(face);
                let base = self.arg(face);
                Node::Stack { over: false, small, base }
            }
            "textcolor" => {
                let color = self.color_spec();
                let body = self.arg(face);
                match color {
                    Some(color) => Node::Color { color, body },
                    None => Node::Group(body),
                }
            }
            "boxed" => Node::Boxed(self.arg(face)),
            "xrightarrow" | "xleftarrow" => self.ext_arrow(n == "xleftarrow", face),
            "not" => self.not(face),
            "tag" => self.tag(),
            "begin" => self.environment(start),
            "mathbin" | "mathrel" | "mathop" | "mathord" => Node::Group(self.arg(face)),
            _ => self.unknown(start, format!("\\{n}")),
        }
    }

    fn left(&mut self, face: Option<Face>) -> Node {
        let open = self.delim();
        let (body, t) = self.seq(face);
        let close = match t {
            Term::Right(d) => d,
            _ => String::new(),
        };
        Node::Delim { open, close, body }
    }

    fn environment(&mut self, start: usize) -> Node {
        let env = self.raw_group();
        let (open, close, align) = match env.trim_end_matches('*') {
            "pmatrix" => ("(", ")", Align::Center),
            "bmatrix" => ("[", "]", Align::Center),
            "Bmatrix" => ("{", "}", Align::Center),
            "vmatrix" => ("|", "|", Align::Center),
            "Vmatrix" => ("‖", "‖", Align::Center),
            "matrix" | "smallmatrix" | "array" | "gathered" | "gather" => ("", "", Align::Center),
            "cases" => ("{", "", Align::Aligned),
            "aligned" | "align" | "split" | "alignedat" | "eqnarray" => ("", "", Align::Aligned),
            _ => {
                let end = format!("\\end{{{env}}}");
                let rest: String = self.c[self.i..].iter().collect();
                let src = match rest.find(&end) {
                    Some(p) => {
                        self.i += rest[..p].chars().count() + end.chars().count();
                        &rest[..p]
                    }
                    None => {
                        self.i = self.c.len();
                        &rest[..]
                    }
                };
                self.note(start, format!("\\begin{{{env}}}"));
                return Node::Unknown(format!("\\begin{{{env}}}{}\\end{{{env}}}", src.trim()));
            }
        };
        if env == "array" {
            self.raw_group();
        }
        let mut rows: Vec<Vec<Vec<Node>>> = vec![vec![]];
        loop {
            let (cell, t) = self.seq(None);
            rows.last_mut().unwrap().push(cell);
            match t {
                Term::Amp => {}
                Term::Cr => rows.push(vec![]),
                _ => break,
            }
        }
        if rows.last().is_some_and(|r| r.len() == 1 && r[0].is_empty()) && rows.len() > 1 {
            rows.pop();
        }
        Node::Matrix { rows, open: open.into(), close: close.into(), align }
    }
}

impl Node {
    fn into_text(self, plain: bool) -> Node {
        match self {
            Node::Atom { text, kind, .. } => {
                if plain {
                    Node::Text(text)
                } else {
                    Node::Atom { text, kind, face: Face::Roman }
                }
            }
            n => n,
        }
    }
}

fn map_text(nodes: &mut [Node], f: fn(char) -> char) {
    for n in nodes {
        match n {
            Node::Atom { text, .. } => *text = text.chars().map(f).collect(),
            Node::Group(g) => map_text(g, f),
            _ => {}
        }
    }
}

/// Peel the `_`/`^` off a lone label argument: `\xrightarrow{_{n\to\infty}}` is a below-label.
///
/// The parser turns a leading `_` with no base into `Script` over an empty group, so that shape is
/// the signal. Returns the below-label and leaves `above` holding whatever the `^` carried.
fn leading_underscore(above: &mut Vec<Node>) -> Option<Vec<Node>> {
    let [Node::Script { base, sup, sub }] = above.as_slice() else { return None };
    if !matches!(base.as_ref(), Node::Group(g) if g.is_empty()) || (sup.is_none() && sub.is_none()) {
        return None;
    }
    let below = sub.clone();
    let rest = sup.clone();
    *above = rest.unwrap_or_default();
    below
}
