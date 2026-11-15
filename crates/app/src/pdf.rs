//! PDF export. The note is laid out the way the reading view shows it, in the light theme at a
//! fixed width, and the resulting display list is written as vector pages with krilla: text stays
//! selectable and searchable, and fonts are embedded as subsets.

use crate::shell::{App, ViewMode};
use crate::view;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect as KRect, Size, Transform};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::Document;
use rustsidian_ui::{Cmd, Color, Dom, El, FontRef, Fonts, Rect};
use std::collections::HashMap;
use std::path::Path;
use theme_engine::Env;

/// A4 in points.
const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
/// Top and bottom page margins in points; the sides come from the blocks' own padding.
const MARGIN_Y: f32 = 48.0;
/// Layout width in CSS pixels, scaled to the page width.
const LAYOUT_W: f32 = 760.0;
/// Tall enough for any note; only the part with content becomes pages.
const LAYOUT_H: f32 = 1_000_000.0;

/// Write the active note to `dest`. Returns the number of pages.
pub fn export(app: &mut App, fonts: &mut Fonts, dest: &Path) -> Result<usize, String> {
    let dark = std::mem::replace(&mut app.dark, false);
    let result = write(app, fonts, dest);
    app.dark = dark;
    result
}

impl App {
    /// Export the active note to a PDF of the same name beside it in the vault.
    pub(crate) fn export_pdf(&mut self) {
        let Some(rel) = self.tab().map(|t| t.path.clone()) else { return };
        let pdf = format!("{}.pdf", vault_core::path::strip_md(&rel));
        let dest = self.store.abs(&pdf);
        let mut fonts = Fonts::new();
        self.notice = Some(match export(self, &mut fonts, &dest) {
            Ok(n) => {
                self.store.refresh(&pdf);
                self.tree = None;
                format!("Exported {pdf} ({n} page{})", if n == 1 { "" } else { "s" })
            }
            Err(e) => format!("Cannot export to PDF: {e}"),
        });
    }
}

