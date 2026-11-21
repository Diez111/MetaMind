//! Workspaces: named layouts of tabs, panes and sidebars, saved and loaded on request.
//!
//! A workspace is a session snapshot, kept in a file of its own so Obsidian's
//! `workspaces.json`, which has a different shape, is never rewritten.

use crate::shell::{App, PromptItem, PromptKind};
use serde_json::{Map, Value};

const FILE: &str = "rustsidian-workspaces.json";

impl App {
    fn read_workspaces(&self) -> Result<Map<String, Value>, String> {
        let path = self.config.dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(e) => return Err(format!("No se pudieron leer los espacios de trabajo: {e}")),
        };
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("No se pudieron leer los espacios de trabajo: {e}"))?;
        Ok(v.get("workspaces").and_then(Value::as_object).cloned().unwrap_or_default())
    }

    fn write_workspaces(&self, all: Map<String, Value>) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&serde_json::json!({ "workspaces": all }))
            .map_err(|e| format!("No se pudieron guardar los espacios de trabajo: {e}"))?;
        std::fs::create_dir_all(&self.config.dir)
            .and_then(|_| std::fs::write(self.config.dir.join(FILE), text))
            .map_err(|e| format!("No se pudieron guardar los espacios de trabajo: {e}"))
    }

    /// Saved workspace names, sorted.
    pub fn workspace_names(&self) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = self.read_workspaces()?.keys().cloned().collect();
        names.sort_unstable_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    pub fn save_workspace(&mut self, name: &str) {
        let name = name.trim();
        let saved = self.read_workspaces().and_then(|mut all| {
            all.insert(name.to_string(), self.session());
            self.write_workspaces(all)
        });
        self.notice = Some(match saved {
            Ok(()) => format!("Espacio de trabajo \u{201c}{name}\u{201d} guardado"),
            Err(e) => e,
        });
    }

    /// Replace the open tabs and panes with workspace `name`. Notes are saved first; if one
    /// cannot be, nothing is replaced.
    pub fn load_workspace(&mut self, name: &str) {
        let v = match self.read_workspaces() {
            Ok(all) => all.get(name).cloned(),
            Err(e) => {
                self.notice = Some(e);
                return;
            }
        };
        let Some(v) = v else {
            self.notice = Some(format!("No hay ningún espacio de trabajo llamado \u{201c}{name}\u{201d}"));
            return;
        };
        self.save_all();
        if self.all_tabs().any(|t| t.editor.buf.is_modified()) {
            self.notice = Some(
                "No se pudo cargar el espacio de trabajo: una nota modificada no se pudo guardar".to_string(),
            );
            return;
        }
        self.tabs.clear();
        self.active = 0;
        self.panes = vec![Default::default()];
        self.layout = crate::split::Layout::Leaf(0);
        self.pane = 0;
        self.popouts.clear();
        self.last_main = 0;
        self.followed = None;
        self.mirrored = None;
        self.show_graph = false;
        self.show_canvas = false;
        self.show_base = false;
        self.apply_session(&v);
        self.notice = Some(format!("Espacio de trabajo \u{201c}{name}\u{201d} cargado"));
    }

    pub fn delete_workspace(&mut self, name: &str) {
        let deleted = self.read_workspaces().and_then(|mut all| {
            all.remove(name);
            self.write_workspaces(all)
        });
        self.notice = Some(match deleted {
            Ok(()) => format!("Espacio de trabajo \u{201c}{name}\u{201d} eliminado"),
            Err(e) => e,
        });
    }

    /// Prompt rows: `target` is "save", "load" or "delete".
    pub(crate) fn workspace_items(&mut self, target: &str, query: &str) -> Vec<PromptItem> {
        let row =
            |label: String, aux: &str, action: String| PromptItem { label, aux: aux.to_string(), action };
        let names = match self.workspace_names() {
            Ok(n) => n,
            Err(e) => return vec![row(e, "", "prompt:dismiss".into())],
        };
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let hits = self.fuzzy_top(query, &refs, 12);
        let mut items = Vec::new();
        let typed = query.trim();
        if target == "save" && !typed.is_empty() && !names.iter().any(|n| n == typed) {
            items.push(row(
                format!("Guardar como \u{201c}{typed}\u{201d}"),
                "Nuevo",
                format!("workspaces:save-as:{typed}"),
            ));
        }
        for i in hits {
            let n = &names[i];
            items.push(match target {
                "save" => row(n.clone(), "Reemplazar", format!("workspaces:save-as:{n}")),
                "delete" => row(n.clone(), "Eliminar", format!("workspaces:delete:{n}")),
                _ => row(n.clone(), "", format!("workspaces:load:{n}")),
            });
        }
        if items.is_empty() && target != "save" {
            items.push(row("No hay espacios de trabajo guardados".into(), "", "prompt:dismiss".into()));
        }
        items
    }

    pub(crate) fn run_workspaces(&mut self, command: &str) -> bool {
        if let Some(n) = command.strip_prefix("workspaces:save-as:") {
            self.save_workspace(n);
        } else if let Some(n) = command.strip_prefix("workspaces:load:") {
            self.load_workspace(n);
        } else if let Some(n) = command.strip_prefix("workspaces:delete:") {
            self.delete_workspace(n);
        } else if let Some(t) =
            command.strip_prefix("workspaces:").filter(|t| ["save", "load", "delete"].contains(t))
        {
            self.open_name_prompt(PromptKind::Workspace, t, "");
            return true;
        } else {
            return false;
        }
        true
    }
}
