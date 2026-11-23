//! A tolerant iCalendar (`.ics`) reader, a bounded `RRULE` expander and the events the user
//! writes itself, in a small JSON file of its own.
//!
//! Google Calendar, Apple Calendar and Thunderbird all publish a calendar as a plain `.ics` file
//! at a URL (Google's "secret address in iCal format"), and any of them can also be exported as a
//! file. This crate reads that text: no OAuth, no credentials, no network. `crates/app` is what
//! fetches and caches it, and what draws it.
//!
//! Two deliberate limits, both because there is no time-zone database in the dependency tree:
//! a `TZID` is kept as a label and the wall clock the file writes is the wall clock that is shown,
//! and only the four frequencies the calendar view needs are expanded (`DAILY`, `WEEKLY`,
//! `MONTHLY`, `YEARLY` with `INTERVAL`, `COUNT`, `UNTIL` and, for weekly rules, `BYDAY`).
//!
//! Nothing here panics on input: a truncated file, an unknown property, a date that does not
//! exist and a rule with no end all come back as fewer events, never as a crash.

mod civil;
pub mod local;
mod parse;
mod rrule;

pub use civil::Day;
pub use local::{Owned, OwnedEvent};
pub use parse::parse;
pub use rrule::parse as parse_rrule;
pub use rrule::{Freq, Rrule};

use rrule::MAX_STEPS;

/// A date and, when the value carries one, a time of day: `minute` is `hour * 60 + minute` and
/// `None` marks an all-day value (`VALUE=DATE`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Instant {
    pub day: Day,
    pub minute: Option<u16>,
}

impl Instant {
    pub fn all_day(day: Day) -> Instant {
        Instant { day, minute: None }
    }

    pub fn at(day: Day, minute: u16) -> Instant {
        Instant { day, minute: Some(minute) }
    }

    pub fn is_all_day(self) -> bool {
        self.minute.is_none()
    }

    /// `(hour, minute)` for a timed value; `None` for an all-day one.
    pub fn clock(self) -> Option<(u32, u32)> {
        self.minute.map(|m| (u32::from(m / 60), u32::from(m % 60)))
    }
}

/// One `VEVENT`, before the recurrence rule is expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// `UID`, or a name built from the summary and the start when the file has none.
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: Instant,
    /// Exclusive for an all-day event, as `DTEND` is; equal to `start` for a timed event with no
    /// end.
    pub end: Instant,
    /// The `TZID` of `DTSTART`, kept as a label. Without a zone database the wall clock the file
    /// writes is what is shown, so this is not used to convert anything.
    pub tzid: Option<String>,
    pub rrule: Option<Rrule>,
}

/// A whole `.ics` file: its display name (`X-WR-CALNAME`) and the events that parsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calendar {
    pub name: String,
    pub events: Vec<Event>,
}

/// One event on one day, after the recurrence rule was expanded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Occurrence {
    /// Index into [`Calendar::events`].
    pub event: usize,
    pub start: Instant,
    pub end: Instant,
}

impl Occurrence {
    pub fn is_all_day(&self) -> bool {
        self.start.is_all_day()
    }
}

/// The events of `cal` that fall on a day in `from..=to`, in the order the day shows them: all-day
/// events first, then by time, then by the order they were read.
pub fn expand(cal: &Calendar, from: Day, to: Day) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for (i, ev) in cal.events.iter().enumerate() {
        expand_event(i, ev, from, to, &mut out);
    }
    out.sort_by_key(|o| (o.start, o.event));
    out
}

fn expand_event(index: usize, ev: &Event, from: Day, to: Day, out: &mut Vec<Occurrence>) {
    let Some(rule) = &ev.rrule else {
        push_occurrences(index, ev.start, ev.end, from, to, out);
        return;
    };
    let len = length(ev);
    let cap = rule.count.unwrap_or(u32::MAX);
    let mut generated = 0u32;
    for k in 0..MAX_STEPS {
        if generated >= cap || rule.anchor(ev.start, k) > to {
            break;
        }
        let mut past = false;
        for day in rule.days_of(ev.start, k) {
            if generated >= cap {
                past = true;
                break;
            }
            // `UNTIL` is inclusive, and compares against the start of the instance. A date-only
            // `UNTIL` covers the whole of its day, so the time is only compared when both sides
            // carry one.
            if rule.until.is_some_and(|until| {
                day > until.day
                    || (day == until.day
                        && matches!((ev.start.minute, until.minute), (Some(start), Some(until)) if start > until))
            }) {
                past = true;
                break;
            }
            generated += 1;
            let start = Instant { day, minute: ev.start.minute };
            push_occurrences(index, start, end_of(start, len), from, to, out);
        }
        if past {
            break;
        }
    }
}

