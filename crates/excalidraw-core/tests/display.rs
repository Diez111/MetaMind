//! What the fixtures paint. The dumps are committed here, next to the fixtures, so a change in the
//! display list shows up in the diff.

use excalidraw_core::{DisplayList, DrawOp, Rgba, Scene, Shape, Style};

const AREAS: &str = include_str!("fixtures/Tablero-Areas.excalidraw");
const SERIES: &str = include_str!("fixtures/Tablero-Series.excalidraw");
const KINDS: &str = include_str!("fixtures/kinds.excalidraw");

fn list(src: &str) -> DisplayList {
    Scene::parse(src).unwrap().display_list()
}

#[test]
fn tablero_areas() {
    let list = list(AREAS);
    assert_eq!(list.skipped, vec![], "the whole board is paintable");
    assert_eq!(list.background, Some(Rgba { r: 255, g: 255, b: 255, a: 255 }));
    assert_eq!(
        list.ops.iter().filter(|op| matches!(op, DrawOp::TextRun(_))).count(),
        7,
        "a title, a hint line, four exercises and the placeholder"
    );
    assert_eq!(
        list.dump(),
        r##"background #ffffff
text 900x40 at 60,40 angle=0 size=32 line=1.25 font=hand align=left/top colour=#1971c2 opacity=1 "Áreas y TFC — tablero (dibujá la región antes de integrar)"
text 900x25 at 60,100 angle=0 size=16 line=1.25 font=hand align=left/top colour=#868e96 opacity=1 "Pasos: graficar · TODAS las intersecciones · quién está arriba · integrar (arriba−abajo) · chequear signo"
text 900x25 at 60,150 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "D1.  ∫₀² |x−1| dx"
text 900x25 at 60,184 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "D5.  área entre y = x³ − x y el eje x"
text 900x25 at 60,218 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "D6.  hallar a > 0 con área 36 entre y = 2x−6 y el eje x en [0; a]"
text 900x25 at 60,252 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "D4.  d/dx ∫₀^(x²) sen t dt"
rect 900x620 at 60,306 angle=0 stroke=#1e1e1e/2/solid fill=#f8f9fa/solid opacity=1 sketchy
text 400x20 at 80,316 angle=0 size=14 line=1.25 font=hand align=left/top colour=#adb5bd opacity=1 "Resolvé acá"
"##
    );
}

#[test]
fn tablero_series() {
    let list = list(SERIES);
    assert_eq!(list.skipped, vec![]);
    assert_eq!(list.dump(), SERIES_DUMP);
}

/// The one drawing box of a board, with its style.
fn the_rect(list: &DisplayList) -> (Shape, Style) {
    let mut rects = list.ops.iter().filter_map(|op| match op {
        DrawOp::Rect(shape, style) => Some((*shape, style.clone())),
        _ => None,
    });
    let rect = rects.next().expect("each board has a drawing box");
    assert!(rects.next().is_none(), "and only one");
    rect
}

#[test]
fn the_two_boards_share_their_frame() {
    let (areas, series) = (the_rect(&list(AREAS)), the_rect(&list(SERIES)));
    assert_eq!((areas.0.width, areas.0.height, areas.0.x, areas.0.y), (900.0, 620.0, 60.0, 306.0));
    assert_eq!((series.0.width, series.0.height, series.0.x, series.0.y), (900.0, 620.0, 60.0, 272.0));
    assert_eq!(areas.1, series.1, "same stroke, fill, width and roughness");

    // Both headers use the same blue at the same size.
    for list in [list(AREAS), list(SERIES)] {
        let header = list
            .ops
            .iter()
            .find_map(|op| match op {
                DrawOp::TextRun(t) if t.size == 32.0 => Some(t.clone()),
                _ => None,
            })
            .expect("a title");
        assert_eq!(header.colour, Some(Rgba { r: 0x19, g: 0x71, b: 0xc2, a: 255 }));
        assert_eq!((header.x, header.width, header.height), (60.0, 900.0, 40.0));
    }
}

