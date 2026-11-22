//! Dirty-rect rendering: repaint only what changed since the frame on screen.
//!
//! The app rebuilds its display list every frame and does not say what changed; the tracker keeps
//! the list it presented last, diffs it against the new one and hands the rasterizer the device
//! rectangles that can differ. Pixels outside them are left untouched, which is only safe because
//! the rasterizer clips every command to the rectangle with a pixel-exact stencil: inside a dirty
//! rect the result is bit for bit what a full repaint would have produced (see the A/B tests in
//! `crates/app/tests/damage.rs`). This is rxi's cached software rendering
//! (<https://rxi.github.io/cached_software_rendering.html>): the app redraws "everything", the
//! renderer figures out what is actually new.

use crate::scene::{paint, Cmd, DevRect, Scene};
use crate::text::Fonts;
use std::time::Instant;
use theme_engine::Color;
use tiny_skia::Pixmap;

/// Dirty rects closer than this (device pixels) are merged: repainting a seam costs more than
/// repainting the gap.
const MERGE_GAP: i32 = 32;
/// Past this much of the window, one full pass over the whole scene is cheaper than a replay per
/// rect, so the tracker asks for a full paint instead.
const FULL_FRACTION: (u64, u64) = (2, 3); // two thirds
/// Never more rects than this; past it the change is spread out enough that its bounding box is
/// the honest answer.
const MAX_RECTS: usize = 24;
/// Above this many raw rects, merging them one by one would cost more than the frame.
const MAX_RAW: usize = 64;

/// What a frame cost: which regions were repainted and how long the two halves took.
#[derive(Clone, Debug, Default)]
pub struct Damage {
    /// The whole pixmap was repainted, either because nothing was retained or because the change
    /// covered most of the window.
    pub full: bool,
    /// The rectangles repainted, in device pixels. Empty with `full == false` means the frame was
    /// identical to the one on screen and nothing was painted at all.
    pub rects: Vec<DevRect>,
    /// Time spent diffing the two display lists.
    pub diff_us: u64,
    /// Time spent rasterizing.
    pub paint_us: u64,
}

impl Damage {
    /// Nothing was painted: the frame on screen already is this frame.
    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }
    /// Device pixels repainted.
    pub fn area(&self, w: u32, h: u32) -> u64 {
        if self.full {
            return u64::from(w) * u64::from(h);
        }
        self.rects.iter().map(DevRect::area).sum()
    }
}

/// The frame that is on screen: its display list and how it was rasterized.
struct Retained {
    cmds: Vec<Cmd>,
    scale: f32,
    clear: Color,
    w: u32,
    h: u32,
    /// The pixel buffer the frame was painted into, so a new one is never mistaken for it.
    ptr: usize,
}

/// Remembers the display list that was presented and turns the next one into dirty rects.
#[derive(Default)]
pub struct DamageTracker {
    prev: Option<Retained>,
    /// Set while the app is in its start-up: the first frames paint everything.
    enabled: bool,
}

impl DamageTracker {
    /// A tracker that has nothing retained: the first frame it sees is painted in full.
    pub fn new() -> DamageTracker {
        DamageTracker { prev: None, enabled: true }
    }
    /// Turns dirty-rect painting off for good (the `METAMIND_FULL_PAINT=1` fallback).
    pub fn disable(&mut self) {
        self.enabled = false;
        self.prev = None;
    }
    /// Forgets the retained frame: the next one is painted in full. What the A/B tests and the
    /// benchmarks use to measure the two paths back to back.
    pub fn reset(&mut self) {
        self.prev = None;
    }
    /// Whether the tracker is doing its job (not disabled by the environment or a caller).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

fn forced_full() -> bool {
    static FORCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| std::env::var_os("METAMIND_FULL_PAINT").is_some_and(|v| v != "0" && !v.is_empty()))
}

/// Paint `scene` into `pixmap`, reusing the pixels the tracker's previous frame left there.
///
/// The scene is taken by value so the tracker can keep it without copying: what it retains is
/// exactly the list that was painted.
pub fn render_damaged(
    scene: Scene,
    fonts: &mut Fonts,
    pixmap: &mut Pixmap,
    scale: f32,
    clear: Color,
    tracker: &mut DamageTracker,
) -> Damage {
    let (w, h) = (pixmap.width(), pixmap.height());
    let ptr = pixmap.data().as_ptr() as usize;
    let t = Instant::now();
    let reusable = tracker.enabled
        && !forced_full()
        && tracker
            .prev
            .as_ref()
            .is_some_and(|p| p.w == w && p.h == h && p.scale == scale && p.ptr == ptr && p.clear == clear);
    let mut damage = Damage::default();
    let rects = if reusable {
        match tracker.prev.as_ref() {
            Some(prev) => dirty_rects(&prev.cmds, &scene.cmds, fonts, scale, w, h),
            None => None,
        }
    } else {
        None
    };
    damage.diff_us = t.elapsed().as_micros() as u64;
    let t = Instant::now();
    match rects {
        None => {
            paint(&scene, fonts, pixmap, scale, clear, None);
            damage.full = true;
        }
        Some(rects) => {
            for r in &rects {
                paint(&scene, fonts, pixmap, scale, clear, Some(*r));
            }
            damage.rects = rects;
        }
    }
    damage.paint_us = t.elapsed().as_micros() as u64;
    tracker.prev = Some(Retained { cmds: scene.cmds, scale, clear, w, h, ptr });
    damage
}

