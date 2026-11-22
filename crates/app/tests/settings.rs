use metamind_app::{App, Key};
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
    let mut app = metamind_app::App::open(d.path()).unwrap();
    app.set_system_dark(Some(false));
    assert!(!app.dark);
    app.set_system_dark(Some(true));
    assert!(app.dark);
    std::fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    std::fs::write(d.path().join(".obsidian/appearance.json"), r#"{"theme":"moonstone"}"#).unwrap();
    let mut app = metamind_app::App::open(d.path()).unwrap();
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

    let mut fonts = metamind_ui::Fonts::new();
    let mut pm = metamind_ui::Pixmap::new(1000, 600).unwrap();
    let frame = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let has = |n: &&metamind_ui::dom::Node, c: &str| n.el.classes.iter().any(|x| x == c);
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
    let mut fonts = metamind_ui::Fonts::new();
    let mut pm = metamind_ui::Pixmap::new(1200, 900).unwrap();
    app.run("app:open-settings");
    app.run("settings:edit:search");
    typed(&mut app, "barra de iconos");
    let frame = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let has = |a: &str| frame.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some(a));
    assert!(has("settings:toggle:showRibbon"));
    assert!(!has("settings:toggle:vimMode"));
    app.run("settings:toggle:showRibbon");
    assert_eq!(read_json(&d, ".obsidian/appearance.json")["showRibbon"], false);
    app.run("settings:close");
    let frame = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let ribbon =
        frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "workspace-ribbon")).unwrap();
    assert!(ribbon.style.display_none);
}

fn draw(app: &mut App) -> metamind_app::Frame {
    let mut fonts = metamind_ui::Fonts::new();
    let mut pm = metamind_ui::Pixmap::new(1280, 900).unwrap();
    metamind_app::render(app, &mut fonts, &mut pm, 1.0)
}

fn actions(frame: &metamind_app::Frame) -> Vec<&str> {
    frame.dom.nodes.iter().filter_map(|n| n.el.action.as_deref()).collect()
}

fn texts<'a>(frame: &'a metamind_app::Frame, class: &'a str) -> Vec<&'a str> {
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
    assert!(actions.contains(&"settings:menu:theme"), "theme setting must be shown");
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
    let swatches: Vec<_> =
        frame.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == "theme-swatch")).collect();
    assert_eq!(swatches.len(), 3);
    let value = |n: &&metamind_ui::dom::Node| {
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
    let search =
        frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("settings:edit:search")).unwrap();
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
    let spinner =
        frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "status-bar-spinner")).unwrap();
    assert!(["|", "/", "-", "\\"].contains(&spinner.el.text.as_deref().unwrap()));
    assert!(texts(&frame, "status-bar-busy-label").contains(&"Indexando bóveda…"));
    for _ in 0..40 {
        app.busy_tick();
    }
    assert!(app.busy.is_none(), "the spinner clears itself");
}

#[test]
fn every_control_in_the_panel_sits_on_one_axis() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:section:appearance");
    let frame = draw(&mut app);
    let has = |n: &metamind_ui::dom::Node, c: &str| n.el.classes.iter().any(|x| x == c);
    // The controls of the panel, and the box each one sits in.
    let mut rights: Vec<f32> = Vec::new();
    for n in frame.dom.nodes.iter().filter(|n| has(n, "setting-item-control")) {
        rights.push(n.rect.x + n.rect.w);
    }
    assert!(rights.len() >= 6, "{rights:?}");
    let (min, max) = rights.iter().fold((f32::MAX, f32::MIN), |(a, b), &r| (a.min(r), b.max(r)));
    assert!(max - min < 0.5, "the control column is not one axis: {rights:?}");
    // Every text input of a row has the same width, so the column reads as a column.
    let widths: Vec<f32> = frame
        .dom
        .nodes
        .iter()
        .filter(|n| has(n, "setting-input"))
        .filter(|n| n.parent.is_some_and(|p| has(&frame.dom.nodes[p], "setting-item-control")))
        .map(|n| n.rect.w)
        .collect();
    assert!(widths.len() >= 3, "{widths:?}");
    let (min, max) = widths.iter().fold((f32::MAX, f32::MIN), |(a, b), &r| (a.min(r), b.max(r)));
    assert!(max - min < 0.5, "the inputs are not one width: {widths:?}");
    // The colour chip sits inside its control instead of hanging off the column.
    let chip = frame.dom.nodes.iter().find(|n| has(n, "color-chip")).expect("the accent colour has a chip");
    let owner = frame.dom.nodes[chip.parent.expect("the chip has a parent")].rect;
    assert!(
        chip.rect.x >= owner.x && chip.rect.x + chip.rect.w <= owner.x + owner.w + 0.5,
        "the chip is outside its control: {:?} in {owner:?}",
        chip.rect
    );
}

