//! The calendar: a month of events in the left sidebar, read from a Google Calendar iCal address
//! and from `.ics` files in the vault, plus the `@{YYYY-MM-DD}` tasks of any note.
//!
//! The calendar is fetched from the URL Google calls the "secret address in iCal format": the
//! user pastes it once in Ajustes → Calendario and there is no OAuth, no browser and no
//! credentials involved. The body is cached in `.metamind/cache/calendar.ics` and revalidated
//! with `ETag`/`Last-Modified`, so the pane paints from the cache first and a fetch never blocks
//! a frame: it runs on a thread of its own and the answer is polled, like `yt-dlp` in [`crate::youtube`].
//!
//! The URL is a secret. It is never put in a notice, a cache file or a log: the cached metadata
//! keeps a hash of it, and the messages of a failed fetch are written here, not taken from the
//! error type of the HTTP client.

use crate::icons;
use calendar_core::{Calendar as Ics, Day, Occurrence};
use metamind_ui::{Dom, El, NodeId};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use vault_core::{fsio, path, VaultStore};

/// The cached body and its validators, under the vault.
const CACHE_ICS: &str = ".metamind/cache/calendar.ics";
const CACHE_META: &str = ".metamind/cache/calendar.json";

/// How old a fetch may be before opening the pane asks for another one.
const FRESH_MS: u64 = 5 * 60 * 1000;

/// Notes looked at for `@{fecha}` tasks in one frame, so the first scan of a big vault does not
/// freeze a frame: the rest continues on the next one.
const TASK_BATCH: usize = 1500;

/// Where an event comes from, which is what colours its mark.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Google,
    Vault,
    Tasks,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Google => "Google",
            Source::Vault => "Bóveda",
            Source::Tasks => "Tareas",
        }
    }

    pub fn css(self) -> &'static str {
        match self {
            Source::Google => "calendar-source-google",
            Source::Vault => "calendar-source-vault",
            Source::Tasks => "calendar-source-tasks",
        }
    }
}

/// One row of the day panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub day: Day,
    /// `None` for an all-day event or a task.
    pub minute: Option<u16>,
    pub title: String,
    pub location: String,
    pub description: String,
    pub source: Source,
    /// The note to open when the row is clicked (a task line, or the `.ics` file of the vault).
    pub note: String,
    pub uid: String,
}

/// A task line of a note: `- [ ] Llamar @{2025-09-16}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub day: Day,
    pub title: String,
    pub note: String,
}

/// A `.ics` file of the vault, with its path so a row can open it.
struct VaultIcs {
    path: String,
    cal: Ics,
}

/// The validators of the cached body.
#[derive(Clone, Default)]
struct Meta {
    url_hash: u64,
    etag: String,
    modified: String,
    fetched_ms: u64,
}

/// What the worker thread sends back.
struct Fetched {
    not_modified: bool,
    body: String,
    etag: String,
    modified: String,
}

pub struct Calendar {
    /// The first day of the month on screen.
    pub month: Day,
    /// The day the panel lists.
    pub selected: Day,
    /// The open detail: the day and the position of the row in that day's list.
    pub detail: Option<(Day, usize)>,
    pub desc_open: bool,
    /// The events of the visible weeks plus the selected day, ready to draw.
    pub rows: Vec<Row>,
    /// Why the last fetch failed, in words that never carry the URL.
    pub error: Option<String>,
    /// When the cached body was last confirmed by the server.
    pub fetched_ms: u64,
    /// Whether a fetch is running, and for which address.
    pub fetching: bool,
    url_hash: u64,
    google: Option<Ics>,
    files: Vec<VaultIcs>,
    /// Tasks by note, so a note that changed replaces its own lines only.
    tasks: BTreeMap<String, Vec<Task>>,
    /// Notes already scanned for tasks, with the stamp they were read at.
    scanned: HashMap<String, (i64, u64)>,
    /// Notes waiting to be looked at, and how far the last frame got.
    queue: Vec<(String, i64, u64)>,
    queue_at: usize,
    /// The vault revision `files` and `queue` were built for.
    scan_revision: u64,
    dirty: bool,
    pending: Option<Receiver<Result<Fetched, String>>>,
    /// Hash of the address the running fetch was started for.
    pending_hash: u64,
    last_attempt_ms: u64,
}

