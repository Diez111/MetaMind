//! The plan as a timeline: a note with `tipo: gantt` is drawn as bars over the weeks it covers.
//!
//! The note is wrote by hand, one line per task:
//!
//! ```text
//! # Plan
//!
//! - Diseño · 2026-11-02 · 2026-11-08
//! - Implementación · 2026-11-09 · 2026-11-22
//! - Entrega · 2026-11-24 · 2026-11-24
//! ```
//!
//! Each line is `- <nombre> · <inicio> · <fin>`, both dates inclusive; a task with the same day at
//! both ends is a milestone (a diamond, not a bar). The chart covers from the first start to the
//! last end, with a line for today when today is inside that range, so a plan shows how late it is
//! without a second look.

use calendar_core::Day;
use metamind_ui::{Dom, El, NodeId};

/// One bar of the timeline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Task {
    pub name: String,
    pub start: Day,
    pub end: Day,
}

impl Task {
    /// A task that starts and ends the same day.
    pub fn milestone(&self) -> bool {
        self.start == self.end
    }
}

/// `- nombre · 2026-11-02 · 2026-11-08`.
fn line_of(text: &str) -> Option<Task> {
    let text = text.trim_start().strip_prefix("- ")?;
    let mut parts = text.split('·').map(str::trim);
    let name = parts.next()?.to_string();
    let start = parse_day(parts.next()?)?;
    let end = parse_day(parts.next()?).unwrap_or(start);
    if name.is_empty() {
        return None;
    }
    Some(Task { name, start, end: end.max(start) })
}

fn parse_day(text: &str) -> Option<Day> {
    let mut parts = text.trim().split('-');
    let y = parts.next()?.trim().parse::<i32>().ok()?;
    let m = parts.next()?.trim().parse::<u32>().ok()?;
    let d = parts.next()?.trim().parse::<u32>().ok()?;
    Day::from_ymd(y, m, d)
}

/// Every task of a note, in the order it is written.
pub fn tasks(text: &str) -> Vec<Task> {
    text.lines().filter_map(line_of).collect()
}

/// The span a plan covers: the first start and the last end.
pub fn span(tasks: &[Task]) -> Option<(Day, Day)> {
    let start = tasks.iter().map(|t| t.start).min()?;
    let end = tasks.iter().map(|t| t.end).max()?;
    Some((start, end.min(start.add_days(370))))
}

/// Where a day sits in `from..=to`, 0..=1.
pub fn share(day: Day, from: Day, to: Day) -> f32 {
    let total = (to.index() - from.index()).max(1) as f32;
    (((day.index() - from.index()) as f32) / total).clamp(0.0, 1.0)
}

/// The left and the width of a bar, 0..=1. A milestone keeps a sliver so it can be seen at all.
pub fn bar(task: &Task, from: Day, to: Day) -> (f32, f32) {
    let left = share(task.start, from, to);
    let right = share(task.end, from, to);
    let width = (right - left).max(0.004);
    (left.min(1.0 - width), width)
}

/// Whether the note on screen is one of these plans.
pub fn is_gantt(app: &crate::shell::App) -> bool {
    let Some(tab) = app.tab() else { return false };
    let Some(entry) = app.store.index.get(&tab.path) else { return false };
    let Some(meta) = entry.meta() else { return false };
    let Some(props) = meta.frontmatter.as_ref() else { return false };
    props.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("tipo")
            && v.as_str().is_some_and(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "gantt" | "plan"))
    })
}

/// The month names of the header, January first.
const MONTHS: [&str; 12] =
    ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];

