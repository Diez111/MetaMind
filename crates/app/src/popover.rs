//! Previews of notes: Ctrl+hover over a link in the editor shows the start of its target.

use crate::shell::App;
use crate::{view, Frame};
use rustsidian_ui::Fonts;
use vault_core::path;

const PREVIEW_LINES: usize = 12;

pub struct HoverPreview {
    pub path: String,
    /// Pointer position the popover opens from.
    pub at: (f32, f32),
    pub text: String,
}

/// The opening non-blank lines of a note's body, without its front matter.
pub fn note_preview(text: &str) -> String {
    let body = strip_front_matter(text);
    body.lines().filter(|l| !l.trim().is_empty()).take(PREVIEW_LINES).collect::<Vec<_>>().join("\n")
}

fn strip_front_matter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return text };
    rest.find("\n---").map_or(text, |i| rest[i + 4..].split_once('\n').map_or("", |(_, b)| b))
}

impl App {
    /// Preview of the Markdown file at `rel`, empty for other files or when it cannot be read.
    pub(crate) fn preview_of(&self, rel: &str) -> String {
        if !path::extension(rel).eq_ignore_ascii_case("md") {
            return String::new();
        }
        self.store.read(rel).map(|(t, _)| note_preview(&t)).unwrap_or_default()
    }

    /// Show or hide the link preview for the pointer at (x, y). Returns true when it changed.
    pub fn hover_link(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32, ctrl: bool) -> bool {
        let target = ctrl.then(|| self.link_target_at(frame, fonts, x, y)).flatten();
        let same = match (&self.hover_preview, &target) {
            (Some(h), Some(p)) => &h.path == p,
            (None, None) => true,
            _ => false,
        };
        if same {
            return false;
        }
        self.hover_preview = target.map(|path| {
            let text = self.preview_of(&path);
            HoverPreview { path, at: (x, y), text }
        });
        true
    }

    /// Vault path of the resolved link under the pointer in the editor.
    fn link_target_at(&self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32) -> Option<String> {
        let t = self.tab().filter(|_| !self.show_graph)?;
        let hit = frame.dom.hit(x, y)?;
        frame.info.blocks.iter().find(|b| b.node == hit)?;
        let off = view::offset_at(&frame.dom, &frame.info, fonts, x, y)? as u32;
        let text = t.editor.buf.text();
        let meta = vault_core::meta::extract(&text);
        let link = meta.links.iter().find(|l| l.start <= off && off < l.end)?;
        let id = self.store.index.resolve(&link.target, &t.path)?;
        self.store.index.entry(id).map(|f| f.path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_skips_front_matter_and_blank_lines() {
        assert_eq!(note_preview("---\ntags: [a]\n---\n# T\n\nbody\n"), "# T\nbody");
        assert_eq!(note_preview("plain"), "plain");
    }
}
