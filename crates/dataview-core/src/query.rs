//! The `TABLE` query language: parsing and the canonical form the parser reads back.

use crate::value::Date;
use std::fmt;

/// A parsed `TABLE` query.
#[derive(Clone, PartialEq, Debug)]
pub struct Query {
    /// `WITHOUT ID`: no link column in front of the named columns.
    pub without_id: bool,
    pub columns: Vec<Column>,
    /// Folder the notes come from; `None` is the whole vault.
    pub from: Option<String>,
    /// The `WHERE` tests, joined with `AND`.
    pub filter: Option<Expr>,
    /// Sort keys; the first one decides first.
    pub sort: Vec<SortKey>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Column {
    pub field: Field,
    /// Header text from `AS "…"`. Without it the header is the field as written.
    pub alias: Option<String>,
}

/// A front matter key, or a `file.` field, as written in the query.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Field(pub String);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SortKey {
    pub field: Field,
    pub dir: Dir,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Asc,
    Desc,
}

/// The right side of a `WHERE` comparison.
#[derive(Clone, PartialEq, Debug)]
pub enum Lit {
    Null,
    Str(String),
    Num(f64),
    Date(Date),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    Cmp(Field, Cmp, Lit),
    /// A chain of `AND` tests; one test is also an `And` of one.
    And(Vec<Expr>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    /// Byte offset in the query source.
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at {})", self.message, self.offset)
    }
}
impl std::error::Error for ParseError {}

pub fn parse(src: &str) -> Result<Query, ParseError> {
    let mut p = P { s: src, i: 0 };
    p.ws();
    p.keyword("TABLE")?;
    let mut q = Query { without_id: false, columns: Vec::new(), from: None, filter: None, sort: Vec::new() };
    if p.keyword_opt("WITHOUT") {
        p.keyword("ID")?;
        q.without_id = true;
    }
    loop {
        q.columns.push(p.column()?);
        if !p.punct(',') {
            break;
        }
    }
    if p.keyword_opt("FROM") {
        q.from = Some(p.string()?);
    }
    if p.keyword_opt("WHERE") {
        q.filter = Some(p.condition()?);
    }
    if p.keyword_opt("SORT") {
        q.sort = p.sorts()?;
    }
    p.ws();
    if !p.done() {
        return Err(p.err("unexpected text after the query"));
    }
    Ok(q)
}

struct P<'a> {
    s: &'a str,
    i: usize,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

