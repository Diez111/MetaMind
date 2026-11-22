//! Right-click menus for explorer rows, tabs and the editor, and the file operations they offer
//! (rename, new folder, make a copy, delete, copy path, reveal in the system file manager), plus
//! cut, copy and paste.

use crate::shell::{App, Focus, Key, PromptKind, ViewMode};
use crate::{clipboard, view, Frame};
use editor_core::{EditKind, Sel};
use metamind_ui::Fonts;
use vault_core::{fsio, path};

pub const ITEM_H: f32 = 28.0;
pub const SEPARATOR_H: f32 = 9.0;
pub const WIDTH: f32 = 248.0;

/// One menu row. A row with an empty action is a separator.
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub icon: &'static str,
    pub action: String,
    pub hotkey: String,
    /// Destructive actions are shown in the warning color.
    pub warning: bool,
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
    /// The row picked with the arrow keys.
    pub selected: Option<usize>,
}

impl Menu {
    pub fn height(&self) -> f32 {
        8.0 + self.items.iter().map(|i| if i.action.is_empty() { SEPARATOR_H } else { ITEM_H }).sum::<f32>()
    }
}

struct Builder<'a> {
    app: &'a App,
    items: Vec<MenuItem>,
}

impl Builder<'_> {
    fn item(&mut self, icon: &'static str, label: &str, action: impl Into<String>) -> &mut Self {
        let action = action.into();
        let hotkey = self.app.hotkey_for(&action);
        self.items.push(MenuItem { label: label.into(), icon, action, hotkey, warning: false });
        self
    }
    fn warning(&mut self, icon: &'static str, label: &str, action: impl Into<String>) -> &mut Self {
        self.item(icon, label, action);
        if let Some(last) = self.items.last_mut() {
            last.warning = true;
        }
        self
    }
    fn separator(&mut self) -> &mut Self {
        if self.items.last().is_some_and(|i| !i.action.is_empty()) {
            self.items.push(MenuItem {
                label: String::new(),
                icon: "",
                action: String::new(),
                hotkey: String::new(),
                warning: false,
            });
        }
        self
    }
}

impl App {
    /// Right click at window coordinates: open the menu for what is under the pointer.
    pub fn context_menu(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32) {
        self.menu = None;
        if self.prompt.is_some() || self.settings.is_some() {
            return;
        }
        if !self.press_pane(frame, x, y) {
            return;
        }
        // Over a kanban board the menu is the column's: rename it, fold it, move it, empty it.
        if self.kanban_active()
            && frame.info.kanban.is_some_and(|n| frame.dom.nodes[n].rect.contains(x, y)) {
                return self.kanban_context_menu(x, y);
            }
        let action = frame.dom.action_at(x, y).map(|(_, a)| a.to_string());
        let mut b = Builder { app: self, items: Vec::new() };
        match action.as_deref() {
            Some(a) if a.starts_with("open:") => file_items(&mut b, &a["open:".len()..]),
            Some(a) if a.starts_with("toggle-folder:") => folder_items(&mut b, &a["toggle-folder:".len()..]),
            Some(a) if a.starts_with("tab:") || a.starts_with("close-tab:") => {
                let i: usize = a.rsplit(':').next().and_then(|i| i.parse().ok()).unwrap_or(0);
                tab_items(&mut b, i);
            }
            Some("graph:focus") => {
                b.item("x", "Cerrar", "graph:close");
            }
            Some("canvas:focus") => {
                b.item("x", "Cerrar", "canvas:close");
            }
            Some("base:focus") => {
                b.item("x", "Cerrar", "base:close");
            }
            Some(_) => return,
            None => {
                let in_files =
                    frame.dom.by_id("files").is_some_and(|n| frame.dom.nodes[n].rect.contains(x, y));
                if in_files {
                    folder_items(&mut b, "");
                } else if let Some(off) = view::offset_at(&frame.dom, &frame.info, fonts, x, y) {
                    drop(b);
                    self.focus = Focus::Editor;
                    if let Some(t) = self.tab_mut() {
                        // Keep a selection the click falls inside, so "Cut" and "Copy" act on it.
                        let inside = t
                            .editor
                            .buf
                            .sels
                            .iter()
                            .any(|s| !s.is_empty() && s.start() <= off && off <= s.end());
                        if !inside {
                            t.editor.buf.set_sels(vec![Sel::cursor(off)]);
                        }
                    }
                    let typo = frame
                        .info
                        .blocks
                        .iter()
                        .filter(|bv| bv.src_start <= off && off <= bv.src_end)
                        .flat_map(|bv| view::misspellings(self, &frame.dom, bv))
                        .find(|&(a, b)| a <= off && off <= b);
                    let mut b = Builder { app: self, items: Vec::new() };
                    editor_items(&mut b, typo);
                    let items = b.items;
                    return self.show_menu(x, y, items);
                } else {
                    return;
                }
            }
        }
        let items = b.items;
        self.show_menu(x, y, items);
    }

