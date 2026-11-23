//! The pen: the pressure a tablet reports, brought to the board.
//!
//! `pen-input` reads the kernel's `evdev` device; this module decides which one to read, keeps the
//! last pressure the pen reported, and hands it to the view that draws. Nothing here draws: a
//! mouse has no pressure and gets a constant width, so the board works the same with and without
//! a tablet.
//!
//! The pointer position still comes from the window toolkit. The pen and the cursor are the same
//! thing on screen, and the toolkit is what knows where the window is on the desk.

use pen_input::{PenDevice, PenKind, Reader};

/// The tablet, when there is one.
pub struct Pen {
    reader: Option<Reader>,
    /// The device being read, for the settings panel.
    pub device: Option<PenDevice>,
    /// 0..=1, the last pressure the pen reported. A mouse leaves it at 0.5.
    pub pressure: f32,
    /// The pen was turned around: the tip erases.
    pub erasing: bool,
    /// The pen is over the tablet, touching it or not.
    pub in_range: bool,
    /// Why the tablet could not be read, in words with the way out.
    pub error: Option<String>,
    /// `pen.pressure` from the settings: off draws every stroke at the same width.
    pub enabled: bool,
    /// A refresh was already tried for this session, so a machine with no tablet is asked once.
    tried: bool,
}

impl Default for Pen {
    fn default() -> Pen {
        Pen {
            reader: None,
            device: None,
            pressure: 0.5,
            erasing: false,
            in_range: false,
            error: None,
            enabled: true,
            tried: false,
        }
    }
}

impl Pen {
    /// Look for a tablet and start reading it. Called once at start-up and again from the
    /// settings panel, which is the button that says "look again" when a tablet was plugged in
    /// after the app started.
    pub fn refresh(&mut self) {
        self.reader = None;
        self.device = None;
        self.error = None;
        self.tried = true;
        let Some(device) = pen_input::devices().into_iter().next() else {
            self.in_range = false;
            self.erasing = false;
            return;
        };
        match Reader::start(device.clone()) {
            Ok(reader) => {
                self.reader = Some(reader);
                self.device = Some(device);
            }
            Err(why) => {
                self.error = Some(why);
                self.device = Some(device);
            }
        }
    }

    /// The tablets the system has, for the settings panel.
    pub fn devices(&self) -> Vec<PenDevice> {
        pen_input::devices()
    }

    /// True when a tablet is being read.
    pub fn connected(&self) -> bool {
        self.reader.is_some()
    }

    /// Ask once, when nobody asked before: a machine with no tablet pays one read of
    /// `/proc/bus/input/devices` and nothing else.
    pub fn open_once(&mut self) {
        if !self.tried {
            self.refresh();
        }
    }

    /// Take what the pen reported since the last frame.
    pub fn poll(&mut self) {
        let Some(reader) = &mut self.reader else { return };
        while let Some(sample) = reader.poll() {
            self.in_range = sample.kind != PenKind::Out;
            if sample.kind == PenKind::Out {
                self.erasing = false;
                continue;
            }
            if self.enabled {
                self.pressure = sample.pressure.clamp(0.0, 1.0);
            }
            self.erasing = sample.kind == PenKind::Eraser;
        }
    }

    /// A test needs the pressure path without a tablet on the machine: this makes the bridge
    /// behave as if one were connected.
    #[cfg(any(test, feature = "window"))]
    pub fn trial_connected(&mut self, on: bool) {
        self.enabled = on;
        if on {
            self.pressure = self.pressure.clamp(0.0, 1.0);
        }
    }

    /// The width a stroke gets at the current pressure: a light touch draws thin, a hard one
    /// thick. A mouse reports 0.5 and gets the middle of the range.
    pub fn width_scale(&self) -> f32 {
        if self.connected() && self.enabled {
            0.35 + 1.15 * self.pressure
        } else {
            1.0
        }
    }
}
