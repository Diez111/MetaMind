//! In-app video playback: a child `ffmpeg` decodes the file and writes raw RGBA frames to a
//! pipe, which a worker thread reads and hands to the view.
//!
//! Nothing is embedded in a window: the pixels are ours, so scrolling, resizing and DPI need no
//! window geometry. The MVP is muted; the `Playback` trait leaves room for a smarter backend.
//!
//! Lifecycle: pausing stops the decoder (`SIGSTOP`), seeking restarts it at a new offset, and
//! closing kills and reaps it, so no child outlives the player.
//!
//! Pacing belongs to the reader thread and to nothing else. ffmpeg's own `-re`/`-readrate` were
//! measured on ffmpeg 9.1 and are not a clock: the same one second clip decoded in 0.8 s, 1.7 s or
//! 3.6 s depending on the flag and the frame rate, and combined with a sleep per frame they ran
//! the film at half speed. So `ffmpeg` decodes as fast as the pipe lets it and the reader sleeps
//! one frame of the file's own rate per frame, on an absolute schedule that does not drift.
//!
//! A source is either a file in the vault or an https URL. The URL is what `yt-dlp` resolves for a
//! YouTube link ([`crate::youtube`]); seeking on it re-opens the stream at the new offset, since
//! `-ss` before `-i` is a fast forward for the http reader too.

use crate::icons;
use metamind_ui::dom::NodeId;
use metamind_ui::{Dom, El, Image, Rect};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vault_core::path;

/// Extensions played inside the application. Anything else still goes to the system player.
pub const VIDEO_EXTS: [&str; 7] = ["mp4", "webm", "mkv", "mov", "m4v", "ogv", "avi"];

/// What the remote reader is asked with. YouTube's CDN answers 403 to a request without one.
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 MetaMind";

/// Frame rate assumed when the container does not say one.
pub const FALLBACK_FPS: f32 = 25.0;

/// Where a decoder reads from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file, normally one inside the vault.
    File(PathBuf),
    /// An `http(s)` URL: what `yt-dlp` resolves a YouTube link to.
    Url(String),
}

impl Source {
    /// A file source.
    pub fn file(path: impl Into<PathBuf>) -> Source {
        Source::File(path.into())
    }

    /// An `http(s)` URL, or `None` when `url` is not one.
    pub fn url(url: &str) -> Option<Source> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        (!rest.is_empty()).then(|| Source::Url(url.to_string()))
    }

    pub fn is_url(&self) -> bool {
        matches!(self, Source::Url(_))
    }

    /// What `ffmpeg` and `ffprobe` are given: a path, or the URL as it is.
    pub fn arg(&self) -> String {
        match self {
            Source::File(p) => p.to_string_lossy().into_owned(),
            Source::Url(u) => u.clone(),
        }
    }

    /// Whether the source can be read at all: a local file has to exist.
    fn usable(&self) -> bool {
        match self {
            Source::File(p) => p.is_file(),
            Source::Url(_) => true,
        }
    }
}

/// Whether `rel` names a file this module can play.
pub fn is_video(rel: &str) -> bool {
    VIDEO_EXTS.iter().any(|e| path::extension(rel).eq_ignore_ascii_case(e))
}

/// What `ffprobe` reports about the first video stream.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// `0.0` when the container does not say.
    pub duration: f64,
}

/// Something a decoder hands back for the view.
pub enum Frame {
    Picture(Image),
    /// The stream ended on its own.
    Ended,
    Failed(String),
}

/// A video decoder the player drives. One implementation today (`FfmpegPipe`); an mpv backend
/// would implement the same surface.
pub trait Playback {
    /// Start decoding `source` at `offset` seconds, scaled to fit `size`, and play.
    fn open(&mut self, source: &Source, size: (u32, u32), offset: f64) -> Result<(), String>;
    fn pause(&mut self);
    fn resume(&mut self);
    /// Restart decoding at `offset`, keeping the paused state.
    fn seek(&mut self, offset: f64) -> Result<(), String>;
    /// Play `speed` times as fast (see [`Clock`]); the reader sleeps for it per frame.
    fn set_speed(&mut self, speed: f32);
    /// The next thing the decoder has for the view, if any.
    fn poll(&mut self) -> Option<Frame>;
    fn is_running(&mut self) -> bool;
    /// Kill and reap the decoder.
    fn close(&mut self);
}

/// A woken event loop, so a frame that arrives between deadlines is shown at once.
type Waker = Arc<dyn Fn() + Send + Sync>;

fn dead_channel() -> Receiver<Frame> {
    let (tx, rx) = sync_channel(1);
    drop(tx);
    rx
}

/// Decoding through `ffmpeg -f rawvideo` on a pipe.
pub struct FfmpegPipe {
    exe: PathBuf,
    probe: PathBuf,
    child: Option<Child>,
    rx: Receiver<Frame>,
    size: (u32, u32),
    /// Frames per second of the source, to pace the reader; `0.0` plays as fast as the pipe fills.
    rate: f32,
    /// Playback speed the reader sleeps for: `2.0` takes half the time per frame.
    speed: f32,
    source: Option<Source>,
    paused: bool,
    waker: Option<Waker>,
}

impl FfmpegPipe {
    /// A decoder driven by the `ffmpeg` and `ffprobe` found on `PATH`.
    pub fn new() -> Result<FfmpegPipe, String> {
        let path = std::env::var_os("PATH");
        let exe = find_program("ffmpeg", path.clone()).ok_or("no está instalado ffmpeg")?;
        let probe = find_program("ffprobe", path).ok_or("no está instalado ffprobe")?;
        Ok(FfmpegPipe::with_tools(exe, probe))
    }

    /// A decoder driven by the given tools.
    pub fn with_tools(exe: PathBuf, probe: PathBuf) -> FfmpegPipe {
        FfmpegPipe {
            exe,
            probe,
            child: None,
            rx: dead_channel(),
            size: (2, 2),
            rate: 0.0,
            speed: 1.0,
            source: None,
            paused: false,
            waker: None,
        }
    }

    /// What `ffprobe` says about `source`, through the probe this decoder was built with.
    pub fn probe(&self, source: &Source) -> Result<MediaInfo, String> {
        probe(&self.probe, source)
    }

    /// Frames per second to play at. Set before `open`.
    pub fn set_rate(&mut self, fps: f32) {
        self.rate = if fps.is_finite() && fps > 0.0 { fps } else { 0.0 };
    }

    /// The playback speed the reader paces to, `1.0` when nothing was set.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Where to call when a frame is ready. Set before `open`.
    pub fn set_waker(&mut self, waker: Option<Waker>) {
        self.waker = waker;
    }

