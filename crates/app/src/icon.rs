//! The application icon, drawn procedurally so no image asset or decoder is needed.
//! Shared by the build script (Windows resource) and the app (window icon, welcome screen).
//!
//! `packaging/metamind.svg` is the source of truth for the drawing; this file reproduces it in
//! unit coordinates so every size is crisp and the binary carries no bitmap.

type P = (f32, f32);
type Rgb = [f32; 3];

/// Inset of the rounded square: 8/256.
const INSET: f32 = 0.031_25;
/// Corner radius: 56/256.
const RADIUS: f32 = 0.218_75;
/// Border: #2a2a4a, 2/256 wide.
const BORDER: Rgb = [42.0, 42.0, 74.0];
const BORDER_W: f32 = 1.0 / 256.0;
/// Background gradient, top-left to bottom-right: #1a1a2e to #16213e.
const BG_A: Rgb = [26.0, 26.0, 46.0];
const BG_B: Rgb = [22.0, 33.0, 62.0];
/// Edges: #3d5a80 at 0.6, 2.5/256 wide.
const EDGE: Rgb = [61.0, 90.0, 128.0];
const EDGE_ALPHA: f32 = 0.6;
const EDGE_W: f32 = 1.25 / 256.0;
/// Node colours: the centre node is the blue-to-green gradient, the hemispheres take one end of
/// it and the two poles are gold.
const NODE_A: Rgb = [122.0, 162.0, 247.0];
const NODE_B: Rgb = [76.0, 175.0, 120.0];
const GOLD: Rgb = [232.0, 194.0, 104.0];

/// A node of the graph: where it sits, how big it is, its colour and how solid it is.
struct Node {
    at: P,
    r: f32,
    color: Rgb,
    /// `false` for the two small hemisphere nodes, which are drawn at 0.7 opacity.
    solid: bool,
    /// `true` for the centre, which carries the blue-to-green gradient.
    gradient: bool,
}

const NODES: [Node; 9] = [
    Node { at: (0.5, 0.5), r: 14.0 / 256.0, color: NODE_A, solid: true, gradient: true },
    // Left hemisphere.
    Node { at: (80.0 / 256.0, 90.0 / 256.0), r: 8.0 / 256.0, color: NODE_A, solid: true, gradient: false },
    Node { at: (72.0 / 256.0, 128.0 / 256.0), r: 6.0 / 256.0, color: NODE_A, solid: false, gradient: false },
    Node { at: (84.0 / 256.0, 170.0 / 256.0), r: 8.0 / 256.0, color: NODE_A, solid: true, gradient: false },
    // Right hemisphere.
    Node { at: (176.0 / 256.0, 90.0 / 256.0), r: 8.0 / 256.0, color: NODE_B, solid: true, gradient: false },
    Node { at: (184.0 / 256.0, 128.0 / 256.0), r: 6.0 / 256.0, color: NODE_B, solid: false, gradient: false },
    Node { at: (172.0 / 256.0, 170.0 / 256.0), r: 8.0 / 256.0, color: NODE_B, solid: true, gradient: false },
    // Poles.
    Node { at: (0.5, 64.0 / 256.0), r: 7.0 / 256.0, color: GOLD, solid: true, gradient: false },
    Node { at: (0.5, 192.0 / 256.0), r: 7.0 / 256.0, color: GOLD, solid: true, gradient: false },
];

/// The edges of the graph, as indices into [`NODES`].
const EDGES: [(usize, usize); 12] =
    [(0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (0, 7), (0, 8), (1, 7), (4, 7), (3, 8), (6, 8)];

/// Two faint rings around the centre, `(radius, opacity)`.
const GLOW: [(f32, f32); 2] = [(40.0 / 256.0, 0.15), (60.0 / 256.0, 0.08)];

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Puts `src` over `dst` with opacity `a`.
fn over(dst: Rgb, src: Rgb, a: f32) -> Rgb {
    mix(dst, src, a)
}

fn dist(a: P, b: P) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Distance from `p` to the segment `a`-`b`.
fn dist_segment(p: P, a: P, b: P) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    if len2 <= f32::EPSILON {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + abx * t, a.1 + aby * t))
}

