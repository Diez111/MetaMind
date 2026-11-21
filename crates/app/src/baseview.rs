//! The pane that shows an open `.base` file.

use crate::bases::{self, Base, Item, Row, View, ViewKind};
use crate::properties::{self, Kind};
use crate::shell::{App, Focus, PromptItem, PromptKind};
use md_engine::frontmatter::Value;
use rustsidian_ui::{Dom, El, NodeId};
use vault_core::fsio::{self, Stamp};
use vault_core::{path, WriteError};

/// Most rows drawn at once; the rest are counted, not built.
const MAX_SHOWN: usize = 500;
const CARDS_PER_ROW: usize = 3;

pub struct BaseView {
    pub path: String,
    pub stamp: Stamp,
    pub base: Result<Base, String>,
    pub view: usize,
    /// Columns and rows of the current view; `None` until computed after a change.
    pub items: Option<(Vec<String>, Vec<Item>)>,
}

impl BaseView {
    pub fn title(&self) -> String {
        path::stem(&self.path).to_string()
    }
}

impl App {
    pub fn base_active(&self) -> bool {
        self.show_base && !self.show_graph && self.base_view.is_some()
    }

    pub(crate) fn open_base(&mut self, rel: &str) {
        if self.base_view.as_ref().is_none_or(|b| !b.path.eq_ignore_ascii_case(rel)) {
            match self.store.read(rel) {
                Ok((text, stamp)) => {
                    self.base_view = Some(BaseView {
                        path: rel.to_string(),
                        stamp,
                        base: Base::parse(&text),
                        view: 0,
                        items: None,
                    })
                }
                Err(e) => {
                    self.notice = Some(format!("No se pudo abrir {rel}: {e}"));
                    return;
                }
            }
        }
        self.show_base = true;
        self.show_canvas = false;
        self.show_graph = false;
        self.focus = Focus::Editor;
    }

    /// Notes or the base file changed: recompute the rows on the next frame.
    pub(crate) fn base_files_changed(&mut self, paths: &[String]) {
        let Some(v) = self.base_view.as_mut() else { return };
        v.items = None;
        if paths.iter().any(|p| p.eq_ignore_ascii_case(&v.path)) {
            if let Ok((text, stamp)) = self.store.read(&v.path) {
                v.base = Base::parse(&text);
                v.stamp = stamp;
            }
        }
    }

