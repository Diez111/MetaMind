//! The calendar as a tab of the main pane: a month, a week with hours, a day and an agenda.
//!
//! The sidebar has the small month and the list of one day; this is the surface you open to look
//! at a week. It reads the same rows the sidebar does ([`crate::calendar::Calendar::rows`]), so an
//! event written in either place is the same event, and it can open the editor.
//!
//! Each view is plain DOM: a grid of cells, a column per day of the week with the events placed by
//! their hour, a list of hours, and a list of the next days with something on them. A week is 168
//! cells at most and a month is 42, which is what the layout engine was built for.

use crate::calendar::{color_hex, Calendar, Row};
use calendar_core::Day;
use metamind_ui::{Dom, El, NodeId};

/// How the tab is looking at the calendar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Month,
    Week,
    Day,
    Agenda,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Month, Mode::Week, Mode::Day, Mode::Agenda];

    pub fn id(self) -> &'static str {
        match self {
            Mode::Month => "month",
            Mode::Week => "week",
            Mode::Day => "day",
            Mode::Agenda => "agenda",
        }
    }

    pub fn from_id(id: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Month => "Mes",
            Mode::Week => "Semana",
            Mode::Day => "Día",
            Mode::Agenda => "Agenda",
        }
    }
}

/// The calendar tab's own state: what it shows and which day it is looking at.
#[derive(Clone, Debug)]
pub struct CalendarView {
    pub mode: Mode,
    /// Any day of the month on screen; the grid starts at its first day.
    pub anchor: Day,
    pub selected: Day,
}

impl CalendarView {
    /// A tab on the month of `selected`, the day the sidebar is on.
    pub fn open(selected: Day) -> CalendarView {
        CalendarView { mode: Mode::Month, anchor: selected.month_start(), selected }
    }

    pub fn title(&self) -> String {
        let (y, m, _) = self.anchor.ymd();
        let month = MONTHS.get(m as usize - 1).copied().unwrap_or_default();
        match self.mode {
            Mode::Month => format!("Calendario · {month} {y}"),
            Mode::Week => {
                let start = week_start(self.selected);
                let end = start.add_days(6);
                let (_, em, ed) = end.ymd();
                let (_, _, sd) = start.ymd();
                format!("Calendario · {sd}–{ed} {em}")
            }
            Mode::Day => {
                let (_, m2, d2) = self.selected.ymd();
                let month = MONTHS.get(m2 as usize - 1).copied().unwrap_or_default();
                format!("Calendario · {d2} de {month}")
            }
            Mode::Agenda => "Calendario · Próximos días".to_string(),
        }
    }

    /// The arrows: a month in the month view, a week in the week view, a day in the rest.
    pub fn step(&mut self, dir: i32) {
        match self.mode {
            Mode::Month => {
                let (y, m, _) = self.anchor.ymd();
                let total = i64::from(y) * 12 + i64::from(m) - 1 + i64::from(dir);
                let (ny, nm) = (total.div_euclid(12) as i32, total.rem_euclid(12) as u32 + 1);
                self.anchor = Day::from_ymd(ny, nm, 1).unwrap_or(self.anchor);
                self.selected = self.anchor;
            }
            Mode::Week => {
                self.selected = self.selected.add_days(7 * dir);
                self.anchor = self.selected.month_start();
            }
            _ => {
                self.selected = self.selected.add_days(dir);
                self.anchor = self.selected.month_start();
            }
        }
    }

    pub fn today(&mut self, today: Day) {
        self.selected = today;
        self.anchor = today.month_start();
    }
}

/// The month names of the header, January first.
const MONTHS: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];
/// The weekday of a column, Monday first.
const WEEKDAYS: [&str; 7] = ["lu", "ma", "mi", "ju", "vi", "sá", "do"];
const DAYS: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];

/// The Monday of the week `day` falls in: `Day::weekday` counts Sunday as 0.
pub fn week_start(day: Day) -> Day {
    day.add_days(-(((day.weekday() + 6) % 7) as i32))
}