impl P<'_> {
    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.i += c.len_utf8();
            } else {
                break;
            }
        }
    }
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    fn done(&self) -> bool {
        self.i >= self.s.len()
    }
    fn err(&self, m: &str) -> ParseError {
        ParseError { offset: self.i, message: m.into() }
    }

    /// The identifier at the cursor, without moving.
    fn word(&self) -> Option<&'_ str> {
        let mut end = self.i;
        for (k, c) in self.s[self.i..].char_indices() {
            if is_ident(c) && !(k == 0 && c == '-') {
                end = self.i + k + c.len_utf8();
            } else {
                break;
            }
        }
        (end > self.i).then(|| &self.s[self.i..end])
    }

    fn keyword_opt(&mut self, kw: &str) -> bool {
        self.ws();
        match self.word() {
            Some(w) if w.eq_ignore_ascii_case(kw) => {
                self.i += w.len();
                true
            }
            _ => false,
        }
    }
    fn keyword(&mut self, kw: &str) -> Result<(), ParseError> {
        if self.keyword_opt(kw) {
            Ok(())
        } else {
            Err(self.err(&format!("expected `{kw}`")))
        }
    }
    fn punct(&mut self, c: char) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.i += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn take(&mut self, s: &str) -> bool {
        self.ws();
        if self.s[self.i..].starts_with(s) {
            self.i += s.len();
            true
        } else {
            false
        }
    }

    fn field(&mut self) -> Result<Field, ParseError> {
        self.ws();
        let mut name = self.word().ok_or_else(|| self.err("expected a field name"))?.to_string();
        let mut at = self.i + name.len();
        while self.s[at..].starts_with('.') {
            self.i = at + 1;
            let part = self.word().ok_or_else(|| self.err("expected a field name after `.`"))?;
            name.push('.');
            name.push_str(part);
            at = self.i + part.len();
        }
        self.i = at;
        Ok(Field(name))
    }

    fn column(&mut self) -> Result<Column, ParseError> {
        let field = self.field()?;
        let alias = if self.keyword_opt("AS") { Some(self.string()?) } else { None };
        Ok(Column { field, alias })
    }

    /// A quoted string; `\"`, `\\`, `\n` and `\t` are the escapes.
    fn string(&mut self) -> Result<String, ParseError> {
        self.ws();
        let quote = match self.peek() {
            Some(c @ ('"' | '\'')) => c,
            _ => return Err(self.err("expected a quoted string")),
        };
        self.i += quote.len_utf8();
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(self.err("unterminated string")) };
            self.i += c.len_utf8();
            match c {
                '\\' => {
                    let Some(e) = self.peek() else { return Err(self.err("unterminated string")) };
                    self.i += e.len_utf8();
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        e => e,
                    });
                }
                c if c == quote => return Ok(out),
                c => out.push(c),
            }
        }
    }

    fn condition(&mut self) -> Result<Expr, ParseError> {
        let mut tests = vec![self.comparison()?];
        while self.keyword_opt("AND") {
            tests.push(self.comparison()?);
        }
        Ok(Expr::And(tests))
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let field = self.field()?;
        self.ws();
        let op = ["!=", ">=", "<=", "=", ">", "<"]
            .into_iter()
            .find(|s| self.s[self.i..].starts_with(s))
            .ok_or_else(|| self.err("expected `=`, `!=`, `<`, `<=`, `>` or `>=`"))?;
        self.i += op.len();
        let cmp = match op {
            "!=" => Cmp::Ne,
            ">=" => Cmp::Ge,
            "<=" => Cmp::Le,
            "=" => Cmp::Eq,
            ">" => Cmp::Gt,
            _ => Cmp::Lt,
        };
        Ok(Expr::Cmp(field, cmp, self.literal()?))
    }

    fn literal(&mut self) -> Result<Lit, ParseError> {
        self.ws();
        if matches!(self.peek(), Some('"' | '\'')) {
            return Ok(Lit::Str(self.string()?));
        }
        if self.keyword_opt("null") {
            return Ok(Lit::Null);
        }
        if self.word().is_some_and(|w| w.eq_ignore_ascii_case("date")) && self.take("date") {
            if !self.punct('(') {
                return Err(self.err("expected `(` after `date`"));
            }
            let start = self.i;
            let end = self.s[self.i..].find(')').map(|k| self.i + k);
            let inner = self.s.get(start..end.unwrap_or(self.s.len())).unwrap_or("");
            let date = Date::parse(inner).ok_or_else(|| ParseError {
                offset: start,
                message: "expected a date written YYYY-MM-DD".into(),
            })?;
            self.i = end.ok_or_else(|| self.err("unterminated `date(`"))? + 1;
            return Ok(Lit::Date(date));
        }
        let start = self.i;
        match self.number() {
            Some(n) => Ok(Lit::Num(n)),
            None => {
                self.i = start;
                Err(self.err("expected a string, a number, `null` or `date(…)`"))
            }
        }
    }

    /// A number at the cursor, exponent included.
    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        let mut any = self.digits();
        if self.peek() == Some('.') {
            self.i += 1;
            any |= self.digits();
        }
        if any && matches!(self.peek(), Some('e' | 'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.i += 1;
            }
            if !self.digits() {
                self.i = save;
            }
        }
        match (any, self.s[start..self.i].parse::<f64>()) {
            (true, Ok(n)) => Some(n),
            _ => None,
        }
    }

    fn digits(&mut self) -> bool {
        let from = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i > from
    }

    fn sorts(&mut self) -> Result<Vec<SortKey>, ParseError> {
        let mut out = Vec::new();
        loop {
            let field = self.field()?;
            let dir = if self.keyword_opt("DESC") {
                Dir::Desc
            } else {
                self.keyword_opt("ASC");
                Dir::Asc
            };
            out.push(SortKey { field, dir });
            if !self.punct(',') {
                return Ok(out);
            }
        }
    }
}

// ---- canonical form ----

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TABLE")?;
        if self.without_id {
            write!(f, " WITHOUT ID")?;
        }
        for (i, c) in self.columns.iter().enumerate() {
            f.write_str(if i == 0 { " " } else { ", " })?;
            write!(f, "{}", c.field)?;
            if let Some(alias) = &c.alias {
                write!(f, " AS \"{}\"", escape(alias))?;
            }
        }
        if let Some(from) = &self.from {
            write!(f, "\nFROM \"{}\"", escape(from))?;
        }
        if let Some(filter) = &self.filter {
            write!(f, "\nWHERE {filter}")?;
        }
        if !self.sort.is_empty() {
            f.write_str("\nSORT ")?;
            for (i, s) in self.sort.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{} {}", s.field, if s.dir == Dir::Asc { "ASC" } else { "DESC" })?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Cmp(field, op, lit) => write!(f, "{field} {} {lit}", op.as_str()),
            Expr::And(tests) => {
                for (i, t) in tests.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" AND ")?;
                    }
                    write!(f, "{t}")?;
                }
                Ok(())
            }
        }
    }
}

