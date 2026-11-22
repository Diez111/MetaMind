//! Nothing is cut and nothing leaves the window.
//!
//! Every view is rendered at the two extremes of the sizes the app ships at (1280x720 and
//! 2560x1440) and each text node of the frame is checked against the box that really clips it:
//!
//! - text measured at its natural width that does not fit its own content box, with no ellipsis,
//!   is text cut in the middle of a word;
//! - a text or a control whose rectangle leaves the nearest clipping ancestor, or the window,
//!   is a piece of the interface hanging out of its box;
//! - a box that clips its content and cannot scroll hides the rest of it with no way to reach
//!   it, which is the other half of the same defect.
//!
//! Content a scroller pushed out of sight is not a defect: the check asks whether a node that is
//! visible is cut, not whether a long note is longer than its pane.
use metamind_app::shell::ViewMode;
use metamind_app::{render, App, Frame};
use metamind_ui::{Dom, Fonts, Pixmap};
use std::fs;

/// A vault with the shapes the panes have to hold: a note with backlinks, a note without, a long
/// line that has to be shortened in a 250 px sidebar, an empty folder, a nested folder, tags and
/// unresolved links.
fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("dir")).unwrap();
    fs::create_dir(d.path().join("dir/nested")).unwrap();
    fs::create_dir(d.path().join("vacia")).unwrap();
    for (p, t) in [
        (
            "A.md",
            "---\ntags: [topic, Area/sub]\n---\n# Uno\n\nsee [[B]] and [[Nowhere]] and \
             [[Otro sitio que tampoco existe|otra cosa]]\n\nLinks: [[B]], [[dir/C]], [[B#Dos]], \
             [[Nowhere]], [[dir/nested/D]]\n",
        ),
        ("B.md", "---\ntags: [topic]\n---\n# Dos\n\nback to [[A]]\n"),
        ("dir/C.md", "alone, no links at all\n"),
        ("dir/nested/D.md", "deep\n"),
        ("Una nota con un nombre larguísimo para la barra de pestañas.md", "# Larga\n\ntexto\n"),
        ("Sin enlaces.md", "nada que enlazar\n"),
    ] {
        fs::write(d.path().join(p), t).unwrap();
    }
    d
}

fn open(d: &tempfile::TempDir) -> App {
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app
}

/// One frame of the app at `w` x `h`, as the window paints it. Two passes, like the window: the
/// first one lays the tree out, the second one is the frame a reader sees.
fn frame(app: &mut App, fonts: &mut Fonts, w: u32, h: u32) -> Frame {
    let mut pm = Pixmap::new(w, h).unwrap();
    render(app, fonts, &mut pm, 1.0);
    render(app, fonts, &mut pm, 1.0)
}

/// The box the text of a node is painted in: its border box minus border and padding.
fn content_box(n: &metamind_ui::dom::Node) -> metamind_ui::Rect {
    let s = &n.style;
    let h = s.padding[1] + s.padding[3] + s.border[1] + s.border[3];
    let v = s.padding[0] + s.padding[2] + s.border[0] + s.border[2];
    metamind_ui::Rect::new(
        n.rect.x + s.border[3] + s.padding[3],
        n.rect.y + s.border[0] + s.padding[0],
        (n.rect.w - h).max(0.0),
        (n.rect.h - v).max(0.0),
    )
}

/// The two ways a box can cut its content.
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// Where the box that clips `id` ends on `axis`: the intersection of its own rectangle with every
/// ancestor that clips. On the vertical axis a scroller is not a cut — its thumb is how the rest
/// of the content is reached — so only `overflow: hidden` ancestors count there.
fn clip_span(nodes: &[metamind_ui::dom::Node], id: usize, axis: Axis) -> Option<(f32, f32)> {
    let mut box_ = nodes[id].rect;
    let mut p = nodes[id].parent;
    while let Some(i) = p {
        let n = &nodes[i];
        if n.style.clip && !(axis == Axis::Vertical && n.style.scroll) {
            let r = n.rect;
            let (x0, y0) = (box_.x.max(r.x), box_.y.max(r.y));
            let (x1, y1) = ((box_.x + box_.w).min(r.x + r.w), (box_.y + box_.h).min(r.y + r.h));
            box_ = metamind_ui::Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
        }
        p = n.parent;
    }
    match axis {
        Axis::Horizontal => (box_.w > 0.0).then_some((box_.x, box_.x + box_.w)),
        Axis::Vertical => (box_.h > 0.0).then_some((box_.y, box_.y + box_.h)),
    }
}

