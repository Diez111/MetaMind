//! A drawing tablet on Linux: the pen's pressure, its tilt and the eraser at the other end.
//!
//! A window toolkit hands a drawing program a mouse: a position and three buttons. What a stylus
//! has instead is a *range* — how hard the tip is pressed, which way it leans — and no window
//! toolkit on Linux gives that to a plain winit window. The kernel does: a tablet shows up as an
//! `evdev` device with `ABS_PRESSURE` and `BTN_TOOL_PEN`, and this crate reads it.
//!
//! # What it does, and what it does not
//!
//! It reads `/dev/input/event*` on a thread of its own and sends [`PenSample`]s over a channel.
//! It does not open a window, move a pointer or draw: the app combines the sample with the
//! pointer position its own toolkit reported, because the pen and the cursor have to be the same
//! thing on screen.
//!
//! # No `unsafe`, no dependency
//!
//! A blocking `read` on the device is enough: the kernel wakes the thread when the pen reports,
//! and returns `ENODEV` when the tablet goes away. The `struct input_event` that comes back is
//! decoded byte by byte ([`Frame`]) with the offsets of the ABI, so the crate needs neither
//! `libc` nor a `bindgen` — and the workspace forbids `unsafe` outright.
//!
//! # Permissions
//!
//! `/dev/input/event*` belongs to `root:input` and is mode `0660`: a user in the `input` group can
//! read it, everyone else gets `EACCES`. [`device_error`] turns that into a sentence with the way
//! out (add the user to the group, or a udev rule for that one device), which is what the settings
//! panel shows.
//!
//! # No tablet, no problem
//!
//! [`devices`] returns an empty list on a machine with no tablet, on a system that is not Linux,
//! and when `/proc/bus/input/devices` cannot be read. Everything is optional: the app draws with
//! the mouse at a constant width and never asks twice.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

// The Linux input ABI, as numbers. They are stable and are what the kernel writes.
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_PRESSURE: u16 = 0x18;
const ABS_TILT_X: u16 = 0x1a;
const ABS_TILT_Y: u16 = 0x1b;
const BTN_TOOL_PEN: u16 = 0x140;
const BTN_TOOL_RUBBER: u16 = 0x141;
const BTN_TOUCH: u16 = 0x14a;
const BTN_STYLUS: u16 = 0x14b;
const BTN_STYLUS2: u16 = 0x14c;

/// A tablet, as `/proc/bus/input/devices` describes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PenDevice {
    /// `/dev/input/eventN`.
    pub path: PathBuf,
    pub name: String,
    /// The device reports `ABS_TILT_X`/`ABS_TILT_Y`.
    pub tilt: bool,
    /// The device reports `BTN_TOOL_RUBBER`: the other end of the pen erases.
    pub eraser: bool,
    /// `(min, max)` of `ABS_X` and `ABS_Y`, in the device's units.
    pub x_range: (i32, i32),
    pub y_range: (i32, i32),
    /// `(min, max)` of `ABS_PRESSURE`.
    pub pressure_range: (i32, i32),
}

impl PenDevice {
    /// A tablet with the ranges most of them use, for a test or a device whose ranges could not be
    /// read.
    pub fn with_default_ranges(path: PathBuf, name: String) -> PenDevice {
        PenDevice {
            path,
            name,
            tilt: false,
            eraser: false,
            x_range: (0, 65_535),
            y_range: (0, 65_535),
            pressure_range: (0, 2047),
        }
    }

    /// `x` normalized to 0..=1. A device with a degenerate range reports 0.5 instead of dividing
    /// by zero.
    pub fn x_norm(&self, x: i32) -> f32 {
        norm(x, self.x_range)
    }

    pub fn y_norm(&self, y: i32) -> f32 {
        norm(y, self.y_range)
    }

    pub fn pressure(&self, value: i32) -> f32 {
        norm(value, self.pressure_range)
    }
}