/// Signed distance to the rounded square: negative inside.
fn rounded_square(p: P) -> f32 {
    let half = 0.5 - INSET;
    let c = (0.5, 0.5);
    let q = ((p.0 - c.0).abs() - (half - RADIUS), (p.1 - c.1).abs() - (half - RADIUS));
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - RADIUS
}

/// The colour at a point of the unit square, or `None` outside the rounded background.
///
/// `detailed` is false for the small sizes: under 64 px the two faint hemisphere nodes and the
/// glow rings turn into noise and the edges and nodes have to be thicker to be seen at all.
fn sample(x: f32, y: f32, detailed: bool) -> Option<Rgb> {
    let p = (x, y);
    if rounded_square(p) > 0.0 {
        return None;
    }
    // Background, then its border.
    let t = ((x - INSET) / (1.0 - 2.0 * INSET) + (y - INSET) / (1.0 - 2.0 * INSET)) / 2.0;
    let mut c = mix(BG_A, BG_B, t);
    let border = rounded_square(p) + BORDER_W;
    if border > -BORDER_W {
        let a = ((BORDER_W - border) / (2.0 * BORDER_W)).clamp(0.0, 1.0);
        c = over(c, BORDER, a);
    }
    // Edges.
    let edge_w = if detailed { EDGE_W } else { EDGE_W * 1.8 };
    for (a, b) in EDGES {
        let d = dist_segment(p, NODES[a].at, NODES[b].at) - edge_w;
        if d < edge_w {
            let cover = ((edge_w - d) / (2.0 * edge_w)).clamp(0.0, 1.0);
            c = over(c, EDGE, EDGE_ALPHA * cover);
        }
    }
    // The faint rings are the innermost decoration: they sit under the nodes.
    let centre = NODES[0].at;
    if detailed {
        for (radius, alpha) in GLOW {
            let d = (dist(p, centre) - radius).abs();
            if d < 0.5 / 256.0 {
                c = over(c, NODE_A, alpha);
            }
        }
    }
    // Nodes last, so they cover the edges that meet them.
    for node in &NODES {
        if !detailed && !node.solid {
            continue;
        }
        let r = if detailed { node.r } else { node.r * 1.3 };
        let d = dist(p, node.at) - r;
        if d < 0.0 {
            let base = if node.gradient {
                let t = ((x - (node.at.0 - r)) / (2.0 * r) + (y - (node.at.1 - r)) / (2.0 * r)) / 2.0;
                mix(NODE_A, NODE_B, t)
            } else {
                node.color
            };
            let alpha = if node.solid { 1.0 } else { 0.7 };
            c = over(c, base, alpha);
        }
    }
    Some(c)
}

/// The icon as straight (non-premultiplied) RGBA, `size` x `size`, antialiased.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    // Under 64 px the drawing keeps only what a 32 px dock icon can show.
    let detailed = size >= 64;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut acc, mut hits) = ([0u32; 3], 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if let Some(c) = sample(x, y, detailed) {
                        hits += 1;
                        for (a, v) in acc.iter_mut().zip(c) {
                            *a += v as u32;
                        }
                    }
                }
            }
            let n = hits.max(1);
            out.extend([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8]);
            out.push((hits * 255 / (SS * SS)) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corners_are_clear_and_the_centre_node_carries_the_gradient() {
        let px = rgba(256);
        let at = |x: u32, y: u32| {
            let i = ((y * 256 + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(0, 0)[3], 0, "a corner is outside the rounded square");
        assert_eq!(at(128, 128)[3], 255, "the centre is opaque");
        // The centre node goes from blue (top-left) to green (bottom-right).
        let tl = at(120, 120);
        let br = at(136, 136);
        assert!(tl[2] > tl[1], "the top-left of the node leans blue: {tl:?}");
        assert!(br[1] > br[0] && br[1] > 150, "the bottom-right leans green: {br:?}");
        // The top pole is gold.
        let pole = at(128, 64);
        assert!(pole[0] > 200 && pole[2] < 140, "the top pole is gold: {pole:?}");
    }
}
