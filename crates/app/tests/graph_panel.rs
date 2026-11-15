use rustsidian_app::shell::Key;
use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, t) in [("A.md", "see [[B]] #topic\n"), ("B.md", "b\n"), ("C.md", "alone\n")] {
        fs::write(d.path().join(p), t).unwrap();
    }
    d
}

fn rect(frame: &Frame, action: &str) -> rustsidian_ui::Rect {
    frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some(action)).unwrap().rect
}

fn graph_json(d: &tempfile::TempDir) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(d.path().join(".obsidian/graph.json")).unwrap()).unwrap()
}

#[test]
fn slider_drag_sets_and_saves_value() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("graph:open");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1200, 1000).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let r = rect(&frame, "graph:slider:node-size");
    // Pressing at the right end of the track sets the maximum.
    app.click(&frame, &mut fonts, r.x + r.w - 1.0, r.y + r.h / 2.0, false);
    assert_eq!(app.graph_view.as_ref().unwrap().settings.node_size, 5.0);
    // Dragging to the left end sets the minimum; release saves it.
    app.drag(&frame, &mut fonts, r.x - 50.0, r.y);
    assert_eq!(app.graph_view.as_ref().unwrap().settings.node_size, 0.1);
    app.release();
    assert_eq!(graph_json(&d)["nodeSizeMultiplier"], 0.1);
    // The drag did not pan the graph.
    assert!(app.graph_view.as_ref().unwrap().auto_fit);
}

#[test]
fn search_groups_sections_and_reset() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("graph:open");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1200, 1000).unwrap();
    render(&mut app, &mut fonts, &mut pm, 1.0);
    let all = app.graph_view.as_ref().unwrap().graph.nodes.len();

    app.run("graph:edit:search");
    app.text_input("A");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    let g = app.graph_view.as_ref().unwrap();
    assert_eq!(g.settings.search, "A");
    assert!(g.graph.nodes.len() < all);
    app.key(Key::parse("Backspace").unwrap());
    app.key(Key::parse("Enter").unwrap());
    assert!(app.graph_view.as_ref().unwrap().panel.edit.is_none());
    assert_eq!(graph_json(&d)["search"], "");

    app.run("graph:group-new");
    app.text_input("tag:#topic");
    app.key(Key::parse("Enter").unwrap());
    app.run("graph:group-color:0");
    let groups = &graph_json(&d)["colorGroups"];
    assert_eq!(groups[0]["query"], "tag:#topic");
    assert_eq!(groups[0]["color"]["rgb"], 0xe9973f);

    app.run("graph:section:forces");
    app.run("graph:toggle-panel");
    let j = graph_json(&d);
    assert_eq!((j["collapse-forces"].as_bool(), j["close"].as_bool()), (Some(true), Some(true)));
    // Closed, only the gear button is drawn.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.dom.nodes.iter().all(|n| n.el.action.as_deref() != Some("graph:slider:repel")));
    rect(&frame, "graph:toggle-panel");

    app.run("graph:toggle-orphans");
    app.run("graph:reset-settings");
    let g = app.graph_view.as_ref().unwrap();
    assert!(g.settings.filters.show_orphans);
    assert_eq!(g.settings.color_groups.len(), 1);
}
