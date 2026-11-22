//! A video embed shows its own first frame instead of a black box: one `ffmpeg` pass takes a
//! single frame per file, off the drawing thread, and the cache keeps the result with a byte
//! budget.
//!
//! Four rules keep it cheap, which is the whole point of the module:
//! - One frame per (path, mtime): a file that does not change is never decoded again.
//! - At most [`JOBS`] decodes in flight, and never in a compact window (a phone-sized screen is
//!   the case the tap-to-play card was made for).
//! - The cache is bounded in bytes and in items; the poster that has gone the longest without
//!   being drawn is the first to go, whether it arrived long ago or not.
//! - A decode that hangs is dropped: a picture is never worth a loop that stops responding.

use metamind_ui::Image;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

/// Decodes that may run at the same time.
const JOBS: usize = 2;
/// How much of the posters the cache may hold, in bytes of decoded pixels.
const BUDGET: usize = 96 * 1024 * 1024;
/// And how many posters, for tiny files where the byte budget never bites.
const ITEMS: usize = 48;
/// The size a poster is decoded at: the note column's width, not a frame of the film. It is the
/// whole card, so a smaller decode shows as a soft picture at the size the reader sees it.
const FRAME: (u32, u32) = (768, 432);
/// A decode that has not answered in this long is given up: the card stays as it was.
const TIMEOUT: Duration = Duration::from_secs(20);

/// What a decode answers: the stamp it was asked with, and the frame.
type Answer = ((i64, u64), Option<Image>);
/// How a poster is taken from a file. The real one runs `ffmpeg`; tests put their own here, so a
/// test never starts a process for a picture it will throw away.
type Decoder = std::sync::Arc<dyn Fn(&std::path::Path) -> Option<Image> + Send + Sync>;

/// What the frame asks for: the file on disk, its vault path and its stamp.
pub type PosterRequest = (PathBuf, String, (i64, u64));

/// The posters already decoded, and the decodes in flight.
#[derive(Default)]
pub struct Posters {
    cache: HashMap<String, Entry>,
    /// Counts every look at a poster: the oldest count is the coldest one.
    clock: Cell<u64>,
    bytes: usize,
    /// Files that could not be decoded, with the version that failed: one attempt each, never a
    /// loop of failing `ffmpeg`, and a file that is replaced gets its chance.
    failed: HashSet<(String, (i64, u64))>,
    /// Files with a decode in flight: the frame asks again every time it is drawn.
    in_flight: HashSet<String>,
    /// (vault path, the decode's mailbox, when it started): the path is here, not in the answer,
    /// so a decode that times out still knows which file it was.
    pending: Vec<(String, Receiver<Answer>, Instant)>,
    /// The decoder the tests install; the real one is [`frame`].
    decode: Option<Decoder>,
}

struct Entry {
    /// (mtime, size) of the file the poster came from: a different file is a different poster.
    stamp: (i64, u64),
    image: Image,
    /// The `clock` reading of the last frame that drew it.
    used: Cell<u64>,
}

/// Whether a poster is worth decoding at all in the current window: a phone-sized one keeps the
/// card with its play button, which is what weak hardware wants.
pub fn wanted(compact: bool) -> bool {
    !compact && !std::env::var("METAMIND_NO_POSTERS").is_ok_and(|v| v == "1")
}

impl Posters {
    /// A cache that decodes with `f` instead of starting `ffmpeg`: the tests' way in, so a test
    /// that draws a card never starts a process for a picture it will throw away.
    pub fn with_decoder(f: impl Fn(&std::path::Path) -> Option<Image> + Send + Sync + 'static) -> Self {
        Posters { decode: Some(std::sync::Arc::new(f)), ..Default::default() }
    }

    /// The poster of `rel`, if it is already decoded and the file has not changed. Looking at it
    /// is using it: the coldest poster is the one evicted.
    pub fn image(&self, rel: &str, stamp: (i64, u64)) -> Option<Image> {
        let entry = self.cache.get(rel).filter(|e| e.stamp == stamp)?;
        entry.used.set(self.tick());
        Some(entry.image.clone())
    }

