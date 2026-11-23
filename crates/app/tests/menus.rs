use metamind_app::shell::Key;
use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, t) in [("A.md", "# A\nsee [[B]]\n"), ("B.md", "b\n"), ("dir/C.md", "c [[B]]\n")] {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    }
    d
}

fn center(frame: &Frame, action: &str) -> (f32, f32) {
    let n = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some(action)).unwrap();
    (n.rect.x + n.rect.w / 2.0, n.rect.y + n.rect.h / 2.0)
}

fn labels(app: &App) -> Vec<String> {
    app.menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter(|i| !i.action.is_empty())
        .map(|i| i.label.clone())
        .collect()
}

/// Click the menu row with this label.
fn pick(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap, label: &str) {
    let i = app.menu.as_ref().unwrap().items.iter().position(|i| i.label == label).unwrap();
    let frame = render(app, fonts, pm, 1.0);
    let (x, y) = center(&frame, &format!("menu:{i}"));
    app.click(&frame, fonts, x, y, false);
}

#[test]
fn file_menu_renames_and_updates_links() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("B");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, "open:B.md");
    app.context_menu(&frame, &mut fonts, x, y);
    assert!(labels(&app).iter().any(|l| l == "Eliminar"));
    pick(&mut app, &mut fonts, &mut pm, "Renombrar…");
    assert!(app.menu.is_none());
    assert_eq!(app.prompt.as_ref().unwrap().query, "B");
    app.key(Key::parse("Backspace").unwrap());
    app.text_input("Bee");
    app.key(Key::parse("Enter").unwrap());
    assert!(d.path().join("Bee.md").exists() && !d.path().join("B.md").exists());
    assert_eq!(app.tab().unwrap().path, "Bee.md");
    assert!(fs::read_to_string(d.path().join("A.md")).unwrap().contains("[[Bee]]"));
    assert!(fs::read_to_string(d.path().join("dir/C.md")).unwrap().contains("[[Bee]]"));
}

#[test]
fn folder_menu_creates_folders_and_notes() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("new-folder-in:dir");
    app.text_input("Inner");
    app.key(Key::parse("Enter").unwrap());
    assert!(d.path().join("dir/Inner/Untitled.md").exists());
    assert_eq!(app.tab().unwrap().path, "dir/Inner/Untitled.md");
    app.run("new-note-in:dir/Inner");
    assert_eq!(app.tab().unwrap().path, "dir/Inner/Untitled 1.md");
    app.run("duplicate:A.md");
    assert_eq!(fs::read_to_string(d.path().join("A 1.md")).unwrap(), "# A\nsee [[B]]\n");
    app.run("delete:dir");
    assert!(!d.path().join("dir").exists());
    assert!(app.tabs.iter().all(|t| !t.path.starts_with("dir/")));
}

#[test]
fn editor_menu_cut_copy_paste_without_system_clipboard() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("B");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let editor = &frame.dom.nodes[frame.info.editor.unwrap()].rect;
    app.context_menu(&frame, &mut fonts, editor.x + 80.0, editor.y + 20.0);
    let shown = labels(&app);
    assert!(
        ["Cortar", "Copiar", "Pegar", "Negrita", "Seleccionar todo"]
            .iter()
            .all(|l| shown.iter().any(|s| s == l)),
        "{shown:?}"
    );
    // Escape closes the menu without running anything.
    app.key(Key::parse("Escape").unwrap());
    assert!(app.menu.is_none());
    // With nothing selected, cut takes the whole line.
    app.run("editor:cut");
    assert_eq!(app.tab().unwrap().editor.buf.text(), "");
    app.run("editor:paste");
    app.run("editor:paste");
    assert_eq!(app.tab().unwrap().editor.buf.text(), "b\nb\n");
    app.run("editor:select-all");
    app.run("editor:insert-wikilink");
    assert_eq!(app.tab().unwrap().editor.buf.text(), "[[b\nb\n]]");
}

#[test]
fn menu_keys_and_tab_menu() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    for n in ["A", "B", "dir/C"] {
        app.open_note(n);
    }
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, "tab:0");
    app.context_menu(&frame, &mut fonts, x, y);
    assert_eq!(labels(&app)[..3], ["Cerrar", "Cerrar las demás", "Cerrar las pestañas a la derecha"]);
    // The menu is drawn and stays inside the window.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let menu = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "menu")).unwrap();
    assert!(menu.rect.x + menu.rect.w <= 1100.0 && menu.rect.y + menu.rect.h <= 700.0);
    for k in ["ArrowDown", "ArrowDown", "ArrowDown", "Enter"] {
        app.key(Key::parse(k).unwrap());
    }
    assert!(app.menu.is_none());
    assert_eq!(app.tabs.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["A.md"]);
    // A click outside closes the menu and does nothing else.
    app.context_menu(&frame, &mut fonts, x, y);
    app.click(&frame, &mut fonts, 1090.0, 690.0, false);
    assert!(app.menu.is_none());
    assert_eq!(app.tabs.len(), 1);
}

