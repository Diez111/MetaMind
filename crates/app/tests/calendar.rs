//! The calendar as a flow: a `.ics` file in the vault reaches the right day of the month, the day
//! panel lists it, and "abrir nota del día" creates the daily note of that day.
use metamind_app::shell::{Focus, Panel};
use metamind_app::{render, App};
use metamind_ui::{Dom, Fonts, Pixmap};
use std::fs;

/// Today in the vault's own formats: `YYYYMMDD` for the `.ics`, `YYYY-MM-DD` for the note name.
fn today() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("el reloj va para adelante")
        .as_secs();
    (metamind_app::datetime::format("YYYYMMDD", secs), metamind_app::datetime::format("YYYY-MM-DD", secs))
}

fn vault(ics: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::create_dir_all(d.path().join("Attachments")).unwrap();
    fs::write(d.path().join("Attachments/cal.ics"), ics).unwrap();
    fs::write(d.path().join("Note.md"), "# Nota\n").unwrap();
    fs::write(d.path().join(".obsidian/daily-notes.json"), r#"{"folder":"Daily"}"#).unwrap();
    d
}

/// The calendar of the vault: a meeting today at 09:30 and an all-day event tomorrow.
fn ics() -> String {
    let (today, _) = today();
    let tomorrow = metamind_app::datetime::format(
        "YYYYMMDD",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("el reloj va para adelante")
            .as_secs()
            + 86_400,
    );
    format!(
        "BEGIN:VCALENDAR\nVERSION:2.0\nX-WR-CALNAME:Trabajo\n\
         BEGIN:VEVENT\nDTSTART;TZID=America/Argentina/Buenos_Aires:{today}T093000\n\
         DTEND;TZID=America/Argentina/Buenos_Aires:{today}T103000\n\
         SUMMARY:Reunión de equipo\nLOCATION:Sala 3\nDESCRIPTION:Punto uno\\nPunto dos\n\
         UID:cal-flow-1@example.com\nEND:VEVENT\n\
         BEGIN:VEVENT\nDTSTART;VALUE=DATE:{tomorrow}\nSUMMARY:Todo el día\n\
         UID:cal-flow-2@example.com\nEND:VEVENT\nEND:VCALENDAR\n"
    )
}

fn frame(app: &mut App) -> (Dom, Fonts) {
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 700).unwrap();
    let f = render(app, &mut fonts, &mut pm, 1.0);
    (f.dom, fonts)
}

fn texts(dom: &Dom) -> Vec<String> {
    dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect()
}

fn has_class(dom: &Dom, class: &str) -> bool {
    dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == class))
}

#[test]
fn an_ics_in_the_vault_reaches_its_day_and_opens_the_daily_note() {
    let (_, yyyy_mm_dd) = today();
    let d = vault(&ics());
    let mut app = App::open(d.path()).unwrap();
    app.run("calendar:open");
    assert_eq!(app.panel, Panel::Calendar, "el comando abre el tab del sidebar");
    assert_eq!(app.focus, Focus::Calendar, "y le da el teclado");
    assert!(app.left_open);

    // The rows are built on the frame, from the index of the vault.
    let (dom, _fonts) = frame(&mut app);
    assert!(has_class(&dom, "calendar-grid"), "el sidebar ancho muestra la grilla");
    assert!(has_class(&dom, "is-today"), "el día de hoy está marcado");
    let shown = texts(&dom);
    assert!(shown.iter().any(|t| t == "Reunión de equipo"), "el evento del .ics está en el día: {shown:?}");
    assert!(shown.iter().any(|t| t == "09:30"), "con su hora: {shown:?}");

    // The next day lists the all-day event of the file.
    let today_day = metamind_app::calendar::Calendar::today();
    app.run(&format!("cal:day:{}", today_day.add_days(1).index()));
    let (dom, _fonts) = frame(&mut app);
    let shown = texts(&dom);
    assert!(shown.iter().any(|t| t == "Todo el día"), "el evento de todo el día: {shown:?}");
    assert_eq!(shown.iter().filter(|t| *t == "Todo el día").count(), 2, "la hora y el título");

    // Back to today: the detail of an event is one click away.
    app.run(&format!("cal:day:{}", today_day.index()));
    app.run(&format!("cal:event:{}:0", today_day.index()));
    let (dom, _fonts) = frame(&mut app);
    let shown = texts(&dom);
    assert!(shown.iter().any(|t| t == "Sala 3"), "el detalle muestra la ubicación: {shown:?}");
    assert!(shown.iter().any(|t| t == "Origen: Bóveda"), "y de dónde viene: {shown:?}");
    assert!(
        shown.iter().any(|t| t == "Abrir Attachments/cal.ics"),
        "y ofrece abrir el archivo del vault: {shown:?}"
    );

    // "Abrir nota del día" crea la nota diaria de ese día y la abre.
    app.run("calendar:open-day-note");
    let path = app.tab().expect("hay una pestaña abierta").path.clone();
    assert_eq!(path, format!("Daily/{yyyy_mm_dd}.md"), "la daily note del día seleccionado");
    assert!(d.path().join(&path).is_file(), "y existe en disco");
}

