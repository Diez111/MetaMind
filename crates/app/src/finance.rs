//! The money view of a note: what came in, what went out, and where it went.
//!
//! A note with `tipo: finanzas` in its front matter is drawn as a small board instead of as text:
//! three totals, a bar per month for the last six, and the categories of the current month with
//! their share. The numbers come from the note itself, in a format a person can write by hand and
//! a machine can read back:
//!
//! ```text
//! ## Ingresos
//! - 2026-11-01 · sueldo · 1.250.000
//!
//! ## Gastos
//! - 2026-11-03 · supermercado · 45.200
//! - 2026-11-05 · transporte · 12.000
//! ```
//!
//! A line is `- <fecha> · <categoría> · <monto>`: the middle dot separates the parts, the amount
//! is taken as it is written (thousands separators are ignored) and the section it sits in says
//! whether the money came in or went out. Anything else in the note is ignored, so the rest of it
//! stays prose.

use calendar_core::Day;
use metamind_ui::{Dom, El, NodeId};

/// Whether an amount came in or went out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    Income,
    Expense,
}

/// One line of money.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub day: Day,
    pub category: String,
    /// In cents, so two amounts add up without rounding.
    pub cents: i64,
    pub flow: Flow,
}

impl Entry {
    /// The amount as it is shown: `$1.250.000` for whole pesos, with cents when there are any.
    pub fn money(&self) -> String {
        money(self.cents)
    }
}

/// `$1.250.000` or `$1.250.000,50`.
pub fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.abs();
    let (whole, rest) = (cents / 100, cents % 100);
    let digits = whole.to_string();
    let mut out = String::with_capacity(digits.len() + 4);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if rest == 0 {
        format!("{sign}${out}")
    } else {
        format!("{sign}${out},{rest:02}")
    }
}

/// `- 2026-11-03 · supermercado · 45.200`, with the flow the section gives it.
fn line_of(text: &str, flow: Flow) -> Option<Entry> {
    let text = text.trim_start().strip_prefix("- ")?;
    let mut parts = text.split('·').map(str::trim);
    let day = parse_day(parts.next()?)?;
    let category = parts.next()?.to_string();
    let amount = parts.next()?;
    if category.is_empty() {
        return None;
    }
    Some(Entry { day, category, cents: parse_money(amount)?, flow })
}

/// `45.200`, `45,200.50`, `45200` and `$45.200` all mean the same: dots and commas group, the
/// last separator with one or two digits is the decimal one, and anything else is ignored.
pub fn parse_money(text: &str) -> Option<i64> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    if digits.is_empty() {
        return None;
    }
    let last = digits.rfind(['.', ',']);
    let (whole, frac) = match last {
        Some(at) if digits[at + 1..].len() <= 2 && !digits[at + 1..].is_empty() => {
            (&digits[..at], &digits[at + 1..])
        }
        _ => (&digits[..], ""),
    };
    let whole: String = whole.chars().filter(char::is_ascii_digit).collect();
    let frac: String = frac.chars().filter(char::is_ascii_digit).collect();
    let whole = if whole.is_empty() { "0".to_string() } else { whole };
    let cents = whole.parse::<i64>().ok()? * 100;
    let part = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        _ => frac.parse::<i64>().ok()?,
    };
    Some(cents + part)
}

fn parse_day(text: &str) -> Option<Day> {
    let text = text.trim();
    let mut parts = text.split('-');
    let y = parts.next()?.trim().parse::<i32>().ok()?;
    let m = parts.next()?.trim().parse::<u32>().ok()?;
    let d = parts.next()?.trim().parse::<u32>().ok()?;
    Day::from_ymd(y, m, d)
}

/// Every entry of a note, in the order it is written.
pub fn entries(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut flow = Flow::Expense;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            let name = heading.trim_start_matches('#').trim().to_ascii_lowercase();
            if name.starts_with("ingreso") || name.starts_with("income") {
                flow = Flow::Income;
            } else if name.starts_with("gasto") || name.starts_with("egreso") || name.starts_with("expense") {
                flow = Flow::Expense;
            } else if name.starts_with("finanzas") || name.is_empty() {
                // The title of the note says nothing about the flow: the next section will.
            }
            continue;
        }
        if let Some(entry) = line_of(trimmed, flow) {
            out.push(entry);
        }
    }
    out
}

