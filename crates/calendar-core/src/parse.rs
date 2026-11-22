//! Reading a `.ics` file: line unfolding, properties with their parameters, and the `VEVENT`s
//! inside.

use crate::civil::Day;
use crate::{Calendar, Event, Instant, Rrule};

/// Read a whole calendar. Nothing in here fails: a property, an event or a tail that does not
/// parse is left out and the rest is kept.
pub fn parse(text: &str) -> Calendar {
    let mut cal = Calendar::default();
    let mut current: Option<Raw> = None;
    for line in unfold(text) {
        let Some((head, value)) = split_property(&line) else { continue };
        let (name, params) = split_name(head);
        match name.as_str() {
            "BEGIN" if value.trim().eq_ignore_ascii_case("VEVENT") => current = Some(Raw::default()),
            "END" if value.trim().eq_ignore_ascii_case("VEVENT") => {
                // An event only counts once its block closed: a file cut in the middle of one
                // loses that event and keeps the ones before it.
                if let Some(raw) = current.take() {
                    if let Some(event) = raw.finish() {
                        cal.events.push(event);
                    }
                }
            }
            "X-WR-CALNAME" | "NAME" if current.is_none() => cal.name = unescape(value).trim().to_string(),
            _ => {
                if let Some(raw) = current.as_mut() {
                    raw.set(&name, &params, value);
                }
            }
        }
    }
    cal
}

/// A date-time value on its own (`UNTIL=20251231`, `UNTIL=20251231T235900Z`).
pub(crate) fn date_value(value: &str) -> Option<Instant> {
    parse_dt(value, &[])
}

/// Unfold the lines of a file: a line that starts with a space or a tab continues the one before.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) if !out.is_empty() => {
                if let Some(last) = out.last_mut() {
                    last.push_str(rest);
                }
            }
            _ => out.push(line.to_string()),
        }
    }
    out
}

/// Split `NAME;PARAM=VALUE:value` at the colon that ends the head. A colon inside a quoted
/// parameter value does not count.
fn split_property(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// `DTSTART;TZID=X`: the name in upper case and the parameters as `(key, value)` pairs.
fn split_name(head: &str) -> (String, Vec<(String, String)>) {
    let mut parts = head.split(';');
    let name = parts.next().unwrap_or("").trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), unquote(v).to_string()))
        .collect();
    (name, params)
}

fn unquote(value: &str) -> &str {
    let v = value.trim();
    match v.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(inner) => inner,
        None => v,
    }
}

/// One `VEVENT` while it is being read.
#[derive(Default)]
struct Raw {
    uid: String,
    summary: String,
    description: String,
    location: String,
    start: Option<Instant>,
    end: Option<Instant>,
    duration: Option<(i32, i32)>,
    tzid: Option<String>,
    rrule: Option<Rrule>,
}

impl Raw {
    fn set(&mut self, name: &str, params: &[(String, String)], value: &str) {
        match name {
            "UID" => self.uid = unescape(value).trim().to_string(),
            "SUMMARY" => self.summary = unescape(value).trim().to_string(),
            "DESCRIPTION" => self.description = unescape(value).trim().to_string(),
            "LOCATION" => self.location = unescape(value).trim().to_string(),
            "DTSTART" => {
                self.start = parse_dt(value, params);
                self.tzid = params.iter().find(|(k, _)| k == "TZID").map(|(_, v)| v.clone());
            }
            "DTEND" => self.end = parse_dt(value, params),
            "DURATION" => self.duration = parse_duration(value),
            "RRULE" => self.rrule = crate::rrule::parse(value),
            _ => {}
        }
    }

    fn finish(self) -> Option<Event> {
        let start = self.start?;
        let all_day = start.is_all_day();
        let end = match self.end {
            Some(end) => end,
            None => match self.duration {
                Some((days, minutes)) => add(start, days, minutes),
                // RFC 5545: an all-day event with no end lasts one day; a timed event with no end
                // is an instant.
                None if all_day => Instant::all_day(start.day.add_days(1)),
                None => start,
            },
        };
        let uid = if self.uid.is_empty() {
            format!("{}-{}-{:?}", self.summary, start.day.index(), start.minute)
        } else {
            self.uid
        };
        Some(Event {
            uid,
            summary: self.summary,
            description: self.description,
            location: self.location,
            start,
            end,
            tzid: self.tzid,
            rrule: self.rrule,
        })
    }
}

