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

fn main() {
    let vault = std::env::args().nth(1).expect("vault");
    let note = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "Facultad/Materias/1° Año/Análisis Matemático I/Unidades/U4c-Taylor.md".into());
    let (W, H) = (1920u32, 1080u32);
    let mut app = App::open(std::path::Path::new(&vault)).unwrap();
    app.window_w = W as f32;
    app.window_h = H as f32;
    if app.store.index.id(&note).is_some() {
        app.open_note(&note);
    }
    app.right_open = true;
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(W, H).unwrap();
    app.refresh_derived();
    let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
    println!("ventana {W}x{H} · escala 1.0 · nota abierta + panel derecho");

    // Baseline frame at 1x.
    let t = Instant::now();
    for _ in 0..20 {
        app.refresh_derived();
        let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
    }
    println!("frame 1x    {:>7.2} ms promedio", ms(t.elapsed()) / 20.0);

    // The HiDPI bill: the same frame at 2x (a 200 % display rasters 4x the pixels).
    let mut pm2 = Pixmap::new(W * 2, H * 2).unwrap();
    let t = Instant::now();
    for _ in 0..20 {
        app.refresh_derived();
        let _ = render(&mut app, &mut fonts, &mut pm2, 2.0);
    }
    println!("frame 2x    {:>7.2} ms promedio (pantalla escala 200%)", ms(t.elapsed()) / 20.0);

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
}
