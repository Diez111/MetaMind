//! Civil dates as day numbers: the same arithmetic as `vault_core::fsio::civil`, with the
//! inverse, so a calendar day can be added, compared and turned back into a date without a
//! time-zone database.

/// A calendar day, counted from 1970-01-01 (day 0).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Day(i32);

pub fn is_leap_year(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Length of `m` (1-12) in `y`; 0 for a month that does not exist.
pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i32::try_from(m).unwrap_or(1) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i32::try_from(d).unwrap_or(1) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day number.
pub fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i32::from(m <= 2), m, d)
}

impl Day {
    /// The day of a civil date, or `None` when the date does not exist (31 April, 29 February on
    /// a common year, month 13).
    pub fn from_ymd(y: i32, m: u32, d: u32) -> Option<Day> {
        if d == 0 || d > days_in_month(y, m) {
            return None;
        }
        Some(Day(days_from_civil(y, m, d)))
    }

    pub fn from_index(index: i32) -> Day {
        Day(index)
    }

    pub fn index(self) -> i32 {
        self.0
    }

    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0)
    }

    /// Weekday with Sunday = 0, as `crate::datetime::local` counts them.
    pub fn weekday(self) -> u32 {
        (self.0 + 4).rem_euclid(7) as u32
    }

    pub fn add_days(self, days: i32) -> Day {
        Day(self.0 + days)
    }

    /// The first day of this day's month.
    pub fn month_start(self) -> Day {
        let (y, m, _) = self.ymd();
        Day(days_from_civil(y, m, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_dates() {
        for (y, m, d) in [(1970, 1, 1), (2025, 9, 16), (2024, 2, 29), (1969, 12, 31), (2100, 3, 1)] {
            let day = Day::from_ymd(y, m, d).unwrap();
            assert_eq!(day.ymd(), (y, m, d));
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
    }

    #[test]
    fn epoch_is_a_thursday() {
        assert_eq!(Day::from_index(0).weekday(), 4);
    }

    #[test]
    fn refuses_dates_that_do_not_exist() {
        assert!(Day::from_ymd(2025, 2, 29).is_none());
        assert!(Day::from_ymd(2024, 2, 29).is_some());
        assert!(Day::from_ymd(2025, 4, 31).is_none());
        assert!(Day::from_ymd(2025, 13, 1).is_none());
        assert!(Day::from_ymd(2025, 0, 1).is_none());
        assert!(Day::from_ymd(2025, 1, 0).is_none());
    }
}
