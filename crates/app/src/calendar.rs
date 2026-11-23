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
use calendar_core::local::{color_hex, Owned, OwnedEvent, NAMED_COLORS};
use calendar_core::{Calendar as Ics, Day, Occurrence};
use metamind_ui::{Dom, El, NodeId};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use vault_core::{fsio, path, VaultStore};

/// The events the user writes, under the vault. Read and written atomically by `calendar-core`.
pub const OWNED_JSON: &str = ".metamind/calendar/events.json";

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
    /// An event the user wrote, in the vault's own `events.json`.
    Owned,
    Tasks,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Google => "Google",
            Source::Vault => "Bóveda",
            Source::Owned => "Propios",
            Source::Tasks => "Tareas",
        }
    }

    pub fn css(self) -> &'static str {
        match self {
            Source::Google => "calendar-source-google",
            Source::Vault => "calendar-source-vault",
            Source::Owned => "calendar-source-owned",
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
    /// The colour an event the user wrote carries, as a palette id or `#rrggbb`.
    pub color: Option<String>,
}

/// A task line of a note: `- [ ] Llamar @{2025-09-16}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// Whether the line is already checked (`- [x]`).
    pub done: bool,
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
#[derive(Debug)]
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
    /// The events the user wrote, and why the file could not be read when it could not.
    pub owned: Owned,
    pub owned_note: Option<String>,
    /// The event being written, when the editor is open.
    pub form: Option<Form>,
    /// Why the last fetch failed, in words that never carry the URL.
    pub error: Option<String>,
    /// When the cached body was last confirmed by the server.
    pub fetched_ms: u64,
    /// Whether a fetch is running, and for which address.
    pub fetching: bool,
    /// A fetch was asked for by hand ("Probar"): its result is reported once as a notice.
    announce: bool,
    report: Option<Result<usize, String>>,
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
        let (owned, owned_note) = Owned::load(&root.join(OWNED_JSON));
        let mut cal = Calendar {
            month: today.month_start(),
            selected: today,
            owned,
            owned_note,
            form: None,
            detail: None,
            desc_open: false,
            rows: Vec::new(),
            error: None,
            fetched_ms: 0,
            fetching: false,
            announce: false,
            report: None,
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

    /// Have the next answer reported once, which is what the "Probar" button does.
    pub fn announce(&mut self) {
        self.announce = true;
    }

    /// The result of the announced fetch, once: how many events it found, or why it failed.
    pub fn take_report(&mut self) -> Option<Result<usize, String>> {
        self.report.take()
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
                if std::mem::take(&mut self.announce) {
                    let count = self.google.as_ref().map_or(0, |c| c.events.len());
                    self.report = Some(Ok(count));
                }
            }
            // The cached events stay on screen: an old calendar is better than an empty pane.
            Err(message) => {
                if std::mem::take(&mut self.announce) {
                    self.report = Some(Err(message.clone()));
                }
                self.error = Some(message);
            }
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
    pub fn rebuild(&mut self) {
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
        // The events the user wrote come first: they are the ones the app can edit, and a row of
        // theirs must be the one on top of a Google event with the same id.
        for (index, occ) in self.owned.expand(from, to) {
            let Some(ev) = self.owned.events.get(index) else { continue };
            rows.push(Row {
                day: occ.start.day,
                minute: occ.start.minute,
                title: if ev.title.trim().is_empty() {
                    "(sin título)".to_string()
                } else {
                    ev.title.clone()
                },
                location: ev.location.clone(),
                description: ev.description.clone(),
                source: Source::Owned,
                note: ev.note.clone().unwrap_or_default(),
                uid: ev.id.clone(),
                color: Some(ev.color.clone()),
            });
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
                    color: None,
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

    /// Pick a day. The month only follows when the day is outside the six weeks on screen: a
    /// click on a cell must not move the grid under the pointer.
    pub fn select(&mut self, day: Day) {
        if self.selected == day {
            return;
        }
        self.selected = day;
        let start = grid_start(self.month);
        if day < start || day > start.add_days(41) {
            self.month = day.month_start();
        }
        self.detail = None;
        self.dirty = true;
    }

    /// Pick a day and show its month, which is what "Hoy" and the week view do.
    pub fn select_in_month(&mut self, day: Day) {
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
            color: None,
        });
    }
}