    /// The explorer's sort order picker, opened below its header button.
    pub(crate) fn sort_menu(&mut self, x: f32, y: f32) {
        let current = self.sort_order().to_string();
        let mut b = Builder { app: self, items: Vec::new() };
        for (i, (order, label)) in [
            ("alphabetical", "Nombre de archivo (A a Z)"),
            ("alphabeticalReverse", "Nombre de archivo (Z a A)"),
            ("byModifiedTime", "Fecha de modificación (reciente a antigua)"),
            ("byModifiedTimeReverse", "Fecha de modificación (antigua a reciente)"),
            ("byCreatedTime", "Fecha de creación (reciente a antigua)"),
            ("byCreatedTimeReverse", "Fecha de creación (antigua a reciente)"),
        ]
        .into_iter()
        .enumerate()
        {
            if i % 2 == 0 {
                b.separator();
            }
            let icon = if order == current { "check" } else { "blank" };
            b.item(icon, label, format!("file-explorer:sort:{order}"));
        }
        let items = b.items;
        self.show_menu(x, y, items);
    }

    /// The AI panel header: pick the profile the copilot sends to.
    pub(crate) fn ai_profile_menu(&mut self, x: f32, y: f32) {
        let cfg = crate::ai::AiConfig::from_config(&self.config);
        let active = cfg.active_index();
        let mut b = Builder { app: self, items: Vec::new() };
        for (i, p) in cfg.profiles.iter().enumerate() {
            let icon = if i == active { "check" } else { "blank" };
            b.item(icon, &format!("{} · {}", p.name, p.model), format!("ai:profile:{}", p.name));
        }
        b.separator();
        b.item("settings", "Gestionar perfiles…", "ai:settings");
        let items = b.items;
        self.show_menu(x, y, items);
    }

    pub(crate) fn show_menu(&mut self, x: f32, y: f32, mut items: Vec<MenuItem>) {
        while items.last().is_some_and(|i| i.action.is_empty()) {
            items.pop();
        }
        if items.is_empty() {
            return;
        }
        self.menu = Some(Menu { x, y, items, selected: None });
    }

