//! The gate for dirty-rect rendering: whatever the app does, the pixels a frame leaves on screen
//! must be the ones a full repaint of that frame would leave — bit for bit.
//!
//! Each scenario drives the real app (typing, hovering, clicking a folder, opening settings,
//! scrolling, opening a note) and records the display list of every frame it paints. The lists are
//! then replayed twice at the user's window size, once with a full repaint per frame and once with
//! a [`DamageTracker`] reusing the pixels of the frame before, and the two pixmaps are compared
//! after every frame: a leak in the damage — one pixel that changed outside the dirty rects, or a
//! rect that missed a changed command — shows up here and nowhere else.
use metamind_app::{render, App, Frame, Key};
use metamind_ui::{Color, DamageTracker, Fonts, Pixmap, Scene};
use std::fs;

const W: u32 = 1280;
const H: u32 = 800;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    let prose = (1..=60)
        .map(|i| format!("Line {i} of the note, long enough that it wraps onto another line when the pane is narrow.\n"))
        .collect::<String>();
    w("Home.md", &format!("# Home\n\nSee [[Second]] and **bold** text.\n\n{prose}"));
    w("Second.md", "# Second\n\nSome other prose, with a [[Home]] link back.\n");
    for i in 1..=12 {
        w(&format!("dir/Note{i}.md"), &format!("# Note {i}\n\nprose {i}\n"));
    }
    w("dir/Third.md", "third\n");
    d
}

/// One frame as it was painted: the display list and the colour the window was cleared with.
struct Painted {
    scene: Scene,
    clear: Color,
    /// What the damage run said it repainted, so the test can prove it was not always "everything".
    partial: bool,
    nothing: bool,
}

/// Drives the app and records every frame's display list.
struct Frames {
    app: App,
    fonts: Fonts,
    pm: Pixmap,
    out: Vec<Painted>,
}

impl Frames {
    fn new(dir: &tempfile::TempDir, open: &str) -> Frames {
        let mut app = App::open(dir.path()).unwrap();
        app.window_w = W as f32;
        app.window_h = H as f32;
        app.open_note(open);
        let mut f = Frames { app, fonts: Fonts::new(), pm: Pixmap::new(W, H).unwrap(), out: Vec::new() };
        f.step();
        f
    }

    /// Paints one frame through the app and keeps its display list.
    fn step(&mut self) -> Frame {
        let frame = render(&mut self.app, &mut self.fonts, &mut self.pm, 1.0);
        let painted = Painted {
            scene: frame.dom.paint(),
            clear: frame.dom.nodes[0].style.background,
            partial: !frame.damage.full && !frame.damage.rects.is_empty(),
            nothing: frame.damage.is_empty(),
        };
        self.out.push(painted);
        frame
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    fn click(&mut self, selector: &str) -> bool {
        let frame = self.step();
        let Some(n) = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == selector)) else {
            return false;
        };
        let (x, y) = (n.rect.x + n.rect.w / 2.0, n.rect.y + n.rect.h / 2.0);
        self.app.click(&frame, &mut self.fonts, x, y, false);
        self.app.release();
        self.step();
        true
    }

    fn hover(&mut self, selector: &str) -> bool {
        let frame = self.step();
        let Some(n) = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == selector)) else {
            return false;
        };
        let (x, y) = (n.rect.x + n.rect.w / 2.0, n.rect.y + n.rect.h / 2.0);
        let moved = self.app.hover(&frame, x, y);
        self.step();
        moved
    }
}

/// Replays the recorded frames through both paths and asserts the pixels agree after each one.
fn assert_same_pixels(name: &str, painted: &[Painted]) {
    let mut fonts = Fonts::new();
    let mut full = Pixmap::new(W, H).unwrap();
    let mut damaged = Pixmap::new(W, H).unwrap();
    let mut tracker = DamageTracker::new();
    for (i, p) in painted.iter().enumerate() {
        metamind_ui::render(&p.scene, &mut fonts, &mut full, 1.0, p.clear);
        metamind_ui::render_damaged(p.scene.clone(), &mut fonts, &mut damaged, 1.0, p.clear, &mut tracker);
        if full.data() != damaged.data() {
            // A whole 1280x800 buffer in an assert message is unreadable: report where the two
            // paths disagree first, then how many pixels are involved.
            let (a, b) = (full.data(), damaged.data());
            let diff = a.iter().zip(b).filter(|(x, y)| x != y).count();
            let at =
                (0..a.len() / 4).position(|px| a[px * 4..px * 4 + 4] != b[px * 4..px * 4 + 4]).unwrap_or(0);
            let (x, y) = (at as u32 % W, at as u32 / W);
            panic!(
                "{name}: frame {i} differs between a full repaint and a damage repaint: \
                 {diff} byte(s) from pixel ({x}, {y}); full {:?}, damaged {:?}",
                &a[at * 4..at * 4 + 4],
                &b[at * 4..at * 4 + 4]
            );
        }
    }
}