/// The height of one hour in the week and day views.
const HOUR_H: f32 = 44.0;
/// Events drawn in a month cell before the rest is counted.
const MONTH_CHIPS: usize = 3;

/// The tab: header, then the view.
pub fn pane(app: &crate::shell::App, dom: &mut Dom, parent: NodeId) -> NodeId {
    let Some(view) = &app.calendar_view else { return parent };
    let header = dom.add(parent, El::div().class("view-header graph-controls calview-bar"));
    let prev = dom.add(header, El::div().class("graph-control calview-step").action("calview:prev"));
    dom.add(prev, crate::icons::el("arrow-left"));
    dom.add(header, El::div().class("calview-title").text(view.title()));
    let next = dom.add(header, El::div().class("graph-control calview-step").action("calview:next"));
    dom.add(next, crate::icons::el("arrow-right"));
    dom.add(header, El::div().class("graph-control").text("Hoy").action("calview:today"));
    let modes = dom.add(header, El::div().class("calview-modes"));
    for mode in Mode::ALL {
        dom.add(
            modes,
            El::div()
                .class("calview-mode")
                .class_if(view.mode == mode, "is-active")
                .text(mode.label())
                .action(format!("calview:mode:{}", mode.id())),
        );
    }
    dom.add(header, El::div().class("graph-control mod-cta").text("Nuevo evento").action("cal:new"));

    let body = dom.add(parent, El::div().class("view-content calview-body"));
    match view.mode {
        Mode::Month => month(&app.calendar, view, dom, body),
        Mode::Week => week(&app.calendar, view, dom, body),
        Mode::Day => day(&app.calendar, view, dom, body),
        Mode::Agenda => agenda(&app.calendar, view, dom, body),
    }
    body
}

/// The six weeks of the month, with the events of each day as chips.
fn month(cal: &Calendar, view: &CalendarView, dom: &mut Dom, body: NodeId) {
    let labels = dom.add(body, El::div().class("calview-weekdays"));
    for name in WEEKDAYS {
        dom.add(labels, El::div().class("calview-weekday").text(name));
    }
    let grid = dom.add(body, El::div().class("calview-month"));
    let start = week_start(view.anchor);
    let today = Calendar::today();
    for week in 0..6 {
        let row = dom.add(grid, El::div().class("calview-week"));
        for (i, name) in WEEKDAYS.iter().enumerate() {
            let day = start.add_days(week * 7 + i as i32);
            let cell = dom.add(
                row,
                El::div()
                    .class("calview-day")
                    .class_if(day.month_start() != view.anchor, "is-other-month")
                    .class_if(day == today, "is-today")
                    .class_if(day == view.selected, "is-selected")
                    .action(format!("calview:day:{}", day.index())),
            );
            let head = dom.add(cell, El::div().class("calview-day-head"));
            let (_, _, number) = day.ymd();
            dom.add(head, El::div().class("calview-day-num").text(number.to_string()));
            dom.add(head, El::div().class("calview-day-name").text(*name));
            let rows: Vec<&Row> = cal.rows_on(day).collect();
            if !rows.is_empty() {
                let list = dom.add(cell, El::div().class("calview-chips"));
                for row in rows.iter().take(MONTH_CHIPS) {
                    chip(dom, list, row);
                }
                if rows.len() > MONTH_CHIPS {
                    dom.add(
                        list,
                        El::div().class("calview-more").text(format!("+{}", rows.len() - MONTH_CHIPS)),
                    );
                }
            }
        }
    }
}

/// One event, as a chip with its colour, its clock and its title.
fn chip(dom: &mut Dom, parent: NodeId, row: &Row) {
    let node = dom.add(parent, El::div().class("calview-chip").class(row.source.css()));
    if let Some(hex) = row.color.as_deref().map(color_hex) {
        dom.nodes[node].el.inline_style =
            Some(format!("background-color: {hex}1f; border-left-color: {hex}"));
    }
    let clock = match row.minute {
        Some(m) => format!("{:02}:{:02} ", m / 60, m % 60),
        None => String::new(),
    };
    dom.add(node, El::div().class("calview-chip-text").text(format!("{clock}{}", row.title)));
}

