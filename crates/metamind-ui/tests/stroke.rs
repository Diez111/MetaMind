//! `Cmd::Stroke`: a polyline whose width changes from point to point.
//!
//! A mouse gives every point the same width, a stylus gives each one its own pressure. The two
//! must agree pixel for pixel when the width is constant, or a board drawn with a mouse would
//! look different from the same board drawn with a pen.

use metamind_ui::{render, Cap, Cmd, Color, Fonts, Pixmap, Rect, Scene};

const W: u32 = 120;
const H: u32 = 120;
const INK: Color = Color::rgb(255, 0, 0);
const CLEAR: Color = Color::rgb(0, 0, 0);

fn paint(cmd: Cmd) -> Pixmap {
    let scene = Scene { cmds: vec![Cmd::PushClip(Rect::new(0.0, 0.0, W as f32, H as f32)), cmd, Cmd::PopClip] };
    let mut pm = Pixmap::new(W, H).unwrap();
    let mut fonts = Fonts::new();
    render(&scene, &mut fonts, &mut pm, 1.0, CLEAR);
    pm
}

/// How many pixels of `x` are inked: the vertical thickness of the stroke there.
fn thickness(pm: &Pixmap, x: u32) -> usize {
    let data = pm.data();
    (0..H)
        .filter(|&y| {
            let o = (y as usize * W as usize + x as usize) * 4;
            data[o] > 40
        })
        .count()
}

fn inked(pm: &Pixmap) -> usize {
    let data = pm.data();
    (0..(W * H) as usize).filter(|&i| data[i * 4] > 40).count()
}

#[test]
fn a_constant_stroke_covers_the_same_pixels_as_a_line_of_the_same_width() {
    let stroke = paint(Cmd::Stroke {
        points: vec![[20.0, 60.0, 6.0], [100.0, 60.0, 6.0]],
        color: INK,
        cap: Cap::Butt,
    });
    let lines = paint(Cmd::Lines { segs: vec![[20.0, 60.0, 100.0, 60.0]], width: 6.0, color: INK });
    for x in [30, 50, 70, 90] {
        assert_eq!(thickness(&stroke, x), thickness(&lines, x), "column {x}");
    }
    // A butt cap does not paint past the ends; a round one does.
    let round = paint(Cmd::Stroke {
        points: vec![[20.0, 60.0, 6.0], [100.0, 60.0, 6.0]],
        color: INK,
        cap: Cap::Round,
    });
    assert_eq!(thickness(&stroke, 18), 0, "butt cap stops at the end point");
    assert!(thickness(&round, 18) > 0, "round cap paints a half disc there");
}

#[test]
fn the_stroke_grows_with_the_pressure_of_each_point() {
    let pm = paint(Cmd::Stroke {
        points: vec![[20.0, 60.0, 1.5], [100.0, 60.0, 12.0]],
        color: INK,
        cap: Cap::Round,
    });
    let (thin, thick) = (thickness(&pm, 22), thickness(&pm, 98));
    assert!(thin <= 4, "the first point is a thin line: {thin}");
    assert!(thick >= 9, "the last point is a wide one: {thick}");
    assert!(thick > thin * 2, "the taper is visible: {thin} -> {thick}");
}

#[test]
fn a_translucent_stroke_is_composited_once() {
    // Overlapping quads at a sharp turn must not double the alpha: the ribbon is one path fill.
    let pm = paint(Cmd::Stroke {
        points: vec![[30.0, 30.0, 10.0], [30.0, 90.0, 10.0], [90.0, 90.0, 10.0]],
        color: Color { r: 255, g: 0, b: 0, a: 128 },
        cap: Cap::Round,
    });
    let data = pm.data();
    let o = (30usize * W as usize + 30) * 4;
    // A single 50 % blend of pure red over black stays at 128, never near 192.
    assert!(data[o] <= 132, "a joint is painted once: {}", data[o]);
    assert!(data[o] >= 120, "and it is painted at all: {}", data[o]);
    let _ = inked(&pm);
}

#[test]
fn a_stroke_of_one_point_paints_nothing() {
    let pm = paint(Cmd::Stroke { points: vec![[60.0, 60.0, 8.0]], color: INK, cap: Cap::Round });
    assert_eq!(inked(&pm), 0, "a polyline needs two points");
}

#[test]
fn the_stroke_stays_inside_its_clip() {
    let scene = Scene {
        cmds: vec![
            Cmd::PushClip(Rect::new(0.0, 0.0, 50.0, 120.0)),
            Cmd::Stroke { points: vec![[10.0, 60.0, 8.0], [110.0, 60.0, 8.0]], color: INK, cap: Cap::Round },
            Cmd::PopClip,
        ],
    };
    let mut pm = Pixmap::new(W, H).unwrap();
    let mut fonts = Fonts::new();
    render(&scene, &mut fonts, &mut pm, 1.0, CLEAR);
    let data = pm.data();
    let right_half = (0..H)
        .flat_map(|y| (60..W).map(move |x| (y, x)))
        .filter(|&(y, x)| data[((y * W + x) * 4) as usize] > 40)
        .count();
    assert_eq!(right_half, 0, "nothing is painted past the clip");
    assert!(thickness(&pm, 20) > 0, "and the visible part is there");
}