/// A node inside a scroller that is only half inside its viewport is mid-scroll, not cut: the
/// reader can bring the rest in. The settings panel's last row and a long list both look like
/// this on a frame that has not been scrolled yet.
fn mid_scroll(nodes: &[metamind_ui::dom::Node], id: usize) -> bool {
    let r = nodes[id].rect;
    let mut p = nodes[id].parent;
    while let Some(i) = p {
        let n = &nodes[i];
        if n.style.scroll {
            let s = n.rect;
            if r.y < s.y - 1.0 || r.y + r.h > s.y + s.h + 1.0 {
                return true;
            }
        }
        p = n.parent;
    }
    false
}

fn hidden(nodes: &[metamind_ui::dom::Node], id: usize) -> bool {
    let mut p = Some(id);
    while let Some(i) = p {
        let n = &nodes[i];
        if n.style.display_none || n.style.opacity <= 0.01 {
            return true;
        }
        p = n.parent;
    }
    false
}

/// The name of a node in a report: its classes and the start of its text, which is what the CSS
/// and the reader share.
fn name(dom: &Dom, id: usize) -> String {
    let n = &dom.nodes[id];
    let classes = n.el.classes.join(".");
    match n.el.text.as_ref() {
        Some(t) => format!(".{classes} \"{}\"", t.chars().take(56).collect::<String>()),
        None => format!(".{classes}"),
    }
}

/// Everything in `f` that is cut or hangs out of its box, as one line per finding.
fn cut_nodes(f: &Frame, fonts: &mut Fonts, w: f32, h: f32) -> Vec<String> {
    let nodes = &f.dom.nodes;
    let mut out = Vec::new();
    // A pixel of slack: layout rounds, and a fraction is not a visual defect.
    const EPS: f32 = 1.0;
    for (id, n) in nodes.iter().enumerate() {
        if hidden(nodes, id) {
            continue;
        }
        // Content a box hides with no way to reach it. The bottom padding is part of `content_h`
        // and not of the content box, so it is the room a scroller leaves at the end, not overflow.
        if n.style.clip && !n.style.scroll {
            let cb = content_box(n);
            if n.content_h > cb.h + n.style.padding[2] + EPS {
                out.push(format!(
                    "{} hides {:.0}px of content in {:.0}px and cannot scroll",
                    name(&f.dom, id),
                    n.content_h,
                    cb.h
                ));
                continue;
            }
        }
        let text = n.shown.as_ref().or(n.el.text.as_ref());
        let is_box = n.style.background.a > 0 || n.style.border.iter().any(|b| *b > 0.0);
        if text.is_none() && !is_box {
            continue;
        }
        if n.rect.w <= 0.0 || n.rect.h <= 0.0 {
            continue;
        }
        let Some((cx0, cx1)) = clip_span(nodes, id, Axis::Horizontal) else { continue };
        let Some((cy0, cy1)) = clip_span(nodes, id, Axis::Vertical) else { continue };
        // A control or a line of text that leaves the window: nothing can bring it back. A
        // vertical scroller inside the window is a way back, so it only excuses the vertical axis.
        let r = n.rect;
        let scrolled = mid_scroll(nodes, id);
        let out_x = r.x < -EPS || r.x + r.w > w + EPS;
        let out_y = !scrolled && (r.y < -EPS || r.y + r.h > h + EPS);
        if out_x || out_y {
            out.push(format!(
                "{} at {:.0},{:.0} {:.0}x{:.0} leaves the window {w:.0}x{h:.0}",
                name(&f.dom, id),
                r.x,
                r.y,
                r.w,
                r.h
            ));
            continue;
        }
        if is_box
            && (r.x < cx0 - EPS
                || r.x + r.w > cx1 + EPS
                || (!scrolled && (r.y < cy0 - EPS || r.y + r.h > cy1 + EPS)))
        {
            out.push(format!(
                "{} at {:.0},{:.0} {:.0}x{:.0} leaves its box x{cx0:.0}-{cx1:.0} y{cy0:.0}-{cy1:.0}",
                name(&f.dom, id),
                r.x,
                r.y,
                r.w,
                r.h
            ));
            continue;
        }
        let Some(text) = text else { continue };
        if text.trim().is_empty() {
            continue;
        }
        let s = &n.style;
        let cb = content_box(n);
        let (tw, th) = fonts.measure(text, &n.el.spans, &s.text, s.text.wrap.then_some(cb.w));
        // Text cut by its own box: the room it needs is more than the room it got and nothing
        // says the shortening is deliberate.
        if !s.ellipsis && tw > cb.w + EPS {
            out.push(format!(
                "{} needs {tw:.0}px in {:.0}px: {}",
                name(&f.dom, id),
                cb.w,
                text.chars().take(60).collect::<String>()
            ));
            continue;
        }
        // Text pushed out of the box that clips it (a row narrower than what it holds).
        let right = cb.x + tw.min(cb.w).max(0.0);
        let bottom = cb.y + th.min(cb.h.max(0.0)).max(0.0);
        let out_h = cb.x < cx0 - EPS || right > cx1 + EPS;
        let out_v = !scrolled && (cb.y < cy0 - EPS || bottom > cy1 + EPS);
        if out_h || out_v {
            out.push(format!(
                "{} text x{:.0}-{:.0} y{:.0}-{:.0} outside its clip x{cx0:.0}-{cx1:.0} y{cy0:.0}-{cy1:.0}",
                name(&f.dom, id),
                cb.x,
                right,
                cb.y,
                bottom
            ));
        }
    }
    out
}

