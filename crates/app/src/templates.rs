//! Note templates and daily notes, configured by the vault's `templates.json` and
//! `daily-notes.json`.

use crate::datetime;
use crate::shell::{App, PromptItem};
use editor_core::EditKind;
use serde_json::Value;
use vault_core::{fsio, path};

/// Fill `{{title}}`, `{{date}}`, `{{time}}` and `{{date:FORMAT}}` / `{{time:FORMAT}}`.
pub fn fill(template: &str, title: &str, date_fmt: &str, time_fmt: &str, secs: u64) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = after[..j].trim();
        let (name, fmt) = inner.split_once(':').map_or((inner, None), |(n, f)| (n.trim(), Some(f.trim())));
        match name.to_ascii_lowercase().as_str() {
            "title" => out.push_str(title),
            "date" => out.push_str(&datetime::format(fmt.unwrap_or(date_fmt), secs)),
            "time" => out.push_str(&datetime::format(fmt.unwrap_or(time_fmt), secs)),
            // Unknown variables stay as written so other tools can fill them.
            _ => out.push_str(&rest[i..i + 2 + j + 2]),
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    out
}

fn read_json(app: &App, file: &str) -> Value {
    std::fs::read_to_string(app.config.dir.join(file))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn non_empty(v: &Value, key: &str, default: &str) -> String {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or(default).to_string()
}

impl App {
    fn template_settings(&self) -> (String, String, String) {
        let v = read_json(self, "templates.json");
        let folder = non_empty(&v, "folder", "Templates").trim_matches('/').to_string();
        (folder, non_empty(&v, "dateFormat", "YYYY-MM-DD"), non_empty(&v, "timeFormat", "HH:mm"))
    }

    /// Notes in the templates folder, for the template picker.
    pub(crate) fn template_items(&mut self, query: &str) -> Vec<PromptItem> {
        let (folder, ..) = self.template_settings();
        let prefix = format!("{}/", folder.to_lowercase());
        let mut paths: Vec<String> = self
            .store
            .index
            .notes()
            .map(|(_, f)| f.path.to_string())
            .filter(|p| p.to_lowercase().starts_with(&prefix))
            .collect();
        paths.sort_unstable();
        if paths.is_empty() {
            return vec![PromptItem {
                label: format!("No templates in {folder}/"),
                aux: "Set the folder in templates.json".into(),
                action: "prompt:dismiss".into(),
            }];
        }
        let names: Vec<&str> = paths.iter().map(|p| path::strip_md(&p[prefix.len()..])).collect();
        let hits = self.fuzzy_top(query, &names, 12);
        hits.into_iter()
            .map(|i| PromptItem {
                label: names[i].to_string(),
                aux: String::new(),
                action: format!("templates:insert:{}", paths[i]),
            })
            .collect()
    }

    fn template_text(&mut self, rel: &str, title: &str) -> Option<String> {
        let (_, date_fmt, time_fmt) = self.template_settings();
        match self.store.read(rel) {
            Ok((text, _)) => Some(fill(&text, title, &date_fmt, &time_fmt, fsio::now_ms() / 1000)),
            Err(e) => {
                self.notice = Some(format!("Cannot read template {rel}: {e}"));
                None
            }
        }
    }

    /// Insert a template at the cursor of the active note.
    fn insert_template(&mut self, rel: &str) {
        let Some(title) = self.tab().map(|t| t.title().to_string()) else {
            self.notice = Some("Open a note to insert a template".into());
            return;
        };
        let Some(text) = self.template_text(rel, &title) else { return };
        let now = fsio::now_ms();
        if let Some(t) = self.tab_mut() {
            t.editor.replace_selections(&text, EditKind::Other, now);
            t.last_edit_ms = now;
        }
    }

    /// Open today's daily note, creating it from the configured folder, format and template.
    pub(crate) fn open_daily_note(&mut self) {
        let v = read_json(self, "daily-notes.json");
        let secs = fsio::now_ms() / 1000;
        let name = datetime::format(&non_empty(&v, "format", "YYYY-MM-DD"), secs);
        let folder = non_empty(&v, "folder", "").trim_matches('/').to_string();
        let rel = path::join(&folder, &format!("{name}.md"));
        if self.store.index.id(&rel).is_none()
            && !self.create_from_template(&rel, &non_empty(&v, "template", ""))
        {
            return;
        }
        self.open_note(&rel);
    }

    /// Create a note named by the current time, with the format, folder and template from
    /// `zk-prefixer.json`. A name already taken gets a counter.
    pub(crate) fn new_unique_note(&mut self) {
        let v = read_json(self, "zk-prefixer.json");
        let name = datetime::format(&non_empty(&v, "format", "YYYYMMDDHHmm"), fsio::now_ms() / 1000);
        let folder = non_empty(&v, "folder", "").trim_matches('/').to_string();
        let mut rel = path::join(&folder, &format!("{name}.md"));
        let mut n = 1;
        while self.store.index.id(&rel).is_some() || self.store.abs(&rel).exists() {
            rel = path::join(&folder, &format!("{name} {n}.md"));
            n += 1;
        }
        if self.create_from_template(&rel, &non_empty(&v, "template", "")) {
            self.open_note(&rel);
        }
    }

    /// Create `rel` with the filled-in template, or empty when `template` is empty. Reports
    /// failures as a notice and returns false.
    fn create_from_template(&mut self, rel: &str, template: &str) -> bool {
        let text = if template.is_empty() {
            String::new()
        } else {
            let t = if template.to_lowercase().ends_with(".md") {
                template.to_string()
            } else {
                format!("{template}.md")
            };
            match self.template_text(&t, path::stem(rel)) {
                Some(text) => text,
                None => return false,
            }
        };
        if let Some(dir) = Some(path::parent(rel)).filter(|d| !d.is_empty()) {
            if let Err(e) = std::fs::create_dir_all(self.store.abs(dir)) {
                self.notice = Some(format!("Cannot create {dir}: {e}"));
                return false;
            }
        }
        if let Err(e) = self.store.create(rel, &text) {
            self.notice = Some(format!("Cannot create {rel}: {e}"));
            return false;
        }
        self.tree = None;
        true
    }

    pub(crate) fn run_templates(&mut self, command: &str) -> bool {
        if let Some(rel) = command.strip_prefix("templates:insert:") {
            self.insert_template(rel);
            return true;
        }
        match command {
            "templates:insert-template" => self.open_prompt(crate::shell::PromptKind::Template),
            "templates:insert-current-date" | "templates:insert-current-time" => {
                let (_, date_fmt, time_fmt) = self.template_settings();
                let fmt = if command.ends_with("date") { date_fmt } else { time_fmt };
                let text = datetime::format(&fmt, fsio::now_ms() / 1000);
                let now = fsio::now_ms();
                if let Some(t) = self.tab_mut() {
                    t.editor.replace_selections(&text, EditKind::Other, now);
                    t.last_edit_ms = now;
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_variables_and_keeps_unknown_ones() {
        let t = (1_709_210_096 - datetime::local_offset(1_709_210_096)) as u64;
        let out = fill(
            "# {{title}}\n{{date}} {{ time }} {{date:dddd}} {{other}} {{",
            "Plan",
            "YYYY-MM-DD",
            "HH:mm",
            t,
        );
        assert_eq!(out, "# Plan\n2024-02-29 12:34 Thursday {{other}} {{");
    }
}
