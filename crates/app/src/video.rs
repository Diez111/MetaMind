//! In-app video playback: a child `ffmpeg` decodes the file and writes raw RGBA frames to a
//! pipe, which a worker thread reads and hands to the view.
//!
//! Nothing is embedded in a window: the pixels are ours, so scrolling, resizing and DPI need no
//! window geometry. The MVP is muted; the `Playback` trait leaves room for a smarter backend.
//!
//! Lifecycle: pausing stops the decoder (`SIGSTOP`), seeking restarts it at a new offset, and
//! closing kills and reaps it, so no child outlives the player. The reader thread paces the frames
//! at the file's own rate: `-re` is asked for, but ffmpeg 9 bursts a short file out regardless.

use crate::icons;
use rustsidian_ui::dom::NodeId;
use rustsidian_ui::{Dom, El, Image, Rect};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vault_core::path;

/// Extensions played inside the application. Anything else still goes to the system player.
pub const VIDEO_EXTS: [&str; 7] = ["mp4", "webm", "mkv", "mov", "m4v", "ogv", "avi"];

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
    /// Start decoding `path` at `offset` seconds, scaled to fit `size`, and play.
    fn open(&mut self, path: &Path, size: (u32, u32), offset: f64) -> Result<(), String>;
    fn pause(&mut self);
    fn resume(&mut self);
    /// Restart decoding at `offset`, keeping the paused state.
    fn seek(&mut self, offset: f64) -> Result<(), String>;
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
    /// Frames per second of the file, to pace the reader; `0.0` plays as fast as the pipe fills.
    rate: f32,
    path: PathBuf,
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
            path: PathBuf::new(),
            paused: false,
            waker: None,
        }
    }

    /// What `ffprobe` says about `path`, through the probe this decoder was built with.
    pub fn probe(&self, path: &Path) -> Result<MediaInfo, String> {
        probe(&self.probe, path)
    }

    /// Frames per second to play at. Set before `open`.
    pub fn set_rate(&mut self, fps: f32) {
        self.rate = if fps.is_finite() && fps > 0.0 { fps } else { 0.0 };
    }

    /// Where to call when a frame is ready. Set before `open`.
    pub fn set_waker(&mut self, waker: Option<Waker>) {
        self.waker = waker;
    }

    /// Run `ffmpeg` for the current file and size, replacing any running decoder.
    fn spawn(&mut self, offset: f64) -> Result<(), String> {
        self.close();
        let args = ffmpeg_args(&self.path, self.size, offset);
        let mut child = Command::new(&self.exe)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo ejecutar ffmpeg: {e}"))?;
        let out = child.stdout.take().ok_or("ffmpeg no dejó una salida que leer")?;
        let (tx, rx) = sync_channel(2);
        let (size, rate, waker) = (self.size, self.rate, self.waker.clone());
        std::thread::spawn(move || read_frames(out, size, rate, tx, waker));
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
    fn open(&mut self, path: &Path, size: (u32, u32), offset: f64) -> Result<(), String> {
        if !usable(&self.exe) {
            return Err("no está instalado ffmpeg".to_string());
        }
        if !usable(&self.probe) {
            return Err("no está instalado ffprobe".to_string());
        }
        if !path.is_file() {
            return Err(format!("no existe {}", path.display()));
        }
        self.path = path.to_path_buf();
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
/// draining stalls the decoder instead of growing memory; the reader sets the pace, which `-re`
/// alone does not do on every build (ffmpeg 9 bursts a short file out in milliseconds).
fn read_frames(mut out: impl Read, size: (u32, u32), rate: f32, tx: SyncSender<Frame>, waker: Option<Waker>) {
    let (w, h) = size;
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    let mut shown = 0u64;
    let step = (rate > 0.0).then(|| Duration::from_secs_f32(1.0 / rate));
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
        // One frame of sleep per frame: the pipe backs up, the decoder blocks, and the film runs
        // on the same clock as the time readout.
        if let Some(step) = step {
            std::thread::sleep(step);
        }
    }
    let end =
        if shown == 0 { Frame::Failed("no se pudo decodificar el video".to_string()) } else { Frame::Ended };
    let _ = tx.send(end);
}

/// A pixmap from raw RGBA rows. `ffmpeg` writes opaque frames; alpha is premultiplied only when
/// the frame actually has some, as the rasterizer expects.
fn rgba_image(src: &[u8], w: u32, h: u32) -> Option<Image> {
    let mut pm = rustsidian_ui::Pixmap::new(w, h)?;
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

/// Ask `ffprobe` about the file. The reason it gives is what the notice shows.
pub fn probe(tool: &Path, path: &Path) -> Result<MediaInfo, String> {
    let out = Command::new(tool)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
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
    let fps = stream.get("r_frame_rate").and_then(|r| r.as_str()).map_or(0.0, parse_rate);
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

/// The argv that decodes to a raw RGBA pipe. `-re` paces decoding to the file's own rate, `-ss`
/// before `-i` seeks by fast forward, and `-an` keeps it muted and cheap.
pub fn ffmpeg_args(path: &Path, size: (u32, u32), offset: f64) -> Vec<String> {
    vec![
        "-nostdin".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-re".to_string(),
        "-ss".to_string(),
        format!("{:.3}", offset.max(0.0)),
        "-i".to_string(),
        path.to_string_lossy().into_owned(),
        "-an".to_string(),
        "-sn".to_string(),
        "-vf".to_string(),
        format!("scale={}:{}", size.0, size.1),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "rgba".to_string(),
        "-".to_string(),
    ]
}

/// The largest size that fits `src` into `cap`, keeping the aspect, never upscaling, with both
/// sides even so the colour planes stay aligned.
pub fn fitted(src: (u32, u32), cap: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let k = (cap.0 as f32 / sw).min(cap.1 as f32 / sh).min(1.0);
    let even = |v: f32| ((v.round() as u32) / 2 * 2).max(2);
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
#[derive(Clone, Copy, Default, Debug)]
pub struct Clock {
    offset: f64,
    since: Option<Instant>,
    played: Duration,
}

impl Clock {
    /// Playing from `offset` at `now`.
    pub fn at(offset: f64, now: Instant) -> Clock {
        Clock { offset: offset.max(0.0), since: Some(now), played: Duration::ZERO }
    }

    /// Stopped at `offset`.
    pub fn paused_at(offset: f64) -> Clock {
        Clock { offset: offset.max(0.0), since: None, played: Duration::ZERO }
    }

    pub fn position(&self, now: Instant) -> f64 {
        let live = self.since.map_or(0.0, |s| now.saturating_duration_since(s).as_secs_f64());
        self.offset + self.played.as_secs_f64() + live
    }

    pub fn playing(&self) -> bool {
        self.since.is_some()
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(since) = self.since.take() {
            self.played += now.saturating_duration_since(since);
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

/// The open player: which file, in which note, at what time, and the last frame shown.
#[derive(Default)]
pub struct VideoPlayer {
    backend: Option<Box<dyn Playback>>,
    /// The vault-relative media path; empty when no player is open.
    path: String,
    /// The vault-relative note that shows it: the player lives while that note is open.
    note: String,
    info: MediaInfo,
    size: (u32, u32),
    clock: Clock,
    current: Option<Image>,
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
        self.close();
        let fail = |e: String| format!("No se puede reproducir {rel}: {e}");
        let mut pipe = FfmpegPipe::new().map_err(fail)?;
        let info = pipe.probe(abs).map_err(fail)?;
        let size = fitted((info.width, info.height), cap);
        self.state = State::Loading;
        pipe.set_waker(self.waker.clone());
        pipe.set_rate(info.fps);
        if let Err(e) = pipe.open(abs, size, 0.0) {
            self.state = State::Idle;
            return Err(fail(e));
        }
        self.backend = Some(Box::new(pipe));
        self.path = rel.to_string();
        self.note = note.to_string();
        self.info = info;
        self.size = size;
        self.clock = Clock::at(0.0, Instant::now());
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

    pub fn duration(&self) -> f64 {
        self.info.duration
    }

    /// Whether a decoder process is alive right now.
    pub fn is_running(&mut self) -> bool {
        self.backend.as_mut().is_some_and(|b| b.is_running())
    }

    pub fn position(&self) -> f64 {
        let p = self.clock.position(Instant::now());
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
            if self.state == State::Loading {
                self.state = State::Playing;
            }
        }
        match end {
            // The last frame stays on screen; play starts again from the beginning.
            Some(Ok(())) => {
                let end =
                    if self.info.duration > 0.0 { self.info.duration } else { self.clock.position(now) };
                self.clock = Clock::paused_at(end);
                self.state = State::Idle;
            }
            Some(Err(e)) => self.state = State::Failed(e),
            None => {}
        }
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
        let Some(backend) = self.backend.as_mut() else { return };
        let t = if self.info.duration > 0.0 { t.clamp(0.0, self.info.duration) } else { t.max(0.0) };
        let paused = self.state == State::Paused;
        if let Err(e) = backend.seek(t) {
            self.state = State::Failed(e);
            return;
        }
        let now = Instant::now();
        if paused {
            backend.pause();
            self.clock = Clock::paused_at(t);
        } else {
            self.clock = Clock::at(t, now);
        }
        self.state = if paused { State::Paused } else { State::Playing };
    }

    /// Kill the decoder and go back to the card.
    pub fn close(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            backend.close();
        }
        self.path.clear();
        self.note.clear();
        self.info = MediaInfo::default();
        self.size = (0, 0);
        self.clock = Clock::default();
        self.current = None;
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

/// The player widget that replaces a video's card: the frame, the controls and the track.
pub fn widget(app: &crate::App, dom: &mut Dom, view: NodeId, rel: &str) -> NodeId {
    let v = &app.video;
    let scale = if app.scale > 0.0 { app.scale } else { 1.0 };
    let (w, h) = v.size();
    let (fw, fh) = if w > 0 && h > 0 { (w as f32 / scale, h as f32 / scale) } else { (640.0, 360.0) };
    let loading = *v.state() == State::Loading;
    let card = dom.add(view, El::div().class("video-player").class_if(loading, "is-loading"));
    let size = format!("width: {fw:.0}px; height: {fh:.0}px");
    if let Some(image) = v.current() {
        dom.add(card, El::div().class("video-player-frame").style(&size).image(image.clone()));
    } else {
        let frame = dom.add(card, El::div().class("video-player-frame").style(&size));
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
    for (delta, label) in [(-10.0, "Retroceder 10 segundos"), (10.0, "Avanzar 10 segundos")] {
        let text = if delta < 0.0 { "−10s" } else { "+10s" };
        dom.add(
            controls,
            El::div()
                .class("video-player-btn video-player-pill")
                .attr("aria-label", label)
                .text(text)
                .action(format!("video:seek:{rel}:{delta:.0}")),
        );
    }
    let shown = v.scrub_preview().unwrap_or_else(|| v.position());
    let duration = v.duration();
    dom.add(
        controls,
        El::div().class("video-player-time").text(format!("{} / {}", mmss(shown), mmss(duration))),
    );
    let track = dom.add(
        controls,
        El::div().class("video-player-track").attr("aria-label", "Posición").action("video:scrub"),
    );
    let t = if duration > 0.0 { (shown / duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
    // Fill and rest share the space left by the thumb in proportion to the time played.
    dom.add(track, El::div().class("video-player-fill").style(&format!("flex-grow: {t:.4}")));
    dom.add(track, El::div().class("video-player-thumb"));
    dom.add(track, El::div().class("video-player-rest").style(&format!("flex-grow: {:.4}", 1.0 - t)));
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
        let args = ffmpeg_args(Path::new("/v/clip.mp4"), (320, 180), 12.5);
        assert_eq!(
            args,
            [
                "-nostdin",
                "-loglevel",
                "error",
                "-re",
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
        assert_eq!(ffmpeg_args(Path::new("a.mp4"), (2, 2), -3.0)[5], "0.000");
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
        let err = pipe.open(Path::new("/etc/hostname"), (10, 10), 0.0).unwrap_err();
        assert!(err.contains("ffmpeg"), "{err}");
        assert!(!pipe.is_running());
        assert!(pipe.poll().is_none());
        pipe.close();
    }
}
