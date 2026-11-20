//! A `dataview` fence is drawn as its table in reading view, and its links open the note.
use rustsidian_app::shell::ViewMode;
use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

const INDICE: &str = "# Índice\n\nTodo lo del año.\n\n```dataview\nTABLE tipo AS \"Tipo\", unidad AS \"Unidad\"\nFROM \"Facultad\"\nWHERE tipo != \"materia\"\nSORT unidad ASC\n```\n";

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    w("Indice.md", INDICE);
    w("Facultad/A.md", "---\ntipo: unidad\nunidad: 2\n---\n# A\n");
    w("Facultad/B.md", "---\ntipo: unidad\nunidad: 1\n---\n# B\n");
    w("Facultad/C.md", "---\ntipo: materia\n---\n# C\n");
    w("Roto.md", "# Roto\n\n```dataview\nTABLE\n```\n");
    d
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 700).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

/// The text of every element with `class`.
fn texts(f: &Frame, class: &str) -> Vec<String> {
    f.dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == class))
        .filter_map(|n| n.el.text.clone())
        .collect()
}

fn find_action(f: &Frame, action: &str) -> Option<(f32, f32)> {
    f.dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "table-cell"))
        .find(|n| n.el.action.as_deref() == Some(action))
        .map(|n| (n.rect.x + 2.0, n.rect.y + 2.0))
}

#[test]
fn the_block_shows_its_table_and_a_cell_opens_the_note() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Indice.md");
    app.tab_mut().unwrap().mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(texts(&f, "table-cell").len(), 9, "three headers and two rows");
    assert_eq!(
        texts(&f, "table-cell"),
        ["File", "Tipo", "Unidad", "B", "unidad", "1", "A", "unidad", "2"],
        "the header and the two units in SORT order"
    );
    assert!(!texts(&f, "cm-codeblock").iter().any(|t| t.contains("TABLE")), "the source is replaced");

    let (x, y) = find_action(&f, "open:Facultad/B.md").expect("a link to B.md");
    app.click(&f, &mut fonts, x, y, false);
    assert_eq!(app.tab().map(|t| t.path.as_str()), Some("Facultad/B.md"));
}

#[test]
fn a_note_that_changes_refreshes_the_table() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Indice.md");
    app.tab_mut().unwrap().mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(texts(&f, "table-cell")[3..6], ["B", "unidad", "1"], "rows sort by unidad");

    fs::write(d.path().join("Facultad/A.md"), "---\ntipo: unidad\nunidad: 0\n---\n# A\n").unwrap();
    app.files_changed(&["Facultad/A.md".to_string()]);
    let f = frame(&mut app, &mut fonts);
    assert_eq!(texts(&f, "table-cell")[3..6], ["A", "unidad", "0"], "the table is recomputed");
}

#[test]
fn a_query_that_does_not_parse_keeps_the_source_and_says_why() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Roto.md");
    app.tab_mut().unwrap().mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert!(texts(&f, "dataview-table").is_empty());
    let notice = texts(&f, "rich-notice");
    assert!(notice.iter().any(|n| n.starts_with("Dataview:") && n.contains("field name")), "{notice:?}");
    assert!(texts(&f, "cm-codeblock").iter().any(|t| t.contains("TABLE")), "the source stays");
}
