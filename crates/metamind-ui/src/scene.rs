//! Display list and the software rasterizer (tiny-skia).

use crate::text::{Fonts, Span, TextStyle};
use rayon::prelude::*;
use std::sync::Arc;
use theme_engine::Color;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// A decoded image shared between frames. Two images are equal when they are the same pixmap.
#[derive(Clone)]
pub struct Image(pub Arc<Pixmap>);

impl Image {
    pub fn size(&self) -> (u32, u32) {
        (self.0.width(), self.0.height())
    }
}

impl PartialEq for Image {
    fn eq(&self, o: &Image) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.0.width(), self.0.height())
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }
}

/// A rectangle in device pixels, half open: `x0..x1` × `y0..y1`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DevRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl DevRect {
    /// The device pixels a logical rectangle can touch, rounded outwards and grown by a pixel so
    /// that anti-aliased edges and pixel snapping stay inside.
    pub fn from_rect(r: &Rect, scale: f32) -> DevRect {
        DevRect {
            x0: (r.x * scale).floor() as i32 - 1,
            y0: (r.y * scale).floor() as i32 - 1,
            x1: ((r.x + r.w) * scale).ceil() as i32 + 1,
            y1: ((r.y + r.h) * scale).ceil() as i32 + 1,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    pub fn area(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            (self.x1 - self.x0) as u64 * (self.y1 - self.y0) as u64
        }
    }
    pub fn intersect(&self, o: &DevRect) -> DevRect {
        DevRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }
    pub fn union(&self, o: &DevRect) -> DevRect {
        DevRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    /// Grown by `px` on every side.
    pub fn grow(&self, px: i32) -> DevRect {
        DevRect { x0: self.x0 - px, y0: self.y0 - px, x1: self.x1 + px, y1: self.y1 + px }
    }
    /// Whether the gap between the two is at most `gap` device pixels on both axes.
    pub fn near(&self, o: &DevRect, gap: i32) -> bool {
        self.x0 - gap <= o.x1 && o.x0 - gap <= self.x1 && self.y0 - gap <= o.y1 && o.y0 - gap <= self.y1
    }
    /// Clipped to a `w`×`h` pixmap.
    pub fn clamped(&self, w: u32, h: u32) -> DevRect {
        self.intersect(&DevRect { x0: 0, y0: 0, x1: w as i32, y1: h as i32 })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Cmd {
    Fill {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    Text {
        x: f32,
        y: f32,
        width: Option<f32>,
        text: String,
        spans: Vec<Span>,
        style: TextStyle,
    },
    /// An image stretched to `rect`.
    Image {
        rect: Rect,
        image: Image,
    },
    /// Line segments `[x0, y0, x1, y1]` stroked in one color.
    Lines {
        segs: Vec<[f32; 4]>,
        width: f32,
        color: Color,
    },
    /// Filled circles `[cx, cy, radius]` in one color.
    Circles {
        items: Vec<[f32; 3]>,
        color: Color,
    },
    /// A polyline whose width changes along it: `[x, y, width]` per point. A pen stroke: a mouse
    /// gives every point the same width, a stylus gives each point its own pressure.
    Stroke {
        points: Vec<[f32; 3]>,
        color: Color,
        cap: Cap,
    },
    PushClip(Rect),
    PopClip,
}

/// The end of a [`Cmd::Stroke`]. Round is what a pen draws; butt is a plain cut.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cap {
    Butt,
    #[default]
    Round,
}

impl Cmd {
    /// The logical rectangle this command can paint into: a superset, never a subset. Text is
    /// measured with the same shaped buffer the renderer will use, then padded for the ink that
    /// hangs outside the line box (ascenders, italics, underlines).
    pub fn bounds(&self, fonts: &mut Fonts) -> Rect {
        match self {
            Cmd::Fill { rect, .. } | Cmd::Image { rect, .. } => *rect,
            Cmd::Lines { segs, width, .. } => {
                let mut r = Rect::default();
                for s in segs {
                    let (x0, x1) = (s[0].min(s[2]), s[0].max(s[2]));
                    let (y0, y1) = (s[1].min(s[3]), s[1].max(s[3]));
                    let part = Rect::new(x0, y0, x1 - x0, y1 - y0);
                    let (ux0, uy0) = (r.x.min(part.x), r.y.min(part.y));
                    let (ux1, uy1) = ((r.x + r.w).max(part.x + part.w), (r.y + r.h).max(part.y + part.h));
                    r = Rect::new(ux0, uy0, ux1 - ux0, uy1 - uy0);
                }
                let pad = width * 0.5 + 1.5;
                Rect::new(r.x - pad, r.y - pad, r.w + pad * 2.0, r.h + pad * 2.0)
            }
            Cmd::Circles { items, .. } => {
                let mut r = Rect::default();
                for it in items {
                    let part = Rect::new(it[0] - it[2], it[1] - it[2], it[2] * 2.0, it[2] * 2.0);
                    let (x0, y0) = (r.x.min(part.x), r.y.min(part.y));
                    let (x1, y1) = ((r.x + r.w).max(part.x + part.w), (r.y + r.h).max(part.y + part.h));
                    r = Rect::new(x0, y0, x1 - x0, y1 - y0);
                }
                r
            }
            Cmd::Stroke { points, .. } => {
                let mut r = Rect::default();
                for p in points {
                    let half = p[2].max(1.0) * 0.5 + 1.0;
                    let part = Rect::new(p[0] - half, p[1] - half, half * 2.0, half * 2.0);
                    let (x0, y0) = (r.x.min(part.x), r.y.min(part.y));
                    let (x1, y1) = ((r.x + r.w).max(part.x + part.w), (r.y + r.h).max(part.y + part.h));
                    r = Rect::new(x0, y0, x1 - x0, y1 - y0);
                }
                r
            }
            Cmd::Text { x, y, width, text, spans, style } => {
                let (w, h) = fonts.measure(text, spans, style, *width);
                let pad = (style.size * 0.6).max(3.0);
                Rect::new(x - pad, y - pad, w + pad * 2.0, h + pad * 2.0)
            }
            // A clip call paints nothing itself, but everything inside it is bounded by it: when
            // the clip moves, the pixels that change are the ones the old and the new rect cover.
            Cmd::PushClip(r) => *r,
            Cmd::PopClip => Rect::default(),
        }
    }
}

/// Clip a segment to a rectangle (Liang-Barsky). `None` when it lies outside.
fn clip_segment(s: [f32; 4], r: &Rect) -> Option<[f32; 4]> {
    let (dx, dy) = (s[2] - s[0], s[3] - s[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, s[0] - r.x), (dx, r.x + r.w - s[0]), (-dy, s[1] - r.y), (dy, r.y + r.h - s[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some([s[0] + dx * t0, s[1] + dy * t0, s[0] + dx * t1, s[1] + dy * t1])
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Scene {
    pub cmds: Vec<Cmd>,
}

fn rounded(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let k = rad * 0.5523;
    let mut p = PathBuilder::new();
    p.move_to(x + rad, y);
    p.line_to(x + w - rad, y);
    p.cubic_to(x + w - rad + k, y, x + w, y + rad - k, x + w, y + rad);
    p.line_to(x + w, y + h - rad);
    p.cubic_to(x + w, y + h - rad + k, x + w - rad + k, y + h, x + w - rad, y + h);
    p.line_to(x + rad, y + h);
    p.cubic_to(x + rad - k, y + h, x, y + h - rad + k, x, y + h - rad);
    p.line_to(x, y + rad);
    p.cubic_to(x, y + rad - k, x + rad - k, y, x + rad, y);
    p.close();
    p.finish()
}

/// A clip rectangle in device pixels, within the pixmap.
struct DevClip {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl DevClip {
    fn new(clip: &Rect, scale: f32, pixmap: &Pixmap) -> DevClip {
        DevClip {
            x0: ((clip.x * scale).floor() as i32).max(0),
            y0: ((clip.y * scale).floor() as i32).max(0),
            x1: (((clip.x + clip.w) * scale).ceil() as i32).min(pixmap.width() as i32),
            y1: (((clip.y + clip.h) * scale).ceil() as i32).min(pixmap.height() as i32),
        }
    }
    fn within(&self, l: &DevRect) -> DevClip {
        DevClip {
            x0: self.x0.max(l.x0),
            y0: self.y0.max(l.y0),
            x1: self.x1.min(l.x1),
            y1: self.y1.min(l.y1),
        }
    }
}

/// A device rect cut down to `limit` (the stencil of a partial repaint); `None` when the result
/// holds no pixels. All the numbers are whole pixels, so cutting a rectangle this way leaves the
/// surviving pixels exactly as a full paint would.
fn within(r: Option<tiny_skia::Rect>, limit: Option<DevRect>) -> Option<tiny_skia::Rect> {
    let r = r?;
    let Some(l) = limit else { return Some(r) };
    tiny_skia::Rect::from_ltrb(
        r.left().max(l.x0 as f32),
        r.top().max(l.y0 as f32),
        r.right().min(l.x1 as f32),
        r.bottom().min(l.y1 as f32),
    )
}

/// Blend `c` over an opaque pixel with coverage `a` (0..=256).
#[inline]
fn blend(px: &mut [u8], c: [u32; 3], a: u32) {
    let ia = 256 - a;
    px[0] = ((c[0] * a + px[0] as u32 * ia) >> 8) as u8;
    px[1] = ((c[1] * a + px[1] as u32 * ia) >> 8) as u8;
    px[2] = ((c[2] * a + px[2] as u32 * ia) >> 8) as u8;
    px[3] = 255;
}

/// Run `f(band rows, band clip)` over horizontal bands of the clip, in parallel. Each band owns
/// its rows of the buffer, so shapes are rasterized without locking.
fn banded(
    data: &mut [u8],
    stride: usize,
    clip: &DevClip,
    work: usize,
    f: impl Fn(&mut [u8], &DevClip) + Sync,
) {
    if clip.x1 <= clip.x0 || clip.y1 <= clip.y0 {
        return;
    }
    let rows = (clip.y1 - clip.y0) as usize;
    // Small jobs are not worth waking the pool for. More bands than threads, so that work
    // stealing evens out bands of unequal density.
    let bands = if work < 2000 { 1 } else { (rayon::current_num_threads() * 4).clamp(1, rows.div_ceil(8)) };
    let band_h = rows.div_ceil(bands);
    let area = &mut data[clip.y0 as usize * stride * 4..clip.y1 as usize * stride * 4];
    let run = |(i, rows): (usize, &mut [u8])| {
        let y0 = clip.y0 + (i * band_h) as i32;
        f(rows, &DevClip { x0: clip.x0, x1: clip.x1, y0, y1: y0 + (rows.len() / (stride * 4)) as i32 });
    };
    if bands == 1 {
        run((0, area));
    } else {
        area.par_chunks_mut(band_h * stride * 4).enumerate().for_each(run);
    }
}

/// Anti-aliased one-pixel line (Xiaolin Wu) between device points, restricted to a band whose
/// first row is `rows[0..]`. `alpha` is 0..=256.
fn thin_line(rows: &mut [u8], stride: usize, band: &DevClip, s: [f32; 4], c: [u32; 3], alpha: f32) {
    let [mut x0, mut y0, mut x1, mut y1] = s;
    if y0.max(y1) + 1.0 < band.y0 as f32 || y0.min(y1) - 1.0 >= band.y1 as f32 {
        return;
    }
    let steep = (y1 - y0).abs() > (x1 - x0).abs();
    if steep {
        std::mem::swap(&mut x0, &mut y0);
        std::mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        std::mem::swap(&mut x0, &mut x1);
        std::mem::swap(&mut y0, &mut y1);
    }
    let grad = if x1 - x0 < 1e-3 { 0.0 } else { (y1 - y0) / (x1 - x0) };
    let (xs, xe) = (x0.round() as i32, x1.round() as i32);
    let y = y0 + grad * (xs as f32 - x0) - 0.5;
    // `u` runs along the major axis, `v` across it.
    let (u0, u1, v0, v1) =
        if steep { (band.y0, band.y1, band.x0, band.x1) } else { (band.x0, band.x1, band.y0, band.y1) };
    let (from, to) = (xs, xe + 1);
    if from >= to {
        return;
    }
    // 16.16 fixed point across the minor axis: no float rounding per pixel. The walk always starts
    // at the same end of the line and every write is filtered by the band, so a line rasterized in
    // bands (or in a dirty rect) lands on exactly the same pixels as one rasterized whole: the
    // fixed point step truncates, and starting mid-line would shift every fraction by one.
    let mut yf = (y * 65536.0) as i32;
    let step = (grad * 65536.0) as i32;
    let alpha = alpha as u32;
    for u in from..to {
        let v = yf >> 16;
        let far = (alpha * ((yf & 0xffff) as u32 >> 8)) >> 8;
        yf = yf.wrapping_add(step);
        for (v, a) in [(v, alpha - far), (v + 1, far)] {
            if v >= v0 && v < v1 && u >= u0 && u < u1 {
                let (px, py) = if steep { (v, u) } else { (u, v) };
                let o = ((py - band.y0) as usize * stride + px as usize) * 4;
                blend(&mut rows[o..o + 4], c, a);
            }
        }
    }
}

/// A device-space mask for a logical rectangle: how a filled path is kept inside a clip.
fn rect_mask(w: u32, h: u32, r: Rect, scale: f32) -> Option<tiny_skia::Mask> {
    let path = tiny_skia::Rect::from_xywh(r.x * scale, r.y * scale, r.w * scale, r.h * scale)
        .map(PathBuilder::from_rect)?;
    let mut m = tiny_skia::Mask::new(w, h)?;
    m.fill_path(&path, FillRule::Winding, false, Transform::identity());
    Some(m)
}

/// The outline of a polyline whose width changes from point to point: one quad per segment and a
/// disc on every joint, filled as a single path so a translucent stroke is composited once. The
/// ends get a disc only when the cap is round.
fn ribbon(points: &[[f32; 3]], cap: Cap) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.01 {
            continue;
        }
        let (nx, ny) = (-dy / len, dx / len);
        let (ra, rb) = (a[2].max(0.3) * 0.5, b[2].max(0.3) * 0.5);
        pb.move_to(a[0] + nx * ra, a[1] + ny * ra);
        pb.line_to(b[0] + nx * rb, b[1] + ny * rb);
        pb.line_to(b[0] - nx * rb, b[1] - ny * rb);
        pb.line_to(a[0] - nx * ra, a[1] - ny * ra);
        pb.close();
    }
    let n = points.len();
    let range = if cap == Cap::Round { 0..n } else { 1..n.saturating_sub(1) };
    for p in &points[range] {
        if p[2] > 0.2 {
            push_disc(&mut pb, p[0], p[1], p[2] * 0.5);
        }
    }
    pb.finish()
}

/// A disc wound the same way as the quads of [`ribbon`]. `PathBuilder::push_circle` winds the
/// other way, and filling the union with the winding rule would cancel the joint instead of
/// filling it: that showed up as a stroke that vanished on its last centimetre.
fn push_disc(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32) {
    let steps = (r * 2.0).clamp(10.0, 40.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32 * std::f32::consts::TAU;
        let (x, y) = (cx + r * t.cos(), cy - r * t.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
}

/// Anti-aliased filled circle in device pixels, restricted to a band.
fn disc(rows: &mut [u8], stride: usize, band: &DevClip, [cx, cy, r]: [f32; 3], c: [u32; 3], alpha: f32) {
    if r <= 0.0 {
        return;
    }
    let (x0, x1) = (((cx - r).floor() as i32).max(band.x0), ((cx + r).ceil() as i32 + 1).min(band.x1));
    let (y0, y1) = (((cy - r).floor() as i32).max(band.y0), ((cy + r).ceil() as i32 + 1).min(band.y1));
    let inner = (r - 0.5).max(0.0);
    let inner2 = inner * inner;
    for py in y0..y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let d2 = dx * dx + dy * dy;
            let cov = if d2 <= inner2 { 1.0 } else { (r + 0.5 - d2.sqrt()).clamp(0.0, 1.0) };
            if cov > 0.0 {
                let o = ((py - band.y0) as usize * stride + px as usize) * 4;
                blend(&mut rows[o..o + 4], c, (alpha * cov) as u32);
            }
        }
    }
}

/// Rasterize a scene. `scale` is the device pixel ratio; scene coordinates are logical pixels.
pub fn render(scene: &Scene, fonts: &mut Fonts, pixmap: &mut Pixmap, scale: f32, clear: Color) {
    paint(scene, fonts, pixmap, scale, clear, None);
}

/// Rasterize a scene but only inside `limit`: the rest of the pixmap is left as it is, and every
/// command is clipped to the rectangle as if it were a stencil. The pixels inside `limit` come out
/// exactly as a full [`render`] would leave them, which is what makes dirty-rect painting safe.
pub(crate) fn paint(
    scene: &Scene,
    fonts: &mut Fonts,
    pixmap: &mut Pixmap,
    scale: f32,
    clear: Color,
    limit: Option<DevRect>,
) {
    match limit {
        None => pixmap.fill(tiny_skia::Color::from_rgba8(clear.r, clear.g, clear.b, 255)),
        Some(l) => {
            let mut paint = Paint::default();
            paint.set_color_rgba8(clear.r, clear.g, clear.b, 255);
            paint.anti_alias = false;
            let w = (l.x1 - l.x0).max(0) as f32;
            let h = (l.y1 - l.y0).max(0) as f32;
            if let Some(r) = tiny_skia::Rect::from_xywh(l.x0 as f32, l.y0 as f32, w, h) {
                pixmap.fill_rect(r, &paint, Transform::identity(), None);
            }
        }
    }
    // The stencil, in device pixels, that every drawing call is restricted to.
    let stencil = limit.and_then(|l| {
        let mut m = tiny_skia::Mask::new(pixmap.width(), pixmap.height())?;
        let path = tiny_skia::Rect::from_ltrb(l.x0 as f32, l.y0 as f32, l.x1 as f32, l.y1 as f32)
            .map(PathBuilder::from_rect)?;
        m.fill_path(&path, FillRule::Winding, false, Transform::identity());
        Some(m)
    });
    let full = Rect::new(0.0, 0.0, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
    let mut clips = vec![full];
    for cmd in &scene.cmds {
        let clip = *clips.last().unwrap_or(&full);
        match cmd {
            Cmd::PushClip(r) => clips.push(clip.intersect(r)),
            Cmd::PopClip => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
            Cmd::Fill { rect, color, radius } => {
                if color.a == 0 {
                    continue;
                }
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r, color.g, color.b, color.a);
                if *radius > 0.5 {
                    // Rounded shapes are only clipped by rectangle rejection: draw when the
                    // shape is at least partly visible, clipped through a mask-free intersect.
                    let vis = rect.intersect(&clip);
                    if vis.w <= 0.0 || vis.h <= 0.0 {
                        continue;
                    }
                    paint.anti_alias = true;
                    if vis == *rect {
                        if let Some(path) = rounded(*rect, *radius) {
                            pixmap.fill_path(
                                &path,
                                &paint,
                                FillRule::Winding,
                                Transform::from_scale(scale, scale),
                                stencil.as_ref(),
                            );
                        }
                        continue;
                    }
                }
                let r = rect.intersect(&clip);
                paint.anti_alias = false;
                let tr = tiny_skia::Rect::from_xywh(
                    (r.x * scale).round(),
                    (r.y * scale).round(),
                    (r.w * scale).round(),
                    (r.h * scale).round(),
                );
                if let Some(tr) = within(tr, limit) {
                    pixmap.fill_rect(tr, &paint, Transform::identity(), None);
                }
            }
            Cmd::Lines { segs, width, color } => {
                if color.a == 0 || clip.w <= 0.0 || clip.h <= 0.0 {
                    continue;
                }
                let mut dev = DevClip::new(&clip, scale, pixmap);
                if let Some(l) = limit {
                    dev = dev.within(&l);
                    if dev.x1 <= dev.x0 || dev.y1 <= dev.y0 {
                        continue;
                    }
                }
                if *width * scale <= 1.5 {
                    // Thin lines are drawn straight into the buffer: a path stroke of a whole
                    // graph costs seconds, this costs milliseconds.
                    let alpha = color.a as f32 * (256.0 / 255.0) * (*width * scale).min(1.0);
                    let rgb = [color.r as u32, color.g as u32, color.b as u32];
                    let stride = pixmap.width() as usize;
                    let lines: Vec<[f32; 4]> = segs
                        .iter()
                        .filter_map(|s| clip_segment(*s, &clip))
                        .map(|c| c.map(|v| v * scale))
                        .collect();
                    banded(pixmap.data_mut(), stride, &dev, lines.len(), |rows, band| {
                        for l in &lines {
                            thin_line(rows, stride, band, *l, rgb, alpha);
                        }
                    });
                    continue;
                }
                let mut pb = PathBuilder::new();
                for s in segs {
                    if let Some(c) = clip_segment(*s, &clip) {
                        pb.move_to(c[0], c[1]);
                        pb.line_to(c[2], c[3]);
                    }
                }
                let Some(path) = pb.finish() else { continue };
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r, color.g, color.b, color.a);
                paint.anti_alias = true;
                let stroke = tiny_skia::Stroke { width: *width, ..Default::default() };
                pixmap.stroke_path(
                    &path,
                    &paint,
                    &stroke,
                    Transform::from_scale(scale, scale),
                    stencil.as_ref(),
                );
            }
            Cmd::Circles { items, color } => {
                if color.a == 0 {
                    continue;
                }
                let mut dev = DevClip::new(&clip, scale, pixmap);
                if let Some(l) = limit {
                    dev = dev.within(&l);
                    if dev.x1 <= dev.x0 || dev.y1 <= dev.y0 {
                        continue;
                    }
                }
                let stride = pixmap.width() as usize;
                let alpha = color.a as f32 * (256.0 / 255.0);
                let rgb = [color.r as u32, color.g as u32, color.b as u32];
                banded(pixmap.data_mut(), stride, &dev, items.len(), |rows, band| {
                    for it in items {
                        let (y, r) = (it[1] * scale, it[2] * scale + 1.0);
                        if y + r >= band.y0 as f32 && y - r < band.y1 as f32 {
                            disc(rows, stride, band, it.map(|v| v * scale), rgb, alpha);
                        }
                    }
                });
            }
            Cmd::Stroke { points, color, cap } => {
                if color.a == 0 || clip.w <= 0.0 || clip.h <= 0.0 || points.len() < 2 {
                    continue;
                }
                let Some(path) = ribbon(points, *cap) else { continue };
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r, color.g, color.b, color.a);
                paint.anti_alias = true;
                // A stroke is a filled shape, so it is kept inside the pane through a mask: cutting
                // it segment by segment would make it jump at the edge.
                let mask = if clip == full {
                    None
                } else {
                    rect_mask(pixmap.width(), pixmap.height(), clip, scale)
                };
                pixmap.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::from_scale(scale, scale),
                    mask.as_ref().or(stencil.as_ref()),
                );
            }
            Cmd::Image { rect, image } => {
                let vis = rect.intersect(&clip);
                let (iw, ih) = image.size();
                if vis.w <= 0.0 || vis.h <= 0.0 || iw == 0 || ih == 0 {
                    continue;
                }
                let mask = if limit.is_some() || vis != *rect {
                    let clip_path =
                        tiny_skia::Rect::from_xywh(vis.x, vis.y, vis.w, vis.h).map(PathBuilder::from_rect);
                    let mut m = tiny_skia::Mask::new(pixmap.width(), pixmap.height());
                    if let (Some(m), Some(path)) = (m.as_mut(), clip_path) {
                        m.fill_path(&path, FillRule::Winding, false, Transform::from_scale(scale, scale));
                        if let Some(l) = limit {
                            let stencil = tiny_skia::Rect::from_ltrb(
                                l.x0 as f32,
                                l.y0 as f32,
                                l.x1 as f32,
                                l.y1 as f32,
                            )
                            .map(PathBuilder::from_rect);
                            if let Some(p) = stencil {
                                m.intersect_path(&p, FillRule::Winding, false, Transform::identity());
                            }
                        }
                    }
                    m
                } else {
                    None
                };
                let (sx, sy) = (rect.w * scale / iw as f32, rect.h * scale / ih as f32);
                let paint = tiny_skia::PixmapPaint {
                    quality: if (sx - 1.0).abs() < 0.01 && (sy - 1.0).abs() < 0.01 {
                        tiny_skia::FilterQuality::Nearest
                    } else {
                        tiny_skia::FilterQuality::Bilinear
                    },
                    ..Default::default()
                };
                let tf = Transform::from_scale(sx, sy).post_translate(rect.x * scale, rect.y * scale);
                pixmap.draw_pixmap(0, 0, image.0.as_ref().as_ref(), &paint, tf, mask.as_ref());
            }
            Cmd::Text { x, y, width, text, spans, style } => {
                let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
                let (mut cx0, mut cy0) = ((clip.x * scale).floor() as i32, (clip.y * scale).floor() as i32);
                let (mut cx1, mut cy1) = (
                    (((clip.x + clip.w) * scale).ceil() as i32).min(pw),
                    (((clip.y + clip.h) * scale).ceil() as i32).min(ph),
                );
                if let Some(l) = limit {
                    cx0 = cx0.max(l.x0);
                    cy0 = cy0.max(l.y0);
                    cx1 = cx1.min(l.x1);
                    cy1 = cy1.min(l.y1);
                }
                if cx1 <= cx0 || cy1 <= cy0 {
                    continue;
                }
                let scaled = TextStyle {
                    size: style.size * scale,
                    line_height: style.line_height * scale,
                    ..style.clone()
                };
                let (ox, oy) = ((x * scale).round() as i32, (y * scale).round() as i32);
                let stride = pw as usize;
                let data = pixmap.data_mut();
                fonts.draw(text, spans, &scaled, width.map(|w| w * scale), |gx, gy, gw, gh, c| {
                    if c.a == 0 {
                        return;
                    }
                    let a = c.a as u32;
                    for py in (oy + gy).max(cy0).max(0)..(oy + gy + gh as i32).min(cy1) {
                        for px in (ox + gx).max(cx0).max(0)..(ox + gx + gw as i32).min(cx1) {
                            let o = (py as usize * stride + px as usize) * 4;
                            data[o] = ((c.r as u32 * a + data[o] as u32 * (255 - a)) / 255) as u8;
                            data[o + 1] = ((c.g as u32 * a + data[o + 1] as u32 * (255 - a)) / 255) as u8;
                            data[o + 2] = ((c.b as u32 * a + data[o + 2] as u32 * (255 - a)) / 255) as u8;
                            data[o + 3] = 255;
                        }
                    }
                });
            }
        }
    }
}