    /// While a menu is open, a click either picks a row or just closes the menu.
    /// Returns true when the click was taken by the menu.
    pub(crate) fn menu_click(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        let Some(menu) = self.menu.take() else { return false };
        let picked = frame
            .dom
            .action_at(x, y)
            .and_then(|(_, a)| a.strip_prefix("menu:"))
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| menu.items.get(i))
            .map(|i| i.action.clone());
        if let Some(action) = picked {
            self.run(&action);
        }
        true
    }

    /// Keys while a menu is open: arrows move, Enter picks, Escape closes. Any other key closes
    /// the menu and is then handled as usual.
    pub(crate) fn menu_key(&mut self, key: &Key) -> Option<bool> {
        let menu = self.menu.as_mut()?;
        let rows: Vec<usize> = (0..menu.items.len()).filter(|&i| !menu.items[i].action.is_empty()).collect();
        let at = menu.selected.and_then(|s| rows.iter().position(|&r| r == s));
        match key.key.as_str() {
            "Escape" => self.menu = None,
            "ArrowDown" => menu.selected = rows.get(at.map_or(0, |a| (a + 1) % rows.len())).copied(),
            "ArrowUp" => {
                menu.selected =
                    rows.get(at.map_or(rows.len() - 1, |a| (a + rows.len() - 1) % rows.len())).copied()
            }
            "Enter" => {
                let action = menu.selected.and_then(|s| menu.items.get(s)).map(|i| i.action.clone());
                self.menu = None;
                if let Some(a) = action {
                    self.run(&a);
                }
            }
            _ => {
                self.menu = None;
                return None;
            }
        }
        Some(true)
    }

    /// File operations and clipboard commands. Returns false for commands it does not know.
    pub(crate) fn run_file_action(&mut self, command: &str) -> bool {
        let (verb, arg) = command.split_once(':').unwrap_or((command, ""));
        match verb {
            "rename-file" => {
                let initial = if self.is_folder(arg) { path::file_name(arg) } else { path::stem(arg) };
                let initial = initial.to_string();
                self.open_name_prompt(PromptKind::Rename, arg, &initial);
            }
            "rename" => {
                let Some((from, name)) = arg.split_once('|') else { return true };
                self.rename_path(from, name);
            }
            "new-folder-in" => self.open_name_prompt(PromptKind::NewFolder, arg, ""),
            "mkdir" => {
                let Some((parent, name)) = arg.split_once('|') else { return true };
                self.make_folder(parent, name);
            }
            "new-note-in" => self.new_note_in(arg),
            "duplicate" => self.duplicate(arg),
            "bookmark" => self.toggle_bookmark(arg),
            "delete" => self.delete_path(arg),
            "copy-path" => {
                let p = arg.to_string();
                self.put_clip(p);
            }
            "copy-link" => {
                let link = format!("[[{}]]", path::strip_md(arg));
                self.put_clip(link);
            }
            "reveal" => self.reveal(arg),
            "open-external" => self.open_external(arg),
            "close-others" => {
                if let Ok(i) = arg.parse::<usize>() {
                    self.active = i.min(self.tabs.len().saturating_sub(1));
                    self.run("workspace:close-others");
                }
            }
            "close-right" => {
                if let Ok(i) = arg.parse::<usize>() {
                    for j in (i + 1..self.tabs.len()).rev() {
                        self.close_tab(j);
                    }
                }
            }
            _ => match command {
                "file-explorer:new-folder" => {
                    let dir = self.new_note_dir();
                    self.open_name_prompt(PromptKind::NewFolder, &dir, "");
                }
                "workspace:edit-file-title" => {
                    if let Some(p) = self.tab().map(|t| t.path.clone()) {
                        self.run(&format!("rename-file:{p}"));
                    }
                }
                "workspace:copy-path" => {
                    if let Some(p) = self.tab().map(|t| t.path.clone()) {
                        self.put_clip(p);
                    }
                }
                "workspace:export-pdf" => self.export_pdf(),
                "workspace:copy-screenshot" => self.screenshot = self.tab().is_some(),
                "file-explorer:reveal-active-file" => self.reveal_in_explorer(),
                "editor:copy" => self.copy(false),
                "editor:cut" => self.copy(true),
                "editor:paste" => self.paste(),
                "editor:insert-wikilink" => self.insert_wikilink(),
                _ => return false,
            },
        }
        true
    }

    fn is_folder(&self, rel: &str) -> bool {
        self.store.index.id(rel).is_none() && self.store.abs(rel).is_dir()
    }

    fn rename_path(&mut self, from: &str, name: &str) {
        let clean = path::sanitize_file_name(name.trim(), path::SanitizePolicy::Strip);
        if clean.is_empty() {
            self.notice = Some("Falta un nombre".into());
            return;
        }
        let folder = self.is_folder(from);
        let ext = path::extension(from);
        let file = if folder || ext.is_empty() || path::extension(&clean).eq_ignore_ascii_case(ext) {
            clean
        } else {
            format!("{clean}.{ext}")
        };
        let to = path::join(path::parent(from), &file);
        if to == from {
            return;
        }
        if let Some(links) = self.move_path(from, &to, "renombrar") {
            self.notice = Some(format!("Renombrado a {file}{links}"));
        }
    }

    /// Move a file or folder into `folder` ("" is the vault root), as an explorer drag does.
    pub(crate) fn move_into(&mut self, from: &str, folder: &str) {
        let to = path::join(folder, path::file_name(from));
        if let Some(links) = self.move_path(from, &to, "mover") {
            self.expanded.insert(crate::shell::folded_path(folder));
            let place = if folder.is_empty() { "la raíz de la bóveda" } else { folder };
            self.notice = Some(format!("Se movió {} a {place}{links}", path::file_name(from)));
        }
    }

    /// Rename on disk and follow the move in tabs, bookmarks and plugins. Returns the notice
    /// suffix about rewritten links, or `None` after reporting a failure.
    fn move_path(&mut self, from: &str, to: &str, verb: &str) -> Option<String> {
        // Unsaved edits would be lost when the files move, so everything is saved first.
        self.save_all();
        match self.store.rename(from, to) {
            Ok(plan) => {
                self.bookmarks_moved(&plan.moves);
                for (old, new) in &plan.moves {
                    for t in self.all_tabs_mut().filter(|t| t.path.eq_ignore_ascii_case(old)) {
                        t.path = new.clone();
                    }
                    self.plugins.push(ext_api::Event::Rename { path: new.clone(), old_path: old.clone() });
                }
                let edited: Vec<String> = plan.edits.iter().map(|e| e.path.clone()).collect();
                self.reload_tabs(&edited);
                self.files_changed(&[]);
                Some(match edited.len() {
                    0 => String::new(),
                    1 => "; se actualizaron los enlaces de 1 archivo".into(),
                    n => format!("; se actualizaron los enlaces de {n} archivos"),
                })
            }
            Err(e) => {
                self.notice = Some(format!("No se pudo {verb} {from}: {e}"));
                None
            }
        }
    }

    /// Replace the text of open tabs whose files were rewritten on disk.
    pub(crate) fn reload_tabs(&mut self, paths: &[String]) {
        for p in paths {
            if let Ok((text, stamp)) = self.store.read(p) {
                for tab in self.all_tabs_mut().filter(|t| t.path.eq_ignore_ascii_case(p)) {
                    let head = tab.editor.buf.sels[0].head.min(text.len());
                    tab.editor = editor_core::Editor::new(&text);
                    tab.editor.buf.set_sels(vec![Sel::cursor(head)]);
                    tab.stamp = stamp;
                }
            }
        }
    }

    /// The explorer lists folders through the files in them, so a new folder starts with a note.
    fn make_folder(&mut self, parent: &str, name: &str) {
        let clean = path::sanitize_file_name(name.trim(), path::SanitizePolicy::Strip);
        if clean.is_empty() {
            self.notice = Some("Falta un nombre".into());
            return;
        }
        let dir = path::join(parent, &clean);
        if self.store.abs(&dir).exists() {
            self.notice = Some(format!("{dir} ya existe"));
            return;
        }
        if let Err(e) = std::fs::create_dir_all(self.store.abs(&dir)) {
            self.notice = Some(format!("No se pudo crear {dir}: {e}"));
            return;
        }
        self.expanded.insert(crate::shell::folded_path(&dir));
        self.tree = None;
        self.new_note_in(&dir);
    }

    fn new_note_in(&mut self, dir: &str) {
        let rel = (0..)
            .map(|n| if n == 0 { "Untitled.md".to_string() } else { format!("Untitled {n}.md") })
            .map(|name| path::join(dir, &name))
            .find(|p| self.store.index.id(p).is_none() && !self.store.abs(p).exists())
            .unwrap_or_default();
        match self.store.create(&rel, "") {
            Ok(_) => {
                self.plugins.push(ext_api::Event::Create { path: rel.clone() });
                self.expanded.insert(crate::shell::folded_path(dir));
                self.tree = None;
                self.files_changed(&[]);
                self.open_note(&rel);
            }
            Err(e) => self.notice = Some(format!("No se pudo crear la nota: {e}")),
        }
    }

    fn duplicate(&mut self, rel: &str) {
        self.save_all();
        let text = match self.store.read(rel) {
            Ok((t, _)) => t,
            Err(e) => {
                self.notice = Some(format!("No se pudo leer {rel}: {e}"));
                return;
            }
        };
        let (dir, stem, ext) = (path::parent(rel), path::stem(rel), path::extension(rel));
        let copy = (1..)
            .map(|n| path::join(dir, &format!("{stem} {n}.{ext}")))
            .find(|p| self.store.index.id(p).is_none() && !self.store.abs(p).exists())
            .unwrap_or_default();
        match self.store.create(&copy, &text) {
            Ok(_) => {
                self.plugins.push(ext_api::Event::Create { path: copy.clone() });
                self.files_changed(&[]);
                self.open_note(&copy);
            }
            Err(e) => self.notice = Some(format!("No se pudo copiar {rel}: {e}")),
        }
    }

    fn delete_path(&mut self, rel: &str) {
        let prefix = format!("{}/", rel.to_lowercase());
        let gone = |p: &str| p.eq_ignore_ascii_case(rel) || p.to_lowercase().starts_with(&prefix);
        match self.store.delete(rel) {
            Ok(dest) => {
                self.retain_tabs(|t| !gone(&t.path));
                self.plugins.push(ext_api::Event::Delete { path: rel.to_string() });
                self.files_changed(&[]);
                self.notice = Some(match dest {
                    Some(_) => format!("Se movió {rel} a la papelera"),
                    None => format!("Se eliminó {rel}"),
                });
            }
            Err(e) => self.notice = Some(format!("No se pudo eliminar {rel}: {e}")),
        }
    }

    /// Show a file or folder in the system file manager.
    fn reveal(&mut self, rel: &str) {
        let abs = self.store.abs(rel);
        let shown = abs.display().to_string();
        let result = if cfg!(windows) {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", shown.replace('/', "\\")))
                .spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg("-R").arg(&abs).spawn()
        } else {
            let dir = if abs.is_dir() {
                abs.clone()
            } else {
                abs.parent().map(|p| p.to_path_buf()).unwrap_or(abs.clone())
            };
            std::process::Command::new("xdg-open").arg(dir).spawn()
        };
        if let Err(e) = result {
            self.notice = Some(format!("No se pudo abrir el gestor de archivos: {e}"));
        }
    }

    /// Open a file with whatever the system uses for it: the only way this build plays a video.
    fn open_external(&mut self, rel: &str) {
        let abs = self.store.abs(rel);
        let result = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", "start", ""]).arg(&abs).spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(&abs).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(&abs).spawn()
        };
        if let Err(e) = result {
            self.notice = Some(format!("No se pudo abrir {}: {e}", abs.display()));
        }
    }

    /// Open a link with the system's browser. Only `http(s)` is handed to the opener, so a note
    /// cannot make the application run an arbitrary command through it.
    pub(crate) fn open_in_browser(&mut self, url: &str) {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            self.notice = Some(format!("No es un enlace web: {url}"));
            return;
        }
        let result = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(url).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(url).spawn()
        };
        if let Err(e) = result {
            self.notice = Some(format!("No se pudo abrir {url} en el navegador: {e}"));
        }
    }

    /// Expand the folders above the current file and scroll the explorer to it.
    fn reveal_in_explorer(&mut self) {
        let Some(p) = self.tab().map(|t| t.path.clone()) else { return };
        let mut dir = path::parent(&p).to_string();
        while !dir.is_empty() {
            // `expanded` holds folded keys, the same ones `folder_open` looks up.
            self.expanded.insert(crate::shell::folded_path(&dir));
            dir = path::parent(&dir).to_string();
        }
        self.tree = None;
        self.run("file-explorer:open");
        self.refresh_derived();
        if let Some(i) = self.tree_rows().iter().position(|r| r.path.eq_ignore_ascii_case(&p)) {
            let y = (i as f32 * crate::shell::TREE_ROW_HEIGHT - 120.0).max(0.0);
            self.scrolls.insert("files".into(), y);
        }
    }

    fn put_clip(&mut self, text: String) {
        if self.system_clipboard {
            if let Err(e) = clipboard::set(&text) {
                self.notice = Some(format!("Copiado solo dentro de MetaMind: {e}"));
            }
        }
        self.clip = text;
    }

    fn take_clip(&mut self) -> String {
        if self.system_clipboard {
            match clipboard::get() {
                Ok(t) => return t,
                Err(e) => self.notice = Some(format!("No se pudo leer el portapapeles del sistema: {e}")),
            }
        }
        self.clip.clone()
    }

    /// Copy (or cut) the selections. With nothing selected the whole line is taken, as in most
    /// code editors.
    fn copy(&mut self, cut: bool) {
        if self.focus != Focus::Editor
            || self.show_graph
            || self.canvas_active()
            || self.base_active()
            || self.exc_active()
        {
            return;
        }
        let now = fsio::now_ms();
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let buf = &t.editor.buf;
        let text = if buf.sels.iter().all(|s| s.is_empty()) {
            let line = buf.line_of(buf.sels[0].head);
            let (start, end) = (buf.line_start(line), buf.line_end(line));
            if cut && t.mode != ViewMode::Reading {
                let next = if line + 1 < buf.line_count() { buf.line_start(line + 1) } else { end };
                let from = if next == end && line > 0 { buf.line_end(line - 1) } else { start };
                t.editor.buf.set_sels(vec![Sel::new(from, next)]);
            }
            format!("{}\n", buf_slice(&t.editor.buf, start, end))
        } else {
            let parts: Vec<String> = buf
                .sels
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| buf_slice(buf, s.start(), s.end()))
                .collect();
            parts.join("\n")
        };
        if cut && t.mode != ViewMode::Reading {
            t.editor.replace_selections("", EditKind::Other, now);
            t.last_edit_ms = now;
        }
        self.put_clip(text);
        if cut {
            self.scroll_to_cursor();
        }
    }

    fn paste(&mut self) {
        let into_note = self.focus == Focus::Editor
            && !self.canvas_active()
            && !self.exc_active()
            && !self.show_graph
            && self.tab().is_some_and(|t| t.mode != ViewMode::Reading);
        let text = if into_note && self.system_clipboard {
            match clipboard::get_paste() {
                Ok(clipboard::Paste::Text(t)) => t,
                Ok(clipboard::Paste::Files(files)) => {
                    self.import_files(&files.iter().map(|f| f.as_path()).collect::<Vec<_>>());
                    self.scroll_to_cursor();
                    return;
                }
                Ok(clipboard::Paste::Png(png)) => {
                    self.import_png(&png);
                    self.scroll_to_cursor();
                    return;
                }
                Err(e) => {
                    self.notice = Some(format!("No se pudo leer el portapapeles del sistema: {e}"));
                    self.clip.clone()
                }
            }
        } else {
            self.take_clip()
        };
        if text.is_empty() {
            return;
        }
        match self.focus {
            Focus::Prompt => {
                let line = text.lines().next().unwrap_or("").to_string();
                if let Some(p) = &mut self.prompt {
                    p.query.push_str(&line);
                    p.selected = 0;
                }
                self.refresh_prompt();
            }
            Focus::Search => self.search_query.push_str(text.lines().next().unwrap_or("")),
            Focus::Editor if self.canvas_active() => self.canvas_text(&text),
            Focus::Editor if self.exc_active() => {
                self.exc_text(&text);
            }
            Focus::Editor => {
                let now = fsio::now_ms();
                let shown = !self.show_graph;
                let Some(t) = self.tabs.get_mut(self.active).filter(|t| shown && t.mode != ViewMode::Reading)
                else {
                    return;
                };
                t.editor.replace_selections(&text, EditKind::Other, now);
                t.last_edit_ms = now;
                self.scroll_to_cursor();
            }
        }
    }

    /// Wrap the selection in `[[ ]]`, or insert an empty link with the cursor inside it.
    fn insert_wikilink(&mut self) {
        let now = fsio::now_ms();
        let Some(t) = self.tabs.get_mut(self.active).filter(|t| t.mode != ViewMode::Reading) else { return };
        let sel = t.editor.buf.sels[0];
        let inner = buf_slice(&t.editor.buf, sel.start(), sel.end());
        t.editor.buf.set_sels(vec![sel]);
        t.editor.replace_selections(&format!("[[{inner}]]"), EditKind::Other, now);
        if inner.is_empty() {
            let at = t.editor.buf.sels[0].head.saturating_sub(2);
            t.editor.buf.set_sels(vec![Sel::cursor(at)]);
        }
        t.last_edit_ms = now;
    }
}

