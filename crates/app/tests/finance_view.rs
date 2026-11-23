//! The money board: a note with `tipo: finanzas` is drawn as totals, months and categories.

use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("Dinero.md"),
        "---\ntipo: finanzas\n---\n# Finanzas\n\n## Ingresos\n- 2026-11-01 · sueldo · 1.250.000\n\n## Gastos\n- 2026-11-03 · supermercado · 45.200\n- 2026-11-05 · transporte · 12.000\n",
    )
    .unwrap();
    d
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 800).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

#[test]
fn the_note_becomes_a_board_with_totals_months_and_categories() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Dinero.md");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("Finanzas"), "{texts}");
    assert!(texts.contains("Ingresos del mes"), "las tres tarjetas: {texts}");
    assert!(texts.contains("$1.250.000"), "el ingreso, con separadores: {texts}");
    assert!(texts.contains("$57.200"), "los gastos sumados: {texts}");
    assert!(texts.contains("$1.192.800"), "el balance: {texts}");
    assert!(texts.contains("supermercado"), "las categorías: {texts}");
    assert!(texts.contains("Últimos 6 meses"), "{texts}");
    // The bars are boxes with a width, not text.
    let bars = f.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == "finance-bar")).count();
    assert!(bars >= 12, "seis meses por dos barras: {bars}");
}

#[test]
fn a_note_without_movements_says_how_to_write_one() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Vacia.md"), "---\ntipo: finanzas\n---\n# Finanzas\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Vacia.md");
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("Todavía no hay movimientos"), "{texts}");
    assert!(texts.contains("supermercado"), "el ejemplo del formato: {texts}");
    // A note that does not say `tipo: finanzas` stays a note.
    fs::write(d.path().join("Otra.md"), "# Otra\n\n- 2026-11-03 · supermercado · 45.200\n").unwrap();
    app.open_note("Otra.md");
    assert!(!metamind_app::finance::is_finance(&app));
}