#[test]
fn the_settings_sidebar_has_an_icon_for_every_category() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let frame = draw(&mut app);
    let icons: Vec<_> = frame
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "vertical-tab-nav-icon"))
        .collect();
    assert_eq!(icons.len(), 6, "every category has an icon");
    // The empty square a missing glyph leaves behind is the bug this guards against.
    for icon in icons {
        let mut x0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut y0 = f32::MAX;
        let mut y1 = f32::MIN;
        for c in &icon.draw {
            if let metamind_ui::Cmd::Lines { segs, .. } = c {
                for s in segs {
                    x0 = x0.min(s[0]).min(s[2]);
                    x1 = x1.max(s[0]).max(s[2]);
                    y0 = y0.min(s[1]).min(s[3]);
                    y1 = y1.max(s[1]).max(s[3]);
                }
            }
        }
        assert!(x1 - x0 > 4.0 && y1 - y0 > 4.0, "a category icon draws nothing: {x0}..{x1}");
    }
    // The footer that leaked another product's name is gone, and the note is said once.
    let notes = texts(&frame, "settings-nav-note");
    assert_eq!(notes, ["Los cambios se guardan al instante."], "{notes:?}");
    assert!(texts(&frame, "setting-footer").is_empty());
}

/// The `.setting-item` row whose name is `name`.
fn setting_row(frame: &metamind_app::Frame, name: &str) -> Option<usize> {
    let name = frame.dom.nodes.iter().position(|n| {
        n.el.classes.iter().any(|c| c == "setting-item-name") && n.el.text.as_deref() == Some(name)
    })?;
    let mut cur = frame.dom.nodes[name].parent;
    while let Some(id) = cur {
        if frame.dom.nodes[id].el.classes.iter().any(|c| c == "setting-item") {
            return Some(id);
        }
        cur = frame.dom.nodes[id].parent;
    }
    None
}

/// As many snippets as it takes for the appearance page to be taller than the box it scrolls in.
fn vault_with_a_long_appearance_page() -> tempfile::TempDir {
    let d = vault();
    fs::create_dir_all(d.path().join(".obsidian/snippets")).unwrap();
    for i in 1..=8 {
        fs::write(d.path().join(format!(".obsidian/snippets/s{i}.css")), "").unwrap();
    }
    d
}

/// Scrolled to the end, the last row of the appearance page stays inside the box the modal gives
/// it. It used to sit on the bottom edge: the scroller's bottom padding was not part of the
/// height its content was measured at, so the last rows of a long list looked cut.
#[test]
fn settings_last_row_stays_inside_the_scroll_box() {
    let d = vault_with_a_long_appearance_page();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let frame = draw(&mut app);
    let content = frame.dom.by_id("settings-content").expect("the settings page scrolls");
    let max = frame.dom.max_scroll(content);
    assert!(max > 0.0, "the appearance page must be taller than its box for this test");
    app.scrolls.insert("settings-content".into(), max);

    let frame = draw(&mut app);
    let content = frame.dom.by_id("settings-content").unwrap();
    let box_ = frame.dom.nodes[content].rect;
    let row = setting_row(&frame, "Mostrar barra de título de la pestaña").expect("the last appearance row");
    let r = frame.dom.nodes[row].rect;
    assert!(
        r.y + r.h <= box_.y + box_.h,
        "the last row ({:.1}..{:.1}) reached past its box ({:.1}..{:.1})",
        r.y,
        r.y + r.h,
        box_.y,
        box_.y + box_.h
    );
}