/// `20250916`, `20250916T093000`, `20250916T093000Z`, with `VALUE=DATE` or a `TZID`. The time is
/// the wall clock the file writes; `Z` is dropped with the same intent (no zone conversion).
fn parse_dt(value: &str, params: &[(String, String)]) -> Option<Instant> {
    let value = value.trim();
    let is_date = params.iter().any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE"));
    let (date, time) = match value.split_once(['T', 't']) {
        Some((d, t)) => (d, Some(t)),
        None => (value, None),
    };
    let day = ymd(date)?;
    let Some(time) = time.filter(|_| !is_date) else { return Some(Instant::all_day(day)) };
    let digits = time.trim_end_matches(['Z', 'z']);
    if digits.len() < 4 {
        return Some(Instant::all_day(day));
    }
    let hour: u32 = digits.get(0..2)?.parse().ok()?;
    let minute: u32 = digits.get(2..4)?.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(Instant::at(day, u16::try_from(hour * 60 + minute).ok()?))
}

/// `YYYYMMDD` as a day, refusing a date that does not exist.
fn ymd(date: &str) -> Option<Day> {
    let date = date.trim();
    if date.len() < 8 || !date.as_bytes()[..8].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year: i32 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(4..6)?.parse().ok()?;
    let day: u32 = date.get(6..8)?.parse().ok()?;
    Day::from_ymd(year, month, day)
}

/// `P1DT2H`, `PT30M`, `P2W`: (days, minutes). Months and years are not honoured, only days and
/// anything after the `T`.
fn parse_duration(value: &str) -> Option<(i32, i32)> {
    let text = value.trim().to_ascii_uppercase();
    let rest = text.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    let mut days = 0i32;
    let mut minutes = 0i32;
    for (part, in_time) in [(date_part, false), (time_part.unwrap_or(""), true)] {
        let mut number = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() {
                number.push(c);
                continue;
            }
            let n: i32 = number.parse().ok()?;
            number.clear();
            match (c, in_time) {
                ('D', false) => days = days.saturating_add(n),
                ('W', false) => days = days.saturating_add(n.saturating_mul(7)),
                ('H', true) => minutes = minutes.saturating_add(n.saturating_mul(60)),
                ('M', true) => minutes = minutes.saturating_add(n),
                // Seconds, months and years are too fine or too coarse for a month view.
                _ => {}
            }
        }
    }
    Some((days, minutes))
}

fn add(start: Instant, days: i32, minutes: i32) -> Instant {
    let mut day = start.day.add_days(days);
    let Some(minute) = start.minute else { return Instant::all_day(day) };
    let mut total = i32::from(minute) + minutes;
    if total < 0 {
        total = 0;
    }
    if total >= 24 * 60 {
        day = day.add_days(total / (24 * 60));
        total %= 24 * 60;
    }
    Instant { day, minute: u16::try_from(total).ok() }
}

/// Undo the escapes of a text property: `\n`, `\,`, `\;` and `\\`.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolds_continuation_lines() {
        let lines = unfold("SUMMARY:Largo\r\n  y partido\r\nBEGIN:VEVENT\n");
        assert_eq!(lines, ["SUMMARY:Largo y partido", "BEGIN:VEVENT", ""]);
    }

    #[test]
    fn parameters_can_hold_colons_and_commas() {
        let (name, params) = split_name(r#"DTSTART;TZID="Europe/Paris";VALUE=DATE-TIME"#);
        assert_eq!(name, "DTSTART");
        assert_eq!(params[0], ("TZID".to_string(), "Europe/Paris".to_string()));
        assert_eq!(params[1], ("VALUE".to_string(), "DATE-TIME".to_string()));
    }

    #[test]
    fn unescapes_the_text_properties() {
        assert_eq!(unescape(r"a\, b\; c\\d\ne"), "a, b; c\\d\ne");
    }

    #[test]
    fn durations_name_days_and_minutes() {
        assert_eq!(parse_duration("P1DT2H"), Some((1, 120)));
        assert_eq!(parse_duration("PT30M"), Some((0, 30)));
        assert_eq!(parse_duration("P2W"), Some((14, 0)));
        assert_eq!(parse_duration("nonsense"), None);
    }

    #[test]
    fn a_date_that_does_not_exist_is_refused() {
        assert!(parse_dt("20250230", &[]).is_none());
        assert_eq!(parse_dt("20250228", &[]), Some(Instant::all_day(Day::from_ymd(2025, 2, 28).unwrap())));
    }

    #[test]
    fn properties_without_a_colon_are_skipped() {
        let cal = parse("BEGIN:VEVENT\nSUMMARY\nEND:VEVENT\n");
        assert!(cal.events.is_empty());
    }

    #[test]
    fn an_event_without_a_start_is_dropped() {
        let cal = parse("BEGIN:VEVENT\nSUMMARY:Hola\nEND:VEVENT\n");
        assert!(cal.events.is_empty());
    }

    #[test]
    fn a_missing_uid_is_built_from_the_event() {
        let cal = parse("BEGIN:VEVENT\nDTSTART;VALUE=DATE:20250916\nSUMMARY:Hola\nEND:VEVENT\n");
        assert_eq!(cal.events.len(), 1);
        assert!(cal.events[0].uid.starts_with("Hola-"));
    }
}
