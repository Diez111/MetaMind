//! Bookmarks and tags panes of the left sidebar.

use crate::icons;
use crate::shell::{App, Panel};
use metamind_ui::{Dom, El, NodeId};
use serde_json::{json, Value};
use std::sync::Arc;
use vault_core::{fsio, path};

/// One visible row of the bookmarks pane; groups are flattened with a depth.
pub struct BookmarkRow {
    pub depth: usize,
    pub icon: &'static str,
    pub title: String,
    pub action: String,
    /// Indices into nested `items` arrays, joined with `.`, for removal.
    pub key: String,
}

/// Tag names with the number of notes using each, in name order.
pub struct TagCounts {
    graph: Arc<vault_core::LinkGraph>,
    pub tags: Vec<(String, usize)>,
}

fn rows_of(items: &[Value], depth: usize, prefix: &str, out: &mut Vec<BookmarkRow>) {
    for (i, it) in items.iter().enumerate() {
        let key = if prefix.is_empty() { i.to_string() } else { format!("{prefix}.{i}") };
        let s = |k: &str| it.get(k).and_then(Value::as_str).unwrap_or("");
        let title = |fallback: &str| {
            let t = s("title");
            if t.is_empty() {
                fallback.to_string()
            } else {
                t.to_string()
            }
        };
        let (icon, title, action) = match s("type") {
            "file" => {
                let p = s("path");
                let name = path::strip_md(p).rsplit('/').next().unwrap_or(p).to_string();
                (icons::for_file(p), title(&name), format!("open:{p}"))
            }
            "folder" => ("folder", title(s("path")), format!("bookmarks:reveal:{}", s("path"))),
            "search" => ("search", title(s("query")), format!("bookmarks:search:{}", s("query"))),
            "group" => ("bookmark", title("Grupo"), String::new()),
            // Kinds this app does not open (graph, url) are listed but inert.
            other => ("bookmark", title(other), String::new()),
        };
        out.push(BookmarkRow { depth, icon, title, action, key: key.clone() });
        if let Some(sub) = it.get("items").and_then(Value::as_array) {
            rows_of(sub, depth + 1, &key, out);
        }
    }
}

impl App {
    pub fn bookmark_rows(&self) -> Vec<BookmarkRow> {
        let mut out = Vec::new();
        if let Some(items) = self.config.bookmarks.value.get("items").and_then(Value::as_array) {
            rows_of(items, 0, "", &mut out);
        }
        out
    }

    /// Index of the top-level file bookmark for `p`.
    fn bookmark_of(&self, p: &str) -> Option<usize> {
        self.config.bookmarks.value.get("items")?.as_array()?.iter().position(|it| {
            it.get("type").and_then(Value::as_str) == Some("file")
                && it.get("path").and_then(Value::as_str).is_some_and(|x| x.eq_ignore_ascii_case(p))
        })
    }

    pub fn is_bookmarked(&self, p: &str) -> bool {
        self.bookmark_of(p).is_some()
    }

    fn bookmark_items(&mut self) -> &mut Vec<Value> {
        let v = &mut self.config.bookmarks.value;
        if !v.is_object() {
            *v = json!({});
        }
        let items = v.as_object_mut().unwrap().entry("items").or_insert_with(|| json!([]));
        if !items.is_array() {
            *items = json!([]);
        }
        items.as_array_mut().unwrap()
    }

    fn save_bookmarks(&mut self) {
        if let Err(e) = self.config.bookmarks.save() {
            self.notice = Some(format!("No se pudo guardar los marcadores: {e}"));
        }
    }

    /// Bookmark `p`, or remove its bookmark if it has one.
    pub fn toggle_bookmark(&mut self, p: &str) {
        match self.bookmark_of(p) {
            Some(i) => {
                self.bookmark_items().remove(i);
                self.notice = Some(format!("Se quitó el marcador de {}", path::strip_md(p)));
            }
            None => {
                let item = json!({"type": "file", "ctime": fsio::now_ms(), "path": p});
                self.bookmark_items().push(item);
                self.notice = Some(format!("Marcador añadido a {}", path::strip_md(p)));
            }
        }
        self.save_bookmarks();
    }

    /// Keep file and folder bookmarks pointing at moved paths.
    pub(crate) fn bookmarks_moved(&mut self, moves: &[(String, String)]) {
        fn walk(items: &mut [Value], moves: &[(String, String)]) -> bool {
            let mut changed = false;
            for it in items {
                if let Some(p) = it.get("path").and_then(Value::as_str) {
                    if let Some((_, new)) = moves.iter().find(|(old, _)| old.eq_ignore_ascii_case(p)) {
                        it["path"] = Value::String(new.clone());
                        changed = true;
                    }
                }
                if let Some(sub) = it.get_mut("items").and_then(Value::as_array_mut) {
                    changed |= walk(sub, moves);
                }
            }
            changed
        }
        let changed = match self.config.bookmarks.value.get_mut("items").and_then(Value::as_array_mut) {
            Some(items) => walk(items, moves),
            None => false,
        };
        if changed {
            self.save_bookmarks();
        }
    }

