//! The events the user writes: a small JSON file under `.metamind/calendar/`, and the expansion
//! that turns it into the same occurrences an `.ics` file produces.
//!
//! Why a file of our own instead of appending to a calendar file: an `.ics` file is a format with
//! `UID`s, `DTSTAMP`s and line folding, and writing it back means owning every rule of it. The
//! events here are simple: a day, an optional clock, a title, a colour. Google and the `.ics`
//! files of the vault stay read-only; this is the part the app can write.
//!
//! Storage rules, all of them to survive the user having the file open in another editor:
//!
//! - a key this crate does not know is read back verbatim: a newer build can add a field without
//!   an older one dropping it on the next save;
//! - a broken file never stops the app: the bad copy is kept next to it (`events.corrupt-*.json`)
//!   and the calendar starts empty with a notice;
//! - one broken event is dropped, not the whole file;
//! - the write is atomic (a temporary file in the same directory, then a rename), so a crash
//!   never leaves half a calendar on disk.

use crate::{parse_rrule, Calendar as Ics, Day, Event, Instant, Occurrence};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// The eight colours the palette offers, as stable ids: a file keeps `"violeta"`, not `#8a5cf5`,
/// so a theme can change the hex without touching a single event.
pub const NAMED_COLORS: [&str; 8] = ["violeta", "azul", "verde", "ambar", "rojo", "rosa", "cian", "gris"];

/// Whether `color` is one the app can draw: a named colour or `#rrggbb`.
pub fn valid_color(color: &str) -> bool {
    if NAMED_COLORS.contains(&color) {
        return true;
    }
    let Some(hex) = color.strip_prefix('#') else { return false };
    hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// One event the user wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OwnedEvent {
    /// Stable: two runs over the same event produce the same id, so a sync does not duplicate it.
    pub id: String,
    pub title: String,
    pub day: Day,
    /// `None` is an all-day event. This is the only source of truth for it: `allDay` is written
    /// for other readers but derived from here on load.
    pub start_minute: Option<u16>,
    /// End of a timed event, same day. `None` means the start (a zero-length event).
    pub end_minute: Option<u16>,
    /// A named colour of [`NAMED_COLORS`] or `#rrggbb`.
    pub color: String,
    pub location: String,
    pub description: String,
    /// The iCalendar rule, verbatim (`FREQ=WEEKLY;INTERVAL=1`): the crate already reads it, and
    /// keeping the text means a rule this crate cannot expand survives a save unchanged.
    pub rrule: Option<String>,
    /// The note this event belongs to, when it was created from one.
    pub note: Option<String>,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Keys this crate does not know, kept as read.
    pub extra: Map<String, Value>,
}

impl OwnedEvent {
    /// An all-day event on `day`, with the next free id.
    pub fn all_day(title: impl Into<String>, day: Day, color: &str) -> OwnedEvent {
        let title = title.into();
        OwnedEvent {
            id: stable_id(&title, day, None),
            title,
            day,
            start_minute: None,
            end_minute: None,
            color: if valid_color(color) { color.to_string() } else { NAMED_COLORS[0].to_string() },
            location: String::new(),
            description: String::new(),
            rrule: None,
            note: None,
            created_ms: 0,
            updated_ms: 0,
            extra: Map::new(),
        }
    }

    /// A timed event on `day`, from `start` to `end` (minutes after midnight).
    pub fn timed(title: impl Into<String>, day: Day, start: u16, end: u16, color: &str) -> OwnedEvent {
        let mut ev = OwnedEvent::all_day(title, day, color);
        ev.start_minute = Some(start.min(24 * 60 - 1));
        ev.end_minute = Some(end.max(start).min(24 * 60 - 1));
        ev.id = stable_id(&ev.title, day, ev.start_minute);
        ev
    }

    pub fn is_all_day(&self) -> bool {
        self.start_minute.is_none()
    }

    /// `(hour, minute)` of the start, for a timed event.
    pub fn clock(&self) -> Option<(u16, u16)> {
        self.start_minute.map(|m| (m / 60, m % 60))
    }

    /// `HH:MM - HH:MM`, or `Todo el día`.
    pub fn when(&self) -> String {
        match (self.start_minute, self.end_minute) {
            (Some(start), Some(end)) if end > start => {
                format!("{:02}:{:02}–{:02}:{:02}", start / 60, start % 60, end / 60, end % 60)
            }
            (Some(start), _) => format!("{:02}:{:02}", start / 60, start % 60),
            (None, _) => "Todo el día".to_string(),
        }
    }

