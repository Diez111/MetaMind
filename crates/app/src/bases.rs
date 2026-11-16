//! Bases: `.base` files that show the notes matching a filter as a table or cards, with columns
//! taken from note properties, file fields and formulas.
//!
//! The file format and expression syntax follow the `.base` files other apps write, so a vault's
//! existing bases open unchanged. Supported: `filters` (expression strings and nested `and`, `or`,
//! `not`), `formulas`, `properties.*.displayName`, and `views` of type `table`, `cards` and `list`
//! with `filters`, `order`, `sort`, `limit` and `groupBy`.

use md_engine::frontmatter::{self, Value as Yaml};
use std::cmp::Ordering;
use std::fmt;
use vault_core::path;

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Val>),
}

impl Val {
    fn from_yaml(y: &Yaml) -> Val {
        match y {
            Yaml::Null | Yaml::Map(_) => Val::Null,
            Yaml::Bool(b) => Val::Bool(*b),
            Yaml::Number(n) => Val::Num(*n),
            Yaml::String(s) => Val::Str(s.clone()),
            Yaml::List(l) => Val::List(l.iter().map(Val::from_yaml).collect()),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Val::Null => false,
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Str(s) => !s.is_empty(),
            Val::List(l) => !l.is_empty(),
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Bool(b) => Some(f64::from(u8::from(*b))),
            Val::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Val::Null => true,
            Val::Str(s) => s.is_empty(),
            Val::List(l) => l.is_empty(),
            _ => false,
        }
    }

    fn items(&self) -> Vec<Val> {
        match self {
            Val::List(l) => l.clone(),
            Val::Null => Vec::new(),
            v => vec![v.clone()],
        }
    }
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Null => Ok(()),
            Val::Bool(b) => write!(f, "{b}"),
            Val::Num(n) => f.write_str(&frontmatter::fmt_number(*n)),
            Val::Str(s) => f.write_str(s),
            Val::List(l) => {
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                Ok(())
            }
        }
    }
}

/// Equality as filters use it: numbers compare by value whatever their spelling, text ignores
/// a `[[...]]` wrapper so `link == "Note"` matches.
fn loose_eq(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Null, v) | (v, Val::Null) => v.is_empty(),
        (Val::List(x), Val::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| loose_eq(a, b)),
        (Val::Num(_), _) | (_, Val::Num(_)) => match (a.num(), b.num()) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        (Val::Bool(x), Val::Bool(y)) => x == y,
        _ => link_name(&a.to_string()).eq_ignore_ascii_case(link_name(&b.to_string())),
    }
}

fn link_name(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")).unwrap_or(s);
    s.split('|').next().unwrap_or(s)
}

/// Sort order: empty values last, then numbers, then text without case.
pub fn compare(a: &Val, b: &Val) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    if let (Val::Num(_) | Val::Bool(_), Val::Num(_) | Val::Bool(_)) = (a, b) {
        return a.num().partial_cmp(&b.num()).unwrap_or(Ordering::Equal);
    }
    let (x, y) = (a.to_string(), b.to_string());
    match (x.parse::<f64>(), y.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => x.to_lowercase().cmp(&y.to_lowercase()),
    }
}

