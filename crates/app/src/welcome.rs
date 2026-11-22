//! The welcome screen shown when MetaMind starts without a vault: recent vaults, a folder
//! browser to open or create a vault, and a sample vault for a first look.

use crate::anim::{Anims, FAST_MS, SLOW_MS};
use crate::{icons, recent};
use metamind_ui::{Dom, El, Fonts, NodeId, Pixmap, Theme};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use theme_engine::Env;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    /// Browse for an existing folder to open.
    Open,
    /// Browse for the folder a new vault is created in.
    Create,
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub name: String,
    pub path: PathBuf,
    /// Has a `.obsidian` folder, so it has been used as a vault before.
    pub vault: bool,
}

pub struct Welcome {
    pub screen: Screen,
    pub recents: Vec<PathBuf>,
    /// The folder being browsed; `None` is the list of drives (Windows).
    pub dir: Option<PathBuf>,
    pub folders: Vec<Folder>,
    /// Name typed for a new vault.
    pub name: String,
    pub error: Option<String>,
    pub hover_action: Option<String>,
    /// Keyboard selection on the home screen: the three actions, then the recent vaults.
    pub sel: Option<usize>,
    /// The system theme; the window sets it.
    pub dark: bool,
    pub scrolls: HashMap<String, f32>,
    pub anims: Anims,
    /// The OS asks for more contrast; the window sets it.
    pub high_contrast: bool,
    /// A note to show first in the vault that was just chosen (the sample vault's welcome note).
    pub open_note: Option<String>,
    /// A vault is being indexed for the first time ("n/total" progress in the corner).
    pub notice: Option<String>,
    theme: Theme,
}

const SAMPLE_NAME: &str = "MetaMind Vault";
const SAMPLE: &[(&str, &str)] = &[
    ("Welcome.md", include_str!("sample/Welcome.md")),
    ("Getting started/Keyboard shortcuts.md", include_str!("sample/Keyboard shortcuts.md")),
    ("Getting started/Markdown basics.md", include_str!("sample/Markdown basics.md")),
    ("Getting started/Links and the graph.md", include_str!("sample/Links and the graph.md")),
];

fn drives() -> Vec<Folder> {
    (b'A'..=b'Z')
        .map(|l| PathBuf::from(format!("{}:\\", l as char)))
        .filter(|p| p.is_dir())
        .map(|p| Folder { name: p.to_string_lossy().into_owned(), path: p, vault: false })
        .collect()
}

fn list(dir: &Path) -> Vec<Folder> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Folder> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with('$') {
                return None;
            }
            let path = e.path();
            path.is_dir().then(|| Folder { vault: path.join(".obsidian").is_dir(), name, path })
        })
        .collect();
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

/// The recent vault's path with the home folder cut to `~`.
fn short_path(path: &Path) -> String {
    let full = path.to_string_lossy();
    if let Some(home) = recent::home_dir() {
        let home = home.to_string_lossy();
        if let Some(rest) = full.strip_prefix(home.as_ref()) {
            return if rest.is_empty() { "~".to_string() } else { format!("~{rest}") };
        }
    }
    full.into_owned()
}

impl Default for Welcome {
    fn default() -> Self {
        Self::new()
    }
}

impl Welcome {
    pub fn new() -> Welcome {
        let mut theme = Theme::new();
        theme.add_sheet("base", crate::shell::BASE_CSS);
        theme.add_sheet("contrast", crate::shell::CONTRAST_CSS);
        let mut w = Welcome {
            screen: Screen::Home,
            recents: recent::load(),
            dir: None,
            folders: Vec::new(),
            name: String::new(),
            error: None,
            hover_action: None,
            sel: None,
            dark: true,
            scrolls: HashMap::new(),
            anims: Anims::default(),
            high_contrast: false,
            open_note: None,
            notice: None,
            theme,
        };
        w.go(recent::home_dir());
        w
    }

    /// Browse a folder (`None` is the list of drives).
    fn go(&mut self, dir: Option<PathBuf>) {
        self.folders = match &dir {
            Some(d) => list(d),
            None => drives(),
        };
        self.dir = dir;
        self.scrolls.remove("folders");
        self.error = None;
    }

    fn show(&mut self, screen: Screen) {
        self.screen = screen;
        self.error = None;
        self.sel = None;
        self.anims.start("screen", SLOW_MS);
    }

