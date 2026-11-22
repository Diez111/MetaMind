//! The gates of the crate, on files shaped the way a real calendar writes them: a Google export
//! with a `TZID`, an all-day event, a weekly rule with `COUNT=8`, and a file cut in the middle.

use calendar_core::{expand, parse, Day, Freq};

fn day(y: i32, m: u32, d: u32) -> Day {
    Day::from_ymd(y, m, d).expect("the date of the test exists")
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).expect("the fixture is in the crate")
}

#[test]
fn google_tzid_event_reads_its_wall_clock() {
    let cal = parse(&fixture("google_tzid.ics"));
    assert_eq!(cal.name, "Trabajo");
    let ev = &cal.events[0];
    assert_eq!(ev.summary, "Reunión de equipo, semanal");
    assert_eq!(ev.location, "Sala 3");
    // The escapes of the description, and the folded line joined.
    assert_eq!(
        ev.description,
        "Línea uno\nLínea dos; con punto y coma y una continuación larga que sigue en la línea de abajo"
    );
    assert_eq!(ev.uid, "abc-123@google.com");
    assert_eq!(ev.tzid.as_deref(), Some("America/Argentina/Buenos_Aires"));
    assert_eq!(ev.start, calendar_core::Instant::at(day(2025, 9, 16), 9 * 60 + 30));
    assert_eq!(ev.end, calendar_core::Instant::at(day(2025, 9, 16), 10 * 60 + 30));
    assert_eq!(ev.rrule.as_ref().map(|r| r.freq), Some(Freq::Weekly));
}

#[test]
fn all_day_event_covers_its_days() {
    let cal = parse(&fixture("all_day.ics"));
    assert_eq!(cal.events.len(), 2);
    let vac = &cal.events[0];
    assert!(vac.start.is_all_day());
    assert_eq!(vac.start.day, day(2025, 9, 16));
    // DTEND of an all-day event is exclusive: the three days 16, 17 and 18.
    assert_eq!(vac.end.day, day(2025, 9, 19));
    // No DTEND means one day.
    assert_eq!(cal.events[1].end.day, day(2025, 9, 21));

    let month = expand(&cal, day(2025, 9, 1), day(2025, 9, 30));
    let days: Vec<i32> = month.iter().map(|o| o.start.day.index()).collect();
    // 16, 17, 18 (the exclusive end is 19) and 20.
    assert_eq!(
        days,
        vec![
            day(2025, 9, 16).index(),
            day(2025, 9, 17).index(),
            day(2025, 9, 18).index(),
            day(2025, 9, 20).index()
        ]
    );
    // Only the first day starts the event.
    assert_eq!(month.iter().filter(|o| o.start.minute.is_none()).count(), 4);
}

#[test]
fn weekly_rule_gives_eight_instances() {
    let cal = parse(&fixture("weekly_count8.ics"));
    let ev = &cal.events[0];
    let rule = ev.rrule.as_ref().expect("the fixture repeats");
    assert_eq!(rule.count, Some(8));
    // Two years of window: COUNT is what stops it, not the window.
    let all = expand(&cal, day(2025, 1, 1), day(2027, 12, 31));
    assert_eq!(all.len(), 8, "COUNT=8 means eight instances, no more");
    let days: Vec<Day> = all.iter().map(|o| o.start.day).collect();
    assert_eq!(days[0], day(2025, 9, 16));
    assert_eq!(days[1], day(2025, 9, 23));
    assert_eq!(days[7], day(2025, 11, 4));
    for o in &all {
        assert_eq!(o.start, calendar_core::Instant::at(o.start.day, 18 * 60));
        assert_eq!(o.end, calendar_core::Instant::at(o.start.day, 19 * 60));
    }
    // A month window only carries the instances of that month.
    let september = expand(&cal, day(2025, 9, 1), day(2025, 9, 30));
    assert_eq!(september.len(), 3);
}

#[test]
fn truncated_file_keeps_the_events_before_the_cut() {
    let text = fixture("truncated.ics");
    let cal = parse(&text);
    assert_eq!(cal.events.len(), 1, "the event cut in half is dropped, the one before it stays");
    assert_eq!(cal.events[0].summary, "Este sí está completo");

    // Any truncation of the file, at any character, is readable and does not panic.
    for (cut, _) in text.char_indices() {
        let part = &text[..cut];
        let cal = parse(part);
        assert!(cal.events.len() <= 1);
    }
}

#[test]
fn expansion_is_bounded_by_the_window() {
    let cal = parse(&fixture("google_tzid.ics"));
    // An endless weekly rule, asked about one month: only that month comes back.
    let march = expand(&cal, day(2026, 3, 1), day(2026, 3, 31));
    assert_eq!(march.len(), 5, "2026-03 has five Tuesdays");
    for o in &march {
        assert_eq!(o.start.day.weekday(), 2, "the series stays on its own weekday");
    }
    let empty = expand(&cal, day(1990, 1, 1), day(1990, 1, 31));
    assert!(empty.is_empty(), "nothing before DTSTART");
}

#[test]
fn until_is_inclusive_and_counted_from_the_start() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20250916\n\
                RRULE:FREQ=DAILY;UNTIL=20250918\nSUMMARY:Diario\nUID:d@x\nEND:VEVENT\nEND:VCALENDAR\n";
    let cal = parse(text);
    let days: Vec<Day> =
        expand(&cal, day(2025, 9, 1), day(2025, 9, 30)).iter().map(|o| o.start.day).collect();
    assert_eq!(days, vec![day(2025, 9, 16), day(2025, 9, 17), day(2025, 9, 18)]);
}
