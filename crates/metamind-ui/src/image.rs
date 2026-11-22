//! Decoding PNG, JPEG, GIF, WebP and SVG files into pixmaps. The format is taken from the file's
//! first bytes, not its name. GIF shows its first frame.

use crate::scene::Image;
use std::io::Cursor;
use std::sync::{Arc, OnceLock};
use tiny_skia::{IntSize, Pixmap, Transform};

/// Larger images are refused before their pixels are allocated.
const MAX_DECODE_SIDE: u32 = 16384;

impl Image {
    /// Decode an image, scaled down if needed so that neither side exceeds `max_side` pixels.
    pub fn decode(bytes: &[u8], max_side: u32) -> Option<Image> {
        let p = if bytes.starts_with(b"\x89PNG") {
            Pixmap::decode_png(bytes).ok()?
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            jpeg(bytes)?
        } else if bytes.starts_with(b"GIF8") {
            gif(bytes)?
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            webp(bytes)?
        } else if is_svg(bytes) {
            return Some(Image(Arc::new(svg(bytes, max_side)?)));
        } else {
            return None;
        };
        Some(Image(Arc::new(fit(p, max_side)?)))
    }
}

fn fit(p: Pixmap, max_side: u32) -> Option<Pixmap> {
    let side = p.width().max(p.height());
    if side <= max_side.max(1) {
        return Some(p);
    }
    let k = max_side.max(1) as f32 / side as f32;
    let (w, h) = (((p.width() as f32 * k) as u32).max(1), ((p.height() as f32 * k) as u32).max(1));
    let mut out = Pixmap::new(w, h)?;
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    out.draw_pixmap(0, 0, p.as_ref(), &paint, Transform::from_scale(k, k), None);
    Some(out)
}

/// A pixmap from straight-alpha RGBA rows.
fn from_rgba(mut rgba: Vec<u8>, w: u32, h: u32) -> Option<Pixmap> {
    for i in (0..rgba.len() / 4 * 4).step_by(4) {
        let a = u16::from(rgba[i + 3]);
        if a < 255 {
            for c in &mut rgba[i..i + 3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
    }
    Pixmap::from_vec(rgba, IntSize::from_wh(w, h)?)
}

fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    (0..rgb.len() / 3).flat_map(|i| [rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], 255]).collect()
}

fn jpeg(bytes: &[u8]) -> Option<Pixmap> {
    use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_DECODE_SIDE as usize)
        .set_max_height(MAX_DECODE_SIDE as usize);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    let px = d.decode().ok()?;
    let info = d.info()?;
    from_rgba(px, u32::from(info.width), u32::from(info.height))
}

fn gif(bytes: &[u8]) -> Option<Pixmap> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut d = opts.read_info(Cursor::new(bytes)).ok()?;
    let (w, h) = (u32::from(d.width()), u32::from(d.height()));
    let frame = d.read_next_frame().ok()??;
    // The first frame may cover only part of the logical screen; the rest stays transparent.
    let mut canvas = vec![0u8; (w * h * 4) as usize];
    let (fw, fl, ft) = (usize::from(frame.width), usize::from(frame.left), usize::from(frame.top));
    for (y, row) in frame.buffer.chunks_exact(fw * 4).enumerate() {
        let cy = ft + y;
        if cy >= h as usize || fl >= w as usize {
            continue;
        }
        let n = fw.min(w as usize - fl) * 4;
        let at = (cy * w as usize + fl) * 4;
        canvas[at..at + n].copy_from_slice(&row[..n]);
    }
    from_rgba(canvas, w, h)
}

fn webp(bytes: &[u8]) -> Option<Pixmap> {
    let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (w, h) = d.dimensions();
    if w > MAX_DECODE_SIDE || h > MAX_DECODE_SIDE {
        return None;
    }
    let mut buf = vec![0u8; d.output_buffer_size()?];
    d.read_image(&mut buf).ok()?;
    let rgba = if d.has_alpha() { buf } else { rgb_to_rgba(&buf) };
    from_rgba(rgba, w, h)
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with('<') && text.contains("<svg")
}

