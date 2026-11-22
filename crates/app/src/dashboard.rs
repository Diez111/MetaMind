//! The dashboard: a note whose front matter says `tipo: dashboard` is drawn as a board of cards
//! built from the vault itself — the last week of daily notes and their inline fields, the open
//! tasks, and today's calendar — instead of as text.
//!
//! Nothing new is stored on disk: the board is a read of what the notes already say. The daily
//! note is found by its date: a file named `YYYY-MM-DD.md` anywhere in the vault, or a note whose
//! front matter says `tipo: diario` (or `daily`, `journal`) with a `fecha` field.

use crate::shell::App;
use calendar_core::Day;
use metamind_ui::{Dom, El, NodeId};

/// How many days the board looks back.
const DAYS: i32 = 7;

/// One day of the board, as its daily note wrote it.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct DayData {
    pub ymd: (i32, u32, u32),
    /// Hours of sleep, from `sueno::`.
    pub sleep: Option<f32>,
    pub steps: Option<u32>,
    pub hrv: Option<u32>,
    /// Whether `entreno::` says something.
    pub training: bool,
    pub open_tasks: usize,
    pub done_tasks: usize,
}

/// Everything the board shows, built once per vault revision and day.
#[derive(Default)]
pub struct Cache {
    /// (vault revision, today's day index): the board is stale when either moves.
    key: (u64, i32),
    pub days: Vec<DayData>,
    /// The notes edited most recently, newest first: (title, hours since the edit).
    pub recent: Vec<(String, f32)>,
}

/// Whether the active note of the focused pane is a dashboard.
pub fn is_dashboard(app: &App) -> bool {
    let Some(tab) = app.tab() else { return false };
    let Some(entry) = app.store.index.get(&tab.path) else { return false };
    let Some(meta) = entry.meta() else { return false };
    let Some(props) = meta.frontmatter.as_ref() else { return false };
    props.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("tipo") && v.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case("dashboard"))
    })
}

impl Cache {
    /// Rebuilds the board when the vault or the day changed; otherwise keeps what it had.
    pub fn refresh(&mut self, store: &vault_core::VaultStore, rev: u64) {
        let today = today();
        let key = (rev, today.index());
        if self.key == key {
            return;
        }
        self.key = key;
        self.days = (0..DAYS)
            .rev()
            .map(|back| {
                let day = today.add_days(-back);
                self.day_of(store, day)
            })
            .collect();
        self.recent = recent(store);
    }

    fn day_of(&self, store: &vault_core::VaultStore, day: Day) -> DayData {
        let mut data = DayData { ymd: day.ymd(), ..DayData::default() };
        let Some(rel) = daily_note(store, day) else { return data };
        let Ok((text, _)) = store.read(&rel) else { return data };
        let fields = inline_fields(&text);
        data.sleep = fields.get("sueno").and_then(|v| sleep_hours(v));
        data.steps = fields.get("pasos").and_then(|v| digits(v));
        data.hrv = fields.get("hrv").and_then(|v| digits(v));
        data.training = fields.get("entreno").is_some_and(|v| !v.trim().is_empty());
        for line in text.lines() {
            let t = line.trim_start();
            if t.starts_with("- [ ]") || t.starts_with("* [ ]") {
                data.open_tasks += 1;
            } else if t.starts_with("- [x]") || t.starts_with("- [X]") || t.starts_with("* [x]") {
                data.done_tasks += 1;
            }
        }
        data
    }
}

/// The day the board is about, in local time.
pub fn today() -> Day {
    let (y, m, d, _, _, _, _) = crate::datetime::local(vault_core::fsio::now_ms() / 1000);
    Day::from_ymd(y as i32, m, d).unwrap_or(Day::from_index(0))
}

