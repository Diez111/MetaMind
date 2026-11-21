//! Settings window: General, Editor, Files & links, Appearance, Interface, Hotkeys and Core
//! plugins, with a search across all of them. Every control writes
//! the same `.obsidian/*.json` key the vault format uses (other keys are left as they are) and takes
//! effect at once.

use crate::shell::{default_keymap, load_theme, App, Key, COMMANDS};
use rustsidian_ui::{Dom, El, NodeId};
use serde_json::{json, Value};
use vault_core::config::JsonFile;
use vault_core::fsio::TrashMode;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Editor,
    Files,
    Appearance,
    Interface,
    Hotkeys,
    CorePlugins,
}

const SECTIONS: [(Section, &str, &str); 7] = [
    (Section::General, "general", "General"),
    (Section::Editor, "editor", "Editor"),
    (Section::Files, "files", "Archivos y enlaces"),
    (Section::Appearance, "appearance", "Apariencia"),
    (Section::Interface, "interface", "Interfaz"),
    (Section::Hotkeys, "hotkeys", "Atajos de teclado"),
    (Section::CorePlugins, "core-plugins", "Plugins principales"),
];

/// Core plugins this app honors: (id in `core-plugins.json`, name, description, the commands
/// that are blocked and hidden while it is off).
pub(crate) const CORE_PLUGINS: &[(&str, &str, &str, &[&str])] = &[
    (
        "backlink",
        "Retroenlaces",
        "Muestra en la barra lateral derecha los enlaces a la nota activa.",
        &["backlinks:open"],
    ),
    ("bookmarks", "Marcadores", "Ten notas y búsquedas a mano.", &["bookmarks:open"]),
    ("canvas", "Canvas", "Coloca notas y tarjetas en un tablero.", &["canvas:new"]),
    ("bases", "Bases", "Muestra notas como tablas y tarjetas a partir de sus propiedades.", &["bases:new"]),
    (
        "command-palette",
        "Paleta de comandos",
        "Ejecuta cualquier comando por su nombre.",
        &["command-palette:open"],
    ),
    (
        "daily-notes",
        "Notas diarias",
        "Abre una nota para hoy.",
        &["daily-notes", "daily-notes:goto-prev", "daily-notes:goto-next"],
    ),
    (
        "file-explorer",
        "Archivos",
        "Lista los archivos de la bóveda en la barra lateral izquierda.",
        &["file-explorer:open"],
    ),
    (
        "graph",
        "Vista de gráfico",
        "Muestra los enlaces entre notas como un gráfico.",
        &["graph:open", "graph:open-local"],
    ),
    (
        "outgoing-link",
        "Enlaces salientes",
        "Muestra los enlaces de la nota activa.",
        &["outgoing-links:open"],
    ),
    ("outline", "Esquema", "Muestra los encabezados de la nota activa.", &["outline:open"]),
    ("switcher", "Selector rápido", "Salta a una nota por su nombre.", &["switcher:open"]),
    ("global-search", "Buscar", "Busca en la bóveda con operadores de consulta.", &["global-search:open"]),
    (
        "workspaces",
        "Espacios de trabajo",
        "Guarda y carga disposiciones de pestañas y paneles.",
        &["workspaces:save", "workspaces:load", "workspaces:delete"],
    ),
    (
        "tag-pane",
        "Vista de etiquetas",
        "Lista las etiquetas de la bóveda con sus recuentos.",
        &["tag-pane:open"],
    ),
    (
        "templates",
        "Plantillas",
        "Inserta una plantilla, la fecha o la hora.",
        &["templates:insert-template", "templates:insert-current-date", "templates:insert-current-time"],
    ),
];

pub struct SettingsUi {
    pub section: Section,
    /// The text field being edited: (setting key, text so far).
    pub edit: Option<(String, String)>,
    /// The command whose hotkey is waiting for a key press.
    pub capture: Option<String>,
    pub filter: String,
    /// The settings search; while it has text, matching settings from every section are shown.
    pub search: String,
    pub themes: Vec<String>,
    pub snippets: Vec<String>,
}

impl SettingsUi {
    fn new(app: &App) -> SettingsUi {
        let mut s = SettingsUi {
            section: Section::General,
            edit: None,
            capture: None,
            filter: String::new(),
            search: String::new(),
            themes: Vec::new(),
            snippets: Vec::new(),
        };
        s.scan(app);
        s
    }

