//! Editor suggestions at the cursor: `[[` offers notes to link (with a preview of the
//! selected one) and `/` offers commands.

use crate::shell::{App, Key, PromptItem, ViewMode, COMMANDS};
use editor_core::{EditKind, Sel};
use vault_core::{fsio, path};

const LIMIT: usize = 10;
const PREVIEW_LINES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestKind {
    Link,
    Slash,
}

pub struct Suggest {
    pub kind: SuggestKind,
    /// Start of the text the accepted item replaces: after `[[`, or at the `/`.
    pub start: usize,
    pub items: Vec<PromptItem>,
    pub selected: usize,
    /// Opening lines of the selected note, for link suggestions.
    pub preview: String,
}

/// The suggestion the text before the cursor asks for: its kind, replace start and query.
pub fn context(line: &str, line_start: usize) -> Option<(SuggestKind, usize, &str)> {
    if let Some(i) = line.rfind("[[") {
        let query = &line[i + 2..];
        if !query.contains([']', '|', '#', '^']) {
            return Some((SuggestKind::Link, line_start + i + 2, query));
        }
    }
    let i = line.rfind('/')?;
    let query = &line[i + 1..];
    let after_space = line[..i].chars().next_back().is_none_or(char::is_whitespace);
    (after_space && !query.contains(char::is_whitespace)).then_some((
        SuggestKind::Slash,
        line_start + i,
        query,
    ))
}

impl App {
    /// Open, refresh or close the suggestion after the editor text or cursor changed.
    pub(crate) fn update_suggest(&mut self) {
        let found = self
            .tabs
            .get(self.active)
            .filter(|t| t.mode != ViewMode::Reading && !self.show_graph && t.editor.buf.sels.len() == 1)
            .and_then(|t| {
                let sel = t.editor.buf.sels[0];
                let doc = t.editor.buf.doc();
                let in_code = doc
                    .block_index_at(sel.head)
                    .is_some_and(|i| matches!(doc.blocks[i].kind(), md_engine::NodeKind::CodeBlock { .. }));
                if !sel.is_empty() || in_code {
                    return None;
                }
                let line_start = t.editor.buf.line_start(t.editor.buf.line_of(sel.head));
                let line = t.editor.buf.slice(line_start, sel.head).into_owned();
                context(&line, line_start).map(|(k, s, q)| (k, s, q.to_string()))
            });
        let Some((kind, start, query)) = found.filter(|f| Some(f.1) != self.suggest_dismissed) else {
            self.suggest = None;
            return;
        };
        self.suggest_dismissed = None;
        let items = match kind {
            SuggestKind::Link => self.link_items(&query),
            SuggestKind::Slash => self.slash_items(&query),
        };
        if items.is_empty() {
            self.suggest = None;
            return;
        }
        let selected =
            self.suggest.as_ref().filter(|s| s.start == start && s.kind == kind).map_or(0, |s| s.selected);
        let selected = selected.min(items.len() - 1);
        self.suggest = Some(Suggest { kind, start, items, selected, preview: String::new() });
        self.load_preview();
    }

    fn link_items(&mut self, query: &str) -> Vec<PromptItem> {
        let paths: Vec<&str> = self.store.index.iter().map(|(_, f)| &*f.path).collect();
        self.fuzzy
            .top(query, &paths, LIMIT)
            .iter()
            .map(|h| {
                let p = paths[h.index];
                PromptItem {
                    label: path::stem(p).to_string(),
                    aux: path::parent(p).to_string(),
                    action: p.to_string(),
                }
            })
            .collect()
    }

    fn slash_items(&mut self, query: &str) -> Vec<PromptItem> {
        let names: Vec<&str> = COMMANDS.iter().map(|c| c.1).collect();
        self.fuzzy
            .top(query, &names, LIMIT)
            .iter()
            .map(|h| {
                let (id, name, _) = COMMANDS[h.index];
                PromptItem { label: name.to_string(), aux: self.hotkey_for(id), action: id.to_string() }
            })
            .collect()
    }