    pub(crate) fn run_base(&mut self, command: &str) -> bool {
        if let Some(rest) = command.strip_prefix("base:edit:") {
            if let Some((rel, col)) = rest.split_once('|') {
                let current =
                    self.store.read(rel).ok().and_then(|(t, _)| cell_text(&t, &bases::canonical(col)));
                self.open_name_prompt(PromptKind::BaseCell, rest, &current.unwrap_or_default());
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("base:move:") {
            self.open_name_prompt(PromptKind::BaseCell, rest, "");
            return true;
        }
        if let Some(rest) = command.strip_prefix("base:set:") {
            let mut parts = rest.splitn(3, '|');
            if let (Some(rel), Some(col), Some(input)) = (parts.next(), parts.next(), parts.next()) {
                self.set_note_property(rel, &bases::canonical(col), input);
            }
            return true;
        }
        if let Some(i) = command.strip_prefix("base:view:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(v) = &mut self.base_view {
                v.view = i;
                v.items = None;
            }
            return true;
        }
        match command {
            "base:focus" => {
                self.show_base = self.base_view.is_some();
                self.show_canvas = false;
                self.show_graph = false;
            }
            "base:close" => {
                self.base_view = None;
                self.show_base = false;
            }
            _ => return false,
        }
        true
    }

    /// The prompt rows for the cell in `target` (`path|column`): the typed value, then the
    /// values the column already has in the current view.
    pub(crate) fn base_cell_items(&self, target: &str, query: &str) -> Vec<PromptItem> {
        let Some((rel, col)) = target.split_once('|') else { return Vec::new() };
        let key = bases::canonical(col);
        let value = query.trim();
        let set = |v: &str| PromptItem {
            label: format!("Establecer \u{201c}{key}\u{201d} en \u{201c}{v}\u{201d}"),
            aux: String::new(),
            action: format!("base:set:{target}|{v}"),
        };
        let lower = value.to_lowercase();
        let mut items: Vec<PromptItem> = self
            .base_column_values(col)
            .iter()
            .filter(|v| *v != value && v.to_lowercase().contains(&lower))
            .map(|v| set(v))
            .collect();
        if value.is_empty() {
            items.push(PromptItem {
                label: format!("Quitar \u{201c}{key}\u{201d} de {}", path::stem(rel)),
                aux: String::new(),
                action: format!("base:set:{target}|"),
            });
        } else {
            items.insert(0, set(value));
        }
        items
    }

    /// Distinct non-empty values of column `col` (or the group property) in the current view.
    fn base_column_values(&self, col: &str) -> Vec<String> {
        let Some(v) = &self.base_view else { return Vec::new() };
        let (Ok(base), Some((cols, items))) = (&v.base, &v.items) else { return Vec::new() };
        let key = bases::canonical(col);
        let at = cols.iter().position(|c| bases::canonical(c) == key);
        let grouped = base
            .views
            .get(v.view)
            .and_then(|view| view.group_by.as_ref())
            .is_some_and(|(g, _)| bases::canonical(g) == key);
        let mut out: Vec<String> = items
            .iter()
            .filter_map(|i| match at {
                Some(c) => Some(i.cells[c].to_string()),
                None if grouped => Some(i.group.to_string()),
                None => None,
            })
            .filter(|s| !s.is_empty())
            .collect();
        out.sort_unstable_by_key(|s| s.to_lowercase());
        out.dedup();
        out
    }

    /// Write property `key` of the note at `rel` on disk; an empty `input` removes it.
    fn set_note_property(&mut self, rel: &str, key: &str, input: &str) {
        // Unsaved edits in an open tab would be overwritten by the rewrite below.
        self.save_all();
        if self.all_tabs().any(|t| t.path.eq_ignore_ascii_case(rel) && t.editor.buf.is_modified()) {
            self.notice = Some(format!("No se pudo cambiar {rel}: sus cambios no se pudieron guardar"));
            return;
        }
        let (text, stamp) = match self.store.read(rel) {
            Ok(r) => r,
            Err(e) => {
                self.notice = Some(format!("No se pudo leer {rel}: {e}"));
                return;
            }
        };
        let out = match with_property(&text, key, input) {
            Ok(out) if out == text => return,
            Ok(out) => out,
            Err(e) => {
                self.notice = Some(e);
                return;
            }
        };
        match self.store.write(rel, &out, Some(stamp)) {
            Ok(_) => {
                if let (Some(c), Some(id)) = (&mut self.corpus, self.store.index.id(rel)) {
                    c.update(id, Some(&out));
                }
                self.reload_tabs(&[rel.to_string()]);
                self.files_changed(&[rel.to_string()]);
            }
            Err(WriteError::Conflict { .. }) => {
                self.notice = Some(format!("{rel} cambió en el disco; inténtalo de nuevo"))
            }
            Err(WriteError::Io(e)) => self.notice = Some(format!("No se pudo actualizar {rel}: {e}")),
        }
    }

    /// Compute the current view's rows if they are stale.
    pub(crate) fn base_refresh(&mut self) {
        let Some(v) = self.base_view.as_mut().filter(|v| v.items.is_none()) else { return };
        let Ok(base) = &v.base else { return };
        let Some(view) = base.views.get(v.view).or(base.views.first()) else { return };
        let cols = if view.order.is_empty() { vec!["file.name".to_string()] } else { view.order.clone() };
        let now = fsio::now_ms() / 1000;
        let metas: Vec<_> = self
            .store
            .index
            .notes()
            .filter_map(|(_, e)| Some((e.path.clone(), e.mtime, e.size, e.meta()?)))
            .collect();
        let rows: Vec<Row> = metas
            .iter()
            .map(|(p, mtime, size, m)| Row {
                path: p,
                props: m.frontmatter.as_deref().unwrap_or(&[]),
                tags: m.all_tags(),
                links: m.links.iter().map(|l| &*l.target).collect(),
                mtime: *mtime,
                size: *size,
                now,
            })
            .collect();
        let items = bases::run(base, view, &cols, &rows);
        v.items = Some((cols, items));
    }
}

/// The front matter value of `key` in `text` as the edit prompt shows it.
fn cell_text(text: &str, key: &str) -> Option<String> {
    let front = properties::scan(text)?;
    let v = &front.entries[front.index(key)?].value;
    Some(match v {
        Value::List(items) => items.iter().map(scalar_text).collect::<Vec<_>>().join(", "),
        v => scalar_text(v),
    })
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::Null | Value::List(_) | Value::Map(_) => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => md_engine::frontmatter::fmt_number(*n),
        Value::String(s) => s.clone(),
    }
}