impl Calendar {
    /// A calendar at the month of `today`, painted from the cache when there is one.
    pub fn new(root: &Path, url: &str) -> Calendar {
        let (y, m, d, ..) = crate::datetime::local(fsio::now_ms() / 1000);
        let today = Day::from_ymd(y as i32, m, d).unwrap_or(Day::from_index(0));
        let mut cal = Calendar {
            month: today.month_start(),
            selected: today,
            detail: None,
            desc_open: false,
            rows: Vec::new(),
            error: None,
            fetched_ms: 0,
            fetching: false,
            url_hash: 0,
            google: None,
            files: Vec::new(),
            tasks: BTreeMap::new(),
            scanned: HashMap::new(),
            queue: Vec::new(),
            queue_at: 0,
            scan_revision: 0,
            dirty: true,
            pending: None,
            pending_hash: 0,
            last_attempt_ms: 0,
        };
        cal.load_cache(root, url);
        cal
    }

    pub fn today() -> Day {
        let (y, m, d, ..) = crate::datetime::local(fsio::now_ms() / 1000);
        Day::from_ymd(y as i32, m, d).unwrap_or(Day::from_index(0))
    }

    /// Whether the URL currently configured matches the cache and the events on screen.
    pub fn has_url(&self, url: &str) -> bool {
        !url.trim().is_empty() && fsio::fold64(url.trim().as_bytes()) == self.url_hash
    }

