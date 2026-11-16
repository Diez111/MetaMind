use rustsidian_app::bases::Val;
use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

const BASE: &str = r#"filters:
  and:
    - file.hasTag("book")
formulas:
  total: "price * qty"
views:
  - type: table
    name: Books
    order:
      - file.name
      - status
      - formula.total
    sort:
      - property: formula.total
        direction: DESC
  - type: cards
    name: Gallery
    order:
      - file.name
      - status
"#;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("A.md"), "---\ntags: [book]\nstatus: reading\nprice: 10\nqty: 1\n---\nalpha\n")
        .unwrap();
    fs::write(d.path().join("B.md"), "---\ntags: [book]\nstatus: done\nprice: 5\nqty: 4\n---\nbeta\n")
        .unwrap();
    fs::write(d.path().join("C.md"), "---\nstatus: done\n---\nnot a book\n").unwrap();
    fs::write(d.path().join("Books.base"), BASE).unwrap();
    d
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 700).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn rows(app: &App) -> Vec<(String, Vec<Val>)> {
    let (_, items) = app.base_view.as_ref().unwrap().items.as_ref().unwrap();
    items.iter().map(|i| (i.path.clone(), i.cells.clone())).collect()
}

fn find_action(f: &Frame, action: &str) -> Option<(f32, f32)> {
    f.dom
        .nodes
        .iter()
        .filter(|n| !n.el.classes.iter().any(|c| c == "nav-file-title"))
        .find(|n| n.el.action.as_deref() == Some(action))
        .map(|n| (n.rect.x + 2.0, n.rect.y + 2.0))
}

#[test]
fn a_base_lists_matching_notes_and_a_click_opens_one() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Books.base");
    assert!(app.base_active());
    assert!(app.tab().is_none());
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(
        rows(&app),
        [
            ("B.md".to_string(), vec![Val::Str("B".into()), Val::Str("done".into()), Val::Num(20.0)]),
            ("A.md".to_string(), vec![Val::Str("A".into()), Val::Str("reading".into()), Val::Num(10.0)]),
        ]
    );
    let (x, y) = find_action(&f, "open:B.md").expect("a link to B.md");
    app.click(&f, &mut fonts, x, y, false);
    assert!(!app.base_active());
    assert_eq!(app.tab().map(|t| t.path.as_str()), Some("B.md"));

    app.run("base:focus");
    assert!(app.base_active());
    app.run("workspace:close");
    assert!(app.base_view.is_none());
}

#[test]
fn switching_views_and_editing_a_note_update_the_rows() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Books.base");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let (x, y) = find_action(&f, "base:view:1").expect("a tab for the second view");
    app.click(&f, &mut fonts, x, y, false);
    let f = frame(&mut app, &mut fonts);
    assert_eq!(rows(&app).len(), 2);
    assert!(f.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "bases-card")));

    // Tagging C as a book in another program adds it.
    app.open_note("C.md");
    let text = app.tab().unwrap().editor.buf.text().replace("status: done", "status: done\ntags: [book]");
    fs::write(d.path().join("C.md"), &text).unwrap();
    app.files_changed(&["C.md".to_string()]);
    app.run("base:focus");
    frame(&mut app, &mut fonts);
    let paths: Vec<String> = rows(&app).into_iter().map(|r| r.0).collect();
    // The gallery view has no sort, so rows are in path order.
    assert_eq!(paths, ["A.md", "B.md", "C.md"]);
}

#[test]
fn a_base_with_bad_syntax_shows_the_error() {
    let d = vault();
    fs::write(d.path().join("Bad.base"), "filters: 'price >'\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Bad.base");
    let f = frame(&mut app, &mut Fonts::new());
    assert!(f.dom.nodes.iter().any(|n| n.el.text.as_deref().is_some_and(|t| t.contains("cannot be read"))));
}

#[test]
fn clicking_a_property_cell_edits_the_note() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.open_note("Books.base");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert!(find_action(&f, "base:edit:A.md|formula.total").is_none());
    let (x, y) = find_action(&f, "base:edit:A.md|status").expect("an editable status cell");
    app.click(&f, &mut fonts, x, y, false);
    assert_eq!(app.prompt.as_ref().map(|p| p.query.as_str()), Some("reading"));
    app.prompt.as_mut().unwrap().query.clear();
    app.text_input("paused");
    app.key(rustsidian_app::Key::new("Enter"));
    let text = fs::read_to_string(d.path().join("A.md")).unwrap();
    assert!(text.contains("status: paused\n"), "{text}");
    // The open tab shows the change, and so do the rows.
    assert!(app.all_tabs().any(|t| t.path == "A.md" && t.editor.buf.text().contains("status: paused")));
    frame(&mut app, &mut fonts);
    assert!(rows(&app).iter().any(|(p, c)| p == "A.md" && c[1] == Val::Str("paused".into())));
}

#[test]
fn a_kanban_view_has_a_column_per_group_and_moves_cards() {
    let d = vault();
    fs::write(
        d.path().join("Board.base"),
        "filters: file.hasTag(\"book\")\nviews:\n  - type: kanban\n    name: Board\n    groupBy: status\n    order: [file.name, price]\n",
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Board.base");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let has = |f: &Frame, class: &str| -> Vec<String> {
        f.dom
            .nodes
            .iter()
            .filter(|n| n.el.classes.iter().any(|c| c == class))
            .map(|n| n.el.text.clone().unwrap_or_default())
            .collect()
    };
    assert_eq!(has(&f, "bases-kanban-heading"), ["done (1)", "reading (1)"]);
    let card = f.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("open:A.md")).unwrap();
    let column = &f.dom.nodes[card.parent.unwrap()];
    let board = &f.dom.nodes[column.parent.unwrap()];
    assert!(card.rect.w > column.rect.w * 0.8, "cards fill their column");
    assert!(column.rect.w > board.rect.w * 0.4, "two columns share the board");

    let (x, y) = find_action(&f, "base:move:A.md|status").expect("a move control on A");
    app.click(&f, &mut fonts, x, y, false);
    let labels: Vec<String> = app.prompt.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(
        labels,
        [
            "Set \u{201c}status\u{201d} to \u{201c}done\u{201d}",
            "Set \u{201c}status\u{201d} to \u{201c}reading\u{201d}",
            "Remove \u{201c}status\u{201d} from A",
        ]
    );
    app.key(rustsidian_app::Key::new("Enter"));
    let f = frame(&mut app, &mut fonts);
    assert_eq!(has(&f, "bases-kanban-heading"), ["done (2)"]);
    assert!(fs::read_to_string(d.path().join("A.md")).unwrap().contains("status: done\n"));
}
