//! The calendar's day panel writes back: a task is marked done from the panel and its note says
//! so. The row carries the note it came from, and the checkbox flips that line and nothing else.

use metamind_app::{render, App};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

/// A vault with three tasks for today: two pending and one already done.
fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let now = vault_core::fsio::now_ms() / 1000;
    let (y, m, day, _, _, _, _) = metamind_app::datetime::local(now);
    let date = format!("{y:04}-{m:02}-{day:02}");
    fs::write(
        d.path().join("Tareas.md"),
        format!(
            "- [ ] Comprar pan @{{{date}}}\n\
             - [ ] Estudiar AGA @{{{date}}}\n\
             - [x] Ya hecha @{{{date}}}\n\
             - [ ] Otra para otro dia @{{2001-01-01}}\n"
        ),
    )
    .unwrap();
    d
}

fn frame(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap) {
    render(app, fonts, pm, 1.0);
}

#[test]
fn a_task_is_marked_done_from_the_day_panel_and_the_note_says_so() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    app.run("calendar:open");
    // The scan of the vault happens in the frames the window draws: the tasks are rows after
    // them, which is the state a click always finds.
    for _ in 0..12 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let day = app.calendar.selected;
    let at = app
        .calendar
        .rows_on(day)
        .position(|r| r.title == "Estudiar AGA")
        .expect("the task of today is a row of the day panel");

    app.run(&format!("cal:done:{}:{at}", day.index()));
    let text = fs::read_to_string(d.path().join("Tareas.md")).unwrap();
    assert!(text.contains("- [x] Estudiar AGA"), "the task is done in its note: {text}");
    assert!(text.contains("- [ ] Comprar pan"), "the task beside it keeps its box: {text}");
    assert!(text.contains("- [x] Ya hecha"), "a task already done is not undone: {text}");
    assert!(text.contains("- [ ] Otra para otro dia"), "another day is another row: {text}");

    // And the same click takes it back.
    for _ in 0..4 {
        frame(&mut app, &mut fonts, &mut pm);
    }
    let at = app
        .calendar
        .rows_on(day)
        .position(|r| r.title == "Estudiar AGA")
        .expect("a done task is still a row: it can be undone");
    app.run(&format!("cal:done:{}:{at}", day.index()));
    let text = fs::read_to_string(d.path().join("Tareas.md")).unwrap();
    assert!(text.contains("- [ ] Estudiar AGA"), "the second click makes it pending again: {text}");
}

/// A day to write on: far enough from today that nothing else lands on it.
fn a_day() -> calendar_core::Day {
    calendar_core::Day::from_ymd(2026, 11, 27).unwrap()
}

/// Drive the editor with the same commands the modal sends.
fn create_event(app: &mut App, day: calendar_core::Day, title: &str, start: u16, end: u16) {
    app.calendar.select_in_month(day);
    app.run("cal:new");
    for c in title.chars() {
        app.text_input(&c.to_string());
    }
    // The form opens at the next quarter hour: move it to the time the test asked for.
    while app.calendar.form.as_ref().is_some_and(|f| f.start < start) {
        app.run("cal:step:start:15");
    }
    while app.calendar.form.as_ref().is_some_and(|f| f.start > start) {
        app.run("cal:step:start:-15");
    }
    while app.calendar.form.as_ref().is_some_and(|f| f.end < end) {
        app.run("cal:step:end:15");
    }
    app.run("cal:save");
}

#[test]
fn an_event_written_in_the_editor_is_on_the_day_and_in_the_file() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let day = a_day();
    create_event(&mut app, day, "Parcial AM I", 9 * 60, 11 * 60);
    assert!(app.calendar.form.is_none(), "saving closes the editor");
    assert_eq!(app.calendar.rows_on(day).count(), 1, "the event is on its day");
    let row = app.calendar.rows_on(day).next().unwrap();
    assert_eq!(row.title, "Parcial AM I");
    assert_eq!(row.minute, Some(9 * 60));
    assert_eq!(row.source, metamind_app::calendar::Source::Owned);

    // And it is in the file, with the shape the reader parses.
    let text = fs::read_to_string(d.path().join(".metamind/calendar/events.json")).unwrap();
    assert!(text.contains("\"title\": \"Parcial AM I\""), "{text}");
    assert!(text.contains("\"startMinute\": 540"), "{text}");
    let (owned, note) = calendar_core::local::Owned::parse(&text).unwrap();
    assert!(note.is_none());
    assert_eq!(owned.events.len(), 1);

    // A second app over the same vault sees it: the file is the calendar.
    let mut again = App::open(d.path()).unwrap();
    again.calendar.select_in_month(day);
    again.calendar.rebuild();
    assert_eq!(again.calendar.rows_on(day).count(), 1, "y se relee al abrir la bóveda");
}