/// Fonts for text inside SVG files, loaded the first time one is drawn.
fn svg_fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Draw an SVG at its own size, or smaller so that neither side exceeds `max_side`.
fn svg(bytes: &[u8], max_side: u32) -> Option<Pixmap> {
    let opts = resvg::usvg::Options { fontdb: svg_fonts(), ..Default::default() };
    let tree = resvg::usvg::Tree::from_data(bytes, &opts).ok()?;
    let size = tree.size();
    let k = (max_side.max(1) as f32 / size.width().max(size.height())).min(1.0);
    let (w, h) = ((size.width() * k).ceil().max(1.0) as u32, (size.height() * k).ceil().max(1.0) as u32);
    let mut p = Pixmap::new(w, h)?;
    resvg::render(&tree, Transform::from_scale(k, k), &mut p.as_mut());
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(img: &Image, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let c = img.0.pixel(x, y).unwrap().demultiply();
        (c.red(), c.green(), c.blue(), c.alpha())
    }

    #[test]
    fn gif_first_frame_and_transparency() {
        // 2x1: a red pixel and a transparent one.
        let mut out = Vec::new();
        {
            let mut e = gif::Encoder::new(&mut out, 2, 1, &[255, 0, 0, 0, 0, 0]).unwrap();
            let mut f = gif::Frame { width: 2, height: 1, transparent: Some(1), ..Default::default() };
            f.buffer = std::borrow::Cow::Borrowed(&[0, 1]);
            e.write_frame(&f).unwrap();
        }
        let img = Image::decode(&out, 100).unwrap();
        assert_eq!(img.size(), (2, 1));
        assert_eq!(pixel(&img, 0, 0), (255, 0, 0, 255));
        assert_eq!(pixel(&img, 1, 0).3, 0);
    }

    #[test]
    fn webp_lossless_round_trip_and_scaling() {
        let rgba: Vec<u8> = (0..4 * 4).flat_map(|_| [0, 128, 255, 255]).collect();
        let mut out = Vec::new();
        image_webp::WebPEncoder::new(&mut out).encode(&rgba, 4, 4, image_webp::ColorType::Rgba8).unwrap();
        let img = Image::decode(&out, 100).unwrap();
        assert_eq!(img.size(), (4, 4));
        assert_eq!(pixel(&img, 2, 2), (0, 128, 255, 255));
        assert_eq!(Image::decode(&out, 2).unwrap().size(), (2, 2));
    }

    #[test]
    fn jpeg_baseline_grayscale() {
        // An 8x8 baseline JPEG built by hand: one block whose DC and AC coefficients are all zero,
        // which decodes to mid gray. Each Huffman table holds a single one-bit code.
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
        j.extend([1u8; 64]);
        j.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00]);
        for class in [0x00, 0x10] {
            j.extend([0xFF, 0xC4, 0x00, 0x14, class, 1]);
            j.extend([0u8; 15]);
            j.push(0);
        }
        j.extend([0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x3F, 0xFF, 0xD9]);
        let img = Image::decode(&j, 100).unwrap();
        assert_eq!(img.size(), (8, 8));
        let (r, g, b, a) = pixel(&img, 3, 5);
        assert!(r.abs_diff(128) <= 1 && r == g && g == b && a == 255, "{:?}", (r, g, b, a));
    }

    #[test]
    fn svg_at_own_size_and_scaled_down() {
        let svg = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#00ff00"/></svg>"##;
        let img = Image::decode(svg, 100).unwrap();
        assert_eq!(img.size(), (40, 20));
        assert_eq!(pixel(&img, 5, 5), (0, 255, 0, 255));
        assert_eq!(pixel(&img, 30, 5).3, 0);
        assert_eq!(Image::decode(svg, 10).unwrap().size(), (10, 5));
        assert!(Image::decode(b"<svg", 100).is_none());
    }

    #[test]
    fn unknown_and_truncated_data_are_rejected() {
        assert!(Image::decode(b"hello", 100).is_none());
        assert!(Image::decode(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0], 100).is_none());
        assert!(Image::decode(b"GIF89a", 100).is_none());
    }
}