    fn scan(&mut self, app: &App) {
        let list = |sub: &str| -> Vec<std::path::PathBuf> {
            let mut v: Vec<_> = std::fs::read_dir(app.config.dir.join(sub))
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            v.sort();
            v
        };
        self.themes = list("themes")
            .into_iter()
            .filter(|p| p.join("theme.css").is_file())
            .filter_map(|p| Some(p.file_name()?.to_str()?.to_string()))
            .collect();
        self.snippets = list("snippets")
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "css"))
            .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
            .collect();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum File {
    App,
    Appearance,
}

enum Control {
    Toggle(bool),
    Choice(&'static str, &'static [(&'static str, &'static str)]),
    Text,
    Number(f64, f64, f64),
}

struct Item {
    section: Section,
    file: File,
    key: &'static str,
    name: &'static str,
    desc: &'static str,
    control: Control,
}

use Control::{Choice, Number, Text, Toggle};
use Section::{Appearance, Editor, Files, Interface};

const ITEMS: &[Item] = &[
    Item {
        section: Interface,
        file: File::Appearance,
        key: "showRibbon",
        name: "Mostrar barra de iconos",
        desc: "Muestra la columna de botones en el borde izquierdo de la ventana.",
        control: Toggle(true),
    },
    Item {
        section: Interface,
        file: File::Appearance,
        key: "showViewHeader",
        name: "Mostrar barra de título de la pestaña",
        desc: "Muestra la barra con el título de la nota y los botones de vista sobre cada nota.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "vimMode",
        name: "Atajos de teclado Vim",
        desc: "Usa teclas al estilo Vim en el editor.",
        control: Toggle(false),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "readableLineLength",
        name: "Longitud de línea legible",
        desc: "Limita el ancho de las líneas en el editor.",
        control: Toggle(false),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "spellcheck",
        name: "Corrección ortográfica",
        desc: "Subraya palabras mal escritas en inglés. Haz clic derecho en una para ver sugerencias.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "propertiesInDocument",
        name: "Propiedades en el documento",
        desc: "Muestra el front matter como tabla de propiedades, como código fuente o no lo muestres.",
        control: Choice("visible", &[("visible", "Visibles"), ("source", "Código fuente"), ("hidden", "Ocultas")]),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "useTab",
        name: "Sangrar con tabulaciones",
        desc: "Tab inserta un carácter de tabulación en lugar de espacios.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "tabSize",
        name: "Tamaño de la sangría",
        desc: "Número de espacios que vale un nivel de sangría.",
        control: Number(4.0, 1.0, 16.0),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "autoPairBrackets",
        name: "Cerrar paréntesis automáticamente",
        desc: "Al escribir ( [ { o una comilla también se inserta el cierre.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "autoPairMarkdown",
        name: "Cerrar sintaxis Markdown automáticamente",
        desc: "Al escribir * _ ` o ~ también se inserta el marcador de cierre.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "smartIndentList",
        name: "Listas inteligentes",
        desc: "Intro continúa un elemento de lista y Backspace elimina uno vacío.",
        control: Toggle(true),
    },
    Item {
        section: Files,
        file: File::App,
        key: "newFileLocation",
        name: "Ubicación predeterminada de las notas nuevas",
        desc: "Dónde guarda el archivo la orden «Crear nota».",
        control: Choice(
            "root",
            &[
                ("root", "Carpeta de la bóveda"),
                ("current", "La misma carpeta que el archivo actual"),
                ("folder", "En la carpeta indicada abajo"),
            ],
        ),
    },
    Item {
        section: Files,
        file: File::App,
        key: "newFileFolderPath",
        name: "Carpeta para notas nuevas",
        desc: "Se usa cuando la ubicación anterior es una carpeta concreta.",
        control: Text,
    },
    Item {
        section: Files,
        file: File::App,
        key: "newLinkFormat",
        name: "Formato de los enlaces nuevos",
        desc: "Cómo se escriben los enlaces al renombrar o mover un archivo.",
        control: Choice(
            "shortest",
            &[
                ("shortest", "Ruta más corta cuando sea posible"),
                ("relative", "Ruta relativa al archivo"),
                ("absolute", "Ruta absoluta en la bóveda"),
            ],
        ),
    },
    Item {
        section: Files,
        file: File::App,
        key: "trashOption",
        name: "Archivos eliminados",
        desc: "Qué pasa cuando eliminas una nota.",
        control: Choice(
            "local",
            &[
                ("system", "Mover a la papelera del sistema"),
                ("local", "Mover a la carpeta .trash de la bóveda"),
                ("none", "Eliminar permanentemente"),
            ],
        ),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "theme",
        name: "Esquema de color base",
        desc: "Base clara u oscura para la interfaz.",
        control: Choice(
            "obsidian",
            &[("obsidian", "Oscuro"), ("moonstone", "Claro"), ("system", "Adaptarse al sistema")],
        ),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "baseFontSize",
        name: "Tamaño de fuente",
        desc: "Tamaño del texto de las notas, en píxeles.",
        control: Number(16.0, 10.0, 30.0),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "textFontFamily",
        name: "Fuente del texto",
        desc: "Fuente del texto de las notas, como lista separada por comas. Se usa la primera fuente instalada.",
        control: Text,
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "accentColor",
        name: "Color de acento",
        desc: "Color de enlaces, etiquetas y resaltados, por ejemplo #8a5cf5. Vacío usa el predeterminado.",
        control: Text,
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "lineWidth",
        name: "Ancho de línea",
        desc: "Ancho máximo del texto de las notas en píxeles cuando «Longitud de línea legible» está activado.",
        control: Number(800.0, 400.0, 2400.0),
    },
];

