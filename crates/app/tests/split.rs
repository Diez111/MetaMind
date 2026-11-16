use rustsidian_app::split::Layout;
use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("A.md"), "alpha\n").unwrap();
    fs::write(d.path().join("B.md"), "beta\n").unwrap();
    d
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 700).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn shown(app: &App, pane: usize) -> Option<&str> {
    let (tabs, active) = app.pane_tabs(pane);
    tabs.get(active).map(|t| t.path.as_str())
}

#[test]
fn split_right_lays_panes_side_by_side_and_a_click_focuses() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.run("workspace:split-vertical");
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.pane, 1);
    assert_eq!(shown(&app, 0), Some("A.md"));
    assert_eq!(shown(&app, 1), Some("A.md"));

    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let rect = |p: usize| f.dom.nodes[f.info.panes.iter().find(|e| e.0 == p).unwrap().1].rect;
    let (left, right) = (rect(0), rect(1));
    assert!(left.x + left.w <= right.x, "{left:?} {right:?}");
    assert!((left.w - right.w).abs() < 2.0, "{left:?} {right:?}");

    app.click(&f, &mut fonts, left.x + left.w / 2.0, left.y + left.h / 2.0, false);
    assert_eq!(app.pane, 0);
}

#[test]
fn edits_mirror_into_the_other_pane_and_save_once() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.run("workspace:split-horizontal");
    app.text_input("x");
    let mut fonts = Fonts::new();
    frame(&mut app, &mut fonts);
    let other = &app.panes[0].tabs[0];
    assert_eq!(other.editor.buf.text(), app.tabs[0].editor.buf.text());
    assert!(other.editor.buf.is_modified());

    app.save_all();
    assert_eq!(app.notice, None);
    assert_eq!(fs::read_to_string(d.path().join("A.md")).unwrap(), app.tabs[0].editor.buf.text());
    assert!(!app.panes[0].tabs[0].editor.buf.is_modified());
    assert!(!app.tabs[0].editor.buf.is_modified());
}

#[test]
fn closing_the_last_tab_of_a_pane_closes_the_pane() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.run("workspace:split-vertical");
    app.run("workspace:close");
    frame(&mut app, &mut Fonts::new());
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.layout, Layout::Leaf(0));
    assert_eq!(app.tab().map(|t| t.path.as_str()), Some("A.md"));
}

#[test]
fn a_linked_pane_follows_the_focused_note() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.run("workspace:split-vertical");
    app.run("link-pane:0");
    app.open_note("B.md");
    frame(&mut app, &mut Fonts::new());
    assert_eq!(shown(&app, 0), Some("B.md"));
    // The linked pane's note was replaced, not opened beside it.
    assert_eq!(app.pane_tabs(0).0.len(), 1);
    assert_eq!(app.pane, 1);

    app.run("workspace:unlink-pane");
    app.open_note("A.md");
    frame(&mut app, &mut Fonts::new());
    assert_eq!(shown(&app, 0), Some("B.md"));
}

#[test]
fn panes_are_restored_with_the_session() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.run("workspace:split-vertical");
    app.open_note("B.md");
    app.focus_pane(0);
    app.save_all();

    let mut app = App::open(d.path()).unwrap();
    app.restore_session();
    assert_eq!(app.panes.len(), 2);
    assert!(matches!(&app.layout, Layout::Split { row: true, kids } if kids.len() == 2));
    assert_eq!(app.pane, 0);
    assert_eq!(shown(&app, 0), Some("A.md"));
    let right: Vec<&str> = app.pane_tabs(1).0.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(right, ["A.md", "B.md"]);
    assert_eq!(shown(&app, 1), Some("B.md"));
}
