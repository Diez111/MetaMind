//! A `![[Tablero.excalidraw]]` embed is drawn where the note says it, and a board that cannot be
//! read leaves the `![[…]]` line and a notice instead of a picture.
//!
//! The fixtures are the two boards of the `vault-rustsidian-test` copy (Análisis Matemático I,
//! Practica), byte for byte: eight elements in one and seven in the other, text plus one box.
use rustsidian_app::shell::ViewMode;
use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Cmd, Fonts, Pixmap};
use std::fs;

const AREAS: &str = include_str!("fixtures/Tablero-Areas.excalidraw");
const SERIES: &str = include_str!("fixtures/Tablero-Series.excalidraw");
const NOTA: &str = "# Nota\n\nintro\n\n![[Tablero-Areas.excalidraw]]\n";
const ROTA: &str = "# Rota\n\nintro\n\n![[roto.excalidraw]]\n";

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    w("Nota.md", NOTA);
    w("Tablero-Areas.excalidraw", AREAS);
    w("Tablero-Series.excalidraw", SERIES);
    w("Rota.md", ROTA);
    // Half-written JSON: the case a crash or a sync conflict leaves behind.
    w("roto.excalidraw", "{\"type\":\"excalidraw\",\"elements\":[{\"id\":\"t\",\"type\":\"text\",");
    d
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 900).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn texts(f: &Frame, class: &str) -> Vec<String> {
    f.dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == class))
        .filter_map(|n| n.el.text.clone())
        .collect()
}

fn caret_at(app: &mut App, needle: &str) {
    let at = app.tab().unwrap().editor.buf.text().find(needle).unwrap();
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(at)]);
}

fn drawn_text(f: &Frame, node: rustsidian_ui::NodeId) -> Vec<String> {
    f.dom.nodes[node]
        .draw
        .iter()
        .filter_map(|c| match c {
            Cmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_board_of_the_embed_is_drawn_in_place_of_the_name() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Nota.md");
    app.tab_mut().unwrap().mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.exc.len(), 1, "one board, one embed");
    let it = &f.info.exc[0];
    assert_eq!(it.board.list.ops.len(), 8, "the board's elements are the ops: 7 text runs and a box");
    assert!(it.board.list.skipped.is_empty(), "{:?}", it.board.list.skipped);
    let cmds = &f.dom.nodes[it.node].draw;
    assert!(cmds.len() >= 9, "a box and the text runs, at least: {}", cmds.len());
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Fill { .. })), "the box has a background");
    let texts = drawn_text(&f, it.node);
    assert!(texts.iter().any(|t| t.starts_with("Áreas y TFC")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("D5.")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("Resolvé acá")), "{texts:?}");
    // Fit-width inside the note column, and tall enough for the whole board.
    let r = f.dom.content_rect(it.node);
    assert!(r.w > 300.0 && r.h > 100.0, "{r:?}");
    let (_, _, bw, bh) = it.board.bounds;
    assert!((r.h - (bh * (r.w / bw).min(1.0) + 2.0 * 8.0)).abs() < 2.0, "height follows the scale");
    // The paint order is the file order: the box is painted before the label inside it.
    let first_fill = cmds.iter().position(|c| matches!(c, Cmd::Fill { .. })).unwrap();
    let label = cmds
        .iter()
        .position(|c| matches!(c, Cmd::Text { text, .. } if text.contains("Resolvé acá")))
        .unwrap();
    assert!(first_fill < label);
}

#[test]
fn both_real_boards_draw_every_element() {
    let d = vault();
    fs::write(d.path().join("Series.md"), "intro\n\n![[Tablero-Series.excalidraw]]\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Series.md");
    app.tabs[0].mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.exc.len(), 1);
    assert_eq!(f.info.exc[0].board.list.ops.len(), 7);
    assert!(f.info.exc[0].board.list.skipped.is_empty());
    assert!(drawn_text(&f, f.info.exc[0].node).iter().any(|t| t.starts_with("Series de potencias")));
}

#[test]
fn a_broken_board_keeps_the_text_and_says_why() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Rota.md");
    app.tabs[0].mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    assert!(f.info.exc.is_empty(), "nothing to paint");
    let notices = texts(&f, "rich-notice");
    assert!(
        notices.iter().any(|t| t.contains("roto.excalidraw") && t.contains("no se pudo leer")),
        "{notices:?}"
    );
    // The line that names the board is still there, in the note's text.
    assert!(f.info.blocks.iter().any(|b| b.display.contains("roto.excalidraw")));
}

#[test]
fn the_source_shows_while_the_caret_is_on_the_embed() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Nota.md");
    let mut fonts = Fonts::new();
    caret_at(&mut app, "intro");
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.exc.len(), 1, "live preview with the cursor away draws the board");
    caret_at(&mut app, "![[Tablero");
    let f = frame(&mut app, &mut fonts);
    assert!(f.info.exc.is_empty(), "the line under the caret stays editable text");
    assert!(f.info.blocks.iter().any(|b| b.display.contains("![[Tablero-Areas.excalidraw]]")));
}

#[test]
fn the_board_is_read_once_per_modification_time() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let a = app.media.borrow_mut().board(&app.store, "Tablero-Areas.excalidraw").unwrap();
    let b = app.media.borrow_mut().board(&app.store, "Tablero-Areas.excalidraw").unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b), "the second frame reuses the board");
    // Rewriting the file changes its mtime, and the next frame reads it again.
    std::thread::sleep(std::time::Duration::from_millis(10));
    let series = fs::read_to_string(d.path().join("Tablero-Series.excalidraw")).unwrap();
    fs::write(d.path().join("Tablero-Areas.excalidraw"), series).unwrap();
    app.files_changed(&["Tablero-Areas.excalidraw".to_string()]);
    let c = app.media.borrow_mut().board(&app.store, "Tablero-Areas.excalidraw").unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a, &c));
    match &*c {
        rustsidian_app::excalidraw::Loaded::Board(board) => assert_eq!(board.list.ops.len(), 7),
        rustsidian_app::excalidraw::Loaded::Failed(e) => panic!("{e}"),
    }
}