// The sizes the app has to hold: a phone in portrait, a tablet in portrait and in landscape, a
// laptop, a desktop and a wide desktop. The window is resizable, so every one of these is a real
// shape of the interface, not a hypothetical.
const SIZES: [(u32, u32); 7] = [
    // The narrowest window worth supporting: a small phone, and the shape where a title ended up
    // one letter per line.
    (360, 640),
    (390, 844),
    (768, 1024),
    (1024, 768),
    (1366, 768),
    (1280, 720),
    (2560, 1440),
];

/// Every view the sweep covers, as the commands that put the window in it.
fn views() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        // Nothing open: the state the app shows on a fresh launch.
        ("sin-nota", vec!["close-tab:0"]),
        ("nota", vec![]),
        ("arbol", vec!["file-explorer:open"]),
        ("buscar", vec!["global-search:open"]),
        ("buscar-opciones", vec!["global-search:open"]),
        ("marcadores", vec!["bookmarks:open"]),
        ("calendario", vec!["calendar:open"]),
        ("etiquetas", vec!["tag-pane:open"]),
        ("retroenlaces", vec!["backlinks:open"]),
        ("salientes", vec!["outgoing-links:open"]),
        ("esquema", vec!["outline:open"]),
        ("ia", vec!["ai:open"]),
        ("grafico", vec!["graph:open"]),
        ("ajustes", vec!["app:open-settings"]),
        ("ajustes-editor", vec!["app:open-settings", "settings:section:editor"]),
        ("ajustes-ia", vec!["app:open-settings", "settings:section:ai"]),
        ("ajustes-plugins", vec!["app:open-settings", "settings:section:plugins"]),
        ("ajustes-avanzado", vec!["app:open-settings", "settings:section:advanced"]),
        ("paleta", vec!["command-palette:open"]),
        ("switcher", vec!["switcher:open"]),
    ]
}

