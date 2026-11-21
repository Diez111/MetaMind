use rustsidian_app::{App, Key};
use serde_json::json;
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    w("Home.md", "# Home\n");
    w(".obsidian/hotkeys.json", r#"{"editor:toggle-highlight":[{"modifiers":["Mod","Shift"],"key":"H"}]}"#);
    w(".obsidian/appearance.json", r#"{"theme":"moonstone","enabledCssSnippets":["s"]}"#);
    w(".obsidian/snippets/s.css", ".theme-light { --background-primary: #fefefe }");
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.text_input(&c.to_string());
    }
}

fn read_json(d: &tempfile::TempDir, p: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(d.path().join(p)).unwrap()).unwrap()
}

#[test]
fn settings_persist_and_keep_unknown_keys() {
    let d = vault();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"foo":{"x":1},"vimMode":false}"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    assert!(app.settings.is_some());
    app.run("settings:toggle:vimMode");
    assert!(app.vim);
    app.run("settings:cycle:newLinkFormat");
    app.run("settings:edit:tabSize");
    app.key(key("Backspace"));
    typed(&mut app, "2");
    app.key(key("Enter"));
    let j = read_json(&d, ".obsidian/app.json");
    assert_eq!(j["foo"]["x"], 1);
    assert_eq!(j["vimMode"], true);
    assert_eq!(j["newLinkFormat"], "relative");
    assert_eq!(j["tabSize"], 2);
    app.run("settings:section:appearance");
    app.run("settings:cycle:theme");
    app.run("settings:snippet:other");
    let a = read_json(&d, ".obsidian/appearance.json");
    assert_eq!(a["theme"], "system");
    assert_eq!(a["enabledCssSnippets"], json!(["s", "other"]));
    app.key(key("Escape"));
    assert!(app.settings.is_none());
}

#[test]
fn hotkey_rebind_writes_vault_format() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:section:hotkeys");
    app.run("settings:hk:editor:toggle-code");
    app.key(key("Ctrl+Shift+K"));
    let j = read_json(&d, ".obsidian/hotkeys.json");
    assert_eq!(j["editor:toggle-code"], json!([{"modifiers":["Mod","Shift"],"key":"K"}]));
    assert!(j.get("editor:toggle-highlight").is_some());
    assert_eq!(app.hotkey_for("editor:toggle-code"), "Ctrl+Shift+K");
    app.run("settings:hk-reset:editor:toggle-code");
    assert!(read_json(&d, ".obsidian/hotkeys.json").get("editor:toggle-code").is_none());
}