fn item(key: &str) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.key == key)
}

fn number_json(n: f64) -> Value {
    if n.fract() == 0.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// Editor options from `app.json`.
pub fn editor_settings(config: &vault_core::config::VaultConfig) -> editor_core::Settings {
    let a = &config.app;
    editor_core::Settings {
        auto_pair_brackets: a.bool("autoPairBrackets", true),
        auto_pair_markdown: a.bool("autoPairMarkdown", true),
        smart_lists: a.bool("smartIndentList", true),
        tab_size: a.get("tabSize").and_then(Value::as_u64).map_or(4, |n| n.clamp(1, 16) as usize),
        use_tabs: a.bool("useTab", true),
    }
}

fn hotkey_json(k: &Key) -> Value {
    let mut mods = Vec::new();
    if k.ctrl {
        mods.push("Mod");
    }
    if k.alt {
        mods.push("Alt");
    }
    if k.shift {
        mods.push("Shift");
    }
    let key = if k.key.chars().count() == 1 { k.key.to_uppercase() } else { k.key.clone() };
    json!([{ "modifiers": mods, "key": key }])
}

impl App {
    /// Whether a core plugin is on. Plugins that `core-plugins.json` does not mention are on.
    pub(crate) fn core_on(&self, id: &str) -> bool {
        self.config.core_plugin_enabled(id).unwrap_or(true)
    }

    /// The core plugin that owns `command`, when that plugin is off.
    pub(crate) fn disabled_by(&self, command: &str) -> Option<&'static str> {
        CORE_PLUGINS.iter().find(|c| c.3.contains(&command) && !self.core_on(c.0)).map(|c| c.1)
    }

    fn setting_file(&self, f: File) -> &JsonFile {
        match f {
            File::App => &self.config.app,
            File::Appearance => &self.config.appearance,
        }
    }

    fn save_setting(&mut self, f: File, key: &str, v: Value) {
        let file = match f {
            File::App => &mut self.config.app,
            File::Appearance => &mut self.config.appearance,
        };
        file.set(key, v);
        let r = file.save();
        if let Err(e) = r {
            self.notice = Some(format!("No se pudo guardar {}: {e}", self.setting_file(f).path.display()));
        }
        self.setting_changed(key);
    }

    /// Make a changed setting take effect.
    fn setting_changed(&mut self, key: &str) {
        match key {
            "vimMode" => {
                let on = self.config.app.bool("vimMode", false);
                self.vim = on;
                for t in self.all_tabs_mut() {
                    t.vim = on.then(Default::default);
                }
            }
            "useTab" | "tabSize" | "autoPairBrackets" | "autoPairMarkdown" | "smartIndentList" => {
                let s = editor_settings(&self.config);
                for t in self.all_tabs_mut() {
                    t.editor.settings = s.clone();
                }
            }
            "newLinkFormat" => self.store.index.link_format = self.config.link_format(),
            "trashOption" => self.store.trash_mode = self.config.trash_option(),
            "theme" => self.dark = self.config.color_scheme().or(self.system_dark).unwrap_or(true),
            "cssTheme" | "baseFontSize" | "textFontFamily" | "accentColor" | "lineWidth"
            | "enabledCssSnippets" => self.theme = load_theme(&self.config),
            "propertiesInDocument" => self.prop_edit = None,
            _ => {}
        }
    }

    /// Settings-derived state for a note tab that was just opened.
    pub(crate) fn init_new_tab(&mut self) {
        let s = editor_settings(&self.config);
        let i = self.tabs.len().saturating_sub(1);
        if let Some(t) = self.tabs.get_mut(i) {
            t.editor.settings = s;
        }
        self.park_cursor_after_front(i);
    }

    pub(crate) fn config_changed(&mut self) {
        for k in ["vimMode", "useTab", "newLinkFormat", "trashOption", "propertiesInDocument"] {
            self.setting_changed(k);
        }
        self.setting_changed("theme");
    }

    /// Folder that `Create new note` uses.
    pub(crate) fn new_note_dir(&self) -> String {
        match self.config.app.str("newFileLocation") {
            Some("current") => {
                self.tab().map(|t| vault_core::path::parent(&t.path).to_string()).unwrap_or_default()
            }
            Some("folder") => {
                vault_core::path::normalize(self.config.app.str("newFileFolderPath").unwrap_or(""))
            }
            _ => String::new(),
        }
    }

    fn set_hotkey(&mut self, cmd: &str, key: Option<Key>) {
        let v = key.as_ref().map_or_else(|| json!([]), hotkey_json);
        self.config.hotkeys.set(cmd, v);
        self.after_hotkey_change();
        if let Some(k) = key {
            let seq = vec![k];
            let other = self.keymap.iter().find(|(s, c)| *s == seq && c != cmd).map(|(_, c)| c.clone());
            if let Some(name) = other.and_then(|c| COMMANDS.iter().find(|x| x.0 == c).map(|x| x.1)) {
                self.notice = Some(format!("Esta tecla también está asignada a \"{name}\""));
            }
        }
    }

    fn after_hotkey_change(&mut self) {
        if let Err(e) = self.config.hotkeys.save() {
            self.notice = Some(format!("No se pudo guardar hotkeys.json: {e}"));
        }
        self.keymap = default_keymap(&self.config);
    }

    fn commit_setting_edit(&mut self) {
        let Some((key, text)) = self.settings.as_mut().and_then(|s| s.edit.take()) else { return };
        if key == "hk-filter" || key == "search" {
            if let Some(s) = &mut self.settings {
                if key == "search" {
                    s.search = text;
                } else {
                    s.filter = text;
                }
            }
            return;
        }
        let Some(it) = item(&key) else { return };
        let v = match it.control {
            Number(_, lo, hi) => match text.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => number_json(n.clamp(lo, hi)),
                _ => {
                    self.notice = Some(format!("\"{}\" no es un número", text.trim()));
                    return;
                }
            },
            _ if it.key == "accentColor" && !text.trim().is_empty() => {
                if theme_engine::parse_color(text.trim()).is_none() {
                    self.notice = Some(format!("\"{}\" no es un color", text.trim()));
                    return;
                }
                json!(text.trim())
            }
            // The value goes into a CSS declaration.
            _ if it.key == "textFontFamily" && text.contains([';', '{', '}']) => {
                self.notice = Some("Una lista de fuentes no puede contener ; { ni }".to_string());
                return;
            }
            _ => json!(text.trim()),
        };
        self.save_setting(it.file, it.key, v);
    }

    pub(crate) fn run_settings(&mut self, command: &str) -> bool {
        if command == "app:open-settings" {
            if self.settings.is_some() {
                self.settings = None;
            } else {
                self.prompt = None;
                self.prop_edit = None;
                self.settings = Some(SettingsUi::new(self));
                self.scrolls.remove("settings-content");
            }
            return true;
        }
        let Some(rest) = command.strip_prefix("settings:") else { return false };
        if self.settings.is_none() {
            return true;
        }
        let (op, arg) = rest.split_once(':').unwrap_or((rest, ""));
        let keeps_edit = matches!(op, "noop") || (op == "edit" && self.edit_key() == Some(arg));
        if !keeps_edit {
            self.commit_setting_edit();
        }
        match op {
            "close" => self.settings = None,
            "section" => {
                if let Some(&(sec, ..)) = SECTIONS.iter().find(|s| s.1 == arg) {
                    if let Some(s) = &mut self.settings {
                        s.section = sec;
                        s.capture = None;
                    }
                    self.scrolls.remove("settings-content");
                }
            }
            "toggle" => {
                if let Some(it) = item(arg) {
                    let Toggle(default) = it.control else { return true };
                    let now = self.setting_file(it.file).bool(it.key, default);
                    self.save_setting(it.file, it.key, json!(!now));
                }
            }
            "cycle" => {
                if let Some((it, Choice(default, opts))) = item(arg).map(|i| (i, &i.control)) {
                    let cur = self.setting_file(it.file).str(it.key).unwrap_or(default).to_string();
                    let at = opts.iter().position(|o| o.0 == cur).map_or(0, |p| p + 1);
                    self.save_setting(it.file, it.key, json!(opts[at % opts.len()].0));
                }
            }
            "edit" => {
                let text = if arg == "hk-filter" {
                    self.settings.as_ref().map(|s| s.filter.clone()).unwrap_or_default()
                } else if arg == "search" {
                    self.settings.as_ref().map(|s| s.search.clone()).unwrap_or_default()
                } else {
                    match item(arg) {
                        Some(it) => match self.setting_file(it.file).get(it.key) {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Number(n)) => n.as_f64().map(vault_core_num).unwrap_or_default(),
                            _ => match it.control {
                                Number(d, ..) => vault_core_num(d),
                                _ => String::new(),
                            },
                        },
                        None => return true,
                    }
                };
                if let Some(s) = &mut self.settings {
                    s.edit = Some((arg.to_string(), text));
                }
            }
            "css-theme" => {
                let mut opts = vec![String::new()];
                opts.extend(self.settings.iter().flat_map(|s| s.themes.clone()));
                let cur = self.config.css_theme().unwrap_or("").to_string();
                if !opts.contains(&cur) {
                    opts.push(cur.clone());
                }
                let at = opts.iter().position(|o| *o == cur).map_or(0, |p| p + 1);
                self.save_setting(File::Appearance, "cssTheme", json!(opts[at % opts.len()]));
            }
            "snippet" => {
                let mut on: Vec<Value> = self
                    .config
                    .appearance
                    .get("enabledCssSnippets")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                match on.iter().position(|v| v.as_str() == Some(arg)) {
                    Some(p) => {
                        on.remove(p);
                    }
                    None => on.push(json!(arg)),
                }
                self.save_setting(File::Appearance, "enabledCssSnippets", Value::Array(on));
            }
            "reload" => {
                self.run("app:reload-theme");
                self.config_changed();
                let ui = SettingsUi::new(self);
                if let Some(s) = &mut self.settings {
                    s.themes = ui.themes;
                    s.snippets = ui.snippets;
                }
            }
            "perf" => self.perf = !self.perf,
            "core" => {
                if CORE_PLUGINS.iter().any(|c| c.0 == arg) {
                    let on = self.core_on(arg);
                    self.config.set_core_plugin(arg, !on);
                    if let Err(e) = self.config.core_plugins.save() {
                        self.notice = Some(format!("No se pudo guardar core-plugins.json: {e}"));
                    }
                }
            }
            "auto-update" => {
                if let Err(e) = crate::update::set_auto_check(!crate::update::auto_check()) {
                    self.notice = Some(format!("No se pudo guardar el ajuste de actualizaciones: {e}"));
                }
            }
            "hk" => {
                if let Some(s) = &mut self.settings {
                    s.capture = (s.capture.as_deref() != Some(arg)).then(|| arg.to_string());
                }
            }
            "hk-reset" => {
                self.config.hotkeys.remove(arg);
                self.after_hotkey_change();
            }
            _ => {}
        }
        true
    }

    fn edit_key(&self) -> Option<&str> {
        self.settings.as_ref()?.edit.as_ref().map(|e| e.0.as_str())
    }

    /// Key press while a property or a settings control has the keyboard. `None` lets the
    /// regular shortcuts see the key.
    pub(crate) fn modal_key(&mut self, key: &Key) -> Option<bool> {
        if let Some(r) = self.prop_key(key) {
            return Some(r);
        }
        if let Some(r) = self.graph_key(key) {
            return Some(r);
        }
        if let Some(r) = self.backlinks_key(key) {
            return Some(r);
        }
        let s = self.settings.as_mut()?;
        let plain = !key.ctrl && !key.alt && !key.shift;
        if let Some(cmd) = s.capture.clone() {
            match key.key.as_str() {
                "Escape" if plain => s.capture = None,
                "Backspace" | "Delete" if plain => {
                    s.capture = None;
                    self.set_hotkey(&cmd, None);
                }
                k if k.chars().count() == 1 && !key.ctrl && !key.alt => {
                    self.notice = Some("Un atajo para un carácter necesita Ctrl o Alt".into());
                }
                _ => {
                    s.capture = None;
                    self.set_hotkey(&cmd, Some(key.clone()));
                }
            }
            return Some(true);
        }
        if let Some((_, buf)) = &mut s.edit {
            match key.key.as_str() {
                "Escape" => s.edit = None,
                "Enter" | "Tab" => self.commit_setting_edit(),
                "Backspace" => {
                    buf.pop();
                }
                _ => {}
            }
            return Some(true);
        }
        if key.key == "Escape" {
            self.settings = None;
            return Some(true);
        }
        if key.ctrl || key.alt {
            return None;
        }
        Some(true)
    }

    pub(crate) fn modal_text(&mut self, text: &str) -> bool {
        if self.prop_text(text) || self.graph_text(text) || self.backlinks_text(text) {
            return true;
        }
        let Some(s) = &mut self.settings else { return false };
        if let Some((_, buf)) = &mut s.edit {
            buf.push_str(text);
        }
        true
    }
}