    /// Read the cached body and its validators. A cache written for another address is ignored:
    /// changing the address must not show the previous calendar.
    fn load_cache(&mut self, root: &Path, url: &str) {
        let meta = read_meta(root);
        self.fetched_ms = meta.fetched_ms;
        self.url_hash = meta.url_hash;
        if url.trim().is_empty() || fsio::fold64(url.trim().as_bytes()) != meta.url_hash {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(root.join(CACHE_ICS)) {
            let cal = calendar_core::parse(&text);
            if !cal.events.is_empty() {
                self.google = Some(cal);
            }
        }
    }

    /// Drop what the cache holds when the address changes.
    pub fn forget(&mut self) {
        self.google = None;
        self.fetched_ms = 0;
        self.url_hash = 0;
        self.error = None;
        self.dirty = true;
    }

    /// Ask for the body unless the cache is fresh or a fetch is already running.
    pub fn maybe_fetch(&mut self, root: &Path, url: &str) {
        let url = url.trim();
        if url.is_empty() || self.fetching {
            return;
        }
        let recent = fsio::now_ms().saturating_sub(self.last_attempt_ms.max(self.fetched_ms)) < FRESH_MS;
        if recent && self.has_url(url) {
            return;
        }
        self.start(root, url);
    }

    /// Fetch now, whatever the cache says.
    pub fn refresh(&mut self, root: &Path, url: &str) {
        let url = url.trim().to_string();
        if url.is_empty() {
            self.error = Some("Falta la dirección iCal: se pega en Ajustes → Calendario.".to_string());
            return;
        }
        self.start(root, &url);
    }

    fn start(&mut self, root: &Path, url: &str) {
        if self.fetching {
            return;
        }
        let meta = read_meta(root);
        let meta = if fsio::fold64(url.as_bytes()) == meta.url_hash { meta } else { Meta::default() };
        self.last_attempt_ms = fsio::now_ms();
        self.fetching = true;
        self.error = None;
        self.pending_hash = fsio::fold64(url.as_bytes());
        let (tx, rx) = channel();
        let url = url.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&url, &meta));
        });
        self.pending = Some(rx);
    }

    /// Take the answer of the running fetch, if it has one. `true` when something changed.
    pub fn poll(&mut self, root: &Path) -> bool {
        let Some(rx) = &self.pending else { return false };
        let answer = match rx.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.fetching = false;
                self.error = Some("La descarga del calendario se cortó.".to_string());
                return true;
            }
        };
        self.pending = None;
        self.fetching = false;
        let url_hash = self.pending_hash;
        match answer {
            Ok(fetched) => {
                let body = if fetched.not_modified {
                    write_meta(root, url_hash, &fetched);
                    std::fs::read_to_string(root.join(CACHE_ICS)).unwrap_or_default()
                } else {
                    write_cache(root, url_hash, &fetched)
                };
                let cal = calendar_core::parse(&body);
                self.google = (!cal.events.is_empty()).then_some(cal);
                self.fetched_ms = fsio::now_ms();
                self.error = None;
                self.dirty = true;
            }
            // The cached events stay on screen: an old calendar is better than an empty pane.
            Err(message) => self.error = Some(message),
        }
        true
    }

    /// Milliseconds until the running fetch should be looked at again.
    pub fn due_in(&self) -> Option<u64> {
        self.fetching.then_some(150)
    }

    /// Ask for the rows to be rebuilt on the next frame.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// The vault revision the file list and the task queue were built for.
    pub fn revision(&self) -> u64 {
        self.scan_revision
    }

    /// Look for `.ics` files and `@{fecha}` tasks, a slice of the notes per call.
    pub fn scan_vault(&mut self, store: &VaultStore, revision: u64) {
        self.scan_revision = revision;
        self.files = store
            .index
            .iter()
            .filter(|(_, f)| path::extension(f.path.as_ref()).eq_ignore_ascii_case("ics"))
            .filter_map(|(_, f)| {
                let text = std::fs::read_to_string(store.abs(f.path.as_ref())).ok()?;
                let cal = calendar_core::parse(&text);
                (!cal.events.is_empty()).then(|| VaultIcs { path: f.path.to_string(), cal })
            })
            .collect();
        let mut queue: Vec<(String, i64, u64)> =
            store.index.notes().map(|(_, f)| (f.path.to_string(), f.mtime, f.size)).collect();
        queue.sort();
        self.queue = queue;
        self.queue_at = 0;
        self.dirty = true;
    }

    /// Read the next slice of notes looking for tasks.
    fn step_tasks(&mut self, store: &VaultStore) {
        let mut budget = TASK_BATCH;
        while budget > 0 && self.queue_at < self.queue.len() {
            let (rel, mtime, size) = self.queue[self.queue_at].clone();
            self.queue_at += 1;
            budget -= 1;
            if self.scanned.get(&rel) == Some(&(mtime, size)) {
                continue;
            }
            self.scanned.insert(rel.clone(), (mtime, size));
            let found = match store.read(&rel) {
                Ok((text, _)) => tasks_of(&rel, &text),
                Err(_) => Vec::new(),
            };
            if found.is_empty() {
                self.tasks.remove(&rel);
            } else {
                self.tasks.insert(rel, found);
            }
            self.dirty = true;
        }
        if self.queue_at >= self.queue.len() && !self.queue.is_empty() {
            self.queue = Vec::new();
        }
    }

    /// Rebuild the rows of the visible weeks, and of the selected day when it is elsewhere.
    fn rebuild(&mut self) {
        self.dirty = false;
        let start = grid_start(self.month);
        let from = start.min(self.selected);
        let to = start.add_days(41).max(self.selected);
        let mut rows = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        // Google first: the same event subscribed twice shows once, with its Google colour.
        if let Some(cal) = self.google.as_ref().and_then(|c| owned(c, &mut seen)) {
            rows_of(&cal, Source::Google, "", from, to, &mut rows);
        }
        for file in &self.files {
            if let Some(cal) = owned(&file.cal, &mut seen) {
                rows_of(&cal, Source::Vault, &file.path, from, to, &mut rows);
            }
        }
        for (note, tasks) in &self.tasks {
            for task in tasks {
                if task.day < from || task.day > to {
                    continue;
                }
                rows.push(Row {
                    day: task.day,
                    minute: None,
                    title: task.title.clone(),
                    location: String::new(),
                    description: String::new(),
                    source: Source::Tasks,
                    note: note.clone(),
                    uid: format!("task:{note}:{}", task.title),
                });
            }
        }
        rows.sort_by(|a, b| {
            a.day.cmp(&b.day).then(a.minute.cmp(&b.minute)).then_with(|| a.title.cmp(&b.title))
        });
        self.rows = rows;
    }

    /// The rows of one day, in the order the panel shows them.
    pub fn rows_on(&self, day: Day) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(move |r| r.day == day)
    }

    /// How many rows a day has, for the mark in its cell.
    pub fn count_on(&self, day: Day) -> usize {
        self.rows.iter().filter(|r| r.day == day).count()
    }

    pub fn has_events(&self) -> bool {
        self.google.is_some() || !self.files.is_empty() || !self.tasks.is_empty()
    }

    /// Move the month by `months`, keeping the day of the month when the new month has it.
    pub fn step_month(&mut self, months: i32) {
        let (y, m, d) = self.selected.ymd();
        let total = i64::from(y) * 12 + i64::from(m) - 1 + i64::from(months);
        let y = total.div_euclid(12) as i32;
        let m = total.rem_euclid(12) as u32 + 1;
        let day = (1..=31).rev().find_map(|d2| Day::from_ymd(y, m, d.min(d2))).unwrap_or(self.selected);
        self.selected = day;
        self.month = day.month_start();
        self.dirty = true;
    }

    /// Move the selected day, following it into another month.
    pub fn step_day(&mut self, days: i32) {
        self.selected = self.selected.add_days(days);
        self.month = self.selected.month_start();
        self.dirty = true;
    }

    pub fn select(&mut self, day: Day) {
        self.selected = day;
        self.month = day.month_start();
        self.detail = None;
        self.dirty = true;
    }

    /// Everything the pane needs, refreshed once per frame.
    pub fn tick(&mut self, root: &Path, store: &VaultStore) {
        if self.poll(root) {
            self.dirty = true;
        }
        if !self.queue.is_empty() {
            self.step_tasks(store);
        }
        if self.dirty {
            self.rebuild();
        }
    }
}