/// The region of the pixmap that can differ between the two display lists, or `None` when the
/// whole window has to be repainted.
fn dirty_rects(prev: &[Cmd], next: &[Cmd], fonts: &mut Fonts, scale: f32, w: u32, h: u32) -> Option<Vec<DevRect>> {
    let (n, m) = (next.len(), prev.len());
    let (mut i, mut jn, mut jm) = (0, n, m);
    while i < n && i < m && next[i] == prev[i] {
        i += 1;
    }
    while jn > i && jm > i && next[jn - 1] == prev[jm - 1] {
        jn -= 1;
        jm -= 1;
    }
    let (old, new) = (&prev[i..jm], &next[i..jn]);
    if old.is_empty() && new.is_empty() {
        return Some(Vec::new());
    }
    // A clip that is pushed or popped differently shifts what everything after it is clipped to,
    // and no rectangle bounds that: repaint the window.
    let clip_changed = |c: &Cmd| matches!(c, Cmd::PushClip(_) | Cmd::PopClip);
    if old.iter().any(clip_changed) || new.iter().any(clip_changed) {
        return None;
    }
    let mut out: Vec<DevRect> = Vec::new();
    let add = |r: &crate::scene::Rect, out: &mut Vec<DevRect>| {
        let d = DevRect::from_rect(r, scale).clamped(w, h);
        if !d.is_empty() {
            out.push(d);
        }
    };
    if old.len() == new.len() {
        // Same shape of list: only the commands that differ matter, and each contributes the
        // pixels it painted before and the ones it paints now.
        for (a, b) in old.iter().zip(new) {
            if a != b {
                add(&a.bounds(fonts), &mut out);
                add(&b.bounds(fonts), &mut out);
            }
        }
    } else {
        // Commands were inserted or removed: everything around them shifted, so the changed
        // stretch is one region, from the first command that differs to the last.
        let mut union: Option<DevRect> = None;
        for c in old.iter().chain(new.iter()) {
            let d = DevRect::from_rect(&c.bounds(fonts), scale).clamped(w, h);
            if !d.is_empty() {
                union = Some(union.map_or(d, |u| u.union(&d)));
            }
        }
        out.extend(union);
    }
    let rects = merge(out, w, h);
    // Past this much of the window, a single full pass beats replaying the scene per rect.
    let area: u64 = rects.iter().map(DevRect::area).sum();
    if area * FULL_FRACTION.1 > u64::from(w) * u64::from(h) * FULL_FRACTION.0 {
        return None;
    }
    Some(rects)
}

/// Unions overlapping and nearby rects, then folds the smallest ones away until at most
/// [`MAX_RECTS`] are left. Disjointness is not needed for correctness, only for cost.
fn merge(mut rects: Vec<DevRect>, w: u32, h: u32) -> Vec<DevRect> {
    rects.retain(|r| !r.is_empty());
    for r in &mut rects {
        *r = r.clamped(w, h);
    }
    rects.retain(|r| !r.is_empty());
    if rects.len() > MAX_RAW {
        // A change spread over the window: one rect around all of it is both correct and cheaper.
        let all = rects.iter().skip(1).fold(rects[0], |u, r| u.union(r));
        return vec![all];
    }
    // Union everything that is close enough that repainting the seam would cost more than the ink.
    loop {
        let mut merged = false;
        'sweep: for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                if rects[i].near(&rects[j], MERGE_GAP) {
                    rects[i] = rects[i].union(&rects[j]);
                    rects.swap_remove(j);
                    merged = true;
                    break 'sweep;
                }
            }
        }
        if !merged {
            break;
        }
    }
    // Still more than the budget: fold the pair whose union wastes the least area.
    while rects.len() > MAX_RECTS {
        let mut best: Option<(u64, usize, usize)> = None;
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let cost = rects[i].union(&rects[j]).area() - rects[i].area() - rects[j].area();
                if best.is_none_or(|(b, _, _)| cost < b) {
                    best = Some((cost, i, j));
                }
            }
        }
        let Some((_, i, j)) = best else { break };
        rects[i] = rects[i].union(&rects[j]);
        rects.swap_remove(j);
    }
    rects
}