// ---- expressions ----

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(Val),
    /// A dotted name such as `status`, `note.status`, `file.name` or `formula.total`.
    Name(Vec<String>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Method(Box<Expr>, String, Vec<Expr>),
    Field(Box<Expr>, String),
    List(Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Sym(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    const SYMS: [&str; 20] = [
        "==", "!=", "<=", ">=", "&&", "||", "<", ">", "!", "+", "-", "*", "/", "%", "(", ")", ",", ".", "[",
        "]",
    ];
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'"' || c == b'\'' {
            let mut s = String::new();
            let mut chars = src[i + 1..].char_indices();
            let mut end = None;
            while let Some((j, ch)) = chars.next() {
                match ch {
                    '\\' => {
                        if let Some((_, e)) = chars.next() {
                            s.push(match e {
                                'n' => '\n',
                                't' => '\t',
                                e => e,
                            });
                        }
                    }
                    ch if ch as u32 == u32::from(c) => {
                        end = Some(i + 1 + j + 1);
                        break;
                    }
                    ch => s.push(ch),
                }
            }
            i = end.ok_or("unterminated string")?;
            out.push(Tok::Str(s));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_digit() || b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
            {
                i += 1;
            }
            out.push(Tok::Num(src[start..i].parse().map_err(|_| "bad number")?));
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                i += 1;
            }
            out.push(Tok::Ident(src[start..i].to_string()));
        } else if let Some(s) = SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            i += s.len();
            out.push(Tok::Sym(s));
        } else {
            return Err(format!("unexpected `{}`", src[i..].chars().next().unwrap_or(' ')));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }
    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(format!("expected `{s}`"))
        }
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[(&str, Op)]; 5] = [
            &[("||", Op::Or)],
            &[("&&", Op::And)],
            &[("==", Op::Eq), ("!=", Op::Ne), ("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)],
            &[("+", Op::Add), ("-", Op::Sub)],
            &[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)],
        ];
        let Some(ops) = LEVELS.get(level) else { return self.unary() };
        let mut lhs = self.binary(level + 1)?;
        'outer: loop {
            for (s, op) in ops.iter() {
                if self.eat(s) {
                    let rhs = self.binary(level + 1)?;
                    lhs = Expr::Bin(*op, Box::new(lhs), Box::new(rhs));
                    continue 'outer;
                }
            }
            return Ok(lhs);
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        let mut e = self.primary()?;
        loop {
            if self.eat(".") {
                let Some(Tok::Ident(name)) = self.peek().cloned() else {
                    return Err("expected a name after `.`".into());
                };
                self.at += 1;
                if self.eat("(") {
                    let args = self.args(")")?;
                    e = Expr::Method(Box::new(e), name, args);
                } else if let Expr::Name(mut parts) = e {
                    parts.push(name);
                    e = Expr::Name(parts);
                } else {
                    e = Expr::Field(Box::new(e), name);
                }
            } else if self.eat("[") {
                let i = self.binary(0)?;
                self.expect("]")?;
                e = Expr::Method(Box::new(e), "at".into(), vec![i]);
            } else {
                return Ok(e);
            }
        }
    }

    fn args(&mut self, close: &str) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if self.eat(close) {
            return Ok(args);
        }
        loop {
            args.push(self.binary(0)?);
            if self.eat(close) {
                return Ok(args);
            }
            self.expect(",")?;
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let t = self.peek().cloned().ok_or("unexpected end")?;
        self.at += 1;
        Ok(match t {
            Tok::Num(n) => Expr::Lit(Val::Num(n)),
            Tok::Str(s) => Expr::Lit(Val::Str(s)),
            Tok::Sym("(") => {
                let e = self.binary(0)?;
                self.expect(")")?;
                e
            }
            Tok::Sym("[") => Expr::List(self.args("]")?),
            Tok::Ident(id) => match id.as_str() {
                "true" => Expr::Lit(Val::Bool(true)),
                "false" => Expr::Lit(Val::Bool(false)),
                "null" => Expr::Lit(Val::Null),
                _ if self.eat("(") => Expr::Call(id, self.args(")")?),
                _ => Expr::Name(vec![id]),
            },
            Tok::Sym(s) => return Err(format!("unexpected `{s}`")),
        })
    }
}

pub fn parse_expr(src: &str) -> Result<Expr, String> {
    let mut p = Parser { toks: lex(src)?, at: 0 };
    let e = p.binary(0)?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(format!("unexpected {t:?}")),
    }
}

// ---- the file ----