/// The tasks of a note: any line that carries `@{YYYY-MM-DD}`.
impl crate::shell::App {
    /// Marks the task of the day panel done, or not done: the checkbox of its line in its note.
    fn toggle_task(&mut self, day: Day, at: usize) {
        // A click always comes after the calendar has drawn, but a command can arrive before the
        // first frame: drain the scan and rebuild the rows so the row it names exists.
        let root = self.store.root.clone();
        let rev = crate::dataview::revision(self);
        if rev != self.calendar.revision() {
            self.calendar.scan_vault(&self.store, rev);
        }
        for _ in 0..8 {
            self.calendar.tick(&root, &self.store);
        }
        let Some(row) = self.calendar.rows_on(day).nth(at).cloned() else {
            self.notice = Some("La tarea ya no esta en la lista".to_string());
            return;
        };
        if row.source != Source::Tasks {
            return;
        }
        let rel = row.note.clone();
        let Ok((text, stamp)) = self.store.read(&rel) else {
            self.notice = Some(format!("No se pudo leer {rel}"));
            return;
        };
        let Some(next) = toggle_task_line(&text, day, &row.title) else {
            self.notice = Some("La tarea ya no esta en la nota".to_string());
            return;
        };
        let done = next != text && task_done(&next, day, &row.title);
        if let Err(e) = self.store.write(&rel, &next, Some(stamp)) {
            self.notice = Some(format!("No se pudo guardar {rel}: {e}"));
            return;
        }
        let rev = crate::dataview::revision(self);
        self.calendar.scan_vault(&self.store, rev);
        self.calendar.mark_dirty();
        self.notice = Some(if done {
            format!("Hecha: {}", row.title)
        } else {
            format!("Pendiente otra vez: {}", row.title)
        });
    }
}

/// The same note with the checkbox of the task `title` on `day` the other way round, or `None`
/// when no line of the note says that task any more.
pub fn toggle_task_line(text: &str, day: Day, title: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 8);
    let mut found = false;
    for line in text.split_inclusive('\n') {
        if found {
            out.push_str(line);
            continue;
        }
        let body = line.strip_suffix('\n').unwrap_or(line);
        let hit = task_line(body).is_some_and(|(d, t)| d == day && t == title);
        if hit {
            found = true;
            out.push_str(&flip_checkbox(body));
        } else {
            out.push_str(body);
        }
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    found.then_some(out)
}

/// Whether the task `title` on `day` is already checked in `text`.
fn task_done(text: &str, day: Day, title: &str) -> bool {
    text.lines().any(|line| task_line(line).is_some_and(|(d, t)| d == day && t == title) && line_done(line))
}

/// Whether a task line is checked.
fn line_done(line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix(['-', '*', '+']).unwrap_or(t).trim_start();
    t.starts_with("[x]") || t.starts_with("[X]")
}

/// The same line with its checkbox flipped. A line without one comes back untouched.
fn flip_checkbox(line: &str) -> String {
    for (from, to) in [("[ ]", "[x]"), ("[x]", "[ ]"), ("[X]", "[ ]")] {
        if let Some(i) = line.find(from) {
            let mut out = String::with_capacity(line.len());
            out.push_str(&line[..i]);
            out.push_str(to);
            out.push_str(&line[i + from.len()..]);
            return out;
        }
    }
    line.to_string()
}