    /// Run `ffmpeg` for the current source and size, replacing any running decoder.
    fn spawn(&mut self, offset: f64) -> Result<(), String> {
        self.close();
        let Some(source) = self.source.clone() else { return Err("no hay nada que reproducir".to_string()) };
        let args = ffmpeg_args(&source, self.size, offset);
        let mut child = Command::new(&self.exe)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo ejecutar ffmpeg: {e}"))?;
        let out = child.stdout.take().ok_or("ffmpeg no dejó una salida que leer")?;
        let (tx, rx) = sync_channel(2);
        let (size, rate, speed, waker) = (self.size, self.rate, self.speed, self.waker.clone());
        std::thread::spawn(move || read_frames(out, size, rate, speed, tx, waker));
        self.rx = rx;
        self.child = Some(child);
        // A seek while paused decodes again, but stops again before the first frame is shown.
        if self.paused {
            signal_child(&self.child, true);
        }
        Ok(())
    }
}

impl Playback for FfmpegPipe {
    fn open(&mut self, source: &Source, size: (u32, u32), offset: f64) -> Result<(), String> {
        if !usable(&self.exe) {
            return Err("no está instalado ffmpeg".to_string());
        }
        if !usable(&self.probe) {
            return Err("no está instalado ffprobe".to_string());
        }
        if !source.usable() {
            return Err(format!("no existe {}", source.arg()));
        }
        self.source = Some(source.clone());
        self.size = (size.0.max(2), size.1.max(2));
        self.paused = false;
        self.spawn(offset)
    }

    fn pause(&mut self) {
        self.paused = true;
        signal_child(&self.child, true);
    }

    fn resume(&mut self) {
        self.paused = false;
        signal_child(&self.child, false);
    }

    fn seek(&mut self, offset: f64) -> Result<(), String> {
        self.spawn(offset)
    }

    fn set_speed(&mut self, speed: f32) {
        self.speed = if speed.is_finite() && speed > 0.0 { speed.clamp(0.1, 4.0) } else { 1.0 };
    }

    fn poll(&mut self) -> Option<Frame> {
        self.rx.try_recv().ok()
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    fn close(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Reaping here is what keeps a closed player from leaving a zombie behind.
            let _ = child.wait();
        }
        self.rx = dead_channel();
    }
}

/// Stop or continue the decoder. On systems without signals the bounded pipe backs up instead:
/// the decoder blocks writing and the frames stop arriving all the same.
#[cfg(unix)]
#[allow(unsafe_code)]
fn signal_child(child: &Option<Child>, stop: bool) {
    let Some(child) = child else { return };
    let sig = if stop { libc::SIGSTOP } else { libc::SIGCONT };
    // Only an integer and a signal: no pointers, and the pid comes from the child we own.
    unsafe { libc::kill(child.id() as libc::pid_t, sig) };
}

#[cfg(not(unix))]
fn signal_child(child: &Option<Child>, stop: bool) {
    let _ = (child, stop);
}

/// Read whole frames off the pipe until it closes. The channel is bounded, so a view that stops
/// draining stalls the decoder instead of growing memory; the reader sets the pace, on an absolute
/// schedule so a slow frame does not shift the rest of the film (see the module docs for why
/// `ffmpeg` is not asked to do it).
fn read_frames(
    mut out: impl Read,
    size: (u32, u32),
    rate: f32,
    speed: f32,
    tx: SyncSender<Frame>,
    waker: Option<Waker>,
) {
    let (w, h) = size;
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    let mut shown = 0u64;
    let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
    // One frame of the file's own rate, at the playback speed asked for.
    let step =
        (rate > 0.0).then(|| Duration::from_secs_f64((1.0 / rate as f64 / speed as f64).clamp(0.001, 1.0)));
    let mut next = Instant::now();
    loop {
        match out.read_exact(&mut buf) {
            Ok(()) => {}
            // A frame boundary: the stream ended.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                let _ = tx.send(Frame::Failed(format!("no se pudo leer el video: {e}")));
                return;
            }
        }
        let Some(image) = rgba_image(&buf, w, h) else {
            let _ = tx.send(Frame::Failed("no se pudo construir el cuadro".to_string()));
            return;
        };
        shown += 1;
        match tx.try_send(Frame::Picture(image)) {
            Ok(()) => {
                if let Some(wake) = &waker {
                    wake();
                }
            }
            // The view is behind: the newest frame is the one that matters.
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return,
        }
        // The pipe backs up, the decoder blocks, and the film runs on the same clock as the time
        // readout: one frame of sleep per frame, counted from the deadline of this frame and not
        // from the moment it was decoded, so the pace never drifts.
        if let Some(step) = step {
            let now = Instant::now();
            match next.checked_add(step) {
                Some(ahead) if ahead > now => {
                    std::thread::sleep(ahead - now);
                    next = ahead;
                }
                // Decoding took longer than a frame: the schedule starts over from here instead
                // of racing through the backlog.
                _ => next = now,
            }
        }
    }
    let end =
        if shown == 0 { Frame::Failed("no se pudo decodificar el video".to_string()) } else { Frame::Ended };
    let _ = tx.send(end);
}

/// A pixmap from raw RGBA rows. `ffmpeg` writes opaque frames; alpha is premultiplied only when
/// the frame actually has some, as the rasterizer expects.
fn rgba_image(src: &[u8], w: u32, h: u32) -> Option<Image> {
    let mut pm = metamind_ui::Pixmap::new(w, h)?;
    let dst = pm.data_mut();
    if dst.len() != src.len() {
        return None;
    }
    if src.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
        dst.copy_from_slice(src);
    } else {
        for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
            let a = s[3] as u16;
            d[0] = (s[0] as u16 * a / 255) as u8;
            d[1] = (s[1] as u16 * a / 255) as u8;
            d[2] = (s[2] as u16 * a / 255) as u8;
            d[3] = s[3];
        }
    }
    Some(Image(Arc::new(pm)))
}

/// Ask `ffprobe` about the source. The reason it gives is what the notice shows. A remote source
/// is probed over http, with the same user agent the decoder is given.
pub fn probe(tool: &Path, source: &Source) -> Result<MediaInfo, String> {
    let mut cmd = Command::new(tool);
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,r_frame_rate,avg_frame_rate:format=duration",
        "-of",
        "json",
    ]);
    if source.is_url() {
        cmd.args(["-user_agent", USER_AGENT]);
    }
    let out = cmd
        .arg(source.arg())
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("no se pudo ejecutar ffprobe: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    probe_json(&text).ok_or_else(|| {
        let reason = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if reason.is_empty() {
            "el archivo no es un video que ffmpeg pueda leer".to_string()
        } else {
            reason
        }
    })
}

/// The fields `probe` asks for, out of ffprobe's JSON.
pub fn probe_json(text: &str) -> Option<MediaInfo> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let stream = v.get("streams")?.get(0)?;
    let width = stream.get("width")?.as_u64()? as u32;
    let height = stream.get("height")?.as_u64()? as u32;
    if width == 0 || height == 0 {
        return None;
    }
    // The nominal rate of the stream is what the reader paces to; a container that only knows the
    // average (a stream copied from a variable-rate source) still gives a usable pace.
    let fps = ["r_frame_rate", "avg_frame_rate"]
        .iter()
        .filter_map(|k| stream.get(*k).and_then(|r| r.as_str()))
        .map(parse_rate)
        .find(|fps| *fps > 0.0)
        .unwrap_or(0.0);
    let duration = match v.get("format").and_then(|f| f.get("duration")) {
        Some(serde_json::Value::String(s)) => s.parse::<f64>().ok().unwrap_or(0.0),
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    let duration = if duration.is_finite() && duration > 0.0 { duration } else { 0.0 };
    Some(MediaInfo { width, height, fps, duration })
}

