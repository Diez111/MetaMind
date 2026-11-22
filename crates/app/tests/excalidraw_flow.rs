//! The user's way into a board, end to end: a note with `![[Tablero.excalidraw]]`, a press on the
//! embed, the board open in its own tab, a rectangle drawn by dragging, the element moved, undo and
//! redo around it, a text typed on a double click, and the file on disk when the tab is given back
//! to the note.
//!
//! This is the flow the screenshots caught failing: the embed was a flat picture with no way into
//! the editor, so the user could never reach a surface to draw on. Every step here goes through
//! the public input path (`click`, `drag`, `key`, `run`), the way a mouse and a keyboard do.

use excalidraw_core::ElementType;
use metamind_app::shell::ViewMode;
use metamind_app::{render, App, Frame, Key};
use metamind_ui::{Cmd, Color, Fonts, Pixmap, Rect};
use std::fs;

const BOARD: &str = r##"{"type":"excalidraw","version":2,"source":"https://excalidraw.com",
"elements":[
 {"id":"title","type":"text","x":20,"y":20,"width":240,"height":28,"angle":0,"strokeColor":"#1971c2",
  "text":"Tablero de areas","fontSize":20,"fontFamily":1,"textAlign":"left","lineHeight":1.25},
 {"id":"box","type":"rectangle","x":40,"y":120,"width":200,"height":100,"angle":0,
  "strokeColor":"#1e1e1e","backgroundColor":"#a5d8ff","strokeWidth":2,"fillStyle":"solid","opacity":100,
  "seed":7,"version":1,"isDeleted":false}
],"appState":{"viewBackgroundColor":"#ffffff","gridSize":null},"files":{}}"##;

const NOTA: &str = "# Nota\n\nintro\n\n![[Tablero.excalidraw]]\n";

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), NOTA).unwrap();
    fs::write(d.path().join("Tablero.excalidraw"), BOARD).unwrap();
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

/// The text of every node with `class`.
fn texts(f: &Frame, class: &str) -> Vec<String> {
    f.dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == class))
        .filter_map(|n| n.el.text.clone())
        .collect()
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 900).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn pane(f: &Frame) -> Rect {
    f.dom.nodes[*f.info.excview.first().expect("the board is in its pane")].rect
}

/// The board as Excalidraw would read it back, from the file on disk.
fn saved(d: &tempfile::TempDir) -> excalidraw_core::Scene {
    excalidraw_core::Scene::parse(&fs::read_to_string(d.path().join("Tablero.excalidraw")).unwrap())
        .unwrap()
}

/// An application with a note open in reading mode, the way the screenshots show it.
fn open_note() -> (tempfile::TempDir, App, Fonts) {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home.md");
    app.tabs[app.active].mode = ViewMode::Reading;
    (d, app, Fonts::new())
}

#[test]
fn the_embed_of_a_note_opens_the_board_where_the_user_can_draw() {
    let (_d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.exc.len(), 1, "the note draws the board");
    assert!(app.tab().is_some(), "the note is the open tab");
    assert!(!app.exc_active(), "and the board is a picture, not an editor");

    // A press on the embed: the board opens in its own tab, with its tools.
    let embed = f.dom.content_rect(f.info.exc[0].node);
    let (x, y) = (embed.x + embed.w / 2.0, embed.y + embed.h / 2.0);
    app.click(&f, &mut fonts, x, y, false);
    assert!(app.exc_active(), "the press opens the editor");
    assert_eq!(app.exc_view.as_ref().unwrap().path, "Tablero.excalidraw");
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.excview.len(), 1, "the editor paints its surface");
    let toolbar = f
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "exc-toolbar"))
        .expect("the toolbar of the editor");
    assert!(toolbar.children.len() >= 9, "the eight tools and the fold button: {}", toolbar.children.len());
    assert!(
        f.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "exc-zoombar")),
        "and the zoom bar with undo and redo"
    );
}