#[test]
fn no_text_is_cut_and_nothing_leaves_the_window() {
    let d = vault();
    let mut fonts = Fonts::new();
    let mut failures: Vec<String> = Vec::new();
    let check =
        |app: &mut App, fonts: &mut Fonts, label: &str, w: u32, h: u32, failures: &mut Vec<String>| {
            let f = frame(app, fonts, w, h);
            for line in cut_nodes(&f, fonts, w as f32, h as f32) {
                failures.push(format!("{label} {w}x{h}: {line}"));
            }
        };
    for (w, h) in SIZES {
        for (label, cmds) in views() {
            let mut app = open(&d);
            for c in cmds {
                app.run(c);
            }
            if label == "buscar-opciones" {
                app.focus = metamind_app::shell::Focus::Search;
            }
            check(&mut app, &mut fonts, label, w, h, &mut failures);
        }
        // A sidebar dragged to its widest: every pane in it keeps its one line instead of
        // leaving a column of empty box with the text cut at the old edge.
        for panel in ["file-explorer:open", "global-search:open", "bookmarks:open", "tag-pane:open"] {
            let mut app = open(&d);
            app.left_w = metamind_app::shell::SIDE_W_MAX;
            app.run(panel);
            check(&mut app, &mut fonts, &format!("ancha:{panel}"), w, h, &mut failures);
        }
        for panel in ["backlinks:open", "outgoing-links:open", "outline:open", "ai:open"] {
            let mut app = open(&d);
            app.right_open = true;
            app.right_w = metamind_app::shell::SIDE_W_MAX;
            app.run(panel);
            check(&mut app, &mut fonts, &format!("ancha-derecha:{panel}"), w, h, &mut failures);
        }
        // The status bar in a narrow window, with the two things that make it long: the hint of
        // whatever is under the pointer and a long-running operation.
        let mut app = open(&d);
        app.hover_action = Some("open:dir/nested/D.md".into());
        app.start_busy("Indexando la bóveda entera");
        check(&mut app, &mut fonts, "barra-estado", 1200, 720, &mut failures);
        let mut app = open(&d);
        app.tab_mut().unwrap().mode = ViewMode::Reading;
        check(&mut app, &mut fonts, "lectura", w, h, &mut failures);
        // A search with no results, and one with a query that matches: the pane's empty state.
        let mut app = open(&d);
        app.run("global-search:open");
        app.search_query = "zzz-sin-resultados".into();
        app.run_search();
        check(&mut app, &mut fonts, "buscar-vacio", w, h, &mut failures);
        let mut app = open(&d);
        app.run("global-search:open");
        app.search_query = "topic".into();
        app.run_search();
        check(&mut app, &mut fonts, "buscar-hits", w, h, &mut failures);
        // Every note open at once: the tab bar has more tabs than room and has to give way
        // instead of cutting one in half.
        let mut app = open(&d);
        for p in [
            "B.md",
            "dir/C.md",
            "dir/nested/D.md",
            "Una nota con un nombre larguísimo para la barra de pestañas.md",
            "Sin enlaces.md",
        ] {
            app.open_note(p);
        }
        check(&mut app, &mut fonts, "muchas-pestanas", w, h, &mut failures);
        // A note with nothing to show on the right: the backlinks pane's empty state.
        let mut app = App::open(d.path()).unwrap();
        app.open_note("Sin enlaces.md");
        app.run("backlinks:open");
        check(&mut app, &mut fonts, "retroenlaces-vacio", w, h, &mut failures);
        // A query longer than its field: the caret is where the typing is, so the field shows the
        // end of the text and an ellipsis for the start, and nothing runs past the box.
        let mut app = open(&d);
        app.run("global-search:open");
        app.focus = metamind_app::shell::Focus::Search;
        app.search_query = "una consulta larguísima ".repeat(6);
        app.run_search();
        let f = frame(&mut app, &mut fonts, w, h);
        let field = f
            .dom
            .nodes
            .iter()
            .find(|n| n.el.classes.iter().any(|c| c == "search-input-container"))
            .and_then(|n| n.shown.as_ref().or(n.el.text.as_ref()));
        let field = field.cloned().unwrap_or_default();
        assert!(field.ends_with('\u{258f}'), "the caret is still in the field: {field:?}");
        assert!(field.starts_with('\u{2026}'), "the field shows the end of the query: {field:?}");
        for line in cut_nodes(&f, &mut fonts, w as f32, h as f32) {
            failures.push(format!("buscar-largo {w}x{h}: {line}"));
        }
        // A palette query longer than the box, for the same reason.
        let mut app = open(&d);
        app.run("command-palette:open");
        for c in "comando con un nombre imposible de largo ".repeat(4).chars() {
            app.text_input(&c.to_string());
        }
        let f = frame(&mut app, &mut fonts, w, h);
        let field = f
            .dom
            .nodes
            .iter()
            .find(|n| n.el.classes.iter().any(|c| c == "prompt-input"))
            .and_then(|n| n.shown.as_ref().or(n.el.text.as_ref()))
            .cloned()
            .unwrap_or_default();
        assert!(field.ends_with('\u{258f}'), "the palette keeps its caret: {field:?}");
        for line in cut_nodes(&f, &mut fonts, w as f32, h as f32) {
            failures.push(format!("paleta-larga {w}x{h}: {line}"));
        }
        // The preview that opens from the pointer, in both corners: it turns around at the edge
        // of the window instead of hanging half of itself out of it.
        for at in [(24.0, 40.0), (w as f32 - 24.0, h as f32 - 40.0)] {
            let mut app = open(&d);
            app.hover_preview = Some(metamind_app::popover::HoverPreview {
                path: "B.md".into(),
                at,
                text: "back to [[A]] and a line long enough to wrap inside the box\n\n".repeat(24),
            });
            check(&mut app, &mut fonts, "popover", w, h, &mut failures);
        }
    }
    assert!(failures.is_empty(), "{} findings:\n{}", failures.len(), failures.join("\n"));
}