/// `"30000/1001"` and `"25"` both mean a frame rate.
pub fn parse_rate(rate: &str) -> f32 {
    let (num, den) = rate.split_once('/').unwrap_or((rate, "1"));
    let (n, d) = (num.trim().parse::<f32>(), den.trim().parse::<f32>());
    match (n, d) {
        (Ok(n), Ok(d)) if d > 0.0 && n.is_finite() => n / d,
        _ => 0.0,
    }
}

/// The argv that decodes to a raw RGBA pipe. `-ss` before `-i` seeks by fast forward (for a URL
/// the http reader seeks, so a remote seek is a re-open at the offset), and `-an` keeps it muted
/// and cheap. Nothing here paces: `-re`/`-readrate` were measured unreliable on ffmpeg 9 and the
/// reader thread owns the clock (see the module docs).
pub fn ffmpeg_args(source: &Source, size: (u32, u32), offset: f64) -> Vec<String> {
    let mut args: Vec<String> = vec!["-nostdin".to_string(), "-loglevel".to_string(), "error".to_string()];
    if source.is_url() {
        // A remote stream needs an agent, and a dropped connection is worth one more try.
        args.extend(["-user_agent".to_string(), USER_AGENT.to_string()]);
        args.extend([
            "-reconnect".to_string(),
            "1".to_string(),
            "-reconnect_streamed".to_string(),
            "1".to_string(),
            "-reconnect_delay_max".to_string(),
            "5".to_string(),
        ]);
    }
    args.extend([
        "-ss".to_string(),
        format!("{:.3}", offset.max(0.0)),
        "-i".to_string(),
        source.arg(),
        "-an".to_string(),
        "-sn".to_string(),
        "-vf".to_string(),
        format!("scale={}:{}", size.0, size.1),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "rgba".to_string(),
        "-".to_string(),
    ]);
    args
}

/// The largest size that fits `src` into `cap`, keeping the aspect, never upscaling, with both
/// sides even so the colour planes stay aligned. Sizes are truncated, never rounded up: a frame
/// that name a pixel more than the box it was fitted to is a frame that overflows it.
pub fn fitted(src: (u32, u32), cap: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let (cw, ch) = (cap.0.max(2) as f32, cap.1.max(2) as f32);
    let k = (cw / sw).min(ch / sh).min(1.0);
    let even = |v: f32| ((v.max(2.0) as u32) / 2 * 2).max(2);
    (even(sw * k), even(sh * k))
}

/// `PATH` lookup. A file that is not executable does not count.
pub fn find_program(name: &str, path: Option<std::ffi::OsString>) -> Option<PathBuf> {
    for dir in std::env::split_paths(&path?) {
        #[cfg(windows)]
        let exe = dir.join(format!("{name}.exe"));
        #[cfg(not(windows))]
        let exe = dir.join(name);
        if usable(&exe) {
            return Some(exe);
        }
    }
    None
}

fn usable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `m:ss`, or `h:mm:ss` from an hour on.
pub fn mmss(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 { seconds.round() as u64 } else { 0 };
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Where playback is, from an anchor time plus the time played since.
///
/// `played` counts **film** seconds, not wall seconds: a speed change folds the time played so far
/// at the speed it was played at and never rewrites it afterwards. Anything else makes the
/// readout jump every time the speed control is pressed (`10 s` at `2x` becoming `10 s` at `1x`
/// is 10 film seconds lost).
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    offset: f64,
    since: Option<Instant>,
    /// Film time already played, at the speed each stretch was played at.
    played: Duration,
    /// How fast the film runs per second of wall time; `1.0` is normal speed.
    speed: f32,
}

impl Default for Clock {
    /// At the start, stopped, at normal speed.
    fn default() -> Clock {
        Clock { offset: 0.0, since: None, played: Duration::ZERO, speed: 1.0 }
    }
}

impl Clock {
    /// Playing from `offset` at `now`.
    pub fn at(offset: f64, now: Instant) -> Clock {
        Clock { offset: offset.max(0.0), since: Some(now), played: Duration::ZERO, speed: 1.0 }
    }

    /// Stopped at `offset`.
    pub fn paused_at(offset: f64) -> Clock {
        Clock { offset: offset.max(0.0), since: None, played: Duration::ZERO, speed: 1.0 }
    }

    pub fn position(&self, now: Instant) -> f64 {
        let live =
            self.since.map_or(0.0, |s| now.saturating_duration_since(s).as_secs_f64() * self.speed as f64);
        self.offset + self.played.as_secs_f64() + live
    }

    /// Where a speed change has to be applied, so the time already played keeps the speed it was
    /// played at instead of being re-read at the new one.
    pub fn set_speed(&mut self, speed: f32, now: Instant) {
        let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
        let playing = self.playing();
        self.pause(now);
        self.speed = speed;
        if playing {
            self.resume(now);
        }
    }

    pub fn playing(&self) -> bool {
        self.since.is_some()
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(since) = self.since.take() {
            let live = now.saturating_duration_since(since).as_secs_f64() * self.speed as f64;
            self.played += Duration::from_secs_f64(live.max(0.0));
        }
    }

    pub fn resume(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    /// Move to `to`, counting from `now` when playing.
    pub fn seek(&mut self, to: f64, now: Instant) {
        let playing = self.playing();
        self.offset = to.max(0.0);
        self.played = Duration::ZERO;
        self.since = playing.then_some(now);
    }
}

/// What the player is doing right now.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum State {
    /// No player open, or the stream reached its end.
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    /// The decoder gave up; the message is what the notice shows.
    Failed(String),
}

/// Narrowest the player card gets. Below this the controls squeeze the scrub track to nothing:
/// the picture of a small clip is drawn at its own size, centred in a card wide enough for the
/// whole row (play, readout, track, speed, full screen, close).
const MIN_CARD_W: f32 = 440.0;

/// The playback rates the speed control cycles through, in order.
pub const SPEEDS: [f32; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];

/// The open player: which file, in which note, at what time, and the last frame shown.
#[derive(Default)]
pub struct VideoPlayer {
    backend: Option<Box<dyn Playback>>,
    /// What identifies the open video: its vault-relative path, or the YouTube watch URL. Empty
    /// when no player is open.
    path: String,
    /// Where the decoder reads from, to decode it again at another size, offset or speed.
    source: Option<Source>,
    /// The vault-relative note that shows it: the player lives while that note is open.
    note: String,
    info: MediaInfo,
    size: (u32, u32),
    /// Playback speed: `1.0` normal, up to [`SPEEDS`]'s ends.
    speed: f32,
    clock: Clock,
    current: Option<Image>,
    /// Frames shown since the player opened: what a smoke test counts to see the film move.
    frames: u64,
    state: State,
    /// The progress track being dragged: its left edge, its width, and the time under the pointer.
    scrub: Option<(f32, f32, f64)>,
    waker: Option<Waker>,
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.close();
    }
}

