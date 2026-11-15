//! Local time and date formatting for daily notes and templates.
//!
//! Formats use the token set of the vault's existing settings (`YYYY-MM-DD`, `dddd`, `HH:mm`,
//! `[literal]`), so `daily-notes.json` and `templates.json` written by other apps keep working.

use vault_core::fsio;

/// Seconds east of UTC for the local zone at `secs` since the epoch.
pub fn local_offset(secs: u64) -> i64 {
    imp::offset(secs)
}

/// Local civil time: (year, month, day, hour, minute, second, weekday with Sunday = 0).
pub fn local(secs: u64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let shifted = (secs as i64 + local_offset(secs)).max(0) as u64;
    let (y, mo, d, h, mi, s) = fsio::civil(shifted);
    // 1970-01-01 was a Thursday.
    let wd = ((shifted / 86400 + 4) % 7) as u32;
    (y, mo, d, h, mi, s, wd)
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Format local time `secs` with a date pattern.
pub fn format(pattern: &str, secs: u64) -> String {
    let (y, mo, d, h, mi, s, wd) = local(secs);
    let doy = {
        let days = |y: i64, m: u32, d: u32| -> i64 {
            // Days from a civil date (the inverse of `fsio::civil`).
            let y = if m <= 2 { y - 1 } else { y };
            let era = y.div_euclid(400);
            let yoe = y - era * 400;
            let mp = (m as i64 + 9) % 12;
            let doy = (153 * mp + 2) / 5 + d as i64 - 1;
            era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy
        };
        days(y, mo, d) - days(y, 1, 1) + 1
    };
    let mut out = String::new();
    let mut rest = pattern;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').unwrap_or(r.len());
            out.push_str(&r[..end]);
            rest = r.get(end + 1..).unwrap_or("");
            continue;
        }
        // Longest token first.
        let tokens: [(&str, String); 22] = [
            ("YYYY", format!("{y:04}")),
            ("MMMM", MONTHS[mo as usize - 1].to_string()),
            ("dddd", DAYS[wd as usize].to_string()),
            ("DDDD", format!("{doy:03}")),
            ("MMM", MONTHS[mo as usize - 1][..3].to_string()),
            ("ddd", DAYS[wd as usize][..3].to_string()),
            ("YY", format!("{:02}", y.rem_euclid(100))),
            ("MM", format!("{mo:02}")),
            ("DD", format!("{d:02}")),
            ("Do", ordinal(d)),
            ("HH", format!("{h:02}")),
            ("hh", format!("{:02}", hour12(h))),
            ("mm", format!("{mi:02}")),
            ("ss", format!("{s:02}")),
            ("M", mo.to_string()),
            ("D", d.to_string()),
            ("d", wd.to_string()),
            ("H", h.to_string()),
            ("h", hour12(h).to_string()),
            ("m", mi.to_string()),
            ("A", if h < 12 { "AM" } else { "PM" }.to_string()),
            ("a", if h < 12 { "am" } else { "pm" }.to_string()),
        ];
        match tokens.iter().find(|(t, _)| rest.starts_with(t)) {
            Some((t, v)) => {
                out.push_str(v);
                rest = &rest[t.len()..];
            }
            None => {
                let c = rest.chars().next().unwrap();
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

fn hour12(h: u32) -> u32 {
    match h % 12 {
        0 => 12,
        x => x,
    }
}

fn ordinal(d: u32) -> String {
    let suffix = match (d % 10, d % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{d}{suffix}")
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::{c_char, c_int, c_long};

    /// `struct tm` as laid out by glibc, musl and macOS.
    #[repr(C)]
    struct Tm {
        sec: c_int,
        min: c_int,
        hour: c_int,
        mday: c_int,
        mon: c_int,
        year: c_int,
        wday: c_int,
        yday: c_int,
        isdst: c_int,
        gmtoff: c_long,
        zone: *const c_char,
    }

    extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }

    // `c_long` is i32 on 32-bit targets, so the conversion is only a no-op on 64-bit ones.
    #[allow(clippy::useless_conversion)]
    pub fn offset(secs: u64) -> i64 {
        let t = secs as i64;
        let mut tm = std::mem::MaybeUninit::<Tm>::uninit();
        // SAFETY: both pointers are valid for the call; `localtime_r` is thread-safe and fills
        // `tm` completely when it returns non-null.
        let r = unsafe { localtime_r(&t, tm.as_mut_ptr()) };
        if r.is_null() {
            return 0;
        }
        // SAFETY: initialised by the successful call above.
        i64::from(unsafe { tm.assume_init() }.gmtoff)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    #[repr(C)]
    struct SystemTime([u16; 8]);

    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(out: *mut TimeZoneInformation) -> u32;
    }

    /// The current zone's offset; Windows does not expose past or future transitions here, so
    /// `secs` is not used.
    pub fn offset(_secs: u64) -> i64 {
        let mut tz = std::mem::MaybeUninit::<TimeZoneInformation>::zeroed();
        // SAFETY: the pointer is valid and the struct matches the Win32 layout.
        let r = unsafe { GetTimeZoneInformation(tz.as_mut_ptr()) };
        // SAFETY: zero-initialised and filled by the call.
        let tz = unsafe { tz.assume_init() };
        const DAYLIGHT: u32 = 2;
        const INVALID: u32 = u32::MAX;
        if r == INVALID {
            return 0;
        }
        let bias = tz.bias + if r == DAYLIGHT { tz.daylight_bias } else { tz.standard_bias };
        -(bias as i64) * 60
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn offset(_secs: u64) -> i64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formatting with the local offset removed, so results do not depend on the test machine.
    fn utc(pattern: &str, secs: u64) -> String {
        let off = local_offset(secs);
        format(pattern, (secs as i64 - off) as u64)
    }

    #[test]
    fn formats_tokens_and_literals() {
        // 2024-02-29 12:34:56 UTC, a Thursday.
        let t = 1_709_210_096;
        assert_eq!(utc("YYYY-MM-DD", t), "2024-02-29");
        assert_eq!(utc("dddd, MMMM Do YYYY", t), "Thursday, February 29th 2024");
        assert_eq!(utc("ddd D MMM YY", t), "Thu 29 Feb 24");
        assert_eq!(utc("HH:mm:ss h A", t), "12:34:56 12 PM");
        assert_eq!(utc("[Week of] YYYY/M/D", t), "Week of 2024/2/29");
        assert_eq!(utc("DDDD", t), "060");
    }

    #[test]
    fn local_offset_is_sane() {
        let off = local_offset(1_709_210_096);
        assert!((-14 * 3600..=14 * 3600).contains(&off));
    }
}
