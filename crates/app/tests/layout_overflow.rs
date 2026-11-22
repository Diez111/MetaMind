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
use metamind_app::shell::{Panel, RightPanel, ViewMode};
use metamind_app::{render, App, Frame, Key};
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
        // A video embed: the card that names the file, with the button that opens the player.
        // The bytes are not a film: the gate never decodes one (see `METAMIND_NO_POSTERS` below).
        ("Video.md", "# Video\n\n![[clip.mp4]]\n\ntexto despues de la tarjeta\n"),
    ] {
        fs::write(d.path().join(p), t).unwrap();
    }
    fs::write(d.path().join("clip.mp4"), b"un archivo que existe y no se decodifica").unwrap();
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
        ("barra-inferior", vec!["calendar:open", "more:open"]),
        ("menu-mas", vec!["more:open"]),
        ("nota", vec![]),
        ("video", vec!["open:Video.md"]),
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
        // The copilot in a compact window: the same command, and the sheet it opens there.
        ("ia-hoja", vec!["ai:open"]),
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
    // The video card is checked with its button, not its poster: decoding one would start an
    // `ffmpeg` per card, and a test does not run processes for a picture it throws away.
    std::env::set_var("METAMIND_NO_POSTERS", "1");
    let d = vault();
    let mut fonts = Fonts::new();
    let mut failures: Vec<String> = Vec::new();
    let check =
        |app: &mut App, fonts: &mut Fonts, label: &str, w: u32, h: u32, failures: &mut Vec<String>| {
            // The final geometry, not a frame on the way to it: the animation clocks are wall time.
            app.anims.settle();
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
        // The copilot with a conversation in it and a profile name too long for a phone: in a
        // compact window the sheet shortens the model instead of pushing the way out of the
        // header, and the input keeps its line above the status bar.
        let mut app = open(&d);
        app.run("ai:open");
        let mut cfg = metamind_app::ai::AiConfig::from_config(&app.config);
        cfg.profiles[0].model = "un-modelo-con-un-nombre-imposible-de-largo-para-un-telefono".into();
        app.config.app.set(metamind_app::ai::KEY_PROFILES, cfg.to_json()["profiles"].clone());
        app.ai.messages.push(metamind_app::ai::Message::user("¿Qué dice la nota sobre los enlaces?"));
        app.ai.messages.push(metamind_app::ai::Message::assistant(
            "Dice que [[A]] enlaza a [[B]] y que los enlaces que no resuelven quedan pendientes. "
                .repeat(3)
                .trim(),
        ));
        check(&mut app, &mut fonts, "ia-hoja-conversacion", w, h, &mut failures);
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

/// The first node wearing `class`, in document order.
fn classed(f: &Frame, class: &str) -> Option<usize> {
    f.dom.nodes.iter().position(|n| n.el.classes.iter().any(|c| c == class))
}

/// The copilot in a compact window is a sheet over the workspace: no column is taken from the
/// note, the header keeps its way out inside the window, and the input ends above the status
/// bar. A wide window keeps the column it always had.
#[test]
fn the_copilot_sheet_covers_the_workspace_in_a_compact_window() {
    let d = vault();
    let mut fonts = Fonts::new();
    let mut app = open(&d);
    // The first frame is where the window learns it is compact.
    let _ = frame(&mut app, &mut fonts, 360, 640);
    app.run("ai:open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    assert!(app.right_open);
    assert_eq!(app.right_panel, RightPanel::Ai);
    assert_eq!(width_of(&f, "mod-right-split"), 0.0, "no column is reserved for the copilot");
    let sheet = classed(&f, "ai-sheet").expect("the copilot is a sheet in a compact window");
    let root = classed(&f, "mod-root").expect("the note");
    assert!(root < sheet, "the sheet is added after the note, so it is painted over it");
    let ws = classed(&f, "workspace").expect("the workspace");
    let (s, w) = (f.dom.nodes[sheet].rect, f.dom.nodes[ws].rect);
    assert!((s.x - w.x).abs() < 1.0 && (s.w - w.w).abs() < 1.0, "the sheet covers the workspace: {s:?}");
    assert!(
        (s.y - w.y).abs() < 1.0 && (s.h - w.h).abs() < 1.0,
        "and ends where it does: sheet {s:?} workspace {w:?}"
    );
    // The way out is inside the window, whatever the profile is called.
    let close = f.dom.nodes[classed(&f, "ai-sheet-close").expect("the close button")].rect;
    assert!(close.x >= 0.0 && close.x + close.w <= 360.0, "the close button stays visible: {close:?}");
    // The input sits on the last line of the sheet and never over the status bar.
    let field = f.dom.nodes[classed(&f, "ai-input-container").expect("the input")].rect;
    assert!(field.y > w.y, "the input is at the bottom of the sheet: {field:?}");
    assert!(field.y + field.h <= w.y + w.h + 0.5, "the input ends above the status bar: {field:?}");
    assert!(f.dom.by_id("ai-messages").is_some(), "the conversation is the panel's own");
    // A press on the blank space of the sheet belongs to the panel: the note behind it keeps its
    // caret where it was, instead of taking a click nobody aimed at it.
    let at = (s.x + s.w / 2.0, s.y + s.h / 2.0);
    let mut hit = f.dom.hit(at.0, at.1);
    let mut over_sheet = false;
    while let Some(i) = hit {
        if i == sheet {
            over_sheet = true;
            break;
        }
        hit = f.dom.nodes[i].parent;
    }
    assert!(over_sheet, "the sheet is what the pointer finds over the note");
    let caret = app.tab().unwrap().editor.buf.sels[0];
    app.click(&f, &mut fonts, at.0, field.y - 30.0, false);
    assert_eq!(app.tab().unwrap().editor.buf.sels[0], caret, "the hidden note keeps its caret");
    // The keys go to the question as well, and not to the note the sheet hides.
    let note = app.tab().unwrap().editor.buf.text();
    app.text_input("x");
    assert!(app.ai.input.ends_with('x'), "typing reaches the question: {:?}", app.ai.input);
    assert_eq!(app.tab().unwrap().editor.buf.text(), note, "the hidden note keeps its text");
    let before = app.right_open;
    app.click(&f, &mut fonts, close.x + close.w / 2.0, close.y + close.h / 2.0, false);
    assert!(before && !app.right_open, "the close button runs ai:close");
    let after = frame(&mut app, &mut fonts, 360, 640);
    assert!(classed(&after, "ai-sheet").is_none(), "the sheet goes with the panel");

    // Escape is the keyboard's way out: the input takes the first press, the sheet goes on the
    // second, and the note keeps the text it had.
    app.run("ai:open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    let field = f.dom.nodes[classed(&f, "ai-input").expect("the field")].rect;
    app.click(&f, &mut fonts, field.x + 4.0, field.y + 4.0, false);
    assert!(app.ai.editing, "a press on the field gives it the keyboard");
    app.key(Key::parse("Escape").unwrap());
    assert!(app.right_open, "the first Escape stops the field editing");
    app.key(Key::parse("Escape").unwrap());
    assert!(!app.right_open, "the second one closes the sheet");
    assert_eq!(app.tab().unwrap().editor.buf.text(), note, "the note is as it was");

    // A window with room keeps the panel as the column beside the note.
    let mut app = open(&d);
    app.run("ai:open");
    let f = frame(&mut app, &mut fonts, 1280, 720);
    assert!(classed(&f, "ai-sheet").is_none(), "no sheet when the window has room");
    assert!(width_of(&f, "mod-right-split") > 100.0, "the right sidebar keeps its column");
    assert!(classed(&f, "ai-panel").is_some(), "and the panel inside it");

    // A profile longer than the header ends in an ellipsis, and the title keeps every letter.
    let mut app = open(&d);
    let _ = frame(&mut app, &mut fonts, 360, 640);
    let mut cfg = metamind_app::ai::AiConfig::from_config(&app.config);
    cfg.profiles[0].model = "un-modelo-con-un-nombre-imposible-de-largo-para-un-telefono".into();
    app.config.app.set(metamind_app::ai::KEY_PROFILES, cfg.to_json()["profiles"].clone());
    app.run("ai:open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    let model = &f.dom.nodes[classed(&f, "ai-sheet-model").expect("the model")];
    assert!(
        model.shown.as_deref().is_some_and(|s| s.ends_with('…')),
        "the model ends in an ellipsis: {:?}",
        model.shown.as_deref().or(model.el.text.as_deref())
    );
    let title = &f.dom.nodes[classed(&f, "ai-sheet-title").expect("the title")];
    assert_eq!(title.el.text.as_deref(), Some("Copiloto"));
    assert!(title.shown.is_none(), "the title is never shortened");
}

/// The sheet slides in from the right on its own clock, and a reader who asked for reduced motion
/// gets it in place on the first frame.
#[test]
fn the_copilot_sheet_slides_in_and_reduced_motion_skips_it() {
    let d = vault();
    let mut fonts = Fonts::new();
    let mut app = open(&d);
    let _ = frame(&mut app, &mut fonts, 360, 640);
    assert!(!app.anims.active());
    app.anims.enabled = true;
    app.run("ai:open");
    let first = frame(&mut app, &mut fonts, 360, 640);
    assert!(app.anims.active(), "opening the sheet starts its clock");
    let sheet = classed(&first, "ai-sheet").expect("the sheet is in the tree while it slides");
    assert!(first.dom.nodes[sheet].rect.x > 100.0, "it starts off the right edge");
    std::thread::sleep(std::time::Duration::from_millis(260));
    let settled = frame(&mut app, &mut fonts, 360, 640);
    let (sheet, ws) = (
        classed(&settled, "ai-sheet").expect("the sheet"),
        classed(&settled, "workspace").expect("the workspace"),
    );
    assert!(
        (settled.dom.nodes[sheet].rect.x - settled.dom.nodes[ws].rect.x).abs() < 1.0,
        "it lands on the workspace"
    );

    // Reduced motion: no clock runs, and the sheet is in place on the first frame.
    app.set_reduced_motion(true);
    app.run("ai:close");
    let _ = frame(&mut app, &mut fonts, 360, 640);
    app.run("ai:open");
    let now = frame(&mut app, &mut fonts, 360, 640);
    assert!(!app.anims.active(), "reduced motion starts no clock");
    let (sheet, ws) =
        (classed(&now, "ai-sheet").expect("the sheet"), classed(&now, "workspace").expect("the workspace"));
    assert!((now.dom.nodes[sheet].rect.x - now.dom.nodes[ws].rect.x).abs() < 1.0);

    // And it leaves the way it came: the frame after the close still draws it, on its way out.
    app.set_reduced_motion(false);
    app.run("ai:open");
    let _ = frame(&mut app, &mut fonts, 360, 640);
    std::thread::sleep(std::time::Duration::from_millis(260));
    let _ = frame(&mut app, &mut fonts, 360, 640);
    app.run("ai:close");
    let leaving = frame(&mut app, &mut fonts, 360, 640);
    assert!(app.anims.active(), "closing the sheet starts its clock too");
    let sheet = classed(&leaving, "ai-sheet").expect("it is still drawn while it leaves");
    let ws = classed(&leaving, "workspace").expect("the workspace");
    let away = leaving.dom.nodes[sheet].rect.x - leaving.dom.nodes[ws].rect.x;
    assert!(away > 1.0, "it has left its place: {away}");
    std::thread::sleep(std::time::Duration::from_millis(260));
    let gone = frame(&mut app, &mut fonts, 360, 640);
    assert!(classed(&gone, "ai-sheet").is_none(), "and it is gone once the clock is done");
}

#[test]
fn the_bottom_navigation_belongs_to_a_compact_window() {
    let d = vault();
    let mut app = open(&d);
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts, 360, 640);
    let all = |f: &Frame, class: &str| -> Vec<usize> {
        f.dom
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.el.classes.iter().any(|c| c == class))
            .map(|(i, _)| i)
            .collect()
    };
    let nav = *all(&f, "bottom-nav").first().expect("a compact window has the bottom navigation");
    let ws = classed(&f, "workspace").expect("the workspace");
    let (n, w) = (f.dom.nodes[nav].rect, f.dom.nodes[ws].rect);
    assert!((n.w - 360.0).abs() < 1.0, "the bar spans the window: {n:?}");
    assert!((n.y - (w.y + w.h)).abs() < 1.0, "it starts where the note ends: {n:?} {w:?}");
    assert!(n.y + n.h <= 640.0, "and stays inside the window: {n:?}");
    // The status bar is still there, under the bar: the navigation sits on top of nothing and
    // the note ends above both.
    let status = classed(&f, "status-bar").expect("a compact window keeps its status bar");
    let s = f.dom.nodes[status].rect;
    assert!(s.h > 1.0, "the status bar has a line of its own: {s:?}");
    assert!((s.y - (n.y + n.h)).abs() < 1.0, "it starts where the navigation ends: {s:?} {n:?}");
    assert!((s.y + s.h - 640.0).abs() < 1.0, "and ends at the bottom of the window: {s:?}");
    assert!(n.h >= 44.0, "every destination is 44 px tall at least: {n:?}");

    // Five destinations and the way into the rest of the app, the ⋮ at the right end.
    let items = all(&f, "bottom-nav-item");
    assert_eq!(items.len(), 6, "five destinations and the menu");
    let more = f.dom.nodes[items[5]].rect;
    assert!((more.x + more.w - 360.0).abs() < 1.0, "the menu sits at the bottom right corner: {more:?}");
    app.run("more:open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    let menu = *all(&f, "more-sheet").first().expect("the menu opens over the note");
    let r = f.dom.nodes[menu].rect;
    assert!(r.h > 400.0 && (r.w - 360.0).abs() < 1.0, "the menu takes the window: {r:?}");
    assert!(all(&f, "more-row").len() >= 8, "the menu lists what the ribbon held");
    app.run("more:close");
    let f = frame(&mut app, &mut fonts, 360, 640);
    // Five destinations and the menu split the width in equal parts at the narrowest window, and
    // each one holds its icon and its name without cutting the name in the middle of a word.
    let slots: Vec<metamind_ui::Rect> =
        all(&f, "bottom-nav-item").iter().map(|&i| f.dom.nodes[i].rect).collect();
    let widest = slots.iter().map(|r| r.w).fold(0.0_f32, f32::max);
    let narrowest = slots.iter().map(|r| r.w).fold(f32::MAX, f32::min);
    assert!((widest - narrowest).abs() < 1.0, "the slots are equal: {slots:?}");
    for &i in &all(&f, "bottom-nav-item") {
        let r = f.dom.nodes[i].rect;
        assert!(r.x >= -0.5 && r.x + r.w <= 360.5, "a destination stays in the window: {r:?}");
        assert!(r.h >= 44.0, "its touch area is comfortable: {r:?}");
        let label = f.dom.nodes[i].children.last().copied().expect("a destination has a label");
        let l = f.dom.nodes[label].rect;
        assert!(l.x >= r.x - 0.5 && l.x + l.w <= r.x + r.w + 0.5, "the label fits its slot: {l:?} {r:?}");
    }
    // With the note in front and no pane open, no destination is in force.
    let active = |f: &Frame| -> Vec<usize> {
        f.dom
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.el.classes.iter().any(|c| c == "bottom-nav-item")
                    && n.el.classes.iter().any(|c| c == "is-active")
            })
            .map(|(i, _)| i)
            .collect()
    };
    assert!(active(&f).is_empty(), "the note alone marks no destination");
    // Opening one marks exactly that one.
    app.run("global-search:open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    let marked = active(&f);
    assert_eq!(marked.len(), 1, "exactly one destination is in force: {marked:?}");
    let label = f.dom.nodes[marked[0]].children.last().copied();
    let text = label.and_then(|l| f.dom.nodes[l].el.text.clone()).unwrap_or_default();
    assert_eq!(text, "Buscar", "the marked destination is the one that opened");
    let dot = *all(&f, "bottom-nav-indicator").first().expect("the indicator");
    let d = f.dom.nodes[dot].rect;
    assert!(d.x >= -0.5 && d.x + d.w <= 360.5, "the indicator stays inside the bar: {d:?}");
    // It is under the second destination: the one in force.
    assert!(
        d.x > slots[1].x && d.x + d.w < slots[1].x + slots[1].w,
        "the indicator marks what opened: {d:?}"
    );

    // Moving to another destination slides the indicator, and a reader who asked for reduced
    // motion gets it in place on the first frame.
    app.anims.enabled = true;
    app.set_reduced_motion(false);
    let _ = frame(&mut app, &mut fonts, 360, 640);
    app.run("calendar:open");
    let moving = frame(&mut app, &mut fonts, 360, 640);
    assert!(app.anims.active(), "changing destination starts its clock");
    let mid = moving.dom.nodes[*all(&moving, "bottom-nav-indicator").first().unwrap()].rect;
    assert!(mid.x >= d.x - 0.5 && mid.x < slots[2].x, "the indicator is on the way: {mid:?}");
    std::thread::sleep(std::time::Duration::from_millis(260));
    let arrived = frame(&mut app, &mut fonts, 360, 640);
    let end = arrived.dom.nodes[*all(&arrived, "bottom-nav-indicator").first().unwrap()].rect;
    assert!(end.x > slots[2].x && end.x + end.w < slots[2].x + slots[2].w, "it arrives: {end:?}");

    app.set_reduced_motion(true);
    app.run("global-search:open");
    let still = frame(&mut app, &mut fonts, 360, 640);
    assert!(!app.anims.active(), "reduced motion starts no clock");
    let now = still.dom.nodes[*all(&still, "bottom-nav-indicator").first().unwrap()].rect;
    assert!(now.x > slots[1].x && now.x + now.w < slots[1].x + slots[1].w, "it is already there: {now:?}");

    // The bar is what the pointer finds: no sheet, no note, nothing is painted over it. A press
    // on Ajustes opens the settings panel and not the note behind it.
    let items_now = all(&still, "bottom-nav-item");
    let slot = still.dom.nodes[items_now[4]].rect;
    let hit_at = (slot.x + slot.w / 2.0, slot.y + slot.h / 2.0);
    let bar = *all(&still, "bottom-nav").first().unwrap();
    let mut hit = still.dom.hit(hit_at.0, hit_at.1);
    let mut on_nav = false;
    while let Some(i) = hit {
        if i == bar {
            on_nav = true;
            break;
        }
        hit = still.dom.nodes[i].parent;
    }
    assert!(on_nav, "the bar is what the pointer finds at the bottom of the window");
    assert!(app.settings.is_none());
    app.click(&still, &mut fonts, hit_at.0, hit_at.1, false);
    assert!(app.settings.is_some(), "the Ajustes destination opens the settings panel");
    app.run("settings:close");

    // A sheet over the note hides the sidebar behind it, so what it hides is not marked: the
    // reader sees one destination in force, not two.
    app.left_open = true;
    app.panel = Panel::Files;
    app.right_panel = RightPanel::Ai;
    app.right_open = true;
    let covered = frame(&mut app, &mut fonts, 360, 640);
    let only = active(&covered);
    assert_eq!(only.len(), 1, "one destination at a time: {only:?}");
    let text = covered.dom.nodes[only[0]]
        .children
        .last()
        .copied()
        .and_then(|l| covered.dom.nodes[l].el.text.clone())
        .unwrap_or_default();
    assert_eq!(text, "Agente", "the sheet is the destination the reader sees");

    // A window with room keeps the ribbon and has no bottom navigation.
    let f = frame(&mut app, &mut fonts, 1366, 768);
    assert!(all(&f, "bottom-nav").is_empty(), "the desktop is untouched");
}

/// A sheet in front of the note is a modal for the whole keyboard, not only for the letters: a
/// key that would edit text or move the caret gives the field back its caret instead of
/// reaching the note the reader cannot see.
#[test]
fn the_copilot_sheet_takes_the_editing_keys_from_the_note_behind_it() {
    let d = vault();
    let mut fonts = Fonts::new();
    let mut app = open(&d);
    let _ = frame(&mut app, &mut fonts, 360, 640);
    // `ai:open` opens the sheet without putting the caret in the field: the keys that follow are
    // exactly the case a reader hits when a shortcut opened the panel.
    app.run("ai:open");
    let _ = frame(&mut app, &mut fonts, 360, 640);
    assert!(app.ai_sheet());
    assert!(!app.ai.editing, "the shortcut leaves no caret in the field");
    let text = app.tab().unwrap().editor.buf.text();
    let caret = app.tab().unwrap().editor.buf.sels[0];
    for name in [
        "Enter",
        "Backspace",
        "Delete",
        "Tab",
        "Home",
        "End",
        "ArrowLeft",
        "ArrowRight",
        "ArrowUp",
        "PageDown",
    ] {
        app.ai.editing = false;
        app.key(Key::parse(name).unwrap());
        assert_eq!(app.tab().unwrap().editor.buf.text(), text, "{name} does not reach the note");
        assert_eq!(app.tab().unwrap().editor.buf.sels[0], caret, "{name} does not move the caret");
        assert!(app.ai.editing, "{name} gives the field the caret, like a typed letter");
    }

    // The palette opens over the sheet, so it is the palette the keyboard speaks to: the letters
    // and the keys that edit text are the palette's, and Escape closes the one on top.
    app.run("command-palette:open");
    app.text_input("zzzz");
    assert!(!app.ai.input.contains('z'), "typing reaches the palette, not the question: {:?}", app.ai.input);
    app.key(Key::parse("Enter").unwrap());
    assert!(!app.ai.editing, "Enter is the palette's, not the field's behind it");
    assert!(app.ai.input.is_empty(), "the question is untouched: {:?}", app.ai.input);
    app.key(Key::parse("Escape").unwrap());
    assert!(app.ai_sheet(), "Escape closes the palette, not the sheet under it");
    app.key(Key::parse("Escape").unwrap());
    assert!(!app.ai_sheet(), "the next one closes the sheet");
    assert_eq!(app.tab().unwrap().editor.buf.text(), text, "the note is as it was");
}

/// The ⋮ menu is a sheet of the same kind: a plain key stays with the list, a press on its blank
/// space is not a caret in the note nor a pan of a graph behind it, Escape closes it, and a
/// command fired while it is open closes it first.
#[test]
fn the_menu_sheet_owns_its_keys_and_its_blank_space() {
    let d = vault();
    let mut fonts = Fonts::new();
    let mut app = open(&d);
    let _ = frame(&mut app, &mut fonts, 360, 640);
    app.run("more:open");
    let _ = frame(&mut app, &mut fonts, 360, 640);
    let text = app.tab().unwrap().editor.buf.text();
    let caret = app.tab().unwrap().editor.buf.sels[0];
    app.key(Key::parse("Backspace").unwrap());
    app.key(Key::parse("Enter").unwrap());
    app.text_input("z");
    assert_eq!(app.tab().unwrap().editor.buf.text(), text, "the note behind the list keeps its text");
    assert_eq!(app.tab().unwrap().editor.buf.sels[0], caret, "and its caret");
    assert!(app.more_open, "the list is still open: it had nothing to delete");

    app.key(Key::parse("Escape").unwrap());
    assert!(!app.more_open, "Escape closes the menu");

    // The blank space of the list is the list's: the field being written keeps the keyboard.
    app.run("more:open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    let menu = f.dom.nodes[classed(&f, "more-sheet").expect("the menu")].rect;
    app.ai.editing = true;
    app.click(&f, &mut fonts, menu.x + 3.0, menu.y + menu.h / 2.0, false);
    assert!(app.ai.editing, "the press does not take the keyboard from the field");

    // A destination of the bottom navigation, a command from the palette, or a row of the list:
    // anything fired while the list is open closes it, so what it opens is not painted under it.
    app.run("more:open");
    app.run("app:open-settings");
    assert!(!app.more_open, "a command closes the list it was fired under");
    assert!(app.settings.is_some());
    app.run("settings:close");
    app.run("more:open");
    app.run("file-explorer:open");
    assert!(!app.more_open && app.left_open, "so does a destination of the bar");
}

/// One destination is in force and it is the one on top: the settings panel over the sheet marks
/// Ajustes, and the sheet under it is not marked a second time.
#[test]
fn the_bar_marks_only_the_destination_the_reader_sees() {
    let d = vault();
    let mut fonts = Fonts::new();
    let mut app = open(&d);
    let _ = frame(&mut app, &mut fonts, 360, 640);
    let marked = |f: &Frame| -> Vec<String> {
        f.dom
            .nodes
            .iter()
            .filter(|n| {
                n.el.classes.iter().any(|c| c == "bottom-nav-item")
                    && n.el.classes.iter().any(|c| c == "is-active")
            })
            .filter_map(|n| {
                n.children.last().copied().and_then(|l| f.dom.nodes.get(l)).and_then(|l| l.el.text.clone())
            })
            .collect()
    };
    // The settings panel is painted over the sheet, so Ajustes is the one destination in force.
    app.run("ai:open");
    app.run("app:open-settings");
    let f = frame(&mut app, &mut fonts, 360, 640);
    assert_eq!(marked(&f), ["Ajustes"], "one destination, and it is the panel on top");

    // The calendar is a panel of the left sidebar: asking for it closes the sheet it would open
    // behind, so the reader sees the calendar and one destination in force.
    app.run("settings:close");
    app.run("ai:open");
    assert!(app.ai_sheet());
    app.run("calendar:open");
    assert!(!app.ai_sheet(), "the calendar opens over the note, not behind the sheet");
    assert!(app.left_open && app.panel == Panel::Calendar, "the calendar is the panel open");
    let f = frame(&mut app, &mut fonts, 360, 640);
    assert_eq!(marked(&f), ["Calendario"], "the destination the reader lands on is the one marked");
    assert!(classed(&f, "ai-sheet").is_none(), "the sheet is not drawn over it");
}

#[test]
fn no_stylesheet_rule_hides_behind_a_line_break() {
    // The CSS reader of this engine takes a selector list only when it is on one line: `a,\nb { … }`
    // is dropped without a word, and the style the rule described never exists. Four rules were
    // dead that way (the ones that fold a settings row in a narrow window), so the sheet is checked.
    let css = metamind_app::shell::BASE_CSS;
    let mut offenders = Vec::new();
    let mut in_comment = false;
    for (i, line) in css.lines().enumerate() {
        // A comment that wraps is not a selector: it just happens to end in a comma.
        let opens = line.matches("/*").count();
        let closes = line.matches("*/").count();
        let was_in_comment = in_comment;
        in_comment = (in_comment || opens > closes) && !(closes > opens);
        if was_in_comment || line.trim_start().starts_with("/*") {
            continue;
        }
        let code = match line.find("/*") {
            Some(at) => &line[..at],
            None => line,
        };
        let t = code.trim_end();
        if t.ends_with(',') && !t.trim_start().starts_with('@') && !t.is_empty() {
            offenders.push(format!("{}: {t}", i + 1));
        }
    }
    assert!(offenders.is_empty(), "a selector list split across lines is never read: {offenders:#?}");
}
