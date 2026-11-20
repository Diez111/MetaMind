//! Values a query sees, and the comparisons the language defines on them.

use md_engine::frontmatter::{self, Value as Yaml};
use std::cmp::Ordering;
use std::fmt;

/// A calendar day, the only date unit this subset compares.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// `YYYY-MM-DD`. The day is not checked against the month length.
    pub fn parse(s: &str) -> Option<Date> {
        let s = s.trim();
        if s.len() != 10 || !s.is_char_boundary(10) {
            return None;
        }
        let b = s.as_bytes();
        if b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let part = |r: std::ops::Range<usize>| -> Option<u32> {
            let t = &s[r];
            t.bytes().all(|c| c.is_ascii_digit()).then(|| t.parse().ok())?
        };
        let (month, day) = (part(5..7)?, part(8..10)?);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Date { year: part(0..4)? as i32, month, day })
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A front matter value as a query compares and shows it.
#[derive(Clone, PartialEq, Debug)]
pub enum Val {
    /// A missing key, `null`, an empty string or an empty list.
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Date(Date),
    List(Vec<Val>),
}

impl Val {
    /// A front matter value. An ISO date reads as a date, lists as lists; a nested map has no
    /// value of its own (its fields are reached with `key.sub`).
    pub fn from_yaml(v: &Yaml) -> Val {
        match v {
            Yaml::Null | Yaml::Map(_) => Val::Null,
            Yaml::Bool(b) => Val::Bool(*b),
            Yaml::Number(n) => Val::Num(*n),
            Yaml::String(s) => Date::parse(s).map_or_else(|| Val::Str(s.clone()), Val::Date),
            Yaml::List(items) => Val::List(items.iter().map(Val::from_yaml).collect()),
        }
    }

    /// Nothing to show: the value is missing or empty for its type.
    pub fn is_empty(&self) -> bool {
        match self {
            Val::Null => true,
            Val::Str(s) => s.is_empty(),
            Val::List(l) => l.is_empty(),
            _ => false,
        }
    }

    /// The cell text: numbers without a trailing `.0`, lists as their items separated by commas.
    pub fn text(&self) -> String {
        match self {
            Val::Null => String::new(),
            Val::Bool(b) => b.to_string(),
            Val::Num(n) => frontmatter::fmt_number(*n),
            Val::Str(s) => s.clone(),
            Val::Date(d) => d.to_string(),
            Val::List(items) => {
                let parts: Vec<String> = items.iter().map(Val::text).filter(|t| !t.is_empty()).collect();
                parts.join(", ")
            }
        }
    }

    /// The number this value stands for, when it has one: numbers, and text spelled as one.
    fn number(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
}

impl From<&crate::query::Lit> for Val {
    fn from(l: &crate::query::Lit) -> Val {
        match l {
            crate::query::Lit::Null => Val::Null,
            crate::query::Lit::Str(s) => Val::Str(s.clone()),
            crate::query::Lit::Num(n) => Val::Num(*n),
            crate::query::Lit::Date(d) => Val::Date(*d),
        }
    }
}

/// Equality as `=` uses it. Text ignores case and surrounding spaces; `[[Note]]` and `Note` are
/// not the same, because only front matter is read. A list equals an item it contains. `null` is
/// equal to a missing value, an empty string and an empty list.
pub fn loose_eq(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Null, v) | (v, Val::Null) => v.is_empty(),
        (Val::List(x), Val::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| loose_eq(a, b)),
        (Val::List(x), v) | (v, Val::List(x)) => x.iter().any(|i| loose_eq(i, v)),
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Date(x), Val::Date(y)) => x == y,
        (Val::Date(x), Val::Str(s)) | (Val::Str(s), Val::Date(x)) => Date::parse(s) == Some(*x),
        (Val::Num(_), _) | (_, Val::Num(_)) => match (a.number(), b.number()) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        (Val::Str(x), Val::Str(y)) => x.trim().eq_ignore_ascii_case(y.trim()),
        _ => false,
    }
}

