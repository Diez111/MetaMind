//! `gantt`: tasks as bars along a time axis, grouped in sections, with the dependencies between
//! them.

use crate::common::{self, Draw};
use crate::{Diagram, Error, Fill, Measure};

/// A day number, counted from 1970-01-01, so that dates can be added and compared.
type Day = i32;

struct Task {
    label: String,
    id: String,
    start: Day,
    days: Day,
    after: Option<String>,
    milestone: bool,
    crit: bool,
}

struct Section {
    name: String,
    tasks: Vec<usize>,
}

/// The day number of a date written `YYYY-MM-DD`.
fn parse_date(text: &str) -> Option<Day> {
    let mut it = text.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i32;
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// The date of a day number, as `(year, month, day)`.
fn civil(mut z: Day) -> (i32, u32, u32) {
    z += 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn format_day(day: Day, long: bool) -> String {
    let (y, m, d) = civil(day);
    if long {
        format!("{y}-{m:02}-{d:02}")
    } else {
        format!("{d:02}/{m:02}")
    }
}

/// A duration such as `3d`, `2w` or `8h`, in days.
fn parse_duration(text: &str) -> Option<Day> {
    let (number, unit) = text.split_at(text.len().checked_sub(1)?);
    let n: f32 = number.parse().ok()?;
    let days = match unit {
        "d" => n,
        "w" => n * 7.0,
        "h" => n / 24.0,
        "m" => n / (24.0 * 60.0),
        _ => return None,
    };
    Some(days.ceil().max(1.0) as Day)
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut title = String::new();
    let mut tasks: Vec<Task> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let word = line.split_whitespace().next().unwrap_or("");
        match word {
            "title" => title = common::clean(&line["title".len()..]),
            "dateFormat" | "axisFormat" | "excludes" | "todayMarker" | "weekday" | "tickInterval"
            | "acctitle" | "accTitle" | "accDescr" => {}
            "section" => {
                sections.push(Section { name: common::clean(&line["section".len()..]), tasks: Vec::new() })
            }
            _ => {
                let task = parse_task(line)?;
                let section = match sections.len() {
                    0 => {
                        sections.push(Section { name: String::new(), tasks: Vec::new() });
                        0
                    }
                    n => n - 1,
                };
                sections[section].tasks.push(tasks.len());
                tasks.push(task);
            }
        }
    }
    if tasks.is_empty() {
        return Err(Error::Syntax("el diagrama de gantt no tiene tareas".into()));
    }
    // A task that starts after another one starts when that one ends, which may itself wait for
    // another: resolve them in a few passes, so that a cycle cannot hang the parser.
    for _ in 0..tasks.len() {
        let mut moved = false;
        for i in 0..tasks.len() {
            let Some(after) = tasks[i].after.clone() else { continue };
            if let Some(j) = tasks.iter().position(|t| t.id == after && !after.is_empty()) {
                if j != i && tasks[i].start < tasks[j].start + tasks[j].days {
                    tasks[i].start = tasks[j].start + tasks[j].days;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    Ok(paint(&title, &tasks, &sections, m))
}

/// `Write the spec :crit, id, 2014-01-01, 30d`.
fn parse_task(line: &str) -> Result<Task, Error> {
    let Some((label, spec)) = line.split_once(':') else {
        return Err(Error::Syntax(format!("tarea sin campos: `{line}`")));
    };
    let mut task = Task {
        label: common::clean(label),
        id: String::new(),
        start: 0,
        days: 1,
        after: None,
        milestone: false,
        crit: false,
    };
    let mut start: Option<Day> = None;
    let mut until: Option<Day> = None;
    for field in spec.split(',') {
        let f = field.trim();
        if f.is_empty() {
            continue;
        }
        if let Some(d) = parse_duration(f) {
            task.days = d;
        } else if let Some(day) = parse_date(f) {
            start = Some(day);
        } else if let Some(id) = f.strip_prefix("after ") {
            task.after = Some(id.trim().to_string());
        } else if let Some(rest) = f.strip_prefix("until ") {
            until = parse_date(rest.trim());
        } else if matches!(f, "milestone" | "crit" | "done" | "active") {
            task.milestone |= f == "milestone";
            task.crit |= f == "crit";
        } else {
            task.id = f.to_string();
        }
    }
    task.start = start.unwrap_or(0);
    if let Some(end) = until {
        task.days = (end - task.start).max(1);
    }
    Ok(task)
}

fn paint(title: &str, tasks: &[Task], sections: &[Section], m: &mut dyn Measure) -> Diagram {
    let first = tasks.iter().map(|t| t.start).min().unwrap_or(0);
    let last = tasks.iter().map(|t| t.start + t.days.max(1)).max().unwrap_or(first + 1);
    let span = (last - first).max(1);
    let ppd = (560.0 / span as f32).clamp(3.0, 16.0);
    let label_w = tasks.iter().map(|t| m.width(&t.label, 11.5, false)).fold(0.0f32, f32::max)
        + m.width("Sección", 11.5, true)
        + 40.0;
    let chart_w = span as f32 * ppd;
    let top = if title.is_empty() { 8.0 } else { 30.0 };
    let axis_h = 26.0;
    let row_h = 30.0;
    let mut d = Draw::new();
    if !title.is_empty() {
        d.text_left(m, 0.0, 14.0, title, true, 14.0);
    }
    let x0 = label_w;
    let grid_bottom =
        top + axis_h + sections.iter().map(|s| 22.0 + s.tasks.len() as f32 * row_h).sum::<f32>();

    // The axis: a line with a date every so many days, and the same lines across the chart.
    d.line(vec![(x0, top + axis_h), (x0 + chart_w, top + axis_h)], false, 1.0);
    let steps = [1, 2, 7, 14, 30, 60, 120, 365];
    let step = steps.iter().copied().find(|s| *s as f32 * ppd >= 54.0).unwrap_or(365);
    let long = span > 300;
    let mut day = first;
    while day <= last {
        let x = x0 + (day - first) as f32 * ppd;
        d.line(vec![(x, top + axis_h), (x, grid_bottom)], true, 0.6);
        d.text(m, x, top + axis_h - 10.0, &format_day(day, long), false, 9.5);
        day += step;
    }
    d.line(vec![(x0, top), (x0, grid_bottom)], false, 0.6);

    let mut row_y: Vec<f32> = vec![0.0; tasks.len()];
    let mut y = top + axis_h + 2.0;
    for s in sections {
        if s.tasks.is_empty() {
            continue;
        }
        d.band(0.0, y, x0 + chart_w, 20.0, 4.0, Fill::Accent);
        if !s.name.is_empty() {
            d.text_left(m, 2.0, y + 10.0, &s.name, true, 11.5);
        }
        y += 22.0;
        for &ti in &s.tasks {
            let t = &tasks[ti];
            row_y[ti] = y + row_h / 2.0;
            let bx = x0 + (t.start - first) as f32 * ppd + 2.0;
            let bw = (t.days.max(1) as f32 * ppd - 4.0).max(6.0);
            d.text_right(m, label_w - 12.0, y + row_h / 2.0, &t.label, false, 11.5);
            if t.milestone {
                d.diamond(bx + bw / 2.0, y + row_h / 2.0, 15.0, 15.0, Fill::Accent);
            } else {
                let fill = if t.crit { Fill::Accent } else { Fill::Node };
                d.rect(bx, y + row_h / 2.0 - 8.0, bw, 16.0, 4.0, fill);
            }
            y += row_h;
        }
    }

    // The dependencies, from the end of the task a task waits for to where it starts.
    for (i, t) in tasks.iter().enumerate() {
        let Some(after) = &t.after else { continue };
        let Some(j) = tasks.iter().position(|o| &o.id == after) else { continue };
        let (from_y, to_y) = (row_y[j], row_y[i]);
        if from_y == to_y {
            continue;
        }
        let prev = &tasks[j];
        let sx = x0 + (prev.start - first + prev.days.max(1)) as f32 * ppd + 4.0;
        let tx = x0 + (t.start - first) as f32 * ppd + 2.0;
        let mid = (sx + tx) / 2.0;
        let pts = if tx > sx + 8.0 {
            vec![(sx, from_y), (mid.max(sx + 6.0), from_y), (mid.max(sx + 6.0), to_y), (tx, to_y)]
        } else {
            vec![
                (sx, from_y),
                (sx + 8.0, from_y),
                (sx + 8.0, to_y - 9.0),
                (tx - 6.0, to_y - 9.0),
                (tx - 6.0, to_y),
            ]
        };
        d.line(pts.clone(), true, 1.0);
        if let Some(last) = pts.last() {
            d.arrow(last.0, last.1, std::f32::consts::FRAC_PI_2, false);
        }
    }
    d.finish(14.0)
}