fn norm(value: i32, (min, max): (i32, i32)) -> f32 {
    if max <= min {
        return 0.5;
    }
    ((value - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
}

/// One thing the pen did.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PenSample {
    /// 0..=1 across the tablet.
    pub x: f32,
    /// 0..=1 down the tablet.
    pub y: f32,
    /// 0..=1. A mouse-like device reports 0.5.
    pub pressure: f32,
    /// Degrees, -60..=60, when the device reports it.
    pub tilt: (f32, f32),
    pub kind: PenKind,
}

impl PenSample {
    pub fn is_contact(self) -> bool {
        matches!(self.kind, PenKind::Tip | PenKind::Eraser)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PenKind {
    /// The pen is in range but not touching: the cursor follows it.
    Hover,
    Tip,
    /// The pen was turned around: the tip erases.
    Eraser,
    /// The stylus button, under the thumb.
    Barrel,
    /// The pen left the tablet.
    Out,
}

/// Why a device could not be opened, in words with the way out.
pub fn device_error(path: &Path, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            "No hay permiso para leer {}: agregá tu usuario al grupo `input` (o una regla udev para \
             la tableta) y volvé a entrar a la sesión.",
            path.display()
        )
    } else if err.kind() == std::io::ErrorKind::NotFound {
        format!("{} ya no está: ¿desenchufaste la tableta?", path.display())
    } else {
        format!("No se pudo abrir {}: {err}", path.display())
    }
}

/// One `input_event`, decoded.
///
/// The record is a `timeval` and three numbers: 24 bytes on a 64-bit machine and 16 on a 32-bit
/// one, with `type`, `code` and `value` in the last eight bytes of it. Reading the tail instead of
/// the whole struct is what keeps this crate free of `unsafe` and of a `libc` dependency.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl Frame {
    /// Bytes of one record on this machine.
    pub const SIZE: usize = if std::mem::size_of::<usize>() == 8 { 24 } else { 16 };

    /// Decode the record at `at`. `None` when the slice is short: a read that ends mid-record
    /// happens when a device is unplugged, and it is not an error worth reporting.
    pub fn at(bytes: &[u8], at: usize) -> Option<Frame> {
        let tail = bytes.get(at + Frame::SIZE - 8..at + Frame::SIZE)?;
        Some(Frame {
            kind: u16::from_le_bytes([tail[0], tail[1]]),
            code: u16::from_le_bytes([tail[2], tail[3]]),
            value: i32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]),
        })
    }

    /// Every record in a buffer a `read` filled.
    pub fn all(bytes: &[u8]) -> impl Iterator<Item = Frame> + '_ {
        (0..bytes.len() / Frame::SIZE).filter_map(move |i| Frame::at(bytes, i * Frame::SIZE))
    }
}

/// The pen's state, folded from the frames of one report.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct State {
    pub x: i32,
    pub y: i32,
    pub pressure: i32,
    pub tilt: (i32, i32),
    pub kind: PenKind,
}

impl Default for State {
    fn default() -> State {
        State { x: 0, y: 0, pressure: 0, tilt: (0, 0), kind: PenKind::Out }
    }
}

impl State {
    /// Fold one frame in. True when the frame closed a report (`SYN_REPORT`), which is when the
    /// caller should take a [`PenSample`].
    pub fn apply(&mut self, frame: Frame) -> bool {
        match (frame.kind, frame.code) {
            (EV_ABS, ABS_X) => self.x = frame.value,
            (EV_ABS, ABS_Y) => self.y = frame.value,
            (EV_ABS, ABS_PRESSURE) => self.pressure = frame.value,
            (EV_ABS, ABS_TILT_X) => self.tilt.0 = frame.value,
            (EV_ABS, ABS_TILT_Y) => self.tilt.1 = frame.value,
            (EV_KEY, BTN_TOUCH) => {
                self.kind = if frame.value != 0 { PenKind::Tip } else { PenKind::Hover };
            }
            (EV_KEY, BTN_TOOL_PEN) => {
                self.kind = if frame.value != 0 { PenKind::Hover } else { PenKind::Out };
            }
            (EV_KEY, BTN_TOOL_RUBBER) => {
                self.kind = if frame.value != 0 { PenKind::Eraser } else { PenKind::Hover };
            }
            (EV_KEY, BTN_STYLUS | BTN_STYLUS2) => {
                if frame.value != 0 {
                    self.kind = PenKind::Barrel;
                } else if self.kind == PenKind::Barrel {
                    self.kind = PenKind::Hover;
                }
            }
            (EV_SYN, SYN_REPORT) => return true,
            _ => {}
        }
        false
    }