#[test]
fn spellcheck_marks_prose_and_fixes_from_the_menu() {
    let config = tempfile::tempdir().unwrap();
    std::env::set_var("METAMIND_CONFIG_DIR", config.path());
    metamind_app::spell::load();
    let d = vault();
    // The pipeline default is off for this user's Spanish vault; this test explicitly turns
    // it on so the marked words and the right-click suggestions can be checked.
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"spellcheck": true}"#).unwrap();
    fs::write(d.path().join("S.md"), "A wrod in `codde` and [[Lnik]] here.\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("S");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let src = app.tab().unwrap().editor.buf.text().to_string();
    let bv = frame.info.blocks.iter().find(|b| b.display.contains("wrod")).unwrap();
    let marked: Vec<&str> =
        metamind_app::view::misspellings(&app, &frame.dom, bv).iter().map(|&(a, b)| &src[a..b]).collect();
    assert_eq!(marked, ["wrod"], "code and links are not checked");

    let node = &frame.dom.nodes[bv.node];
    let cb = frame.dom.content_rect(bv.node);
    let at = bv.display.find("wrod").unwrap() + 1;
    let r = fonts.range_rects(&bv.display, &node.el.spans, &node.style.text, Some(cb.w), at, at + 1)[0];
    app.context_menu(&frame, &mut fonts, cb.x + r.x + 1.0, cb.y + r.y + 5.0);
    let shown = labels(&app);
    assert!(
        shown.iter().any(|l| l == "word") && shown.iter().any(|l| l == "Añadir al diccionario"),
        "{shown:?}"
    );
    pick(&mut app, &mut fonts, &mut pm, "word");
    assert!(app.tab().unwrap().editor.buf.text().starts_with("A word in `codde`"));

    assert!(!metamind_app::spell::misspelled("Zorblax").is_empty());
    app.run("spell:add:Zorblax");
    assert!(metamind_app::spell::misspelled("Zorblax").is_empty());
    assert_eq!(fs::read_to_string(config.path().join("dictionary.txt")).unwrap(), "Zorblax\n");
}

/// The menu of a tab carries the pane commands, and the ones with nothing to act on say so
/// instead of looking ready and doing nothing. Picking a muted row explains it.
#[test]
fn the_tab_menu_offers_the_pane_commands_and_marks_the_ones_that_cannot_run() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, "tab:0");
    app.context_menu(&frame, &mut fonts, x, y);
    let items = &app.menu.as_ref().unwrap().items;
    let find = |label: &str| items.iter().find(|i| i.label == label).unwrap();
    assert!(find("Cerrar").disabled.is_none());
    assert_eq!(find("Cerrar panel").disabled, Some("Ya hay un solo panel"));
    assert!(find("Dividir a la derecha").disabled.is_none(), "one tab is enough to split");
    assert!(find("Dividir abajo").disabled.is_none());

    // The muted row leaves the pane alone and says why.
    let i = items.iter().position(|i| i.label == "Cerrar panel").unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, &format!("menu:{i}"));
    app.click(&frame, &mut fonts, x, y, false);
    assert!(app.menu.is_none());
    assert_eq!(app.notice.as_deref(), Some("Ya hay un solo panel"));
    assert_eq!(app.panes.len(), 1);

    // Splitting from the menu of the tab copies that tab into a pane of its own...
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, "tab:0");
    app.context_menu(&frame, &mut fonts, x, y);
    let i = app.menu.as_ref().unwrap().items.iter().position(|i| i.label == "Dividir a la derecha").unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, &format!("menu:{i}"));
    app.click(&frame, &mut fonts, x, y, false);
    assert_eq!(app.panes.len(), 2);

    // ...and with two panes, "Cerrar panel" closes the one the menu belongs to.
    let f = render(&mut app, &mut fonts, &mut pm, 1.0);
    let tab = f.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("tab:0")).unwrap().rect;
    app.context_menu(&f, &mut fonts, tab.x + 12.0, tab.y + tab.h / 2.0);
    let items = &app.menu.as_ref().unwrap().items;
    assert!(items.iter().find(|i| i.label == "Cerrar panel").unwrap().disabled.is_none());
    let i = items.iter().position(|i| i.label == "Cerrar panel").unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, &format!("menu:{i}"));
    app.click(&frame, &mut fonts, x, y, false);
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.tabs.len(), 1, "the tab of the pane that closed is gone with it");
}

/// Splitting from the menu of a tab that is not in front copies that tab, not the one that was
/// in front when the menu was opened.
#[test]
fn splitting_from_the_menu_of_a_tab_splits_that_tab() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    for n in ["A", "B"] {
        app.open_note(n);
    }
    assert_eq!(app.active, 1);
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let f = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&f, "tab:0");
    app.context_menu(&f, &mut fonts, x, y);
    let i = app.menu.as_ref().unwrap().items.iter().position(|i| i.label == "Dividir abajo").unwrap();
    let f = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&f, &format!("menu:{i}"));
    app.click(&f, &mut fonts, x, y, false);
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.pane, 1);
    let (tabs, active) = app.pane_tabs(1);
    assert_eq!(tabs[active].path, "A.md");
    assert!(matches!(app.layout, metamind_app::split::Layout::Split { row: false, .. }));
}