/// `text` with property `key` set from `input`, keeping the type the property has: lists split
/// on commas, numbers and checkboxes must parse. An empty `input` removes the property.
pub fn with_property(text: &str, key: &str, input: &str) -> Result<String, String> {
    let input = input.trim();
    let front = properties::scan(text);
    let at = front.as_ref().and_then(|f| f.index(key));
    let old = front.as_ref().zip(at).map(|(f, i)| &f.entries[i].value);
    let change = if input.is_empty() {
        let (Some(f), Some(i)) = (&front, at) else { return Ok(text.to_string()) };
        f.remove(i)
    } else {
        let value = match properties::kind(key, old.unwrap_or(&Value::Null)) {
            Kind::List => Value::List(
                input
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.into()))
                    .collect(),
            ),
            Kind::Number => match input.parse::<f64>() {
                Ok(n) if n.is_finite() => Value::Number(n),
                _ => return Err(format!("\"{input}\" no es un número")),
            },
            Kind::Checkbox => match input.to_ascii_lowercase().as_str() {
                "true" | "yes" => Value::Bool(true),
                "false" | "no" => Value::Bool(false),
                _ => return Err(format!("\"{key}\" es una casilla: escribe true o false")),
            },
            Kind::Other => return Err(format!("\"{key}\" contiene valores anidados; edítalo en la nota")),
            Kind::Text | Kind::Date => Value::String(input.to_string()),
        };
        match (&front, at) {
            (Some(f), Some(i)) => f.set_value(i, &value),
            (Some(f), None) => f.add(key, &value),
            (None, _) if text.starts_with("---") => {
                return Err("No se puede leer el front matter de la nota; edítalo en la nota".to_string())
            }
            (None, _) => properties::create(key, &value),
        }
    };
    let mut out = text.to_string();
    out.replace_range(change.start..change.end, &change.text);
    Ok(out)
}

/// Columns that are note properties, which the table can edit.
fn editable(col: &str) -> bool {
    !col.starts_with("file.") && !col.starts_with("formula.")
}

fn file_name_cell(col: &str) -> bool {
    matches!(col, "file.name" | "file.basename" | "file.path")
}

