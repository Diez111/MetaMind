//! Per-folder expand timing through the REAL click path: hit-test on the rendered
//! frame, App::click at the row's coordinates, then rebuild + render, at the user's
//! window size — plus the cost of a HiDPI (2x) raster, which is what a scaled
//! display pays per frame.
//!
//! `cargo run --release -p metamind --example expand_bench -- <vault> [note]`

use metamind_app::{render, App};
use metamind_ui::{Fonts, Pixmap};
use std::time::{Duration, Instant};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Average over `n` frames of the same still window, with the damage tracker reset first (a full
/// repaint every frame, what the app did before) or left alone (dirty rects).
fn bench_frames(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap, scale: f32, n: usize, full: bool) -> f64 {
    // One frame outside the measurement so both modes start from the same list.
    let _ = render(app, fonts, pm, scale);
    let t = Instant::now();
    for _ in 0..n {
        if full {
            app.damage.reset();
        }
        app.refresh_derived();
        let _ = render(app, fonts, pm, scale);
    }
    ms(t.elapsed()) / n as f64
}

fn main() {
    let vault = std::env::args().nth(1).expect("vault");
    let note = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "Facultad/Materias/1° Año/Análisis Matemático I/Unidades/U4c-Taylor.md".into());
    let (w, h) = (1920u32, 1080u32);
    let mut app = App::open(std::path::Path::new(&vault)).unwrap();
    app.window_w = w as f32;
    app.window_h = h as f32;
    if app.store.index.id(&note).is_some() {
        app.open_note(&note);
    }
    app.right_open = true;
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(w, h).unwrap();
    app.refresh_derived();
    let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
    println!("ventana {w}x{h} · escala 1.0 · nota abierta + panel derecho");

    // Baseline frame at 1x, both paths of the renderer: a full repaint per frame (what the app
    // did before the damage tracker) and dirty-rect painting of the same still window.
    let full1 = bench_frames(&mut app, &mut fonts, &mut pm, 1.0, 20, true);
    let dmg1 = bench_frames(&mut app, &mut fonts, &mut pm, 1.0, 20, false);
    println!(
        "frame 1x    {:>7.2} ms completo · {:>7.2} ms con daño (quieto, nada que repintar)",
        full1, dmg1
    );

    // The HiDPI bill: the same frame at 2x (a 200 % display rasters 4x the pixels).
    let mut pm2 = Pixmap::new(w * 2, h * 2).unwrap();
    let full2 = bench_frames(&mut app, &mut fonts, &mut pm2, 2.0, 20, true);
    let dmg2 = bench_frames(&mut app, &mut fonts, &mut pm2, 2.0, 20, false);
    println!("frame 2x    {:>7.2} ms completo · {:>7.2} ms con daño (pantalla escala 200%)", full2, dmg2);

    // The status-bar hint: hovering a row repaints the row and the bar, not the window.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let row = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "nav-file-title"));
    if let Some(r) = row {
        let (x, y) = (r.rect.x + r.rect.w / 2.0, r.rect.y + r.rect.h / 2.0);
        app.hover(&frame, x, y);
        let t = Instant::now();
        let n = 20;
        let mut area = 0u64;
        let mut fulls = 0;
        let mut rects = 0usize;
        for i in 0..n {
            // Alternate between two rows so every frame really changes the hint.
            let other = if i % 2 == 0 { y + r.rect.h } else { y };
            app.hover(&frame, x, other);
            let f = render(&mut app, &mut fonts, &mut pm, 1.0);
            area += f.damage.area(w, h);
            fulls += usize::from(f.damage.full);
            rects += f.damage.rects.len();
            if std::env::var_os("BENCH_DEBUG").is_some() && i < 3 {
                eprintln!("  [hover {i}] full={} rects={:?}", f.damage.full, f.damage.rects);
            }
        }
        println!(
            "hover/hint  {:>7.2} ms promedio (daño {:>5.1}% de la ventana · {} rects · {} de {n} completos)",
            ms(t.elapsed()) / n as f64,
            area as f64 / (n as f64) * 100.0 / (f64::from(w) * f64::from(h)),
            rects / n,
            fulls
        );
    }

    // The real click: find a closed folder row, press at its chevron, full path.
    let mut results: Vec<(f64, String, usize)> = Vec::new();
    loop {
        let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
        let row = frame
            .dom
            .nodes
            .iter()
            .find(|n| {
                n.el.classes.iter().any(|c| c == "nav-folder-title")
                    && n.el.action.as_deref().is_some_and(|a| a.starts_with("toggle-folder:"))
                    && n.rect.y + n.rect.h < app.window_h
            })
            .map(|n| (n.rect.x + 10.0, n.rect.y + n.rect.h / 2.0, n.rect.h as usize))
            .filter(|(x, y, _)| *x >= 0.0 && *y < frame.dom.nodes[0].rect.h);
        let Some((x, y, _h)) = row else { break };
        let name = frame
            .dom
            .action_at(x, y)
            .and_then(|(_, a)| a.strip_prefix("toggle-folder:"))
            .unwrap_or("?")
            .to_string();
        let t = Instant::now();
        app.click(&frame, &mut fonts, x, y, false);
        app.refresh_derived();
        let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
        results.push((ms(t.elapsed()), name, 0));
        if results.len() > 400 {
            break;
        }
    }
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let total: f64 = results.iter().map(|r| r.0).sum();
    println!("\nclicks reales (hit-test + toggle + rebuild + render 1x): {} carpetas", results.len());
    println!(
        "promedio {:>6.2} ms · p95 {:>6.2} ms · total {:.0} ms",
        total / results.len() as f64,
        {
            let n = results.len();
            results[n * 95 / 100].0
        },
        total
    );
    println!("peor 8:");
    for (t, name, _) in results.iter().take(8) {
        println!("  {:>7.2} ms  {}", t, name);
    }
    bench_clicks_full(&mut app, &mut fonts, &mut pm);
}

/// The same clicks with a full repaint per frame, so the two paths are measured side by side.
fn bench_clicks_full(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap) {
    let mut worst: Vec<(f64, String)> = Vec::new();
    let mut total = 0.0;
    loop {
        let frame = render(app, fonts, pm, 1.0);
        let Some(row) = frame.dom.nodes.iter().find(|n| {
            n.el.classes.iter().any(|c| c == "nav-folder-title")
                && n.el.action.as_deref().is_some_and(|a| a.starts_with("toggle-folder:"))
                && n.rect.y + n.rect.h < app.window_h
        }) else {
            break;
        };
        let (x, y) = (row.rect.x + 10.0, row.rect.y + row.rect.h / 2.0);
        let name = frame
            .dom
            .action_at(x, y)
            .and_then(|(_, a)| a.strip_prefix("toggle-folder:"))
            .unwrap_or("?")
            .to_string();
        let t = Instant::now();
        app.click(&frame, fonts, x, y, false);
        app.refresh_derived();
        app.damage.reset();
        let _ = render(app, fonts, pm, 1.0);
        let dt = ms(t.elapsed());
        total += dt;
        worst.push((dt, name));
        if worst.len() > 400 {
            break;
        }
    }
    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!("\nmismos clicks con repintado completo: {} carpetas", worst.len());
    println!(
        "promedio {:>6.2} ms · p95 {:>6.2} ms · total {:.0} ms",
        total / worst.len() as f64,
        {
            let n = worst.len();
            worst[n * 95 / 100].0
        },
        total
    );
    println!("peor 8:");
    for (t, name) in worst.iter().take(8) {
        println!("  {:>7.2} ms  {}", t, name);
    }
}