fn vault_core_num(n: f64) -> String {
    md_engine::frontmatter::fmt_number(n)
}

fn row(dom: &mut Dom, parent: NodeId, name: &str, desc: &str) -> NodeId {
    let item = dom.add(parent, El::div().class("setting-item"));
    let info = dom.add(item, El::div().class("setting-item-info"));
    dom.add(info, El::div().class("setting-item-name").text(name));
    if !desc.is_empty() {
        dom.add(info, El::div().class("setting-item-description").text(desc));
    }
    dom.add(item, El::div().class("setting-item-control"))
}

fn toggle(dom: &mut Dom, ctl: NodeId, on: bool, action: String) {
    let c = dom.add(ctl, El::div().class("checkbox-container").class_if(on, "is-enabled").action(action));
    dom.add(c, El::div().class("checkbox-thumb"));
}

fn button(dom: &mut Dom, ctl: NodeId, label: &str, action: String, cta: bool) {
    dom.add(ctl, El::div().class("setting-button").class_if(cta, "mod-cta").text(label).action(action));
}

fn input(dom: &mut Dom, ctl: NodeId, key: &str, shown: &str, edit: Option<&(String, String)>) {
    let editing = edit.filter(|e| e.0 == key);
    let text = editing.map_or_else(|| shown.to_string(), |e| format!("{}▏", e.1));
    dom.add(
        ctl,
        El::div()
            .class("setting-input")
            .class_if(editing.is_some(), "is-editing")
            .text(text)
            .action(format!("settings:edit:{key}")),
    );
}

