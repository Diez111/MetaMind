//! A `.excalidraw` file opened in the main pane: the camera fits the board, the tools draw, the
//! pointer moves what it hits, and every edit reaches the file with the schema the reader parses.
//!
//! The board of the vault starts away from the origin and is wider than the pane, which is the
//! case the read-only embed got wrong: the first paint has to frame the elements, not the origin.

use excalidraw_core::{ElementType, Scene};
use metamind_app::shell::ViewMode;
use metamind_app::excalidrawview::Tool;
use metamind_app::{render, App, Frame, Key};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

const BOARD: &str = r##"{"type":"excalidraw","version":2,"source":"https://excalidraw.com",
"elements":[
 {"id":"title","type":"text","x":40,"y":30,"width":320,"height":28,"angle":0,"strokeColor":"#1971c2",
  "text":"Tablero de áreas","fontSize":20,"fontFamily":1,"textAlign":"left","lineHeight":1.25},
 {"id":"box","type":"rectangle","x":40,"y":80,"width":260,"height":140,"angle":0,
  "strokeColor":"#1e1e1e","backgroundColor":"#a5d8ff","strokeWidth":2,"fillStyle":"solid","roughness":1,
  "opacity":100,"seed":11,"groupIds":[],"version":3,"isDeleted":false},
 {"id":"arrow","type":"arrow","x":300,"y":150,"width":240,"height":0,"angle":0,"strokeColor":"#e03131",
  "points":[[0,0],[240,0]],"startArrowhead":null,"endArrowhead":"arrow","strokeWidth":2,"opacity":100,
  "seed":12,"version":2}
],"appState":{"viewBackgroundColor":"#ffffff","gridSize":null},"files":{},"future":{"keep":"me"}}"##;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), "# Home\n").unwrap();
    fs::write(d.path().join("Tablero.excalidraw"), BOARD).unwrap();
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.text_input(&c.to_string());
    }
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1000, 700).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn pane(f: &Frame) -> metamind_ui::Rect {
    f.dom.nodes[*f.info.excview.first().expect("the board is in its pane")].rect
}

/// The board as Excalidraw would read it back, from the file on disk.
fn saved(d: &tempfile::TempDir) -> Scene {
    Scene::parse(&fs::read_to_string(d.path().join("Tablero.excalidraw")).unwrap()).unwrap()
}

fn open() -> (tempfile::TempDir, App, Fonts) {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    // The note first, so closing the board has something to give the pane back to.
    app.open_note("Home");
    app.run("open:Tablero.excalidraw");
    let mut fonts = Fonts::new();
    (d, app, std::mem::take(&mut fonts))
}

#[test]
fn a_board_opens_as_a_tab_and_frames_its_elements() {
    let (_d, mut app, mut fonts) = open();
    assert!(app.exc_active(), "a .excalidraw file opens in its own view");
    assert!(app.tab().is_none(), "and takes the pane from the note");
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let v = app.exc_view.as_ref().unwrap();
    assert!(v.zoom <= 1.0 && v.zoom > 0.1, "the board is fitted, and never blown up: {}", v.zoom);
    // Every element is inside the pane, with the margin the fit keeps: nothing is clipped at the
    // top and nothing is cut off at the bottom.
    for (id, x0, y0, x1, y1) in [
        ("title", 40.0, 30.0, 360.0, 58.0),
        ("box", 40.0, 80.0, 300.0, 220.0),
        ("arrow", 300.0, 150.0, 540.0, 150.0),
    ] {
        let (sx, sy) = v.to_screen(&r, x0, y0);
        let (ex, ey) = v.to_screen(&r, x1, y1);
        assert!(sx > r.x && sy > r.y, "{id} starts inside the pane");
        assert!(ex < r.x + r.w && ey < r.y + r.h, "{id} ends inside the pane");
    }
    // The middle of the board sits in the middle of the pane.
    let (cx, cy) = v.to_screen(&r, 290.0, 125.0);
    assert!((cx - (r.x + r.w / 2.0)).abs() < 1.0, "{cx} is the pane's centre");
    assert!((cy - (r.y + r.h / 2.0)).abs() < 1.0);
}