/// The events of `cal` whose `UID` nobody claimed yet.
fn owned(cal: &Ics, seen: &mut HashSet<String>) -> Option<Ics> {
    let events: Vec<_> = cal.events.iter().filter(|e| seen.insert(e.uid.to_lowercase())).cloned().collect();
    (!events.is_empty()).then(|| Ics { name: cal.name.clone(), events })
}

fn rows_of(cal: &Ics, source: Source, note: &str, from: Day, to: Day, out: &mut Vec<Row>) {
    for Occurrence { event, start, .. } in calendar_core::expand(cal, from, to) {
        let Some(ev) = cal.events.get(event) else { continue };
        out.push(Row {
            day: start.day,
            minute: start.minute,
            title: if ev.summary.trim().is_empty() {
                "(sin título)".to_string()
            } else {
                ev.summary.clone()
            },
            location: ev.location.clone(),
            description: ev.description.clone(),
            source,
            note: note.to_string(),
            uid: ev.uid.clone(),
        });
    }
}

/// The tasks of a note: any line that carries `@{YYYY-MM-DD}`.
pub fn tasks_of(note: &str, text: &str) -> Vec<Task> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((day, title)) = task_line(line) else { continue };
        out.push(Task { day, title, note: note.to_string() });
    }
    out
}

/// A task line as (day, title), or `None` when the line carries no date.
fn task_line(line: &str) -> Option<(Day, String)> {
    let at = line.find("@{")?;
    let rest = line.get(at + 2..)?;
    let end = rest.find('}')?;
    let day = ymd(rest.get(..end)?.trim())?;
    let mut title = String::with_capacity(line.len());
    title.push_str(line.get(..at)?);
    title.push_str(rest.get(end + 1..)?);
    // The line's list marker and checkbox are not part of the title.
    let title = title.trim();
    let title = title.strip_prefix(['-', '*', '+']).unwrap_or(title).trim_start();
    let title = title
        .strip_prefix("[ ]")
        .or_else(|| title.strip_prefix("[x]"))
        .or_else(|| title.strip_prefix("[X]"))
        .unwrap_or(title);
    Some((day, title.trim().to_string()))
}