fn write(app: &App, fonts: &mut Fonts, dest: &Path) -> Result<usize, String> {
    let tab = app.tab().ok_or("no note is open")?;
    let pal = view::palette(app);
    let mut info = view::ViewInfo::default();
    let mut dom = Dom::new();
    dom.nodes[0].el.classes.push("theme-light".to_string());
    let content = dom.add(0, El::div().class("view-content markdown-reading-view"));
    let blocks = tab.editor.buf.doc().blocks.len();
    view::note_blocks(app, &mut dom, content, &pal, &mut info, tab, ViewMode::Reading, 0..blocks, true);
    dom.style(&app.theme, Env::default());
    dom.layout(fonts, LAYOUT_W, LAYOUT_H);
    view::add_decorations(&mut dom, &info, fonts);
    crate::rich::paint(&mut dom, &info.rich, fonts);
    let cmds = dom.paint().cmds;

    let scale = PAGE_W / LAYOUT_W;
    let page_h = (PAGE_H - 2.0 * MARGIN_Y) / scale;
    let tops: Vec<f32> = dom.nodes[content].children.iter().map(|&c| dom.nodes[c].rect.y).collect();
    let end = dom.nodes[content]
        .children
        .iter()
        .map(|&c| dom.nodes[c].rect.y + dom.nodes[c].rect.h)
        .fold(0.0f32, f32::max);
    let pages = paginate(&tops, tops.first().copied().unwrap_or(0.0), end, page_h);

    let extents: Vec<Option<(f32, f32)>> = cmds.iter().map(|c| extent(c, fonts)).collect();
    let mut doc = Document::new();
    doc.set_metadata(Metadata::new().title(tab.title().to_string()).creator("Rustsidian".to_string()));
    let mut out = Out { fonts, font_cache: HashMap::new(), images: HashMap::new() };
    for &(top, bottom) in &pages {
        let settings = PageSettings::from_wh(PAGE_W, PAGE_H).ok_or("bad page size")?;
        let mut page = doc.start_page_with(settings);
        let mut s = page.surface();
        s.push_transform(&Transform::from_row(scale, 0.0, 0.0, scale, 0.0, MARGIN_Y - top * scale));
        let clip = Rect::new(0.0, top, LAYOUT_W, bottom - top);
        push_clip(&mut s, clip);
        for (cmd, b) in cmds.iter().zip(&extents) {
            if b.is_none_or(|(y0, y1)| y0 < bottom && y1 > top) {
                out.cmd(&mut s, cmd)?;
            }
        }
        s.pop();
        s.pop();
        s.finish();
        page.finish();
    }
    let bytes = doc.finish().map_err(|e| format!("{e:?}"))?;
    vault_core::fsio::atomic_write(dest, &bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(pages.len())
}

/// Page ranges `(top, bottom)` in layout pixels. A page ends where the last block that starts on
/// it begins; a block taller than a page is cut at the page edge.
fn paginate(tops: &[f32], start: f32, end: f32, page_h: f32) -> Vec<(f32, f32)> {
    let mut pages = Vec::new();
    let mut top = start;
    while top < end - 0.5 {
        let limit = top + page_h;
        let bottom = if limit >= end {
            end
        } else {
            tops.iter().copied().filter(|&t| t > top + 1.0 && t <= limit).fold(limit, |_, t| t)
        };
        pages.push((top, bottom));
        top = bottom;
    }
    if pages.is_empty() {
        pages.push((start, start + page_h));
    }
    pages
}

/// The vertical span a command paints. Clips have none and are always kept so pushes and pops
/// stay paired.
fn extent(cmd: &Cmd, fonts: &mut Fonts) -> Option<(f32, f32)> {
    let span = |ys: &mut dyn Iterator<Item = f32>| {
        ys.fold(None, |acc: Option<(f32, f32)>, y| Some(acc.map_or((y, y), |(a, b)| (a.min(y), b.max(y)))))
    };
    match cmd {
        Cmd::Fill { rect, .. } | Cmd::Image { rect, .. } => Some((rect.y, rect.y + rect.h)),
        Cmd::Text { y, width, text, spans, style, .. } => {
            let (_, h) = fonts.measure(text, spans, style, *width);
            Some((*y, y + h))
        }
        Cmd::Lines { segs, width, .. } => {
            span(&mut segs.iter().flat_map(|s| [s[1], s[3]])).map(|(a, b)| (a - width, b + width))
        }
        Cmd::Circles { items, .. } => span(&mut items.iter().flat_map(|c| [c[1] - c[2], c[1] + c[2]])),
        Cmd::PushClip(_) | Cmd::PopClip => None,
    }
}

struct Out<'a> {
    fonts: &'a mut Fonts,
    font_cache: HashMap<FontRef, Option<Font>>,
    images: HashMap<usize, krilla::image::Image>,
}