    /// Asks for the poster of `rel`. Does nothing when it is already here, already in flight, or
    /// known to fail. At most [`JOBS`] decodes run at once; the frame asks again next time, so a
    /// card that is not on screen never queues a process.
    pub fn request(&mut self, path: PathBuf, rel: String, stamp: (i64, u64)) {
        if self.cache.get(&rel).is_some_and(|e| e.stamp == stamp)
            || self.failed.contains(&(rel.clone(), stamp))
            || self.in_flight.contains(&rel)
            || self.pending.len() >= JOBS
        {
            return;
        }
        let decode = self.decoder();
        let (tx, rx) = channel();
        self.in_flight.insert(rel.clone());
        std::thread::spawn(move || {
            let _ = tx.send((stamp, decode(&path)));
        });
        self.pending.push((rel, rx, Instant::now()));
    }

    /// Whether a decode is in flight.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Takes a finished decode, if any. Returns whether the cache changed.
    pub fn poll(&mut self) -> bool {
        // Answers come out of the receivers first, so the cache can be written while the list of
        // pending decodes is not borrowed.
        let mut answers: Vec<(String, (i64, u64), Option<Image>)> = Vec::new();
        let mut done = Vec::new();
        for (i, (rel, rx, started)) in self.pending.iter().enumerate() {
            match rx.try_recv() {
                Ok((stamp, image)) => {
                    answers.push((rel.clone(), stamp, image));
                    done.push(i);
                }
                // A thread that never answers (a decoder stuck on a broken file) is let go: the
                // card keeps its play button and the file is not tried again.
                Err(_) if started.elapsed() > TIMEOUT => {
                    answers.push((rel.clone(), (0, 0), None));
                    done.push(i);
                }
                Err(_) => {}
            }
        }
        for i in done.into_iter().rev() {
            self.pending.remove(i);
        }
        let changed = !answers.is_empty();
        for (rel, stamp, image) in answers {
            self.in_flight.remove(&rel);
            match image {
                Some(image) => self.insert(rel, stamp, image),
                None => {
                    self.failed.insert((rel, stamp));
                }
            }
        }
        changed
    }

    fn decoder(&self) -> Decoder {
        match &self.decode {
            Some(f) => f.clone(),
            None => std::sync::Arc::new(frame) as Decoder,
        }
    }

    fn tick(&self) -> u64 {
        let now = self.clock.get() + 1;
        self.clock.set(now);
        now
    }

    fn insert(&mut self, rel: String, stamp: (i64, u64), image: Image) {
        let bytes = image.0.data().len();
        if let Some(old) = self.cache.remove(&rel) {
            self.bytes = self.bytes.saturating_sub(old.image.0.data().len());
        }
        let used = Cell::new(self.tick());
        self.cache.insert(rel, Entry { stamp, image, used });
        self.bytes += bytes;
        while self.bytes > BUDGET || self.cache.len() > ITEMS {
            // The coldest poster goes first: the one the frames have gone longest without drawing.
            let Some(cold) = self.cache.iter().min_by_key(|(_, e)| e.used.get()).map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.cache.remove(&cold) {
                self.bytes = self.bytes.saturating_sub(e.image.0.data().len());
            }
        }
    }
}

