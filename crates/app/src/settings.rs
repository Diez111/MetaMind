//! Settings window: appearance, editor, AI, plugins and advanced options, with a search across
//! all of them. Every control writes the same `.obsidian/*.json` key the vault format uses (other
//! keys are left as they are) and takes effect at once.

use crate::ai::{AiConfig, AiProfile};
use crate::icons;
use crate::plugins::PluginInfo;
use crate::shell::{default_keymap, load_theme, App, Key, COMMANDS};
use metamind_ui::{Dom, El, NodeId};
use serde_json::{json, Value};
use vault_core::config::JsonFile;
use vault_core::fsio::TrashMode;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Appearance,
    Editor,
    Ai,
    Plugins,
    Advanced,
}

/// The settings categories, in the order they appear: (section, slug in commands, name, one-line
/// description, icon).
const SECTIONS: [(Section, &str, &str, &str, &str); 5] = [
    (
        Section::Appearance,
        "appearance",
        "Apariencia",
        "Temas, tipografía, color de acento y fragmentos CSS.",
        "sun",
    ),
    (
        Section::Editor,
        "editor",
        "Editor",
        "Escritura, sangría, propiedades y corrección ortográfica.",
        "edit",
    ),
    (Section::Ai, "ai", "IA", "Perfiles de proveedores, modelos y claves del copiloto.", "sparkles"),
    (
        Section::Plugins,
        "plugins",
        "Plugins",
        "Funciones integradas que puedes activar y configurar.",
        "layout-dashboard",
    ),
    (
        Section::Advanced,
        "advanced",
        "Avanzado",
        "Bóveda, archivos y enlaces, atajos de teclado, rendimiento y actualizaciones.",
        "settings",
    ),
];

fn section_for_slug(slug: &str) -> Option<Section> {
    Some(match slug {
        "appearance" => Section::Appearance,
        "editor" => Section::Editor,
        "ai" => Section::Ai,
        "plugins" | "core-plugins" => Section::Plugins,
        // The old tabs fold into Avanzado; keeping their slugs means old links still work.
        "advanced" | "general" | "files" | "interface" | "hotkeys" => Section::Advanced,
        _ => return None,
    })
}

fn section_meta(
    section: Section,
) -> &'static (Section, &'static str, &'static str, &'static str, &'static str) {
    SECTIONS.iter().find(|s| s.0 == section).unwrap_or(&SECTIONS[0])
}

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
        "ai",
        "Copiloto de IA",
        "Pregunta a un modelo compatible con OpenAI sobre la nota abierta.",
        &["ai:open"],
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
    /// The text field being edited: (setting key, text so far). Plugin settings use
    /// `@<plugin id>:<key>` here.
    pub edit: Option<(String, String)>,
    /// The command whose hotkey is waiting for a key press.
    pub capture: Option<String>,
    pub filter: String,
    /// The settings search; while it has text, matching settings from every section are shown.
    pub search: String,
    pub themes: Vec<String>,
    pub snippets: Vec<String>,
    /// The built-in plugin whose settings page is open, if any.
    pub plugin: Option<&'static str>,
    /// Keyboard focus ring on the category list: index into `SECTIONS`.
    pub nav: Option<usize>,
    /// The profile whose form Ajustes → IA shows; `None` means the active one.
    pub ai_edit: Option<usize>,
}