/// The timeline: a label column, a bar column, and a line for today.
pub fn pane(app: &crate::shell::App, dom: &mut Dom, parent: NodeId) -> NodeId {
    let Some(tab) = app.tab() else { return parent };
    let text = tab.editor.buf.text();
    let tasks = tasks(&text);
    let body = dom.add(parent, El::div().class("view-content gantt"));
    let head = dom.add(body, El::div().class("gantt-head"));
    dom.add(head, El::div().class("gantt-title").text("Plan"));
    let Some((from, to)) = span(&tasks) else {
        let empty = dom.add(body, El::div().class("gantt-empty"));
        dom.add(empty, El::div().class("gantt-empty-title").text("Todavía no hay tareas con fechas"));
        dom.add(
            empty,
            El::div().class("gantt-empty-hint").text(
                "Escribí una línea por tarea: «- Diseño · 2026-11-02 · 2026-11-08». El mismo día en \
                 los dos extremos es un hito.",
            ),
        );
        return body;
    };
    let (fy, fm, fd) = from.ymd();
    let (ty, tm, td) = to.ymd();
    dom.add(
        head,
        El::div().class("gantt-range").text(format!(
            "{fd} {} {fy} → {td} {} {ty}",
            MONTHS[fm as usize - 1],
            MONTHS[tm as usize - 1]
        )),
    );

    // The months of the range, as a scale above the bars.
    let scale = dom.add(body, El::div().class("gantt-scale"));
    dom.add(scale, El::div().class("gantt-label-cell"));
    let track = dom.add(scale, El::div().class("gantt-track"));
    let mut month = Day::from_ymd(fy, fm, 1).unwrap_or(from);
    while month <= to {
        let (_, m, _) = month.ymd();
        let next = {
            let (y2, m2) = if m == 12 { (month.ymd().0 + 1, 1) } else { (month.ymd().0, m + 1) };
            Day::from_ymd(y2, m2, 1).unwrap_or(to)
        };
        let start = month.max(from);
        let end = next.min(to.add_days(1));
        let width = ((end.index() - start.index()) as f32 / (to.index() - from.index() + 1) as f32).max(0.0);
        let cell = dom.add(track, El::div().class("gantt-month"));
        dom.nodes[cell].el.inline_style = Some(format!("width: {:.2}%", width * 100.0));
        dom.add(cell, El::div().class("gantt-month-text").text(MONTHS[m as usize - 1].to_string()));
        month = next;
    }

    let today = crate::calendar::Calendar::today();
    let mut order = tasks.clone();
    order.sort_by_key(|t| (t.start, t.end, t.name.clone()));
    for task in &order {
        let row = dom.add(body, El::div().class("gantt-row"));
        let name = dom.add(row, El::div().class("gantt-label-cell"));
        dom.add(name, El::div().class("gantt-name").text(task.name.clone()));
        let (ry, rm, rd) = task.start.ymd();
        let (_, _, ed) = task.end.ymd();
        let when = if task.milestone() {
            format!("{rd} {}", MONTHS[rm as usize - 1])
        } else {
            format!("{rd}–{ed} {} {}", MONTHS[rm as usize - 1], &ry.to_string()[2..])
        };
        dom.add(name, El::div().class("gantt-when").text(when));
        let track = dom.add(row, El::div().class("gantt-track"));
        if today >= from && today <= to {
            let mark = dom.add(track, El::div().class("gantt-today"));
            dom.nodes[mark].el.inline_style = Some(format!("left: {:.2}%", share(today, from, to) * 100.0));
        }
        let (left, width) = bar(task, from, to);
        let bar_node =
            dom.add(track, El::div().class("gantt-bar").class_if(task.milestone(), "is-milestone"));
        // A milestone is a diamond the CSS sizes: only its place comes from here.
        let size = if task.milestone() { String::new() } else { format!(" width: {:.2}%;", width * 100.0) };
        dom.nodes[bar_node].el.inline_style = Some(format!(
            "left: {:.2}%;{size} background-color: {}",
            left * 100.0,
            if task.end < today { "var(--text-faint)" } else { "var(--interactive-accent)" }
        ));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Plan\n\n- Diseño · 2026-11-02 · 2026-11-08\n- Implementación · 2026-11-09 · 2026-11-22\n- Entrega · 2026-11-24 · 2026-11-24\n";

    #[test]
    fn the_lines_of_a_plan_are_tasks_with_their_dates() {
        let t = tasks(NOTE);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].name, "Diseño");
        assert_eq!(t[0].start, Day::from_ymd(2026, 11, 2).unwrap());
        assert_eq!(t[0].end, Day::from_ymd(2026, 11, 8).unwrap());
        assert!(t[2].milestone(), "un día solo es un hito");
        // A line without dates, or a date that does not exist, is prose.
        assert!(line_of("- sin fechas",).is_none());
        assert!(line_of("- Mala · 2026-02-31 · 2026-03-01").is_none());
    }

    #[test]
    fn the_span_covers_every_task_and_the_bars_fit_inside_it() {
        let t = tasks(NOTE);
        let (from, to) = span(&t).unwrap();
        assert_eq!(from, Day::from_ymd(2026, 11, 2).unwrap());
        assert_eq!(to, Day::from_ymd(2026, 11, 24).unwrap());
        let (left, width) = bar(&t[0], from, to);
        assert!(left.abs() < 0.001, "la primera empieza en el borde: {left}");
        assert!((width - 6.0 / 22.0).abs() < 0.01, "siete días de veintitrés: {width}");
        let (mleft, mwidth) = bar(&t[2], from, to);
        assert!(mleft > 0.9 && mwidth > 0.0, "el hito se ve: {mleft} {mwidth}");
        assert!(mleft + mwidth <= 1.001, "y no se sale del gráfico");
        // A task that ends before the start is read as a one-day task.
        let odd = line_of("- Rara · 2026-11-10 · 2026-11-08").unwrap();
        assert_eq!(odd.start, odd.end);
    }
}