/// One frame of the file, at [`FRAME`], from the first second: the opening frame of many videos is
/// black, which is exactly what a poster must not be.
fn frame(path: &std::path::Path) -> Option<Image> {
    let ffmpeg = crate::video::find_program("ffmpeg", std::env::var_os("PATH"))?;
    let (w, h) = FRAME;
    let out = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-ss", "1", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.len() != (w * h * 4) as usize {
        return None;
    }
    crate::video::poster_pixmap(&out.stdout, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn image(w: u32, h: u32) -> Image {
        Image(std::sync::Arc::new(metamind_ui::Pixmap::new(w, h).unwrap()))
    }

    /// Polls until the cache answers for `rel` or the wait runs out: the decode itself is a
    /// thread, so the test waits for it instead of racing it.
    fn settle(p: &mut Posters) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            p.poll();
            if !p.busy() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn the_cache_keeps_its_item_cap_and_drops_the_coldest() {
        let mut p = Posters::default();
        for i in 0..(ITEMS + 12) {
            p.insert(format!("v{i}.mp4"), (i as i64, 1), image(8, 8));
        }
        assert_eq!(p.cache.len(), ITEMS, "the cache never grows past its cap");
        assert!(p.cache.contains_key(&format!("v{}.mp4", ITEMS + 11)), "the newest stays");
        assert!(!p.cache.contains_key("v0.mp4"), "the oldest went first");
        // 60 were added, 48 are kept: the twelve oldest went.
        assert!(!p.cache.contains_key("v11.mp4"), "and so did the twelve oldest");
        assert!(p.cache.contains_key("v12.mp4"), "the thirteenth is the oldest kept");
    }

    #[test]
    fn drawing_a_poster_keeps_it_out_of_the_next_eviction() {
        let mut p = Posters::default();
        for i in 0..ITEMS {
            p.insert(format!("v{i}.mp4"), (i as i64, 1), image(8, 8));
        }
        // The first one was inserted first, but a frame drew it just now: v1 is the cold one.
        assert!(p.image("v0.mp4", (0, 1)).is_some());
        p.insert("nuevo.mp4".to_string(), (100, 1), image(8, 8));
        assert!(p.cache.contains_key("v0.mp4"), "the poster that was on screen stays");
        assert!(!p.cache.contains_key("v1.mp4"), "the one nobody drew is the one that goes");
    }

    #[test]
    fn the_byte_budget_evicts_before_the_item_cap() {
        let mut p = Posters::default();
        // 8 MB each: a dozen of them is the 96 MB budget, well under the item cap.
        for i in 0..16 {
            p.insert(format!("v{i}.mp4"), (i as i64, 1), image(1000, 2000));
        }
        assert!(p.bytes <= BUDGET, "the byte budget holds: {}", p.bytes);
        assert!(p.cache.len() < ITEMS, "the budget bit before the count: {}", p.cache.len());
        assert!(p.cache.contains_key("v15.mp4"), "the newest stays");
    }

    #[test]
    fn a_decoded_frame_lands_in_the_cache_with_its_stamp() {
        let mut p = Posters::with_decoder(|_| Some(image(8, 8)));
        p.request(PathBuf::from("/v/clip.mp4"), "clip.mp4".to_string(), (7, 99));
        assert!(settle(&mut p), "the decode answers");
        assert!(p.image("clip.mp4", (7, 99)).is_some(), "the frame is kept for the file it came from");
        assert!(p.image("clip.mp4", (8, 99)).is_none(), "another mtime is another poster");
        assert!(p.in_flight.is_empty(), "nothing is left in flight");
    }

    #[test]
    fn a_failed_file_is_asked_for_once() {
        let mut p = Posters::with_decoder(|_| None);
        p.request(PathBuf::from("/v/roto.mp4"), "roto.mp4".to_string(), (1, 1));
        assert!(settle(&mut p), "the decode answers");
        assert!(p.failed.contains(&("roto.mp4".to_string(), (1, 1))), "the failure is noted");
        p.request(PathBuf::from("/v/roto.mp4"), "roto.mp4".to_string(), (1, 1));
        assert!(p.pending.is_empty(), "a known failure is not retried");
        // The file was replaced: another mtime is another file, and it deserves its attempt.
        p.request(PathBuf::from("/v/roto.mp4"), "roto.mp4".to_string(), (2, 1));
        assert_eq!(p.pending.len(), 1, "a changed file is decoded again");
    }

    #[test]
    fn no_more_than_the_job_limit_decodes_at_once() {
        let live = std::sync::Arc::new(AtomicUsize::new(0));
        let seen = live.clone();
        let mut p = Posters::with_decoder(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            None
        });
        for i in 0..5 {
            p.request(PathBuf::from(format!("/v/v{i}.mp4")), format!("v{i}.mp4"), (1, 1));
        }
        assert_eq!(p.pending.len(), JOBS, "only the job limit was started");
        assert_eq!(p.in_flight.len(), JOBS, "and no more files are marked in flight");
        assert!(settle(&mut p), "the decodes answer");
        assert_eq!(live.load(Ordering::SeqCst), JOBS, "the other three never started");
    }

    #[test]
    fn a_phone_sized_window_keeps_the_play_button() {
        assert!(!wanted(true), "compact windows do not decode posters");
        assert!(wanted(false));
    }
}