/// One month of the board.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Month {
    pub year: i32,
    pub month: u32,
    pub income: i64,
    pub expense: i64,
}

impl Month {
    pub fn balance(&self) -> i64 {
        self.income - self.expense
    }
    pub fn label(&self) -> String {
        format!(
            "{} {}",
            MONTHS.get(self.month as usize - 1).copied().unwrap_or(""),
            &self.year.to_string()[2..]
        )
    }
}

const MONTHS: [&str; 12] =
    ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];

/// The last `months` months, oldest first, with the money of each. A month with nothing in it is
/// still a bar (of zero), because a gap in a chart is a question, not an answer.
pub fn months(entries: &[Entry], last: Day, count: usize) -> Vec<Month> {
    let (ly, lm, _) = last.ymd();
    let mut out = Vec::with_capacity(count);
    for back in (0..count).rev() {
        let total = i64::from(ly) * 12 + i64::from(lm) - 1 - back as i64;
        let year = total.div_euclid(12) as i32;
        let month = total.rem_euclid(12) as u32 + 1;
        let mut m = Month { year, month, income: 0, expense: 0 };
        for e in entries {
            let (ey, em, _) = e.day.ymd();
            if ey == year && em == month {
                match e.flow {
                    Flow::Income => m.income += e.cents,
                    Flow::Expense => m.expense += e.cents,
                }
            }
        }
        out.push(m);
    }
    out
}

/// The categories of one month, biggest first, with how much of the month's spending each is.
pub fn categories(entries: &[Entry], year: i32, month: u32) -> Vec<(String, i64)> {
    let mut map: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for e in entries {
        let (y, m, _) = e.day.ymd();
        if e.flow == Flow::Expense && y == year && m == month {
            *map.entry(e.category.clone()).or_default() += e.cents;
        }
    }
    let mut out: Vec<(String, i64)> = map.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Whether the note on screen is one of these boards.
pub fn is_finance(app: &crate::shell::App) -> bool {
    let Some(tab) = app.tab() else { return false };
    let Some(entry) = app.store.index.get(&tab.path) else { return false };
    let Some(meta) = entry.meta() else { return false };
    let Some(props) = meta.frontmatter.as_ref() else { return false };
    props.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("tipo")
            && v.as_str()
                .is_some_and(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "finanzas" | "finance"))
    })
}