fn heading(dom: &mut Dom, parent: NodeId, text: &str) {
    dom.add(parent, El::div().class("setting-item-heading").text(text));
}

fn items_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, section: Section, needle: &str) {
    for it in ITEMS
        .iter()
        .filter(|i| i.section == section && (needle.is_empty() || matches(needle, i.name, i.desc)))
    {
        let ctl = row(dom, parent, it.name, it.desc);
        let file = app.setting_file(it.file);
        match &it.control {
            Toggle(d) => toggle(dom, ctl, file.bool(it.key, *d), format!("settings:toggle:{}", it.key)),
            Choice(d, opts) => {
                let cur = file.str(it.key).unwrap_or(d);
                let label = opts.iter().find(|o| o.0 == cur).map_or(cur, |o| o.1);
                dom.add(
                    ctl,
                    El::div()
                        .class("dropdown")
                        .text(format!("{label} ▾"))
                        .action(format!("settings:cycle:{}", it.key)),
                );
            }
            Text => input(dom, ctl, it.key, file.str(it.key).unwrap_or(""), ui.edit.as_ref()),
            Number(d, ..) => {
                let n = file.get(it.key).and_then(Value::as_f64).unwrap_or(*d);
                input(dom, ctl, it.key, &vault_core_num(n), ui.edit.as_ref());
            }
        }
    }
}