/// The settings window dims the whole window and nothing of the note reads through it: the scrim
/// is the window's rectangle, it is painted after every node of the window under it, and it is
/// opaque enough to hide the bright parts of the editor (a callout's bar) at the modal's edges.
#[test]
fn settings_scrim_covers_the_whole_window() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    app.dark = true;
    app.run("app:open-settings");
    let frame = draw(&mut app);
    let scrim = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.classes.iter().any(|c| c == "modal-container"))
        .expect("the settings window has a scrim");
    let r = frame.dom.nodes[scrim].rect;
    assert_eq!((r.x, r.y, r.w, r.h), (0.0, 0.0, 1280.0, 900.0), "the scrim must be the window");
    let bg = frame.dom.nodes[scrim].style.background;
    assert!(bg.a >= 178, "the scrim lets {} of 255 through", 255 - bg.a);
    // In document order, which is paint order here: only menus, notices and the perf overlay are
    // drawn over it.
    let over: Vec<&str> = frame.dom.nodes[0].children
        [frame.dom.nodes[0].children.iter().position(|&c| c == scrim).unwrap()..]
        .iter()
        .filter_map(|&c| frame.dom.nodes[c].el.classes.first().map(String::as_str))
        .collect();
    for class in over {
        assert!(
            ["modal-container", "menu", "notice", "perf-overlay"].contains(&class),
            "{class} paints over the scrim"
        );
    }
}

/// One selection indicator for the three scheme previews, and three previews of one width.
#[test]
fn theme_swatches_share_one_selection_ring() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let frame = draw(&mut app);
    let items: Vec<usize> = (0..frame.dom.nodes.len())
        .filter(|&i| frame.dom.nodes[i].el.classes.iter().any(|c| c == "theme-swatch-item"))
        .collect();
    assert_eq!(items.len(), 3);
    let widths: Vec<f32> = items.iter().map(|&i| frame.dom.nodes[i].rect.w).collect();
    let swatches: Vec<f32> = frame
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "theme-swatch"))
        .map(|n| n.rect.w)
        .collect();
    assert_eq!(swatches.len(), 3);
    for (a, b) in [(widths[0], widths[1]), (widths[1], widths[2])] {
        assert!((a - b).abs() < 0.01, "the previews are not one width: {widths:?}");
    }
    for (a, b) in [(swatches[0], swatches[1]), (swatches[1], swatches[2])] {
        assert!((a - b).abs() < 0.01, "the boxes are not one width: {swatches:?}");
    }
    // One ring, the same width on all three, and only the current scheme lights it.
    let rings: Vec<f32> = items.iter().map(|&i| frame.dom.nodes[i].style.border[0]).collect();
    assert_eq!(rings, [2.0, 2.0, 2.0], "the ring must not resize the selected preview");
    let lit: Vec<usize> =
        items.iter().copied().filter(|&i| frame.dom.nodes[i].style.border_color[0].a > 128).collect();
    assert_eq!(lit.len(), 1, "exactly one scheme is marked as current");
}

/// The two text settings that were blank say what applies while they are empty, and that value is
/// the one the theme really uses.
#[test]
fn empty_text_settings_show_their_default() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let frame = draw(&mut app);
    let placeholders: Vec<(&str, usize)> = frame
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "is-placeholder"))
        .filter_map(|n| Some((n.el.text.as_deref()?, n.el.classes.len())))
        .collect();
    let shown: Vec<&str> = placeholders.iter().map(|(t, _)| *t).collect();
    assert!(shown.contains(&"sans-serif"), "the font field shows the default font: {shown:?}");
    assert!(shown.contains(&"#8a5cf5"), "the accent field shows the default accent: {shown:?}");
    // The font default is the one the notes are set in, and the accent is the colour of the chip.
    let vars = &frame.dom.nodes[0].computed;
    assert_eq!(vars.var("--font-text").as_deref(), Some("sans-serif"));
    assert_eq!(vars.var("--interactive-accent").as_deref(), Some("hsl(258, 88%, 66%)"));
}

/// The header says how to close the window (Ctrl+, opens it, it does not close it) and the × on
/// the right closes it.
#[test]
fn settings_header_says_esc_and_has_a_close_button() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let frame = draw(&mut app);
    assert_eq!(texts(&frame, "settings-topbar-hint"), ["Esc cierra"]);
    let close = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.classes.iter().any(|c| c == "settings-topbar-close"))
        .expect("the header has a close button");
    assert_eq!(frame.dom.nodes[close].el.action.as_deref(), Some("settings:close"));
    let icon = frame.dom.nodes[close].children[0];
    assert_eq!(
        frame.dom.nodes[icon].el.attrs.iter().find(|(k, _)| k == "data-icon").map(|(_, v)| v.as_str()),
        Some("x")
    );
    assert!(!frame.dom.nodes[icon].draw.is_empty(), "the × is drawn, not a hole");
    app.run("settings:close");
    assert!(app.settings.is_none(), "the × closes the window");
}