#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    Expr(Expr),
    And(Vec<Filter>),
    Or(Vec<Filter>),
    /// True when none of the filters match.
    Not(Vec<Filter>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Table,
    Cards,
    List,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub kind: ViewKind,
    pub name: String,
    pub filter: Option<Filter>,
    /// Column property names, such as `file.name`, `status` or `formula.total`.
    pub order: Vec<String>,
    /// Property and ascending.
    pub sort: Vec<(String, bool)>,
    pub limit: Option<usize>,
    pub group_by: Option<(String, bool)>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Base {
    pub filter: Option<Filter>,
    pub formulas: Vec<(String, Expr)>,
    pub display: Vec<(String, String)>,
    pub views: Vec<View>,
}

fn get<'a>(map: &'a [(String, Yaml)], key: &str) -> Option<&'a Yaml> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn parse_filter(y: &Yaml) -> Result<Filter, String> {
    match y {
        Yaml::String(s) => parse_expr(s).map(Filter::Expr).map_err(|e| format!("filter `{s}`: {e}")),
        Yaml::Bool(b) => Ok(Filter::Expr(Expr::Lit(Val::Bool(*b)))),
        Yaml::Map(m) => {
            let list = |k: &str| -> Result<Option<Vec<Filter>>, String> {
                match get(m, k) {
                    Some(Yaml::List(items)) => {
                        items.iter().map(parse_filter).collect::<Result<_, _>>().map(Some)
                    }
                    Some(other) => Ok(Some(vec![parse_filter(other)?])),
                    None => Ok(None),
                }
            };
            if let Some(f) = list("and")? {
                Ok(Filter::And(f))
            } else if let Some(f) = list("or")? {
                Ok(Filter::Or(f))
            } else if let Some(f) = list("not")? {
                Ok(Filter::Not(f))
            } else {
                Err("a filter map needs `and`, `or` or `not`".into())
            }
        }
        _ => Err("a filter must be an expression or an `and`/`or`/`not` map".into()),
    }
}

fn opt_filter(m: &[(String, Yaml)]) -> Result<Option<Filter>, String> {
    get(m, "filters").filter(|y| **y != Yaml::Null).map(parse_filter).transpose()
}

/// Turn `note.x` into `x` so columns written either way share one key.
pub fn canonical(prop: &str) -> String {
    prop.trim().strip_prefix("note.").unwrap_or(prop.trim()).to_string()
}