fn appearance_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi) {
    items_view(app, dom, parent, ui, Section::Appearance, "");
    let theme = app.config.css_theme().unwrap_or("");
    let ctl = row(dom, parent, "Tema", "Tema de la comunidad en .obsidian/themes. Haz clic para cambiar.");
    let label = if theme.is_empty() { "Predeterminado" } else { theme };
    dom.add(ctl, El::div().class("dropdown").text(format!("{label} ▾")).action("settings:css-theme"));
    heading(dom, parent, "Fragmentos CSS");
    let ctl = row(dom, parent, "Recargar", "Vuelve a leer temas, fragmentos y ajustes del disco.");
    button(dom, ctl, "Recargar", "settings:reload".into(), false);
    if ui.snippets.is_empty() {
        dom.add(
            parent,
            El::div().class("setting-item-description").text("No hay fragmentos en .obsidian/snippets."),
        );
    }
    let enabled = app.config.enabled_snippets();
    for s in &ui.snippets {
        let ctl = row(dom, parent, &format!("{s}.css"), "");
        toggle(dom, ctl, enabled.contains(&s.as_str()), format!("settings:snippet:{s}"));
    }
}

fn hotkeys_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi) {
    let ctl = row(dom, parent, "Filtrar", "Muestra los comandos cuyo nombre contiene este texto.");
    let shown = if ui.filter.is_empty() { "Filtrar comandos…" } else { &ui.filter };
    input(dom, ctl, "hk-filter", shown, ui.edit.as_ref());
    let needle = ui.filter.to_lowercase();
    let custom = app.config.hotkeys.value.as_object();
    for (id, name, _) in COMMANDS.iter().filter(|c| c.1.to_lowercase().contains(&needle)) {
        let ctl = row(dom, parent, name, "");
        let capturing = ui.capture.as_deref() == Some(id);
        let bound = app.hotkey_for(id);
        let label = if capturing {
            "Pulsa una tecla (Esc cancela, Backspace borra)".to_string()
        } else if bound.is_empty() {
            "Sin asignar".to_string()
        } else {
            bound
        };
        dom.add(
            ctl,
            El::div()
                .class("setting-hotkey")
                .class_if(capturing, "is-capturing")
                .class_if(label == "Sin asignar", "is-blank")
                .text(label)
                .action(format!("settings:hk:{id}")),
        );
        if custom.is_some_and(|o| o.contains_key(*id)) {
            dom.add(
                ctl,
                El::div().class("setting-hotkey-reset").text("↺").action(format!("settings:hk-reset:{id}")),
            );
        }
    }
}

