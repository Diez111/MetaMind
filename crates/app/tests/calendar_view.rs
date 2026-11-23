//! The calendar tab: four ways of looking at the same events.

use metamind_app::calendarview::{CalendarView, Mode};
use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".metamind/calendar")).unwrap();
    let events = r#"{"version":1,"events":[
{"id":"a","title":"Parcial AM I","start":"2026-11-27","startMinute":540,"endMinute":660,"color":"violeta"},
{"id":"b","title":"Finde largo","start":"2026-11-28","color":"verde"}]}"#;
    fs::write(d.path().join(".metamind/calendar/events.json"), events).unwrap();
    d
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 760).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn open(mode: Mode) -> (tempfile::TempDir, App, Fonts) {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.calendar.select_in_month(calendar_core::Day::from_ymd(2026, 11, 27).unwrap());
    app.run("calview:open");
    app.run(&format!("calview:mode:{}", mode.id()));
    (d, app, Fonts::new())
}

#[test]
fn the_tab_opens_on_the_month_with_the_events_as_chips() {
    let (_d, mut app, mut fonts) = open(Mode::Month);
    assert!(app.calendar_view_active(), "la pestaña queda adelante");
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("noviembre 2026"), "el título del mes: {texts}");
    assert!(texts.contains("Parcial AM I"), "el evento del día: {texts}");
    assert!(f.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "calview-month")));
}

#[test]
fn the_week_places_an_event_by_its_hour() {
    let (_d, mut app, mut fonts) = open(Mode::Week);
    let f = frame(&mut app, &mut fonts);
    let event = f
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "calview-event"))
        .expect("el evento en la semana");
    // 09:00 at 44 px an hour: the block starts 396 px into the column.
    let style = event.el.inline_style.clone().unwrap_or_default();
    assert!(style.contains("top: 396"), "el bloque arranca a las 9: {style}");
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("09:00 Parcial AM I"), "{texts}");
}

#[test]
fn the_day_lists_the_hours_and_the_agenda_the_next_days() {
    let (_d, mut app, mut fonts) = open(Mode::Day);
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("09:00"), "{texts}");
    assert!(texts.contains("Parcial AM I"), "{texts}");
    app.run("calview:mode:agenda");
    let f = frame(&mut app, &mut fonts);
    let texts: String = f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect::<Vec<_>>().join("|");
    assert!(texts.contains("Parcial AM I"), "la agenda lista el evento: {texts}");
    assert!(texts.contains("Finde largo"), "{texts}");
}

#[test]
fn the_arrows_move_a_month_a_week_or_a_day_and_today_comes_back() {
    let (_d, mut app, _fonts) = open(Mode::Month);
    let before = app.calendar_view.as_ref().unwrap().anchor;
    app.run("calview:next");
    let after = app.calendar_view.as_ref().unwrap().anchor;
    assert!(after > before, "el mes siguiente");
    app.run("calview:mode:week");
    let day = app.calendar_view.as_ref().unwrap().selected;
    app.run("calview:next");
    assert_eq!(app.calendar_view.as_ref().unwrap().selected, day.add_days(7), "una semana");
    app.run("calview:today");
    assert_eq!(app.calendar_view.as_ref().unwrap().selected, metamind_app::calendar::Calendar::today());
    // The tab closes and the note comes back.
    app.run("calview:close");
    assert!(!app.calendar_view_active());
}

#[test]
fn the_view_state_is_honest_about_what_it_shows() {
    let mut view = CalendarView::open(calendar_core::Day::from_ymd(2026, 11, 27).unwrap());
    assert_eq!(view.mode, Mode::Month);
    assert!(view.title().contains("noviembre 2026"));
    view.mode = Mode::Week;
    assert!(view.title().contains("23–29"), "{}", view.title());
    view.mode = Mode::Day;
    assert!(view.title().contains("27 de noviembre"), "{}", view.title());
    assert_eq!(
        metamind_app::calendarview::week_start(view.selected).weekday(),
        1,
        "la semana empieza el lunes"
    );
}