pub fn tasks_of(note: &str, text: &str) -> Vec<Task> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((day, title)) = task_line(line) else { continue };
        out.push(Task { done: line_done(line), day, title, note: note.to_string() });
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
    // The date left a gap where it was: the title reads as one line.
    let mut out = String::with_capacity(title.len());
    let mut space = false;
    for c in title.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    Some((day, out))
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
        // The editor: every command below belongs to the form, and leaves it as it was when it
        // has nothing to do.
        if let Some(rest) = command.strip_prefix("cal:field:") {
            if let (Ok(index), Some(form)) = (rest.parse::<usize>(), &mut self.calendar.form) {
                form.field = index.min(FIELDS - 1);
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("cal:color:") {
            if let Some(form) = &mut self.calendar.form {
                if calendar_core::local::valid_color(rest) {
                    form.color = rest.to_string();
                }
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("cal:stepday:") {
            if let (Ok(delta), Some(form)) = (rest.parse::<i32>(), &mut self.calendar.form) {
                form.day = form.day.add_days(delta.clamp(-366, 366));
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("cal:step:") {
            let mut parts = rest.split(':');
            let which = parts.next().unwrap_or_default();
            let delta = parts.next().and_then(|d| d.parse::<i32>().ok()).unwrap_or(0);
            if let Some(form) = &mut self.calendar.form {
                let step = |value: u16, delta: i32| -> u16 {
                    let v = i32::from(value) + delta;
                    v.clamp(0, 24 * 60 - 1) as u16
                };
                match which {
                    "start" => {
                        form.start = step(form.start, delta);
                        if form.end < form.start {
                            form.end = form.start;
                        }
                    }
                    "end" => form.end = step(form.end, delta),
                    _ => {}
                }
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("cal:edit:") {
            if !self.calendar.edit_form(rest) {
                // Not one of ours: the detail of a read-only event is all there is to show.
                if let Some(row) = self.calendar.rows.iter().find(|r| r.uid == rest) {
                    let (day, at) = (row.day, 0);
                    self.calendar.detail = Some((day, at));
                }
            }
            return true;
        }
        if let Some(rest) = command.strip_prefix("cal:event:") {
            let mut parts = rest.split(':');
            if let (Some(day), Some(at)) = (parts.next(), parts.next()) {
                if let (Ok(day), Ok(at)) = (day.parse::<i32>(), at.parse::<usize>()) {
                    let day = Day::from_index(day);
                    // An event of ours opens its editor; one that comes from an `.ics` file has
                    // nothing to edit and shows its detail instead.
                    let owned = self
                        .calendar
                        .rows_on(day)
                        .nth(at)
                        .filter(|r| r.source == Source::Owned)
                        .map(|r| r.uid.clone());
                    match owned {
                        Some(uid) => {
                            self.calendar.edit_form(&uid);
                        }
                        None => {
                            let key = (day, at);
                            self.calendar.detail = (self.calendar.detail != Some(key)).then_some(key);
                            self.calendar.desc_open = false;
                        }
                    }
                }
            }
            return true;
        }
        // A task of the day panel is a line of a note: the checkbox writes the line back.
        if let Some(rest) = command.strip_prefix("cal:done:") {
            let mut parts = rest.split(':');
            if let (Some(day), Some(at)) = (parts.next(), parts.next()) {
                if let (Ok(day), Ok(at)) = (day.parse::<i32>(), at.parse::<usize>()) {
                    self.toggle_task(Day::from_index(day), at);
                }
            }
            return true;
        }
        let root = self.store.root.clone();
        let url = self.config.app.str("cal.ical_url").unwrap_or_default().to_string();
        match command {
            "cal:none" => {}
            "cal:new" | "cal:today-new" => {
                self.calendar.new_form(self.calendar.selected);
            }
            "cal:save" => self.save_form(),
            "cal:cancel" => {
                self.calendar.form = None;
            }
            "cal:del" => self.delete_form_event(),
            "cal:allday" => {
                if let Some(form) = &mut self.calendar.form {
                    form.all_day = !form.all_day;
                }
            }
            "calendar:open" => {
                // The bottom navigation of a compact window follows the destination.
                self.nav_go(2.0);
                // Through `show_side`: a floating sidebar makes room for the one asked for, so the
                // calendar is not opened behind a copilot sheet that covers it.
                self.show_side(true);
                self.panel = crate::shell::Panel::Calendar;
                self.focus = crate::shell::Focus::Calendar;
                self.calendar.mark_dirty();
                self.calendar.maybe_fetch(&root, &url);
            }
            "calendar:refresh" | "cal:refresh" => {
                self.show_side(true);
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
                self.calendar.select_in_month(Calendar::today());
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

/// Width below which the grid is replaced by the list of the day: at the narrowest the sidebar
/// goes (180 px, see `SIDE_W_MIN`) seven columns cannot hold a day number. The default width
/// (250 px) keeps the grid, with the events of a day as dots.
pub const PANE_MIN_GRID: f32 = 210.0;

/// Events drawn in one cell before the rest is counted as "+N".
const DOTS_PER_CELL: usize = 4;
/// Chips (`09:30 Título`) drawn in one cell before the rest is counted: a chip is three lines
/// tall, and a cell that showed ten would leave no room for its own number.
const CHIPS_PER_CELL: usize = 3;

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
        grid(cal, dom, parent, today, app.left_w);
    }
    day_panel(app, dom, parent, today, viewport_h);
}

/// The six weeks of the month, a row of weekday labels above them. The cells share their borders
/// —one frame around the block, a line between cells— so the month reads as a calendar and not as
/// twenty-eight separate chips.
fn grid(cal: &Calendar, dom: &mut Dom, parent: NodeId, today: Day, width: f32) {
    let labels = dom.add(parent, El::div().class("calendar-weekdays"));
    for name in WEEKDAYS {
        dom.add(labels, El::div().class("calendar-weekday").text(name));
    }
    let body = dom.add(parent, El::div().class("calendar-grid"));
    let start = grid_start(cal.month);
    // A cell narrower than this cannot hold `09:30 Título`: there the day shows dots instead.
    let cell_w = (width - 16.0 - 2.0) / 7.0;
    let chips = cell_w >= 46.0;
    for week in 0..6 {
        let row = dom.add(body, El::div().class("calendar-row"));
        for weekday in 0..7 {
            let day = start.add_days(week * 7 + weekday);
            let cell = dom.add(
                row,
                El::div()
                    .class("calendar-cell")
                    .class_if(day.month_start() != cal.month, "is-other-month")
                    .class_if(day == today, "is-today")
                    .class_if(day == cal.selected, "is-selected")
                    .action(format!("cal:day:{}", day.index())),
            );
            let (_, _, number) = day.ymd();
            let head = dom.add(cell, El::div().class("calendar-day-head"));
            dom.add(head, El::div().class("calendar-day-num").text(number.to_string()));
            let rows: Vec<&Row> = cal.rows_on(day).collect();
            if chips && !rows.is_empty() {
                let list = dom.add(cell, El::div().class("calendar-chips"));
                for row in rows.iter().take(CHIPS_PER_CELL) {
                    let color = row.color.as_deref().map(color_hex);
                    let chip = dom.add(list, El::div().class("calendar-chip").class(row.source.css()));
                    if let Some(hex) = &color {
                        dom.nodes[chip].el.inline_style =
                            Some(format!("background-color: {}1f; border-left-color: {hex}", hex));
                    }
                    let clock = match row.minute {
                        Some(m) => format!("{:02}:{:02} ", m / 60, m % 60),
                        None => String::new(),
                    };
                    dom.add(
                        chip,
                        El::div().class("calendar-chip-text").text(format!("{clock}{}", row.title)),
                    );
                }
                if rows.len() > CHIPS_PER_CELL {
                    dom.add(
                        list,
                        El::div().class("calendar-more").text(format!("+{}", rows.len() - CHIPS_PER_CELL)),
                    );
                }
            } else if !rows.is_empty() {
                let marks = dom.add(cell, El::div().class("calendar-marks"));
                for row in rows.iter().take(DOTS_PER_CELL) {
                    let dot = dom.add(marks, El::div().class("calendar-dot").class(row.source.css()));
                    dom.nodes[dot].el.attrs.push(("aria-label".into(), row.title.clone()));
                    if let Some(hex) = row.color.as_deref().map(color_hex) {
                        dom.nodes[dot].el.inline_style = Some(format!("background-color: {hex}"));
                    }
                }
                if rows.len() > DOTS_PER_CELL {
                    dom.add(
                        marks,
                        El::div().class("calendar-more").text(format!("+{}", rows.len() - DOTS_PER_CELL)),
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
        El::div()
            .class("calendar-day-title")
            .id("calendar-day-title")
            .attr("data-day", &day.index().to_string())
            .text(format!("{name} {number} de {month} de {year}{suffix}")),
    );
    let add = dom.add(head, El::div().class("calendar-day-add").action("cal:new"));
    dom.add(add, icons::el("plus"));
    dom.add(add, El::div().class("calendar-day-open-label").text("Evento"));
    let open = dom.add(head, El::div().class("calendar-day-open").action("calendar:open-day-note"));
    // The icon and the label are two boxes: the text of a box is painted at its origin, so an
    // icon added next to it landed on top of the first letter.
    dom.add(open, icons::el("note").class("calendar-day-open-icon"));
    dom.add(open, El::div().class("calendar-day-open-label").text("Nota"));

    let list = dom.add(parent, El::div().class("nav-files-container calendar-day-list").id("calendar-day"));
    let rows: Vec<&Row> = cal.rows_on(day).collect();
    let _ = viewport_h;
    if rows.is_empty() {
        let empty = dom.add(list, El::div().class("pane-empty").text("Sin eventos este día."));
        dom.add(empty, El::div().class("pane-empty-action").text("Crear evento").action("cal:new"));
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
        if let Some(hex) = row.color.as_deref().map(color_hex) {
            dom.nodes[line].el.inline_style = Some(format!("border-left-color: {hex}"));
        }
        let clock = match row.minute {
            Some(minute) => format!("{:02}:{:02}", minute / 60, minute % 60),
            None => "Todo el día".to_string(),
        };
        if row.source == Source::Tasks {
            // The line is a task of a note: the box writes the note back.
            let done = cal
                .tasks
                .get(&row.note)
                .and_then(|tasks| tasks.iter().find(|t| t.title == row.title))
                .is_some_and(|t| t.done);
            let check = dom.add(
                line,
                El::div().class("calendar-task-check").class_if(done, "is-done").action(format!(
                    "cal:done:{}:{}",
                    day.index(),
                    at
                )),
            );
            dom.add(check, icons::el("check").class("calendar-task-check-mark"));
        }
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
    if row.source == Source::Owned {
        dom.add(
            box_,
            El::div()
                .class("calendar-detail-open")
                .text("Editar evento")
                .action(format!("cal:edit:{}", row.uid)),
        );
    }
    if row.source != Source::Google && row.source != Source::Owned && !row.note.is_empty() {
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
mod task_tests {
    use super::*;

    fn day() -> Day {
        Day::from_ymd(2026, 11, 22).unwrap()
    }

    #[test]
    fn a_task_line_is_flipped_and_the_rest_of_the_note_is_untouched() {
        let text =
            "---\ntipo: diario\n---\n\n- [ ] Comprar pan @{2026-11-22}\n- [ ] Otra cosa @{2026-11-23}\n";
        let next = toggle_task_line(text, day(), "Comprar pan").expect("the task is there");
        assert!(next.contains("- [x] Comprar pan"), "{next}");
        assert!(next.contains("- [ ] Otra cosa"), "the other task keeps its box");
        assert!(next.contains("tipo: diario"), "the front matter is untouched");
        // And back.
        let back = toggle_task_line(&next, day(), "Comprar pan").expect("still there");
        assert!(back.contains("- [ ] Comprar pan"), "{back}");
        assert!(!line_done("- [ ] Comprar pan"));
        assert!(line_done("- [x] Comprar pan"));
        assert!(line_done("* [X] Comprar pan"));
    }

    #[test]
    fn a_task_that_moved_day_is_not_the_one_flipped() {
        let text = "- [ ] Comprar pan @{2026-11-23}\n";
        assert!(toggle_task_line(text, day(), "Comprar pan").is_none());
        // A note with the same title on another day keeps its box.
        let both = "- [ ] Comprar pan @{2026-11-22}\n- [ ] Comprar pan @{2026-11-23}\n";
        let next = toggle_task_line(both, day(), "Comprar pan").unwrap();
        assert_eq!(next.matches("- [x]").count(), 1, "{next}");
        assert_eq!(next.matches("- [ ]").count(), 1, "{next}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loopback server that answers one GET with `status`, `body` and the headers given. Returns
    /// the URL and the request head the server saw.
    fn mock_server(
        status: &'static str,
        headers: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("puerto libre");
        let addr = listener.local_addr().expect("dirección local");
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("una conexión");
            let mut reader = BufReader::new(sock.try_clone().expect("clon del socket"));
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let reply = format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes());
            head
        });
        (format!("http://{addr}/secret-address/basic.ics"), handle)
    }

    #[test]
    fn a_304_keeps_the_cached_body() {
        let (url, server) = mock_server("304 Not Modified", "", "");
        let meta = Meta { etag: "\"v1\"".to_string(), ..Meta::default() };
        let fetched = fetch(&url, &meta).expect("304 es una respuesta");
        assert!(fetched.not_modified);
        assert!(fetched.body.is_empty());
        // The cache's validator is what the request asked for.
        assert!(server.join().expect("el hilo del servidor").contains("if-none-match: \"v1\""));
    }

    #[test]
    fn the_body_and_its_validators_come_back() {
        let body = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20250916\nEND:VEVENT\nEND:VCALENDAR\n";
        let (url, server) =
            mock_server("200 OK", "ETag: \"v2\"\r\nLast-Modified: Tue, 16 Sep 2025 09:00:00 GMT\r\n", body);
        let fetched = fetch(&url, &Meta::default()).expect("la respuesta llega");
        assert!(!fetched.not_modified);
        assert_eq!(fetched.body, body);
        assert_eq!(fetched.etag, "\"v2\"");
        assert_eq!(fetched.modified, "Tue, 16 Sep 2025 09:00:00 GMT");
        server.join().expect("el hilo del servidor");
    }

    #[test]
    fn a_failed_fetch_never_repeats_the_address() {
        let (url, server) = mock_server("403 Forbidden", "", "");
        let error = fetch(&url, &Meta::default()).expect_err("403 no es una respuesta válida");
        assert!(error.contains("403") || error.contains("rechaz"), "{error}");
        // The address is a secret: the message says what happened, not where.
        assert!(!error.contains("secret-address"), "{error}");
        assert!(!error.contains("127.0.0.1"), "{error}");
        server.join().expect("el hilo del servidor");
    }

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

// ---- the event editor ----

/// The event being written, with the field the keyboard is in.
///
/// The form is a value of its own so the modal can be drawn from it and the save can be tested
/// without a window: `to_event` is where the rules live (an end before the start is a mistake, an
/// empty title is not).
#[derive(Clone, PartialEq, Debug)]
pub struct Form {
    /// The event being edited; `None` while a new one is being written.
    pub id: Option<String>,
    pub title: String,
    pub location: String,
    pub description: String,
    pub day: Day,
    pub all_day: bool,
    /// Minutes after midnight. Ignored while `all_day`.
    pub start: u16,
    pub end: u16,
    pub color: String,
    /// 0 title, 1 location, 2 description: where a letter goes.
    pub field: usize,
    pub error: Option<String>,
    /// The event as it was when the form opened, so a save can be undone from the notice.
    pub before: Option<OwnedEvent>,
    /// The delete was asked for once: the second press does it.
    pub confirm_delete: bool,
}

/// How many text fields the form has.
pub const FIELDS: usize = 3;

impl Form {
    /// A new event on `day`, at the next quarter of an hour.
    pub fn new(day: Day, now_minute: u16) -> Form {
        let start = (now_minute / 15 + 1) * 15;
        let start = if start >= 23 * 60 { 9 * 60 } else { start };
        Form {
            id: None,
            title: String::new(),
            location: String::new(),
            description: String::new(),
            day,
            all_day: false,
            start,
            end: (start + 60).min(24 * 60 - 1),
            color: NAMED_COLORS[0].to_string(),
            field: 0,
            error: None,
            before: None,
            confirm_delete: false,
        }
    }

    /// The form that edits `ev`.
    pub fn from_event(ev: &OwnedEvent) -> Form {
        Form {
            id: Some(ev.id.clone()),
            title: ev.title.clone(),
            location: ev.location.clone(),
            description: ev.description.clone(),
            day: ev.day,
            all_day: ev.is_all_day(),
            start: ev.start_minute.unwrap_or(9 * 60),
            end: ev.end_minute.unwrap_or(10 * 60).max(ev.start_minute.unwrap_or(9 * 60)),
            color: ev.color.clone(),
            field: 0,
            error: None,
            before: Some(ev.clone()),
            confirm_delete: false,
        }
    }

    /// The text of the field the keyboard is in.
    pub fn active(&self) -> &str {
        match self.field {
            1 => &self.location,
            2 => &self.description,
            _ => &self.title,
        }
    }

    pub fn active_mut(&mut self) -> &mut String {
        match self.field {
            1 => &mut self.location,
            2 => &mut self.description,
            _ => &mut self.title,
        }
    }

    /// The event this form describes, or why it cannot be one yet.
    pub fn to_event(&self, now_ms: u64) -> Result<OwnedEvent, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Poné un título: es lo que se ve en la grilla.".to_string());
        }
        if !self.all_day && self.end < self.start {
            return Err("La hora de fin es anterior a la de inicio.".to_string());
        }
        let mut ev = OwnedEvent::all_day(title, self.day, &self.color);
        ev.id = self.id.clone().unwrap_or_default();
        ev.location = self.location.trim().to_string();
        ev.description = self.description.trim().to_string();
        ev.color = if calendar_core::local::valid_color(&self.color) {
            self.color.clone()
        } else {
            NAMED_COLORS[0].to_string()
        };
        if !self.all_day {
            ev.start_minute = Some(self.start);
            ev.end_minute = Some(self.end.max(self.start));
        }
        ev.updated_ms = now_ms;
        ev.created_ms = now_ms;
        Ok(ev)
    }

    /// `09:30`, or `Todo el día`.
    pub fn when(&self) -> String {
        if self.all_day {
            return "Todo el día".to_string();
        }
        format!("{:02}:{:02}–{:02}:{:02}", self.start / 60, self.start % 60, self.end / 60, self.end % 60)
    }
}

impl Calendar {
    /// Open the editor on a new event for `day`.
    pub fn new_form(&mut self, day: Day) {
        let (_, _, _, h, m, _, _) = crate::datetime::local(fsio::now_ms() / 1000);
        self.form = Some(Form::new(day, h as u16 * 60 + m as u16));
    }

    /// Open the editor on the event `uid`, when it is one the app can edit.
    pub fn edit_form(&mut self, uid: &str) -> bool {
        let Some(ev) = self.owned.events.iter().find(|e| e.id == uid) else { return false };
        self.form = Some(Form::from_event(ev));
        true
    }
}

/// The editor over the window: a small card with the event in it.
pub fn editor(app: &crate::shell::App, dom: &mut Dom) {
    let Some(form) = &app.calendar.form else { return };
    let backdrop = dom.add(0, El::div().class("modal-container mod-dim cal-modal").action("cal:cancel"));
    let card = dom.add(backdrop, El::div().class("cal-form").action("cal:none"));
    let head = dom.add(card, El::div().class("cal-form-head"));
    dom.add(
        head,
        El::div().class("cal-form-title").text(if form.id.is_some() {
            "Editar evento"
        } else {
            "Nuevo evento"
        }),
    );
    let close = dom.add(head, El::div().class("clickable-icon").action("cal:cancel"));
    dom.add(close, icons::el("x"));

    let field = |dom: &mut Dom, index: usize, label: &str, text: &str, placeholder: &str, multi: bool| {
        let row = dom.add(card, El::div().class("cal-field"));
        dom.add(row, El::div().class("cal-field-label").text(label));
        let input = dom.add(
            row,
            El::div()
                .class("cal-input")
                .class_if(form.field == index, "is-focus")
                .class_if(multi, "is-multi")
                .action(format!("cal:field:{index}")),
        );
        if text.is_empty() {
            dom.add(input, El::div().class("cal-placeholder").text(placeholder));
        } else {
            dom.add(input, El::div().class("cal-input-text").text(text.to_string()));
        }
    };
    field(dom, 0, "Título", &form.title, "Qué pasa ese día", false);

    let when = dom.add(card, El::div().class("cal-field"));
    dom.add(when, El::div().class("cal-field-label").text("Cuándo"));
    let line = dom.add(when, El::div().class("cal-when"));
    let step = |dom: &mut Dom, text: &str, action: &str| {
        dom.add(line, El::div().class("cal-step").text(text.to_string()).action(action.to_string()));
    };
    step(dom, "‹", "cal:stepday:-1");
    let (year, month, number) = form.day.ymd();
    let name = DAYS[form.day.weekday() as usize];
    let month = MONTHS.get(month as usize - 1).copied().unwrap_or_default();
    dom.add(line, El::div().class("cal-when-text").text(format!("{name} {number} de {month} de {year}")));
    step(dom, "›", "cal:stepday:1");

    let all_day = dom.add(card, El::div().class("cal-field cal-field-row"));
    dom.add(all_day, El::div().class("cal-field-label").text("Todo el día"));
    dom.add(
        all_day,
        El::div()
            .class("cal-toggle")
            .class_if(form.all_day, "is-on")
            .action("cal:allday")
            .attr("aria-label", "Todo el día"),
    );

    if !form.all_day {
        let times = dom.add(card, El::div().class("cal-field"));
        dom.add(times, El::div().class("cal-field-label").text("Horario"));
        let line = dom.add(times, El::div().class("cal-when"));
        let clock = |dom: &mut Dom, minute: u16, up: &str, down: &str| {
            let group = dom.add(line, El::div().class("cal-clock"));
            dom.add(group, El::div().class("cal-step").text("–").action(down.to_string()));
            dom.add(
                group,
                El::div().class("cal-clock-text").text(format!("{:02}:{:02}", minute / 60, minute % 60)),
            );
            dom.add(group, El::div().class("cal-step").text("+").action(up.to_string()));
        };
        clock(dom, form.start, "cal:step:start:15", "cal:step:start:-15");
        dom.add(line, El::div().class("cal-when-text").text("a"));
        clock(dom, form.end, "cal:step:end:15", "cal:step:end:-15");
    }

    let colors = dom.add(card, El::div().class("cal-field"));
    dom.add(colors, El::div().class("cal-field-label").text("Color"));
    let row = dom.add(colors, El::div().class("cal-colors"));
    for name in NAMED_COLORS {
        let on = form.color == name;
        let hex = color_hex(name);
        dom.add(
            row,
            El::div()
                .class("cal-color")
                .class_if(on, "is-active")
                .style(&format!("background-color: {hex}"))
                .action(format!("cal:color:{name}"))
                .attr("aria-label", calendar_core::local::color_label(name)),
        );
    }

    field(dom, 1, "Lugar", &form.location, "Aula, dirección, enlace", false);
    field(dom, 2, "Notas", &form.description, "Detalles del evento", true);

    if let Some(error) = &form.error {
        dom.add(card, El::div().class("cal-error").text(error.clone()));
    }
    let actions = dom.add(card, El::div().class("cal-actions"));
    let save = dom.add(actions, El::div().class("cal-button mod-cta").action("cal:save"));
    dom.add(save, El::div().text("Guardar"));
    let cancel = dom.add(actions, El::div().class("cal-button").action("cal:cancel"));
    dom.add(cancel, El::div().text("Cancelar"));
    if form.id.is_some() {
        let delete = dom.add(
            actions,
            El::div()
                .class("cal-button cal-button-danger")
                .class_if(form.confirm_delete, "is-confirming")
                .action("cal:del"),
        );
        dom.add(delete, El::div().text(if form.confirm_delete { "¿Borrar? Sí" } else { "Borrar" }));
    }
}

impl crate::shell::App {
    /// Write the event the editor is showing, and say so when it cannot be written.
    fn save_form(&mut self) {
        let root = self.store.root.clone();
        let now = fsio::now_ms();
        let Some(form) = self.calendar.form.clone() else { return };
        let mut ev = match form.to_event(now) {
            Ok(ev) => ev,
            Err(why) => {
                if let Some(form) = &mut self.calendar.form {
                    form.error = Some(why);
                }
                return;
            }
        };
        if let Some(before) = &form.before {
            ev.created_ms = before.created_ms;
        }
        self.calendar.owned.upsert(ev);
        match self.calendar.owned.save_atomic(&root.join(OWNED_JSON)) {
            Ok(()) => {
                self.calendar.form = None;
                self.calendar.owned_note = None;
                self.calendar.detail = None;
                self.calendar.select_in_month(form.day);
                // The rows are what the panel lists: rebuilt now, so the event is on screen
                // without waiting for the next frame (and a test sees it too).
                self.calendar.rebuild();
                self.notice = Some("Evento guardado".to_string());
            }
            Err(e) => {
                if let Some(form) = &mut self.calendar.form {
                    form.error = Some(format!("No se pudo guardar: {e}"));
                }
            }
        }
    }

    /// Take the event out of the file. The first press asks, the second does it.
    fn delete_form_event(&mut self) {
        let Some(form) = self.calendar.form.clone() else { return };
        if !form.confirm_delete {
            if let Some(form) = &mut self.calendar.form {
                form.confirm_delete = true;
            }
            return;
        }
        let Some(id) = form.id.clone() else { return };
        let root = self.store.root.clone();
        self.calendar.owned.remove(&id);
        match self.calendar.owned.save_atomic(&root.join(OWNED_JSON)) {
            Ok(()) => {
                self.calendar.form = None;
                self.calendar.detail = None;
                self.calendar.mark_dirty();
                self.calendar.rebuild();
                self.notice = Some("Evento borrado".to_string());
            }
            Err(e) => {
                if let Some(form) = &mut self.calendar.form {
                    form.error = Some(format!("No se pudo guardar: {e}"));
                }
            }
        }
    }

    /// The keys of the event editor. True when the key was one of them, so the note behind the
    /// modal never sees it.
    pub(crate) fn calendar_key(&mut self, key: &crate::Key) -> bool {
        if self.calendar.form.is_none() {
            return false;
        }
        match key.key.as_str() {
            "Escape" => {
                self.calendar.form = None;
                true
            }
            "Enter" => {
                self.run("cal:save");
                true
            }
            "Tab" => {
                if let Some(form) = &mut self.calendar.form {
                    let n = FIELDS;
                    form.field = if key.shift { (form.field + n - 1) % n } else { (form.field + 1) % n };
                }
                true
            }
            "Backspace" => {
                if let Some(form) = &mut self.calendar.form {
                    form.active_mut().pop();
                    form.error = None;
                }
                true
            }
            "Delete" => {
                self.run("cal:del");
                true
            }
            "ArrowUp" => {
                self.run("cal:step:start:15");
                true
            }
            "ArrowDown" => {
                self.run("cal:step:start:-15");
                true
            }
            _ => false,
        }
    }

    /// A letter or a digit while the editor is open: it goes into the focused field.
    pub(crate) fn calendar_text(&mut self, text: &str) -> bool {
        let Some(form) = &mut self.calendar.form else { return false };
        form.active_mut().push_str(text);
        form.error = None;
        true
    }
}