impl Base {
    pub fn parse(text: &str) -> Result<Base, String> {
        if text.trim().is_empty() {
            return Ok(Base { views: vec![View::default_table()], ..Base::default() });
        }
        let top = frontmatter::parse(text).map_err(|e| e.to_string())?;
        let mut base = Base { filter: opt_filter(&top)?, ..Base::default() };
        if let Some(Yaml::Map(f)) = get(&top, "formulas") {
            for (k, v) in f {
                let src = match v {
                    Yaml::String(s) => s.clone(),
                    other => frontmatter::to_string(&[(String::new(), other.clone())])
                        .trim_start_matches(':')
                        .trim()
                        .to_string(),
                };
                base.formulas.push((k.clone(), parse_expr(&src).map_err(|e| format!("formula {k}: {e}"))?));
            }
        }
        if let Some(Yaml::Map(p)) = get(&top, "properties") {
            for (k, v) in p {
                let Yaml::Map(m) = v else { continue };
                if let Some(Yaml::String(name)) = get(m, "displayName") {
                    base.display.push((canonical(k), name.clone()));
                }
            }
        }
        for v in get(&top, "views").map(|v| v_list(v)).unwrap_or_default() {
            let Yaml::Map(m) = v else { continue };
            let str_of = |k: &str| get(m, k).and_then(Yaml::as_str).map(str::to_string);
            let kind = match str_of("type").as_deref() {
                Some("cards") => ViewKind::Cards,
                Some("list") => ViewKind::List,
                _ => ViewKind::Table,
            };
            let order = get(m, "order").map(|o| o.as_string_list()).unwrap_or_default();
            let sort = get(m, "sort")
                .map(v_list)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|s| match s {
                    Yaml::Map(s) => {
                        let p = get(s, "property").and_then(Yaml::as_str)?;
                        let asc = !get(s, "direction")
                            .and_then(Yaml::as_str)
                            .is_some_and(|d| d.eq_ignore_ascii_case("desc"));
                        Some((canonical(p), asc))
                    }
                    Yaml::String(p) => Some((canonical(p), true)),
                    _ => None,
                })
                .collect();
            let group_by = match get(m, "groupBy") {
                Some(Yaml::Map(g)) => get(g, "property").and_then(Yaml::as_str).map(|p| {
                    let desc = get(g, "direction")
                        .and_then(Yaml::as_str)
                        .is_some_and(|d| d.eq_ignore_ascii_case("desc"));
                    (canonical(p), !desc)
                }),
                Some(Yaml::String(p)) => Some((canonical(p), true)),
                _ => None,
            };
            base.views.push(View {
                kind,
                name: str_of("name").unwrap_or_else(|| "View".into()),
                filter: opt_filter(m)?,
                order: order.iter().map(|o| canonical(o)).collect(),
                sort,
                limit: get(m, "limit").and_then(|l| match l {
                    Yaml::Number(n) if *n >= 1.0 => Some(*n as usize),
                    _ => None,
                }),
                group_by,
            });
        }
        if base.views.is_empty() {
            base.views.push(View::default_table());
        }
        Ok(base)
    }

    /// The column header for `prop`.
    pub fn label(&self, prop: &str) -> String {
        if let Some((_, d)) = self.display.iter().find(|(k, _)| k == prop) {
            return d.clone();
        }
        match prop {
            "file.name" => "Name".into(),
            "file.path" => "Path".into(),
            "file.folder" => "Folder".into(),
            "file.mtime" => "Modified".into(),
            "file.size" => "Size".into(),
            "file.tags" => "Tags".into(),
            p => p.strip_prefix("formula.").unwrap_or(p).to_string(),
        }
    }
}

fn v_list(y: &Yaml) -> Vec<&Yaml> {
    match y {
        Yaml::List(l) => l.iter().collect(),
        Yaml::Null => Vec::new(),
        other => vec![other],
    }
}

impl View {
    fn default_table() -> View {
        View {
            kind: ViewKind::Table,
            name: "Table".into(),
            filter: None,
            order: vec!["file.name".into()],
            sort: Vec::new(),
            limit: None,
            group_by: None,
        }
    }
}

// ---- evaluation ----

/// What an expression can see of one note.
pub struct Row<'a> {
    pub path: &'a str,
    pub props: &'a [(String, Yaml)],
    pub tags: Vec<&'a str>,
    pub links: Vec<&'a str>,
    /// Nanoseconds since the epoch.
    pub mtime: i64,
    pub size: u64,
    /// Seconds since the epoch, for `now()` and `today()`.
    pub now: u64,
}

pub struct Ctx<'a> {
    pub base: &'a Base,
    pub row: &'a Row<'a>,
    depth: usize,
}

fn date_string(secs: u64, time: bool) -> String {
    let (y, mo, d, h, mi, s, _) = crate::datetime::local(secs);
    if time {
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
    } else {
        format!("{y:04}-{mo:02}-{d:02}")
    }
}

fn has_tag(tags: &[&str], want: &str) -> bool {
    let want = want.trim_start_matches('#');
    tags.iter().any(|t| {
        t.eq_ignore_ascii_case(want)
            || t.len() > want.len()
                && t.as_bytes()[want.len()] == b'/'
                && t[..want.len()].eq_ignore_ascii_case(want)
    })
}

