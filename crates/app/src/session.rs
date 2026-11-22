//! The open tabs and sidebar state, restored when a vault is reopened.
//!
//! Kept in its own file so the layout tree in `workspace.json`, which other apps own, is never
//! rewritten.

use crate::shell::{App, Panel, RightPanel, Tab, ViewMode};
use editor_core::Sel;
use serde_json::{json, Value};

const FILE: &str = "metamind-session.json";

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
        Panel::Calendar => "calendar",
    }
}

fn tabs_json(tabs: &[Tab]) -> Vec<Value> {
    tabs.iter()
        .map(|t| {
            let s = t.editor.buf.sels.first().copied().unwrap_or(Sel::cursor(0));
            json!({
                "path": t.path,
                "mode": mode_name(t.mode),
                "top": t.top_block,
                "cursor": [s.anchor, s.head],
                "folds": t.folds.headers(),
                "pinned": t.pinned,
            })
        })
        .collect()
}

impl App {
    /// Whether the tree remembers the folders opened in the last session
    /// (`rememberExpandedFolders` in `app.json`, on unless it says otherwise).
    pub fn remember_expanded(&self) -> bool {
        self.config.app.bool("rememberExpandedFolders", true)
    }

    /// The session as JSON. `tabs` and `active` repeat the focused pane for versions without
    /// split panes.
    pub fn session(&self) -> Value {
        let panes: Vec<Value> = (0..self.panes.len())
            .map(|i| {
                let (tabs, active) = self.pane_tabs(i);
                json!({ "tabs": tabs_json(tabs), "active": active, "link": self.panes[i].link,
                    "stacked": self.panes[i].stacked })
            })
            .collect();
        let mut expanded: Vec<&String> = self.expanded.iter().collect();
        expanded.sort_unstable();
        let mut session = json!({
            "tabs": tabs_json(&self.tabs),
            "active": self.active,
            "panes": panes,
            "layout": crate::split::layout_json(&self.docked_layout()),
            "pane": self.pane,
            "left": self.left_open,
            "right": self.right_open,
            "leftWidth": self.left_w,
            "rightWidth": self.right_w,
            "panel": panel_name(self.panel),
            "rightPanel": match self.right_panel {
                RightPanel::Backlinks => "backlinks",
                RightPanel::Outgoing => "outgoing",
                RightPanel::Outline => "outline",
                RightPanel::Ai => "ai",
            },
        });
        // Folders the user opened are kept only when the setting says so; otherwise the tree
        // starts folded every time, which is also what an empty list means.
        if self.remember_expanded() {
            session["expanded"] = json!(expanded);
        }
        session
    }

    pub fn save_session(&mut self) {
        let text = match serde_json::to_string_pretty(&self.session()) {
            Ok(t) => t,
            Err(e) => {
                self.notice = Some(format!("No se pudo guardar la sesión: {e}"));
                return;
            }
        };
        let path = self.config.dir.join(FILE);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return;
        }
        let res = std::fs::create_dir_all(&self.config.dir).and_then(|_| std::fs::write(&path, text));
        if let Err(e) = res {
            self.notice = Some(format!("No se pudo guardar la sesión: {e}"));
        }
    }

    /// Reopen the tabs and sidebar state of the last session. Notes that no longer exist are
    /// skipped.
    pub fn restore_session(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.config.dir.join(FILE)) else { return };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                self.notice = Some(format!("Se ignoró la sesión guardada: {e}"));
                return;
            }
        };
        self.apply_session(&v);
    }

    /// Open the tabs, panes and sidebar state of session `v`.
    pub(crate) fn apply_session(&mut self, v: &Value) {
        let num = |v: &Value| v.as_u64().map(|n| n as usize);
        let panes = v.get("panes").and_then(Value::as_array).filter(|p| !p.is_empty());
        let layout = panes.and_then(|p| crate::split::parse_layout(v.get("layout")?, p.len()));
        match (panes, layout) {
            (Some(panes), Some(layout)) => {
                self.panes = panes.iter().map(|_| Default::default()).collect();
                self.layout = layout;
                for (i, p) in panes.iter().enumerate() {
                    self.panes[i].link = p.get("link").and_then(Value::as_u64).map(|g| g as u32);
                    self.panes[i].stacked = p.get("stacked").and_then(Value::as_bool).unwrap_or(false);
                    self.in_pane(i, |a| a.restore_tabs(p));
                }
                self.focus_pane(v.get("pane").and_then(num).filter(|&p| p < panes.len()).unwrap_or(0));
            }
            _ => self.restore_tabs(v),
        }
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        self.left_open = flag("left").unwrap_or(self.left_open);
        self.right_open = flag("right").unwrap_or(self.right_open);
        self.panel = match v.get("panel").and_then(Value::as_str) {
            Some("search") => Panel::Search,
            Some("bookmarks") => Panel::Bookmarks,
            Some("tags") => Panel::Tags,
            Some("calendar") => Panel::Calendar,
            _ => Panel::Files,
        };
        self.right_panel = match v.get("rightPanel").and_then(Value::as_str) {
            Some("outgoing") => RightPanel::Outgoing,
            Some("outline") => RightPanel::Outline,
            Some("ai") => RightPanel::Ai,
            _ => RightPanel::Backlinks,
        };
        if let Some(c) = v.get("leftWidth").and_then(Value::as_f64) {
            self.left_w = (c as f32).clamp(crate::shell::SIDE_W_MIN, crate::shell::SIDE_W_MAX);
        }
        if let Some(c) = v.get("rightWidth").and_then(Value::as_f64) {
            self.right_w = (c as f32).clamp(crate::shell::SIDE_W_MIN, crate::shell::SIDE_W_MAX);
        }
        // The explorer starts folded; only a session that recorded the open folders brings them
        // back, and only while "recordar carpetas expandidas" is on.
        if self.remember_expanded() {
            if let Some(c) = v.get("expanded").and_then(Value::as_array) {
                self.expanded = c.iter().filter_map(Value::as_str).map(crate::shell::folded_path).collect();
                self.tree = None;
            }
        }
    }

    /// Open the tabs saved in `v` (`tabs` and `active`) in the focused pane.
    fn restore_tabs(&mut self, v: &Value) {
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
            tab.pinned = t.get("pinned").and_then(Value::as_bool).unwrap_or(false);
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
    }
}
