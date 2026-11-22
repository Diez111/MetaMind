//! Timing probe against a real vault: which phase eats the time when it opens.
//!
//! `cargo run --release -p metamind --example vault_probe -- <vault> [--expand]`
//!
//! `--expand` reproduces the user's session: a note open with the backlinks panel
//! visible, then every folder of the explorer opened one click at a time, timing
//! each click's rebuild + frame.

use metamind_app::{render, App};
use metamind_ui::{Fonts, Pixmap};
use std::path::Path;
use std::time::Instant;

fn ms(t: std::time::Duration) -> f64 {
    t.as_secs_f64() * 1000.0
}

fn main() {
    let arg = std::env::args().nth(1).expect("uso: vault_probe <vault> [--expand]");
    let root = Path::new(&arg);
    let expand = std::env::args().any(|a| a == "--expand");
    println!("vault: {}", root.display());

    let t = Instant::now();
    let (store, stats) = vault_core::VaultStore::open(root).unwrap();
    println!(
        "store.open   {:>8.1} ms  (walk {:.1} · cache {:.1} · parse {:.1} · reusadas {} · parseadas {})",
        ms(t.elapsed()),
        ms(stats.walk),
        ms(stats.load_cache),
        ms(stats.parse),
        stats.reused,
        stats.parsed,
    );
    drop(store);

    let t = Instant::now();
    let mut app = App::open(root).unwrap();
    println!("App::open    {:>8.1} ms", ms(t.elapsed()));

    let t = Instant::now();
    app.refresh_derived();
    println!(
        "derived      {:>8.1} ms  (grafo de links + árbol · notas {})",
        ms(t.elapsed()),
        app.store.index.notes().count(),
    );

    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1600, 900).unwrap();
    let t = Instant::now();
    let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
    println!("frame 1      {:>8.1} ms", ms(t.elapsed()));

    let t = Instant::now();
    for _ in 0..50 {
        app.refresh_derived();
        let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
    }
    println!("frame medio  {:>8.2} ms  (50 ciclos)", ms(t.elapsed()) / 50.0);

    if expand {
        // The user's session: a note open, the right panel showing backlinks.
        let note = "Facultad/Materias/1° Año/Análisis Matemático I/Unidades/U4c-Taylor.md";
        if app.store.index.id(note).is_some() {
            app.open_note(note);
        } else {
            let any = app.store.index.notes().next().map(|(_, f)| f.path.clone()).unwrap_or_default();
            app.open_note(&any);
        }
        app.right_open = true;
        app.refresh_derived();
        let _ = render(&mut app, &mut fonts, &mut pm, 1.0);

        // Open every folder, one click at a time, as the user does.
        let mut worst = 0.0f64;
        let mut clicks = 0usize;
        let mut total = 0.0f64;
        loop {
            let closed: Vec<String> = app
                .tree_rows()
                .iter()
                .filter(|r| r.folder && !r.collapsed || r.folder && !app.folder_open(&r.path))
                .map(|r| r.path.clone())
                .collect();
            let next: Vec<String> = app
                .tree_rows()
                .iter()
                .filter(|r| r.folder && !app.folder_open(&r.path))
                .map(|r| r.path.clone())
                .collect();
            let _ = closed;
            if next.is_empty() {
                break;
            }
            for f in next {
                let t = Instant::now();
                app.run(&format!("toggle-folder:{f}"));
                app.refresh_derived();
                let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
                let d = ms(t.elapsed());
                clicks += 1;
                total += d;
                worst = worst.max(d);
            }
        }
        println!(
            "expandir     {:>8.2} ms promedio por carpeta · peor {:.1} ms · {} carpetas (click+rebuild+frame)",
            total / clicks.max(1) as f64,
            worst,
            clicks
        );
        println!("filas visibles al final: {}", app.tree_rows().len());

        // Steady state with everything open and a note showing.
        let t = Instant::now();
        for _ in 0..50 {
            app.refresh_derived();
            let _ = render(&mut app, &mut fonts, &mut pm, 1.0);
        }
        println!("frame medio  {:>8.2} ms  (todo expandido + backlinks)", ms(t.elapsed()) / 50.0);
    }
}
