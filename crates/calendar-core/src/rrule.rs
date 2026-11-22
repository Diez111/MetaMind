//! `RRULE` parsing and the days each instance falls on.
//!
//! Only what a month view needs: `FREQ` in `DAILY`/`WEEKLY`/`MONTHLY`/`YEARLY`, `INTERVAL`,
//! `COUNT`, `UNTIL` and `BYDAY` (honoured for weekly rules, which is how Google writes a series
//! that repeats on more than one weekday a week). Everything else — `BYSETPOS`, `BYMONTHDAY`,
//! `BYMONTH`, `WKST` — is parsed past and ignored, which is reported as a limitation.

use crate::{Day, Instant};

/// How many instances of one rule are looked at before stopping. A rule with no `COUNT` and no
/// `UNTIL` still ends at the edge of the window the caller asks for; this is the belt for a file
/// that says something the parser cannot honour.
pub(crate) const MAX_STEPS: u32 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rrule {
    pub freq: Freq,
    /// At least 1: `INTERVAL=0` is read as 1 instead of looping forever.
    pub interval: u32,
    /// Instances in total, `DTSTART` included.
    pub count: Option<u32>,
    /// Last instance, inclusive.
    pub until: Option<Instant>,
    /// Weekdays (`Sunday = 0`) of a weekly rule; empty means "the weekday of `DTSTART`".
    pub by_day: Vec<u32>,
}

/// The Monday of the week `day` falls in.
fn week_start(day: Day) -> Day {
    day.add_days(-(((day.weekday() + 6) % 7) as i32))
}

impl Rrule {
    /// The start days of instance `k`, with `k = 0` being `DTSTART`. Empty when that instance has
    /// no date: the 31st in a 30-day month, the 29th of February in a common year. Ascending.
    pub fn days_of(&self, start: Instant, k: u32) -> Vec<Day> {
        let step = k.saturating_mul(self.interval);
        match self.freq {
            Freq::Daily => vec![start.day.add_days(step as i32)],
            Freq::Weekly => {
                if self.by_day.is_empty() {
                    return vec![start.day.add_days(step.saturating_mul(7) as i32)];
                }
                // A weekly rule with `BYDAY` repeats inside a week that starts on Monday
                // (`WKST` default), and never before `DTSTART`.
                let week = week_start(start.day).add_days(step.saturating_mul(7) as i32);
                let mut days: Vec<Day> = self
                    .by_day
                    .iter()
                    .map(|&d| week.add_days(((d + 6) % 7) as i32))
                    .filter(|&d| d >= start.day)
                    .collect();
                days.sort_unstable();
                days.dedup();
                days
            }
            Freq::Monthly => {
                let (y, m, d) = start.day.ymd();
                let months = i64::from(y) * 12 + i64::from(m) - 1 + i64::from(step);
                let y = months.div_euclid(12) as i32;
                let m = months.rem_euclid(12) as u32 + 1;
                Day::from_ymd(y, m, d).into_iter().collect()
            }
            Freq::Yearly => {
                let (y, m, d) = start.day.ymd();
                Day::from_ymd(y + step as i32, m, d).into_iter().collect()
            }
        }
    }

    /// A day no instance at or after `k` can come before: what the expansion stops on once it has
    /// passed the end of the window. For a monthly or yearly rule whose date does not exist in a
    /// month, the first of that month (year) is still a safe bound.
    pub fn anchor(&self, start: Instant, k: u32) -> Day {
        let step = k.saturating_mul(self.interval);
        match self.freq {
            Freq::Daily => start.day.add_days(step as i32),
            // The Monday of the instance's week: no day of the instance can come before it.
            Freq::Weekly if self.by_day.is_empty() => start.day.add_days(step.saturating_mul(7) as i32),
            Freq::Weekly => week_start(start.day).add_days(step.saturating_mul(7) as i32),
            Freq::Monthly => {
                let (y, m, _) = start.day.ymd();
                let months = i64::from(y) * 12 + i64::from(m) - 1 + i64::from(step);
                let y = months.div_euclid(12) as i32;
                let m = months.rem_euclid(12) as u32 + 1;
                Day::from_ymd(y, m, 1).unwrap_or(start.day)
            }
            Freq::Yearly => {
                let (y, _, _) = start.day.ymd();
                Day::from_ymd(y + step as i32, 1, 1).unwrap_or(start.day)
            }
        }
    }
}

