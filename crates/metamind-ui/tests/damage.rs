//! The dirty-rect renderer on its own: a synthetic display list that uses every command kind, the
//! pixels a partial repaint leaves versus a full one, and what the diff costs against the frame it
//! saves.
use metamind_ui::scene::Cmd;
use metamind_ui::{Color, DamageTracker, DevRect, Fonts, Pixmap, Rect, Scene, TextStyle};

const W: u32 = 900;
const H: u32 = 600;
const CLEAR: Color = Color { r: 24, g: 24, b: 27, a: 255 };

fn fill(x: f32, y: f32, w: f32, h: f32, color: Color, radius: f32) -> Cmd {
    Cmd::Fill { rect: Rect::new(x, y, w, h), color, radius }
}

fn text(x: f32, y: f32, t: &str, width: Option<f32>, size: f32) -> Cmd {
    Cmd::Text {
        x,
        y,
        width,
        text: t.to_string(),
        spans: Vec::new(),
        style: TextStyle {
            size,
            line_height: size * 1.35,
            color: Color { r: 220, g: 221, b: 225, a: 255 },
            bold: false,
            italic: false,
            mono: false,
            wrap: width.is_some(),
            family: None,
        },
    }
}

/// A scene that exercises every command kind and a nested clip, the way a note pane does.
fn scene(rows: usize, typed: Option<char>) -> Scene {
    let mut s = Scene::default();
    s.cmds.push(fill(0.0, 0.0, W as f32, H as f32, CLEAR, 0.0));
    s.cmds.push(fill(0.0, 0.0, 220.0, H as f32, Color { r: 38, g: 38, b: 44, a: 255 }, 0.0));
    for i in 0..rows {
        let y = 8.0 + i as f32 * 22.0;
        if y > H as f32 {
            break;
        }
        let active = i == 3;
        if active {
            s.cmds.push(fill(6.0, y - 2.0, 200.0, 20.0, Color { r: 124, g: 92, b: 240, a: 255 }, 4.0));
        }
        s.cmds.push(text(12.0, y, &format!("Nota {i} del explorador"), None, 13.0));
    }
    // The note pane: a clip, wrapped prose, an image, lines and bullets.
    s.cmds.push(Cmd::PushClip(Rect::new(220.0, 0.0, W as f32 - 220.0, H as f32)));
    for i in 0..rows {
        let y = 10.0 + i as f32 * 19.0;
        if y > H as f32 {
            break;
        }
        let line = match typed {
            Some(c) if i == 5 => format!("linea {i} con el caracter {c} escrito al final"),
            _ => format!("linea {i} de la nota, con prosa suficiente para llenar el ancho"),
        };
        s.cmds.push(text(240.0, y, &line, Some(W as f32 - 260.0), 14.0));
    }
    s.cmds.push(Cmd::PopClip);
    s.cmds.push(Cmd::Lines {
        segs: (0..40).map(|i| [240.0, 20.0 + i as f32 * 6.0, 700.0, 40.0 + i as f32 * 6.0]).collect(),
        width: 1.0,
        color: Color { r: 90, g: 90, b: 100, a: 120 },
    });
    s.cmds.push(Cmd::Circles {
        items: (0..20).map(|i| [760.0 + i as f32, 40.0 + i as f32 * 9.0, 3.0]).collect(),
        color: Color { r: 220, g: 180, b: 90, a: 200 },
    });
    s.cmds.push(fill(224.0, H as f32 - 26.0, W as f32 - 224.0, 26.0, Color { r: 30, g: 30, b: 36, a: 255 }, 0.0));
    s.cmds.push(text(240.0, H as f32 - 20.0, "1.234 palabras · 12 min de lectura", None, 12.0));
    s
}