    /// The event in the shape the expander reads. A rule that cannot be parsed is ignored instead
    /// of dropping the event: a typo in `RRULE` must not hide the appointment.
    pub fn to_event(&self) -> Event {
        let (start, end) = match (self.start_minute, self.end_minute) {
            (Some(s), Some(e)) if e > s => (Instant::at(self.day, s), Instant::at(self.day, e)),
            (Some(s), _) => (Instant::at(self.day, s), Instant::at(self.day, s)),
            // `DTEND` of an all-day event is exclusive: one day after the start.
            (None, _) => (Instant::all_day(self.day), Instant::all_day(self.day.add_days(1))),
        };
        Event {
            uid: self.id.clone(),
            summary: self.title.clone(),
            description: self.description.clone(),
            location: self.location.clone(),
            start,
            end,
            tzid: None,
            rrule: self.rrule.as_deref().and_then(parse_rrule),
        }
    }
}

/// The whole file: the events and the top-level keys this crate does not know.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Owned {
    pub events: Vec<OwnedEvent>,
    pub extra: Map<String, Value>,
}

impl Owned {
    /// Read a file body. A body that is not an object, or has no `events` array, is an error: the
    /// caller backs the file up and starts empty. A single broken event is skipped, and the
    /// message says how many were.
    pub fn parse(text: &str) -> Result<(Owned, Option<String>), String> {
        if text.trim().is_empty() {
            return Ok((Owned::default(), None));
        }
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let Value::Object(mut top) = value else { return Err("el archivo no es un objeto".into()) };
        let list = match top.remove("events") {
            Some(Value::Array(list)) => list,
            None => return Ok((Owned::default(), None)),
            Some(_) => return Err("\"events\" no es una lista".into()),
        };
        // Written by this crate: a known key, not one to keep verbatim.
        top.remove("version");
        let total = list.len();
        let mut events = Vec::with_capacity(total);
        for value in list {
            if let Some(ev) = event_from(value) {
                events.push(ev);
            }
        }
        let skipped = total - events.len();
        Ok((
            Owned { events, extra: top },
            (skipped > 0).then(|| format!("{skipped} evento(s) con datos inválidos quedaron fuera")),
        ))
    }

    pub fn to_json(&self) -> String {
        let events: Vec<Value> = self.events.iter().map(event_to_json).collect();
        let mut top = Map::new();
        top.insert("version".into(), Value::from(1));
        top.insert("events".into(), Value::Array(events));
        for (k, v) in &self.extra {
            top.insert(k.clone(), v.clone());
        }
        serde_json::to_string_pretty(&Value::Object(top)).unwrap_or_default()
    }

    /// Read the file at `path`. The second value is a notice for the reader: a corrupt file was
    /// backed up and the calendar starts empty, or some events were dropped.
    pub fn load(path: &Path) -> (Owned, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Owned::default(), None),
            Err(e) => return (Owned::default(), Some(format!("no se pudo leer el calendario: {e}"))),
        };
        match Owned::parse(&text) {
            Ok((owned, note)) => (owned, note),
            Err(e) => {
                let backup = backup_path(path);
                let kept = std::fs::write(&backup, &text).is_ok();
                let where_ = backup.file_name().and_then(|n| n.to_str()).unwrap_or("events.corrupt.json");
                let note = if kept {
                    format!("El calendario tenía un error ({e}); se guardó una copia en {where_}.")
                } else {
                    format!("El calendario tenía un error ({e}) y no se pudo respaldar.")
                };
                (Owned::default(), Some(note))
            }
        }
    }

    /// Write the file atomically: a temporary file in the same directory, then a rename.
    pub fn save_atomic(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }

    /// The occurrences in `from..=to`, each with the index of its event.
    pub fn expand(&self, from: Day, to: Day) -> Vec<(usize, Occurrence)> {
        let mut cal = Ics::default();
        cal.name = "Propios".into();
        cal.events = self.events.iter().map(OwnedEvent::to_event).collect();
        crate::expand(&cal, from, to).into_iter().map(|o| (o.event, o)).collect()
    }

    /// Insert or replace an event by id, keeping the creation stamp of the one it replaces. An
    /// event with no id gets a stable one.
    pub fn upsert(&mut self, mut ev: OwnedEvent) -> String {
        if ev.id.is_empty() {
            ev.id = stable_id(&ev.title, ev.day, ev.start_minute);
        }
        if !valid_color(&ev.color) {
            ev.color = NAMED_COLORS[0].to_string();
        }
        if ev.end_minute.is_some_and(|e| ev.start_minute.is_some_and(|s| e < s)) {
            ev.end_minute = ev.start_minute;
        }
        match self.events.iter_mut().find(|e| e.id == ev.id) {
            Some(slot) => {
                ev.created_ms = slot.created_ms;
                *slot = ev.clone();
            }
            None => self.events.push(ev.clone()),
        }
        self.events.sort_by_key(|a| (a.day, a.start_minute, a.title.clone()));
        ev.id
    }

    /// Take one event out. True when it was there.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.events.len();
        self.events.retain(|e| e.id != id);
        self.events.len() != before
    }
}

/// A copy of a file that could not be read, next to it and never overwritten.
fn backup_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("events");
    path.with_file_name(format!("{name}.corrupt-{stamp}.json"))
}