impl Out<'_> {
    fn cmd(&mut self, s: &mut Surface, cmd: &Cmd) -> Result<(), String> {
        match cmd {
            Cmd::Fill { rect, color, radius } => {
                if let Some(path) = rounded(*rect, *radius) {
                    s.set_fill(Some(fill(*color)));
                    s.set_stroke(None);
                    s.draw_path(&path);
                }
            }
            Cmd::Text { x, y, width, text, spans, style } => {
                for run in self.fonts.glyph_runs(text, spans, style, *width) {
                    let Some(first) = run.glyphs.first() else { continue };
                    let font = match self.font_cache.get(&run.font) {
                        Some(f) => f.clone(),
                        None => {
                            let f = self
                                .fonts
                                .font_data(run.font)
                                .and_then(|(data, index)| Font::new(data.into(), index));
                            self.font_cache.insert(run.font, f.clone());
                            f
                        }
                    };
                    // A font krilla cannot read is skipped rather than failing the whole export.
                    let Some(font) = font else { continue };
                    let glyphs: Vec<KrillaGlyph> = run
                        .glyphs
                        .iter()
                        .enumerate()
                        .map(|(i, g)| {
                            let advance = run.glyphs.get(i + 1).map_or(g.advance, |n| n.x - g.x);
                            KrillaGlyph::new(
                                GlyphId::new(g.id as u32),
                                advance / run.size,
                                0.0,
                                g.y_offset / run.size,
                                0.0,
                                g.text.clone(),
                                None,
                            )
                        })
                        .collect();
                    s.set_fill(Some(fill(run.color)));
                    s.set_stroke(None);
                    let slant = if run.synthetic_italic { -(14f32.to_radians().tan()) } else { 0.0 };
                    let origin = Transform::from_row(1.0, 0.0, slant, 1.0, x + first.x, y + run.baseline);
                    s.push_transform(&origin);
                    s.draw_glyphs(Point::from_xy(0.0, 0.0), &glyphs, font, &run.text, run.size, false);
                    s.pop();
                }
            }
            Cmd::Image { rect, image } => {
                let key = std::sync::Arc::as_ptr(&image.0) as usize;
                let img = self
                    .images
                    .entry(key)
                    .or_insert_with(|| {
                        let pm = &image.0;
                        let rgba = pm
                            .pixels()
                            .iter()
                            .flat_map(|p| {
                                let c = p.demultiply();
                                [c.red(), c.green(), c.blue(), c.alpha()]
                            })
                            .collect();
                        krilla::image::Image::from_rgba8(rgba, pm.width(), pm.height())
                    })
                    .clone();
                let Some(size) = Size::from_wh(rect.w, rect.h) else { return Ok(()) };
                s.push_transform(&Transform::from_translate(rect.x, rect.y));
                s.draw_image(img, size);
                s.pop();
            }
            Cmd::Lines { segs, width, color } => {
                let mut pb = PathBuilder::new();
                for seg in segs {
                    pb.move_to(seg[0], seg[1]);
                    pb.line_to(seg[2], seg[3]);
                }
                if let Some(path) = pb.finish() {
                    s.set_fill(None);
                    s.set_stroke(Some(Stroke {
                        paint: to_rgb(*color).into(),
                        width: *width,
                        opacity: alpha(*color),
                        ..Stroke::default()
                    }));
                    s.draw_path(&path);
                    s.set_stroke(None);
                }
            }
            Cmd::Circles { items, color } => {
                let mut pb = PathBuilder::new();
                for &[cx, cy, r] in items {
                    circle(&mut pb, cx, cy, r);
                }
                if let Some(path) = pb.finish() {
                    s.set_fill(Some(fill(*color)));
                    s.set_stroke(None);
                    s.draw_path(&path);
                }
            }
            Cmd::PushClip(r) => push_clip(s, *r),
            Cmd::PopClip => s.pop(),
        }
        Ok(())
    }
}

fn to_rgb(c: Color) -> rgb::Color {
    rgb::Color::new(c.r, c.g, c.b)
}

fn alpha(c: Color) -> NormalizedF32 {
    NormalizedF32::new(c.a as f32 / 255.0).unwrap_or(NormalizedF32::ONE)
}

fn fill(c: Color) -> Fill {
    Fill { paint: to_rgb(c).into(), opacity: alpha(c), rule: FillRule::NonZero }
}

fn push_clip(s: &mut Surface, r: Rect) {
    let path = KRect::from_xywh(r.x, r.y, r.w.max(0.0), r.h.max(0.0)).and_then(|kr| {
        let mut pb = PathBuilder::new();
        pb.push_rect(kr);
        pb.finish()
    });
    match path {
        Some(p) => s.push_clip_path(&p, &FillRule::NonZero),
        // An empty clip still needs a push so the matching pop stays balanced.
        None => s.push_transform(&Transform::identity()),
    }
}

/// Kappa for approximating a quarter circle with one cubic.
const K: f32 = 0.552_284_8;

fn rounded(r: Rect, radius: f32) -> Option<krilla::geom::Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    if rad < 0.5 {
        pb.push_rect(KRect::from_xywh(r.x, r.y, r.w, r.h)?);
        return pb.finish();
    }
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let k = rad * K;
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

fn circle(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32) {
    let k = r * K;
    pb.move_to(cx + r, cy);
    pb.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    pb.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    pb.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    pb.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    pb.close();
}

#[cfg(test)]
mod tests {
    use super::paginate;

    #[test]
    fn pages_break_between_blocks() {
        // Blocks start every 300px; a page holds 700px, so two blocks fit per page.
        let tops = [0.0, 300.0, 600.0, 900.0, 1200.0];
        assert_eq!(paginate(&tops, 0.0, 1400.0, 700.0), [(0.0, 600.0), (600.0, 1200.0), (1200.0, 1400.0)]);
    }

    #[test]
    fn a_tall_block_is_cut_at_the_page_edge() {
        assert_eq!(
            paginate(&[0.0, 1500.0], 0.0, 1600.0, 700.0),
            [(0.0, 700.0), (700.0, 1400.0), (1400.0, 1600.0)]
        );
    }
}
