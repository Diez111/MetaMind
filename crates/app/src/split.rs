//! Split panes: the editor area as a tree of panes, each with its own tabs.
//!
//! The focused pane's tabs live in `App::tabs` and `App::active`, so code that works on "the
//! current tab" needs no knowledge of panes; the other panes keep theirs in `App::panes`.

use crate::shell::{App, Focus, Tab};
use editor_core::{Change, EditKind};
use serde_json::{json, Value};
use vault_core::fsio;

#[derive(Default)]
pub struct Pane {
    /// Empty for the focused pane, whose tabs are in `App::tabs`.
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// Panes in the same link group follow each other's active note.
    pub link: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Leaf(usize),
    /// `row`: children side by side, else stacked. A split always has at least two children,
    /// and their ratios sum to 1.
    Split {
        row: bool,
        kids: Vec<(Layout, f32)>,
    },
}

/// A divider being dragged: the split at `path` (child indices from the root), between its
/// children `k` and `k + 1`, laid out in `rect`.
pub struct PaneDrag {
    pub path: Vec<usize>,
    pub k: usize,
    pub rect: rustsidian_ui::Rect,
}

fn insert_beside(node: &mut Layout, cur: usize, new: usize, row: bool) -> bool {
    match node {
        Layout::Leaf(i) if *i == cur => {
            *node = Layout::Split { row, kids: vec![(Layout::Leaf(cur), 0.5), (Layout::Leaf(new), 0.5)] };
            true
        }
        Layout::Leaf(_) => false,
        Layout::Split { row: r, kids } => {
            if *r == row {
                if let Some(k) = kids.iter().position(|(c, _)| *c == Layout::Leaf(cur)) {
                    let half = kids[k].1 / 2.0;
                    kids[k].1 = half;
                    kids.insert(k + 1, (Layout::Leaf(new), half));
                    return true;
                }
            }
            kids.iter_mut().any(|(c, _)| insert_beside(c, cur, new, row))
        }
    }
}

fn first_leaf(node: &Layout) -> usize {
    match node {
        Layout::Leaf(i) => *i,
        Layout::Split { kids, .. } => first_leaf(&kids[0].0),
    }
}

/// Remove leaf `i`, giving its space to a neighbour. Returns a leaf of that neighbour.
fn remove_leaf(node: &mut Layout, i: usize) -> Option<usize> {
    let Layout::Split { kids, .. } = node else { return None };
    if let Some(k) = kids.iter().position(|(c, _)| *c == Layout::Leaf(i)) {
        let (_, r) = kids.remove(k);
        let n = k.min(kids.len() - 1);
        kids[n].1 += r;
        let near = first_leaf(&kids[n].0);
        if kids.len() == 1 {
            *node = kids.pop().unwrap().0;
        }
        return Some(near);
    }
    kids.iter_mut().find_map(|(c, _)| remove_leaf(c, i))
}

fn renumber(node: &mut Layout, removed: usize) {
    match node {
        Layout::Leaf(i) if *i > removed => *i -= 1,
        Layout::Leaf(_) => {}
        Layout::Split { kids, .. } => kids.iter_mut().for_each(|(c, _)| renumber(c, removed)),
    }
}

fn node_at_mut<'a>(node: &'a mut Layout, path: &[usize]) -> Option<&'a mut Layout> {
    match (path.split_first(), node) {
        (None, node) => Some(node),
        (Some((&k, rest)), Layout::Split { kids, .. }) => node_at_mut(&mut kids.get_mut(k)?.0, rest),
        (Some(_), Layout::Leaf(_)) => None,
    }
}

fn leaves(node: &Layout, out: &mut Vec<usize>) {
    match node {
        Layout::Leaf(i) => out.push(*i),
        Layout::Split { kids, .. } => kids.iter().for_each(|(c, _)| leaves(c, out)),
    }
}

pub(crate) fn layout_json(node: &Layout) -> Value {
    match node {
        Layout::Leaf(i) => json!(i),
        Layout::Split { row, kids } => json!({
            "row": row,
            "kids": kids.iter().map(|(c, r)| json!([layout_json(c), r])).collect::<Vec<_>>(),
        }),
    }
}

