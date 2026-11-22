use metamind_app::{render, App, Frame, Key};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn frame(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap) -> Frame {
    render(app, fonts, pm, 1.0)
}

#[test]
fn probe_frame_cost() {
    for n in [400usize, 2000] {
        let d = tempfile::tempdir().unwrap();
        let text: String = (0..n).map(|i| format!("linea {i} del documento\n\n")).collect();
        fs::write(d.path().join("Larga.md"), text).unwrap();
        let mut app = App::open(d.path()).unwrap();
        app.open_note("Larga.md");
        let mut fonts = Fonts::new();
        let mut pm = Pixmap::new(900, 700).unwrap();
        let mut times = Vec::new();
        for f in 0..40 {
            let t = std::time::Instant::now();
            frame(&mut app, &mut fonts, &mut pm);
            times.push(t.elapsed().as_micros() as u64);
            if f % 5 == 0 {
                let ed = app.tab().unwrap();
                eprintln!(
                    "{n} f{f:02}: frame {:6} us  extent total {:8.1} blocks {}",
                    times[f],
                    ed.extent.total(),
                    ed.editor.buf.doc().blocks.len()
                );
            }
        }
        times.sort();
        eprintln!("{n} bloques: mediana {} us, max {} us, nominal {}", times[20], times[39], app.last_frame_us);
    }
}

#[test]
fn probe_keyboard_end() {
    let d = tempfile::tempdir().unwrap();
    let text: String = (0..400).map(|i| format!("linea {i} del documento\n\n")).collect();
    fs::write(d.path().join("Larga.md"), text).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Larga.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    for _ in 0..12 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    app.key(Key::parse("Ctrl+End").unwrap());
    for _ in 0..3 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let f = frame(&mut app, &mut fonts, &mut pm);
    let ed = f.info.editor.unwrap();
    let n = &f.dom.nodes[ed];
    eprintln!(
        "ctrl+end: top={} px={:.1} scroll={:.1} content_h={:.1} max={:.1} total={:.1}",
        app.tab().unwrap().top_block,
        app.tab().unwrap().scroll_px,
        n.scroll_y,
        n.content_h,
        f.dom.max_scroll(ed),
        app.tab().unwrap().extent.total()
    );
    let lines: Vec<(String, f32)> = f
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "cm-line"))
        .filter_map(|n| n.el.text.clone().map(|t| (t, n.rect.y)))
        .collect();
    eprintln!("ultimas: {:?}", lines.iter().rev().take(2).collect::<Vec<_>>());
    eprintln!("view box: {:?}", n.rect);
}

#[test]
fn probe_resize_and_split() {
    let d = tempfile::tempdir().unwrap();
    let text: String = (0..400).map(|i| format!("linea {i} del documento\n\n")).collect();
    fs::write(d.path().join("Larga.md"), text).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Larga.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1366, 768).unwrap();
    for _ in 0..14 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    // Scroll to the middle, then change the window width: the visible content must not move.
    for _ in 0..10 {
        app.scroll_editor(200.0);
        frame(&mut app, &mut fonts, &mut pm);
    }
    for _ in 0..60 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let mid = frame(&mut app, &mut fonts, &mut pm);
    let first_line = |f: &Frame| -> Option<String> {
        let ed = f.info.editor?;
        let r = f.dom.nodes[ed].rect;
        f.dom
            .nodes
            .iter()
            .filter(|n| n.el.classes.iter().any(|c| c == "cm-line"))
            .filter(|n| n.rect.y >= r.y - 1.0 && n.rect.y < r.y + r.h)
            .min_by(|a, b| a.rect.y.partial_cmp(&b.rect.y).unwrap())
            .and_then(|n| n.el.text.clone())
    };
    eprintln!("1366 medio: primera visible {:?}", first_line(&mid));
    let mut pm2 = Pixmap::new(1100, 700).unwrap();
    for _ in 0..3 {
        frame(&mut app, &mut fonts, &mut pm2);
    }
    let after = frame(&mut app, &mut fonts, &mut pm2);
    eprintln!("1100 tras resize: primera visible {:?}", first_line(&after));

    // A split: each pane has its own column width and its own extent.
    app.run("workspace:split-vertical");
    for _ in 0..14 {
        frame(&mut app, &mut fonts, &mut pm2);
    }
    let f = frame(&mut app, &mut fonts, &mut pm2);
    let ed = f.info.editor.unwrap();
    let n = &f.dom.nodes[ed];
    eprintln!(
        "split: box {:?} content_h {:.1} max {:.1} total {:.1} widths {:?}",
        n.rect,
        n.content_h,
        f.dom.max_scroll(ed),
        app.tab().unwrap().extent.total(),
        app.note_widths
    );
}