fn buf_slice(buf: &editor_core::Buffer, start: usize, end: usize) -> String {
    buf.slice(start, end).into_owned()
}

fn file_items(b: &mut Builder<'_>, p: &str) {
    b.item("note", "Abrir", format!("open:{p}"));
    let mark = if b.app.is_bookmarked(p) { "Quitar marcador" } else { "Añadir marcador" };
    b.item("bookmark", mark, format!("bookmark:{p}"));
    b.separator();
    b.item("copy", "Hacer una copia", format!("duplicate:{p}"));
    b.item("edit", "Renombrar…", format!("rename-file:{p}"));
    b.separator();
    b.item("link", "Copiar enlace", format!("copy-link:{p}"));
    b.item("clipboard", "Copiar ruta", format!("copy-path:{p}"));
    b.item("folder", "Mostrar en el explorador del sistema", format!("reveal:{p}"));
    b.separator();
    b.warning("trash", "Eliminar", format!("delete:{p}"));
}

fn folder_items(b: &mut Builder<'_>, dir: &str) {
    b.item("note-plus", "Nota nueva", format!("new-note-in:{dir}"));
    b.item("folder-plus", "Carpeta nueva", format!("new-folder-in:{dir}"));
    if dir.is_empty() {
        b.separator();
        b.item("collapse", "Contraer todo", "file-explorer:collapse-all");
        b.item("folder", "Mostrar en el explorador del sistema", "reveal:");
        return;
    }
    b.separator();
    b.item("edit", "Renombrar…", format!("rename-file:{dir}"));
    b.item("clipboard", "Copiar ruta", format!("copy-path:{dir}"));
    b.item("folder", "Mostrar en el explorador del sistema", format!("reveal:{dir}"));
    b.separator();
    b.warning("trash", "Eliminar", format!("delete:{dir}"));
}