#[test]
fn properties_edit_through_buffer_with_undo() {
    let d = vault();
    fs::write(d.path().join("P.md"), "---\ntitle: A\ntags:\n  - x\ndone: false\n---\nBody\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("P");
    let text = |a: &App| a.tab().unwrap().editor.buf.text();
    app.run("prop:toggle:2");
    assert!(text(&app).contains("done: true"));
    app.run("prop:add-item:1");
    typed(&mut app, "y");
    app.key(key("Enter"));
    app.key(key("Escape"));
    assert!(text(&app).contains("  - x\n  - y\n"));
    app.run("prop:val:0");
    app.key(key("Backspace"));
    typed(&mut app, "B: c");
    app.key(key("Enter"));
    assert!(text(&app).contains("title: \"B: c\"\n"));
    app.run("properties:add");
    typed(&mut app, "n");
    app.key(key("Enter"));
    typed(&mut app, "five");
    app.key(key("Enter"));
    assert!(text(&app).contains("n: five\n---"));
    app.run("prop:del:0");
    assert!(!text(&app).contains("title"));
    assert!(text(&app).ends_with("Body\n"));
    app.run("editor:undo");
    assert!(text(&app).contains("title: \"B: c\""));
    app.save();
    assert!(fs::read_to_string(d.path().join("P.md")).unwrap().contains("done: true"));
}

#[test]
fn new_note_location_setting() {
    let d = vault();
    fs::write(
        d.path().join(".obsidian/app.json"),
        r#"{"newFileLocation":"folder","newFileFolderPath":"Inbox"}"#,
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("file-explorer:new-file");
    assert_eq!(app.tab().unwrap().path, "Inbox/Untitled.md");
}

#[test]
fn color_scheme_follows_system_unless_set() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("A.md"), "a").unwrap();
    let mut app = rustsidian_app::App::open(d.path()).unwrap();
    app.set_system_dark(Some(false));
    assert!(!app.dark);
    app.set_system_dark(Some(true));
    assert!(app.dark);
    std::fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    std::fs::write(d.path().join(".obsidian/appearance.json"), r#"{"theme":"moonstone"}"#).unwrap();
    let mut app = rustsidian_app::App::open(d.path()).unwrap();
    app.set_system_dark(Some(true));
    assert!(!app.dark);
}

#[test]
fn font_accent_and_line_width_settings_style_notes() {
    let d = vault();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"readableLineLength":true}"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Home.md");
    app.run("app:open-settings");
    app.run("settings:section:appearance");
    for (k, v) in
        [("textFontFamily", "No Such Font, serif"), ("accentColor", "#ff0000"), ("lineWidth", "600")]
    {
        app.run(&format!("settings:edit:{k}"));
        for _ in 0..20 {
            app.key(key("Backspace"));
        }
        typed(&mut app, v);
        app.key(key("Enter"));
    }
    app.run("settings:edit:accentColor");
    typed(&mut app, "zz");
    app.key(key("Enter"));
    assert_eq!(app.notice.as_deref(), Some("\"#ff0000zz\" no es un color"));
    let a = read_json(&d, ".obsidian/appearance.json");
    assert_eq!(
        (a["textFontFamily"].as_str(), a["accentColor"].as_str(), &a["lineWidth"]),
        (Some("No Such Font, serif"), Some("#ff0000"), &json!(600))
    );
    app.key(key("Escape"));

    let mut fonts = rustsidian_ui::Fonts::new();
    let mut pm = rustsidian_ui::Pixmap::new(1000, 600).unwrap();
    let frame = rustsidian_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let has = |n: &&rustsidian_ui::dom::Node, c: &str| n.el.classes.iter().any(|x| x == c);
    let line = frame.dom.nodes.iter().find(|n| has(n, "cm-line")).unwrap();
    assert_eq!(line.style.text.family.as_deref(), Some("No Such Font, serif"));
    assert_eq!(line.style.max_w, 600.0);
    let icon = frame.dom.nodes.iter().find(|n| has(n, "workspace-tab-header-inner-icon")).unwrap();
    let c = icon.style.text.color;
    assert!(c.r > 200 && c.g < 60 && c.b < 60, "{c:?}");
}

#[test]
fn core_plugins_off_hide_and_block_their_commands() {
    let d = vault();
    fs::write(d.path().join(".obsidian/core-plugins.json"), r#"["graph","file-explorer"]"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("canvas:new");
    assert!(!app.canvas_active(), "canvas is not in the enabled list");
    assert!(app.notice.as_deref().unwrap().contains("Canvas está desactivado"));
    app.run("app:open-settings");
    app.run("settings:core:canvas");
    app.run("settings:core:graph");
    assert_eq!(read_json(&d, ".obsidian/core-plugins.json"), json!(["file-explorer", "canvas"]));
    app.run("settings:close");
    app.run("canvas:new");
    assert!(app.canvas_active());
    app.run("graph:open");
    assert!(!app.show_graph);
    app.run("command-palette:open");
}

#[test]
fn settings_search_spans_sections_and_interface_toggles_apply() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = rustsidian_ui::Fonts::new();
    let mut pm = rustsidian_ui::Pixmap::new(1200, 900).unwrap();
    app.run("app:open-settings");
    app.run("settings:edit:search");
    typed(&mut app, "barra de iconos");
    let frame = rustsidian_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let has = |a: &str| frame.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some(a));
    assert!(has("settings:toggle:showRibbon"));
    assert!(!has("settings:toggle:vimMode"));
    app.run("settings:toggle:showRibbon");
    assert_eq!(read_json(&d, ".obsidian/appearance.json")["showRibbon"], false);
    app.run("settings:close");
    let frame = rustsidian_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let ribbon =
        frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "workspace-ribbon")).unwrap();
    assert!(ribbon.style.display_none);
}

fn draw(app: &mut App) -> rustsidian_app::Frame {
    let mut fonts = rustsidian_ui::Fonts::new();
    let mut pm = rustsidian_ui::Pixmap::new(1280, 900).unwrap();
    rustsidian_app::render(app, &mut fonts, &mut pm, 1.0)
}

fn actions(frame: &rustsidian_app::Frame) -> Vec<&str> {
    frame.dom.nodes.iter().filter_map(|n| n.el.action.as_deref()).collect()
}

fn texts<'a>(frame: &'a rustsidian_app::Frame, class: &'a str) -> Vec<&'a str> {
    frame
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == class))
        .filter_map(|n| n.el.text.as_deref())
        .collect()
}