#[test]
fn a_drag_draws_a_rectangle_and_undo_takes_it_away() {
    let (d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    app.click(&f, &mut fonts, embed.x + 40.0, embed.y + 40.0, false);
    assert!(app.exc_active());
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let before = app.exc_view.as_ref().unwrap().scene.elements.len();

    // Rectangle tool, drag: the shape exists at the size the drag covered, at any zoom.
    app.run("exc:tool:rect");
    let (x, y) = (r.x + 120.0, r.y + 120.0);
    app.click(&f, &mut fonts, x, y, false);
    app.drag(&f, &mut fonts, x + 140.0, y + 90.0);
    app.release();
    let v = app.exc_view.as_ref().unwrap();
    assert_eq!(v.scene.elements.len(), before + 1, "the rectangle is in the scene");
    let made = v.scene.elements.last().unwrap().clone();
    assert_eq!(made.kind, ElementType::Rectangle);
    assert!((made.width as f32 * v.zoom - 140.0).abs() < 1.5, "the box is the drag: {}", made.width);
    assert!((made.height as f32 * v.zoom - 90.0).abs() < 1.5);
    let at = (made.x, made.y);

    // The select tool drags it somewhere else.
    app.run("exc:tool:select");
    let vr = app.exc_view.as_ref().unwrap();
    let (sx, sy) = vr.to_screen(&r, made.x + made.width / 2.0, made.y + made.height / 2.0);
    app.click(&f, &mut fonts, sx, sy, false);
    assert_eq!(app.exc_view.as_ref().unwrap().selected.as_deref(), Some(made.id.as_str()));
    app.drag(&f, &mut fonts, sx + 50.0, sy + 30.0);
    app.release();
    let moved = app.exc_view.as_ref().unwrap().scene.element(&made.id).cloned().unwrap();
    assert!(moved.x > at.0 && moved.y > at.1, "{moved:?} moved from {at:?}");

    // Undo the move: it goes back where it was drawn, and the step before that takes the shape
    // away — the two edits are two steps, as in every editor.
    app.key(key("Ctrl+Z"));
    let back = app.exc_view.as_ref().unwrap().scene.element(&made.id).cloned().unwrap();
    assert!((back.x - at.0).abs() < 0.01 && (back.y - at.1).abs() < 0.01, "{back:?}");
    app.key(key("Ctrl+Z"));
    assert!(app.exc_view.as_ref().unwrap().scene.element(&made.id).is_none(), "the shape is gone");

    // Redo puts the drawing back, in both steps.
    app.key(key("Ctrl+Shift+Z"));
    assert!(app.exc_view.as_ref().unwrap().scene.element(&made.id).is_some(), "redo brings it back");
    app.key(key("Ctrl+Y"));
    let redone = app.exc_view.as_ref().unwrap().scene.element(&made.id).cloned().unwrap();
    assert!(redone.x > at.0 && redone.y > at.1, "redo replays the move: {redone:?}");

    // Undo reaches the file too: what the user sees is what is saved.
    app.key(key("Ctrl+Z"));
    assert_eq!(saved(&d).elements.len(), before + 1, "the move is undone on disk");
    app.key(key("Ctrl+Z"));
    assert_eq!(saved(&d).elements.len(), before, "and so is the drawing");
}

#[test]
fn a_text_is_typed_on_a_double_click_and_the_tab_writes_the_file_when_it_is_left() {
    let (d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    app.click(&f, &mut fonts, embed.x + 40.0, embed.y + 40.0, false);
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);

    // Text tool, double click on empty surface: a box to type into.
    app.run("exc:tool:text");
    let (x, y) = (r.x + 260.0, r.y + 360.0);
    app.click(&f, &mut fonts, x, y, false);
    app.click(&f, &mut fonts, x, y, false);
    assert!(app.exc_view.as_ref().unwrap().editing.is_some(), "a text is being typed");
    for c in "area = 4".chars() {
        app.text_input(&c.to_string());
    }
    app.key(key("Escape"));
    let v = app.exc_view.as_ref().unwrap();
    let text = v.scene.elements.iter().find(|e| e.text().contains("area = 4")).expect("the text is there");
    assert_eq!(text.kind, ElementType::Text);

    // Leaving the tab for the note writes the board: `Ctrl+S` is not the only way out.
    app.run("tab:0");
    assert!(!app.exc_active() && app.tab().is_some(), "the note has the pane again");
    let on_disk = saved(&d);
    assert!(
        on_disk.elements.iter().any(|e| e.kind == ElementType::Text && e.text().contains("area = 4")),
        "the text reached the file: {:?}",
        on_disk.elements.iter().map(|e| e.text()).collect::<Vec<_>>()
    );
    assert!(on_disk.display_list().ops.len() >= 2);
}

#[test]
fn the_embed_is_painted_on_the_theme_surface_and_not_in_white() {
    let (_d, mut app, mut fonts) = open_note();
    app.dark = true;
    let f = frame(&mut app, &mut fonts);
    let it = &f.info.exc[0];
    let draw = &f.dom.nodes[it.node].draw;
    // The page is the theme's soft grey (a mix of the theme colours), never the file's `#ffffff`.
    let page = draw.iter().find_map(|c| match c {
        Cmd::Fill { color, .. } if color.a == 255 => Some(*color),
        _ => None,
    });
    let page = page.expect("the board paints a page");
    assert_ne!(page, Color::rgb(255, 255, 255), "a dark note never gets a white board");
    assert!(page.r < 90 && page.g < 100, "the page is the dark surface: {page:?}");
    // And the surface carries the dot grid, the same one the editor paints.
    assert!(
        draw.iter().any(|c| matches!(c, Cmd::Circles { items, .. } if items.len() > 20)),
        "the grid is on the embed"
    );
}

#[test]
fn the_toolbar_folds_away_and_the_fullscreen_view_is_the_whole_window() {
    let (_d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    app.click(&f, &mut fonts, embed.x + 40.0, embed.y + 40.0, false);
    let f = frame(&mut app, &mut fonts);
    let has = |f: &Frame, class: &str| f.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == class));
    assert!(has(&f, "exc-toolbar"));
    app.run("exc:tools");
    let f = frame(&mut app, &mut fonts);
    assert!(!has(&f, "exc-toolbar"), "the toolbar is folded away");
    assert!(has(&f, "exc-tools-handle"), "and a handle brings it back");
    app.run("exc:tools");
    let f = frame(&mut app, &mut fonts);
    assert!(has(&f, "exc-toolbar"), "one press and it is back");

    // Full screen covers the window, not a box inside it, and its own button gives it back.
    let small = pane(&f);
    app.run("exc:fullscreen");
    let f = frame(&mut app, &mut fonts);
    let backdrop = f
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "exc-fullscreen"))
        .expect("the full-screen backdrop");
    assert!((backdrop.rect.w - app.window_w).abs() < 2.0, "{:?} in {}", backdrop.rect, app.window_w);
    assert!(backdrop.rect.h >= 890.0, "{:?} of 900", backdrop.rect);
    let full = f.dom.nodes[*f.info.excview.last().unwrap()].rect;
    assert!(full.w > small.w && full.h > small.h, "{full:?} vs {small:?}");
    assert!(has(&f, "exc-fullscreen-exit"), "the minimize button is in the view");
    app.run("exc:fullscreen-close");
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.excview.len(), 1);
}