/// The daily note of `day`: a file named `YYYY-MM-DD.md`, or a note whose front matter says it is
/// the journal of that date.
fn daily_note(store: &vault_core::VaultStore, day: Day) -> Option<String> {
    let (y, m, d) = day.ymd();
    let stem = format!("{y:04}-{m:02}-{d:02}");
    store
        .index
        .notes()
        .find(|(_, f)| vault_core::path::stem(&f.path) == stem)
        .map(|(_, f)| f.path.to_string())
        .or_else(|| {
            store.index.notes().find(|(_, f)| {
                let Some(meta) = f.meta() else { return false };
                let Some(props) = meta.frontmatter.as_ref() else { return false };
                let is_entry = props.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("tipo")
                        && v.as_str().is_some_and(|s| {
                            let s = s.trim().to_lowercase();
                            s == "diario" || s == "daily" || s == "journal"
                        })
                });
                is_entry
                    && props.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case("fecha") && v.as_str().is_some_and(|s| s.trim() == stem)
                    })
            })
            .map(|(_, f)| f.path.to_string())
        })
}

/// The `name:: value` pairs (and `name: value`) of a note, by lowercase name. Only the first
/// occurrence of each name counts: a daily note that repeats a field keeps the one it opened with.
pub fn inline_fields(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut in_front = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if i == 0 && t == "---" {
            in_front = true;
            continue;
        }
        if in_front {
            if t == "---" {
                in_front = false;
            }
            continue;
        }
        let Some((name, value)) = t.split_once("::").or_else(|| t.split_once(": ")) else { continue };
        let name = name.trim().to_lowercase();
        if name.is_empty() || name.contains(' ') || name.starts_with('-') || name.starts_with('#') {
            continue;
        }
        out.entry(name).or_insert_with(|| value.trim().to_string());
    }
    out
}

