//! LaTeX math subset: parse and lay out into positioned glyphs and line segments.
//!
//! Unsupported commands are kept as their source text instead of failing, and reported in
//! [`Diagnostic`]s so the caller can mark them without hiding the formula.
//!
//! The box model is the interface: a [`Layout`] is a padded box (`Metrics`) holding glyphs
//! positioned by their baseline and line segments for rules, radicals and braces. Colours named in
//! the source travel per run in [`Glyph::color`] and [`Seg::color`].

mod color;
mod layout;
mod parse;
mod syms;

#[cfg(test)]
mod snapshots;
#[cfg(test)]
mod tests;

pub use color::Color;
pub use layout::{
    layout as layout_nodes, layout_with as layout_nodes_with, Approx, Glyph, Layout, Measure, Metrics, Seg,
};
pub use parse::{parse, parse_full, Diagnostic, Face, Node, Parsed};

/// A laid-out formula and what the parser could not read.
#[derive(Clone, Default, Debug)]
pub struct Rendered {
    pub layout: Layout,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse and lay out `src` at font size `size`. `display` selects display style (block math).
pub fn render(src: &str, size: f32, display: bool, m: &mut dyn Measure) -> Layout {
    render_full(src, size, display, &Metrics::default(), m).layout
}

/// Same as [`render`], with explicit spacing knobs and the commands that fell back to source text.
pub fn render_full(src: &str, size: f32, display: bool, metrics: &Metrics, m: &mut dyn Measure) -> Rendered {
    let p = parse_full(src);
    Rendered { layout: layout_nodes_with(&p.nodes, size, display, metrics, m), diagnostics: p.diagnostics }
}