impl VideoPlayer {
    /// Probe `abs` and start decoding it, replacing any player already open.
    pub fn open(&mut self, rel: &str, abs: &Path, note: &str, cap: (u32, u32)) -> Result<(), String> {
        let fail = |e: String| format!("No se puede reproducir {rel}: {e}");
        self.open_source(rel, Source::file(abs), note, cap).map_err(fail)
    }

    /// Play a stream URL, which is how a YouTube link reaches the player. `key` is what the card
    /// and the full-screen overlay look the player up by: the watch URL.
    pub fn open_url(&mut self, key: &str, url: &str, note: &str, cap: (u32, u32)) -> Result<(), String> {
        let Some(source) = Source::url(url) else {
            return Err(format!("no es una dirección de video: {url}"));
        };
        let fail = |e: String| format!("No se puede reproducir el video: {e}");
        self.open_source(key, source, note, cap).map_err(fail)
    }

    /// Probe and start decoding `source`, replacing any player already open.
    fn open_source(&mut self, key: &str, source: Source, note: &str, cap: (u32, u32)) -> Result<(), String> {
        self.close();
        let mut pipe = FfmpegPipe::new()?;
        let info = pipe.probe(&source)?;
        let rate = if info.fps > 0.0 { info.fps } else { FALLBACK_FPS };
        let size = fitted((info.width, info.height), cap);
        self.state = State::Loading;
        pipe.set_waker(self.waker.clone());
        pipe.set_rate(rate);
        pipe.set_speed(self.speed());
        if let Err(e) = pipe.open(&source, size, 0.0) {
            self.state = State::Idle;
            return Err(e);
        }
        self.backend = Some(Box::new(pipe));
        self.path = key.to_string();
        self.source = Some(source);
        self.note = note.to_string();
        self.info = info;
        self.size = size;
        self.clock = self.clock_from(0.0, Instant::now(), true);
        self.current = None;
        // Until the first picture is decoded the widget shows its loading face.
        self.state = State::Loading;
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        !self.path.is_empty()
    }

    /// The vault-relative media path of the open player.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The vault-relative note whose card opened the player.
    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The size the file says it has. The widget fits the picture by this number and not by the
    /// frame it decodes to: the decode is a quality choice, the shape of the film is not.
    pub fn source_size(&self) -> (u32, u32) {
        if self.info.width > 0 && self.info.height > 0 {
            (self.info.width, self.info.height)
        } else {
            self.size
        }
    }

    pub fn duration(&self) -> f64 {
        self.info.duration
    }

    /// The playback rate, `1.0` at normal speed.
    pub fn speed(&self) -> f32 {
        if self.speed > 0.0 {
            self.speed
        } else {
            1.0
        }
    }

    /// What the speed control shows: `1x`, `1.25x`, `0.75x`.
    pub fn speed_label(&self) -> String {
        let s = self.speed();
        let text = if (s.fract() * 100.0).round() == 0.0 {
            format!("{s:.0}")
        } else {
            format!("{s:.2}").trim_end_matches('0').to_string()
        };
        format!("{text}x")
    }

    /// Step to the next of [`SPEEDS`], decoding again at the same time so the picture keeps up.
    pub fn cycle_speed(&mut self) {
        let i = SPEEDS.iter().position(|s| (s - self.speed()).abs() < 0.01).unwrap_or(1);
        self.set_speed(SPEEDS[(i + 1) % SPEEDS.len()]);
    }

    /// Play `speed` times as fast, from where playback is now. The time already played keeps the
    /// speed it was played at: the readout does not jump, and the decoder is moved to exactly the
    /// position the clock names, once.
    pub fn set_speed(&mut self, speed: f32) {
        let now = Instant::now();
        let t = self.position_at(now);
        let speed = if speed.is_finite() && speed > 0.0 { speed.clamp(0.1, 4.0) } else { 1.0 };
        self.speed = speed;
        self.clock.set_speed(speed, now);
        if let Some(b) = self.backend.as_mut() {
            b.set_speed(speed);
        }
        if self.is_open() {
            self.restart_at(t, now);
        }
    }

    /// Decode again at a frame size that fits `cap`, keeping the time and the paused state. The
    /// full-screen view asks for the space the overlay has; the note asks for the card's.
    pub fn set_cap(&mut self, cap: (u32, u32)) {
        let size = fitted((self.info.width, self.info.height), cap);
        if size == self.size || size.0 < 2 || size.1 < 2 {
            return;
        }
        let t = self.position();
        let paused = self.state == State::Paused;
        self.size = size;
        let Some(source) = self.source.clone() else { return };
        let Some(backend) = self.backend.as_mut() else { return };
        if let Err(e) = backend.open(&source, size, t) {
            self.state = State::Failed(e);
            return;
        }
        let now = Instant::now();
        // A paused player keeps the picture it had: the decoder is stopped again before it can
        // send the first frame at the new size, so clearing it would leave an empty box.
        if !paused {
            self.current = None;
        }
        self.clock = self.clock_from(t, now, !paused);
        if paused {
            if let Some(backend) = self.backend.as_mut() {
                backend.pause();
            }
            self.state = State::Paused;
        }
    }

    /// Whether a decoder process is alive right now.
    pub fn is_running(&mut self) -> bool {
        self.backend.as_mut().is_some_and(|b| b.is_running())
    }

    pub fn position(&self) -> f64 {
        self.position_at(Instant::now())
    }

    /// The clock's position at `now`, clamped to the file: the readout and the progress bar are
    /// both painted from this one value, so they can never disagree.
    fn position_at(&self, now: Instant) -> f64 {
        let p = self.clock.position(now);
        if self.info.duration > 0.0 {
            p.min(self.info.duration)
        } else {
            p
        }
    }

    /// The last frame to draw.
    pub fn current(&self) -> Option<&Image> {
        self.current.as_ref()
    }

    /// How many frames have been shown since the player opened.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Where to wake the interface when a frame is ready. Set before opening a player.
    pub fn set_waker(&mut self, waker: Box<dyn Fn() + Send + Sync>) {
        self.waker = Some(Arc::from(waker));
    }

    /// Take whatever the decoder produced since the last frame: the newest picture wins.
    pub fn tick(&mut self, now: Instant) {
        let Some(backend) = self.backend.as_mut() else { return };
        let mut newest = None;
        let mut end = None;
        while let Some(frame) = backend.poll() {
            match frame {
                Frame::Picture(image) => newest = Some(image),
                Frame::Ended => {
                    end = Some(Ok(()));
                    break;
                }
                Frame::Failed(e) => {
                    end = Some(Err(e));
                    break;
                }
            }
        }
        if let Some(image) = newest {
            self.current = Some(image);
            self.frames += 1;
            if self.state == State::Loading {
                self.state = State::Playing;
            }
        }
        match end {
            // The last frame stays on screen; play starts again from the beginning.
            Some(Ok(())) => {
                let end =
                    if self.info.duration > 0.0 { self.info.duration } else { self.clock.position(now) };
                self.clock = self.clock_from(end, now, false);
                self.state = State::Idle;
            }
            Some(Err(e)) => self.state = State::Failed(e),
            None => {}
        }
    }