fn tab_items(b: &mut Builder<'_>, i: usize) {
    let count = b.app.tabs.len();
    let p = b.app.tabs.get(i).map(|t| t.path.clone()).unwrap_or_default();
    let pinned = b.app.tabs.get(i).is_some_and(|t| t.pinned);
    if !pinned {
        b.item("x", "Cerrar", format!("close-tab:{i}"));
    }
    if count > 1 {
        b.item("x", "Cerrar las demás", format!("close-others:{i}"));
    }
    if i + 1 < count {
        b.item("arrow-right", "Cerrar las pestañas a la derecha", format!("close-right:{i}"));
    }
    b.item("x", "Cerrar todas", "workspace:close-all");
    b.separator();
    if i == b.app.active && !p.is_empty() {
        b.item("separator-vertical", "Dividir a la derecha", "workspace:split-vertical");
        b.item("separator-horizontal", "Dividir abajo", "workspace:split-horizontal");
        b.item("arrow-up-right", "Mover a una ventana nueva", "workspace:move-to-new-window");
        let stacked =
            if b.app.panes[b.app.pane].stacked { "Desapilar pestañas" } else { "Apilar pestañas" };
        b.item("list", stacked, "workspace:toggle-stacked-tabs");
        let app = b.app;
        let group = app.panes[app.pane].link;
        let others: Vec<(usize, String)> = (0..app.panes.len())
            .filter(|&j| j != app.pane && (group.is_none() || app.panes[j].link != group))
            .filter_map(|j| {
                let (tabs, active) = app.pane_tabs(j);
                Some((j, tabs.get(active)?.title().to_string()))
            })
            .collect();
        for (j, title) in others {
            b.item("link", &format!("Vincular con “{title}”"), format!("link-pane:{j}"));
        }
        if group.is_some() {
            b.item("unlink", "Desvincular pestaña", "workspace:unlink-pane");
        }
        b.separator();
    }
    b.item("pin", if pinned { "Desanclar" } else { "Anclar" }, format!("toggle-pin:{i}"));
    if !p.is_empty() {
        let mark = if b.app.is_bookmarked(&p) { "Quitar marcador" } else { "Añadir marcador" };
        b.item("bookmark", mark, format!("bookmark:{p}"));
    }
    b.separator();
    if !p.is_empty() {
        b.item("edit", "Renombrar…", format!("rename-file:{p}"));
        b.item("link", "Copiar enlace", format!("copy-link:{p}"));
        b.item("clipboard", "Copiar ruta", format!("copy-path:{p}"));
        b.item("folder", "Mostrar en el explorador del sistema", format!("reveal:{p}"));
        if i == b.app.active {
            b.item("book", "Exportar a PDF", "workspace:export-pdf");
            if !b.app.show_graph && !b.app.canvas_active() && !b.app.base_active() && !b.app.exc_active() {
                b.item("image", "Copiar captura de pantalla", "workspace:copy-screenshot");
            }
        }
        b.separator();
    }
    b.item("arrow-up", "Reabrir pestaña cerrada", "workspace:undo-close-pane");
}