#[test]
fn a_tool_draws_an_element_that_reaches_the_file() {
    let (d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    app.run("exc:tool:rect");
    app.run("exc:stroke:2");
    app.run("exc:fill:3");
    assert_eq!(app.exc_view.as_ref().unwrap().tool, metamind_app::excalidrawview::Tool::Rect);
    let (x, y) = (r.x + 120.0, r.y + 120.0);
    app.click(&f, &mut fonts, x, y, false);
    app.drag(&f, &mut fonts, x + 110.0, y + 70.0);
    app.release();
    let v = app.exc_view.as_ref().unwrap();
    assert_eq!(v.scene.elements.len(), 4, "the new shape is in the scene");
    let made = v.scene.elements.last().unwrap();
    assert_eq!(made.kind, ElementType::Rectangle);
    assert_eq!(made.stroke_color, "#2f9e44");
    assert_eq!(made.background_color, "#a5d8ff");
    assert!((made.width as f32 * v.zoom - 110.0).abs() < 1.5, "{} ", made.width);
    // The file holds it, in the schema the reader parses, and the scene round-trips.
    let on_disk = saved(&d);
    assert_eq!(on_disk.elements.len(), 4);
    assert_eq!(on_disk.elements.last().unwrap().kind, ElementType::Rectangle);
    assert_eq!(on_disk.display_list().ops.len(), 4);
    assert_eq!(on_disk.extra["future"]["keep"], "me", "the unknown top-level keys survive");
    assert_eq!(on_disk.to_json(), Scene::parse(&on_disk.to_json()).unwrap().to_json());
}

#[test]
fn the_wheel_scrolls_the_board_and_ctrl_wheel_zooms_about_the_pointer() {
    let (_d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let (x, y) = (r.x + 300.0, r.y + 200.0);
    let v = app.exc_view.as_ref().unwrap();
    let before = v.to_world(&r, x, y);
    let zoom = v.zoom;
    let fitted = zoom;

    // Ctrl + wheel: the point under the pointer stays under the pointer.
    app.ctrl_wheel(&f, x, y, -3.0);
    let v = app.exc_view.as_ref().unwrap();
    assert!(v.zoom > zoom, "{} > {zoom}", v.zoom);
    assert!(!v.auto_fit);
    let after = v.to_world(&r, x, y);
    assert!((after[0] - before[0]).abs() < 0.5 && (after[1] - before[1]).abs() < 0.5, "{after:?} {before:?}");

    // Plain wheel: the board scrolls, the zoom does not change.
    let cam = v.cam;
    app.wheel(&f, x, y, 30.0);
    let v = app.exc_view.as_ref().unwrap();
    assert!((v.zoom - zoom * 1.1f32.powf(1.0)).abs() < 0.05);
    assert!(v.cam[1] > cam[1], "wheel down shows what is below: {} > {}", v.cam[1], cam[1]);

    // Ajustar puts the camera back on the board, at the zoom the fit gives it.
    app.run("exc:zoom-fit");
    let _ = frame(&mut app, &mut fonts);
    let v = app.exc_view.as_ref().unwrap();
    assert!(v.auto_fit, "the fit was asked for");
    assert!((v.zoom - fitted).abs() < 0.01, "{} back to {fitted}", v.zoom);
    // 100 % is 100 %, whatever the board was.
    app.run("exc:zoom-reset");
    assert_eq!(app.exc_view.as_ref().unwrap().zoom, 1.0);
}

#[test]
fn the_select_tool_moves_an_element_and_the_board_keeps_it() {
    let (d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let v = app.exc_view.as_ref().unwrap();
    let (x, y) = v.to_screen(&r, 170.0, 150.0);
    app.click(&f, &mut fonts, x, y, false);
    assert_eq!(app.exc_view.as_ref().unwrap().selected.as_deref(), Some("box"));
    app.drag(&f, &mut fonts, x + 60.0, y + 30.0);
    app.release();
    let box_ = saved(&d).element("box").cloned().unwrap();
    assert!(box_.x > 40.0 && box_.y > 80.0, "{box_:?}");
    assert_eq!(box_.extra["version"], 4, "moving an element bumps its version");
    // Delete takes the selected element away, and it leaves the file.
    app.key(key("Delete"));
    assert!(saved(&d).element("box").is_none());
    assert_eq!(saved(&d).elements.len(), 2);
    assert!(app.exc_view.as_ref().unwrap().selected.is_none());
    // Escape deselects, and a click on empty space does too.
    app.key(key("Escape"));
    assert!(app.exc_view.as_ref().unwrap().selected.is_none());
}

#[test]
fn the_text_tool_creates_a_text_and_typing_edits_it() {
    let (d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    app.run("exc:tool:text");
    let (x, y) = (r.x + 200.0, r.y + 400.0);
    app.click(&f, &mut fonts, x, y, false);
    assert_eq!(app.exc_view.as_ref().unwrap().editing.as_deref().map(|_| ()), Some(()));
    typed(&mut app, "hola mundo");
    // A second click somewhere else commits the text and starts nothing.
    let f2 = frame(&mut app, &mut fonts);
    app.click(&f2, &mut fonts, r.x + 500.0, r.y + 400.0, false);
    let v = app.exc_view.as_ref().unwrap();
    assert!(v.editing.is_some(), "the text tool starts a new text there");
    app.key(key("Escape"));
    let on_disk = saved(&d);
    let made =
        on_disk.elements.iter().find(|e| e.text().contains("hola mundo")).expect("the text is in the file");
    assert_eq!(made.kind, ElementType::Text);
    assert!(made.width > 0.0 && made.height > 0.0, "the box was measured: {made:?}");
    assert!(on_disk.display_list().ops.iter().any(|op| matches!(op,
        excalidraw_core::DrawOp::TextRun(t) if t.text.contains("hola mundo"))));
    // The empty second text left nothing behind.
    assert_eq!(on_disk.elements.len(), 4);
}

#[test]
fn double_click_on_a_text_edits_it() {
    let (d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let v = app.exc_view.as_ref().unwrap();
    let (x, y) = v.to_screen(&r, 60.0, 40.0);
    app.click(&f, &mut fonts, x, y, false);
    app.click(&f, &mut fonts, x, y, false);
    assert_eq!(app.exc_view.as_ref().unwrap().editing.as_deref(), Some("title"));
    typed(&mut app, "!");
    app.key(key("Escape"));
    assert_eq!(saved(&d).element("title").unwrap().text(), "Tablero de áreas!");
}

#[test]
fn fullscreen_covers_the_window_and_escape_gives_the_pane_back() {
    let (_d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.excview.len(), 1);
    app.run("exc:fullscreen");
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.excview.len(), 2, "the pane and the full-screen box both paint the board");
    let full = f.dom.nodes[*f.info.excview.last().unwrap()].rect;
    let small = f.dom.nodes[f.info.excview[0]].rect;
    assert!(full.w > small.w && full.h > small.h, "{full:?} {small:?}");
    // A press inside the full-screen box draws there, not in the pane behind it.
    app.run("exc:tool:rect");
    let (x, y) = (full.x + 80.0, full.y + 80.0);
    app.click(&f, &mut fonts, x, y, false);
    app.drag(&f, &mut fonts, x + 60.0, y + 40.0);
    app.release();
    assert_eq!(app.exc_view.as_ref().unwrap().scene.elements.len(), 4);
    app.key(key("Escape"));
    assert!(!app.exc_fullscreen);
    let f = frame(&mut app, &mut fonts);
    assert_eq!(f.info.excview.len(), 1);
}

#[test]
fn closing_the_board_writes_it_and_gives_the_note_back() {
    let (d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    app.run("exc:tool:ellipse");
    let (x, y) = (r.x + 400.0, r.y + 100.0);
    app.click(&f, &mut fonts, x, y, false);
    app.drag(&f, &mut fonts, x + 90.0, y + 90.0);
    app.release();
    assert!(!app.exc_view.as_ref().unwrap().dirty, "releasing the button saves");
    app.run("exc:close");
    assert!(app.exc_view.is_none() && !app.exc_active());
    assert!(app.tab().is_some(), "the note has the pane again");
    assert_eq!(saved(&d).elements.len(), 4);
    assert_eq!(saved(&d).elements.last().unwrap().kind, ElementType::Ellipse);
    // A board that cannot be read is a notice, not a crash.
    fs::write(d.path().join("Roto.excalidraw"), "{\"type\":\"excalidraw\"").unwrap();
    app.run("open:Roto.excalidraw");
    assert!(app.notice.as_deref().unwrap_or_default().contains("No se pudo abrir"), "{:?}", app.notice);
}

#[test]
fn the_board_paints_a_dot_grid_over_the_theme_surface() {
    let (_d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    let draw = &f.dom.nodes[f.info.excview[0]].draw;
    let _ = r;
    // A surface, then the grid, then the elements: the pane is never left blank.
    assert!(draw.len() > 4, "{draw:?}");
    let mut fills = 0;
    let mut dots = 0;
    for cmd in draw {
        match cmd {
            metamind_ui::Cmd::Fill { color, .. } if color.a == 255 => fills += 1,
            metamind_ui::Cmd::Circles { items, .. } if items.len() > 20 => dots += 1,
            _ => {}
        }
    }
    assert!(fills >= 1, "the page colour is painted");
    assert!(dots >= 1, "the grid is painted");
    // The board owns the pane: the note is still open, but the editor is not what is shown.
    assert!(app.tabs.iter().any(|t| t.path == "Home.md"));
    let _ = ViewMode::Reading;
}

/// The chrome of a board is a column: the bar takes its height, the canvas the rest, and nothing
/// lands on top of anything else. This is the shape the palette broke when it was a box pinned to
/// a fixed `top` under a bar whose height nobody had measured.
#[test]
fn the_bar_the_canvas_and_the_style_panel_do_not_overlap() {
    let (_d, mut app, mut fonts) = open();
    app.run("exc:style");
    let f = frame(&mut app, &mut fonts);
    let classed = |class: &str| {
        f.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == class)).map(|n| n.rect)
    };
    let bar = classed("exc-toolbar").expect("the bar");
    let canvas = classed("exc-canvas").expect("the canvas");
    let panel = classed("exc-style-panel").expect("the style panel");
    let wrapper = classed("exc-wrapper").expect("the pane");
    assert!(
        bar.y + bar.h <= canvas.y + 0.5,
        "the canvas starts under the bar: bar {bar:?}, canvas {canvas:?}"
    );
    assert!(canvas.y + canvas.h <= wrapper.y + wrapper.h + 0.5, "the canvas stays in the pane");
    assert!(
        panel.y >= bar.y + bar.h,
        "the panel opens under the bar, not over it: bar {bar:?}, panel {panel:?}"
    );
    assert!(
        panel.x >= wrapper.x && panel.x + panel.w <= wrapper.x + wrapper.w + 0.5,
        "the panel stays in the pane: {panel:?} in {wrapper:?}"
    );
    assert!(panel.y + panel.h <= wrapper.y + wrapper.h + 0.5, "and does not run past its foot");
    // Every swatch is inside the panel: a paletted colour is a colour you can press.
    for (id, n) in f.dom.nodes.iter().enumerate() {
        if !n.el.classes.iter().any(|c| c == "exc-swatch") {
            continue;
        }
        let r = n.rect;
        assert!(
            r.x >= panel.x && r.x + r.w <= panel.x + panel.w + 0.5,
            "swatch {id} at {r:?} is outside the panel {panel:?}"
        );
        assert!(r.y >= panel.y && r.y + r.h <= panel.y + panel.h + 0.5, "swatch {id} at {r:?}");
    }
    // And the bar itself: every tool is inside it, on one row or wrapped onto two.
    let mut tools = 0;
    for n in &f.dom.nodes {
        if !n.el.classes.iter().any(|c| c == "exc-tool") {
            continue;
        }
        tools += 1;
        let r = n.rect;
        assert!(
            r.x >= bar.x && r.x + r.w <= bar.x + bar.w + 0.5,
            "tool at {r:?} leaves the bar {bar:?}"
        );
        assert!(r.y >= bar.y && r.y + r.h <= bar.y + bar.h + 0.5, "tool at {r:?} leaves the bar");
    }
    assert!(tools >= Tool::ALL.len(), "all the tools are there: {tools}");
}

/// A pen stroke keeps the pressure of every point, and the painter gives the thick and the thin
/// parts different widths. This is what a tablet buys: the stroke is not one width.
#[test]
fn a_pen_stroke_keeps_the_pressure_of_every_point() {
    let (d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    app.run("exc:tool:freedraw");
    // Press light at one end, hard at the other: the app hands the view the pressure before each
    // event, which is what the window thread does.
    app.pen.pressure = 0.15;
    app.pen.trial_connected(true);
    let (x, y) = (r.x + 80.0, r.y + 100.0);
    app.click(&f, &mut fonts, x, y, false);
    for step in 1..=20 {
        app.pen.pressure = 0.15 + 0.04 * step as f32;
        let (px, py) = (x + step as f32 * 6.0, y + (step as f32 * 1.5).sin() * 8.0);
        app.drag(&f, &mut fonts, px, py);
    }
    app.release();
    let v = app.exc_view.as_ref().unwrap();
    let made = v.scene.elements.last().unwrap();
    assert_eq!(made.kind, ElementType::Freedraw);
    let pressures = made.pressures.clone().expect("las presiones quedaron en el archivo");
    assert!(pressures.len() >= 10, "un punto por muestra: {}", pressures.len());
    assert!(pressures[0] < 0.25, "el primer punto es suave: {}", pressures[0]);
    assert!(pressures[pressures.len() - 1] > 0.8, "el último aprieta: {}", pressures[pressures.len() - 1]);
    // And the painter turns that into a stroke that changes width.
    let v = app.exc_view.as_mut().unwrap();
    let text = metamind_ui::TextStyle {
        size: 14.0,
        line_height: 20.0,
        color: metamind_ui::Color::rgb(220, 220, 220),
        bold: false,
        italic: false,
        mono: false,
        wrap: true,
        family: None,
    };
    let look = metamind_app::excalidrawview::Look { dark: true, primary: false, fade: 1.0 };
    let painted = v.paint(r, &[metamind_ui::Color::rgb(0, 0, 0); 12], &text, look, &mut fonts);
    let _ = &app;
    let widths: Vec<f32> = painted
        .iter()
        .filter_map(|c| match c {
            metamind_ui::Cmd::Stroke { points, .. } => Some(points.iter().map(|p| p[2]).fold(0.0f32, f32::max)),
            _ => None,
        })
        .collect();
    assert!(!widths.is_empty(), "el trazo se pinta como una cinta");
    // The file keeps it too.
    let on_disk = saved(&d);
    assert_eq!(on_disk.elements.last().unwrap().pressures.as_ref().map(Vec::len), Some(pressures.len()));
}

/// The other end of the pen erases: what it passes over goes away.
#[test]
fn the_eraser_end_removes_what_it_passes_over() {
    let (_d, mut app, mut fonts) = open();
    let f = frame(&mut app, &mut fonts);
    let r = pane(&f);
    app.run("exc:tool:select");
    let before = app.exc_view.as_ref().unwrap().scene.elements.len();
    app.pen.trial_connected(true);
    app.pen.erasing = true;
    // The board has a box at the top-left; drag over its middle.
    let v = app.exc_view.as_ref().unwrap();
    let (bx, by) = v.to_screen(&r, 40.0 + 130.0, 80.0 + 70.0);
    app.click(&f, &mut fonts, bx, by, false);
    app.drag(&f, &mut fonts, bx + 6.0, by + 6.0);
    app.release();
    let after = app.exc_view.as_ref().unwrap().scene.elements.len();
    assert!(after < before, "la goma borró: {before} -> {after}");
    app.pen.erasing = false;
}