    /// The player's clock, at `offset`, running `speed` times as fast.
    fn clock_from(&self, offset: f64, now: Instant, playing: bool) -> Clock {
        let mut c = if playing { Clock::at(offset, now) } else { Clock::paused_at(offset) };
        c.set_speed(self.speed(), now);
        c
    }

    /// Pause a running player, resume a paused one, and restart one that reached the end.
    pub fn toggle_pause(&mut self) {
        let now = Instant::now();
        match self.state {
            State::Playing => {
                if let Some(b) = self.backend.as_mut() {
                    b.pause();
                }
                self.clock.pause(now);
                self.state = State::Paused;
            }
            State::Paused => {
                if let Some(b) = self.backend.as_mut() {
                    b.resume();
                }
                self.clock.resume(now);
                self.state = State::Playing;
            }
            State::Idle => self.seek_to(0.0),
            State::Loading | State::Failed(_) => {}
        }
    }

    pub fn seek_by(&mut self, delta: f64) {
        self.seek_to(self.position() + delta);
    }

    /// Decode again from `t`, clamped to the file. Unbounded when its length is unknown.
    pub fn seek_to(&mut self, t: f64) {
        let now = Instant::now();
        let t = self.clamped(t);
        self.restart_at(t, now);
    }

    /// Move the decoder to `t` and put the clock there, keeping the paused state. `now` is the
    /// instant the move belongs to, so a caller that already read the clock does not read it twice
    /// (which is what makes a seek duplicate or restart the time).
    fn restart_at(&mut self, t: f64, now: Instant) {
        let t = self.clamped(t);
        let paused = self.state == State::Paused;
        let clock = self.clock_from(t, now, !paused);
        let Some(backend) = self.backend.as_mut() else { return };
        if let Err(e) = backend.seek(t) {
            self.state = State::Failed(e);
            return;
        }
        if paused {
            backend.pause();
        }
        self.clock = clock;
        self.state = if paused { State::Paused } else { State::Playing };
    }

    /// `t` brought inside the file; unbounded when its length is not known.
    fn clamped(&self, t: f64) -> f64 {
        let t = if t.is_finite() { t } else { 0.0 };
        if self.info.duration > 0.0 {
            t.clamp(0.0, self.info.duration)
        } else {
            t.max(0.0)
        }
    }

    /// Kill the decoder and go back to the card.
    pub fn close(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            backend.close();
        }
        self.path.clear();
        self.source = None;
        self.note.clear();
        self.info = MediaInfo::default();
        self.size = (0, 0);
        self.clock = Clock::default();
        self.current = None;
        self.frames = 0;
        self.state = State::Idle;
        self.scrub = None;
    }

    /// Run a `video:` command. Returns whether it was one. Opening a player needs the vault, so
    /// `video:open:` is handled by `App`.
    pub fn run_action(&mut self, command: &str) -> bool {
        if !self.is_open() {
            return false;
        }
        match command {
            "video:toggle" => self.toggle_pause(),
            "video:speed" => self.cycle_speed(),
            "video:close" => self.close(),
            _ => {
                let Some((_, delta)) =
                    command.strip_prefix("video:seek:").and_then(|rest| rest.rsplit_once(':'))
                else {
                    return false;
                };
                let Ok(delta) = delta.parse::<f64>() else { return false };
                self.seek_by(delta);
            }
        }
        true
    }

    /// Press on the progress track.
    pub fn scrub_press(&mut self, track: Rect, x: f32) {
        if !self.is_open() {
            return;
        }
        let t = self.time_at(track.x, track.w, x);
        self.scrub = Some((track.x, track.w, t));
    }

    /// Pointer moved with the button held. Returns whether a scrub is being dragged.
    pub fn scrub_drag(&mut self, x: f32) -> bool {
        let Some((x0, w, _)) = self.scrub else { return false };
        let t = self.time_at(x0, w, x);
        self.scrub = Some((x0, w, t));
        true
    }

    /// Button released. Returns whether it ended a scrub (the decode then restarts there).
    pub fn scrub_release(&mut self) -> bool {
        let Some((_, _, t)) = self.scrub.take() else { return false };
        self.seek_to(t);
        true
    }

    /// The time under the pointer while the track is dragged.
    pub fn scrub_preview(&self) -> Option<f64> {
        self.scrub.map(|(.., t)| t)
    }

    fn time_at(&self, x0: f32, w: f32, x: f32) -> f64 {
        let k = ((x - x0) / w.max(1.0)).clamp(0.0, 1.0) as f64;
        k * self.info.duration
    }
}

/// The height the control row of the player takes: the 26 px of `.video-player-btn` on top of the
/// row's 8 px of padding and its 1 px rule, which is what `.video-player-controls` asks for.
pub const CONTROLS_H: f32 = 43.0;

/// The 1 px `.video-player` draws on each side (base.css). `width` in this engine is the border
/// box, so the card's width has to include it or the picture hangs a pixel past its own edge.
pub const CARD_BORDER: f32 = 1.0;

/// The air the over-window player keeps between its card and the edge of the workspace.
pub const OVERLAY_MARGIN: f32 = 20.0;

/// The room the player card asks for in a note: the reading column (644 px, `--file-line-width`
/// less the 28 px of padding each side) and a height a tall clip cannot pass, so a portrait video
/// is letterboxed in the note instead of pushing the paragraph under it off screen.
pub const EMBED_SPACE: (f32, f32) = (640.0, 480.0);

/// The box the player card may use over the window, in logical pixels: the workspace less the air
/// the card keeps around itself.
pub fn overlay_space(area: (f32, f32)) -> (f32, f32) {
    let air = 2.0 * OVERLAY_MARGIN;
    ((area.0 - air).max(160.0), (area.1 - air).max(160.0))
}

/// The room the picture itself has inside the card's box: the border and the control row are not
/// the picture's, and a picture measured against the whole card comes out a row too tall.
fn picture_space(space: (f32, f32)) -> (f32, f32) {
    ((space.0 - 2.0 * CARD_BORDER).max(1.0), (space.1 - CONTROLS_H - 2.0 * CARD_BORDER).max(1.0))
}

/// The frame size the over-window player can decode at, in device pixels: the same room the
/// picture is drawn in, so the decoder never hands back a frame wider than the row under it.
pub fn overlay_cap(area: (f32, f32), scale: f32) -> (u32, u32) {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (w, h) = picture_space(overlay_space(area));
    ((w * scale) as u32, (h * scale) as u32)
}

