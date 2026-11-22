use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn y_of(frame: &Frame, text: &str) -> Option<f32> {
    let b = frame.info.blocks.iter().find(|b| b.display == text)?;
    Some(frame.dom.nodes[b.node].rect.y)
}

#[test]
fn editor_scrolls_by_pixels_across_blocks() {
    let d = tempfile::tempdir().unwrap();
    let text: String = (0..300).map(|i| format!("Paragraph {i}\n\n")).collect();
    fs::write(d.path().join("N.md"), text).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("N.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let mut frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let ed = frame.dom.nodes[frame.info.editor.unwrap()].rect;
    let (x, y) = (ed.x + ed.w / 2.0, ed.y + ed.h / 2.0);
    let start = y_of(&frame, "Paragraph 5").unwrap();
    let per = (y_of(&frame, "Paragraph 25").unwrap() - start) / 20.0;

    // A touchpad's small deltas apply at once, a pixel at a time rather than a block.
    app.wheel(&frame, x, y, 0.25);
    frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(y_of(&frame, "Paragraph 5").unwrap(), start - 10.0);

    // Far enough that the view is laid out from a later block, the text still moves exactly.
    for _ in 0..100 {
        app.wheel(&frame, x, y, 0.5);
        frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    }
    assert!(app.tab().unwrap().top_block > 5);
    let at = y_of(&frame, "Paragraph 100").unwrap();
    let expect = start + per * 95.0 - 10.0 - 2000.0;
    assert!((at - expect).abs() < 0.5, "Paragraph 100 at {at}, expected {expect}");

    // A wheel notch is eased in over several frames.
    app.wheel(&frame, x, y, 3.0);
    frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let moved = at - y_of(&frame, "Paragraph 100").unwrap();
    assert!(moved > 0.0 && moved < 120.0, "{moved}");
    assert!(app.scrolling());
    for _ in 0..60 {
        frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    }
    assert!(!app.scrolling());
    assert!((at - 120.0 - y_of(&frame, "Paragraph 100").unwrap()).abs() < 0.5);

    // Scrolling up stops at the top of the note.
    app.wheel(&frame, x, y, -300.0);
    for _ in 0..60 {
        frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    }
    assert!(!app.scrolling());
    assert_eq!(y_of(&frame, "Paragraph 5").unwrap(), start);
}
