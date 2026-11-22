//! Mermaid diagrams without JavaScript: `graph`/`flowchart` with a layered layout,
//! `sequenceDiagram`, and the class, state, entity-relationship, gantt, pie, mindmap, timeline,
//! journey, git graph and quadrant charts. Everything else is reported as unsupported so the
//! caller can show the source.

#![allow(clippy::needless_range_loop)]

mod class;
mod common;
mod er;
mod flow;
mod gantt;
mod gitgraph;
mod journey;
mod mindmap;
mod pie;
mod quadrant;
mod seq;
mod state;
mod timeline;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fill {
    None,
    Node,
    Accent,
    Background,
    /// A shade of the theme between the background and the text, so that neighbouring shapes
    /// (the slices of a pie, the levels of a mindmap) can be told apart. `0` is the faintest.
    Tone(u8),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Item {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: Fill,
        stroke: bool,
    },
    Diamond {
        cx: f32,
        cy: f32,
        w: f32,
        h: f32,
        fill: Fill,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: Fill,
    },
    Line {
        pts: Vec<(f32, f32)>,
        dashed: bool,
        width: f32,
    },
    /// Filled triangle with its tip at `(x, y)` pointing along `angle` (radians).
    Arrow {
        x: f32,
        y: f32,
        angle: f32,
        open: bool,
    },
    /// Text centered on `(x, y)`.
    Text {
        x: f32,
        y: f32,
        text: String,
        bold: bool,
        size: f32,
    },
    /// A pie slice: the part of the disc of radius `r` around `(cx, cy)` swept from `from` to
    /// `to` radians, clockwise from the positive x axis (y grows downwards).
    Sector {
        cx: f32,
        cy: f32,
        r: f32,
        from: f32,
        to: f32,
        fill: Fill,
    },
}

#[derive(Clone, Default, Debug)]
pub struct Diagram {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    Unsupported(String),
    Syntax(String),
}

pub trait Measure {
    /// Width of one line of text at `size` (bold for node titles).
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32;
}

/// Font-independent widths, for sizing before real fonts exist.
pub struct Approx;

impl Measure for Approx {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let n: f32 = text
            .chars()
            .map(|c| match c {
                ' ' | 'i' | 'l' | 'j' | '.' | ',' | ':' | '\'' | '|' | '!' => 0.3,
                'f' | 't' | 'r' | '(' | ')' | '[' | ']' => 0.38,
                'm' | 'w' | 'M' | 'W' => 0.85,
                'A'..='Z' => 0.68,
                c if c.len_utf8() > 1 => 1.0,
                _ => 0.56,
            })
            .sum();
        n * size * if bold { 1.07 } else { 1.0 }
    }
}

pub const TEXT_SIZE: f32 = 14.0;

/// Lay out the diagram described by `src` (the contents of a ```mermaid fence).
pub fn render(src: &str, m: &mut dyn Measure) -> Result<Diagram, Error> {
    // The indentation is kept: a mindmap is written as a nested list. Every other kind trims its
    // own lines.
    let lines: Vec<&str> = src
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("%%"))
        .collect();
    let mut start = 0;
    if lines.first().is_some_and(|l| l.trim() == "---") {
        if let Some(end) = lines[1..].iter().position(|l| l.trim() == "---") {
            start = end + 2;
        }
    }
    let Some(first) = lines.get(start).map(|l| l.trim()) else {
        return Err(Error::Syntax("el diagrama está vacío".into()));
    };
    let word = first.split(|c: char| c.is_whitespace() || c == ';' || c == ':').next().unwrap_or("");
    dispatch(word, first, &lines[start + 1..], m)
}

/// Hands the diagram over to the renderer its first word names.
fn dispatch(word: &str, first: &str, body: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    match word {
        "graph" | "flowchart" | "flowchart-elk" => flow::render(first, body, m),
        "sequenceDiagram" => seq::render(body, m),
        "classDiagram" | "classDiagram-v2" => class::render(body, m),
        "stateDiagram" | "stateDiagram-v2" => state::render(body, m),
        "erDiagram" => er::render(body, m),
        "gantt" => gantt::render(body, m),
        "pie" => pie::render(first, body, m),
        "mindmap" => mindmap::render(body, m),
        "timeline" => timeline::render(body, m),
        "journey" => journey::render(body, m),
        "gitGraph" => gitgraph::render(body, m),
        "quadrantChart" => quadrant::render(body, m),
        other => Err(Error::Unsupported(other.to_string())),
    }
}

#[cfg(test)]
mod tests;