/// The size, in logical pixels, the picture is drawn at: the largest box with the file's own
/// aspect that fits `space`, never larger than the file's pixels at this scale, and never
/// cropped. Whatever the space has left over is letterbox: the stage fills it with black and the
/// picture is centred in it.
///
/// This is the fix for the reported bug. The frame used to be sized from the decoded pixels and
/// then clamped by the card's `max-width`, so a landscape slide in a portrait box lost its edges —
/// the title, the page counter and the arrow of the axis were all outside the box. Fitting by
/// `min(space / source)` on both sides keeps every edge of the picture inside the box at any size.
pub fn picture_size(src: (u32, u32), space: (f32, f32), scale: f32) -> (f32, f32) {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (sw, sh) = (src.0.max(1) as f32 / scale, src.1.max(1) as f32 / scale);
    let (bw, bh) = (space.0.max(1.0), space.1.max(1.0));
    let k = (bw / sw).min(bh / sh).min(1.0);
    // Truncated, never rounded up: a picture that names a pixel it does not have is a pixel the
    // card would have to clip.
    (((sw * k) as u32) as f32, ((sh * k) as u32) as f32)
}

/// The width of the player card for a picture `w` logical pixels wide: the border it draws around
/// the picture, and never narrower than the control row needs. The controls are exactly as wide as
/// the film, which is what the second reported bug was about.
fn card_width(w: f32) -> f32 {
    (w + 2.0 * CARD_BORDER).max(MIN_CARD_W)
}