/// The week: a column per day, an hour row every `HOUR_H`, and every event placed where it
/// belongs by its start and its length.
fn week(cal: &Calendar, view: &CalendarView, dom: &mut Dom, body: NodeId) {
    let start = week_start(view.selected);
    let today = Calendar::today();
    let head = dom.add(body, El::div().class("calview-week-head"));
    dom.add(head, El::div().class("calview-hours-gutter"));
    for i in 0..7 {
        let day = start.add_days(i);
        let (_, _, number) = day.ymd();
        dom.add(
            head,
            El::div()
                .class("calview-week-day")
                .class_if(day == today, "is-today")
                .class_if(day == view.selected, "is-selected")
                .text(format!("{} {number}", WEEKDAYS[i as usize]))
                .action(format!("calview:day:{}", day.index())),
        );
    }
    let grid = dom.add(body, El::div().class("calview-week-grid"));
    let gutter = dom.add(grid, El::div().class("calview-hours"));
    for hour in 0..24 {
        dom.add(gutter, El::div().class("calview-hour").text(format!("{hour:02}:00")));
    }
    for i in 0..7 {
        let day = start.add_days(i);
        let column = dom.add(
            grid,
            El::div()
                .class("calview-week-col")
                .class_if(day == today, "is-today")
                .style(&format!("height: {:.0}px", 24.0 * HOUR_H)),
        );
        // The hour lines, so the column reads as a day and not as an empty strip.
        for _ in 0..24 {
            dom.add(column, El::div().class("calview-week-slot"));
        }
        for row in cal.rows_on(day) {
            let (top, height) = match row.minute {
                Some(m) => (f32::from(m) / 60.0 * HOUR_H, 34.0),
                None => (0.0, 22.0),
            };
            let node = dom.add(
                column,
                El::div()
                    .class("calview-event")
                    .class(row.source.css())
                    .style(&format!("top: {top:.1}px; height: {height:.0}px")),
            );
            if let Some(hex) = row.color.as_deref().map(color_hex) {
                dom.nodes[node].el.inline_style =
                    Some(format!("top: {top:.1}px; height: {height:.0}px; background-color: {hex}26; border-left-color: {hex}"));
            }
            let clock = match row.minute {
                Some(m) => format!("{:02}:{:02} ", m / 60, m % 60),
                None => String::new(),
            };
            dom.add(node, El::div().class("calview-event-text").text(format!("{clock}{}", row.title)));
        }
    }
}

/// One day, hour by hour, with the all-day events on top.
fn day(cal: &Calendar, view: &CalendarView, dom: &mut Dom, body: NodeId) {
    let day = view.selected;
    let rows: Vec<&Row> = cal.rows_on(day).collect();
    let all_day: Vec<&&Row> = rows.iter().filter(|r| r.minute.is_none()).collect();
    if !all_day.is_empty() {
        let strip = dom.add(body, El::div().class("calview-allday"));
        for row in all_day {
            chip(dom, strip, row);
        }
    }
    let grid = dom.add(body, El::div().class("calview-day-grid"));
    for hour in 0..24 {
        let line = dom.add(grid, El::div().class("calview-day-hour"));
        dom.add(line, El::div().class("calview-hour").text(format!("{hour:02}:00")));
        let slot = dom.add(line, El::div().class("calview-day-slot"));
        for row in rows.iter().filter(|r| r.minute.is_some_and(|m| m / 60 == hour)) {
            let clock = row.minute.map(|m| format!("{:02}:{:02} ", m / 60, m % 60)).unwrap_or_default();
            let node = dom.add(
                slot,
                El::div().class("calview-pill").class(row.source.css()).text(format!("{clock}{}", row.title)),
            );
            if let Some(hex) = row.color.as_deref().map(color_hex) {
                dom.nodes[node].el.inline_style =
                    Some(format!("background-color: {hex}26; border-left-color: {hex}"));
            }
        }
    }
}