    fn load_preview(&mut self) {
        let Some(s) = &self.suggest else { return };
        let item = &s.items[s.selected];
        let text = match s.kind {
            SuggestKind::Link if path::extension(&item.action).eq_ignore_ascii_case("md") => {
                self.store.read(&item.action).map(|(t, _)| t).unwrap_or_default()
            }
            _ => String::new(),
        };
        let body = strip_front_matter(&text);
        let preview: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).take(PREVIEW_LINES).collect();
        if let Some(s) = &mut self.suggest {
            s.preview = preview.join("\n");
        }
    }

    /// Keys while a suggestion is showing. `None` lets the key through to the editor.
    pub(crate) fn suggest_key(&mut self, key: &Key) -> Option<bool> {
        let s = self.suggest.as_mut()?;
        if key.ctrl || key.alt {
            return None;
        }
        match key.key.as_str() {
            "ArrowDown" => s.selected = (s.selected + 1) % s.items.len(),
            "ArrowUp" => s.selected = (s.selected + s.items.len() - 1) % s.items.len(),
            "Enter" | "Tab" => {
                let i = s.selected;
                self.accept_suggest(i);
                return Some(true);
            }
            "Escape" => {
                self.suggest_dismissed = Some(s.start);
                self.suggest = None;
                return Some(true);
            }
            _ => return None,
        }
        self.load_preview();
        Some(true)
    }

    /// Replace the typed query with item `i`: a link, or (for `/`) run the command.
    pub(crate) fn accept_suggest(&mut self, i: usize) {
        let Some(s) = self.suggest.take() else { return };
        let Some(item) = s.items.get(i) else { return };
        let link = match s.kind {
            SuggestKind::Link => self.store.index.id(&item.action).map(|id| {
                let source = self.tabs.get(self.active).map_or("", |t| t.path.as_str());
                self.store.index.link_text(id, source, self.store.index.link_format, false)
            }),
            SuggestKind::Slash => None,
        };
        let now = fsio::now_ms();
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let head = t.editor.buf.sels[0].head;
        if head < s.start {
            return;
        }
        t.editor.buf.set_sels(vec![Sel::new(s.start, head)]);
        match link {
            Some(link) => {
                // Typing `[[` already paired the closing brackets.
                let closed = t.editor.buf.slice(head, (head + 2).min(t.editor.buf.len())) == "]]";
                let text = if closed { link } else { format!("{link}]]") };
                t.editor.replace_selections(&text, EditKind::Other, now);
                let at = t.editor.buf.sels[0].head + if closed { 2 } else { 0 };
                t.editor.buf.set_sels(vec![Sel::cursor(at)]);
            }
            None => t.editor.replace_selections("", EditKind::Other, now),
        }
        t.last_edit_ms = now;
        if s.kind == SuggestKind::Slash {
            let command = item.action.clone();
            self.run(&command);
        }
    }
}

fn strip_front_matter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return text };
    rest.find("\n---").map_or(text, |i| {
        let after = &rest[i + 4..];
        after.split_once('\n').map_or("", |(_, b)| b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts() {
        assert_eq!(context("see [[No", 10), Some((SuggestKind::Link, 16, "No")));
        assert_eq!(context("see [[No|x", 0), None);
        assert_eq!(context("/bo", 5), Some((SuggestKind::Slash, 5, "bo")));
        assert_eq!(context("a /bo", 0), Some((SuggestKind::Slash, 2, "bo")));
        assert_eq!(context("a/b", 0), None);
        assert_eq!(context("/a b", 0), None);
    }

    #[test]
    fn front_matter_is_skipped() {
        assert_eq!(strip_front_matter("---\ntags: [a]\n---\nbody\n"), "body\n");
        assert_eq!(strip_front_matter("plain"), "plain");
    }
}
