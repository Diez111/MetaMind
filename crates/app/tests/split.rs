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

#[test]
fn a_tab_moved_to_a_window_renders_alone_there_and_focus_follows_the_window() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.open_note("B.md");
    app.run("workspace:move-to-new-window");
    let &[(id, pane)] = app.popouts.as_slice() else { panic!("{:?}", app.popouts) };
    assert_eq!(app.window_pane(id), Some(pane));
    assert_eq!(app.pane, pane);
    assert_eq!(shown(&app, pane), Some("B.md"));
    assert_eq!(shown(&app, 0), Some("A.md"));
    assert_eq!(app.layout, Layout::Leaf(0));

    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(800, 600).unwrap();
    let f = rustsidian_app::render_window(&mut app, &mut fonts, &mut pm, 1.0, Some(pane));
    assert_eq!(f.info.panes.iter().map(|p| p.0).collect::<Vec<_>>(), [pane]);
    let main = frame(&mut app, &mut fonts);
    assert_eq!(main.info.panes.iter().map(|p| p.0).collect::<Vec<_>>(), [0]);

    app.focus_window(None);
    assert_eq!(app.pane, 0);
    app.focus_window(Some(id));
    assert_eq!(app.pane, pane);
    app.run("workspace:split-vertical");
    assert_eq!(app.panes.len(), 2, "a pop-out window holds one pane");
}

#[test]
fn closing_a_window_docks_its_pane_and_closing_its_last_tab_closes_it() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.open_note("B.md");
    app.run("workspace:move-to-new-window");
    let (id, pane) = app.popouts[0];
    let saved = app.session();
    assert_eq!(app.layout, Layout::Leaf(0));
    assert!(saved["layout"]["kids"].is_array(), "the session keeps the window's pane: {saved}");

    app.dock_window(id);
    assert!(app.popouts.is_empty());
    let Layout::Split { row: true, kids } = &app.layout else { panic!("{:?}", app.layout) };
    assert_eq!(kids.iter().map(|k| k.0.clone()).collect::<Vec<_>>(), [Layout::Leaf(0), Layout::Leaf(pane)]);

    app.focus_pane(pane);
    app.run("workspace:move-to-new-window");
    let (id, pane) = app.popouts[0];
    assert_eq!(app.window_pane(id), Some(pane));
    app.run("workspace:close");
    let mut fonts = Fonts::new();
    frame(&mut app, &mut fonts);
    assert!(app.popouts.is_empty());
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.layout, Layout::Leaf(0));
    assert_eq!(app.pane, 0);
    assert_eq!(shown(&app, 0), Some("A.md"));
}

#[test]
fn stacked_columns_end_at_the_active_tab_and_the_rest_fold() {
    use rustsidian_app::view::stack_range;
    assert_eq!(stack_range(1, 0, 300.0), (0, 0));
    assert_eq!(stack_range(3, 2, 1000.0), (1, 2));
    assert_eq!(stack_range(3, 0, 1000.0), (0, 1));
    assert_eq!(stack_range(3, 1, 5000.0), (0, 2));
    assert_eq!(stack_range(5, 4, 200.0), (4, 4));
}

#[test]
fn stacked_tabs_show_side_by_side_and_a_click_on_a_spine_activates_it() {
    let d = vault();
    fs::write(d.path().join("C.md"), "gamma\n").unwrap();
    fs::write(d.path().join("D.md"), "delta\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    for n in ["A.md", "B.md", "C.md", "D.md"] {
        app.open_note(n);
    }
    app.left_open = false;
    app.right_open = false;
    app.run("workspace:toggle-stacked-tabs");
    assert!(app.session()["panes"][0]["stacked"].as_bool().unwrap());

    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.stack.iter().map(|e| e.1).collect::<Vec<_>>(), [0, 1, 2, 3]);
    let rect = |f: &Frame, i: usize| f.dom.nodes[f.info.stack[i].2].rect;
    let (spine, col, active) = (rect(&f, 0), rect(&f, 2), rect(&f, 3));
    assert!(spine.w < 40.0, "{spine:?}");
    assert!(col.w > 400.0 && col.x + col.w <= active.x + 1.0, "{col:?} {active:?}");

    app.click(&f, &mut fonts, spine.x + spine.w / 2.0, spine.y + 40.0, false);
    assert_eq!(app.active, 0);
    let f = frame(&mut app, &mut fonts);
    assert!(rect(&f, 0).w > 400.0, "the activated tab gets a column");

    app.run("workspace:toggle-stacked-tabs");
    let f = frame(&mut app, &mut fonts);
    assert!(f.info.stack.is_empty());
}
