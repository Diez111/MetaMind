//! The end of a long note is reachable: scrolling all the way down shows its last line, and the
//! scroll does not spring back to a shorter limit than the note has.

use metamind_app::{render, App};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn long_note(lines: usize) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let mut text = String::new();
    for i in 0..lines {
        text.push_str(&format!("linea {i} del documento\n\n"));
    }
    fs::write(d.path().join("Larga.md"), text).unwrap();
    d
}

fn frame(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap) -> metamind_app::Frame {
    render(app, fonts, pm, 1.0)
}

#[test]
fn the_last_line_of_a_long_note_can_be_reached() {
    let d = long_note(400);
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Larga.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    for _ in 0..4 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    // Scroll far past the end, several times: each frame eases part of the way, and the limit has
    // to come from the note, not from what the previous frame happened to lay out.
    for _ in 0..40 {
        app.scroll_editor(2000.0);
        frame(&mut app, &mut fonts, &mut pm);
    }
    let here = app.tab().unwrap().top_block;
    let view = frame(&mut app, &mut fonts, &mut pm);
    let last = view
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "cm-line") && n.el.text.as_deref() == Some("linea 399 del documento"))
        .map(|n| n.rect)
        .next();
    let Some(last) = last else {
        panic!("the last line is not laid out at all after scrolling (top_block {here})");
    };
    let box_ = view
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "view-content"))
        .map(|n| n.rect)
        .expect("the note's view");
    assert!(
        last.y >= box_.y - 1.0 && last.y + last.h <= box_.y + box_.h + 1.0,
        "the last line is inside the view: line {last:?} view {box_:?}"
    );
}

#[test]
fn the_last_file_of_a_big_vault_can_be_reached_in_the_explorer() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..400 {
        fs::write(d.path().join(format!("nota-{i:03}.md")), "# nota\n").unwrap();
    }
    let mut app = App::open(d.path()).unwrap();
    app.open_note("nota-000.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    app.run("file-explorer:open");
    for _ in 0..4 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let before = frame(&mut app, &mut fonts, &mut pm);
    let n0 = before.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == "nav-file-title-content")).count();
    eprintln!("filas antes de scrollear: {n0}");
    // The wheel over the explorer, all the way down: the same path a reader's wheel takes.
    for _ in 0..80 {
        let f = frame(&mut app, &mut fonts, &mut pm);
        app.wheel(&f, 100.0, 400.0, 12.0);
    }
    let view = frame(&mut app, &mut fonts, &mut pm);
    let names: Vec<&str> = view
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "nav-file-title-content"))
        .filter_map(|n| n.el.text.as_deref())
        .collect();
    assert!(
        names.contains(&"nota-399"),
        "the last file is drawn when the list is scrolled to its end ({} rows drawn, last: {:?})",
        names.len(),
        names.last()
    );
}