/// Add the days an instance is visible on. A timed instance is one occurrence on its day; an
/// all-day event shows on every day it covers, because `DTEND` of an all-day event is exclusive
/// and a week of holidays is a mark on each of its days.
fn push_occurrences(
    index: usize,
    start: Instant,
    end: Instant,
    from: Day,
    to: Day,
    out: &mut Vec<Occurrence>,
) {
    if start.minute.is_some() {
        if start.day >= from && start.day <= to {
            out.push(Occurrence { event: index, start, end });
        }
        return;
    }
    // A file that ends the event before it starts (or on the same day) still shows one day.
    let last = end.day.max(start.day.add_days(1));
    let mut day = start.day.max(from);
    while day < last && day <= to {
        out.push(Occurrence { event: index, start: Instant::all_day(day), end });
        day = day.add_days(1);
    }
}

/// Duration of an event as (whole days, extra minutes).
fn length(ev: &Event) -> (i32, i32) {
    let days = ev.end.day.index() - ev.start.day.index();
    let minutes = match (ev.end.minute, ev.start.minute) {
        (Some(end), Some(start)) => i32::from(end) - i32::from(start),
        _ => 0,
    };
    (days, minutes)
}

/// The end of an instance that starts at `start`, keeping the duration of the event.
fn end_of(start: Instant, (days, minutes): (i32, i32)) -> Instant {
    let mut day = start.day.add_days(days);
    let Some(minute) = start.minute else {
        return Instant { day, minute: None };
    };
    let mut minute = i32::from(minute) + minutes;
    if minute < 0 {
        minute = 0;
    }
    // An event that runs past midnight keeps its clock and moves to the next day.
    if minute >= 24 * 60 {
        day = day.add_days(minute / (24 * 60));
        minute %= 24 * 60;
    }
    Instant { day, minute: u16::try_from(minute).ok() }
}

/// Drop events whose `UID` repeats, keeping the last one read: a refetch, or the same calendar
/// subscribed twice, shows each event once. The first position a `UID` took is kept, so the order
/// of a file does not change.
pub fn dedup(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for ev in events {
        let key = ev.uid.to_lowercase();
        match seen.get(&key) {
            Some(&at) => out[at] = ev,
            None => {
                seen.insert(key, out.len());
                out.push(ev);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> Day {
        Day::from_ymd(y, m, d).unwrap()
    }

    #[test]
    fn end_of_moves_past_midnight() {
        let start = Instant::at(day(2025, 9, 16), 23 * 60 + 30);
        let end = end_of(start, (0, 90));
        assert_eq!(end.day, day(2025, 9, 17));
        assert_eq!(end.clock(), Some((1, 0)));
    }

    #[test]
    fn dedup_keeps_the_last_of_a_uid_in_place() {
        let ev = |uid: &str, summary: &str| Event {
            uid: uid.to_string(),
            summary: summary.to_string(),
            description: String::new(),
            location: String::new(),
            start: Instant::all_day(day(2025, 9, 16)),
            end: Instant::all_day(day(2025, 9, 17)),
            tzid: None,
            rrule: None,
        };
        let out = dedup(vec![ev("a", "first"), ev("b", "other"), ev("A", "second")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].summary, "second");
        assert_eq!(out[1].summary, "other");
    }

    #[test]
    fn all_day_events_sort_before_timed_ones() {
        let all = Occurrence {
            event: 0,
            start: Instant::all_day(day(2025, 9, 16)),
            end: Instant::all_day(day(2025, 9, 17)),
        };
        let timed = Occurrence {
            event: 1,
            start: Instant::at(day(2025, 9, 16), 540),
            end: Instant::at(day(2025, 9, 16), 600),
        };
        let mut v = [timed, all];
        v.sort_by_key(|o| (o.start, o.event));
        assert!(v[0].is_all_day());
    }
}