#[test]
fn a_narrow_sidebar_shows_the_day_list_instead_of_the_grid() {
    let d = vault(&ics());
    let mut app = App::open(d.path()).unwrap();
    app.run("calendar:open");
    app.left_w = 180.0;
    let (dom, _fonts) = frame(&mut app);
    assert!(!has_class(&dom, "calendar-grid"), "sin grilla en un sidebar angosto");
    assert!(has_class(&dom, "calendar-day-list"), "pero sí la lista del día");
    assert!(texts(&dom).iter().any(|t| t == "Reunión de equipo"));
}

#[test]
fn the_arrows_move_the_selected_day_and_the_month_changes_with_it() {
    let d = vault(&ics());
    let mut app = App::open(d.path()).unwrap();
    app.run("calendar:open");
    let start = app.calendar.selected;
    app.key(metamind_app::Key::parse("ArrowRight").unwrap());
    assert_eq!(app.calendar.selected, start.add_days(1));
    app.key(metamind_app::Key::parse("ArrowDown").unwrap());
    assert_eq!(app.calendar.selected, start.add_days(8));
    // Un mes adelante y otro atrás vuelve al mismo mes.
    let month = app.calendar.month;
    app.run("cal:next");
    assert_ne!(app.calendar.month, month);
    app.run("cal:prev");
    assert_eq!(app.calendar.month, month);
    // Escape devuelve el teclado al editor.
    app.key(metamind_app::Key::parse("Escape").unwrap());
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn a_weekly_rule_marks_more_than_one_week_in_the_month() {
    let (today, _) = today();
    let d = vault(&format!(
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:{today}\nRRULE:FREQ=WEEKLY;COUNT=8\n\
         SUMMARY:Clase de piano\nUID:weekly-flow@example.com\nEND:VEVENT\nEND:VCALENDAR\n"
    ));
    let mut app = App::open(d.path()).unwrap();
    app.run("calendar:open");
    let (dom, _fonts) = frame(&mut app);
    let marks = dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "calendar-cell"))
        .filter(|n| {
            let dotted =
                n.children.iter().any(|&c| dom.nodes[c].el.classes.iter().any(|k| k == "calendar-marks"));
            dotted
        })
        .count();
    assert!(marks >= 2, "la serie semanal marca varios días del mes, no uno: {marks}");
}

/// The month is a grid: every row the same height and every cell the same width, with or without
/// dots. A row that grows because one of its days has events is what makes a month look broken.
#[test]
fn the_month_cells_are_evenly_spaced_at_every_width() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("Tareas.md"),
        "- [ ] Uno @{2026-11-24}\n- [ ] Dos @{2026-11-27}\n- [ ] Tres @{2026-11-24}\n",
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.calendar.select_in_month(calendar_core::Day::from_ymd(2026, 11, 23).unwrap());
    app.run("calendar:open");
    let mut fonts = Fonts::new();
    for width in [220.0f32, 300.0, 480.0] {
        app.left_open = true;
        app.left_w = width;
        let mut pm = Pixmap::new(width as u32 + 400, 700).unwrap();
        render(&mut app, &mut fonts, &mut pm, 1.0);
        let f = render(&mut app, &mut fonts, &mut pm, 1.0);
        let classed = |class: &str| -> Vec<metamind_ui::Rect> {
            f.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == class)).map(|n| n.rect).collect()
        };
        let rows = classed("calendar-row");
        assert_eq!(rows.len(), 6, "seis semanas");
        let first = rows[0].h;
        for (i, r) in rows.iter().enumerate() {
            assert!((r.h - first).abs() < 0.5, "la fila {i} mide {} y la primera {first}", r.h);
            assert!((r.y - (rows[0].y + i as f32 * first)).abs() < 0.5, "la fila {i} no está en su lugar");
        }
        let cells = classed("calendar-cell");
        assert_eq!(cells.len(), 42);
        for week in 0..6 {
            let row = &cells[week * 7..week * 7 + 7];
            let w = row[0].w;
            for (i, c) in row.iter().enumerate() {
                assert!(
                    (c.w - w).abs() < 0.5,
                    "a {width}px la celda {week}.{i} mide {} y la primera {w}",
                    c.w
                );
                assert!(
                    (c.x - (row[0].x + i as f32 * w)).abs() < 0.5,
                    "a {width}px la celda {week}.{i} no está en su lugar: x {} vs {}",
                    c.x,
                    row[0].x + i as f32 * w
                );
            }
        }
    }
}