/// The id of an event, from what makes it that event: its title, its day and its clock. FNV-1a,
/// the same fold the crate uses elsewhere. Two events with the same title on the same day at the
/// same clock are the same event, which is what a repeated save must conclude.
pub fn stable_id(title: &str, day: Day, minute: Option<u16>) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in title.as_bytes().iter().chain(&day.index().to_le_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    if let Some(m) = minute {
        hash ^= u64::from(m);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("ev-{hash:016x}")
}

fn minute_of(value: Option<&Value>) -> Option<u16> {
    let n = value?.as_u64()?;
    u16::try_from(n).ok().map(|m| m.min(24 * 60 - 1))
}

fn event_from(value: Value) -> Option<OwnedEvent> {
    let Value::Object(mut map) = value else { return None };
    let title = map.remove("title")?.as_str()?.to_string();
    let (y, m, d) = {
        let text = map.remove("start")?.as_str()?.to_string();
        let mut parts = text.split('-');
        let y = parts.next()?.parse::<i32>().ok()?;
        let m = parts.next()?.parse::<u32>().ok()?;
        let d = parts.next()?.parse::<u32>().ok()?;
        (y, m, d)
    };
    let day = Day::from_ymd(y, m, d)?;
    let start_minute = minute_of(map.remove("startMinute").as_ref());
    let end_minute = minute_of(map.remove("endMinute").as_ref());
    let color = map.remove("color").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let color = if valid_color(&color) { color } else { NAMED_COLORS[0].to_string() };
    let mut text_of =
        |key: &str| map.remove(key).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let location = text_of("location");
    let description = text_of("description");
    let rrule = map.remove("rrule").and_then(|v| v.as_str().map(str::to_string)).filter(|r| !r.is_empty());
    let note = map.remove("note").and_then(|v| v.as_str().map(str::to_string));
    let created_ms = map.remove("createdMs").and_then(|v| v.as_u64()).unwrap_or(0);
    let updated_ms = map.remove("updatedMs").and_then(|v| v.as_u64()).unwrap_or(0);
    let id = map
        .remove("id")
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| stable_id(&title, day, start_minute));
    // `allDay` is what another reader sees; this crate derives it from the clock.
    map.remove("allDay");
    Some(OwnedEvent {
        id,
        title,
        day,
        start_minute,
        end_minute,
        color,
        location,
        description,
        rrule,
        note,
        created_ms,
        updated_ms,
        extra: map,
    })
}

fn event_to_json(ev: &OwnedEvent) -> Value {
    let (y, m, d) = ev.day.ymd();
    let mut map = Map::new();
    map.insert("id".into(), Value::from(ev.id.clone()));
    map.insert("title".into(), Value::from(ev.title.clone()));
    map.insert("start".into(), Value::from(format!("{y:04}-{m:02}-{d:02}")));
    map.insert("allDay".into(), Value::from(ev.is_all_day()));
    if let Some(start) = ev.start_minute {
        map.insert("startMinute".into(), Value::from(start));
    }
    if let Some(end) = ev.end_minute {
        map.insert("endMinute".into(), Value::from(end));
    }
    map.insert("color".into(), Value::from(ev.color.clone()));
    if !ev.location.is_empty() {
        map.insert("location".into(), Value::from(ev.location.clone()));
    }
    if !ev.description.is_empty() {
        map.insert("description".into(), Value::from(ev.description.clone()));
    }
    if let Some(rule) = &ev.rrule {
        map.insert("rrule".into(), Value::from(rule.clone()));
    }
    if let Some(note) = &ev.note {
        map.insert("note".into(), Value::from(note.clone()));
    }
    map.insert("createdMs".into(), Value::from(ev.created_ms));
    map.insert("updatedMs".into(), Value::from(ev.updated_ms));
    for (k, v) in &ev.extra {
        map.insert(k.clone(), v.clone());
    }
    Value::Object(map)
}

/// The hex a colour id draws with: the eight names have a colour of their own, and a custom
/// `#rrggbb` is kept as written. An unknown id falls back to the first name instead of drawing
/// nothing.
pub fn color_hex(id: &str) -> String {
    let named = match id {
        "azul" => "#3b82f6",
        "verde" => "#2f9e44",
        "ambar" => "#f08c00",
        "rojo" => "#e03131",
        "rosa" => "#e64980",
        "cian" => "#0c8599",
        "gris" => "#868e96",
        _ => "#8a5cf5",
    };
    if id.starts_with('#') && valid_color(id) {
        id.to_string()
    } else {
        named.to_string()
    }
}

/// The name shown next to a colour in the picker.
pub fn color_label(id: &str) -> &'static str {
    match id {
        "azul" => "Azul",
        "verde" => "Verde",
        "ambar" => "Ámbar",
        "rojo" => "Rojo",
        "rosa" => "Rosa",
        "cian" => "Cian",
        "gris" => "Gris",
        _ => "Violeta",
    }
}