/// Hours of sleep from the ways a note tends to write them: `7:30`, `7h 30m`, `7.5`, `450m`.
pub fn sleep_hours(raw: &str) -> Option<f32> {
    let v = raw.trim().to_lowercase().replace(',', ".");
    if v.is_empty() {
        return None;
    }
    let clamp = |h: f32| (h > 0.0 && h <= 24.0).then_some(h);
    if let Some((h, m)) = v.split_once(':') {
        let (h, m): (f32, f32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
        return clamp(h + m / 60.0);
    }
    if let Some(pos) = v.find('h') {
        let h: f32 = v[..pos].trim().parse().unwrap_or(0.0);
        let rest = v[pos + 1..].trim().trim_end_matches('m').trim();
        let m: f32 = if rest.is_empty() { 0.0 } else { rest.parse().ok()? };
        return clamp(h + m / 60.0);
    }
    if let Some(pos) = v.find('m') {
        let m: f32 = v[..pos].trim().parse().ok()?;
        return clamp(m / 60.0);
    }
    v.parse::<f32>().ok().and_then(|h| if h > 24.0 { clamp(h / 60.0) } else { clamp(h) })
}

/// The first number of a field, for steps, HRV and the like.
fn digits(raw: &str) -> Option<u32> {
    let start = raw.find(|c: char| c.is_ascii_digit())?;
    let digits: String = raw[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// The notes edited most recently, with how many hours ago that was.
fn recent(store: &vault_core::VaultStore) -> Vec<(String, f32)> {
    let now = vault_core::fsio::now_ms();
    let mut files: Vec<(&str, i64)> = store
        .index
        .notes()
        .filter(|(_, f)| f.path.ends_with(".md"))
        .map(|(_, f)| (&*f.path, f.mtime / 1_000_000))
        .collect();
    files.sort_by_key(|(_, ms)| -ms);
    files
        .into_iter()
        .take(6)
        .map(|(path, ms)| {
            let hours = ((now - ms as u64) as f32 / 3_600_000.0).max(0.0);
            (vault_core::path::stem(path).to_string(), hours)
        })
        .collect()
}

const WEEKDAYS: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];
const MONTHS: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre",
    "noviembre", "diciembre",
];

/// `lunes 9 de noviembre`, from a day.
pub fn long_date(ymd: (i32, u32, u32)) -> String {
    let day = Day::from_ymd(ymd.0, ymd.1, ymd.2).unwrap_or(Day::from_index(0));
    // `weekday` counts Sunday as 0, so the table starts there too.
    let wd = WEEKDAYS.get(day.weekday() as usize % 7).copied().unwrap_or("");
    let month = MONTHS.get(ymd.1.saturating_sub(1) as usize).copied().unwrap_or("");
    format!("{wd} {} de {month}", ymd.2)
}

/// "hace 2 h" for a note edited `hours` ago.
fn ago(hours: f32) -> String {
    if hours < 1.0 {
        let min = (hours * 60.0).round().max(1.0) as u32;
        format!("hace {min} min")
    } else if hours < 24.0 {
        format!("hace {} h", hours.round() as u32)
    } else {
        format!("hace {} d", (hours / 24.0).round() as u32)
    }
}

/// The board, in the pane the dashboard note is open in.
pub fn pane(app: &App, dom: &mut Dom, parent: NodeId) {
    let wrap = dom.add(parent, El::div().class("dash"));
    let today = today();
    let head = dom.add(wrap, El::div().class("dash-head"));
    dom.add(head, El::div().class("dash-title").text("Hoy"));
    dom.add(head, El::div().class("dash-date").text(long_date(today.ymd())));

    let grid = dom.add(wrap, El::div().class("dash-grid"));
    sleep_card(app, dom, grid);
    week_card(app, dom, grid);
    calendar_card(app, dom, grid, today);
    recent_card(app, dom, grid);
}

fn card(dom: &mut Dom, parent: NodeId, title: &str) -> NodeId {
    let c = dom.add(parent, El::div().class("dash-card"));
    dom.add(c, El::div().class("dash-card-title").text(title.to_string()));
    c
}

/// The week of sleep, one bar per day, with the average.
fn sleep_card(app: &App, dom: &mut Dom, grid: NodeId) {
    let c = card(dom, grid, "Sueño · últimos 7 días");
    let values: Vec<Option<f32>> = app.dash.days.iter().map(|d| d.sleep).collect();
    let known: Vec<f32> = values.iter().flatten().copied().collect();
    if known.is_empty() {
        let (y, m, d) = today().ymd();
        dom.add(
            c,
            El::div()
                .class("dash-empty")
                .text(format!("Todavía no hay un `sueno::` cargado en la nota diaria ({y:04}-{m:02}-{d:02})")),
        );
        return;
    }
    let avg = known.iter().sum::<f32>() / known.len() as f32;
    let row = dom.add(c, El::div().class("dash-big-row"));
    dom.add(row, El::div().class("dash-big").text(format_hours(avg)));
    dom.add(
        row,
        El::div()
            .class("dash-big-note")
            .text(format!("promedio · {} de {DAYS} días", known.len())),
    );
    let bars = dom.add(c, El::div().class("dash-bars"));
    let max = values.iter().flatten().copied().fold(9.0f32, f32::max);
    for (data, value) in app.dash.days.iter().zip(&values) {
        let col = dom.add(bars, El::div().class("dash-bar-col"));
        let track = dom.add(col, El::div().class("dash-bar-track"));
        match value {
            Some(h) => {
                let pct = (h / max * 100.0).clamp(4.0, 100.0);
                let bar = El::div().class("dash-bar").style(&format!("height: {pct:.0}%"));
                let bar = if *h >= 7.0 { bar.class("is-good") } else { bar };
                dom.add(track, bar);
            }
            None => {
                dom.add(track, El::div().class("dash-bar is-missing").style("height: 4%"));
            }
        }
        let (_, _, d) = data.ymd;
        dom.add(col, El::div().class("dash-bar-label").text(format!("{d}")));
    }
}

/// Training, steps and HRV of the week, as one line each.
fn week_card(app: &App, dom: &mut Dom, grid: NodeId) {
    let c = card(dom, grid, "Esta semana");
    let training = app.dash.days.iter().filter(|d| d.training).count();
    let steps: Vec<u32> = app.dash.days.iter().filter_map(|d| d.steps).collect();
    let hrv: Vec<u32> = app.dash.days.iter().filter_map(|d| d.hrv).collect();
    let open: usize = app.dash.days.iter().map(|d| d.open_tasks).sum();
    let done: usize = app.dash.days.iter().map(|d| d.done_tasks).sum();
    let mut stat = |label: &str, value: String, note: &str| {
        let cell = dom.add(c, El::div().class("dash-stat"));
        dom.add(cell, El::div().class("dash-stat-label").text(label));
        let line = dom.add(cell, El::div().class("dash-stat-line"));
        dom.add(line, El::div().class("dash-stat-value").text(value));
        if !note.is_empty() {
            dom.add(line, El::div().class("dash-stat-note").text(note.to_string()));
        }
    };
    stat("Entrenamientos", format!("{training}"), &format!("de {DAYS} días"));
    stat(
        "Pasos",
        match (steps.is_empty(), steps.iter().sum::<u32>()) {
            (true, _) => "—".to_string(),
            (false, sum) => format!("{:.0}k", sum as f32 / 1000.0),
        },
        if steps.is_empty() { "sin registro" } else { "en la semana" },
    );
    stat(
        "HRV",
        match (hrv.is_empty(), hrv.iter().sum::<u32>()) {
            (true, _) => "—".to_string(),
            (false, sum) => format!("{}", sum / hrv.len() as u32),
        },
        if hrv.is_empty() { "sin registro" } else { "promedio" },
    );
    stat("Tareas", format!("{open}"), &format!("{done} hechas"));
}

/// Today's events, from the calendar the app already keeps.
fn calendar_card(app: &App, dom: &mut Dom, grid: NodeId, day: Day) {
    let c = card(dom, grid, "Hoy");
    let mut any = false;
    for row in app.calendar.rows_on(day) {
        any = true;
        let item = dom.add(c, El::div().class("dash-event"));
        let when = match row.minute {
            Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
            None => "todo el día".to_string(),
        };
        dom.add(item, El::div().class("dash-event-time").text(when));
        dom.add(item, El::div().class("dash-event-title").text(row.title.clone()));
    }
    if !any {
        dom.add(c, El::div().class("dash-empty").text("Sin eventos para hoy"));
    }
}

/// The notes edited most recently.
fn recent_card(app: &App, dom: &mut Dom, grid: NodeId) {
    let c = card(dom, grid, "Retomá donde estabas");
    if app.dash.recent.is_empty() {
        dom.add(c, El::div().class("dash-empty").text("Sin notas todavía"));
        return;
    }
    for (title, hours) in &app.dash.recent {
        let row = dom.add(c, El::div().class("dash-note").action(format!("dashboard:open:{title}")));
        dom.add(row, El::div().class("dash-note-title").text(title.clone()));
        dom.add(row, El::div().class("dash-note-ago").text(ago(*hours)));
    }
}

/// `7 h 44 min`, the way the board says a night.
fn format_hours(h: f32) -> String {
    let total = (h * 60.0).round() as u32;
    let (h, m) = (total / 60, total % 60);
    if m == 0 {
        format!("{h} h")
    } else {
        format!("{h} h {m} min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_reads_every_way_a_note_writes_it() {
        assert_eq!(sleep_hours("7:30"), Some(7.5));
        assert_eq!(sleep_hours("7h 30m"), Some(7.5));
        assert_eq!(sleep_hours("7h30"), Some(7.5));
        assert_eq!(sleep_hours("7.5"), Some(7.5));
        assert_eq!(sleep_hours("450m"), Some(7.5));
        assert_eq!(sleep_hours(""), None);
        assert_eq!(sleep_hours("nada"), None);
        // A number over 24 is read as minutes, not as hours.
        assert_eq!(sleep_hours("420"), Some(7.0));
        assert_eq!(sleep_hours("30"), Some(0.5));
    }

    #[test]
    fn inline_fields_skip_the_front_matter_and_take_the_first_value() {
        let text = "---\ntipo: diario\nsueno:: 1:00\n---\n# lunes\n\nsueno:: 7:20\nanimo:: bien\nno es campo\n- [ ] tarea\n";
        let f = inline_fields(text);
        assert_eq!(f.get("sueno").map(String::as_str), Some("7:20"), "the front matter is not a field");
        assert_eq!(f.get("animo").map(String::as_str), Some("bien"));
        assert!(!f.contains_key("no es campo"));
    }

    #[test]
    fn the_long_date_is_in_spanish_and_starts_on_monday() {
        // 2026-11-22 is a Sunday.
        assert_eq!(long_date((2026, 11, 22)), "domingo 22 de noviembre");
        assert_eq!(long_date((2026, 11, 23)), "lunes 23 de noviembre");
    }
}