fn parse_node(v: &Value) -> Option<Layout> {
    if let Some(i) = v.as_u64() {
        return Some(Layout::Leaf(i as usize));
    }
    let row = v.get("row")?.as_bool()?;
    let kids = v
        .get("kids")?
        .as_array()?
        .iter()
        .map(|k| {
            let [c, r] = k.as_array()?.as_slice() else { return None };
            Some((parse_node(c)?, r.as_f64().filter(|r| *r > 0.0)? as f32))
        })
        .collect::<Option<Vec<_>>>()?;
    (kids.len() >= 2).then_some(Layout::Split { row, kids })
}

/// A saved layout over panes `0..n`, if it uses each of them exactly once.
pub(crate) fn parse_layout(v: &Value, n: usize) -> Option<Layout> {
    let layout = parse_node(v)?;
    let mut seen = Vec::new();
    leaves(&layout, &mut seen);
    seen.sort_unstable();
    (seen == (0..n).collect::<Vec<_>>()).then_some(layout)
}

/// Bring `tab` to `text` with one edit covering the part that differs, so its cursor, folds
/// and undo history stay usable.
fn mirror_into(tab: &mut Tab, text: &str, now: u64) {
    let cur = tab.editor.buf.text();
    if cur == text {
        return;
    }
    let (a, b) = (cur.as_bytes(), text.as_bytes());
    let mut pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    while !cur.is_char_boundary(pre) || !text.is_char_boundary(pre) {
        pre -= 1;
    }
    let room = a.len().min(b.len()) - pre;
    let mut suf = a.iter().rev().zip(b.iter().rev()).take(room).take_while(|(x, y)| x == y).count();
    while !cur.is_char_boundary(a.len() - suf) || !text.is_char_boundary(b.len() - suf) {
        suf -= 1;
    }
    let change = Change { start: pre, end: a.len() - suf, text: text[pre..b.len() - suf].to_string() };
    tab.editor.buf.apply(vec![change], None, EditKind::Typing, now);
    tab.refresh_folds();
}

impl App {
    /// The tabs and active tab index of pane `i`.
    pub fn pane_tabs(&self, i: usize) -> (&[Tab], usize) {
        if i == self.pane {
            (&self.tabs, self.active)
        } else {
            (&self.panes[i].tabs, self.panes[i].active)
        }
    }