/// Ordering as `<`, `<=`, `>`, `>=` use it: `None` when the two values cannot be compared,
/// which is the case between a date and text that is not a date, a number and text that is not
/// one, and a list and a single value.
pub fn partial_order(a: &Val, b: &Val) -> Option<Ordering> {
    match (a, b) {
        (Val::Date(x), Val::Date(y)) => Some(x.cmp(y)),
        (Val::Date(x), Val::Str(s)) => Date::parse(s).map(|y| x.cmp(&y)),
        (Val::Str(s), Val::Date(y)) => Date::parse(s).map(|x| x.cmp(y)),
        (Val::Num(x), Val::Num(y)) => x.partial_cmp(y),
        (Val::Num(x), Val::Str(s)) => s.trim().parse::<f64>().ok()?.partial_cmp(x).map(Ordering::reverse),
        (Val::Str(s), Val::Num(y)) => s.trim().parse::<f64>().ok()?.partial_cmp(y),
        (Val::Bool(x), Val::Bool(y)) => Some(x.cmp(y)),
        (Val::Str(x), Val::Str(y)) => Some(text_order(x, y)),
        _ => None,
    }
}

/// A total order for `SORT`: [`partial_order`] where the values compare, otherwise the kind of
/// value decides (dates and numbers first, then text, then booleans, then lists).
pub fn order(a: &Val, b: &Val) -> Ordering {
    if let Some(o) = partial_order(a, b) {
        return o;
    }
    fn rank(v: &Val) -> u8 {
        match v {
            Val::Num(_) | Val::Date(_) => 0,
            Val::Str(_) => 1,
            Val::Bool(_) => 2,
            Val::List(_) => 3,
            Val::Null => 4,
        }
    }
    rank(a).cmp(&rank(b)).then_with(|| a.text().to_lowercase().cmp(&b.text().to_lowercase()))
}

fn text_order(a: &str, b: &str) -> Ordering {
    a.trim().to_lowercase().cmp(&b.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Val {
        Val::Str(v.into())
    }

    #[test]
    fn iso_dates_are_dates() {
        assert_eq!(Date::parse("2026-11-09"), Some(Date { year: 2026, month: 11, day: 9 }));
        assert_eq!(Date::parse("2026-13-01"), None);
        assert_eq!(Date::parse("2026-11-9"), None);
        assert_eq!(Date::parse("hoy"), None);
        assert_eq!(Val::from_yaml(&Yaml::String("2026-11-09".into())).text(), "2026-11-09");
        assert!(matches!(Val::from_yaml(&Yaml::String("2026-11-09".into())), Val::Date(_)));
        // Dated text does not become a date on display, only on comparison.
        assert_eq!(Val::from_yaml(&Yaml::String("hoy".into())).text(), "hoy");
    }

    #[test]
    fn equality_ignores_case_and_blank_text() {
        assert!(loose_eq(&s("Materia"), &s("materia")));
        assert!(loose_eq(&s("  materia "), &s("MATERIA")));
        assert!(!loose_eq(&s("materia"), &s("mapa")));
        assert!(loose_eq(&Val::Null, &s("")));
        assert!(loose_eq(&Val::Null, &Val::List(Vec::new())));
        assert!(loose_eq(&Val::Num(1.0), &s("1")));
        assert!(loose_eq(&Val::List(vec![s("a"), s("b")]), &s("b")));
        assert!(!loose_eq(&Val::List(vec![s("a")]), &s("b")));
    }

    #[test]
    fn ordering_puts_dates_by_day_and_numbers_by_value() {
        let (a, b) = (Val::Date(Date { year: 2026, month: 11, day: 9 }), s("2026-11-11"));
        assert_eq!(order(&a, &b), Ordering::Less);
        assert_eq!(order(&Val::Num(9.0), &s("10")), Ordering::Less);
        assert_eq!(order(&s("Zeta"), &s("alfa")), Ordering::Greater);
        assert_eq!(order(&Val::Num(10.0), &Val::Num(9.0)), Ordering::Greater);
        // Values of different kinds sort together without mixing.
        assert_eq!(order(&a, &s("hoy")), Ordering::Less);
    }

    #[test]
    fn values_that_do_not_compare_have_no_order() {
        let date = Val::Date(Date { year: 2026, month: 11, day: 11 });
        assert_eq!(partial_order(&date, &s("2026-11-10")), Some(Ordering::Greater));
        assert_eq!(partial_order(&date, &s("{{date:YYYY-MM-DD}}")), None);
        assert_eq!(partial_order(&Val::Num(1.0), &s("uno")), None);
        assert_eq!(partial_order(&Val::List(vec![s("1")]), &Val::Num(1.0)), None);
    }
}