#[test]
fn an_event_opens_its_editor_and_the_title_can_be_rewritten() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let day = a_day();
    create_event(&mut app, day, "Clase", 10 * 60, 11 * 60);
    let uid = app.calendar.rows_on(day).next().unwrap().uid.clone();
    // A click on the row of an event of ours opens the editor, not the detail.
    app.run(&format!("cal:edit:{uid}"));
    let form = app.calendar.form.clone().expect("the editor is open");
    assert_eq!(form.title, "Clase");
    assert_eq!(form.id.as_deref(), Some(uid.as_str()));
    // Clear the title and write another one.
    for _ in 0.."Clase".len() {
        app.key(metamind_app::Key::new("Backspace"));
    }
    for c in "Clase de repaso".chars() {
        app.text_input(&c.to_string());
    }
    app.run("cal:save");
    let row = app.calendar.rows_on(day).next().unwrap();
    assert_eq!(row.title, "Clase de repaso");
    assert_eq!(row.uid, uid, "editar no crea otro evento");
    assert_eq!(app.calendar.rows_on(day).count(), 1);
}

#[test]
fn an_event_without_a_title_is_not_saved_and_the_form_says_why() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let day = a_day();
    app.calendar.select_in_month(day);
    app.run("cal:new");
    app.run("cal:save");
    assert!(app.calendar.form.is_some(), "the editor stays open");
    let error = app.calendar.form.as_ref().unwrap().error.clone().expect("hay un error");
    assert!(error.contains("título"), "{error}");
    assert_eq!(app.calendar.rows_on(day).count(), 0);
    assert!(!d.path().join(".metamind/calendar/events.json").exists(), "nada se escribió");
}

#[test]
fn the_first_press_asks_before_deleting_and_the_second_does_it() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let day = a_day();
    create_event(&mut app, day, "Se va", 12 * 60, 13 * 60);
    let uid = app.calendar.rows_on(day).next().unwrap().uid.clone();
    app.run(&format!("cal:edit:{uid}"));
    app.run("cal:del");
    assert!(app.calendar.form.as_ref().unwrap().confirm_delete, "pregunta primero");
    assert_eq!(app.calendar.rows_on(day).count(), 1, "todavía está");
    app.run("cal:del");
    assert!(app.calendar.form.is_none());
    assert_eq!(app.calendar.rows_on(day).count(), 0);
    let text = fs::read_to_string(d.path().join(".metamind/calendar/events.json")).unwrap();
    let (owned, _) = calendar_core::local::Owned::parse(&text).unwrap();
    assert!(owned.events.is_empty(), "y no está en el archivo");
}

#[test]
fn choosing_a_day_shows_its_events_and_leaves_the_month_where_it_is() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let month = calendar_core::Day::from_ymd(2026, 11, 1).unwrap();
    app.calendar.select_in_month(calendar_core::Day::from_ymd(2026, 11, 27).unwrap());
    let day = calendar_core::Day::from_ymd(2026, 11, 25).unwrap();
    app.run(&format!("cal:day:{}", day.index()));
    assert_eq!(app.calendar.selected, day);
    assert_eq!(app.calendar.month, month, "el mes no se mueve bajo el puntero");
    // A day of the next month that is on screen selects without moving the grid either.
    let next = calendar_core::Day::from_ymd(2026, 12, 3).unwrap();
    if app.calendar.rows.is_empty() {
        app.calendar.rebuild();
    }
    app.run(&format!("cal:day:{}", next.index()));
    assert_eq!(app.calendar.month, month, "un día visible del mes siguiente no cambia el mes");
    assert_eq!(app.calendar.selected, next);
}

#[test]
fn the_colour_of_a_day_is_the_one_picked_in_the_editor() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let day = a_day();
    app.calendar.select_in_month(day);
    app.run("cal:new");
    for c in "Verde".chars() {
        app.text_input(&c.to_string());
    }
    app.run("cal:color:verde");
    app.run("cal:save");
    let row = app.calendar.rows_on(day).next().unwrap();
    assert_eq!(row.color.as_deref(), Some("verde"));
    assert_eq!(calendar_core::local::color_hex("verde"), "#2f9e44");
}

#[test]
fn the_editor_is_a_modal_and_the_note_behind_it_never_sees_a_letter() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("A.md"), "# A\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A");
    let before = app.tab().unwrap().editor.buf.text();
    app.run("cal:new");
    for c in "hola".chars() {
        app.text_input(&c.to_string());
    }
    assert_eq!(app.tab().unwrap().editor.buf.text(), before, "la nota no cambió");
    assert_eq!(app.calendar.form.as_ref().unwrap().title, "hola");
    app.key(metamind_app::Key::new("Escape"));
    assert!(app.calendar.form.is_none(), "Escape cierra el editor");
}