#[test]
fn kinds_dump() {
    let list = list(KINDS);
    assert_eq!(
        list.dump(),
        r##"background #ffffff
rect 260x120 at 40,60 angle=0 stroke=#1971c2/2/solid fill=#a5d8ff/solid opacity=1 sketchy
text 220x60 at 60,90 angle=0 size=20 line=1.25 font=normal align=center/middle colour=#1971c2 opacity=1 "Etiqueta\nrectángulo"
diamond 200x140 at 360,60 angle=0 stroke=#e03131/4/dashed fill=#ffc9c9/hachure opacity=1 sketchy
ellipse 180x100 at 640,80 angle=0.25 stroke=#2f9e44/1/solid fill=none opacity=0.5
text 300x60 at 380,300 angle=0.5 size=28 line=1.35 font=code align=right/top colour=#f08c00 opacity=1 "Texto rotado"
polyline (300,140) (330,150) (360,130) heads=none:arrow stroke=#e03131/2/solid fill=none opacity=1 sketchy
text 60x20 at 310,118 angle=0 size=16 line=1.25 font=hand align=center/top colour=#e03131 opacity=1 "flecha"
polyline (40,300) (240,300) heads=none:none stroke=#1e1e1e/2/dotted fill=none opacity=1 sketchy
polyline (40,380) (50,400) (65,420) (80,410) heads=none:none stroke=#7048e8/3/solid fill=none opacity=1 sketchy
image 120x120 at 700,300 angle=0 file=file-red mime=image/png opacity=1
skip gone deleted
skip frame-a type=frame
"##
    );
}

#[test]
fn rotation_is_carried_and_changes_nothing_else() {
    let scene = Scene::parse(KINDS).unwrap();
    let text = scene.display_list().ops.iter().find_map(|op| match op {
        DrawOp::TextRun(t) if t.text == "Texto rotado" => Some(t.clone()),
        _ => None,
    });
    let text = text.expect("the rotated text element");
    assert_eq!(text.angle, 0.5);
    assert_eq!((text.x, text.y, text.width, text.height), (380.0, 300.0, 300.0, 60.0));

    // The ellipse is the only op whose style asks for 50% opacity; its angle survives too.
    let ellipse = scene.display_list().ops.iter().find_map(|op| match op {
        DrawOp::Ellipse(shape, style) => Some((*shape, style.clone())),
        _ => None,
    });
    let (shape, style) = ellipse.expect("the ellipse");
    assert_eq!(shape.angle, 0.25);
    assert_eq!(style.opacity, 0.5);
}

/// The second board is kept as a constant below because its dump shares no line with the first.
const SERIES_DUMP: &str = r##"background #ffffff
text 900x40 at 60,40 angle=0 size=32 line=1.25 font=hand align=left/top colour=#1971c2 opacity=1 "Series de potencias — tablero"
text 900x25 at 60,100 angle=0 size=16 line=1.25 font=hand align=left/top colour=#868e96 opacity=1 "4 pasos: 1) a_n (x−c)^n  2) R = 1/L  3) (c−R; c+R)  4) los dos extremos"
text 900x25 at 60,150 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "A1.  Σ x^n / (n·3^n)          → radio, intervalo y los DOS extremos"
text 900x25 at 60,184 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "A5.  Σ (x−1)^n / (2^n·√n)     → idem (ojo: los extremos pueden dar distinto)"
text 900x25 at 60,218 angle=0 size=18 line=1.25 font=hand align=left/top colour=#1e1e1e opacity=1 "A6.  Σ (x−1)^n / (k^n·n)  con R = 3     → hallar k > 0"
rect 900x620 at 60,272 angle=0 stroke=#1e1e1e/2/solid fill=#f8f9fa/solid opacity=1 sketchy
text 400x20 at 80,282 angle=0 size=14 line=1.25 font=hand align=left/top colour=#adb5bd opacity=1 "Resolvé acá"
"##;