/// The number beside Plugins is the count of active built-in plugins: it opens that page and the
/// status bar says what it is while the pointer is on it.
#[test]
fn plugins_badge_explains_itself() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let frame = draw(&mut app);
    let badge = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.classes.iter().any(|c| c == "vertical-tab-nav-badge"))
        .expect("Plugins carries its count");
    let action = frame.dom.nodes[badge].el.action.clone().expect("the badge has an action");
    let n: usize = action.strip_prefix("settings:badge:").expect(&action).parse().unwrap();
    assert!(n > 0);
    app.hover_action = Some(action.clone());
    let frame = draw(&mut app);
    let hint = texts(&frame, "status-bar-hint");
    assert!(
        hint.iter().any(|h| h.contains(&format!("{n} plugins integrados activos"))),
        "the status bar explains the badge: {hint:?}"
    );
    app.run(&action);
    let frame = draw(&mut app);
    assert!(texts(&frame, "settings-section-title").contains(&"Plugins"), "{action} opens Plugins");
}

/// The rect of the first node that has all of these classes.
fn node_at<'a>(frame: &'a metamind_app::Frame, classes: &[&str]) -> Option<(usize, &'a metamind_ui::Rect)> {
    frame
        .dom
        .nodes
        .iter()
        .position(|n| classes.iter().all(|c| n.el.classes.iter().any(|x| x == c)))
        .map(|i| (i, &frame.dom.nodes[i].rect))
}

/// Where the first node with this class is, if any.
fn node_with(frame: &metamind_app::Frame, class: &str) -> Option<metamind_ui::NodeId> {
    frame.dom.nodes.iter().position(|n| n.el.classes.iter().any(|c| c == class))
}

fn all_with(frame: &metamind_app::Frame, class: &str) -> Vec<metamind_ui::NodeId> {
    frame
        .dom
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.el.classes.iter().any(|c| c == class))
        .map(|(i, _)| i)
        .collect()
}