fn editor_items(b: &mut Builder<'_>, typo: Option<(usize, usize)>) {
    let reading = b.app.tab().is_some_and(|t| t.mode == ViewMode::Reading);
    if let Some((start, end)) = typo.filter(|_| !reading) {
        let word = b.app.tab().map(|t| t.editor.buf.slice(start, end).into_owned()).unwrap_or_default();
        for s in crate::spell::suggest(&word) {
            b.item("blank", &s, format!("spell:replace:{start}:{end}:{s}"));
        }
        b.item("plus", "Añadir al diccionario", format!("spell:add:{word}"));
        b.separator();
    }
    if b.app.link_under_cursor().is_some() {
        b.item("link", "Abrir enlace", "editor:follow-link");
        b.separator();
    }
    if !reading {
        b.item("scissors", "Cortar", "editor:cut");
    }
    b.item("copy", "Copiar", "editor:copy");
    if !reading {
        b.item("clipboard", "Pegar", "editor:paste");
        b.separator();
        b.item("link", "Añadir enlace interno", "editor:insert-wikilink");
        b.item("bold", "Negrita", "editor:toggle-bold");
        b.item("italic", "Cursiva", "editor:toggle-italics");
        b.item("strikethrough", "Tachado", "editor:toggle-strikethrough");
        b.item("highlight", "Resaltado", "editor:toggle-highlight");
        b.item("code", "Código", "editor:toggle-code");
        b.separator();
        b.item("check", "Alternar casilla", "editor:toggle-checklist-status");
        b.item("table", "Insertar tabla", "editor:insert-table");
    }
    b.separator();
    b.item("select", "Seleccionar todo", "editor:select-all");
    b.item(
        if reading { "edit" } else { "book" },
        if reading { "Editar" } else { "Vista de lectura" },
        "markdown:toggle-preview",
    );
}