/// Every scenario in one place: a list of interactions to run from a fresh app.
fn run<T>(name: &str, scenario: impl FnOnce(&mut Frames) -> T) -> (String, Vec<Painted>) {
    let d = vault();
    let mut f = Frames::new(&d, "Home.md");
    f.steps(2);
    scenario(&mut f);
    (name.to_string(), f.out)
}

#[test]
fn damage_never_changes_a_pixel() {
    let mut all: Vec<(String, Vec<Painted>)> = Vec::new();

    all.push(run("quiet", |f| f.steps(3)));
    all.push(run("typing", |f| {
        f.app.key(Key::parse("Ctrl+End").unwrap());
        for c in "hola".chars() {
            f.app.text_input(&c.to_string());
            f.step();
        }
    }));
    all.push(run("hover", |f| {
        assert!(f.hover("nav-file-title"), "no explorer row to hover");
        assert!(f.hover("nav-folder-title"), "no folder row to hover");
    }));
    all.push(run("toggle-folder", |f| {
        assert!(f.click("nav-folder-title"), "no folder row to click");
        f.steps(2);
    }));
    all.push(run("open-settings", |f| {
        f.app.run("app:open-settings");
        f.steps(2);
        f.app.key(Key::parse("Escape").unwrap());
        f.steps(2);
    }));
    all.push(run("sidebar", |f| {
        f.app.run("app:toggle-left-sidebar");
        f.steps(6);
        f.app.run("app:toggle-left-sidebar");
        f.steps(6);
    }));
    all.push(run("scroll", |f| {
        let frame = f.step();
        let view = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "view-content")).unwrap();
        let (x, y) = (view.rect.x + 20.0, view.rect.y + 20.0);
        for _ in 0..4 {
            f.app.wheel(&frame, x, y, -3.0);
            f.steps(2);
        }
    }));
    all.push(run("open-note", |f| {
        f.app.open_note("dir/Second.md");
        f.steps(3);
        assert!(f.click("nav-file-title"), "no explorer row to click");
    }));

    for (name, painted) in &all {
        assert!(painted.len() > 2, "{name}: no frames recorded");
        assert_same_pixels(name, painted);
    }
    let partials = |name: &str| -> usize {
        all.iter().find(|(n, _)| n == name).map(|(_, p)| p.iter().filter(|f| f.partial).count()).unwrap_or(0)
    };
    // The scenarios whose whole point is a small change: they must go through the dirty path, or
    // the pixel gate above would pass by repainting everything every time.
    let summary: Vec<String> = all.iter().map(|(n, _)| format!("{n}: {}", partials(n))).collect();
    println!("frames de daño parcial por escenario: {summary:?}");
    if forced_full() {
        // The escape hatch repaints everything by design: only the pixels are asserted then.
        return;
    }
    for name in ["typing", "hover"] {
        assert!(partials(name) > 0, "{name} never took the dirty path: {summary:?}");
    }
    // The gate is only worth something if the damage path actually ran: at least some frames must
    // have repainted a part of the window instead of all of it.
    let partial = all.iter().flat_map(|(_, p)| p).filter(|p| p.partial).count();
    let nothing = all.iter().flat_map(|(_, p)| p).filter(|p| p.nothing).count();
    assert!(partial > 0, "no frame went through a partial repaint: the test proves nothing");
    assert!(nothing > 0, "no frame was skipped as unchanged: the tracker never saw a still window");
}

#[test]
fn the_check_catches_a_leaked_pixel() {
    // A control for the gate above: the comparison it makes really does notice a difference of
    // one pixel, so a green `damage_never_changes_a_pixel` is not vacuous.
    let mut a = Pixmap::new(8, 8).unwrap();
    let mut b = Pixmap::new(8, 8).unwrap();
    a.fill(metamind_ui::SkColor::from_rgba8(10, 20, 30, 255));
    b.fill(metamind_ui::SkColor::from_rgba8(10, 20, 30, 255));
    assert_eq!(a.data(), b.data());
    let at = b.width() as usize * 4 + 4;
    b.data_mut()[at] ^= 1;
    assert_ne!(a.data(), b.data(), "the pixel comparison is blind");
}

/// Whether the run is the `METAMIND_FULL_PAINT` fallback, where nothing is meant to be partial.
fn forced_full() -> bool {
    std::env::var_os("METAMIND_FULL_PAINT").is_some_and(|v| v != "0" && !v.is_empty())
}

#[test]
fn full_paint_fallback_tracks_the_environment() {
    // `METAMIND_FULL_PAINT` is the escape hatch; when it is set the tracker must stop diffing and
    // paint everything. The variable is process wide, so this test only asserts the default here:
    // the fallback itself is exercised by running the suite with it set.
    if forced_full() {
        return;
    }
    let d = vault();
    let mut f = Frames::new(&d, "Home.md");
    f.steps(3);
    let last = f.step();
    assert!(!last.damage.full, "a still window still repainted everything");
}