/// Read a `RRULE` value (`FREQ=WEEKLY;INTERVAL=2;COUNT=8`). `None` when there is no frequency the
/// view can expand, which leaves the event as a single occurrence.
pub fn parse(value: &str) -> Option<Rrule> {
    let mut freq = None;
    let mut interval = 1u32;
    let mut count = None;
    let mut until = None;
    let mut by_day = Vec::new();
    for part in value.split(';') {
        let Some((key, val)) = part.split_once('=') else { continue };
        let val = val.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = match val.to_ascii_uppercase().as_str() {
                    "DAILY" => Some(Freq::Daily),
                    "WEEKLY" => Some(Freq::Weekly),
                    "MONTHLY" => Some(Freq::Monthly),
                    "YEARLY" => Some(Freq::Yearly),
                    _ => None,
                };
            }
            "INTERVAL" => interval = val.parse::<u32>().ok().filter(|n| *n > 0).unwrap_or(1),
            "COUNT" => count = val.parse::<u32>().ok().filter(|n| *n > 0),
            "UNTIL" => until = crate::parse::date_value(val),
            "BYDAY" => by_day = val.split(',').filter_map(weekday_number).collect(),
            _ => {}
        }
    }
    let freq = freq?;
    if freq != Freq::Weekly {
        // Only a weekly rule repeats on more than one weekday; elsewhere `BYDAY` narrows within
        // the month and is not honoured (see the module note).
        by_day.clear();
    }
    Some(Rrule { freq, interval: interval.max(1), count, until, by_day })
}

/// `MO`..`SU` as a weekday number, Sunday = 0. A leading ordinal (`2MO`, `-1FR`) is dropped.
fn weekday_number(token: &str) -> Option<u32> {
    let token = token.trim().to_ascii_uppercase();
    let code = token.get(token.len().saturating_sub(2)..)?;
    Some(match code {
        "MO" => 1,
        "TU" => 2,
        "WE" => 3,
        "TH" => 4,
        "FR" => 5,
        "SA" => 6,
        "SU" => 0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civil::Day as DayType;

    fn date(y: i32, m: u32, d: u32) -> DayType {
        DayType::from_ymd(y, m, d).unwrap()
    }

    #[test]
    fn parses_the_parts_it_supports() {
        let r = parse("FREQ=WEEKLY;INTERVAL=2;COUNT=5;BYDAY=MO,WE").unwrap();
        assert_eq!(r.freq, Freq::Weekly);
        assert_eq!(r.interval, 2);
        assert_eq!(r.count, Some(5));
        assert_eq!(r.by_day, vec![1, 3]);
        assert!(parse("FREQ=HOURLY").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn interval_zero_is_one() {
        assert_eq!(parse("FREQ=DAILY;INTERVAL=0").unwrap().interval, 1);
    }

    #[test]
    fn monthly_skips_a_day_the_month_does_not_have() {
        // 31 January, repeating monthly: February has no 31st, so that month is skipped.
        let r = parse("FREQ=MONTHLY").unwrap();
        let start = Instant::all_day(date(2025, 1, 31));
        assert!(r.days_of(start, 1).is_empty());
        assert_eq!(r.days_of(start, 2), vec![date(2025, 3, 31)]);
    }

    #[test]
    fn yearly_skips_29_february_on_a_common_year() {
        let r = parse("FREQ=YEARLY").unwrap();
        let start = Instant::all_day(date(2024, 2, 29));
        assert!(r.days_of(start, 1).is_empty());
        assert_eq!(r.days_of(start, 4), vec![date(2028, 2, 29)]);
    }

    #[test]
    fn weekly_by_day_stays_in_the_same_week() {
        let r = parse("FREQ=WEEKLY;BYDAY=MO,WE").unwrap();
        // 2025-09-16 is a Tuesday. The week of DTSTART starts on Monday the 15th, which is before
        // the event and therefore not an instance; the next week starts on the 22nd.
        let start = Instant::all_day(date(2025, 9, 16));
        assert_eq!(r.days_of(start, 0), vec![date(2025, 9, 17)]);
        assert_eq!(r.days_of(start, 1), vec![date(2025, 9, 22), date(2025, 9, 24)]);
    }

    #[test]
    fn until_can_be_a_date_or_a_date_time() {
        assert_eq!(
            parse("FREQ=DAILY;UNTIL=20251231").unwrap().until,
            Some(Instant::all_day(date(2025, 12, 31)))
        );
        assert_eq!(
            parse("FREQ=DAILY;UNTIL=20251231T235900Z").unwrap().until,
            Some(Instant::at(date(2025, 12, 31), 23 * 60 + 59))
        );
    }
}