impl SettingsUi {
    fn new(app: &App) -> SettingsUi {
        let mut s = SettingsUi {
            section: Section::Appearance,
            edit: None,
            capture: None,
            filter: String::new(),
            search: String::new(),
            themes: Vec::new(),
            snippets: Vec::new(),
            plugin: None,
            nav: None,
            ai_edit: None,
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
use Section::{Advanced, Appearance, Editor};

const ITEMS: &[Item] = &[
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "theme",
        name: "Esquema de color base",
        desc: "Base clara u oscura para toda la interfaz. Los swatches la previsualizan.",
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
        desc: "Fuente de las notas, separada por comas; se usa la primera instalada.",
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
        desc: "Ancho máximo del texto cuando «Longitud de línea legible» está activado.",
        control: Number(800.0, 400.0, 2400.0),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "showRibbon",
        name: "Mostrar barra de iconos",
        desc: "Muestra la columna de botones en el borde izquierdo de la ventana.",
        control: Toggle(true),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "showViewHeader",
        name: "Mostrar barra de título de la pestaña",
        desc: "Muestra el título de la nota y los botones de vista sobre cada nota.",
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
        desc: "Subraya palabras mal escritas en inglés; clic derecho para ver sugerencias.",
        control: Toggle(false),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "propertiesInDocument",
        name: "Propiedades en el documento",
        desc: "Muestra el front matter como tabla, como código fuente o no lo muestra.",
        control: Choice(
            "visible",
            &[("visible", "Visibles"), ("source", "Código fuente"), ("hidden", "Ocultas")],
        ),
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
        section: Advanced,
        file: File::App,
        key: "rememberExpandedFolders",
        name: "Archivos: recordar carpetas expandidas",
        desc: "Al abrir la bóveda, el explorador restaura las carpetas que dejaste abiertas. Apagado, arranca siempre con todas contraídas y no las guarda.",
        control: Toggle(true),
    },
    Item {
        section: Advanced,
        file: File::App,
        key: "newFileLocation",
        name: "Ubicación de las notas nuevas",
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
        section: Advanced,
        file: File::App,
        key: "newFileFolderPath",
        name: "Carpeta para notas nuevas",
        desc: "Se usa cuando la ubicación anterior es una carpeta concreta.",
        control: Text,
    },
    Item {
        section: Advanced,
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
        section: Advanced,
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
    pub(crate) fn setting_changed(&mut self, key: &str) {
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
        // Plugin settings keep their text under `@<plugin>:<key>`.
        if let Some((plugin, setting)) = key.strip_prefix('@').and_then(|r| r.split_once(':')) {
            self.set_plugin_setting(plugin, setting, json!(text.trim()));
            return;
        }
        if let Some((field, at)) = ai_field_key(&key) {
            self.commit_ai_field(field, at, &text);
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
                // The editor's suggestion popover is drawn after the window's overlays, so it
                // would sit on top of the settings scrim: nothing of the editor stays on.
                self.suggest = None;
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
            "section" | "badge" => {
                // The count on the Plugins nav item opens that page too, so the badge is a
                // shortcut with a hint rather than a number nobody can place.
                let sec = match op {
                    "badge" => Some(Section::Plugins),
                    _ => section_for_slug(arg),
                };
                if let Some(sec) = sec {
                    let at = SECTIONS.iter().position(|s| s.0 == sec);
                    if let Some(s) = &mut self.settings {
                        s.section = sec;
                        s.capture = None;
                        s.plugin = None;
                        s.nav = at;
                    }
                    self.scrolls.remove("settings-content");
                }
            }
            "set" => {
                // `settings:set:<key>:<value>` writes a choice directly (the theme swatches).
                let Some((key, value)) = arg.split_once(':') else { return true };
                let Some((it, Choice(default, opts))) = item(key).map(|i| (i, &i.control)) else {
                    return true;
                };
                let cur = self.setting_file(it.file).str(it.key).unwrap_or(default);
                if cur == value || opts.iter().any(|o| o.0 == value) {
                    self.save_setting(it.file, it.key, json!(value));
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
                } else if let Some((plugin, setting)) = arg.strip_prefix('@').and_then(|r| r.split_once(':'))
                {
                    match self.plugin_registry().get(plugin).cloned() {
                        Some(p) => self.plugin_setting(&p, setting).as_str().unwrap_or("").to_string(),
                        None => return true,
                    }
                } else if let Some((field, at)) = ai_field_key(arg) {
                    match ai_field_text(&self.ai_config(), field, at) {
                        Some(text) => text,
                        None => return true,
                    }
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
            "plugin" => {
                if let Some(p) = self.plugin_registry().get(arg) {
                    let id = p.id;
                    if let Some(s) = &mut self.settings {
                        s.plugin = Some(id);
                    }
                    self.scrolls.remove("settings-content");
                }
            }
            "plugins" => {
                if let Some(s) = &mut self.settings {
                    s.plugin = None;
                }
                self.scrolls.remove("settings-content");
            }
            "penable" => {
                let on = self.plugin_registry().get(arg).map(|p| p.enabled).unwrap_or(false);
                self.set_builtin_plugin(arg, !on);
            }
            "ptoggle" => {
                let Some((plugin, key)) = arg.split_once(':') else { return true };
                let Some(p) = self.plugin_registry().get(plugin).cloned() else { return true };
                let Some(setting) = p.settings.iter().find(|s| s.key == key) else { return true };
                let crate::plugins::SettingKind::Toggle(default) = setting.kind else { return true };
                let on = self.plugin_setting(&p, key).as_bool().unwrap_or(default);
                self.set_plugin_setting(plugin, key, json!(!on));
            }
            "pcycle" => {
                let Some((plugin, key)) = arg.split_once(':') else { return true };
                let Some(p) = self.plugin_registry().get(plugin).cloned() else { return true };
                let Some(setting) = p.settings.iter().find(|s| s.key == key) else { return true };
                let crate::plugins::SettingKind::Choice(default, opts) = setting.kind else { return true };
                let cur = self.plugin_setting(&p, key);
                let cur = cur.as_str().unwrap_or(default);
                let at = opts.iter().position(|o| o.0 == cur).map_or(0, |p| p + 1);
                self.set_plugin_setting(plugin, key, json!(opts[at % opts.len()].0));
            }
            "pset" => {
                let parts: Vec<&str> = arg.splitn(3, ':').collect();
                if let [plugin, key, value] = parts.as_slice() {
                    self.set_plugin_setting(plugin, key, json!(value));
                }
            }
            "community" => {
                let on = self.plugin_list().iter().find(|p| p.id == arg).is_some_and(|p| p.enabled);
                if let Err(e) = self.set_plugin_enabled(arg, !on) {
                    self.notice = Some(format!("No se pudo guardar community-plugins.json: {e}"));
                } else {
                    self.run("plugins:reload");
                }
            }
            "reload-plugins" => {
                self.run("plugins:reload");
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
            "ai-new" => {
                let mut cfg = AiConfig::from_config(&self.config);
                let name = cfg.free_name("Perfil");
                cfg.profiles.push(AiProfile::named(name));
                self.set_ai_config(&cfg);
                if let Some(s) = &mut self.settings {
                    s.ai_edit = Some(cfg.profiles.len() - 1);
                }
            }
            "ai-dup" => {
                let Ok(at) = arg.parse::<usize>() else { return true };
                let mut cfg = AiConfig::from_config(&self.config);
                let Some(src) = cfg.profiles.get(at).cloned() else { return true };
                let mut copy = src;
                copy.name = cfg.free_name(&format!("{} (copia)", copy.name));
                cfg.profiles.insert(at + 1, copy);
                self.set_ai_config(&cfg);
                if let Some(s) = &mut self.settings {
                    s.ai_edit = Some(at + 1);
                }
            }
            "ai-del" => {
                let Ok(at) = arg.parse::<usize>() else { return true };
                let mut cfg = AiConfig::from_config(&self.config);
                if at >= cfg.profiles.len() {
                    return true;
                }
                cfg.profiles.remove(at);
                // Normalising keeps at least one profile and points `active` at one that exists.
                cfg.normalize();
                self.set_ai_config(&cfg);
                if let Some(s) = &mut self.settings {
                    s.ai_edit = Some(at.min(cfg.profiles.len() - 1));
                }
            }
            "ai-use" => {
                let Ok(at) = arg.parse::<usize>() else { return true };
                let mut cfg = AiConfig::from_config(&self.config);
                if let Some(p) = cfg.profiles.get(at) {
                    cfg.active = p.name.clone();
                    self.set_ai_config(&cfg);
                }
                if let Some(s) = &mut self.settings {
                    s.ai_edit = Some(at);
                }
            }
            "ai-select" => {
                if let Ok(at) = arg.parse::<usize>() {
                    if let Some(s) = &mut self.settings {
                        s.ai_edit = Some(at);
                    }
                }
            }
            "ai-preset" => {
                let Ok(at) = arg.parse::<usize>() else { return true };
                let mut cfg = AiConfig::from_config(&self.config);
                if let Some(p) = cfg.profiles.get_mut(at) {
                    let next = p.preset().next();
                    p.apply_preset(next);
                }
                self.set_ai_config(&cfg);
                if let Some(s) = &mut self.settings {
                    s.ai_edit = Some(at);
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

    /// Write one field of profile `at`, then save the profiles.
    fn commit_ai_field(&mut self, field: &str, at: usize, text: &str) {
        let mut cfg = AiConfig::from_config(&self.config);
        if at >= cfg.profiles.len() {
            return;
        }
        let old_name = cfg.profiles[at].name.clone();
        let mut new_name: Option<String> = None;
        match field {
            "name" => {
                let name = text.trim();
                if name.is_empty() {
                    self.notice = Some("El perfil necesita un nombre".to_string());
                    return;
                }
                new_name = Some(name.to_string());
            }
            "base_url" => cfg.profiles[at].base_url = text.trim().to_string(),
            "model" => cfg.profiles[at].model = text.trim().to_string(),
            "api_key" => cfg.profiles[at].api_key = text.trim().to_string(),
            "system_prompt" => cfg.profiles[at].system_prompt = text.trim().to_string(),
            "timeout_s" => match text.trim().parse::<u64>() {
                Ok(n) => cfg.profiles[at].timeout_s = n.clamp(1, 600),
                Err(_) => {
                    self.notice = Some(format!("\"{}\" no es un número de segundos", text.trim()));
                    return;
                }
            },
            _ => return,
        }
        if let Some(name) = new_name {
            cfg.profiles[at].name = name.clone();
            if cfg.active == old_name {
                cfg.active = name;
            }
        }
        // A blank URL or model means the default one.
        let d = AiProfile::default();
        if cfg.profiles[at].base_url.trim().is_empty() {
            cfg.profiles[at].base_url = d.base_url;
        }
        if cfg.profiles[at].model.trim().is_empty() {
            cfg.profiles[at].model = d.model;
        }
        cfg.normalize();
        self.set_ai_config(&cfg);
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
        if let Some(r) = self.ai_key(key) {
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
        // Keyboard navigation: arrows and Tab move the focus ring through the categories.
        if !key.ctrl && !key.alt {
            let step = match key.key.as_str() {
                "ArrowDown" => Some(1isize),
                "ArrowUp" => Some(-1isize),
                "Tab" => Some(if key.shift { -1 } else { 1 }),
                _ => None,
            };
            if let Some(step) = step {
                let n = SECTIONS.len() as isize;
                let cur = s.nav.map_or(if step > 0 { -1 } else { 0 }, |i| i as isize);
                let slug = SECTIONS[(cur + step).rem_euclid(n) as usize].1;
                self.run_settings(&format!("settings:section:{slug}"));
                return Some(true);
            }
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
        if self.ai_text(text) {
            return true;
        }
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

/// A setting row: name and control on one line, the one-line description below the whole row.
fn row(dom: &mut Dom, parent: NodeId, name: &str, desc: &str) -> NodeId {
    let item = dom.add(parent, El::div().class("setting-item"));
    let main = dom.add(item, El::div().class("setting-item-main"));
    let info = dom.add(main, El::div().class("setting-item-info"));
    dom.add(info, El::div().class("setting-item-name").text(name));
    let ctl = dom.add(main, El::div().class("setting-item-control"));
    if !desc.is_empty() {
        dom.add(item, El::div().class("setting-item-description").text(desc));
    }
    ctl
}

fn toggle(dom: &mut Dom, ctl: NodeId, on: bool, action: String) {
    let c = dom.add(ctl, El::div().class("checkbox-container").class_if(on, "is-enabled").action(action));
    dom.add(c, El::div().class("checkbox-thumb"));
}

fn button(dom: &mut Dom, ctl: NodeId, label: &str, action: String, cta: bool) {
    dom.add(ctl, El::div().class("setting-button").class_if(cta, "mod-cta").text(label).action(action));
}

/// The value a text setting takes while its field is empty: the default the app already
/// applies, so the control says what is in force instead of looking broken.
fn text_placeholder(key: &str) -> Option<&'static str> {
    match key {
        // `--font-text` in the base sheet, which is what the notes are set in until one is picked.
        "textFontFamily" => Some("sans-serif"),
        // hsl(258, 88%, 66%): the accent of the default scheme, the one the chip beside the
        // field shows.
        "accentColor" => Some("#8a5cf5"),
        _ => None,
    }
}

/// A text control. While it is the one being edited it gets the keyboard focus ring.
/// `placeholder` is what an empty field shows, in the faint colour of a hint.
fn input(
    dom: &mut Dom,
    ctl: NodeId,
    key: &str,
    shown: &str,
    edit: Option<&(String, String)>,
    placeholder: Option<&str>,
) -> NodeId {
    let editing = edit.filter(|e| e.0 == key);
    let placeholder = placeholder.filter(|p| !p.is_empty() && shown.trim().is_empty() && editing.is_none());
    let text = match (editing, placeholder) {
        (Some(e), _) => format!("{}▏", e.1),
        (None, Some(p)) => p.to_string(),
        (None, None) => shown.to_string(),
    };
    let id = dom.add(
        ctl,
        El::div()
            .class("setting-input")
            .class_if(editing.is_some(), "is-editing")
            .class_if(placeholder.is_some(), "is-placeholder")
            .text(text)
            .action(format!("settings:edit:{key}")),
    );
    if editing.is_some() {
        dom.nodes[id].focus = true;
    }
    id
}

fn heading(dom: &mut Dom, parent: NodeId, text: &str) {
    dom.add(parent, El::div().class("setting-item-heading").text(text));
}

/// The prominent header of a settings category: icon, title and a one-line description.
fn section_header(dom: &mut Dom, parent: NodeId, section: Section) {
    let (_, _, name, desc, icon) = *section_meta(section);
    let header = dom.add(parent, El::div().class("settings-section-header"));
    let box_ = dom.add(header, El::div().class("settings-section-icon"));
    dom.add(box_, icons::el(icon));
    let text = dom.add(header, El::div().class("settings-section-text"));
    dom.add(text, El::div().class("settings-section-title").text(name));
    dom.add(text, El::div().class("settings-section-desc").text(desc));
}

fn search_header(dom: &mut Dom, parent: NodeId, query: &str, results: usize) {
    let header = dom.add(parent, El::div().class("settings-section-header"));
    let box_ = dom.add(header, El::div().class("settings-section-icon"));
    dom.add(box_, icons::el("search"));
    let text = dom.add(header, El::div().class("settings-section-text"));
    dom.add(text, El::div().class("settings-section-title").text("Resultados de búsqueda"));
    let n = if results == 1 { "1 ajuste".to_string() } else { format!("{results} ajustes") };
    dom.add(text, El::div().class("settings-section-desc").text(format!("{n} para «{query}».")));
}

/// The three theme swatches, each a mini window painted with that theme's colors.
struct Swatch {
    value: &'static str,
    label: &'static str,
    bg: &'static str,
    bar: &'static str,
    accent: &'static str,
}

const SWATCHES: [Swatch; 3] = [
    Swatch { value: "obsidian", label: "Oscuro", bg: "#1e1e1e", bar: "#3a3a3a", accent: "#8a5cf5" },
    Swatch { value: "moonstone", label: "Claro", bg: "#ffffff", bar: "#dcdcdc", accent: "#8a5cf5" },
    Swatch { value: "system", label: "Sistema", bg: "#3c3c3c", bar: "#d8d8d8", accent: "#8a5cf5" },
];

fn theme_swatches(dom: &mut Dom, ctl: NodeId, current: &str) {
    let wrap = dom.add(ctl, El::div().class("theme-swatches"));
    for s in &SWATCHES {
        // The name goes under the preview, in the flow: a label laid over the box was cut by the
        // row below it.
        let item =
            dom.add(wrap, El::div().class("theme-swatch-item").class_if(s.value == current, "is-active"));
        let sw = dom.add(
            item,
            El::div()
                .class("theme-swatch")
                .style(&format!("background-color: {};", s.bg))
                .attr("data-value", s.value)
                .action(format!("settings:set:theme:{}", s.value)),
        );
        dom.add(sw, El::div().class("theme-swatch-bar").style(&format!("background-color: {};", s.bar)));
        dom.add(sw, El::div().class("theme-swatch-dot").style(&format!("background-color: {};", s.accent)));
        dom.add(item, El::div().class("theme-swatch-label").text(s.label));
    }
}

/// The rows of a section. `previews` is what the panel shows and a row of search results does
/// not: the scheme as three previews instead of a dropdown that names it.
fn items_view(
    app: &App,
    dom: &mut Dom,
    parent: NodeId,
    ui: &SettingsUi,
    section: Section,
    needle: &str,
    previews: bool,
) {
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
                // The scheme picks itself from three previews; the dropdown that says its name is
                // the same control twice. It stays for a row of search results, which has no room
                // for the previews.
                if it.key == "theme" && previews {
                    theme_swatches(dom, ctl, cur);
                    continue;
                }
                dom.add(
                    ctl,
                    El::div()
                        .class("dropdown")
                        .text(format!("{label} ▾"))
                        .action(format!("settings:cycle:{}", it.key)),
                );
            }
            Text => {
                input(
                    dom,
                    ctl,
                    it.key,
                    file.str(it.key).unwrap_or(""),
                    ui.edit.as_ref(),
                    text_placeholder(it.key),
                );
                match it.key {
                    "accentColor" => {
                        let color = file
                            .str(it.key)
                            .filter(|c| !c.trim().is_empty())
                            .unwrap_or("var(--interactive-accent)");
                        dom.add(
                            ctl,
                            El::div()
                                .class("color-chip")
                                .style(&format!("background-color: {};", color.trim())),
                        );
                    }
                    "textFontFamily" => {
                        if let Some(font) = file.str(it.key).filter(|f| !f.trim().is_empty()) {
                            dom.add(
                                ctl,
                                El::div()
                                    .class("font-sample")
                                    .style(&format!("font-family: {};", font.trim()))
                                    .text("Aa"),
                            );
                        }
                    }
                    _ => {}
                }
            }
            Number(d, ..) => {
                let n = file.get(it.key).and_then(Value::as_f64).unwrap_or(*d);
                input(dom, ctl, it.key, &vault_core_num(n), ui.edit.as_ref(), None);
            }
        }
    }
}

fn appearance_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, previews: bool) {
    items_view(app, dom, parent, ui, Section::Appearance, "", previews);
    heading(dom, parent, "Tema de la comunidad");
    let theme = app.config.css_theme().unwrap_or("");
    let ctl = row(dom, parent, "Tema", "Lee los temas de .obsidian/themes; haz clic para cambiar.");
    let label = if theme.is_empty() { "Predeterminado" } else { theme };
    dom.add(ctl, El::div().class("dropdown").text(format!("{label} ▾")).action("settings:css-theme"));
    heading(dom, parent, "Fragmentos CSS");
    let ctl = row(dom, parent, "Recargar", "Vuelve a leer temas, fragmentos y ajustes del disco.");
    button(dom, ctl, "Recargar", "settings:reload".into(), false);
    if ui.snippets.is_empty() {
        dom.add(parent, El::div().class("settings-note").text("No hay fragmentos en .obsidian/snippets."));
    }
    let enabled = app.config.enabled_snippets();
    for s in &ui.snippets {
        let ctl = row(dom, parent, &format!("{s}.css"), "Activa este fragmento al instante.");
        toggle(dom, ctl, enabled.contains(&s.as_str()), format!("settings:snippet:{s}"));
    }
}

fn hotkeys_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi) {
    let ctl = row(dom, parent, "Filtrar", "Muestra los comandos cuyo nombre contiene este texto.");
    let shown = if ui.filter.is_empty() { "Filtrar comandos…" } else { &ui.filter };
    input(dom, ctl, "hk-filter", shown, ui.edit.as_ref(), None);
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
        let key = dom.add(
            ctl,
            El::div()
                .class("setting-hotkey")
                .class_if(capturing, "is-capturing")
                .class_if(label == "Sin asignar", "is-blank")
                .text(label)
                .action(format!("settings:hk:{id}")),
        );
        if capturing {
            dom.nodes[key].focus = true;
        }
        if custom.is_some_and(|o| o.contains_key(*id)) {
            dom.add(
                ctl,
                El::div().class("setting-hotkey-reset").text("↺").action(format!("settings:hk-reset:{id}")),
            );
        }
    }
}

/// Core plugins not promoted to a card in the built-in registry.
fn core_plugins_view(app: &App, dom: &mut Dom, parent: NodeId, needle: &str, skip: &[&str]) {
    for (id, name, desc, _) in CORE_PLUGINS {
        if skip.contains(id) || (!needle.is_empty() && !matches(needle, name, desc)) {
            continue;
        }
        let ctl = row(dom, parent, name, desc);
        toggle(dom, ctl, app.core_on(id), format!("settings:core:{id}"));
    }
}

fn kind_label(kind: ext_api::PluginKind) -> &'static str {
    match kind {
        ext_api::PluginKind::Wasm => "WASM",
        ext_api::PluginKind::Js => "JavaScript",
        ext_api::PluginKind::Script => "Script",
    }
}

fn community_plugins_view(app: &App, dom: &mut Dom, parent: NodeId) {
    let list = app.plugin_list();
    heading(dom, parent, "Plugins de la comunidad");
    let ctl = row(dom, parent, "Recargar plugins", "Vuelve a leer .obsidian/plugins y .obsidian/scripts.");
    button(dom, ctl, "Recargar", "settings:reload-plugins".into(), false);
    if list.is_empty() {
        dom.add(
            parent,
            El::div()
                .class("settings-note")
                .text("No hay plugins ni scripts en esta bóveda. Añádelos en .obsidian/plugins."),
        );
        return;
    }
    for p in &list {
        let state = match (&p.error, p.loaded, p.enabled) {
            // A plugin without a native port is expected to sit there until its port arrives:
            // the row says so without the word the list uses for real failures.
            (Some(e), _, _) if e == crate::plugins::NO_PORT => {
                format!("v{} · {} · {e}", p.version, kind_label(p.kind))
            }
            (Some(e), _, _) => format!("Error: {e}"),
            (_, true, _) => format!("v{} · {} · en ejecución", p.version, kind_label(p.kind)),
            (_, _, true) => format!("v{} · {} · se cargará al recargar", p.version, kind_label(p.kind)),
            _ => format!("v{} · {} · desactivado", p.version, kind_label(p.kind)),
        };
        let ctl = row(dom, parent, &p.name, &state);
        toggle(dom, ctl, p.enabled, format!("settings:community:{}", p.id));
    }
    if list.iter().any(|p| p.error.as_deref() == Some(crate::plugins::NO_PORT)) {
        dom.add(
            parent,
            El::div()
                .class("settings-note")
                .text("Los plugins sin port nativo aún quedan en la lista: llegarán los que hagan falta."),
        );
    }
}

fn plugin_matches(needle: &str, p: &PluginInfo) -> bool {
    matches(needle, p.name, p.description)
        || p.settings.iter().any(|s| matches(needle, s.label, s.description))
}

fn plugin_card(app: &App, dom: &mut Dom, parent: NodeId, p: &PluginInfo) {
    let card = dom.add(
        parent,
        El::div()
            .class("plugin-card")
            .class_if(p.enabled, "is-enabled")
            .action(format!("settings:plugin:{}", p.id)),
    );
    let head = dom.add(card, El::div().class("plugin-card-head"));
    let icon = dom.add(head, El::div().class("plugin-icon"));
    dom.add(icon, icons::el(p.icon));
    let info = dom.add(head, El::div().class("plugin-card-info"));
    let title = dom.add(info, El::div().class("plugin-card-title"));
    dom.add(title, El::div().class("plugin-card-name").text(p.name));
    dom.add(title, El::div().class("plugin-card-version").text(format!("v{}", p.version)));
    dom.add(info, El::div().class("plugin-card-desc").text(p.description));
    let ctl = dom.add(head, El::div().class("setting-item-control"));
    toggle(dom, ctl, p.enabled, format!("settings:penable:{}", p.id));
    button(dom, ctl, "Configurar", format!("settings:plugin:{}", p.id), false);
    if !p.settings.is_empty() {
        dom.add(card, El::div().class("plugin-card-meta").text(format!("{} ajustes", p.settings.len())));
    }
    let _ = app;
}

fn plugin_list_view(app: &App, dom: &mut Dom, parent: NodeId, needle: &str) {
    let reg = app.plugin_registry();
    let active = reg.enabled_count();
    let summary = dom.add(parent, El::div().class("plugin-summary"));
    dom.add(
        summary,
        El::div()
            .class("plugin-summary-text")
            .text(format!("{active} de {} plugins integrados activos", reg.plugins.len())),
    );
    for p in reg.plugins.iter().filter(|p| needle.is_empty() || plugin_matches(needle, p)) {
        plugin_card(app, dom, parent, p);
    }
    if !needle.is_empty() {
        if !reg.plugins.iter().any(|p| plugin_matches(needle, p)) {
            dom.add(parent, El::div().class("settings-note").text("Ningún plugin coincide."));
        }
        return;
    }
    community_plugins_view(app, dom, parent);
    heading(dom, parent, "Otros complementos principales");
    core_plugins_view(app, dom, parent, "", &["daily-notes", "templates", "bookmarks"]);
}

fn plugin_detail_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, p: &PluginInfo) {
    let back = dom.add(parent, El::div().class("plugin-back").action("settings:plugins"));
    dom.add(back, icons::el("arrow-left"));
    dom.add(back, El::div().class("plugin-back-label").text("Plugins"));
    let head = dom.add(parent, El::div().class("plugin-detail-head"));
    let icon = dom.add(head, El::div().class("plugin-icon mod-large"));
    dom.add(icon, icons::el(p.icon));
    let info = dom.add(head, El::div().class("plugin-detail-info"));
    let title = dom.add(info, El::div().class("plugin-card-title"));
    dom.add(title, El::div().class("plugin-detail-name").text(p.name));
    dom.add(title, El::div().class("plugin-card-version").text(format!("v{}", p.version)));
    dom.add(info, El::div().class("plugin-detail-desc").text(p.description));
    let ctl = dom.add(head, El::div().class("setting-item-control"));
    toggle(dom, ctl, p.enabled, format!("settings:penable:{}", p.id));
    heading(dom, parent, "Ajustes del plugin");
    if p.settings.is_empty() {
        dom.add(
            parent,
            El::div()
                .class("settings-note")
                .text("Este plugin no tiene ajustes propios: se activa y se desactiva con el interruptor."),
        );
        return;
    }
    for s in &p.settings {
        let ctl = row(dom, parent, s.label, s.description);
        let current = app.plugin_setting(p, s.key);
        match s.kind {
            crate::plugins::SettingKind::Toggle(default) => {
                let on = current.as_bool().unwrap_or(default);
                toggle(dom, ctl, on, format!("settings:ptoggle:{}:{}", p.id, s.key));
            }
            crate::plugins::SettingKind::Choice(default, opts) => {
                let cur = current.as_str().unwrap_or(default);
                let label = opts.iter().find(|o| o.0 == cur).map_or(cur, |o| o.1);
                dom.add(
                    ctl,
                    El::div()
                        .class("dropdown")
                        .text(format!("{label} ▾"))
                        .action(format!("settings:pcycle:{}:{}", p.id, s.key)),
                );
            }
            crate::plugins::SettingKind::Text(_) => {
                let shown = current.as_str().unwrap_or("");
                input(dom, ctl, &format!("@{}:{}", p.id, s.key), shown, ui.edit.as_ref(), None);
            }
        }
    }
}

fn plugins_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, needle: &str) {
    if let Some(id) = ui.plugin {
        if let Some(p) = app.plugin_registry().get(id) {
            return plugin_detail_view(app, dom, parent, ui, p);
        }
    }
    plugin_list_view(app, dom, parent, needle);
}

fn matches(needle: &str, name: &str, desc: &str) -> bool {
    name.to_lowercase().contains(needle) || desc.to_lowercase().contains(needle)
}

/// Whether the needle names the category itself (its title or its description).
fn category_matches(needle: &str, section: Section) -> bool {
    let (_, _, name, desc, _) = *section_meta(section);
    matches(needle, name, desc)
}

fn search_empty(dom: &mut Dom, parent: NodeId, needle: &str) {
    let empty = dom.add(parent, El::div().class("settings-empty"));
    dom.add(empty, icons::el("search").class("settings-empty-icon"));
    dom.add(empty, El::div().class("settings-empty-title").text("Ningún ajuste coincide"));
    dom.add(empty, El::div().class("settings-empty-text").text(format!("No hay ajustes para «{needle}».")));
}

/// Settings from every category whose name or description contains the search text.
fn search_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, needle: &str) {
    let reg = app.plugin_registry();
    let mut found = 0usize;
    for (sec, ..) in SECTIONS {
        let whole = category_matches(needle, sec);
        let hits = if whole {
            usize::MAX
        } else {
            match sec {
                Section::Plugins => {
                    reg.plugins.iter().filter(|p| plugin_matches(needle, p)).count()
                        + CORE_PLUGINS.iter().filter(|c| matches(needle, c.1, c.2)).count()
                }
                Section::Ai => usize::from(ai_matches(needle)),
                _ => ITEMS.iter().filter(|i| i.section == sec && matches(needle, i.name, i.desc)).count(),
            }
        };
        if hits == 0 {
            continue;
        }
        found += 1;
        section_header(dom, parent, sec);
        match sec {
            Section::Plugins => plugins_view(app, dom, parent, ui, if whole { "" } else { needle }),
            Section::Ai => ai_view(app, dom, parent, ui),
            Section::Appearance if whole => appearance_view(app, dom, parent, ui, false),
            Section::Advanced => advanced_view(app, dom, parent, ui, if whole { "" } else { needle }),
            _ => items_view(app, dom, parent, ui, sec, if whole { "" } else { needle }, false),
        }
    }
    if found == 0 {
        search_empty(dom, parent, needle);
    } else {
        dom.add(
            parent,
            El::div()
                .class("settings-note")
                .text("Se muestran los ajustes que coinciden con la búsqueda en todas las categorías."),
        );
    }
}

fn general_view(app: &App, dom: &mut Dom, parent: NodeId) {
    let ctl = row(dom, parent, "MetaMind", "Base de conocimiento Markdown nativa y rápida.");
    dom.add(ctl, El::div().class("setting-value").text(format!("v{}", env!("CARGO_PKG_VERSION"))));
    // The path is data, not a tooltip, so it wraps under its row instead of being shortened.
    let ctl = row(dom, parent, "Bóveda", "");
    dom.add(ctl, El::div().class("setting-value").text(app.store.root.display().to_string()));
    let ctl = row(dom, parent, "Notas", "Notas que el índice conoce ahora mismo.");
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
        row(dom, parent, "Buscar actualizaciones automáticamente", "Una vez al día, al iniciar MetaMind.");
    toggle(dom, ctl, crate::update::auto_check(), "settings:auto-update".into());
}

fn advanced_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, needle: &str) {
    if needle.is_empty() {
        heading(dom, parent, "Bóveda y actualizaciones");
        general_view(app, dom, parent);
        heading(dom, parent, "Archivos y enlaces");
    }
    items_view(app, dom, parent, ui, Section::Advanced, needle, false);
    if needle.is_empty() {
        heading(dom, parent, "Atajos de teclado");
        hotkeys_view(app, dom, parent, ui);
    }
}

/// The fields of a profile the settings form edits, keyed `ai.p:<field>:<index>`.
fn ai_field_key(key: &str) -> Option<(&str, usize)> {
    let (field, at) = key.strip_prefix("ai.p:")?.rsplit_once(':')?;
    Some((field, at.parse().ok()?))
}

/// The text a profile field opens with.
fn ai_field_text(cfg: &AiConfig, field: &str, at: usize) -> Option<String> {
    let p = cfg.profiles.get(at)?;
    Some(match field {
        "name" => p.name.clone(),
        "base_url" => p.base_url.clone(),
        "model" => p.model.clone(),
        "api_key" => p.api_key.clone(),
        "system_prompt" => p.system_prompt.clone(),
        "timeout_s" => p.timeout_s.to_string(),
        _ => return None,
    })
}

/// Whether the settings search text names something of Ajustes → IA.
fn ai_matches(needle: &str) -> bool {
    ["perfil", "perfiles", "proveedor", "proveedores", "modelo", "clave", "endpoint", "ollama"]
        .iter()
        .any(|w| w.contains(needle) || needle.contains(w))
        || category_matches(needle, Section::Ai)
}

/// A text control whose contents are never painted: a row of dots when a value is set, and one
/// dot per character while it is being edited.
fn masked_input(dom: &mut Dom, ctl: NodeId, key: &str, set: bool, edit: Option<&(String, String)>) -> NodeId {
    let editing = edit.filter(|e| e.0 == key);
    let text = match editing {
        Some(e) => format!("{}▏", "•".repeat(e.1.chars().count().clamp(1, 24))),
        None if set => "••••••••".to_string(),
        None => "Sin clave".to_string(),
    };
    let id = dom.add(
        ctl,
        El::div()
            .class("setting-input")
            .class_if(editing.is_some(), "is-editing")
            .text(text)
            .action(format!("settings:edit:{key}")),
    );
    if editing.is_some() {
        dom.nodes[id].focus = true;
    }
    id
}

fn ai_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi) {
    let cfg = AiConfig::from_config(&app.config);
    let active = cfg.active_index();
    heading(dom, parent, "Perfiles de proveedor");
    dom.add(
        parent,
        El::div().class("settings-note").text(
            "Cada perfil guarda la URL, el modelo y la clave de un proveedor compatible con OpenAI. \
             El copiloto usa el perfil activo: podés tener uno por servicio.",
        ),
    );
    for (i, p) in cfg.profiles.iter().enumerate() {
        let ctl = row(dom, parent, &p.name, &format!("{} · {}", p.model, p.base_url));
        if i == active {
            dom.add(ctl, El::div().class("setting-value").text("Activo"));
        } else {
            button(dom, ctl, "Activar", format!("settings:ai-use:{i}"), false);
        }
        button(dom, ctl, "Editar", format!("settings:ai-select:{i}"), false);
        button(dom, ctl, "Duplicar", format!("settings:ai-dup:{i}"), false);
        button(dom, ctl, "Eliminar", format!("settings:ai-del:{i}"), false);
    }
    let ctl = row(dom, parent, "Nuevo perfil", "Agrega un perfil con el endpoint predeterminado.");
    button(dom, ctl, "Agregar", "settings:ai-new".into(), true);

    let at = ui.ai_edit.filter(|&i| i < cfg.profiles.len()).unwrap_or(active);
    let p = &cfg.profiles[at];
    heading(dom, parent, &format!("Perfil «{}»", p.name));
    let ctl = row(dom, parent, "Nombre", "Cómo se llama en la lista y en el panel del copiloto.");
    input(dom, ctl, &format!("ai.p:name:{at}"), &p.name, ui.edit.as_ref(), Some("Mi perfil"));
    let ctl = row(
        dom,
        parent,
        "Proveedor",
        "Elegir un preset completa la URL y un modelo típico; «Personalizado» deja los tuyos.",
    );
    dom.add(
        ctl,
        El::div()
            .class("dropdown")
            .text(format!("{} ▾", p.preset().label()))
            .action(format!("settings:ai-preset:{at}")),
    );
    let ctl = row(dom, parent, "URL base", "Origen de la API, con o sin su /v1.");
    input(
        dom,
        ctl,
        &format!("ai.p:base_url:{at}"),
        &p.base_url,
        ui.edit.as_ref(),
        Some("https://api.deepseek.com"),
    );
    let ctl = row(dom, parent, "Modelo", "Nombre del modelo tal como lo espera el proveedor.");
    input(dom, ctl, &format!("ai.p:model:{at}"), &p.model, ui.edit.as_ref(), Some("deepseek-v4.1-flash"));
    let ctl = row(
        dom,
        parent,
        "Clave de API",
        "Se guarda en .obsidian/app.json y no se muestra en claro. Sin clave el panel no envía.",
    );
    masked_input(dom, ctl, &format!("ai.p:api_key:{at}"), !p.api_key.trim().is_empty(), ui.edit.as_ref());
    let ctl = row(
        dom,
        parent,
        "Prompt del sistema",
        "Instrucciones de este perfil. Vacío usa el prompt predeterminado.",
    );
    input(dom, ctl, &format!("ai.p:system_prompt:{at}"), &p.system_prompt, ui.edit.as_ref(), None);
    let ctl = row(dom, parent, "Tiempo de espera", "Segundos antes de cortar la conexión (1 a 600).");
    input(dom, ctl, &format!("ai.p:timeout_s:{at}"), &p.timeout_s.to_string(), ui.edit.as_ref(), Some("30"));
}

/// The settings window, drawn over everything else.
pub fn modal(app: &App, dom: &mut Dom) {
    let Some(ui) = &app.settings else { return };
    let t = app.anims.t("settings");
    let backdrop = El::div().class("modal-container mod-dim").action("settings:close");
    let mut modal = El::div().class("modal mod-settings").action("settings:noop");
    if t < 1.0 {
        modal = modal.style(&format!("margin-top: {:.1}px", 14.0 * (1.0 - t)));
    }
    let backdrop = dom.add(0, backdrop);
    let modal = dom.add(backdrop, modal);

    let bar = dom.add(modal, El::div().class("settings-topbar"));
    dom.add(bar, El::div().class("settings-topbar-title").text("Ajustes"));
    let search_box = dom.add(bar, El::div().class("settings-search"));
    dom.add(search_box, icons::el("search").class("settings-search-icon"));
    let search = ui.edit.as_ref().filter(|e| e.0 == "search").map_or(ui.search.as_str(), |e| e.1.as_str());
    input(dom, search_box, "search", search, ui.edit.as_ref(), Some("Buscar ajustes…"));
    // The hint only says how to close: Ctrl+, is what opens this window, and it is in the
    // command list and the ribbon already.
    dom.add(bar, El::div().class("settings-topbar-hint").text("Esc cierra"));
    let close = dom.add(
        bar,
        El::div()
            .class("settings-topbar-close")
            .attr("aria-label", "Cerrar ajustes")
            .action("settings:close"),
    );
    dom.add(close, icons::el("x"));

    let body = dom.add(modal, El::div().class("settings-body"));
    let tabs = dom.add(body, El::div().class("vertical-tab-header"));
    dom.add(tabs, El::div().class("vertical-tab-header-group-title").text("Categorías"));
    for (i, (sec, slug, name, _, icon)) in SECTIONS.iter().enumerate() {
        let item = dom.add(
            tabs,
            El::div()
                .class("vertical-tab-nav-item")
                .class_if(*sec == ui.section, "is-active")
                .class_if(ui.nav == Some(i), "is-focused")
                .action(format!("settings:section:{slug}")),
        );
        dom.add(item, icons::el(icon).class("vertical-tab-nav-icon"));
        dom.add(item, El::div().class("vertical-tab-nav-label").text(*name));
        if *sec == Section::Plugins {
            let n = app.plugin_registry().enabled_count();
            // Its own action, so hovering the badge explains the number in the status bar
            // instead of leaving it as a mark with no meaning; it opens the same page.
            dom.add(
                item,
                El::div()
                    .class("vertical-tab-nav-badge")
                    .text(n.to_string())
                    .action(format!("settings:badge:{n}")),
            );
        }
        if ui.nav == Some(i) {
            dom.nodes[item].focus = true;
        }
    }
    dom.add(tabs, El::div().class("settings-nav-note").text("Los cambios se guardan al instante."));

    let container = dom.add(body, El::div().class("vertical-tab-content-container"));
    let needle = search.trim().to_lowercase();
    if !needle.is_empty() {
        let results = ITEMS.iter().filter(|i| matches(&needle, i.name, i.desc)).count();
        search_header(dom, container, search.trim(), results);
    } else if !(ui.section == Section::Plugins && ui.plugin.is_some()) {
        section_header(dom, container, ui.section);
    }
    let content = dom.add(container, El::div().class("vertical-tab-content").id("settings-content"));
    if !needle.is_empty() {
        search_view(app, dom, content, ui, &needle);
    } else {
        match ui.section {
            Section::Appearance => appearance_view(app, dom, content, ui, true),
            Section::Editor => items_view(app, dom, content, ui, Section::Editor, "", false),
            Section::Ai => ai_view(app, dom, content, ui),
            Section::Plugins => plugins_view(app, dom, content, ui, ""),
            Section::Advanced => advanced_view(app, dom, content, ui, ""),
        }
    }
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
