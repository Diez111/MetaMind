//! The events the user writes: a JSON file of their own, the expansion that feeds the views, and
//! the tolerance that keeps a bad file from taking the calendar down with it.

use calendar_core::local::{valid_color, Owned, OwnedEvent, NAMED_COLORS};
use calendar_core::Day;

fn day(y: i32, m: u32, d: u32) -> Day {
    Day::from_ymd(y, m, d).unwrap()
}

fn sample() -> Owned {
    let mut owned = Owned::default();
    let mut parcial = OwnedEvent::timed("Parcial AM I", day(2026, 11, 27), 9 * 60, 11 * 60, "violeta");
    parcial.location = "Aula 12".into();
    parcial.description = "Unidades 5 a 8".into();
    owned.upsert(parcial);
    let mut finde = OwnedEvent::all_day("Finde largo", day(2026, 11, 21), "verde");
    finde.description = "Sin clases".into();
    owned.upsert(finde);
    owned
}

#[test]
fn a_file_read_and_written_again_is_the_same_file() {
    let owned = sample();
    let text = owned.to_json();
    let (again, note) = Owned::parse(&text).unwrap();
    assert!(note.is_none());
    assert_eq!(again, owned);
    assert_eq!(again.to_json(), text, "escribir lo que se leyó no cambia el archivo");
}

#[test]
fn a_key_this_crate_does_not_know_survives_a_save() {
    let text = r##"{"version":1,"events":[
{"id":"ev-1","title":"Clase","start":"2026-11-27","startMinute":540,"color":"azul",
 "reminderMinutes":[10,30],"future":{"keep":"me"}}],"owner":"Ana","calendar":{"extra":1}}"##;
    let (owned, note) = Owned::parse(text).unwrap();
    assert!(note.is_none());
    let out = owned.to_json();
    assert!(out.contains("\"reminderMinutes\""), "a per-event key survives: {out}");
    assert!(out.contains("\"owner\": \"Ana\""), "a top-level key survives: {out}");
    assert!(out.contains("\"extra\": 1"), "a nested key survives: {out}");
}

#[test]
fn a_broken_event_is_dropped_and_the_rest_is_kept() {
    let text = r#"{"events":[
{"id":"a","title":"Buena","start":"2026-11-27","color":"rojo"},
{"id":"b","title":"Sin fecha","color":"rojo"},
{"id":"c","title":"Fecha imposible","start":"2026-02-31","color":"rojo"},
{"id":"d","title":"Hora absurda","start":"2026-11-28","startMinute":"nueve","color":"rojo"}]}"#;
    let (owned, note) = Owned::parse(text).unwrap();
    assert_eq!(owned.events.len(), 2, "la buena y la de hora absurda (sin hora)");
    assert_eq!(owned.events[0].title, "Buena");
    let note = note.expect("se avisa");
    assert!(note.contains('2'), "the notice counts the dropped ones: {note}");
}

#[test]
fn a_file_that_is_not_a_calendar_is_an_error_and_gets_a_backup() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("events.json");
    std::fs::write(&path, "{ esto no es json").unwrap();
    let (owned, note) = Owned::load(&path);
    assert!(owned.events.is_empty());
    let note = note.unwrap();
    assert!(note.contains("copia"), "{note}");
    let backups: Vec<_> = std::fs::read_dir(d.path())
        .unwrap()
        .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.contains("corrupt-"))
        .collect();
    assert_eq!(backups.len(), 1, "the original text was kept: {backups:?}");
    let kept = std::fs::read_to_string(d.path().join(&backups[0])).unwrap();
    assert_eq!(kept, "{ esto no es json");
}

#[test]
fn saving_is_atomic_and_leaves_no_temporary_behind() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("sub/events.json");
    sample().save_atomic(&path).unwrap();
    assert!(path.exists());
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no .tmp file stays: {leftovers:?}");
    let (loaded, note) = Owned::load(&path);
    assert!(note.is_none());
    assert_eq!(loaded.events.len(), 2);
}

#[test]
fn a_missing_file_is_an_empty_calendar_without_a_notice() {
    let d = tempfile::tempdir().unwrap();
    let (owned, note) = Owned::load(&d.path().join("nada.json"));
    assert!(owned.events.is_empty());
    assert!(note.is_none());
}