/// The width of the first node wearing `class`, in window pixels: a sidebar that is closed is not
/// in the tree at all, so a missing box reads as zero.
fn width_of(f: &Frame, class: &str) -> f32 {
    f.dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == class))
        .map(|n| n.rect.w)
        .fold(0.0, f32::max)
}

/// A sidebar slides from its width to nothing and back, the right one the same way as the left
/// one, and a reader who asked for reduced motion gets the end state on the first frame.
#[test]
fn sidebars_slide_and_reduced_motion_skips_it() {
    let d = vault();
    let mut app = open(&d);
    let mut fonts = Fonts::new();
    // What a window does once it knows whether the desk asked for motion: the clocks run.
    app.anims.enabled = true;
    // The first frame is the one the animation reads: nothing is moving yet.
    let start = frame(&mut app, &mut fonts, 1280, 720);
    assert!(width_of(&start, "mod-left-split") > 100.0, "the explorer starts open");
    assert!(!app.anims.active());

    // The close is an animation too: the frame that starts it still holds the full width, and
    // the one after it has the room back.
    app.left_open = false;
    let _ = frame(&mut app, &mut fonts, 1280, 720);
    assert!(app.anims.active(), "closing a sidebar starts its clock");
    std::thread::sleep(std::time::Duration::from_millis(260));
    let closed = frame(&mut app, &mut fonts, 1280, 720);
    assert_eq!(width_of(&closed, "mod-left-split"), 0.0);
    assert_eq!(width_of(&closed, "mod-right-split"), 0.0);

    // Opening starts both clocks, and the boxes are still a fraction of what they are going to be.
    app.left_open = true;
    app.right_open = true;
    let (want_l, want_r) = (app.left_w, app.right_w);
    let first = frame(&mut app, &mut fonts, 1280, 720);
    assert!(app.anims.active(), "opening a sidebar starts its clock");
    let (l0, r0) = (width_of(&first, "mod-left-split"), width_of(&first, "mod-right-split"));
    assert!(l0 < want_l * 0.9, "left starts at {l0} of {want_l}");
    assert!(r0 < want_r * 0.9, "right starts at {r0} of {want_r}");

    // A quarter of a second later both are at the width they were dragged to.
    std::thread::sleep(std::time::Duration::from_millis(260));
    let open = frame(&mut app, &mut fonts, 1280, 720);
    assert!((width_of(&open, "mod-left-split") - want_l).abs() < 1.0);
    assert!((width_of(&open, "mod-right-split") - want_r).abs() < 1.0);

    // Reduced motion: no clock runs, and the end state is there on the first frame.
    app.set_reduced_motion(true);
    app.left_open = false;
    app.right_open = false;
    let _ = frame(&mut app, &mut fonts, 1280, 720);
    app.left_open = true;
    app.right_open = true;
    let now = frame(&mut app, &mut fonts, 1280, 720);
    assert!(!app.anims.active(), "reduced motion starts no clocks");
    assert!((width_of(&now, "mod-left-split") - want_l).abs() < 1.0);
    assert!((width_of(&now, "mod-right-split") - want_r).abs() < 1.0);
}