    pub fn sample(&self, device: &PenDevice) -> PenSample {
        PenSample {
            x: device.x_norm(self.x),
            y: device.y_norm(self.y),
            pressure: device.pressure(self.pressure),
            tilt: (self.tilt.0 as f32, self.tilt.1 as f32),
            kind: self.kind,
        }
    }
}

/// The tablets the system has now, from `/proc/bus/input/devices`. Empty when there are none, when
/// the file cannot be read, and on a system that is not Linux.
pub fn devices() -> Vec<PenDevice> {
    match std::fs::read_to_string("/proc/bus/input/devices") {
        Ok(text) => parse_devices(&text),
        Err(_) => Vec::new(),
    }
}

/// Parse the kernel's device list. Each block is `Key: value` lines; a tablet is a block with
/// `ABS_PRESSURE` in its `B: ABS=` line and a `BTN_TOOL_PEN` in its `B: KEY=` line.
pub fn parse_devices(text: &str) -> Vec<PenDevice> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut name = String::new();
        let mut handlers = String::new();
        let mut abs = String::new();
        let mut keys = String::new();
        for line in block.lines() {
            // A line is `TAG: what=value`: `N: Name="…"`, `H: Handlers=event12`,
            // `B: KEY=…`. The tag comes before the colon.
            let Some((tag, rest)) = line.split_once(':') else { continue };
            match tag.trim() {
                // The value follows the `=`: `Name="…"`, `Handlers=event12`.
                "N" => {
                    name = after_eq(rest).trim_matches('"').to_string();
                }
                "H" => handlers = after_eq(rest).to_string(),
                "B" => {
                    let Some((kind, bits)) = rest.split_once('=') else { continue };
                    match kind.trim() {
                        "ABS" => abs = bits.trim().to_string(),
                        "KEY" => keys = bits.trim().to_string(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let has_abs = |bit: u16| caps_set(&abs, u32::from(bit));
        if !caps_set(&keys, u32::from(BTN_TOOL_PEN)) || !has_abs(ABS_PRESSURE) {
            continue;
        }
        let Some(event) = handlers.split_whitespace().find(|h| h.starts_with("event")) else {
            continue;
        };
        out.push(PenDevice {
            path: PathBuf::from(format!("/dev/input/{event}")),
            name,
            tilt: has_abs(ABS_TILT_X) && has_abs(ABS_TILT_Y),
            eraser: caps_set(&keys, u32::from(BTN_TOOL_RUBBER)),
            x_range: (0, 65_535),
            y_range: (0, 65_535),
            pressure_range: (0, 2047),
        });
    }
    out
}

/// What follows the first `=` of a `TAG: field=value` line, or the whole rest when there is none.
fn after_eq(rest: &str) -> &str {
    rest.split_once('=').map_or(rest.trim(), |(_, v)| v.trim())
}

/// Whether bit `bit` is set in a `B:`-style bitmap, written as hex words from the most significant
/// down, separated by spaces.
pub fn caps_set(words: &str, bit: u32) -> bool {
    let words: Vec<&str> = words.split_whitespace().collect();
    let total = words.len() as u32 * 64;
    if bit >= total {
        return false;
    }
    // The first word holds the highest bits.
    let word_index = words.len() as u32 - 1 - bit / 64;
    let Ok(word) = u64::from_str_radix(words[word_index as usize], 16) else { return false };
    (word >> (bit % 64)) & 1 == 1
}

/// A reader on its own thread.
pub struct Reader {
    rx: Receiver<PenSample>,
    /// The device being read while it lasts, for the settings panel to show.
    pub device: PenDevice,
}

impl Reader {
    /// Start reading `device`. The error is [`device_error`] when the device cannot be opened.
    pub fn start(device: PenDevice) -> Result<Reader, String> {
        #[cfg(target_os = "linux")]
        {
            let file = std::fs::File::open(&device.path).map_err(|e| device_error(&device.path, &e))?;
            let (tx, rx) = channel();
            let dev = device.clone();
            std::thread::Builder::new()
                .name("pen-input".into())
                .spawn(move || read_loop(file, dev, tx))
                .map_err(|e| format!("No se pudo arrancar el lector del lápiz: {e}"))?;
            Ok(Reader { rx, device })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = device;
            Err("El lápiz con presión se lee por evdev, que solo existe en Linux.".to_string())
        }
    }

    /// The last sample the pen sent, if it sent one since the last call. Never blocks.
    pub fn poll(&mut self) -> Option<PenSample> {
        let mut last = None;
        while let Ok(sample) = self.rx.try_recv() {
            last = Some(sample);
        }
        last
    }
}

/// Read the device until it goes away: a blocking `read` returns on every report and errors out
/// with `ENODEV` when the tablet is unplugged.
#[cfg(target_os = "linux")]
fn read_loop(mut file: std::fs::File, device: PenDevice, tx: std::sync::mpsc::Sender<PenSample>) {
    use std::io::Read;
    let mut buf = [0u8; Frame::SIZE * 64];
    let mut state = State::default();
    loop {
        match file.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                for frame in Frame::all(&buf[..n]) {
                    if state.apply(frame) && tx.send(state.sample(&device)).is_err() {
                        return;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block as `/proc/bus/input/devices` writes a tablet: a pen with pressure, tilt and an
    /// eraser, and a second device that is only a mouse.
    const PROC: &str = "I: Bus=0003 Vendor=28bd Product=0241 Version=0100
N: Name=\"XP-Pen Artist 12 Pen\"
P: Phys=usb-0000:00:14.0-1/input1
H: Handlers=event12
B: PROP=0
B: EV=1b
B: KEY=3 0 0 0 0 0
B: ABS=1000f000003

I: Bus=0003 Vendor=046d Product=c52b Version=0111
N: Name=\"USB Optical Mouse\"
H: Handlers=mouse0 event5
B: EV=17
B: KEY=70000 0 0 0 0
B: ABS=0
";

    #[test]
    fn a_tablet_is_found_and_a_mouse_is_not() {
        let found = parse_devices(PROC);
        assert_eq!(found.len(), 1, "{found:?}");
        let pen = &found[0];
        assert_eq!(pen.name, "XP-Pen Artist 12 Pen");
        assert_eq!(pen.path, PathBuf::from("/dev/input/event12"));
        assert!(pen.tilt, "el lápiz reporta inclinación");
        assert!(pen.eraser, "y tiene goma en el otro extremo");
    }

    #[test]
    fn the_axes_normalize_to_zero_and_one() {
        let pen = PenDevice {
            path: PathBuf::from("/dev/input/event0"),
            name: "test".into(),
            tilt: false,
            eraser: false,
            x_range: (0, 1000),
            y_range: (0, 500),
            pressure_range: (0, 2047),
        };
        assert_eq!(pen.x_norm(0), 0.0);
        assert_eq!(pen.x_norm(1000), 1.0);
        assert!((pen.y_norm(250) - 0.5).abs() < 0.001);
        assert_eq!(pen.pressure(0), 0.0);
        assert_eq!(pen.pressure(2047), 1.0);
        assert!((pen.pressure(1024) - 0.5).abs() < 0.01);
        // A device that reports a single value does not divide by zero.
        let odd = PenDevice { x_range: (5, 5), ..pen.clone() };
        assert_eq!(odd.x_norm(5), 0.5);
        assert_eq!(pen.pressure(9999), 1.0, "fuera de rango se recorta");
    }

    #[test]
    fn a_machine_without_a_tablet_is_an_empty_list() {
        assert!(parse_devices("").is_empty());
        assert!(
            parse_devices("I: Bus=0003\nN: Name=\"Teclado\"\nH: Handlers=event0\nB: EV=120013\n").is_empty()
        );
    }

    #[test]
    fn a_permission_error_says_what_to_do() {
        let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let msg = device_error(Path::new("/dev/input/event12"), &err);
        assert!(msg.contains("grupo"), "{msg}");
        assert!(msg.contains("event12"), "{msg}");
    }

    #[test]
    fn the_bits_of_a_capability_line_are_read_from_the_most_significant_word() {
        // `1c03 0 0 0 0`: BTN_TOOL_PEN (0x140 = 320) is not in the low word, so a pen in the last
        // word means the first word holds the high bits. 0x3 sets bits 0 and 1 there.
        assert!(caps_set("3", 0));
        assert!(caps_set("3", 1));
        assert!(!caps_set("3", 2));
        assert!(!caps_set("3", 64), "no está en el segundo word");
        assert!(caps_set("1 0", 64), "el bit 64 es el primero del word alto");
    }

    #[test]
    fn a_record_is_decoded_from_the_end_of_the_struct() {
        // 24 bytes on a 64-bit machine: a timeval of zeros, then type, code and value.
        let mut bytes = [0u8; 24];
        bytes[16..18].copy_from_slice(&EV_ABS.to_le_bytes());
        bytes[18..20].copy_from_slice(&ABS_PRESSURE.to_le_bytes());
        bytes[20..24].copy_from_slice(&1024i32.to_le_bytes());
        let frame = Frame::at(&bytes, 0).expect("un registro entero");
        assert_eq!(frame, Frame { kind: EV_ABS, code: ABS_PRESSURE, value: 1024 });
        // A read cut in the middle of a record yields nothing instead of panicking.
        assert!(Frame::at(&bytes[..20], 0).is_none());
    }

    #[test]
    fn the_pen_state_follows_the_frames_and_a_report_is_one_sample() {
        let device = PenDevice::with_default_ranges(PathBuf::from("/dev/input/event0"), "test".into());
        let mut state = State::default();
        assert!(!state.apply(Frame { kind: EV_ABS, code: ABS_X, value: 32_767 }));
        assert!(!state.apply(Frame { kind: EV_ABS, code: ABS_Y, value: 0 }));
        assert!(!state.apply(Frame { kind: EV_ABS, code: ABS_PRESSURE, value: 1024 }));
        assert!(!state.apply(Frame { kind: EV_KEY, code: BTN_TOUCH, value: 1 }));
        assert!(state.apply(Frame { kind: EV_SYN, code: SYN_REPORT, value: 0 }), "cierra el reporte");
        let sample = state.sample(&device);
        assert!((sample.x - 0.5).abs() < 0.01);
        assert_eq!(sample.y, 0.0);
        assert!((sample.pressure - 0.5).abs() < 0.01, "{sample:?}");
        assert_eq!(sample.kind, PenKind::Tip);
        assert!(sample.is_contact());
        // Turning the pen around is the eraser.
        state.apply(Frame { kind: EV_KEY, code: BTN_TOOL_RUBBER, value: 1 });
        assert_eq!(state.kind, PenKind::Eraser);
        state.apply(Frame { kind: EV_KEY, code: BTN_TOOL_RUBBER, value: 0 });
        assert_eq!(state.kind, PenKind::Hover);
    }
}