pub fn pane(v: &BaseView, dom: &mut Dom, parent: NodeId) {
    let header = dom.add(parent, El::div().class("view-header graph-controls bases-header"));
    let content = dom.add(parent, El::div().class("view-content bases-view"));
    let base = match &v.base {
        Ok(b) => b,
        Err(e) => {
            dom.add(header, El::div().class("graph-count").text(v.path.clone()));
            dom.add(content, El::div().class("bases-error").text(format!("No se puede leer esta base: {e}")));
            return;
        }
    };
    for (i, view) in base.views.iter().enumerate() {
        let on = i == v.view.min(base.views.len() - 1);
        dom.add(
            header,
            El::div()
                .class("graph-control bases-view-tab")
                .class_if(on, "is-active")
                .text(view.name.clone())
                .action(format!("base:view:{i}")),
        );
    }
    let Some((cols, items)) = &v.items else { return };
    let count =
        if items.len() == 1 { "1 resultado".to_string() } else { format!("{} resultados", items.len()) };
    dom.add(header, El::div().class("graph-count bases-count").text(count));
    let view = &base.views[v.view.min(base.views.len() - 1)];
    let shown = &items[..items.len().min(MAX_SHOWN)];
    if view.kind == ViewKind::Kanban {
        kanban(base, view, cols, shown, dom, content);
    } else {
        groups(base, view, cols, shown, dom, content);
    }
    if items.len() > MAX_SHOWN {
        dom.add(
            content,
            El::div().class("bases-more").text(format!("{} más sin mostrar", items.len() - MAX_SHOWN)),
        );
    }
}

fn groups(base: &Base, view: &View, cols: &[String], shown: &[Item], dom: &mut Dom, content: NodeId) {
    let mut start = 0;
    while start < shown.len() {
        let group = &shown[start].group;
        let end = if view.group_by.is_some() {
            shown[start..].iter().position(|i| i.group != *group).map_or(shown.len(), |n| start + n)
        } else {
            shown.len()
        };
        if view.group_by.is_some() {
            let label = if group.to_string().is_empty() { "Ninguno".to_string() } else { group.to_string() };
            dom.add(
                content,
                El::div().class("bases-group-heading").text(format!("{label} ({})", end - start)),
            );
        }
        let rows = &shown[start..end];
        match view.kind {
            ViewKind::Kanban => unreachable!("kanban views return before the group loop"),
            ViewKind::Table => table(base, cols, rows, dom, content),
            ViewKind::Cards => cards(base, cols, rows, dom, content),
            ViewKind::List => list(cols, rows, dom, content),
        }
        start = end;
    }
}

/// A column per value of the group property, each holding its notes as cards.
fn kanban(base: &Base, view: &View, cols: &[String], shown: &[Item], dom: &mut Dom, content: NodeId) {
    let Some((group, _)) = &view.group_by else {
        dom.add(
            content,
            El::div().class("bases-error").text("Una vista kanban necesita una propiedad groupBy"),
        );
        return;
    };
    let movable = editable(group);
    let mut runs: Vec<&[Item]> = Vec::new();
    let mut start = 0;
    while start < shown.len() {
        let g = &shown[start].group;
        let end = shown[start..].iter().position(|i| i.group != *g).map_or(shown.len(), |n| start + n);
        runs.push(&shown[start..end]);
        start = end;
    }
    let board = dom.add(content, El::div().class("bases-kanban"));
    for run in runs {
        let label = run[0].group.to_string();
        let label = if label.is_empty() { "Ninguno".to_string() } else { label };
        let column = dom.add(board, El::div().class("bases-kanban-column"));
        dom.add(column, El::div().class("bases-kanban-heading").text(format!("{label} ({})", run.len())));
        for item in run {
            let c = card(base, cols, item, dom, column);
            if movable {
                dom.add(
                    c,
                    El::div()
                        .class("bases-card-move")
                        .text("Mover…")
                        .action(format!("base:move:{}|{group}", item.path)),
                );
            }
        }
    }
}

fn link_text(item: &Item, col: &str, i: usize) -> String {
    match col {
        "file.path" => item.path.clone(),
        _ => item.cells[i].to_string(),
    }
}