/// How many pixels differ and where, instead of dumping two megapixels into the test output.
fn differing(a: &Pixmap, b: &Pixmap) -> (usize, Option<(u32, u32, u32, u32)>) {
    let (w, h) = (a.width(), a.height());
    let (da, db) = (a.data(), b.data());
    let (mut n, mut bbox) = (0usize, None::<(u32, u32, u32, u32)>);
    for y in 0..h {
        for x in 0..w {
            let o = (y as usize * w as usize + x as usize) * 4;
            if da[o..o + 4] != db[o..o + 4] {
                n += 1;
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    (n, bbox)
}

#[test]
fn a_partial_repaint_leaves_the_pixels_of_a_full_one() {
    let mut fonts = Fonts::new();
    let mut full = Pixmap::new(W, H).unwrap();
    let mut damaged = Pixmap::new(W, H).unwrap();
    let mut tracker = DamageTracker::new();
    // The same sequence of frames the app produces: a first frame, a still one, typing, a hover
    // highlight moving, and a change that inserts commands (rows) instead of replacing them.
    let frames = [
        scene(60, None),
        scene(60, None),
        scene(60, Some('x')),
        scene(60, Some('y')),
        scene(20, Some('y')),
        scene(60, Some('y')),
    ];
    for (i, s) in frames.iter().enumerate() {
        metamind_ui::render(s, &mut fonts, &mut full, 1.0, CLEAR);
        metamind_ui::render_damaged(s.clone(), &mut fonts, &mut damaged, 1.0, CLEAR, &mut tracker);
        let (pixels, bbox) = differing(&full, &damaged);
        assert_eq!(pixels, 0, "frame {i}: {pixels} pixels differ from a full repaint, bbox {bbox:?}");
    }
}

/// Whether the run is the `METAMIND_FULL_PAINT` fallback, where there is no diff to measure.
fn forced_full() -> bool {
    std::env::var_os("METAMIND_FULL_PAINT").is_some_and(|v| v != "0" && !v.is_empty())
}

#[test]
fn the_diff_is_cheaper_than_the_frame_it_saves() {
    if forced_full() {
        return;
    }
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(W, H).unwrap();
    let mut tracker = DamageTracker::new();
    let a = scene(200, None);
    let b = scene(200, Some('x'));
    // Warm up both paths so caches and the thread pool are out of the picture.
    for _ in 0..3 {
        metamind_ui::render(&b, &mut fonts, &mut pm, 1.0, CLEAR);
        metamind_ui::render_damaged(a.clone(), &mut fonts, &mut pm, 1.0, CLEAR, &mut tracker);
        metamind_ui::render_damaged(b.clone(), &mut fonts, &mut pm, 1.0, CLEAR, &mut tracker);
    }
    let mut full_us = 0u128;
    let mut diff_us = 0u128;
    let mut paint_us = 0u128;
    let n = 10;
    for _ in 0..n {
        let t = std::time::Instant::now();
        metamind_ui::render(&b, &mut fonts, &mut pm, 1.0, CLEAR);
        full_us += t.elapsed().as_micros();
        // A still frame: nothing to paint, so what is left is the diff itself.
        let d = metamind_ui::render_damaged(b.clone(), &mut fonts, &mut pm, 1.0, CLEAR, &mut tracker);
        diff_us += d.diff_us as u128;
        paint_us += d.paint_us as u128;
        assert!(d.is_empty(), "a still frame asked for a repaint");
    }
    let (full, diff, paint) = (full_us / n, diff_us / n, paint_us / n);
    println!("frame completo {full} µs · diff {diff} µs · pintura de un frame quieto {paint} µs");
    assert!(diff < full, "the diff ({diff} µs) costs more than the frame it saves ({full} µs)");
}

#[test]
fn a_dirty_rect_never_escapes_the_pixmap() {
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(W, H).unwrap();
    let mut tracker = DamageTracker::new();
    // Off-screen and oversized commands: the rects handed to the rasterizer must stay inside.
    let mut s = scene(10, None);
    s.cmds.push(fill(-500.0, -500.0, 4000.0, 4000.0, Color { r: 10, g: 20, b: 30, a: 255 }, 8.0));
    s.cmds.push(text(-200.0, -200.0, "fuera de la ventana", None, 14.0));
    metamind_ui::render_damaged(s.clone(), &mut fonts, &mut pm, 1.0, CLEAR, &mut tracker);
    s.cmds.push(fill(10.0, 10.0, 30.0, 30.0, Color { r: 200, g: 10, b: 10, a: 255 }, 0.0));
    let d = metamind_ui::render_damaged(s, &mut fonts, &mut pm, 1.0, CLEAR, &mut tracker);
    let inside = |r: &DevRect| r.x0 >= 0 && r.y0 >= 0 && r.x1 <= W as i32 && r.y1 <= H as i32;
    assert!(d.rects.iter().all(inside), "pitmap escaped: {:?}", d.rects);
    assert!(!d.is_empty(), "a new command painted nothing");
}
