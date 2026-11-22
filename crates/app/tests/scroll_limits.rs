//! The end of a long note is reachable: scrolling all the way down shows its last line, and the
//! scroll does not spring back to a shorter limit than the note has.

use metamind_app::{render, App, Key};
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
    // The list is virtualised with spacers: its content is every row it did not build, and the
    // wheel's limit is that whole list, not the rows that happened to be in view.
    let list = view.dom.by_id("files").expect("the file list");
    let n = &view.dom.nodes[list];
    let (content_h, scroll, pad) = (n.content_h, n.scroll_y, n.style.padding[2]);
    let max = view.dom.max_scroll(list);
    assert!((content_h - (400.0 * 26.0 + pad)).abs() <= 1.0, "400 rows of 26 px: {content_h}");
    assert!((scroll - max).abs() <= 1.0, "the list is at its end: {scroll} of {max}");
    let cb = view.dom.content_rect(list);
    assert!((cb.h + scroll - content_h).abs() <= 1.0, "the box is the limit: {cb:?} and {scroll}");
}

/// The name of a note line, and where it is.
fn line_at(view: &metamind_app::Frame, text: &str) -> Option<metamind_ui::Rect> {
    view.dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "cm-line"))
        .find(|n| n.el.text.as_deref() == Some(text))
        .map(|n| n.rect)
}

/// The box a note is shown in, and the scroller's own numbers: what it thinks the content is and
/// how far it can go. The thumb is drawn from these, so a note that lies here lies there too.
fn scroller(view: &metamind_app::Frame) -> (metamind_ui::Rect, f32, f32, f32) {
    let id = view.info.editor.expect("the note's view");
    let n = &view.dom.nodes[id];
    (n.rect, n.content_h, n.scroll_y, view.dom.max_scroll(id))
}

/// A reader's wheel, all the way down: the scroll is eased in over the frames, so each notch is
/// followed by the frames that apply it.
fn wheel_to_the_end(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap, notches: usize) {
    let mut view = frame(app, fonts, pm);
    let ed = view.info.editor.unwrap();
    let (x, y) = (view.dom.nodes[ed].rect.x + 40.0, view.dom.nodes[ed].rect.y + 40.0);
    for _ in 0..notches {
        app.wheel(&view, x, y, 3.0);
        view = frame(app, fonts, pm);
    }
    // The eased scroll needs its frames after the last notch.
    for _ in 0..120 {
        frame(app, fonts, pm);
    }
}

/// A 500-paragraph note is some nineteen thousand pixels tall and the scroll says so: the limit,
/// the scrollbar thumb and the end of the note all come from the whole document, not from the
/// blocks the view happened to lay out. The height of one paragraph is measured on the first two,
/// so the expected height is the note's own, not the number the scroller reports back.
#[test]
fn the_scroll_knows_how_tall_the_note_is() {
    let blocks = 500;
    let d = long_note(blocks);
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Larga.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    for _ in 0..4 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let first = frame(&mut app, &mut fonts, &mut pm);
    let step = line_at(&first, "linea 1 del documento").unwrap().y
        - line_at(&first, "linea 0 del documento").unwrap().y;
    assert!(step > 10.0, "one paragraph is {step} px tall");
    let (box_, content_h, _, max) = scroller(&first);
    let padding = first.dom.nodes[first.info.editor.unwrap()].style.padding[2];
    let want = blocks as f32 * step + padding;
    assert!(
        (content_h - want).abs() <= step,
        "the note is {want} px tall ({blocks} blocks of {step} px); the scroller says {content_h}"
    );
    let cb = first.dom.content_rect(first.info.editor.unwrap()).h;
    assert!((max + cb - content_h).abs() <= 1.0, "the limit is the note's: max {max}, content {content_h}");

    // The thumb is drawn from these same numbers: `track * box / content` long, and moved by
    // `scroll / max`. At the top it starts at the top of its rail, and it is as short as the
    // note is long compared with the window.
    let rail = box_.h;
    let (thumb, at) = thumb_geom(rail, cb, content_h, 0.0, max);
    assert!(at.abs() < 1.0, "the thumb starts at the top: {at}");
    assert!(thumb < rail / 4.0, "a fifteen-thousand-pixel note is a short thumb: {thumb} of {rail}");
    wheel_to_the_end(&mut app, &mut fonts, &mut pm, 200);
    let view = frame(&mut app, &mut fonts, &mut pm);
    let (box_, content_h2, y, max) = scroller(&view);
    assert!((content_h2 - content_h).abs() <= 1.0, "the height does not drift while scrolling");
    assert!((y - max).abs() <= 1.0, "the end is the limit: {y} of {max}");
    let cb = view.dom.content_rect(view.info.editor.unwrap()).h;
    assert!((y + cb - content_h2).abs() <= 2.0, "the view's bottom is the note's bottom: {y} + {cb}");
    let (thumb, at) = thumb_geom(rail, cb, content_h2, y, max);
    assert!((at + thumb - rail).abs() <= 2.0, "the thumb ends at the bottom of its rail: {at}+{thumb} of {rail}");
    let last = line_at(&view, &format!("linea {} del documento", blocks - 1)).expect("the last line");
    assert!(
        last.y >= box_.y - 1.0 && last.y + last.h <= box_.y + box_.h + 1.0,
        "the last line is inside the view: {last:?} in {box_:?}"
    );
}

