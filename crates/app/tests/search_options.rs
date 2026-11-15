use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

fn has(frame: &Frame, action: &str) -> bool {
    frame.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some(action))
}

fn click(app: &mut App, frame: &Frame, fonts: &mut Fonts, action: &str) {
    let r = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some(action)).unwrap().rect;
    app.click(frame, fonts, r.x + r.w / 2.0, r.y + r.h / 2.0, false);
}

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("A.md"), "a #topic\n").unwrap();
    d
}

#[test]
fn search_pane_offers_operators_while_empty() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1200, 900).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(!has(&frame, "search:insert:tag:"));

    app.run("global-search:open");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    click(&mut app, &frame, &mut fonts, "search:insert:tag:");
    assert_eq!(app.search_query, "tag:");
    app.text_input("#topic");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(!has(&frame, "search:insert:path:"), "options hide once the query has text");
}

#[test]
fn graph_filter_offers_operators_while_empty() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("graph:open");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1200, 1000).unwrap();
    render(&mut app, &mut fonts, &mut pm, 1.0);
    app.run("graph:edit:search");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    click(&mut app, &frame, &mut fonts, "graph:insert:path:");
    let g = app.graph_view.as_ref().unwrap();
    assert_eq!(g.settings.search, "path:");
    assert!(g.panel.edit.is_some(), "picking an option keeps the field in edit");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(!has(&frame, "graph:insert:file:"));
}
