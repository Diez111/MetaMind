//! The open tabs and sidebar state, restored when a vault is reopened.
//!
//! Kept in its own file so the layout tree in `workspace.json`, which other apps own, is never
//! rewritten.

use crate::shell::{App, Panel, ViewMode};
use editor_core::Sel;
use serde_json::{json, Value};

const FILE: &str = "rustsidian-session.json";

fn mode_name(m: ViewMode) -> &'static str {
    match m {
        ViewMode::Source => "source",
        ViewMode::LivePreview => "live",
        ViewMode::Reading => "reading",
    }
}

fn panel_name(p: Panel) -> &'static str {
    match p {
        Panel::Files => "files",
        Panel::Search => "search",
        Panel::Bookmarks => "bookmarks",
        Panel::Tags => "tags",
    }
}

impl App {
    /// The session as JSON.
    pub fn session(&self) -> Value {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|t| {
                let s = t.editor.buf.sels.first().copied().unwrap_or(Sel::cursor(0));
                json!({
                    "path": t.path,
                    "mode": mode_name(t.mode),
                    "top": t.top_block,
                    "cursor": [s.anchor, s.head],
                    "folds": t.folds.headers(),
                })
            })
            .collect();
        let mut collapsed: Vec<&String> = self.collapsed.iter().collect();
        collapsed.sort_unstable();
        json!({
            "tabs": tabs,
            "active": self.active,
            "left": self.left_open,
            "right": self.right_open,
            "panel": panel_name(self.panel),
            "collapsed": collapsed,
        })
    }

    pub fn save_session(&mut self) {
        let text = match serde_json::to_string_pretty(&self.session()) {
            Ok(t) => t,
            Err(e) => {
                self.notice = Some(format!("Cannot save the session: {e}"));
                return;
            }
        };
        let path = self.config.dir.join(FILE);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return;
        }
        let res = std::fs::create_dir_all(&self.config.dir).and_then(|_| std::fs::write(&path, text));
        if let Err(e) = res {
            self.notice = Some(format!("Cannot save the session: {e}"));
        }
    }

    /// Reopen the tabs and sidebar state of the last session. Notes that no longer exist are
    /// skipped.
    pub fn restore_session(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.config.dir.join(FILE)) else { return };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                self.notice = Some(format!("Ignoring the saved session: {e}"));
                return;
            }
        };
        let num = |v: &Value| v.as_u64().map(|n| n as usize);
        let mut active = None;
        let want = v.get("active").and_then(num);
        for (i, t) in v.get("tabs").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let Some(p) = t.get("path").and_then(Value::as_str) else { continue };
            if self.store.index.id(p).is_none() {
                continue;
            }
            let before = self.tabs.len();
            self.open_note(p);
            if self.tabs.len() == before {
                continue;
            }
            let tab = self.tabs.last_mut().unwrap();
            tab.mode = match t.get("mode").and_then(Value::as_str) {
                Some("source") => ViewMode::Source,
                Some("reading") => ViewMode::Reading,
                _ => ViewMode::LivePreview,
            };
            tab.top_block = t.get("top").and_then(num).unwrap_or(0);
            if let Some([a, h]) = t.get("cursor").and_then(Value::as_array).map(Vec::as_slice) {
                if let (Some(a), Some(h)) = (num(a), num(h)) {
                    tab.editor.buf.set_sels(vec![Sel::new(a, h)]);
                }
            }
            let len = tab.editor.buf.len();
            for f in t.get("folds").and_then(Value::as_array).into_iter().flatten().filter_map(num) {
                if f < len {
                    tab.folds.fold(f);
                }
            }
            tab.hidden_dirty = true;
            if want == Some(i) {
                active = Some(self.tabs.len() - 1);
            }
        }
        if let Some(i) = active.or(self.tabs.len().checked_sub(1)) {
            self.open_note(&self.tabs[i].path.clone());
        }
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        self.left_open = flag("left").unwrap_or(self.left_open);
        self.right_open = flag("right").unwrap_or(self.right_open);
        self.panel = match v.get("panel").and_then(Value::as_str) {
            Some("search") => Panel::Search,
            Some("bookmarks") => Panel::Bookmarks,
            Some("tags") => Panel::Tags,
            _ => Panel::Files,
        };
        if let Some(c) = v.get("collapsed").and_then(Value::as_array) {
            self.collapsed = c.iter().filter_map(Value::as_str).map(str::to_string).collect();
            self.tree = None;
        }
    }
}