/// `YYYY-MM-DD`, the shape the tasks use.
fn ymd(text: &str) -> Option<Day> {
    if text.len() != 10 || text.as_bytes().get(4) != Some(&b'-') || text.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let y = text.get(0..4)?.parse().ok()?;
    let m = text.get(5..7)?.parse().ok()?;
    let d = text.get(8..10)?.parse().ok()?;
    Day::from_ymd(y, m, d)
}

/// The Monday that starts the first week of `month`'s grid.
pub fn grid_start(month: Day) -> Day {
    let first = month.month_start();
    first.add_days(-(((first.weekday() + 6) % 7) as i32))
}

/// The weekday labels of the grid, Monday first.
pub const WEEKDAYS: [&str; 7] = ["lu", "ma", "mi", "ju", "vi", "sá", "do"];

fn meta_path(root: &Path) -> PathBuf {
    root.join(CACHE_META)
}

fn read_meta(root: &Path) -> Meta {
    let Ok(text) = std::fs::read_to_string(meta_path(root)) else { return Meta::default() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return Meta::default() };
    Meta {
        url_hash: v.get("url_hash").and_then(Value::as_u64).unwrap_or(0),
        etag: v.get("etag").and_then(Value::as_str).unwrap_or_default().to_string(),
        modified: v.get("modified").and_then(Value::as_str).unwrap_or_default().to_string(),
        fetched_ms: v.get("fetched_ms").and_then(Value::as_u64).unwrap_or(0),
    }
}

/// Store the body and its validators. Returns the text that was written.
fn write_cache(root: &Path, url_hash: u64, fetched: &Fetched) -> String {
    let body = root.join(CACHE_ICS);
    if let Some(dir) = body.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = fsio::atomic_write(&body, fetched.body.as_bytes());
    write_meta(root, url_hash, fetched);
    fetched.body.clone()
}

/// Store the validators: a hash of the address (never the address), the `ETag`, the
/// `Last-Modified` and when the server last confirmed the body.
fn write_meta(root: &Path, url_hash: u64, fetched: &Fetched) {
    let old = read_meta(root);
    let meta = json!({
        "url_hash": url_hash,
        "etag": if fetched.etag.is_empty() { old.etag } else { fetched.etag.clone() },
        "modified": if fetched.modified.is_empty() { old.modified } else { fetched.modified.clone() },
        "fetched_ms": fsio::now_ms(),
    });
    let _ = fsio::atomic_write(&meta_path(root), meta.to_string().as_bytes());
}

/// GET the address, with the validators of the cache. The error messages are written here: the
/// `Display` of the HTTP client can carry the URL, which is a secret.
fn fetch(url: &str, meta: &Meta) -> Result<Fetched, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(15)))
            .timeout_recv_body(Some(Duration::from_secs(15)))
            // The status is inspected below: 304 is an answer, not an error.
            .http_status_as_error(false)
            .build(),
    );
    let mut request =
        agent.get(url).header("User-Agent", concat!("MetaMind/", env!("CARGO_PKG_VERSION"), " (calendario)"));
    if !meta.etag.is_empty() {
        request = request.header("If-None-Match", &meta.etag);
    }
    if !meta.modified.is_empty() {
        request = request.header("If-Modified-Since", &meta.modified);
    }
    let mut response = request
        .call()
        .map_err(|_| "No se pudo conectar con el calendario. Se muestra la copia guardada.".to_string())?;
    let status = response.status().as_u16();
    if status == 304 {
        return Ok(Fetched {
            not_modified: true,
            body: String::new(),
            etag: String::new(),
            modified: String::new(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => "Google rechazó la dirección iCal: revisá que siga siendo la secreta.".to_string(),
            404 => "La dirección iCal no existe (HTTP 404).".to_string(),
            other => format!("El calendario respondió HTTP {other}."),
        });
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| "La respuesta del calendario no se pudo leer.".to_string())?;
    let header = |name: &str| {
        response.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
    };
    Ok(Fetched { not_modified: false, body, etag: header("etag"), modified: header("last-modified") })
}

