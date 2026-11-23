//! The plan as a timeline: a note with `tipo: gantt` is drawn as bars over its own dates.

use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1000, 700).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

#[test]
fn the_note_becomes_a_timeline_with_a_bar_per_task() {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("Plan.md"),
        "---\ntipo: gantt\n---\n# Plan\n\n- Diseño · 2026-11-02 · 2026-11-08\n- Implementación · 2026-11-09 · 2026-11-22\n- Entrega · 2026-11-24 · 2026-11-24\n",
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Plan.md");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("Plan"), "{texts}");
    assert!(texts.contains("Diseño") && texts.contains("Implementación"), "{texts}");
    assert!(texts.contains("2 nov 2026") || texts.contains("2 nov → 24 nov"), "el rango: {texts}");
    let bars = f.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == "gantt-bar")).count();
    assert_eq!(bars, 3, "una barra por tarea");
    let milestone = f.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == "is-milestone")).count();
    assert_eq!(milestone, 1, "un hito con una sola fecha");
    // Each bar is placed in the track: a left and a width, not a default.
    let styled: Vec<String> = f
        .dom
        .nodes
        .iter()
        .filter_map(|n| n.el.inline_style.clone())
        .filter(|s| s.contains("left:"))
        .collect();
    assert!(styled.len() >= 3, "{styled:?}");
    assert!(styled.iter().any(|s| s.contains("width:")), "las barras tienen ancho: {styled:?}");
}

#[test]
fn a_plan_without_dates_says_how_to_write_one() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Vacio.md"), "---\ntipo: gantt\n---\n# Plan\n\n- una idea\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Vacio.md");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("Todavía no hay tareas con fechas"), "{texts}");
    assert!(texts.contains("2026-11-02"), "el ejemplo del formato: {texts}");
}
