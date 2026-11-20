//! Running a parsed query over the vault index.

use crate::query::{Cmp, Dir, Expr, Lit, Query, SortKey};
use crate::value::{loose_eq, order, partial_order, Date, Val};
use md_engine::frontmatter::Value as Yaml;
use std::cmp::Ordering;
use std::sync::Arc;
use vault_core::meta::NoteMeta;
use vault_core::{path, Vault};

/// Settings of one evaluation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Opts {
    /// Seconds east of UTC, used to turn `file.mtime` into a date. The app passes the local
    /// offset; a test leaves it at zero and compares in UTC.
    pub utc_offset: i64,
}

/// A query result: a header per column and a row per matching note.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Row>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// The note the row stands for.
    pub path: String,
    pub cells: Vec<Cell>,
}

/// A table cell. `link` is the vault path the cell opens when it is a link (`File` column and
/// `file.link`); its text is what the link shows.
#[derive(Clone, PartialEq, Debug)]
pub struct Cell {
    pub text: String,
    pub link: Option<String>,
}

/// The notes `q` matches, as a table.
pub fn table(vault: &Vault, q: &Query, opts: Opts) -> Table {
    let folder = q.from.as_deref().map(|f| path::normalize(f.trim()));
    let folder = folder.as_deref().filter(|f| !f.is_empty());
    let mut entries: Vec<Hit> = Vec::new();
    for (_, f) in vault.notes() {
        let p: &str = &f.path;
        if folder.is_some_and(|dir| !in_folder(p, dir)) {
            continue;
        }
        let meta = f.meta();
        let ctx = Ctx { path: p, mtime: f.mtime, meta: meta.as_deref() };
        if q.filter.as_ref().is_some_and(|e| !eval(e, &ctx, opts)) {
            continue;
        }
        let keys: Vec<Val> = q.sort.iter().map(|s| ctx.field(&s.field.0, opts)).collect();
        entries.push(Hit { path: p.to_string(), mtime: f.mtime, meta, keys });
    }
    entries.sort_by(|a, b| {
        for (k, key) in q.sort.iter().enumerate() {
            let o = sort_order(&a.keys[k], &b.keys[k], key);
            if o != Ordering::Equal {
                return o;
            }
        }
        // A stable order whatever the index order is, so the same vault gives the same table.
        a.path.to_lowercase().cmp(&b.path.to_lowercase())
    });
    let mut headers = Vec::with_capacity(q.columns.len() + 1);
    if !q.without_id {
        headers.push("File".to_string());
    }
    for c in &q.columns {
        headers.push(c.alias.clone().unwrap_or_else(|| c.field.0.clone()));
    }
    let rows = entries
        .iter()
        .map(|h| {
            let ctx = Ctx { path: &h.path, mtime: h.mtime, meta: h.meta.as_deref() };
            let mut cells = Vec::with_capacity(headers.len());
            if !q.without_id {
                cells.push(link_cell(&h.path));
            }
            for c in &q.columns {
                let name = &c.field.0;
                cells.push(if is_file(name, "link") {
                    link_cell(&h.path)
                } else {
                    Cell { text: ctx.field(name, opts).text(), link: None }
                });
            }
            Row { path: h.path.clone(), cells }
        })
        .collect();
    Table { headers, rows }
}

struct Hit {
    path: String,
    /// Nanoseconds since the epoch.
    mtime: i64,
    meta: Option<Arc<NoteMeta>>,
    /// One value per sort key.
    keys: Vec<Val>,
}

fn link_cell(path: &str) -> Cell {
    Cell { text: path::stem(path).to_string(), link: Some(path.to_string()) }
}

/// Empty values go last whichever direction is asked for.
fn sort_order(a: &Val, b: &Val, key: &SortKey) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let o = order(a, b);
            if key.dir == Dir::Desc {
                o.reverse()
            } else {
                o
            }
        }
    }
}

/// Whether `path` is in `folder` or one of its subfolders, without case.
fn in_folder(path: &str, folder: &str) -> bool {
    let p = path.to_lowercase();
    let dir = folder.to_lowercase();
    p.len() > dir.len() && p.starts_with(&dir) && p.as_bytes()[dir.len()] == b'/'
}

/// What a field name reads to in one note.
struct Ctx<'a> {
    path: &'a str,
    mtime: i64,
    meta: Option<&'a NoteMeta>,
}

fn is_file(name: &str, field: &str) -> bool {
    match name.split_once('.') {
        Some((head, rest)) => head.eq_ignore_ascii_case("file") && rest.eq_ignore_ascii_case(field),
        None => false,
    }
}

impl Ctx<'_> {
    fn field(&self, name: &str, opts: Opts) -> Val {
        if let Some((head, rest)) = name.split_once('.') {
            if head.eq_ignore_ascii_case("file") {
                return match rest.to_ascii_lowercase().as_str() {
                    "name" => Val::Str(path::stem(self.path).to_string()),
                    // Compared as text, the link reads as the name it shows.
                    "link" => Val::Str(path::stem(self.path).to_string()),
                    "mtime" => Val::Date(mtime_date(self.mtime, opts)),
                    _ => Val::Null,
                };
            }
        }
        let Some(meta) = self.meta else { return Val::Null };
        let mut parts = name.split('.');
        let Some(first) = parts.next() else { return Val::Null };
        let Some(mut value) = meta.property(first) else { return Val::Null };
        for part in parts {
            let Yaml::Map(entries) = value else { return Val::Null };
            match entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(part)) {
                Some((_, v)) => value = v,
                None => return Val::Null,
            }
        }
        Val::from_yaml(value)
    }
}

/// The day `mtime` (nanoseconds since the epoch) falls on, in the local zone.
fn mtime_date(mtime: i64, opts: Opts) -> Date {
    let secs = (mtime.max(0) / 1_000_000_000 + opts.utc_offset).max(0) as u64;
    let (year, month, day, ..) = vault_core::fsio::civil(secs);
    Date { year: year as i32, month, day }
}

fn eval(e: &Expr, ctx: &Ctx, opts: Opts) -> bool {
    match e {
        Expr::And(tests) => tests.iter().all(|t| eval(t, ctx, opts)),
        Expr::Cmp(field, op, lit) => compare(&ctx.field(&field.0, opts), *op, lit),
    }
}

fn compare(v: &Val, op: Cmp, lit: &Lit) -> bool {
    let want = Val::from(lit);
    match (op, lit) {
        (Cmp::Eq, Lit::Null) => v.is_empty(),
        (Cmp::Ne, Lit::Null) => !v.is_empty(),
        (Cmp::Eq, _) => loose_eq(v, &want),
        (Cmp::Ne, _) => !loose_eq(v, &want),
        // A missing value is not greater or smaller than anything, and neither are values of
        // kinds that do not compare (`fecha >= date(…)` does not match text that is not a date).
        _ => match partial_order(v, &want) {
            Some(o) => match op {
                Cmp::Lt => o == Ordering::Less,
                Cmp::Le => o != Ordering::Greater,
                Cmp::Gt => o == Ordering::Greater,
                _ => o != Ordering::Less,
            },
            None => false,
        },
    }
}