impl Cmp {
    fn as_str(self) -> &'static str {
        match self {
            Cmp::Eq => "=",
            Cmp::Ne => "!=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }
}

impl fmt::Display for Lit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lit::Null => f.write_str("null"),
            Lit::Str(s) => write!(f, "\"{}\"", escape(s)),
            Lit::Num(n) => f.write_str(&md_engine::frontmatter::fmt_number(*n)),
            Lit::Date(d) => write!(f, "date({d})"),
        }
    }
}

/// Write a string so that `P::string` reads it back unchanged.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> Field {
        Field(name.into())
    }

    #[test]
    fn a_table_query_parses() {
        let q = parse(
            "TABLE tipo AS \"Tipo\", unidad AS \"Unidad\", file.mtime AS \"Editado\"\n\
             FROM \"Facultad/Materias/1° Año/Análisis Matemático I\"\n\
             WHERE tipo != \"materia\"\n\
             SORT unidad ASC, file.name ASC\n",
        )
        .unwrap();
        assert!(!q.without_id);
        assert_eq!(q.columns.len(), 3);
        assert_eq!(q.columns[1], Column { field: f("unidad"), alias: Some("Unidad".into()) });
        assert_eq!(q.columns[2].field, f("file.mtime"));
        assert_eq!(q.from.as_deref(), Some("Facultad/Materias/1° Año/Análisis Matemático I"));
        assert_eq!(
            q.filter,
            Some(Expr::And(vec![Expr::Cmp(f("tipo"), Cmp::Ne, Lit::Str("materia".into()))]))
        );
        assert_eq!(
            q.sort,
            vec![
                SortKey { field: f("unidad"), dir: Dir::Asc },
                SortKey { field: f("file.name"), dir: Dir::Asc }
            ]
        );
    }

    #[test]
    fn without_id_null_tests_and_dates() {
        let q = parse(
            "table WITHOUT id anio, estado\n\
             where tipo = \"materia\" AND tipo != null\n\
             sort anio desc, file.name\n",
        )
        .unwrap();
        assert!(q.without_id);
        assert_eq!(q.from, None);
        assert_eq!(
            q.filter,
            Some(Expr::And(vec![
                Expr::Cmp(f("tipo"), Cmp::Eq, Lit::Str("materia".into())),
                Expr::Cmp(f("tipo"), Cmp::Ne, Lit::Null),
            ]))
        );
        assert_eq!(q.sort[0].dir, Dir::Desc);
        assert_eq!(q.sort[1].dir, Dir::Asc, "without ASC or DESC a key sorts ascending");

        let q = parse("TABLE fecha WHERE fecha >= date(2026-11-11)").unwrap();
        assert_eq!(
            q.filter,
            Some(Expr::And(vec![Expr::Cmp(
                f("fecha"),
                Cmp::Ge,
                Lit::Date(Date { year: 2026, month: 11, day: 11 })
            )]))
        );
        let q = parse("TABLE x WHERE n = -1.5 AND m < 2e3").unwrap();
        assert!(matches!(q.filter, Some(Expr::And(ref v))
            if v[0] == Expr::Cmp(f("n"), Cmp::Eq, Lit::Num(-1.5))
                && v[1] == Expr::Cmp(f("m"), Cmp::Lt, Lit::Num(2000.0))));
    }

    #[test]
    fn strings_keep_their_escapes() {
        let q = parse("TABLE x AS \"a \\\"b\\\" \\\\ c\"").unwrap();
        assert_eq!(q.columns[0].alias.as_deref(), Some("a \"b\" \\ c"));
    }

    #[test]
    fn errors_name_what_was_expected() {
        for (src, msg) in [
            ("", "expected `TABLE`"),
            ("TABLE", "expected a field name"),
            ("TABLE a WHERE b", "expected `=`, `!=`"),
            ("TABLE a WHERE b = ", "expected a string"),
            ("TABLE a WHERE b = \"x", "unterminated string"),
            ("TABLE a WHERE b >= date(2026-11)", "expected a date written YYYY-MM-DD"),
            ("TABLE a FROM x", "expected a quoted string"),
            ("TABLE a\nLIMIT 5", "unexpected text after the query"),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.message.starts_with(msg), "{src:?} gave {:?}", e.message);
        }
    }

    #[test]
    fn the_canonical_form_reads_back() {
        let src = "TABLE tipo AS \"Tipo\", unidad\nFROM \"Facultad\"\nWHERE tipo != null\nSORT unidad DESC";
        let q = parse(src).unwrap();
        assert_eq!(q.to_string(), src);
        assert_eq!(parse(&q.to_string()).unwrap(), q);
    }
}