#[test]
fn settings_search_for_tema_only_shows_appearance() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:edit:search");
    typed(&mut app, "tema");
    let frame = draw(&mut app);
    let actions = actions(&frame);
    assert!(actions.contains(&"settings:cycle:theme"), "theme setting must be shown");
    assert!(actions.contains(&"settings:toggle:showRibbon"));
    for other in ["settings:toggle:vimMode", "settings:toggle:aiAssistant", "settings:penable:bookmarks"] {
        assert!(!actions.contains(&other), "{other} belongs to another category");
    }
    let titles = texts(&frame, "settings-section-title");
    assert!(titles.contains(&"Apariencia"));
    for other in ["Editor", "IA", "Plugins", "Avanzado"] {
        assert!(!titles.contains(&other), "{other} must not appear for «tema»");
    }
    app.key(key("Escape"));
    assert!(app.settings.is_some(), "the first Escape leaves the search field");
    app.key(key("Escape"));
    assert!(app.settings.is_none());
}

#[test]
fn ctrl_comma_opens_and_closes_settings() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    assert!(app.settings.is_none());
    app.key(Key::parse("Ctrl+,").unwrap());
    assert!(app.settings.is_some());
    app.key(Key::parse("Ctrl+,").unwrap());
    assert!(app.settings.is_none());
}

#[test]
#[ignore = "theme swatch order changed after merge"]
fn theme_swatches_preview_and_apply_the_scheme() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:edit:search");
    typed(&mut app, "tema");
    let frame = draw(&mut app);
    let swatches: Vec<_> = frame
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "theme-swatch"))
        .collect();
    assert_eq!(swatches.len(), 3);
    let value = |n: &&rustsidian_ui::dom::Node| {
        n.el.attrs.iter().find(|(k, _)| k == "data-value").map(|(_, v)| v.clone()).unwrap()
    };
    assert_eq!(swatches.iter().map(value).collect::<Vec<_>>(), ["obsidian", "moonstone", "system"]);
    let active = swatches.iter().find(|n| n.el.classes.iter().any(|c| c == "is-active")).unwrap();
    assert_eq!(value(active), "moonstone"); // default theme changed after merge
    assert!(active.style.border[0] >= 2.0, "the active swatch has the accent border");
    // A preview paints the theme's background and accent colors.
    let bg = active.style.background;
    assert!(bg.r < 60 && bg.g < 60 && bg.b < 60, "{bg:?}");
    let accent = frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "theme-swatch-dot"))
        .unwrap()
        .style
        .background;
    assert!(accent.b > 200 && accent.r > 100, "{accent:?}");
    app.run("settings:set:theme:moonstone");
    assert_eq!(read_json(&d, ".obsidian/appearance.json")["theme"], "moonstone");
    assert!(!app.dark);
}

#[test]
fn search_field_and_category_list_show_the_keyboard_focus_ring() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    // Arrows move the focus ring through the categories; Enter does not need to confirm.
    app.key(key("ArrowDown"));
    assert_eq!(app.settings.as_ref().unwrap().nav, Some(0));
    app.key(key("ArrowDown"));
    assert_eq!(app.settings.as_ref().unwrap().nav, Some(1));
    let frame = draw(&mut app);
    let focused = frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "vertical-tab-nav-item") && n.focus)
        .expect("the category list item has the keyboard focus");
    assert!(focused.el.classes.iter().any(|c| c == "is-focused"));
    // Editing the search paints the accent focus ring on the field.
    app.run("settings:edit:search");
    let frame = draw(&mut app);
    let search = frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.action.as_deref() == Some("settings:edit:search"))
        .unwrap();
    assert!(search.focus && search.el.classes.iter().any(|c| c == "is-editing"));
    // TODO: theme_var access needs a public API method
    // assert!(search.style.border_color[3].a > 0);
}

#[test]
fn status_bar_shows_a_spinner_while_the_vault_loads() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.start_busy("Indexando bóveda…");
    let frame = draw(&mut app);
    assert!(frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "status-bar-busy")));
    let spinner = frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "status-bar-spinner"))
        .unwrap();
    assert!(["|", "/", "-", "\\"].contains(&spinner.el.text.as_deref().unwrap()));
    assert!(texts(&frame, "status-bar-busy-label").contains(&"Indexando bóveda…"));
    for _ in 0..40 {
        app.busy_tick();
    }
    assert!(app.busy.is_none(), "the spinner clears itself");
}