impl<'a> Ctx<'a> {
    pub fn new(base: &'a Base, row: &'a Row<'a>) -> Ctx<'a> {
        Ctx { base, row, depth: 0 }
    }

    /// The value of a column such as `file.name`, `status` or `formula.total`.
    pub fn prop(&mut self, prop: &str) -> Val {
        let parts: Vec<String> = prop.split('.').map(str::to_string).collect();
        self.name(&parts)
    }

    fn name(&mut self, parts: &[String]) -> Val {
        let r = self.row;
        let (head, rest) = match parts.first().map(String::as_str) {
            Some("file") if parts.len() >= 2 => {
                let v = match parts[1].as_str() {
                    "name" => Val::Str(path::stem(r.path).to_string()),
                    "basename" => Val::Str(path::stem(r.path).to_string()),
                    "path" => Val::Str(r.path.to_string()),
                    "folder" => Val::Str(path::parent(r.path).to_string()),
                    "ext" => Val::Str(path::extension(r.path).to_string()),
                    "size" => Val::Num(r.size as f64),
                    "mtime" | "ctime" => Val::Str(date_string((r.mtime.max(0) / 1_000_000_000) as u64, true)),
                    "tags" => Val::List(r.tags.iter().map(|t| Val::Str(t.to_string())).collect()),
                    "links" => Val::List(r.links.iter().map(|t| Val::Str(t.to_string())).collect()),
                    _ => Val::Null,
                };
                (v, &parts[2..])
            }
            Some("formula") if parts.len() >= 2 => {
                let f = self.base.formulas.iter().find(|(k, _)| *k == parts[1]).map(|(_, e)| e);
                let v = match f {
                    Some(e) if self.depth < 16 => {
                        self.depth += 1;
                        let v = self.eval(e);
                        self.depth -= 1;
                        v
                    }
                    _ => Val::Null,
                };
                (v, &parts[2..])
            }
            Some("note") if parts.len() >= 2 => (self.property(&parts[1]), &parts[2..]),
            Some(p) => (self.property(p), &parts[1..]),
            None => (Val::Null, &parts[..0]),
        };
        rest.iter().fold(head, |v, f| field(&v, f))
    }

    fn property(&self, key: &str) -> Val {
        self.row
            .props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map_or(Val::Null, |(_, v)| Val::from_yaml(v))
    }

    pub fn eval(&mut self, e: &Expr) -> Val {
        match e {
            Expr::Lit(v) => v.clone(),
            Expr::Name(parts) => self.name(parts),
            Expr::Not(x) => Val::Bool(!self.eval(x).truthy()),
            Expr::Neg(x) => self.eval(x).num().map_or(Val::Null, |n| Val::Num(-n)),
            Expr::List(items) => Val::List(items.iter().map(|i| self.eval(i)).collect()),
            Expr::Field(x, f) => field(&self.eval(x), f),
            Expr::Bin(op, a, b) => {
                match op {
                    Op::And => return Val::Bool(self.eval(a).truthy() && self.eval(b).truthy()),
                    Op::Or => {
                        let l = self.eval(a);
                        return if l.truthy() { l } else { self.eval(b) };
                    }
                    _ => {}
                }
                let (l, r) = (self.eval(a), self.eval(b));
                binop(*op, &l, &r)
            }
            Expr::Call(name, args) => self.call(name, args),
            Expr::Method(recv, name, args) => {
                // `file.hasTag(...)` and friends act on the row, not on a value.
                if let Expr::Name(p) = recv.as_ref() {
                    if p.len() == 1 && p[0] == "file" {
                        return self.file_method(name, args);
                    }
                }
                let v = self.eval(recv);
                let args: Vec<Val> = args.iter().map(|a| self.eval(a)).collect();
                method(&v, name, &args)
            }
        }
    }

    fn file_method(&mut self, name: &str, args: &[Expr]) -> Val {
        let args: Vec<String> = args.iter().map(|a| self.eval(a).to_string()).collect();
        let r = self.row;
        Val::Bool(match name {
            "hasTag" => args.iter().any(|t| has_tag(&r.tags, t)),
            "inFolder" => args.iter().any(|f| {
                let f = f.trim_matches('/');
                f.is_empty()
                    || r.path.len() > f.len()
                        && r.path.as_bytes()[f.len()] == b'/'
                        && r.path[..f.len()].eq_ignore_ascii_case(f)
            }),
            "hasLink" => args.iter().any(|l| {
                let want = link_name(l);
                let want = want.strip_suffix(".md").unwrap_or(want);
                r.links.iter().any(|x| {
                    let x = x.strip_suffix(".md").unwrap_or(x);
                    x.eq_ignore_ascii_case(want) || path::stem(x).eq_ignore_ascii_case(want)
                })
            }),
            "hasProperty" => args.iter().any(|k| r.props.iter().any(|(p, _)| p.eq_ignore_ascii_case(k))),
            _ => false,
        })
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Val {
        if name == "if" {
            let c = args.first().map(|a| self.eval(a)).unwrap_or(Val::Null);
            let pick = if c.truthy() { args.get(1) } else { args.get(2) };
            return pick.map_or(Val::Null, |e| self.eval(e));
        }
        let v: Vec<Val> = args.iter().map(|a| self.eval(a)).collect();
        let nums = || v.iter().flat_map(Val::items).filter_map(|x| x.num());
        match name {
            "now" => Val::Str(date_string(self.row.now, true)),
            "today" => Val::Str(date_string(self.row.now, false)),
            "date" => v.first().map_or(Val::Null, |d| Val::Str(d.to_string())),
            "number" => v.first().and_then(Val::num).map_or(Val::Null, Val::Num),
            "list" => Val::List(v.first().map(Val::items).unwrap_or_default()),
            "link" => v.first().map_or(Val::Null, |l| Val::Str(link_name(&l.to_string()).to_string())),
            "min" => nums().reduce(f64::min).map_or(Val::Null, Val::Num),
            "max" => nums().reduce(f64::max).map_or(Val::Null, Val::Num),
            _ => Val::Null,
        }
    }
}

fn field(v: &Val, f: &str) -> Val {
    match (v, f) {
        (Val::Str(s), "length") => Val::Num(s.chars().count() as f64),
        (Val::List(l), "length") => Val::Num(l.len() as f64),
        _ => Val::Null,
    }
}

fn binop(op: Op, l: &Val, r: &Val) -> Val {
    let cmp = || compare(l, r);
    match op {
        Op::Eq => Val::Bool(loose_eq(l, r)),
        Op::Ne => Val::Bool(!loose_eq(l, r)),
        Op::Lt | Op::Le | Op::Gt | Op::Ge if l.is_empty() || r.is_empty() => Val::Bool(false),
        Op::Lt => Val::Bool(cmp() == Ordering::Less),
        Op::Le => Val::Bool(cmp() != Ordering::Greater),
        Op::Gt => Val::Bool(cmp() == Ordering::Greater),
        Op::Ge => Val::Bool(cmp() != Ordering::Less),
        Op::Add => match (l.num(), r.num()) {
            (Some(a), Some(b)) if !matches!(l, Val::Str(_)) && !matches!(r, Val::Str(_)) => Val::Num(a + b),
            _ => match (l, r) {
                (Val::List(a), b) => Val::List(a.iter().cloned().chain(b.items()).collect()),
                _ => Val::Str(format!("{l}{r}")),
            },
        },
        Op::Sub | Op::Mul | Op::Div | Op::Rem => match (l.num(), r.num()) {
            (Some(a), Some(b)) => {
                let n = match op {
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div if b == 0.0 => return Val::Null,
                    Op::Div => a / b,
                    Op::Rem if b == 0.0 => return Val::Null,
                    _ => a % b,
                };
                Val::Num(n)
            }
            _ => Val::Null,
        },
        Op::And | Op::Or => unreachable!("short-circuited in eval"),
    }
}

fn method(v: &Val, name: &str, args: &[Val]) -> Val {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Val::Null);
    let text = || v.to_string();
    let contains = |x: &Val| match v {
        Val::List(l) => l.iter().any(|i| loose_eq(i, x)),
        _ => text().to_lowercase().contains(&x.to_string().to_lowercase()),
    };
    match name {
        "contains" => Val::Bool(contains(&arg(0))),
        "containsAny" => Val::Bool(args.iter().flat_map(Val::items).any(|x| contains(&x))),
        "containsAll" => Val::Bool(args.iter().flat_map(Val::items).all(|x| contains(&x))),
        "startsWith" => Val::Bool(text().to_lowercase().starts_with(&arg(0).to_string().to_lowercase())),
        "endsWith" => Val::Bool(text().to_lowercase().ends_with(&arg(0).to_string().to_lowercase())),
        "isEmpty" => Val::Bool(v.is_empty()),
        "lower" => Val::Str(text().to_lowercase()),
        "upper" => Val::Str(text().to_uppercase()),
        "trim" => Val::Str(text().trim().to_string()),
        "toString" => Val::Str(text()),
        "join" => {
            let sep = args.first().map_or(", ".to_string(), Val::to_string);
            Val::Str(v.items().iter().map(Val::to_string).collect::<Vec<_>>().join(&sep))
        }
        "at" => {
            let items = v.items();
            arg(0).num().and_then(|i| items.get(i as usize).cloned()).unwrap_or(Val::Null)
        }
        "round" | "toFixed" => match v.num() {
            Some(n) => {
                let d = arg(0).num().unwrap_or(0.0).clamp(0.0, 10.0) as i32;
                let p = 10f64.powi(d);
                let r = (n * p).round() / p;
                if name == "toFixed" {
                    Val::Str(format!("{r:.*}", d as usize))
                } else {
                    Val::Num(r)
                }
            }
            None => Val::Null,
        },
        "abs" => v.num().map_or(Val::Null, |n| Val::Num(n.abs())),
        _ => Val::Null,
    }
}

pub fn matches(f: &Filter, ctx: &mut Ctx) -> bool {
    match f {
        Filter::Expr(e) => ctx.eval(e).truthy(),
        Filter::And(fs) => fs.iter().all(|f| matches(f, ctx)),
        Filter::Or(fs) => fs.iter().any(|f| matches(f, ctx)),
        Filter::Not(fs) => !fs.iter().any(|f| matches(f, ctx)),
    }
}

/// One row of a view's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: String,
    pub cells: Vec<Val>,
    pub group: Val,
}