fn core_plugins_view(app: &App, dom: &mut Dom, parent: NodeId, needle: &str) {
    for (id, name, desc, _) in CORE_PLUGINS {
        if !needle.is_empty() && !matches(needle, name, desc) {
            continue;
        }
        let ctl = row(dom, parent, name, desc);
        toggle(dom, ctl, app.core_on(id), format!("settings:core:{id}"));
    }
}

fn matches(needle: &str, name: &str, desc: &str) -> bool {
    name.to_lowercase().contains(needle) || desc.to_lowercase().contains(needle)
}

/// Settings from every section whose name or description contains the search text.
fn search_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, needle: &str) {
    let mut found = false;
    for (sec, _, title) in SECTIONS {
        let hits = ITEMS.iter().any(|i| i.section == sec && matches(needle, i.name, i.desc));
        let core = sec == Section::CorePlugins && CORE_PLUGINS.iter().any(|c| matches(needle, c.1, c.2));
        if !hits && !core {
            continue;
        }
        found = true;
        heading(dom, parent, title);
        if core {
            core_plugins_view(app, dom, parent, needle);
        } else {
            items_view(app, dom, parent, ui, sec, needle);
        }
    }
    if !found {
        dom.add(parent, El::div().class("setting-item-description").text("Ningún ajuste coincide."));
    }
}