    pub(crate) fn tag_counts(&mut self) -> &[(String, usize)] {
        let Some(graph) = self.graph.clone() else { return &[] };
        if self.tags.as_ref().is_none_or(|t| !Arc::ptr_eq(&t.graph, &graph)) {
            let mut counts: std::collections::BTreeMap<String, (String, usize)> = Default::default();
            for (_, f) in self.store.index.notes() {
                let Some(m) = f.meta_parts(vault_core::cache::parts::TAGS) else { continue };
                for t in m.all_tags() {
                    counts.entry(t.to_lowercase()).or_insert_with(|| (t.to_string(), 0)).1 += 1;
                }
            }
            let tags = counts.into_values().collect();
            self.tags = Some(TagCounts { graph, tags });
        }
        &self.tags.as_ref().unwrap().tags
    }

    /// Handle `bookmarks:*` and `tags:*` commands.
    pub(crate) fn run_panes(&mut self, command: &str) -> bool {
        if let Some(q) = command.strip_prefix("bookmarks:search:") {
            self.search_query = q.to_string();
            self.run("global-search:open");
            self.run_search();
        } else if let Some(dir) = command.strip_prefix("bookmarks:reveal:") {
            let mut p = dir;
            while !p.is_empty() {
                self.collapsed.remove(p);
                p = path::parent(p);
            }
            self.tree = None;
            self.left_open = true;
            self.panel = Panel::Files;
        } else if let Some(key) = command.strip_prefix("bookmarks:remove:") {
            let idx: Vec<usize> = key.split('.').filter_map(|s| s.parse().ok()).collect();
            let mut items = self.bookmark_items();
            for (n, &i) in idx.iter().enumerate() {
                if i >= items.len() {
                    return true;
                }
                if n + 1 == idx.len() {
                    items.remove(i);
                    break;
                }
                match items[i].get_mut("items").and_then(Value::as_array_mut) {
                    Some(sub) => items = sub,
                    None => return true,
                }
            }
            self.save_bookmarks();
        } else if let Some(tag) = command.strip_prefix("tags:search:") {
            self.search_query = format!("tag:#{tag}");
            self.run("global-search:open");
            self.run_search();
        } else {
            match command {
                "bookmarks:open" => {
                    self.left_open = true;
                    self.panel = Panel::Bookmarks;
                }
                "tag-pane:open" => {
                    self.left_open = true;
                    self.panel = Panel::Tags;
                }
                "bookmarks:bookmark-current-view" => match self.tab().map(|t| t.path.clone()) {
                    Some(p) => self.toggle_bookmark(&p),
                    None => self.notice = Some("Abre una nota para añadirla a marcadores".into()),
                },
                "bookmarks:bookmark-current-search" => {
                    let q = self.search_query.trim().to_string();
                    if q.is_empty() {
                        self.notice = Some("Escribe primero una búsqueda".into());
                    } else {
                        let item = json!({"type": "search", "ctime": fsio::now_ms(), "query": q});
                        self.bookmark_items().push(item);
                        self.save_bookmarks();
                        self.notice = Some(format!("Búsqueda marcada: {q}"));
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

pub fn bookmarks(app: &App, dom: &mut Dom, parent: NodeId) {
    let header = dom.add(parent, El::div().class("nav-header"));
    dom.add(header, icons::el("bookmark"));
    dom.add(header, El::div().class("nav-header-title").text("Marcadores"));
    let b = dom.add(header, El::div().class("clickable-icon").action("bookmarks:bookmark-current-view"));
    dom.add(b, icons::el("plus"));
    let list = dom.add(parent, El::div().class("nav-files-container").id("bookmarks"));
    let rows = app.bookmark_rows();
    if rows.is_empty() {
        dom.add(
            list,
            El::div()
                .class("pane-empty")
                .text("Aún no hay marcadores. Añade una nota desde su menú o con +."),
        );
    }
    for r in rows {
        let el = El::div()
            .class("tree-item-self bookmark")
            .class_if(r.action.is_empty(), "is-group")
            .style(&format!("padding-left: {}px", 8 + r.depth * 14));
        let el = if r.action.is_empty() { el } else { el.action(r.action) };
        let row = dom.add(list, el);
        dom.add(row, icons::el(r.icon));
        dom.add(row, El::div().class("tree-item-inner").text(r.title));
        let x = dom.add(
            row,
            El::div().class("tree-item-flair clickable-icon").action(format!("bookmarks:remove:{}", r.key)),
        );
        dom.add(x, icons::el("x"));
    }
}

pub fn tags(app: &App, dom: &mut Dom, parent: NodeId) {
    let header = dom.add(parent, El::div().class("nav-header"));
    dom.add(header, icons::el("tag"));
    dom.add(header, El::div().class("nav-header-title").text("Etiquetas"));
    let list = dom.add(parent, El::div().class("nav-files-container").id("tags"));
    let tags = app.tags.as_ref().map(|t| t.tags.as_slice()).unwrap_or(&[]);
    if tags.is_empty() {
        dom.add(list, El::div().class("pane-empty").text("No hay etiquetas en esta bóveda."));
    }
    for (tag, n) in tags {
        // Nested tags (`a/b`) are indented under their parent's name.
        let depth = tag.matches('/').count();
        let leaf = tag.rsplit('/').next().unwrap_or(tag);
        let row = dom.add(
            list,
            El::div()
                .class("tree-item-self tag-pane-tag")
                .style(&format!("padding-left: {}px", 8 + depth * 14))
                .action(format!("tags:search:{tag}")),
        );
        dom.add(row, El::div().class("tree-item-inner").text(format!("#{leaf}")));
        dom.add(row, El::div().class("tree-item-flair").text(n.to_string()));
    }
}