fn table(base: &Base, cols: &[String], rows: &[Item], dom: &mut Dom, parent: NodeId) {
    let grid = dom.add(parent, El::div().class("markdown-table bases-table"));
    let width = format!("width: {:.3}%", 100.0 / cols.len() as f32);
    let head = dom.add(grid, El::div().class("table-row is-header"));
    for c in cols {
        dom.add(head, El::div().class("table-cell").style(&width).text(base.label(c)));
    }
    for item in rows {
        let row = dom.add(grid, El::div().class("table-row"));
        for (i, c) in cols.iter().enumerate() {
            let cell = El::div().class("table-cell").style(&width);
            let cell = if file_name_cell(c) {
                cell.class("internal-link").text(link_text(item, c, i)).action(format!("open:{}", item.path))
            } else if editable(c) {
                cell.class("bases-editable")
                    .text(item.cells[i].to_string())
                    .action(format!("base:edit:{}|{c}", item.path))
            } else {
                cell.text(item.cells[i].to_string())
            };
            dom.add(row, cell);
        }
    }
}

fn cards(base: &Base, cols: &[String], rows: &[Item], dom: &mut Dom, parent: NodeId) {
    // The layout engine does not wrap flex lines, so cards go in rows of a fixed count.
    for chunk in rows.chunks(CARDS_PER_ROW) {
        let grid = dom.add(parent, El::div().class("bases-cards"));
        for item in chunk {
            card(base, cols, item, dom, grid);
        }
        for _ in chunk.len()..CARDS_PER_ROW {
            dom.add(grid, El::div().class("bases-card-filler"));
        }
    }
}

/// A card for `item`: its title and the non-empty columns other than the file name.
fn card(base: &Base, cols: &[String], item: &Item, dom: &mut Dom, parent: NodeId) -> NodeId {
    let card = dom.add(parent, El::div().class("bases-card").action(format!("open:{}", item.path)));
    dom.add(card, El::div().class("bases-card-title").text(path::stem(&item.path).to_string()));
    for (i, c) in cols.iter().enumerate().filter(|(_, c)| !file_name_cell(c)) {
        let v = item.cells[i].to_string();
        if v.is_empty() {
            continue;
        }
        let line = dom.add(card, El::div().class("bases-card-property"));
        dom.add(line, El::div().class("bases-card-label").text(base.label(c)));
        dom.add(line, El::div().class("bases-card-value").text(v));
    }
    card
}

fn list(cols: &[String], rows: &[Item], dom: &mut Dom, parent: NodeId) {
    let ul = dom.add(parent, El::div().class("bases-list"));
    for item in rows {
        let extra: Vec<String> = cols
            .iter()
            .enumerate()
            .filter(|(_, c)| !file_name_cell(c))
            .map(|(i, _)| item.cells[i].to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let li = dom.add(ul, El::div().class("bases-list-item"));
        dom.add(
            li,
            El::div()
                .class("internal-link")
                .text(path::stem(&item.path).to_string())
                .action(format!("open:{}", item.path)),
        );
        if !extra.is_empty() {
            dom.add(li, El::div().class("bases-list-extra").text(extra.join(" · ")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::with_property;

    #[test]
    fn set_keeps_the_property_type() {
        let t = "---\nstatus: done\nprice: 5\ntags: [a]\nok: true\n---\nbody\n";
        assert_eq!(with_property(t, "status", "open").unwrap(), t.replace("done", "open"));
        assert!(with_property(t, "price", "7").unwrap().contains("price: 7\n"));
        assert!(with_property(t, "price", "lots").is_err());
        assert!(with_property(t, "ok", "no").unwrap().contains("ok: false\n"));
        let tags = with_property(t, "tags", "a, b").unwrap();
        assert!(!tags.contains("[a]") && tags.contains("b"), "{tags}");
    }

    #[test]
    fn adds_removes_and_creates_front_matter() {
        let t = "---\nstatus: done\n---\nbody\n";
        assert_eq!(with_property(t, "status", " ").unwrap(), "---\n---\nbody\n");
        assert_eq!(with_property(t, "owner", "me").unwrap(), "---\nstatus: done\nowner: me\n---\nbody\n");
        assert_eq!(with_property("body\n", "owner", "me").unwrap(), "---\nowner: me\n---\nbody\n");
        assert_eq!(with_property("body\n", "owner", "").unwrap(), "body\n");
    }
}