/// Seconds of the noon of `day`, so the daily note of a day is named by that day whatever the
/// offset of the zone: at noon a date cannot move when the clock changes.
fn day_noon(day: Day) -> u64 {
    let base = i64::from(day.index()) * 86_400 + 43_200;
    let offset = crate::datetime::local_offset(base.max(0) as u64);
    (base - offset).max(0) as u64
}

impl crate::shell::App {
    /// Handle the `calendar:*` and `cal:*` commands. Returns whether `command` was one of them.
    pub(crate) fn run_calendar(&mut self, command: &str) -> bool {
        if let Some(rest) = command.strip_prefix("cal:day:") {
            if let Ok(index) = rest.parse::<i32>() {
                self.calendar.select(Day::from_index(index));
                self.focus = crate::shell::Focus::Calendar;
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("cal:event:") {
            let mut parts = rest.split(':');
            if let (Some(day), Some(at)) = (parts.next(), parts.next()) {
                if let (Ok(day), Ok(at)) = (day.parse::<i32>(), at.parse::<usize>()) {
                    let key = (Day::from_index(day), at);
                    self.calendar.detail = (self.calendar.detail != Some(key)).then_some(key);
                    self.calendar.desc_open = false;
                }
            }
            return true;
        }
        let root = self.store.root.clone();
        let url = self.config.app.str("cal.ical_url").unwrap_or_default().to_string();
        match command {
            "calendar:open" => {
                self.left_open = true;
                self.panel = crate::shell::Panel::Calendar;
                self.focus = crate::shell::Focus::Calendar;
                self.calendar.mark_dirty();
                self.calendar.maybe_fetch(&root, &url);
            }
            "calendar:refresh" | "cal:refresh" => {
                self.left_open = true;
                self.panel = crate::shell::Panel::Calendar;
                self.calendar.refresh(&root, &url);
            }
            "calendar:open-day-note" => {
                let secs = day_noon(self.calendar.selected);
                self.open_daily_note_on(secs);
            }
            "cal:prev" => {
                self.calendar.step_month(-1);
                self.focus = crate::shell::Focus::Calendar;
            }
            "cal:next" => {
                self.calendar.step_month(1);
                self.focus = crate::shell::Focus::Calendar;
            }
            "cal:today" => {
                self.calendar.select(Calendar::today());
                self.focus = crate::shell::Focus::Calendar;
            }
            "cal:desc" => self.calendar.desc_open = !self.calendar.desc_open,
            _ => return false,
        }
        true
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

/// Width below which the grid is replaced by the list of the day: seven columns in a sidebar
/// narrower than this are too small to read a number in.
pub const PANE_MIN_GRID: f32 = 320.0;

/// Events drawn in one cell before the rest is counted as "+N".
const DOTS_PER_CELL: usize = 4;

/// The calendar pane of the left sidebar: the month (or, in a narrow sidebar, the selected day),
/// and under it the events of the selected day.
pub fn panel(app: &crate::shell::App, dom: &mut Dom, parent: NodeId, viewport_h: f32) {
    let cal = &app.calendar;
    let header = dom.add(parent, El::div().class("nav-header"));
    dom.add(header, icons::el("calendar"));
    dom.add(header, El::div().class("nav-header-title").text("Calendario"));
    let refresh = dom.add(
        header,
        El::div().class("clickable-icon").class_if(cal.fetching, "is-busy").action("cal:refresh"),
    );
    dom.add(refresh, icons::el("rotate-ccw"));

    let nav = dom.add(parent, El::div().class("calendar-nav"));
    let button = |dom: &mut Dom, icon: &str, action: &str| {
        let b = dom.add(nav, El::div().class("clickable-icon").action(action));
        dom.add(b, icons::el(icon));
    };
    button(dom, "arrow-left", "cal:prev");
    let (year, month, _) = cal.month.ymd();
    let title = format!("{} {year}", MONTHS.get(month as usize - 1).copied().unwrap_or_default());
    dom.add(nav, El::div().class("calendar-nav-title").text(title));
    button(dom, "arrow-right", "cal:next");
    dom.add(nav, El::div().class("calendar-today").text("Hoy").action("cal:today"));

    if let Some(error) = &cal.error {
        dom.add(parent, El::div().class("calendar-error").text(error.clone()));
    }
    let has_url = !app.config.app.str("cal.ical_url").unwrap_or_default().trim().is_empty();
    if !has_url && cal.files.is_empty() && cal.tasks.is_empty() && cal.error.is_none() {
        dom.add(
            parent,
            El::div()
                .class("pane-empty")
                .text("Conectá tu calendario en Ajustes → Calendario, o agregá un archivo .ics a la bóveda."),
        );
    }

    let today = Calendar::today();
    if app.left_w >= PANE_MIN_GRID {
        grid(cal, dom, parent, today);
    }
    day_panel(app, dom, parent, today, viewport_h);
}

/// The six weeks of the month, a row of weekday labels above them.
fn grid(cal: &Calendar, dom: &mut Dom, parent: NodeId, today: Day) {
    let labels = dom.add(parent, El::div().class("calendar-weekdays"));
    for name in WEEKDAYS {
        dom.add(labels, El::div().class("calendar-weekday").text(name));
    }
    let body = dom.add(parent, El::div().class("calendar-grid"));
    let start = grid_start(cal.month);
    for week in 0..6 {
        let row = dom.add(body, El::div().class("calendar-row"));
        for weekday in 0..7 {
            let day = start.add_days(week * 7 + weekday);
            let classes = "calendar-cell";
            let cell = dom.add(
                row,
                El::div()
                    .class(classes)
                    .class_if(day.month_start() != cal.month, "is-other-month")
                    .class_if(day == today, "is-today")
                    .class_if(day == cal.selected, "is-selected")
                    .action(format!("cal:day:{}", day.index())),
            );
            let (_, _, number) = day.ymd();
            dom.add(cell, El::div().class("calendar-day-num").text(number.to_string()));
            let count = cal.count_on(day);
            if count > 0 {
                let marks = dom.add(cell, El::div().class("calendar-marks"));
                for row in cal.rows_on(day).take(DOTS_PER_CELL) {
                    dom.add(
                        marks,
                        El::div()
                            .class("calendar-dot")
                            .class(row.source.css())
                            .attr("aria-label", row.title.as_str()),
                    );
                }
                if count > DOTS_PER_CELL {
                    dom.add(
                        marks,
                        El::div().class("calendar-more").text(format!("+{}", count - DOTS_PER_CELL)),
                    );
                }
            }
        }
    }
}

/// The events of the selected day, with the button that opens its daily note.
fn day_panel(app: &crate::shell::App, dom: &mut Dom, parent: NodeId, today: Day, viewport_h: f32) {
    let cal = &app.calendar;
    let day = cal.selected;
    let head = dom.add(parent, El::div().class("calendar-day-head"));
    let (year, month, number) = day.ymd();
    let name = DAYS[day.weekday() as usize];
    let month = MONTHS.get(month as usize - 1).copied().unwrap_or_default();
    let suffix = if day == today { " · hoy" } else { "" };
    dom.add(
        head,
        El::div().class("calendar-day-title").text(format!("{name} {number} de {month} de {year}{suffix}")),
    );
    let open = dom.add(
        head,
        El::div().class("calendar-day-open").text("Abrir nota del día").action("calendar:open-day-note"),
    );
    dom.add(open, icons::el("note"));

    let list = dom.add(parent, El::div().class("nav-files-container calendar-day-list").id("calendar-day"));
    let rows: Vec<&Row> = cal.rows_on(day).collect();
    let _ = viewport_h;
    if rows.is_empty() {
        dom.add(list, El::div().class("pane-empty").text("Sin eventos este día."));
    }
    for (at, row) in rows.iter().enumerate() {
        let open_detail = cal.detail == Some((day, at));
        let line = dom.add(
            list,
            El::div()
                .class("calendar-event")
                .class(row.source.css())
                .class_if(open_detail, "is-open")
                .action(format!("cal:event:{}:{}", day.index(), at)),
        );
        let clock = match row.minute {
            Some(minute) => format!("{:02}:{:02}", minute / 60, minute % 60),
            None => "Todo el día".to_string(),
        };
        dom.add(line, El::div().class("calendar-event-time").text(clock));
        dom.add(line, El::div().class("calendar-event-title").text(row.title.clone()));
        if open_detail {
            detail(dom, list, row, cal.desc_open);
        }
    }
}

/// The open detail of an event: where it is, what it says, and where it came from.
fn detail(dom: &mut Dom, parent: NodeId, row: &Row, desc_open: bool) {
    let box_ = dom.add(parent, El::div().class("calendar-detail"));
    if !row.location.is_empty() {
        let line = dom.add(box_, El::div().class("calendar-detail-line"));
        dom.add(line, icons::el("home"));
        dom.add(line, El::div().class("calendar-detail-text").text(row.location.clone()));
    }
    if !row.description.is_empty() {
        let limit = if desc_open { 4000 } else { 180 };
        let text: String = row.description.chars().take(limit).collect();
        let more = row.description.chars().count() > limit;
        dom.add(box_, El::div().class("calendar-detail-text").text(text));
        if more || desc_open {
            dom.add(
                box_,
                El::div()
                    .class("calendar-detail-more")
                    .text(if desc_open { "menos" } else { "más" })
                    .action("cal:desc"),
            );
        }
    }
    let source = dom.add(box_, El::div().class("calendar-detail-source"));
    dom.add(source, El::div().class("calendar-dot").class(row.source.css()));
    dom.add(source, El::div().class("calendar-detail-text").text(format!("Origen: {}", row.source.label())));
    if row.source != Source::Google {
        dom.add(
            box_,
            El::div()
                .class("calendar-detail-open")
                .text(format!("Abrir {}", row.note))
                .action(format!("open:{}", row.note)),
        );
    }
    if row.source == Source::Google {
        dom.add(
            box_,
            El::div().class("calendar-readonly").text(
                "Los eventos de Google son de solo lectura: MetaMind los muestra desde la dirección iCal.",
            ),
        );
    }
}

/// The weekdays of the day panel, Sunday first, as `Day::weekday` counts them.
const DAYS: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_line_gives_its_day_and_title() {
        let (day, title) = task_line("- [ ] Llamar al banco @{2025-09-16}").expect("hay fecha");
        assert_eq!(day, Day::from_ymd(2025, 9, 16).expect("existe"));
        assert_eq!(title, "Llamar al banco");
        let (day, title) = task_line("Presentar informe @{2025-09-16} mañana").expect("hay fecha");
        assert_eq!(day, Day::from_ymd(2025, 9, 16).expect("existe"));
        assert_eq!(title, "Presentar informe mañana");
        assert!(task_line("sin fecha").is_none());
        assert!(task_line("@{2025-9-16}").is_none(), "el formato es estricto");
        assert!(task_line("@{2025-02-30}").is_none(), "y la fecha tiene que existir");
    }

    #[test]
    fn tasks_are_read_from_a_note() {
        let text = "# Semana\n- [ ] Uno @{2025-09-16}\n- [x] Dos @{2025-09-17}\n\nOtra cosa\n";
        let tasks = tasks_of("Notas.md", text);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].title, "Dos");
        assert_eq!(tasks[1].note, "Notas.md");
    }

    #[test]
    fn the_grid_starts_on_a_monday() {
        let month = Day::from_ymd(2025, 9, 1).expect("existe");
        let start = grid_start(month);
        assert_eq!(start.weekday(), 1);
        // 2025-09-01 is a Monday itself.
        assert_eq!(start, month);
        // 2025-10-01 is a Wednesday: the grid starts on Monday the 29th of September.
        let october = Day::from_ymd(2025, 10, 1).expect("existe");
        assert_eq!(grid_start(october), Day::from_ymd(2025, 9, 29).expect("existe"));
        // Six weeks always fit a month.
        assert!(grid_start(october).add_days(41) >= Day::from_ymd(2025, 10, 31).expect("existe"));
    }
}