/// The board: three totals, a bar per month and the categories of this one.
pub fn pane(app: &crate::shell::App, dom: &mut Dom, parent: NodeId, width: f32) -> NodeId {
    let Some(tab) = app.tab() else { return parent };
    let text = tab.editor.buf.text();
    let entries = entries(&text);
    let today = crate::calendar::Calendar::today();
    let (year, month, _) = today.ymd();
    let now = months(&entries, today, 6);
    let current = now.last().cloned().unwrap_or_default();
    let body = dom.add(parent, El::div().class("view-content finance"));

    let head = dom.add(body, El::div().class("finance-head"));
    dom.add(head, El::div().class("finance-title").text("Finanzas"));
    dom.add(head, El::div().class("finance-range").text(format!("{} · {}/{}", tab.path, month, year)));

    // Three totals: what came in, what went out and what is left of the month.
    let cards = dom.add(body, El::div().class("finance-cards"));
    for (label, value, class) in [
        ("Ingresos del mes", current.income, "is-income"),
        ("Gastos del mes", current.expense, "is-expense"),
        ("Balance", current.balance(), if current.balance() < 0 { "is-negative" } else { "is-positive" }),
    ] {
        let card = dom.add(cards, El::div().class("finance-card").class(class));
        dom.add(card, El::div().class("finance-card-label").text(label));
        dom.add(card, El::div().class("finance-card-value").text(money(value)));
    }

    if entries.is_empty() {
        let empty = dom.add(body, El::div().class("finance-empty"));
        dom.add(empty, El::div().class("finance-empty-title").text("Todavía no hay movimientos"));
        dom.add(
            empty,
            El::div().class("finance-empty-hint").text(
                "Agregá líneas como «- 2026-11-03 · supermercado · 45.200» bajo un título ## Ingresos \
                 o ## Gastos, y esta tabla se arma sola.",
            ),
        );
        return body;
    }

    // A bar per month, income and expenses side by side.
    let chart = dom.add(body, El::div().class("finance-chart"));
    dom.add(chart, El::div().class("finance-chart-title").text("Últimos 6 meses"));
    let top = now.iter().map(|m| m.income.max(m.expense)).max().unwrap_or(0).max(1);
    for m in &now {
        let row = dom.add(chart, El::div().class("finance-month"));
        dom.add(row, El::div().class("finance-month-label").text(m.label()));
        let bars = dom.add(row, El::div().class("finance-bars"));
        for (value, class) in [(m.income, "is-income"), (m.expense, "is-expense")] {
            let bar =
                dom.add(bars, El::div().class("finance-bar").class(class).class_if(value == 0, "is-empty"));
            let share = (value as f32 / top as f32).clamp(0.0, 1.0);
            dom.nodes[bar].el.inline_style = Some(format!("width: {:.1}%", share * 100.0));
        }
        dom.add(row, El::div().class("finance-month-total").text(money(m.balance())));
    }

    // Where the money went this month.
    let cats = categories(&entries, year, month);
    if !cats.is_empty() {
        let total: i64 = cats.iter().map(|(_, v)| *v).sum();
        let list = dom.add(body, El::div().class("finance-cats"));
        dom.add(list, El::div().class("finance-chart-title").text("En qué se fue este mes"));
        for (name, value) in cats.iter().take(8) {
            let row = dom.add(list, El::div().class("finance-cat"));
            dom.add(row, El::div().class("finance-cat-name").text(name.clone()));
            let track = dom.add(row, El::div().class("finance-cat-track"));
            let fill = dom.add(track, El::div().class("finance-cat-bar"));
            let share = (*value as f32 / total.max(1) as f32).clamp(0.0, 1.0);
            dom.nodes[fill].el.inline_style = Some(format!("width: {:.1}%", share * 100.0));
            dom.add(row, El::div().class("finance-cat-value").text(money(*value)));
        }
    }
    let _ = width;
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Finanzas\n\n## Ingresos\n- 2026-11-01 · sueldo · 1.250.000\n\n## Gastos\n- 2026-11-03 · supermercado · 45.200\n- 2026-11-05 · transporte · 12.000\n- 2026-10-28 · supermercado · 30.000\n";

    #[test]
    fn the_lines_of_a_note_are_read_with_their_flow() {
        let e = entries(NOTE);
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].flow, Flow::Income);
        assert_eq!(e[0].cents, 125_000_000);
        assert_eq!(e[1].category, "supermercado");
        assert_eq!(e[1].cents, 4_520_000);
        assert_eq!(e[0].money(), "$1.250.000");
        // A line that is not one of these is prose and is left alone.
        assert!(line_of("- una nota cualquiera", Flow::Expense).is_none());
    }

    #[test]
    fn an_amount_is_read_the_way_a_person_writes_it() {
        assert_eq!(parse_money("45.200"), Some(4_520_000));
        assert_eq!(parse_money("45200"), Some(4_520_000));
        assert_eq!(parse_money("$ 1.250.000"), Some(125_000_000));
        assert_eq!(parse_money("1.250,50"), Some(125_050));
        assert_eq!(parse_money("12,5"), Some(1_250));
        assert_eq!(parse_money("nada"), None);
    }

    #[test]
    fn the_months_come_oldest_first_and_the_categories_biggest_first() {
        let e = entries(NOTE);
        let today = Day::from_ymd(2026, 11, 23).unwrap();
        let ms = months(&e, today, 3);
        assert_eq!(ms.len(), 3);
        assert_eq!((ms[2].year, ms[2].month), (2026, 11));
        assert_eq!(ms[2].income, 125_000_000);
        assert_eq!(ms[2].expense, 5_720_000);
        assert_eq!(ms[2].balance(), 119_280_000);
        assert_eq!(ms[1].expense, 3_000_000, "octubre");
        let cats = categories(&e, 2026, 11);
        assert_eq!(cats[0].0, "supermercado");
        assert_eq!(cats[0].1, 4_520_000);
        assert_eq!(cats.len(), 2);
    }
}