/// The next days with something on them.
fn agenda(cal: &Calendar, view: &CalendarView, dom: &mut Dom, body: NodeId) {
    let from = view.selected;
    let mut shown = 0;
    let mut day = from;
    for _ in 0..60 {
        let rows: Vec<&Row> = cal.rows_on(day).collect();
        if !rows.is_empty() {
            shown += 1;
            let (y, m, d) = day.ymd();
            let name = DAYS[day.weekday() as usize];
            let month = MONTHS.get(m as usize - 1).copied().unwrap_or_default();
            let head = dom.add(body, El::div().class("calview-agenda-day"));
            dom.add(
                head,
                El::div()
                    .class("calview-agenda-title")
                    .text(format!("{name} {d} de {month} de {y}"))
                    .action(format!("calview:day:{}", day.index())),
            );
            let list = dom.add(body, El::div().class("calview-agenda-rows"));
            for row in rows {
                let (minute, title) = (row.minute, row.title.clone());
                let source = row.source;
                let line = dom.add(
                    list,
                    El::div()
                        .class("calview-agenda-row")
                        .class(source.css())
                        .action(format!("calview:day:{}", day.index())),
                );
                if let Some(hex) = row.color.as_deref().map(color_hex) {
                    dom.nodes[line].el.inline_style = Some(format!("border-left-color: {hex}"));
                }
                let clock = match minute {
                    Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
                    None => "Todo el día".to_string(),
                };
                dom.add(line, El::div().class("calview-agenda-clock").text(clock));
                dom.add(line, El::div().class("calview-agenda-text").text(title));
            }
        }
        day = day.add_days(1);
        if shown >= 20 {
            break;
        }
    }
    if shown == 0 {
        dom.add(body, El::div().class("pane-empty").text("Nada agendado en los próximos 60 días."));
    }
}

impl crate::shell::App {
    /// Whether the calendar tab is the one the pane is showing.
    pub fn calendar_view_active(&self) -> bool {
        self.show_calendar_view
            && self.calendar_view.is_some()
            && !self.show_graph
            && !self.show_canvas
            && !self.show_base
            && !self.show_exc
            && !self.show_kanban
    }
}

/// The commands of the tab: `calview:*`. True when the command was one of them.
impl crate::shell::App {
    pub(crate) fn run_calendar_view(&mut self, command: &str) -> bool {
        let Some(rest) = command.strip_prefix("calview:") else { return false };
        if rest == "open" {
            let selected = self.calendar.selected;
            self.calendar_view = Some(CalendarView::open(selected));
            self.show_calendar_view = true;
            self.show_graph = false;
            self.show_canvas = false;
            self.show_base = false;
            self.show_exc = false;
            self.show_kanban = false;
            return true;
        }
        if rest == "close" {
            self.show_calendar_view = false;
            return true;
        }
        if rest == "focus" {
            self.show_calendar_view = true;
            return true;
        }
        let Some(view) = &mut self.calendar_view else { return true };
        match rest {
            "prev" => view.step(-1),
            "next" => view.step(1),
            "today" => view.today(Calendar::today()),
            _ => {
                let Some((name, arg)) = rest.split_once(':') else { return false };
                match name {
                    "mode" => {
                        if let Some(mode) = Mode::from_id(arg) {
                            view.mode = mode;
                        }
                    }
                    "day" => {
                        if let Ok(index) = arg.parse::<i32>() {
                            let day = Day::from_index(index);
                            view.selected = day;
                            view.anchor = day.month_start();
                        }
                    }
                    _ => return false,
                }
            }
        }
        self.calendar
            .select_in_month(self.calendar_view.as_ref().map_or(self.calendar.selected, |v| v.selected));
        true
    }
}