/// The player widget that replaces a video's card: the frame, the controls and the track. The
/// frame plays and pauses on a click, as a video anywhere else does. `space` is the room the card
/// has, in logical pixels and control row included: the note passes [`EMBED_SPACE`] and the
/// overlay the box the workspace leaves it, so the picture is fitted to where it is really drawn.
pub fn widget(app: &crate::App, dom: &mut Dom, view: NodeId, space: (f32, f32)) -> NodeId {
    let v = &app.video;
    let scale = if app.scale > 0.0 { app.scale } else { 1.0 };
    let loading = *v.state() == State::Loading;
    // The picture keeps the file's own shape, fitted into the room the caller gave it: the card is
    // as wide as that picture, so the control row under it measures exactly as wide as the film.
    let (fw, fh) = picture_size(v.source_size(), picture_space(space), scale);
    let wide = card_width(fw).min(space.0);
    let card = dom.add(
        view,
        El::div()
            .class("video-player")
            .class_if(loading, "is-loading")
            .class_if(app.video_fullscreen, "is-fullscreen")
            .style(&format!("width: {wide:.0}px")),
    );
    let size = format!("width: {fw:.0}px; height: {fh:.0}px");
    // The picture is centred in the card: a narrow clip on a card as wide as the controls needs
    // it, and a wide one loses nothing.
    let stage = dom.add(card, El::div().class("video-player-stage"));
    if let Some(image) = v.current() {
        dom.add(
            stage,
            El::div().class("video-player-frame").style(&size).image(image.clone()).action("video:toggle"),
        );
    } else {
        let frame = dom.add(stage, El::div().class("video-player-frame").style(&size).action("video:toggle"));
        let icon = dom.add(frame, El::div().class("media-card-icon"));
        dom.add(icon, icons::el("play"));
        dom.add(frame, El::div().class("video-player-loading").text("Cargando…"));
    }
    let controls = dom.add(card, El::div().class("video-player-controls"));
    let playing = *v.state() == State::Playing;
    let label = if playing { "Pausa" } else { "Play" };
    let button = dom.add(
        controls,
        El::div().class("clickable-icon video-player-btn").attr("aria-label", label).action("video:toggle"),
    );
    dom.add(button, icons::el(if playing { "pause" } else { "play" }));
    let shown = v.scrub_preview().unwrap_or_else(|| v.position());
    let duration = v.duration();
    // The readout and the bar are painted from this one number: they cannot disagree.
    let t = if duration > 0.0 { (shown / duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
    dom.add(
        controls,
        El::div().class("video-player-time").text(format!("{} / {}", mmss(shown), mmss(duration))),
    );
    let track = dom.add(
        controls,
        El::div()
            .class("video-player-track")
            .attr("aria-label", "Posición")
            .attr("role", "slider")
            .attr("aria-valuenow", &format!("{:.0}", (t * 100.0)))
            .action("video:scrub"),
    );
    // Fill, marker and rest share the track in proportion to the time played: the marker is the
    // zero-width join, so the bar says the same thing as the readout.
    dom.add(track, El::div().class("video-player-fill").style(&format!("flex-grow: {t:.4}")));
    dom.add(track, El::div().class("video-player-thumb"));
    dom.add(track, El::div().class("video-player-rest").style(&format!("flex-grow: {:.4}", 1.0 - t)));
    // Speed: one button that says what it is doing and steps through the rates.
    dom.add(
        controls,
        El::div()
            .class("video-player-btn video-player-pill video-player-speed")
            .attr("aria-label", "Velocidad de reproducción")
            .text(v.speed_label())
            .action("video:speed"),
    );
    // Fullscreen: opens the video in a modal over the window, like the mermaid zoom. The glyph
    // says what the press will do, so the button reads as a toggle instead of a dead box.
    let full = app.video_fullscreen;
    let fullscreen = dom.add(
        controls,
        El::div()
            .class("clickable-icon video-player-btn")
            .class_if(full, "is-active")
            .attr("aria-label", if full { "Salir de pantalla completa" } else { "Pantalla completa" })
            .action("video:fullscreen"),
    );
    dom.add(fullscreen, icons::el(if full { "minimize" } else { "maximize" }));
    let close = dom.add(
        controls,
        El::div()
            .class("clickable-icon video-player-btn")
            .attr("aria-label", "Cerrar el reproductor")
            .action("video:close"),
    );
    dom.add(close, icons::el("x"));
    card
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFPROBE: &str = r#"{
    "programs": [],
    "streams": [
        {
            "r_frame_rate": "30000/1001",
            "width": 160,
            "height": 120
        }
    ],
    "format": {
        "filename": "clip.mp4",
        "duration": "1.000000"
    }
}"#;

    #[test]
    fn fitted_keeps_the_aspect_and_never_upscales() {
        assert_eq!(fitted((160, 120), (640, 360)), (160, 120));
        assert_eq!(fitted((1920, 1080), (640, 360)), (640, 360));
        // A tall clip is bounded by the cap's height.
        assert_eq!(fitted((1080, 1920), (640, 360)), (202, 360));
        // Both sides stay even and at least 2, whatever the rounding.
        for src in [(3, 3), (1919, 1079), (1, 1000), (4000, 4)] {
            let (w, h) = fitted(src, (640, 360));
            assert_eq!((w % 2, h % 2), (0, 0), "{src:?} -> {w}x{h}");
            assert!(w >= 2 && h >= 2);
            assert!(w <= 640 && h <= 360);
        }
        assert_eq!(fitted((2, 2), (1, 1)), (2, 2));
    }

    #[test]
    fn rates_are_parsed_from_fractions() {
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("25"), 25.0);
        assert_eq!(parse_rate("0/0"), 0.0);
        assert_eq!(parse_rate("nope"), 0.0);
    }

    #[test]
    fn probe_json_reads_the_first_stream() {
        let info = probe_json(FFPROBE).expect("the fixture is the shape ffprobe prints");
        assert_eq!((info.width, info.height), (160, 120));
        assert!((info.fps - 29.97).abs() < 0.01);
        assert_eq!(info.duration, 1.0);
        // No video stream at all, and a duration the container does not know.
        assert!(probe_json(r#"{"streams":[],"format":{"duration":"1.0"}}"#).is_none());
        let info = probe_json(r#"{"streams":[{"width":2,"height":2}]}"#).expect("width and height");
        assert_eq!((info.duration, info.fps), (0.0, 0.0));
    }

    #[test]
    fn ffmpeg_argv_seeks_before_the_input_and_scales_to_the_asked_size() {
        let args = ffmpeg_args(&Source::file("/v/clip.mp4"), (320, 180), 12.5);
        assert_eq!(
            args,
            [
                "-nostdin",
                "-loglevel",
                "error",
                "-ss",
                "12.500",
                "-i",
                "/v/clip.mp4",
                "-an",
                "-sn",
                "-vf",
                "scale=320:180",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-",
            ]
            .map(String::from)
        );
        // A negative offset never reaches ffmpeg.
        assert_eq!(ffmpeg_args(&Source::file("a.mp4"), (2, 2), -3.0)[4], "0.000");
        // ffmpeg is not asked to pace the film: it was measured bursting or crawling depending on
        // the build, the rate and the offset, and the reader sleeps one frame per frame anyway.
        for args in [
            ffmpeg_args(&Source::file("a.mp4"), (2, 2), 0.0),
            ffmpeg_args(&Source::Url("https://example.com/v".to_string()), (2, 2), 0.0),
        ] {
            assert!(!args.iter().any(|a| a == "-re" || a == "-readrate"), "{args:?}");
        }
    }

    /// A URL is played the same way a file is, with the agent YouTube needs and the offset that
    /// makes a remote seek a re-open.
    #[test]
    fn a_remote_source_carries_its_agent_and_its_offset() {
        let url = Source::Url("https://rr1.googlevideo.com/videoplayback?x=1".to_string());
        let args = ffmpeg_args(&url, (160, 90), 30.0);
        let at = |needle: &str| args.iter().position(|a| a == needle).unwrap_or_else(|| panic!("{args:?}"));
        assert_eq!(args[at("-user_agent") + 1], USER_AGENT);
        assert_eq!(args[at("-ss") + 1], "30.000");
        assert_eq!(args[at("-i") + 1], "https://rr1.googlevideo.com/videoplayback?x=1");
        assert_eq!(args[at("-vf") + 1], "scale=160:90");
        // A file is not given the http options: they are options of the http reader.
        let local = ffmpeg_args(&Source::file("a.mp4"), (2, 2), 0.0);
        assert!(!local.iter().any(|a| a == "-user_agent" || a == "-reconnect"), "{local:?}");
        // Only `http(s)` is a URL source.
        assert!(Source::url("https://youtu.be/x").is_some());
        assert!(Source::url("http://x/v").is_some());
        assert!(Source::url("file:///tmp/x.mp4").is_none());
        assert!(Source::url("https://").is_none());
        assert!(Source::url("/tmp/x.mp4").is_none());
    }

    #[test]
    fn the_clock_counts_only_while_playing() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut c = Clock::at(4.0, t0);
        assert!((c.position(at(1000)) - 5.0).abs() < 1e-6);
        c.pause(at(1000));
        assert!((c.position(at(9000)) - 5.0).abs() < 1e-6);
        c.resume(at(9000));
        assert!((c.position(at(9500)) - 5.5).abs() < 1e-6);
        c.seek(0.0, at(9500));
        assert!(c.position(at(9500)) < 1e-6);
        assert!((c.position(at(10_000)) - 0.5).abs() < 1e-6);
        // Seeking while paused stays put.
        c.pause(at(10_000));
        c.seek(2.0, at(10_000));
        assert!(!c.playing());
        assert!((c.position(at(11_000)) - 2.0).abs() < 1e-6);
        assert!((Clock::paused_at(3.0).position(at(0)) - 3.0).abs() < 1e-6);
    }

    /// The gate on bug (a): after a second of wall time at rate R the clock reads R seconds of
    /// film, within 10%.
    #[test]
    fn the_clock_runs_at_the_rate_it_is_given() {
        let t0 = Instant::now();
        for rate in [0.75_f32, 1.0, 1.25, 1.5, 2.0] {
            let mut c = Clock::at(0.0, t0);
            c.set_speed(rate, t0);
            let after = c.position(t0 + Duration::from_secs(2));
            assert!((after - 2.0 * rate as f64).abs() < 0.1 * (2.0 * rate as f64), "{rate}x read {after}");
        }
    }

    /// The other half of bug (a): a speed change must not rewrite the time already played. Ten
    /// seconds at `0.5x` are five film seconds whatever the speed becomes afterwards; the clock
    /// used to re-read them at the new speed, and the bar jumped under the readout.
    #[test]
    fn a_speed_change_does_not_rewrite_the_time_already_played() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut c = Clock::at(0.0, t0);
        c.set_speed(0.5, t0);
        let played = c.position(at(10_000));
        assert!((played - 5.0).abs() < 1e-6, "{played}");
        for rate in [1.0_f32, 2.0, 1.25, 0.75] {
            let before = c.position(at(10_000));
            c.set_speed(rate, at(10_000));
            let after = c.position(at(10_000));
            assert!((after - before).abs() < 1e-6, "{rate}x moved {before} to {after}");
            // And from there on it runs at the new speed.
            let later = c.position(at(11_000));
            assert!((later - (before + rate as f64)).abs() < 1e-6, "{rate}x read {later}");
        }
        // A paused clock keeps its position across a speed change too.
        c.pause(at(20_000));
        let paused = c.position(at(20_000));
        c.set_speed(1.5, at(20_000));
        assert!((c.position(at(30_000)) - paused).abs() < 1e-6);
    }

    /// Pacing, in the reader's terms: the reader shows one frame every `1 / fps` of wall time, and
    /// the clock runs at the playback rate. After N frames of a 30 fps file the clock has to read
    /// N/30 * rate seconds of film, within 10%, whatever the rate.
    #[test]
    fn ticking_at_a_rate_reads_the_film_at_that_rate() {
        let fps = 30.0_f64;
        let step = Duration::from_secs_f64(1.0 / fps);
        for rate in [0.75_f32, 1.0, 1.25, 1.5, 2.0] {
            let t0 = Instant::now();
            let mut clock = Clock::at(0.0, t0);
            clock.set_speed(rate, t0);
            let mut now = t0;
            let mut frames = 0;
            while now < t0 + Duration::from_secs(1) {
                frames += 1;
                now += step;
            }
            let wall = now.saturating_duration_since(t0).as_secs_f64();
            let read = clock.position(now);
            let want = wall * rate as f64;
            assert!(
                (read - want).abs() <= 0.1 * want,
                "{rate}x: {frames} frames over {wall:.3}s read {read:.3}s of film, wanted {want:.3}s"
            );
            assert!(frames >= 30, "a second of a 30 fps file is {frames} frames");
        }
    }

    /// `overlay_cap` leaves out everything the overlay draws around the picture: the air the card
    /// keeps around itself, its border and the control row. The same arithmetic is what keeps the
    /// controls inside the box and the picture inside its card.
    #[test]
    fn the_overlay_cap_leaves_the_controls_and_the_margin_out() {
        let (w, h) = overlay_cap((1100.0, 700.0), 1.0);
        let air = 2.0 * OVERLAY_MARGIN;
        assert_eq!(w as f32, 1100.0 - air - 2.0 * CARD_BORDER);
        assert_eq!(h as f32, 700.0 - air - 2.0 * CARD_BORDER - CONTROLS_H);
        // The whole card, controls included, fits the box the workspace is, with its air around it.
        let card_h = h as f32 + CONTROLS_H + 2.0 * CARD_BORDER;
        assert_eq!(card_h, 700.0 - air, "the card is {card_h}px tall");
        // At 2x the device pixels are what grows, never the logical box.
        let (dw, dh) = overlay_cap((1100.0, 700.0), 2.0);
        assert_eq!((dw, dh), (w * 2, h * 2));
        // A window too small for the arithmetic still asks for something playable.
        let (sw, sh) = overlay_cap((0.0, 0.0), 1.0);
        assert!(sw >= 120 && sh >= 90, "{sw}x{sh}");
    }

    /// The bug the user photographed: a slide is 16:9 and the overlay box was portrait, so the
    /// picture was drawn at the box's shape and its edges were cut — the title, the page counter
    /// and the arrow of the axis were all off screen. Fitting by the file's aspect keeps the whole
    /// picture in any box, and leaves the difference as letterbox.
    #[test]
    fn the_picture_keeps_its_aspect_in_any_box() {
        // 2:1 into a square box: the width is the limit, the height comes from the aspect.
        let (w, h) = picture_size((1280, 640), (800.0, 800.0), 1.0);
        assert_eq!((w, h), (800.0, 400.0));
        assert!((w / h - 2.0).abs() < 1e-6);
        // 16:9 into a portrait box (what the overlay used to be measured as): still 16:9, and the
        // whole picture fits the box instead of filling it and losing the sides.
        let (w, h) = picture_size((1920, 1080), (639.0, 856.0), 1.0);
        assert!((w / h - 16.0 / 9.0).abs() < 0.01, "{w}x{h}");
        assert!(w <= 639.0 && h <= 856.0, "{w}x{h} does not fit the box");
        assert_eq!(h, 359.0, "the height is what runs out in a portrait box");
        // A tall clip in a wide box is bounded by the height, never stretched to the width.
        let (w, h) = picture_size((1080, 1920), (1200.0, 600.0), 1.0);
        assert_eq!(h, 600.0);
        assert!((w / h - 9.0 / 16.0).abs() < 0.01, "{w}x{h}");
        // Never upscaled past the file's own pixels.
        assert_eq!(picture_size((320, 180), (1200.0, 700.0), 1.0), (320.0, 180.0));
        // A HiDPI window scales in device pixels, not in the logical box.
        assert_eq!(picture_size((1920, 1080), (960.0, 540.0), 2.0), (960.0, 540.0));
        // Whatever the box and the source, the drawn picture stays inside the box and keeps the
        // shape, to within the pixel the truncation costs.
        for src in [(160, 120), (1920, 1080), (1080, 1920), (640, 360), (1440, 1080)] {
            for space in [(640.0, 480.0), (639.0, 856.0), (1200.0, 300.0), (320.0, 240.0)] {
                let (w, h) = picture_size(src, space, 1.0);
                assert!(w <= space.0 && h <= space.1, "{src:?} in {space:?} is {w}x{h}");
                let want = src.0 as f32 / src.1 as f32;
                assert!((w - want * h).abs() <= 1.0, "{src:?} in {space:?} is {w}x{h}");
            }
        }
        // A box too small to hold the shape still keeps the whole picture in it.
        let (w, h) = picture_size((1920, 1080), (2.0, 2.0), 1.0);
        assert!(w <= 2.0 && h <= 2.0, "{w}x{h}");
    }

    /// The card is as wide as its picture, plus the border it draws: the control row under a film
    /// measures exactly as wide as the film, which is the alignment bug of the capture.
    #[test]
    fn the_card_measures_the_picture_plus_its_border() {
        assert_eq!(card_width(640.0), 642.0);
        assert_eq!(card_width(1200.0), 1202.0);
        // A picture narrower than the controls still gets a card the row can live in.
        assert_eq!(card_width(120.0), MIN_CARD_W);
        // What the note asks for is a card inside the reading column.
        let (w, _) = picture_size((1920, 1080), picture_space(EMBED_SPACE), 1.0);
        assert!(card_width(w) <= EMBED_SPACE.0, "the note card is {} px wide", card_width(w));
    }

    #[test]
    fn times_read_as_a_player_shows_them() {
        assert_eq!(mmss(0.0), "0:00");
        assert_eq!(mmss(12.4), "0:12");
        assert_eq!(mmss(225.0), "3:45");
        assert_eq!(mmss(3600.0), "1:00:00");
        assert_eq!(mmss(3725.0), "1:02:05");
        assert_eq!(mmss(-4.0), "0:00");
        assert_eq!(mmss(f64::NAN), "0:00");
    }

    #[test]
    fn programs_are_found_only_when_executable() {
        let d = tempfile::tempdir().unwrap();
        let (yes, no) = (d.path().join("buffmpeg"), d.path().join("notexec"));
        std::fs::write(&yes, b"").unwrap();
        std::fs::write(&no, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&yes, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = Some(d.path().as_os_str().to_owned());
        assert_eq!(find_program("buffmpeg", path.clone()), Some(yes));
        assert_eq!(find_program("notexec", path.clone()), None);
        assert_eq!(find_program("missing", path), None);
        assert_eq!(find_program("buffmpeg", None), None);
        // The empty entry a trailing separator produces finds nothing.
        let empty = Some(std::ffi::OsString::from(""));
        assert_eq!(find_program("buffmpeg", empty), None);
    }

    #[test]
    fn a_player_with_no_backend_ignores_its_controls() {
        let mut v = VideoPlayer::default();
        assert!(!v.is_open());
        assert!(!v.run_action("video:toggle"));
        assert!(!v.run_action("video:seek:clip.mp4:-10"));
        assert!(!v.scrub_drag(10.0));
        assert!(!v.scrub_release());
        v.tick(Instant::now());
        v.seek_to(4.0);
        v.close();
        assert_eq!(v.state(), &State::Idle);
    }

    #[test]
    fn a_decoder_with_bogus_tools_opens_nothing() {
        let mut pipe = FfmpegPipe::with_tools(PathBuf::from("/nonexistent/ffmpeg"), PathBuf::from("/x"));
        let err = pipe.open(&Source::file("/etc/hostname"), (10, 10), 0.0).unwrap_err();
        assert!(err.contains("ffmpeg"), "{err}");
        assert!(!pipe.is_running());
        assert!(pipe.poll().is_none());
        pipe.close();
    }
}
