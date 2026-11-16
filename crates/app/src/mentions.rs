//! Unlinked mentions: other notes that name the active note in plain text.

use crate::shell::App;
use search_core::engine::lower_same_len;
use search_core::{snippet, Corpus, Match};
use std::sync::Arc;
use vault_core::{path, FileId, LinkGraph, WriteError};

/// At most this many notes are listed, so a common name cannot flood the pane.
const LIMIT: usize = 50;

pub struct Mentions {
    graph: Arc<LinkGraph>,
    path: String,
    /// Notes with mentions, with up to three snippets each.
    pub files: Vec<(String, Vec<String>)>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte ranges where `name` occurs in `lower` as a whole word, outside front matter, wikilinks
/// and Markdown links. Both are lowercased with `lower_same_len`.
pub fn find(lower: &str, name: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if name.trim().is_empty() {
        return out;
    }
    let body = lower.strip_prefix("---\n").and_then(|r| r.find("\n---")).map_or(0, |i| 4 + i + 4);
    let mut from = body;
    while let Some(i) = lower[from..].find(name) {
        let (s, e) = (from + i, from + i + name.len());
        from = s + name.len().max(1);
        let before = lower[..s].chars().next_back();
        let after = lower[e..].chars().next();
        if before.is_some_and(is_word) || after.is_some_and(is_word) {
            continue;
        }
        let line = &lower[lower[..s].rfind('\n').map_or(0, |p| p + 1)..s];
        let in_wikilink = line.rfind("[[").is_some_and(|o| !line[o..].contains("]]"));
        let in_mdlink = line.rfind('[').is_some_and(|o| !line[o..].contains(']'))
            && lower[e..].split('\n').next().is_some_and(|rest| rest.contains("]("));
        let in_target = line.rfind("](").is_some_and(|o| !line[o..].contains(')'));
        if !in_wikilink && !in_mdlink && !in_target {
            out.push((s, e));
        }
    }
    out
}

impl App {
    /// The active note's file name and aliases, lowercased for matching.
    fn mention_names(&self) -> Vec<String> {
        let Some(t) = self.tabs.get(self.active) else { return Vec::new() };
        let mut names = vec![path::stem(&t.path).to_string()];
        if let Some((_, m)) = &t.meta {
            names.extend(m.aliases.iter().map(|a| a.to_string()));
        }
        let mut names: Vec<String> = names.iter().map(|n| lower_same_len(n.trim())).collect();
        names.retain(|n| !n.is_empty());
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Recompute unlinked mentions of the active note when the vault or the note changed.
    pub(crate) fn refresh_mentions(&mut self) {
        let Some(graph) = self.graph.clone() else { return };
        let Some(rel) = self.tabs.get(self.active).map(|t| t.path.clone()) else { return };
        let fresh = self.mentions.as_ref().is_some_and(|m| Arc::ptr_eq(&m.graph, &graph) && m.path == rel);
        if fresh {
            return;
        }
        let names = self.mention_names();
        let me = self.store.index.id(&rel);
        let linking: Vec<FileId> = me.and_then(|id| graph.back.get(id as usize)).cloned().unwrap_or_default();
        let root = self.store.root.clone();
        // Built on first use, shared with search.
        let corpus = self.corpus.get_or_insert_with(|| Corpus::build(&self.store.index, &root));
        let mut files = Vec::new();
        for (id, f) in self.store.index.notes() {
            if Some(id) == me || linking.contains(&id) {
                continue;
            }
            let Some(lower) = corpus.lower(id) else { continue };
            let mut hits: Vec<(usize, usize)> = names.iter().flat_map(|n| find(lower, n)).collect();
            if hits.is_empty() {
                continue;
            }
            hits.sort_unstable();
            let p = f.path.to_string();
            let text = std::fs::read_to_string(root.join(&p)).unwrap_or_default();
            let snippets = hits
                .iter()
                .take(3)
                .map(|&(s, e)| {
                    snippet(&text, Match { start: s as u32, end: e as u32 }, 40).0.trim().to_string()
                })
                .collect();
            files.push((p, snippets));
            if files.len() == LIMIT {
                break;
            }
        }
        files.sort_unstable_by_key(|f| f.0.to_lowercase());
        self.mentions = Some(Mentions { graph, path: rel, files });
    }

    /// Recompute the notes linking to the active note, with a snippet around each link.
    pub(crate) fn refresh_linked(&mut self) {
        let Some(graph) = self.graph.clone() else { return };
        let Some(rel) = self.tabs.get(self.active).map(|t| t.path.clone()) else { return };
        if self.linked.as_ref().is_some_and(|m| Arc::ptr_eq(&m.graph, &graph) && m.path == rel) {
            return;
        }
        let Some(me) = self.store.index.id(&rel) else { return };
        let context = if self.backlinks_more { 2000 } else { 40 };
        let mut files = Vec::new();
        for &src in graph.back.get(me as usize).map(Vec::as_slice).unwrap_or(&[]) {
            let Some(f) = self.store.index.entry(src) else { continue };
            let p = f.path.to_string();
            let links = f.meta().map(|m| m.links.clone()).unwrap_or_default();
            let text = match std::fs::read_to_string(self.store.root.join(&p)) {
                Ok(t) => t,
                Err(e) => {
                    files.push((p, vec![format!("Cannot read: {e}")]));
                    continue;
                }
            };
            let snippets = links
                .iter()
                .filter(|l| self.store.index.resolve(&l.target, &p) == Some(me))
                .filter(|l| text.is_char_boundary(l.start as usize) && text.is_char_boundary(l.end as usize))
                .take(3)
                .map(|l| snippet(&text, Match { start: l.start, end: l.end }, context).0.trim().to_string())
                .collect();
            files.push((p, snippets));
        }
        files.sort_unstable_by_key(|f| f.0.to_lowercase());
        self.linked = Some(Mentions { graph, path: rel, files });
    }

    pub(crate) fn backlinks_text(&mut self, text: &str) -> bool {
        match &mut self.backlinks_filter {
            Some(f) if self.backlinks_editing => {
                f.push_str(text);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn backlinks_key(&mut self, key: &crate::shell::Key) -> Option<bool> {
        let f = self.backlinks_filter.as_mut().filter(|_| self.backlinks_editing)?;
        match key.key.as_str() {
            "Backspace" if key.ctrl => f.clear(),
            "Backspace" => {
                f.pop();
            }
            "Enter" | "Escape" | "Tab" => self.backlinks_editing = false,
            _ if key.ctrl || key.alt => return None,
            _ => {}
        }
        Some(true)
    }

    /// Turn every unlinked mention of the active note in `rel` into a link.
    pub(crate) fn link_mentions(&mut self, rel: &str) {
        let Some(target) = self.tab().map(|t| t.path.clone()) else { return };
        let Some(id) = self.store.index.id(&target) else { return };
        // Unsaved edits in an open tab would be overwritten by the rewrite below.
        if let Some(i) = self.tabs.iter().position(|t| t.path.eq_ignore_ascii_case(rel)) {
            let active = self.active;
            self.active = i;
            let saved = self.save();
            self.active = active;
            if !saved {
                return;
            }
        }
        let (text, stamp) = match self.store.read(rel) {
            Ok(r) => r,
            Err(e) => {
                self.notice = Some(format!("Cannot read {rel}: {e}"));
                return;
            }
        };
        let lower = lower_same_len(&text);
        let mut hits: Vec<(usize, usize)> =
            self.mention_names().iter().flat_map(|n| find(&lower, n)).collect();
        hits.sort_unstable();
        hits.dedup_by(|b, a| b.0 < a.1);
        let link = self.store.index.link_text(id, rel, self.store.index.link_format, false);
        let mut out = text.clone();
        for &(s, e) in hits.iter().rev() {
            let shown = &text[s..e];
            let rep = if shown == link { format!("[[{link}]]") } else { format!("[[{link}|{shown}]]") };
            out.replace_range(s..e, &rep);
        }
        match self.store.write(rel, &out, Some(stamp)) {
            Ok(_) => {
                if let (Some(c), Some(fid)) = (&mut self.corpus, self.store.index.id(rel)) {
                    c.update(fid, Some(&out));
                }
                self.files_changed(&[rel.to_string()]);
                // The write reindexed the note, so `files_changed` sees no change to reload.
                self.reload_tabs(&[rel.to_string()]);
                self.mentions = None;
                self.notice = Some(format!("Linked {} mention(s) in {}", hits.len(), path::strip_md(rel)));
            }
            Err(WriteError::Conflict { .. }) => {
                self.notice = Some(format!("{rel} changed on disk; try again"));
            }
            Err(WriteError::Io(e)) => self.notice = Some(format!("Cannot update {rel}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn finds_whole_words_outside_links() {
        let t = "---\nalias: plan\n---\nthe plan, [[plan]] and [the plan](plan.md) planning plan\n";
        let hits: Vec<&str> = find(t, "plan").iter().map(|&(s, e)| &t[s..e]).collect();
        assert_eq!(hits.len(), 2);
        assert_eq!(find(t, "plan")[0].0, t.find("the plan").unwrap() + 4);
        assert!(find(t, "").is_empty());
    }
}