    /// Where the sample vault goes: Documents when there is one, else the home folder.
    fn sample_path() -> Option<PathBuf> {
        let home = recent::home_dir()?;
        let docs = home.join("Documents");
        Some(if docs.is_dir() { docs } else { home }.join(SAMPLE_NAME))
    }

    fn create_sample(&mut self) -> Option<PathBuf> {
        let Some(root) = Self::sample_path() else {
            self.error =
                Some("No se pudo encontrar tu carpeta personal para crear la bóveda de ejemplo.".to_string());
            return None;
        };
        for (rel, text) in SAMPLE {
            let file = root.join(rel);
            if file.exists() {
                continue;
            }
            let made = file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&file, text));
            if let Err(e) = made {
                self.error = Some(format!("No se pudo crear {}: {e}", root.display()));
                return None;
            }
        }
        self.open_note = Some(SAMPLE[0].0.to_string());
        Some(root)
    }

    fn create_named(&mut self) -> Option<PathBuf> {
        let name = self.name.trim();
        let Some(dir) = &self.dir else {
            self.error = Some("Elige primero una carpeta donde crear la bóveda.".to_string());
            return None;
        };
        if name.is_empty() {
            self.error = Some("Escribe un nombre para la bóveda nueva.".to_string());
            return None;
        }
        if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || name.starts_with('.') {
            self.error = Some("Ese nombre tiene caracteres que no puede tener una carpeta.".to_string());
            return None;
        }
        let root = dir.join(name);
        if root.exists() {
            self.error =
                Some(format!("{} ya existe. Usa \"Abrir carpeta como bóveda\" para ella.", root.display()));
            return None;
        }
        if let Err(e) = std::fs::create_dir_all(&root) {
            self.error = Some(format!("No se pudo crear {}: {e}", root.display()));
            return None;
        }
        Some(root)
    }

    /// Run an action. Returns the vault to open when one was chosen.
    pub fn run(&mut self, action: &str) -> Option<PathBuf> {
        if let Some(i) = action.strip_prefix("recent:").and_then(|i| i.parse::<usize>().ok()) {
            return self.recents.get(i).cloned();
        }
        if let Some(i) = action.strip_prefix("forget:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(p) = self.recents.get(i) {
                recent::forget(p);
                self.recents = recent::load();
            }
            return None;
        }
        if let Some(i) = action.strip_prefix("enter:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(f) = self.folders.get(i) {
                let path = f.path.clone();
                self.go(Some(path));
            }
            return None;
        }
        match action {
            "welcome:open" => self.show(Screen::Open),
            "welcome:create" => {
                self.name.clear();
                self.show(Screen::Create);
            }
            "welcome:sample" => return self.create_sample(),
            "welcome:back" => self.show(Screen::Home),
            "welcome:up" => {
                let parent = self.dir.as_ref().and_then(|d| d.parent().map(Path::to_path_buf));
                if parent.is_some() || cfg!(windows) {
                    self.go(parent);
                }
            }
            "welcome:home" => self.go(recent::home_dir()),
            "welcome:choose" => match self.screen {
                Screen::Open => {
                    if self.dir.is_none() {
                        self.error = Some("Elige primero una carpeta.".to_string());
                    }
                    return self.dir.clone();
                }
                Screen::Create => return self.create_named(),
                Screen::Home => {}
            },
            _ => {}
        }
        None
    }

    pub fn click(&mut self, dom: &Dom, x: f32, y: f32) -> Option<PathBuf> {
        let action = dom.action_at(x, y)?.1.to_string();
        self.run(&action)
    }

    /// A named key (not text). Returns the vault to open when one was chosen.
    pub fn key(&mut self, name: &str) -> Option<PathBuf> {
        match name {
            "ArrowDown" | "Tab" if self.screen == Screen::Home => self.step(true),
            "ArrowUp" if self.screen == Screen::Home => self.step(false),
            "Enter" if self.screen == Screen::Home => {
                if let Some(action) = self.selected() {
                    return self.run(&action);
                }
            }
            "Escape" if self.screen != Screen::Home => self.show(Screen::Home),
            "Enter" if self.screen != Screen::Home => return self.run("welcome:choose"),
            "Backspace" if self.screen == Screen::Create => {
                self.name.pop();
            }
            "Backspace" if self.screen == Screen::Open => {
                self.run("welcome:up");
            }
            _ => {}
        }
        None
    }

    /// The three home actions, in the order they are drawn.
    const ACTIONS: [(&'static str, &'static str, &'static str); 3] = [
        ("folder", "Abrir bóveda", "welcome:open"),
        ("folder-plus", "Crear bóveda", "welcome:create"),
        ("sparkles", "Bóveda de ejemplo", "welcome:sample"),
    ];

    /// Move the keyboard selection through the actions and the recent vaults.
    fn step(&mut self, down: bool) {
        let n = Self::ACTIONS.len() + self.recents.len();
        self.sel = Some(match (self.sel, down) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        });
    }

    /// The action the keyboard selection points at.
    fn selected(&self) -> Option<String> {
        let i = self.sel?;
        if i < Self::ACTIONS.len() {
            return Some(Self::ACTIONS[i].2.to_string());
        }
        Some(format!("recent:{}", i - Self::ACTIONS.len()))
    }

    pub fn text_input(&mut self, text: &str) {
        if self.screen == Screen::Create {
            self.name.extend(text.chars().filter(|c| !c.is_control()));
            self.error = None;
        }
    }

    pub fn wheel(&mut self, dom: &Dom, x: f32, y: f32, lines: f32) {
        let Some(node) = dom.scroller_at(x, y) else { return };
        if let Some(id) = dom.nodes[node].el.id.clone() {
            let cur = dom.nodes[node].scroll_y;
            self.scrolls.insert(id, (cur + lines * 40.0).clamp(0.0, dom.max_scroll(node)));
        }
    }

    /// The pointer moved. Returns true when the frame must be redrawn.
    pub fn hover(&mut self, dom: &Dom, x: f32, y: f32) -> bool {
        let action = dom.action_at(x, y).map(|(_, a)| a.to_string());
        let changed = action != self.hover_action;
        self.hover_action = action;
        changed
    }

    fn home(&self, dom: &mut Dom, main: NodeId) {
        let actions = dom.add(main, El::div().class("welcome-actions"));
        for (i, (icon, title, action)) in Self::ACTIONS.iter().enumerate() {
            let row = dom.add(
                actions,
                El::div()
                    .class("welcome-action")
                    .class_if(self.sel == Some(i), "is-focused")
                    .action(*action),
            );
            dom.add(row, icons::el(icon).class("welcome-action-icon"));
            dom.add(row, El::div().class("welcome-action-title").text(*title));
            dom.add(row, icons::el("chevron-right").class("welcome-action-go"));
        }
        if self.recents.is_empty() {
            return;
        }
        let recents = dom.add(main, El::div().class("welcome-recents"));
        dom.add(recents, El::div().class("welcome-section-title").text("Recientes"));
        let list = dom.add(recents, El::div().class("welcome-list").id("recents"));
        for (i, p) in self.recents.iter().enumerate() {
            let row = dom.add(
                list,
                El::div()
                    .class("welcome-row")
                    .class_if(self.sel == Some(Self::ACTIONS.len() + i), "is-focused")
                    .action(format!("recent:{i}")),
            );
            dom.add(row, icons::el("vault").class("welcome-row-icon"));
            let texts = dom.add(row, El::div().class("welcome-row-text"));
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            dom.add(texts, El::div().class("welcome-row-title").text(name));
            dom.add(texts, El::div().class("welcome-row-path").text(short_path(p)));
            let x = dom.add(row, El::div().class("clickable-icon").action(format!("forget:{i}")));
            dom.add(x, icons::el("x"));
        }
    }

    fn browser(&self, dom: &mut Dom, main: NodeId) {
        let create = self.screen == Screen::Create;
        let header = dom.add(main, El::div().class("welcome-browser-header"));
        let back = dom.add(header, El::div().class("clickable-icon").action("welcome:back"));
        dom.add(back, icons::el("arrow-left"));
        let title = if create { "Crear bóveda nueva" } else { "Abrir carpeta como bóveda" };
        dom.add(header, El::div().class("welcome-browser-title").text(title));
        let bar = dom.add(main, El::div().class("welcome-path"));
        for (icon, action) in [("arrow-up", "welcome:up"), ("home", "welcome:home")] {
            let b = dom.add(bar, El::div().class("clickable-icon").action(action));
            dom.add(b, icons::el(icon));
        }
        let shown = self.dir.as_ref().map_or("Este equipo".to_string(), |d| d.to_string_lossy().into_owned());
        dom.add(bar, El::div().class("welcome-path-text").text(shown));
        let list = dom.add(main, El::div().class("welcome-list welcome-folders").id("folders"));
        if self.folders.is_empty() {
            dom.add(list, El::div().class("welcome-empty").text("No hay carpetas aquí."));
        }
        for (i, f) in self.folders.iter().enumerate() {
            let row = dom.add(list, El::div().class("welcome-row").action(format!("enter:{i}")));
            let icon = if self.dir.is_none() {
                "drive"
            } else if f.vault {
                "vault"
            } else {
                "folder"
            };
            dom.add(row, icons::el(icon).class(if f.vault { "is-vault" } else { "" }));
            dom.add(row, El::div().class("welcome-row-title").text(f.name.clone()));
            if f.vault {
                dom.add(row, El::div().class("welcome-tag").text("bóveda"));
            }
            dom.add(row, icons::el("chevron-right"));
        }
        let footer = dom.add(main, El::div().class("welcome-footer"));
        if create {
            let shown = if self.name.is_empty() {
                "Nombre de la bóveda▏".to_string()
            } else {
                crate::view::with_caret(&self.name, 40)
            };
            dom.add(
                footer,
                El::div().class("welcome-input").class_if(self.name.is_empty(), "is-empty").text(shown),
            );
            dom.add(footer, El::div().class("welcome-button mod-cta").text("Crear").action("welcome:choose"));
        } else {
            let here =
                self.dir.as_ref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
            dom.add(
                footer,
                El::div()
                    .class("welcome-footer-hint")
                    .text("Entra en la carpeta que contiene tus notas y ábrela."),
            );
            let label = here.map_or("Abrir esta carpeta".to_string(), |n| format!("Abrir \"{n}\""));
            dom.add(footer, El::div().class("welcome-button mod-cta").text(label).action("welcome:choose"));
        }
    }

    pub fn build(&self) -> Dom {
        let mut dom = Dom::new();
        dom.nodes[0].el.classes.push(if self.dark { "theme-dark" } else { "theme-light" }.to_string());
        let t = self.anims.t("intro");
        let wrap = El::div().class("welcome");

        let wrap = dom.add(0, wrap);
        let mut card = El::div().class("welcome-card");
        if self.screen != Screen::Home {
            card = card.class("mod-browser");
        } else if self.recents.len() > 6 {
            // Past a handful of vaults the list scrolls instead of growing the card off screen.
            card = card.class("mod-scroll");
        }
        if t < 1.0 {
            card = card.style(&format!("margin-top: {:.1}px", 18.0 * (1.0 - t)));
        }
        let card = dom.add(wrap, card);
        if self.screen == Screen::Home {
            let brand = dom.add(card, El::div().class("welcome-brand"));
            dom.add(brand, icons::el("vault").class("welcome-logo"));
            dom.add(brand, El::div().class("welcome-name").text("MetaMind"));
            dom.add(brand, El::div().class("welcome-data").text("50 000 notas · abre en 0,2 s"));
            dom.add(
                brand,
                El::div().class("welcome-version").text(format!("Versión {}", env!("CARGO_PKG_VERSION"))),
            );
        }
        let main = dom.add(card, El::div().class("welcome-main"));
        match self.screen {
            Screen::Home => self.home(&mut dom, main),
            Screen::Open | Screen::Create => self.browser(&mut dom, main),
        }
        if let Some(e) = &self.error {
            dom.add(main, El::div().class("welcome-error").text(e.clone()));
        }
        for (id, y) in &self.scrolls {
            if let Some(n) = dom.by_id(id) {
                dom.nodes[n].scroll_y = *y;
            }
        }
        if let Some(n) = &self.notice {
            dom.add(0, El::div().class("notice").text(n.clone()));
        }
        crate::view::mark_hover(&mut dom, self.hover_action.as_deref());
        dom
    }

    /// Build, style, lay out and paint the welcome screen into `pixmap`.
    pub fn render(&mut self, fonts: &mut Fonts, pixmap: &mut Pixmap, scale: f32) -> Dom {
        self.anims.tick();
        let mut dom = self.build();
        dom.style(&self.theme, Env { dark: self.dark, high_contrast: self.high_contrast, ..Env::default() });
        dom.scrollbar_thumb = crate::scrollbar_thumb(&dom);
        dom.layout(fonts, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
        icons::paint(&mut dom);
        let scene = dom.paint();
        let clear = dom.nodes[0].style.background;
        metamind_ui::render(&scene, fonts, pixmap, scale, clear);
        dom
    }

    /// Start the opening animation (the window calls this once it can animate).
    pub fn intro(&mut self) {
        self.anims.enabled = true;
        self.anims.start("intro", SLOW_MS + FAST_MS);
    }
}