    pub fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().chain(self.panes.iter().flat_map(|p| p.tabs.iter()))
    }

    pub fn all_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut().chain(self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()))
    }

    /// Keep only the tabs, in every pane, for which `keep` holds.
    pub fn retain_tabs(&mut self, keep: impl Fn(&Tab) -> bool) {
        self.tabs.retain(&keep);
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        for p in &mut self.panes {
            p.tabs.retain(&keep);
            p.active = p.active.min(p.tabs.len().saturating_sub(1));
        }
    }

    fn swap_pane(&mut self, i: usize) {
        let old = self.pane;
        self.panes[old].tabs = std::mem::take(&mut self.tabs);
        self.panes[old].active = self.active;
        self.tabs = std::mem::take(&mut self.panes[i].tabs);
        self.active = self.panes[i].active;
        self.pane = i;
    }

    pub fn focus_pane(&mut self, i: usize) {
        if i == self.pane || i >= self.panes.len() {
            return;
        }
        self.swap_pane(i);
        self.show_graph = false;
        self.show_canvas = false;
        self.show_base = false;
        self.focus = Focus::Editor;
    }

    /// Run `f` with pane `i` holding `App::tabs`, then put the focused pane back.
    pub(crate) fn in_pane<R>(&mut self, i: usize, f: impl FnOnce(&mut App) -> R) -> R {
        let back = self.pane;
        self.swap_pane(i);
        let r = f(self);
        self.swap_pane(back);
        r
    }

    /// Open the active note in a new pane to the right of (`row`) or below the focused one.
    pub fn split(&mut self, row: bool) {
        let Some(t) = self.tab() else {
            self.notice = Some("Open a note to split".to_string());
            return;
        };
        let mut tab = Tab::new(t.path.clone(), &t.editor.buf.text(), t.stamp, t.vim.is_some());
        tab.mode = t.mode;
        tab.top_block = t.top_block;
        let sels = t.editor.buf.sels.clone();
        let new = self.panes.len();
        self.panes.push(Pane::default());
        insert_beside(&mut self.layout, self.pane, new, row);
        self.focus_pane(new);
        self.tabs.push(tab);
        self.active = 0;
        self.init_new_tab();
        if let Some(t) = self.tabs.last_mut() {
            t.editor.buf.set_sels(sels);
        }
    }

    /// Close panes left without tabs, while another pane remains.
    fn prune_panes(&mut self) {
        while self.panes.len() > 1 {
            let Some(i) = (0..self.panes.len()).find(|&i| self.pane_tabs(i).0.is_empty()) else { break };
            let near = remove_leaf(&mut self.layout, i).unwrap_or(0);
            if self.pane == i {
                self.focus_pane(near);
            }
            self.panes.remove(i);
            renumber(&mut self.layout, i);
            if self.pane > i {
                self.pane -= 1;
            }
        }
    }

    /// Link the focused pane with pane `j`: from then on each follows the other's active note.
    pub fn link_pane(&mut self, j: usize) {
        if j >= self.panes.len() || j == self.pane {
            return;
        }
        let next = self.panes.iter().filter_map(|p| p.link).max().map_or(0, |m| m + 1);
        let group = self.panes[j].link.or(self.panes[self.pane].link).unwrap_or(next);
        self.panes[j].link = Some(group);
        self.panes[self.pane].link = Some(group);
        self.followed = None;
    }

    pub fn unlink_pane(&mut self) {
        let p = self.pane;
        self.panes[p].link = None;
    }

    /// Show `rel` in the active tab of the focused pane, replacing the note there unless pinned.
    fn show_in_tab(&mut self, rel: &str) {
        let old = self.active;
        let replace = self.tabs.get(old).is_some_and(|t| !t.pinned);
        let before = self.tabs.len();
        self.open_note_inner(rel, false);
        if replace && self.tabs.len() > before {
            self.close_tab(old);
        }
    }

    fn follow_links(&mut self) {
        let cur = self.tab().map(|t| t.path.clone());
        if cur == self.followed {
            return;
        }
        self.followed = cur.clone();
        let (Some(path), Some(group)) = (cur, self.panes[self.pane].link) else { return };
        for j in 0..self.panes.len() {
            if j == self.pane || self.panes[j].link != Some(group) {
                continue;
            }
            let shown = self.panes[j].tabs.get(self.panes[j].active).map(|t| t.path.as_str());
            if shown.is_some_and(|p| !p.eq_ignore_ascii_case(&path)) {
                self.in_pane(j, |a| a.show_in_tab(&path));
            }
        }
    }

    /// Copy edits of the focused note into the same note open in other panes.
    fn mirror_edits(&mut self) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let key = (t.path.clone(), t.editor.buf.version);
        if self.mirrored.as_ref() == Some(&key) {
            return;
        }
        let others = self.panes.iter().flat_map(|p| &p.tabs).any(|o| o.path.eq_ignore_ascii_case(&t.path));
        let (text, stamp, saved, edited) =
            (t.editor.buf.text(), t.stamp, !t.editor.buf.is_modified(), t.last_edit_ms);
        self.mirrored = Some(key);
        if !others {
            return;
        }
        let now = fsio::now_ms();
        let path = &self.mirrored.as_ref().unwrap().0;
        for o in self.panes.iter_mut().flat_map(|p| &mut p.tabs).filter(|o| o.path.eq_ignore_ascii_case(path))
        {
            mirror_into(o, &text, now);
            o.last_edit_ms = edited;
            if saved {
                o.stamp = stamp;
                o.editor.buf.mark_saved();
            }
        }
    }

    /// After `path` was saved as `text`: copies of it in other panes are saved too.
    pub(crate) fn saved_elsewhere(&mut self, path: &str, text: &str, stamp: vault_core::fsio::Stamp) {
        for o in self.panes.iter_mut().flat_map(|p| &mut p.tabs).filter(|o| o.path.eq_ignore_ascii_case(path))
        {
            if o.editor.buf.text() == text {
                o.stamp = stamp;
                o.editor.buf.mark_saved();
            }
        }
    }

    /// Once a frame: drop empty panes, follow links, and mirror edits.
    pub(crate) fn sync_panes(&mut self) {
        self.prune_panes();
        self.follow_links();
        self.mirror_edits();
    }

    /// Scroll the active tab of an unfocused pane.
    pub(crate) fn scroll_pane(&mut self, i: usize, blocks: isize) {
        let Some(p) = self.panes.get_mut(i) else { return };
        if let Some(t) = p.tabs.get_mut(p.active) {
            let max = t.editor.buf.doc().blocks.len().saturating_sub(1) as isize;
            t.top_block = (t.top_block as isize + blocks).clamp(0, max.max(0)) as usize;
        }
    }

    pub(crate) fn pane_drag_to(&mut self, x: f32, y: f32) {
        let (Some(d), layout) = (&self.pane_drag, &mut self.layout) else { return };
        let Some(Layout::Split { row, kids }) = node_at_mut(layout, &d.path) else { return };
        if d.k + 1 >= kids.len() {
            return;
        }
        let f = if *row { (x - d.rect.x) / d.rect.w.max(1.0) } else { (y - d.rect.y) / d.rect.h.max(1.0) };
        let before: f32 = kids[..d.k].iter().map(|k| k.1).sum();
        let pair = kids[d.k].1 + kids[d.k + 1].1;
        let min = 0.05f32.min(pair / 2.0);
        let a = (f - before).clamp(min, pair - min);
        kids[d.k].1 = a;
        kids[d.k + 1].1 = pair - a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(row: bool, kids: Vec<Layout>) -> Layout {
        let r = 1.0 / kids.len() as f32;
        Layout::Split { row, kids: kids.into_iter().map(|k| (k, r)).collect() }
    }

    #[test]
    fn splitting_the_same_way_adds_a_sibling() {
        let mut l = Layout::Leaf(0);
        insert_beside(&mut l, 0, 1, true);
        insert_beside(&mut l, 1, 2, true);
        let Layout::Split { row: true, kids } = &l else { panic!("{l:?}") };
        assert_eq!(kids.iter().map(|k| k.0.clone()).collect::<Vec<_>>(), [0, 1, 2].map(Layout::Leaf));
        assert_eq!(kids.iter().map(|k| k.1).collect::<Vec<_>>(), [0.5, 0.25, 0.25]);
    }

    #[test]
    fn splitting_the_other_way_nests() {
        let mut l = Layout::Leaf(0);
        insert_beside(&mut l, 0, 1, true);
        insert_beside(&mut l, 1, 2, false);
        let want = split(true, vec![Layout::Leaf(0), split(false, vec![Layout::Leaf(1), Layout::Leaf(2)])]);
        assert_eq!(l, want);
    }

    #[test]
    fn removing_a_leaf_collapses_its_split_and_keeps_ratios_whole() {
        let mut l = split(true, vec![Layout::Leaf(0), split(false, vec![Layout::Leaf(1), Layout::Leaf(2)])]);
        assert_eq!(remove_leaf(&mut l, 1), Some(2));
        renumber(&mut l, 1);
        assert_eq!(l, split(true, vec![Layout::Leaf(0), Layout::Leaf(1)]));
        assert_eq!(remove_leaf(&mut l, 0), Some(1));
        assert_eq!(l, Layout::Leaf(1));
    }

    #[test]
    fn layout_round_trips_and_rejects_bad_leaves() {
        let l = split(true, vec![Layout::Leaf(1), split(false, vec![Layout::Leaf(0), Layout::Leaf(2)])]);
        let v = layout_json(&l);
        assert_eq!(parse_layout(&v, 3), Some(l));
        assert_eq!(parse_layout(&v, 4), None);
        assert_eq!(parse_layout(&json!({"row": true, "kids": [[0, 1.0]]}), 1), None);
    }

    #[test]
    fn mirroring_replaces_only_the_changed_middle() {
        let mut t = Tab::new("a.md".into(), "héllo wörld", Default::default(), false);
        t.editor.buf.set_sels(vec![editor_core::Sel::cursor(13)]);
        mirror_into(&mut t, "héllo, wörld", 0);
        assert_eq!(t.editor.buf.text(), "héllo, wörld");
        assert_eq!(t.editor.buf.sels[0].head, 14);
        assert!(t.editor.buf.is_modified());
    }
}
