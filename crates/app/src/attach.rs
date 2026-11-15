//! Files dropped onto the window from outside: copied into the vault's attachment folder and
//! linked from the active note at the cursor.

use crate::shell::App;
use editor_core::EditKind;
use std::path::Path;
use vault_core::{fsio, path};

impl App {
    /// Where new attachments go for a note at `source`, from `attachmentFolderPath`: `/` is the
    /// vault root, `./` the note's folder, `./sub` a folder below it, anything else a vault path.
    fn attachment_dir(&self, source: &str) -> String {
        let setting = self.config.attachment_folder().trim();
        match setting.strip_prefix("./") {
            Some(sub) => path::join(path::parent(source), sub.trim_matches('/')),
            None if setting == "." => path::parent(source).to_string(),
            None => setting.trim_matches('/').to_string(),
        }
    }

    /// Bring dropped files into the vault and link them from the active note. Files already
    /// inside the vault are linked where they are.
    pub fn import_files(&mut self, files: &[&Path]) {
        let source = self.tab().map(|t| t.path.clone()).filter(|p| p.ends_with(".md"));
        let dir = self.attachment_dir(source.as_deref().unwrap_or(""));
        let mut added = Vec::new();
        for file in files {
            let rel = match file.strip_prefix(&self.store.root) {
                Ok(inside) => path::normalize(&inside.to_string_lossy().replace('\\', "/")),
                Err(_) => match self.copy_in(file, &dir) {
                    Ok(rel) => rel,
                    Err(e) => {
                        self.notice = Some(format!("Cannot import {}: {e}", file.display()));
                        continue;
                    }
                },
            };
            added.push(rel);
        }
        if added.is_empty() {
            return;
        }
        self.files_changed(&[]);
        let Some(source) = source else {
            self.notice = Some(match added.as_slice() {
                [one] => format!("Added {one}"),
                many => format!("Added {} files", many.len()),
            });
            return;
        };
        let (format, markdown) = (self.store.index.link_format, self.config.use_markdown_links());
        let links: Vec<String> = added
            .iter()
            .filter_map(|rel| {
                let id = self.store.index.id(rel)?;
                let target = self.store.index.link_text(id, &source, format, markdown);
                let bang = if rel.ends_with(".md") { "" } else { "!" };
                Some(if markdown {
                    format!("{bang}[{}]({target})", path::stem(rel))
                } else {
                    format!("{bang}[[{target}]]")
                })
            })
            .collect();
        let now = fsio::now_ms();
        if let Some(t) = self.tab_mut() {
            t.editor.replace_selections(&links.join("\n"), EditKind::Other, now);
            t.last_edit_ms = now;
        }
    }

    /// Copy an outside file into `dir`, adding a counter to the name if it is taken.
    fn copy_in(&mut self, file: &Path, dir: &str) -> std::io::Result<String> {
        if !file.is_file() {
            return Err(std::io::Error::other("only files can be added"));
        }
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (stem, ext) = (path::stem(&name), path::extension(&name));
        let taken = |p: &str| self.store.index.id(p).is_some() || self.store.abs(p).exists();
        let mut rel = path::join(dir, &name);
        let mut n = 1;
        while taken(&rel) {
            let numbered = if ext.is_empty() { format!("{stem} {n}") } else { format!("{stem} {n}.{ext}") };
            rel = path::join(dir, &numbered);
            n += 1;
        }
        let abs = self.store.abs(&rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(file, &abs)?;
        self.store.refresh(&rel);
        self.plugins.push(ext_api::Event::Create { path: rel.clone() });
        Ok(rel)
    }
}
