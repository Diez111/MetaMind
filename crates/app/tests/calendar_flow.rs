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
