//! The pane that shows an open `.base` file.

use crate::bases::{self, Base, Item, Row, ViewKind};
use crate::shell::{App, Focus};
use rustsidian_ui::{Dom, El, NodeId};
use vault_core::fsio::{self, Stamp};
use vault_core::path;

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
                    self.notice = Some(format!("Cannot open {rel}: {e}"));
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
            dom.add(content, El::div().class("bases-error").text(format!("This base cannot be read: {e}")));
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
    let count = if items.len() == 1 { "1 result".to_string() } else { format!("{} results", items.len()) };
    dom.add(header, El::div().class("graph-count bases-count").text(count));
    let view = &base.views[v.view.min(base.views.len() - 1)];
    let shown = &items[..items.len().min(MAX_SHOWN)];
    let mut start = 0;
    while start < shown.len() {
        let group = &shown[start].group;
        let end = if view.group_by.is_some() {
            shown[start..].iter().position(|i| i.group != *group).map_or(shown.len(), |n| start + n)
        } else {
            shown.len()
        };
        if view.group_by.is_some() {
            let label = if group.to_string().is_empty() { "None".to_string() } else { group.to_string() };
            dom.add(
                content,
                El::div().class("bases-group-heading").text(format!("{label} ({})", end - start)),
            );
        }
        let rows = &shown[start..end];
        match view.kind {
            ViewKind::Table => table(base, cols, rows, dom, content),
            ViewKind::Cards => cards(base, cols, rows, dom, content),
            ViewKind::List => list(cols, rows, dom, content),
        }
        start = end;
    }
    if items.len() > MAX_SHOWN {
        dom.add(
            content,
            El::div().class("bases-more").text(format!("{} more not shown", items.len() - MAX_SHOWN)),
        );
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
            let card = dom.add(grid, El::div().class("bases-card").action(format!("open:{}", item.path)));
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
        }
        for _ in chunk.len()..CARDS_PER_ROW {
            dom.add(grid, El::div().class("bases-card-filler"));
        }
    }
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