/// Ctrl+End puts the caret on the last line of the note and the view follows it there: a jump
/// that parks the caret's block eighteen blocks below the top leaves the last line off screen on
/// a 605 px column, which is what a reader sees when the keyboard ignores the end of the note.
#[test]
fn the_keyboard_takes_the_last_line_into_view() {
    for (w, h) in [(900u32, 700u32), (360, 740)] {
        let blocks = 500;
        let d = long_note(blocks);
        let mut app = App::open(d.path()).unwrap();
        app.open_note("Larga.md");
        let mut fonts = Fonts::new();
        let mut pm = Pixmap::new(w, h).unwrap();
        for _ in 0..4 {
            frame(&mut app, &mut fonts, &mut pm);
        }
        app.key(Key::parse("Ctrl+End").unwrap());
        for _ in 0..6 {
            frame(&mut app, &mut fonts, &mut pm);
        }
        let view = frame(&mut app, &mut fonts, &mut pm);
        let (box_, content_h, y, max) = scroller(&view);
        assert!(y + box_.h >= content_h - 40.0, "{w}x{h}: the end of the note is on screen: {y} of {max}");
        let last = line_at(&view, &format!("linea {} del documento", blocks - 1)).expect("the last line");
        assert!(
            last.y >= box_.y - 1.0 && last.y + last.h <= box_.y + box_.h + 1.0,
            "{w}x{h}: the caret's line is in the view: {last:?} in {box_:?}"
        );
        assert!(
            last.y + last.h >= box_.y + box_.h - 48.0,
            "{w}x{h}: the caret's line is at the bottom of the view: {last:?} in {box_:?}"
        );
    }
}

/// Where the painter puts the thumb: `dom.rs` draws it `track * box / content` long, and moves it
/// by `scroll / max` over the room the track leaves. The limit and the bar are one calculation.
fn thumb_geom(rail: f32, cb: f32, content_h: f32, y: f32, max: f32) -> (f32, f32) {
    let thumb = (rail * cb / content_h).max(24.0);
    let at = (rail - thumb) * if max > 0.0 { (y / max).clamp(0.0, 1.0) } else { 0.0 };
    (thumb, at)
}

/// The settings panel is a scroller of its own: its content is taller than the box the modal
/// gives it, and the wheel takes it to its last row, not to a limit short of it.
#[test]
fn the_settings_panel_reaches_its_last_row_with_the_wheel() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("A.md"), "# A\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    for _ in 0..4 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let mut view = frame(&mut app, &mut fonts, &mut pm);
    let panel = view.dom.by_id("settings-content").expect("the settings content scrolls");
    let (x, y) = {
        let r = view.dom.nodes[panel].rect;
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    };
    for _ in 0..40 {
        app.wheel(&view, x, y, 3.0);
        view = frame(&mut app, &mut fonts, &mut pm);
    }
    for _ in 0..60 {
        view = frame(&mut app, &mut fonts, &mut pm);
    }
    let (box_, content_h, scroll, max) = {
        let n = &view.dom.nodes[panel];
        (n.rect, n.content_h, n.scroll_y, view.dom.max_scroll(panel))
    };
    assert!(content_h > box_.h + 1.0, "the panel has more rows than it shows: {content_h} in {}", box_.h);
    assert!((scroll - max).abs() <= 1.0, "the wheel reaches the end of the panel: {scroll} of {max}");
    // Nothing of the panel is left below the box it scrolls in.
    let last = view.dom.nodes[panel].children.last().copied();
    if let Some(row) = last {
        let r = view.dom.nodes[row].rect;
        assert!(r.y + r.h <= box_.y + box_.h + 1.0, "the last row is inside the panel: {r:?} in {box_:?}");
    }
}