#[test]
fn the_id_is_the_event_and_not_the_order() {
    let mut owned = Owned::default();
    let first = owned.upsert(OwnedEvent::timed("Clase", day(2026, 11, 27), 540, 600, "azul"));
    let again = owned.upsert(OwnedEvent::timed("Clase", day(2026, 11, 27), 540, 610, "rojo"));
    assert_eq!(first, again, "the same event keeps its id when it is edited");
    assert_eq!(owned.events.len(), 1);
    assert_eq!(owned.events[0].color, "rojo");
    assert_eq!(owned.events[0].end_minute, Some(610));
    let other = owned.upsert(OwnedEvent::timed("Clase", day(2026, 11, 27), 600, 660, "azul"));
    assert_ne!(first, other, "another clock is another event");
    assert_eq!(owned.events.len(), 2);
}

#[test]
fn an_edit_keeps_the_creation_stamp_and_an_upsert_sorts_by_day() {
    let mut owned = Owned::default();
    let mut late = OwnedEvent::all_day("Tarde", day(2026, 12, 3), "gris");
    late.created_ms = 42;
    owned.upsert(late);
    owned.upsert(OwnedEvent::all_day("Temprano", day(2026, 11, 1), "gris"));
    assert_eq!(owned.events[0].title, "Temprano", "el archivo queda ordenado por día");
    let mut edit = owned.events[1].clone();
    edit.title = "Tarde (editada)".into();
    owned.upsert(edit);
    assert_eq!(owned.events[1].created_ms, 42, "editar no reescribe la creación");
    assert_eq!(owned.events[1].title, "Tarde (editada)");
}

#[test]
fn a_timed_event_expands_on_its_day_and_an_all_day_one_on_each_day_it_covers() {
    let owned = sample();
    let week = owned.expand(day(2026, 11, 20), day(2026, 11, 30));
    let titles: Vec<&str> = week.iter().map(|(i, _)| owned.events[*i].title.as_str()).collect();
    assert_eq!(titles, ["Finde largo", "Parcial AM I"]);
    let (index, occ) = week[1];
    assert_eq!(owned.events[index].title, "Parcial AM I");
    assert_eq!(occ.start.minute, Some(540));
    assert_eq!(occ.end.minute, Some(660));
}

#[test]
fn a_weekly_rule_shows_every_week_and_ends_on_until() {
    let mut owned = Owned::default();
    let mut ev = OwnedEvent::timed("Práctica", day(2026, 11, 2), 600, 660, "cian");
    ev.rrule = Some("FREQ=WEEKLY;INTERVAL=1;UNTIL=20261123".into());
    owned.upsert(ev);
    // Five Mondays: 2, 9, 16, 23 (the 30th is past UNTIL).
    let all = owned.expand(day(2026, 11, 1), day(2026, 11, 30));
    assert_eq!(all.len(), 4, "{all:?}");
    assert!(all.iter().all(|(_, o)| o.start.day.weekday() == 1), "todos los lunes");
}

#[test]
fn an_unreadable_rule_keeps_the_event() {
    let text =
        r#"{"events":[{"id":"a","title":"Con regla rota","start":"2026-11-27","rrule":"FREQ=NARANJA"}]}"#;
    let (owned, _) = Owned::parse(text).unwrap();
    assert_eq!(owned.events.len(), 1);
    let all = owned.expand(day(2026, 11, 27), day(2026, 11, 27));
    assert_eq!(all.len(), 1, "la cita sigue estando aunque la regla no se entienda");
}

#[test]
fn the_colours_are_the_eight_names_or_a_hex() {
    for name in NAMED_COLORS {
        assert!(valid_color(name), "{name}");
    }
    assert!(valid_color("#8a5cf5"));
    assert!(!valid_color("#8a5cf"));
    assert!(!valid_color("violeta claro"));
    assert!(!valid_color("rgb(1,2,3)"));
    let mut owned = Owned::default();
    let mut ev = OwnedEvent::all_day("Sin color válido", day(2026, 11, 5), "chartreuse");
    ev.created_ms = 1;
    owned.upsert(ev);
    assert_eq!(owned.events[0].color, "violeta", "un color inválido cae al primero");
}

#[test]
fn five_thousand_events_load_fast() {
    let mut owned = Owned::default();
    for i in 0..5000 {
        let ev = OwnedEvent::timed(
            format!("Evento {i}"),
            day(2026, 1 + (i % 12) as u32, 1 + (i % 28) as u32),
            8 * 60 + (i % 10) as u16,
            9 * 60 + (i % 10) as u16,
            "azul",
        );
        owned.upsert(ev);
    }
    let text = owned.to_json();
    let start = std::time::Instant::now();
    let (again, _) = Owned::parse(&text).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(again.events.len(), 5000);
    assert!(elapsed.as_millis() < 200, "5000 eventos en {elapsed:?}");
    let month = again.expand(day(2026, 6, 1), day(2026, 6, 30));
    assert!(!month.is_empty());
}