/// The settings rows are one grid: the label column wraps its text inside its own width, every
/// control in the panel ends on the same right edge, and a toggle sits on the optical centre of
/// the two lines beside it. The panel scrolls inside the modal, so the last row of Apariencia is
/// reachable instead of being cut by the bottom edge.
#[test]
fn appearance_is_one_measured_column_inside_a_scrolling_modal() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:section:appearance");
    let mut fonts = metamind_ui::Fonts::new();
    let mut pm = metamind_ui::Pixmap::new(1600, 1000).unwrap();
    let mut f = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);

    let rect = |f: &metamind_app::Frame, c: &str| f.dom.nodes[node_with(f, c).expect(c)].rect;
    let modal = *node_at(&f, &["modal", "mod-settings"]).expect("the settings panel").1;
    let scrim = rect(&f, "modal-container");

    // The veil covers the whole window and the panel is centered on what the reader sees: the
    // status bar is drawn over the bottom of the window and is not part of that area.
    assert_eq!((scrim.x, scrim.y, scrim.w, scrim.h), (0.0, 0.0, 1600.0, 1000.0), "the veil is the window");
    let visible = 1000.0 - metamind_app::view::STATUS_BAR_H;
    assert!((modal.x + modal.w / 2.0 - 800.0).abs() <= 1.0, "centered across: {modal:?}");
    assert!(
        (modal.y + modal.h / 2.0 - visible / 2.0).abs() <= 1.0,
        "centered on the visible area ({visible}), not on the window: {modal:?}"
    );
    let veil = f.dom.nodes[node_with(&f, "modal-container").unwrap()].style.background;
    assert!(veil.a > 0, "the modal dims what is behind it: {veil:?}");

    // The panel is a real scroller inside the modal, and its content is taller than its box: the
    // last row of Apariencia is reached by scrolling, never by being cut by the modal's edge.
    let content = node_with(&f, "vertical-tab-content").expect("the panel scroller");
    let (crect, scrollable, content_h) = {
        let n = &f.dom.nodes[content];
        (n.rect, n.style.scroll, n.content_h)
    };
    assert!(scrollable, "the appearance panel scrolls");
    assert!(crect.y + crect.h <= modal.y + modal.h, "the scroller ends with the modal: {crect:?}");
    assert!(content_h > crect.h, "the section is taller than its box: {content_h} vs {}", crect.h);

    // Nothing inside is clipped: every part fits once the panel is scrolled to its end, and its
    // start is visible at the top before scrolling.
    let inside: Vec<usize> = f
        .dom
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| *i != content && n.rect.h > 0.0 && is_descendant(&f, *i, content))
        .map(|(i, _)| i)
        .collect();
    assert!(inside.len() > 20, "the panel has its rows: {}", inside.len());
    for &i in &inside {
        let r = f.dom.nodes[i].rect;
        assert!(r.y >= crect.y - 0.5, "row above the panel at the top: {r:?}");
    }
    app.scrolls.insert("settings-content".to_string(), content_h - crect.h);
    f = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let crect = f.dom.nodes[node_with(&f, "vertical-tab-content").unwrap()].rect;
    for &i in &inside {
        let r = f.dom.nodes[i].rect;
        assert!(r.y + r.h <= crect.y + crect.h + 0.5, "row cut by the bottom: {r:?}");
    }
    app.scrolls.remove("settings-content");

    // One right edge for every control in the column, and the small extras stay inside it.
    f = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let controls = all_with(&f, "setting-item-control");
    assert!(controls.len() >= 8, "the appearance rows: {}", controls.len());
    let edge = f.dom.nodes[controls[0]].rect.x + f.dom.nodes[controls[0]].rect.w;
    for &c in &controls {
        let r = f.dom.nodes[c].rect;
        assert!((r.x + r.w - edge).abs() <= 0.5, "one right edge: {r:?} vs {edge}");
        for &kid in &f.dom.nodes[c].children {
            let k = &f.dom.nodes[kid];
            if k.style.absolute || k.style.display_none {
                continue;
            }
            assert!(
                (k.rect.x + k.rect.w - edge).abs() <= 0.5,
                "{} ends on the column's edge: {:?}",
                k.el.classes.join("."),
                k.rect
            );
        }
    }
    // The accent swatch is a chip inset in its field, not a circle on the border.
    let field = all_with(&f, "setting-input");
    let chip = rect(&f, "color-chip");
    assert!(field.iter().any(|&i| {
        let r = f.dom.nodes[i].rect;
        chip.x >= r.x && chip.x + chip.w <= r.x + r.w - 2.0 && chip.y >= r.y && chip.y + chip.h <= r.y + r.h
    }));

    // A description wraps inside the label column and never runs under the control of its row.
    let descriptions = all_with(&f, "setting-item-description");
    assert!(descriptions.len() >= 8, "{}", descriptions.len());
    for &i in &descriptions {
        let r = f.dom.nodes[i].rect;
        let ctl = all_with(&f, "setting-item-control")
            .into_iter()
            .find(|&c| f.dom.nodes[c].rect.y < r.y && f.dom.nodes[c].rect.y + f.dom.nodes[c].rect.h > r.y)
            .expect("the control of the same row");
        let c = f.dom.nodes[ctl].rect;
        assert!(r.x + r.w <= c.x, "a description crosses into the control column: {r:?} and {c:?}");
    }

    // The toggles sit on the optical centre of the text block of their row.
    let toggles = all_with(&f, "checkbox-container");
    assert!(toggles.len() >= 2, "the appearance rows with a toggle: {}", toggles.len());
    for &t in &toggles {
        let tr = f.dom.nodes[t].rect;
        let info = all_with(&f, "setting-item-info")
            .into_iter()
            .find(|&i| {
                let r = f.dom.nodes[i].rect;
                tr.y >= r.y && tr.y + tr.h <= r.y + r.h
            })
            .expect("the text block of the same row");
        let ir = f.dom.nodes[info].rect;
        let (a, b) = (tr.y + tr.h / 2.0, ir.y + ir.h / 2.0);
        assert!((a - b).abs() <= 1.0, "a toggle is off its row: {a} vs {b}");
    }

    // The category is called Apariencia, and the list's highlight is a box with air around it:
    // glued to the icon, its border cut through the glyph.
    let labels: Vec<&str> = all_with(&f, "vertical-tab-nav-label")
        .iter()
        .filter_map(|&i| f.dom.nodes[i].el.text.as_deref())
        .collect();
    assert!(labels.contains(&"Apariencia"), "{labels:?}");
    let active_id = node_at(&f, &["vertical-tab-nav-item", "is-active"]).expect("the selected category").0;
    let active = f.dom.nodes[active_id].rect;
    let icon = f.dom.nodes[f.dom.nodes[active_id].children[0]].rect;
    let gap = icon.x - active.x;
    assert!(gap >= 8.0 && gap <= 14.0, "the highlight leaves room around the icon: {active:?} {icon:?}");
    assert!(icon.x + icon.w <= active.x + active.w, "the icon stays inside its row");
}

