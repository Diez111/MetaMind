//! A video embed shows its own first frame instead of a black box: one `ffmpeg` pass takes a
//! single frame per file, off the drawing thread, and the cache keeps the result with a byte
//! budget.
//!
//! Three rules keep it cheap, which is the whole point of the module:
//! - One frame per (path, mtime): a file that does not change is never decoded again.
//! - At most [`JOBS`] decodes in flight, and never in a compact window (a phone-sized screen is
//!   the case the tap-to-play card was made for).
//! - The cache is bounded in bytes and in items; the least recently seen poster goes first.

use metamind_ui::Image;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

/// Decodes that may run at the same time.
const JOBS: usize = 2;
/// How much of the posters the cache may hold, in bytes of decoded pixels.
const BUDGET: usize = 96 * 1024 * 1024;
/// And how many posters, for tiny files where the byte budget never bites.
const ITEMS: usize = 48;
/// The size a poster is decoded at: it is a card behind an overlay, not a frame of the film.
const FRAME: (u32, u32) = (480, 270);

/// What a decode answers: the file it belongs to, the stamp it was asked with, and the frame.
type Answer = (String, (i64, u64), Option<Image>);

/// What the frame asks for: the file on disk, its vault path and its stamp.
pub type PosterRequest = (std::path::PathBuf, String, (i64, u64));

/// The posters already decoded, and the decodes in flight.
#[derive(Default)]
pub struct Posters {
    cache: HashMap<String, Entry>,
    /// Least recently used last: the front is the next to go when the budget is full.
    order: VecDeque<String>,
    bytes: usize,
    /// Files that could not be decoded: one attempt each, never a loop of failing `ffmpeg`.
    failed: HashSet<String>,
    /// Files with a decode in flight: the frame asks again every time it is drawn.
    in_flight: HashSet<String>,
    pending: Vec<Receiver<Answer>>,
}

struct Entry {
    /// (mtime, size) of the file the poster came from: a different file is a different poster.
    stamp: (i64, u64),
    image: Image,
}

/// Whether a poster is worth decoding at all in the current window: a phone-sized one keeps the
/// card with its play button, which is what weak hardware wants.
pub fn wanted(compact: bool) -> bool {
    !compact && !std::env::var("METAMIND_NO_POSTERS").is_ok_and(|v| v == "1")
}

impl Posters {
    /// The poster of `rel`, if it is already decoded and the file has not changed.
    pub fn image(&self, rel: &str, stamp: (i64, u64)) -> Option<Image> {
        self.cache.get(rel).filter(|e| e.stamp == stamp).map(|e| e.image.clone())
    }

    /// Asks for the poster of `rel`. Does nothing when it is already here, already in flight, or
    /// known to fail.
    pub fn request(&mut self, path: PathBuf, rel: String, stamp: (i64, u64)) {
        if self.cache.get(&rel).is_some_and(|e| e.stamp == stamp)
            || self.failed.contains(&rel)
            || self.in_flight.contains(&rel)
            || self.pending.len() >= JOBS
        {
            return;
        }
        let (tx, rx) = channel();
        self.in_flight.insert(rel.clone());
        let key = rel.clone();
        std::thread::spawn(move || {
            let _ = tx.send((key, stamp, frame(&path)));
        });
        self.pending.push(rx);
    }

    /// Whether a decode is in flight.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Takes a finished decode, if any. Returns whether the cache changed.
    pub fn poll(&mut self) -> bool {
        // Answers come out of the receivers first, so the cache can be written while the list of
        // pending decodes is not borrowed.
        let mut answers = Vec::new();
        let mut done = Vec::new();
        for (i, rx) in self.pending.iter().enumerate() {
            if let Ok((rel, stamp, image)) = rx.try_recv() {
                answers.push((rel, stamp, image));
                done.push(i);
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
                    self.failed.insert(rel);
                }
            }
        }
        changed
    }

    fn insert(&mut self, rel: String, stamp: (i64, u64), image: Image) {
        let bytes = image.0.data().len();
        if let Some(old) = self.cache.remove(&rel) {
            self.bytes = self.bytes.saturating_sub(old.image.0.data().len());
        }
        self.order.retain(|k| *k != rel);
        self.order.push_front(rel.clone());
        self.cache.insert(rel, Entry { stamp, image });
        self.bytes += bytes;
        while self.bytes > BUDGET || self.cache.len() > ITEMS {
            let Some(old) = self.order.pop_back() else { break };
            if let Some(e) = self.cache.remove(&old) {
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

    fn image(w: u32, h: u32) -> Image {
        Image(std::sync::Arc::new(metamind_ui::Pixmap::new(w, h).unwrap()))
    }

    #[test]
    fn the_cache_keeps_its_item_cap_and_drops_the_oldest() {
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
    fn a_failed_file_is_asked_for_once() {
        let mut p = Posters::default();
        p.failed.insert("roto.mp4".to_string());
        p.request(PathBuf::from("/no/existe.mp4"), "roto.mp4".to_string(), (1, 1));
        assert!(p.pending.is_empty(), "a known failure is not retried");
    }

    #[test]
    fn a_phone_sized_window_keeps_the_play_button() {
        assert!(!wanted(true), "compact windows do not decode posters");
        assert!(wanted(false));
    }
}

#[cfg(test)]
mod pixel_tests {
    use super::*;

    #[test]
    fn a_frame_becomes_an_opaque_image_with_its_colors() {
        // Two pixels of pure orange, as ffmpeg writes them.
        let mut raw = vec![255u8, 140, 0, 255, 255, 140, 0, 255];
        let img = crate::video::poster_pixmap(&raw, 2, 1).expect("the frame becomes an image");
        let pm = &img.0;
        let data = pm.data();
        eprintln!("data {:?}", &data[..8]);
        assert_eq!(&data[..4], &[255, 140, 0, 255], "the colour and its alpha survive");
        raw.clear();
        assert!(crate::video::poster_pixmap(&raw, 2, 1).is_none(), "a short buffer is refused");
    }
}