/// The rows of `view`: filtered by the base and view filters, sorted, grouped and limited.
/// `cols` is the column order to compute cells for.
pub fn run(base: &Base, view: &View, cols: &[String], rows: &[Row]) -> Vec<Item> {
    let mut out: Vec<(Item, Vec<Val>)> = Vec::new();
    for row in rows {
        let mut ctx = Ctx::new(base, row);
        if base.filter.as_ref().is_some_and(|f| !matches(f, &mut ctx)) {
            continue;
        }
        if view.filter.as_ref().is_some_and(|f| !matches(f, &mut ctx)) {
            continue;
        }
        let cells = cols.iter().map(|c| ctx.prop(c)).collect();
        let keys = view.sort.iter().map(|(p, _)| ctx.prop(p)).collect();
        let group = view.group_by.as_ref().map_or(Val::Null, |(g, _)| ctx.prop(g));
        out.push((Item { path: row.path.to_string(), cells, group }, keys));
    }
    out.sort_by(|(a, ka), (b, kb)| {
        let g = view.group_by.as_ref().map_or(Ordering::Equal, |(_, asc)| {
            let o = compare(&a.group, &b.group);
            if *asc || a.group.is_empty() || b.group.is_empty() {
                o
            } else {
                o.reverse()
            }
        });
        let keys =
            view.sort.iter().zip(ka.iter().zip(kb)).fold(Ordering::Equal, |acc, ((_, asc), (x, y))| {
                acc.then_with(|| {
                    let o = compare(x, y);
                    if *asc || x.is_empty() || y.is_empty() {
                        o
                    } else {
                        o.reverse()
                    }
                })
            });
        g.then(keys).then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    let mut items: Vec<Item> = out.into_iter().map(|(i, _)| i).collect();
    if let Some(n) = view.limit {
        items.truncate(n);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(path: &'a str, props: &'a [(String, Yaml)], tags: Vec<&'a str>) -> Row<'a> {
        Row { path, props, tags, links: vec!["Other.md"], mtime: 0, size: 10, now: 0 }
    }

    fn eval(src: &str, r: &Row) -> Val {
        let base = Base::default();
        Ctx::new(&base, r).eval(&parse_expr(src).unwrap())
    }

    #[test]
    fn expressions() {
        let props = frontmatter::parse("status: done\nprice: 12\nqty: 3\ntags: [book]\n").unwrap();
        let r = row("Books/Dune.md", &props, vec!["book/scifi"]);
        assert_eq!(eval("price * qty + 1", &r), Val::Num(37.0));
        assert_eq!(eval("status == \"done\" && !(price < 10)", &r), Val::Bool(true));
        assert_eq!(eval("note.status != 'todo'", &r), Val::Bool(true));
        assert_eq!(eval("file.name", &r), Val::Str("Dune".into()));
        assert_eq!(eval("file.hasTag(\"book\")", &r), Val::Bool(true));
        assert_eq!(eval("file.inFolder(\"Books\")", &r), Val::Bool(true));
        assert_eq!(eval("file.inFolder(\"Book\")", &r), Val::Bool(false));
        assert_eq!(eval("file.hasLink(\"Other\")", &r), Val::Bool(true));
        assert_eq!(eval("if(price > 100, \"dear\", \"cheap\")", &r), Val::Str("cheap".into()));
        assert_eq!(eval("status.upper()", &r), Val::Str("DONE".into()));
        assert_eq!(eval("tags.contains(\"book\")", &r), Val::Bool(true));
        assert_eq!(eval("missing.isEmpty()", &r), Val::Bool(true));
        assert_eq!(eval("(price / 7).toFixed(2)", &r), Val::Str("1.71".into()));
        assert!(parse_expr("price >").is_err());
    }

    #[test]
    fn a_base_file_filters_sorts_and_limits() {
        let src = r#"filters:
  and:
    - file.hasTag("book")
    - 'status != "dropped"'
formulas:
  total: "price * qty"
properties:
  status:
    displayName: State
views:
  - type: table
    name: Books
    order:
      - file.name
      - note.status
      - formula.total
    sort:
      - property: formula.total
        direction: DESC
    limit: 2
  - type: cards
    name: Gallery
"#;
        let base = Base::parse(src).unwrap();
        assert_eq!(base.views.len(), 2);
        assert_eq!(base.views[1].kind, ViewKind::Cards);
        assert_eq!(base.label("status"), "State");
        let v = &base.views[0];
        assert_eq!(v.order, ["file.name", "status", "formula.total"]);

        let a = frontmatter::parse("status: reading\nprice: 10\nqty: 1\n").unwrap();
        let b = frontmatter::parse("status: done\nprice: 5\nqty: 4\n").unwrap();
        let c = frontmatter::parse("status: dropped\nprice: 99\nqty: 9\n").unwrap();
        let d = frontmatter::parse("status: done\nprice: 1\nqty: 1\n").unwrap();
        let rows = [
            row("A.md", &a, vec!["book"]),
            row("B.md", &b, vec!["book"]),
            row("C.md", &c, vec!["book"]),
            row("D.md", &d, vec!["book"]),
            row("E.md", &b, vec![]),
        ];
        let items = run(&base, v, &v.order, &rows);
        let names: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(names, ["B.md", "A.md"]);
        assert_eq!(items[0].cells, [Val::Str("B".into()), Val::Str("done".into()), Val::Num(20.0)]);
    }

    #[test]
    fn an_empty_file_shows_every_note() {
        let base = Base::parse("").unwrap();
        assert_eq!(base.views.len(), 1);
        assert!(base.filter.is_none());
    }
}