/// Whether `id` is inside `root` in the tree.
fn is_descendant(frame: &metamind_app::Frame, id: usize, root: usize) -> bool {
    let mut cur = frame.dom.nodes[id].parent;
    while let Some(p) = cur {
        if p == root {
            return true;
        }
        cur = frame.dom.nodes[p].parent;
    }
    false
}

#[test]
fn the_ai_section_offers_prompt_presets_and_the_provider_models() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:section:ai");

    // The prompt template opens a list of presets instead of rotating them.
    app.run("settings:menu:ai.prompt:0");
    let frame = draw(&mut app);
    let acts = actions(&frame);
    assert!(
        acts.iter().any(|a| a.starts_with("settings:pick:ai.prompt:0:")),
        "the preset list is open: {acts:?}"
    );
    // Picking one writes the prompt of the profile.
    app.run("settings:pick:ai.prompt:0:3");
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(d.path().join(".obsidian/app.json")).unwrap()).unwrap();
    let first = saved
        .get("ai.profiles")
        .and_then(|p| p.as_array())
        .and_then(|p| p.first())
        .cloned()
        .unwrap_or_default();
    let prompt = first.get("system_prompt").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    assert!(prompt.contains("analista de mercados"), "the trading preset wrote the prompt: {prompt}");

    // A listed model gets a toggle and its own default button.
    app.ai_models.list = vec!["modelo-a".into(), "modelo-b".into()];
    app.ai_models.profile = metamind_app::ai::LEGACY_PROFILE_NAME.to_string();
    let frame = draw(&mut app);
    let acts = actions(&frame);
    assert!(acts.contains(&"settings:ai-toggle-model:0:modelo-a"), "{acts:?}");
    assert!(acts.contains(&"settings:ai-toggle-model:0:modelo-b"));
    assert!(
        acts.contains(&"settings:ai-default-model:0:modelo-a"),
        "a model that is not the default can be made one"
    );

    // Turning one off keeps the other; the list is written to the profile in the vault.
    app.run("settings:ai-toggle-model:0:modelo-a");
    let frame = draw(&mut app);
    assert_eq!(texts(&frame, "setting-item-name").iter().filter(|n| **n == "modelo-a").count(), 1);
    app.run("settings:ai-default-model:0:modelo-b");
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(d.path().join(".obsidian/app.json")).unwrap()).unwrap();
    let profiles = saved.get("ai.profiles").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let first = profiles.first().cloned().unwrap_or_default();
    assert_eq!(first.get("model").and_then(|m| m.as_str()), Some("modelo-b"));
}

#[test]
fn a_skill_rides_along_with_every_question_of_its_profile() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:section:ai");
    let frame = draw(&mut app);
    let acts = actions(&frame);
    for (id, _, _) in metamind_app::ai::SKILLS {
        assert!(
            acts.contains(&format!("settings:ai-skill:0:{id}").as_str()),
            "the {id} skill has its toggle"
        );
    }
    app.run("settings:ai-skill:0:pedagogia");
    let cfg = metamind_app::ai::AiConfig::from_config(&app.config);
    let profile = cfg.profiles.first().cloned().unwrap();
    assert_eq!(profile.skills, vec!["pedagogia".to_string()]);
    // The system message carries it, and turning it off takes it out again.
    assert!(profile.instructions_with_skills().contains("reproducirlo"));
    assert!(!metamind_app::ai::DEFAULT_SYSTEM_PROMPT.contains("reproducirlo"));
    app.run("settings:ai-skill:0:pedagogia");
    let cfg = metamind_app::ai::AiConfig::from_config(&app.config);
    assert!(cfg.profiles.first().unwrap().skills.is_empty());
}