fn general_view(app: &App, dom: &mut Dom, parent: NodeId) {
    let ctl = row(dom, parent, "Rustsidian", "Base de conocimiento Markdown nativa y rápida.");
    dom.add(ctl, El::div().class("setting-value").text(format!("v{}", env!("CARGO_PKG_VERSION"))));
    // The path goes in the description, which wraps; a long one does not fit beside the name.
    row(dom, parent, "Bóveda", &app.store.root.display().to_string());
    let ctl = row(dom, parent, "Notas", "");
    dom.add(ctl, El::div().class("setting-value").text(app.store.index.notes().count().to_string()));
    let ctl = row(
        dom,
        parent,
        "Superposición de rendimiento",
        "Muestra el tiempo por fotograma y el número de nodos en la esquina.",
    );
    toggle(dom, ctl, app.perf, "settings:perf".into());
    let ctl = row(dom, parent, "Actualizaciones", &app.update.describe());
    match &app.update {
        crate::update::State::Available(r) => {
            button(dom, ctl, &format!("Install {}", r.tag), "app:install-update".into(), true)
        }
        s if !s.busy() => button(dom, ctl, "Buscar ahora", "app:check-for-updates".into(), false),
        _ => {}
    }
    let ctl =
        row(dom, parent, "Buscar actualizaciones automáticamente", "Una vez al día, al iniciar Rustsidian.");
    toggle(dom, ctl, crate::update::auto_check(), "settings:auto-update".into());
}

/// The settings window, drawn over everything else.
pub fn modal(app: &App, dom: &mut Dom) {
    let Some(ui) = &app.settings else { return };
    let t = app.anims.t("settings");
    let mut backdrop = El::div().class("modal-container mod-dim").action("settings:close");
    let mut modal = El::div().class("modal mod-settings").action("settings:noop");
    if t < 1.0 {
        backdrop = backdrop.style(&format!("opacity: {t:.3}"));
        modal = modal.style(&format!("margin-top: {:.1}px", 14.0 * (1.0 - t)));
    }
    let backdrop = dom.add(0, backdrop);
    let modal = dom.add(backdrop, modal);
    let tabs = dom.add(modal, El::div().class("vertical-tab-header"));
    let search = ui.edit.as_ref().filter(|e| e.0 == "search").map_or(ui.search.as_str(), |e| e.1.as_str());
    let shown = if ui.search.is_empty() { "Buscar ajustes…" } else { &ui.search };
    input(dom, tabs, "search", shown, ui.edit.as_ref());
    dom.add(tabs, El::div().class("vertical-tab-header-group-title").text("Opciones"));
    for (sec, slug, name) in SECTIONS {
        dom.add(
            tabs,
            El::div()
                .class("vertical-tab-nav-item")
                .class_if(sec == ui.section, "is-active")
                .text(name)
                .action(format!("settings:section:{slug}")),
        );
    }
    let container = dom.add(modal, El::div().class("vertical-tab-content-container"));
    let content = dom.add(container, El::div().class("vertical-tab-content").id("settings-content"));
    let needle = search.trim().to_lowercase();
    if !needle.is_empty() {
        search_view(app, dom, content, ui, &needle);
    } else {
        let title = SECTIONS.iter().find(|s| s.0 == ui.section).map_or("", |s| s.2);
        heading(dom, content, title);
        match ui.section {
            Section::General => general_view(app, dom, content),
            Section::Appearance => appearance_view(app, dom, content, ui),
            Section::Hotkeys => hotkeys_view(app, dom, content, ui),
            Section::CorePlugins => core_plugins_view(app, dom, content, ""),
            s => items_view(app, dom, content, ui, s, ""),
        }
    }
    dom.add(
        container,
        El::div()
            .class("setting-footer")
            .text("Los cambios se guardan en .obsidian a medida que los haces.  Esc cierra."),
    );
    let _ = TrashMode::Vault;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_json_uses_vault_names() {
        let k = Key::parse("Ctrl+Shift+h").unwrap();
        assert_eq!(hotkey_json(&k), json!([{ "modifiers": ["Mod", "Shift"], "key": "H" }]));
        let k = Key::parse("Alt+ArrowUp").unwrap();
        assert_eq!(hotkey_json(&k), json!([{ "modifiers": ["Alt"], "key": "ArrowUp" }]));
    }

    #[test]
    fn items_have_unique_keys() {
        let mut keys: Vec<_> = ITEMS.iter().map(|i| i.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ITEMS.len());
    }
}