#[test]
fn the_embed_of_a_note_announces_itself_and_a_press_opens_the_tab() {
    let (_d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    // The pointer over the embed is a pointer over an action, and the status bar says what it does.
    assert!(app.hover(&f, embed.x + 10.0, embed.y + 10.0));
    assert_eq!(app.hover_action.as_deref(), Some("exc:open:Tablero.excalidraw"));
    let f = frame(&mut app, &mut fonts);
    let hint = texts(&f, "status-bar-hint").join(" ");
    assert!(hint.contains("Abrir tablero"), "the hint: {hint:?}");
    assert!(hint.contains("Tablero.excalidraw"), "the hint names the file: {hint:?}");
}

#[test]
fn every_shape_tool_draws_its_own_element() {
    let (_d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    app.click(&f, &mut fonts, embed.x + 40.0, embed.y + 40.0, false);
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    // The zoom is whatever the fit gave the board: the mapping from the pointer to the scene has
    // to hold at that zoom, which is the one the user gets when the tab opens.
    let zoom = app.exc_view.as_ref().unwrap().zoom;
    for (i, (tool, kind)) in [
        ("rect", ElementType::Rectangle),
        ("ellipse", ElementType::Ellipse),
        ("diamond", ElementType::Diamond),
        ("arrow", ElementType::Arrow),
        ("line", ElementType::Line),
        ("freedraw", ElementType::Freedraw),
    ]
    .into_iter()
    .enumerate()
    {
        let before = app.exc_view.as_ref().unwrap().scene.elements.len();
        app.run(&format!("exc:tool:{tool}"));
        let (x, y) = (r.x + 90.0 + i as f32 * 20.0, r.y + 90.0);
        app.click(&f, &mut fonts, x, y, false);
        app.drag(&f, &mut fonts, x + 80.0, y + 50.0);
        app.release();
        let v = app.exc_view.as_ref().unwrap();
        assert_eq!(v.scene.elements.len(), before + 1, "{tool} draws an element");
        let made = v.scene.elements.last().unwrap();
        assert_eq!(made.kind, kind, "{tool}");
        assert!(
            (made.width as f32 * zoom - 80.0).abs() < 2.0,
            "{tool}: {} scene units at {zoom} is not 80 px",
            made.width
        );
        assert!(made.width > 0.0 && made.height > 0.0, "{tool} has a box");
    }
}

#[test]
fn the_rubber_band_selects_everything_it_touches_and_delete_takes_it_away() {
    let (_d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    app.click(&f, &mut fonts, embed.x + 40.0, embed.y + 40.0, false);
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let before = app.exc_view.as_ref().unwrap().scene.elements.len();
    assert!(before >= 2, "the fixture has a text and a box");

    // Select tool on empty surface, dragged over the whole board: everything is selected.
    app.run("exc:tool:select");
    app.click(&f, &mut fonts, r.x + 4.0, r.y + 4.0, false);
    app.drag(&f, &mut fonts, r.x + r.w - 4.0, r.y + r.h - 4.0);
    app.release();
    let v = app.exc_view.as_ref().unwrap();
    assert!(v.selected.is_some(), "a primary element");
    assert_eq!(v.ids().len(), before, "and every element of the board: {:?}", v.ids());

    // Supr takes the whole selection, in one step.
    app.key(key("Delete"));
    assert_eq!(app.exc_view.as_ref().unwrap().scene.elements.len(), 0);
    assert!(app.exc_view.as_ref().unwrap().ids().is_empty(), "nothing stays selected");
    // And one Ctrl+Z brings all of it back.
    app.key(key("Ctrl+Z"));
    assert_eq!(app.exc_view.as_ref().unwrap().scene.elements.len(), before);

    // Escape drops the selection without touching the board.
    app.run("editor:select-all");
    assert_eq!(app.exc_view.as_ref().unwrap().ids().len(), before);
    app.key(key("Escape"));
    assert!(app.exc_view.as_ref().unwrap().ids().is_empty());
    assert_eq!(app.exc_view.as_ref().unwrap().scene.elements.len(), before);
}

#[test]
fn a_just_drawn_element_fades_in_and_reduced_motion_skips_the_fade() {
    let (_d, mut app, mut fonts) = open_note();
    let f = frame(&mut app, &mut fonts);
    let embed = f.dom.content_rect(f.info.exc[0].node);
    app.click(&f, &mut fonts, embed.x + 40.0, embed.y + 40.0, false);
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let faded = |f: &Frame| {
        f.dom.nodes[*f.info.excview.first().unwrap()]
            .draw
            .iter()
            .any(|c| matches!(c, Cmd::Fill { color, .. } if color.a < 250) )
    };
    // The animation clock on: the first frame of the new shape is dimmer than the final one.
    app.anims.enabled = true;
    app.run("exc:tool:rect");
    app.run("exc:fill:3");
    let (x, y) = (r.x + 120.0, r.y + 120.0);
    app.click(&f, &mut fonts, x, y, false);
    app.drag(&f, &mut fonts, x + 100.0, y + 80.0);
    app.release();
    let f = frame(&mut app, &mut fonts);
    assert!(faded(&f), "the shape fades in");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let f = frame(&mut app, &mut fonts);
    assert!(!faded(&f), "and it is fully there when the clock is done");

    // Reduced motion: the clock is off, so the shape is there at once.
    app.anims.enabled = false;
    let (x, y) = (r.x + 300.0, r.y + 300.0);
    app.click(&f, &mut fonts, x, y, false);
    app.drag(&f, &mut fonts, x + 90.0, y + 60.0);
    app.release();
    let f = frame(&mut app, &mut fonts);
    assert!(!faded(&f), "no animation with reduced motion");
}
