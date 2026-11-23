//! What a scene paints: boxes, ellipses, diamonds, polylines, text runs and image references, in
//! paint order. Coordinates are scene coordinates; nothing here knows about zoom or scrolling.

use crate::style::{FillStyle, FontFamily, Rgba, StrokeStyle, TextAlign, VerticalAlign};
use crate::{Arrowhead, Element, ElementType, Scene};

/// An axis-aligned box, plus the rotation Excalidraw stores for it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shape {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Radians, clockwise, around the centre of the box.
    pub angle: f64,
}

/// How an op is painted. Every field comes straight from the element; nothing is resolved.
#[derive(Clone, PartialEq, Debug)]
pub struct Style {
    /// `None` when the element asks for `"transparent"` or a colour this crate cannot read.
    pub stroke: Option<Rgba>,
    /// `None` when the element has no background.
    pub fill: Option<Rgba>,
    pub fill_style: FillStyle,
    pub stroke_width: f64,
    pub stroke_style: StrokeStyle,
    /// `roughness > 0`: the element wants a hand-drawn stroke. Kept as a flag — this crate does not
    /// emulate sketchy strokes.
    pub sketchy: bool,
    /// 0..1, from the element's 0..100 `opacity`.
    pub opacity: f64,
}

/// An open polyline in scene coordinates: a line, an arrow or a freedraw stroke.
#[derive(Clone, PartialEq, Debug)]
pub struct Polyline {
    pub points: Vec<(f64, f64)>,
    /// One 0..=1 pressure per point, for a stroke drawn with a pen. Empty when the file has none:
    /// the renderer then draws a constant width.
    pub pressures: Vec<f64>,
    /// The arrowhead mark at the first and last point. The geometry of the mark is the renderer's
    /// job; this crate only says which mark the file asked for.
    pub start_head: Option<Arrowhead>,
    pub end_head: Option<Arrowhead>,
    pub style: Style,
}

/// One text element, laid out as Excalidraw left it: the box is the element's, not a measurement.
#[derive(Clone, PartialEq, Debug)]
pub struct TextRun {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub angle: f64,
    pub size: f64,
    pub line_height: f64,
    pub family: FontFamily,
    pub align: TextAlign,
    pub vertical: VerticalAlign,
    /// Text is painted in the element's stroke colour.
    pub colour: Option<Rgba>,
    pub opacity: f64,
}

/// An embedded image. The bytes stay in [`Scene::files`]; this op only points at them.
#[derive(Clone, PartialEq, Debug)]
pub struct ImageRef {
    pub file_id: String,
    pub mime_type: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub angle: f64,
    /// Excalidraw's `scale`: metadata only, already multiplied into `width`/`height`.
    pub scale: Option<(f64, f64)>,
    pub opacity: f64,
}

#[derive(Clone, PartialEq, Debug)]
pub enum DrawOp {
    Rect(Shape, Style),
    Ellipse(Shape, Style),
    Diamond(Shape, Style),
    Polyline(Polyline),
    TextRun(TextRun),
    ImageRef(ImageRef),
}

/// Why an element produced no drawing command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Skip {
    /// `isDeleted: true`: Excalidraw keeps deleted elements in the file until it is saved again.
    Deleted,
    /// A type this crate does not paint, such as `frame`, `embed` or `iframe`.
    Type(String),
    /// A text element with no text.
    EmptyText,
    /// An arrow or line with fewer than two points.
    NoGeometry,
    /// An image whose `fileId` is not in `files`.
    MissingFile(String),
}

/// A box in scene coordinates: everything the display list paints fits in one of these.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }
}

/// The drawing commands of one scene, in paint order.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DisplayList {
    /// The page colour from `appState.viewBackgroundColor`.
    pub background: Option<Rgba>,
    pub ops: Vec<DrawOp>,
    /// Elements that produced no op, with the reason, so a viewer can say what it dropped.
    pub skipped: Vec<(String, Skip)>,
}

pub(crate) fn build(scene: &Scene) -> DisplayList {
    let mut list = DisplayList {
        background: scene.app_state.view_background_color.as_deref().and_then(Rgba::parse),
        ..DisplayList::default()
    };
    for e in &scene.elements {
        match op(scene, e) {
            Ok(op) => list.ops.push(op),
            Err(skip) => list.skipped.push((e.id.clone(), skip)),
        }
    }
    list
}

fn op(scene: &Scene, e: &Element) -> Result<DrawOp, Skip> {
    if e.is_deleted == Some(true) {
        return Err(Skip::Deleted);
    }
    let shape = Shape { x: e.x, y: e.y, width: e.width, height: e.height, angle: e.angle };
    let style = Style::of(e);
    match &e.kind {
        ElementType::Rectangle => Ok(DrawOp::Rect(shape, style)),
        ElementType::Diamond => Ok(DrawOp::Diamond(shape, style)),
        ElementType::Ellipse => Ok(DrawOp::Ellipse(shape, style)),
        ElementType::Text => {
            let text = e.text.as_deref().or(e.original_text.as_deref()).unwrap_or("").to_string();
            if text.is_empty() {
                return Err(Skip::EmptyText);
            }
            Ok(DrawOp::TextRun(TextRun {
                text,
                x: e.x,
                y: e.y,
                width: e.width,
                height: e.height,
                angle: e.angle,
                size: e.font_size.unwrap_or(20.0),
                line_height: e.line_height.unwrap_or(1.25),
                family: e.font_family.unwrap_or_default(),
                align: e.text_align.clone().unwrap_or_default(),
                vertical: e.vertical_align.clone().unwrap_or_default(),
                colour: colour(&e.stroke_color),
                opacity: opacity(e.opacity),
            }))
        }
        ElementType::Arrow | ElementType::Line => {
            let points = absolute_points(e);
            if points.len() < 2 {
                return Err(Skip::NoGeometry);
            }
            let (start_head, end_head) = match e.kind {
                // A plain line has no arrowheads and Excalidraw writes `null` for both.
                ElementType::Arrow => (e.start_arrowhead.clone(), e.end_arrowhead.clone()),
                _ => (None, None),
            };
            Ok(DrawOp::Polyline(Polyline { points, pressures: Vec::new(), start_head, end_head, style }))
        }
        ElementType::Freedraw => {
            let points = absolute_points(e);
            if points.len() < 2 {
                return Err(Skip::NoGeometry);
            }
            // A pen stroke carries how hard the tip was pressed at every point: the renderer
            // uses it for the width, and a file drawn with a mouse simply has none.
            let pressures = e
                .pressures
                .clone()
                .filter(|p| p.len() == points.len())
                .map(|p| p.into_iter().map(|v| v.clamp(0.0, 1.0)).collect())
                .unwrap_or_default();
            Ok(DrawOp::Polyline(Polyline { points, pressures, start_head: None, end_head: None, style }))
        }
        ElementType::Image => {
            let file_id = e.file_id.clone().unwrap_or_default();
            let Some(file) = scene.files.get(&file_id) else {
                return Err(Skip::MissingFile(file_id));
            };
            Ok(DrawOp::ImageRef(ImageRef {
                file_id,
                mime_type: Some(file.mime_type.clone()).filter(|m| !m.is_empty()),
                x: e.x,
                y: e.y,
                width: e.width,
                height: e.height,
                angle: e.angle,
                scale: e.scale.map(|s| (s[0], s[1])),
                opacity: opacity(e.opacity),
            }))
        }
        ElementType::Other(name) => Err(Skip::Type(name.clone())),
    }
}

impl Style {
    fn of(e: &Element) -> Style {
        Style {
            stroke: colour(&e.stroke_color),
            fill: colour(&e.background_color),
            fill_style: e.fill_style.clone(),
            stroke_width: e.stroke_width,
            stroke_style: e.stroke_style.clone(),
            sketchy: e.roughness > 0.0,
            opacity: opacity(e.opacity),
        }
    }
}

/// `None` for `"transparent"`, for a colour that does not parse and for a fully transparent one.
fn colour(s: &str) -> Option<Rgba> {
    Rgba::parse(s).filter(|c| c.a > 0)
}

fn opacity(percent: f64) -> f64 {
    (percent / 100.0).clamp(0.0, 1.0)
}

/// An element's `points` are offsets from its `x`/`y`; every consumer wants scene coordinates.
pub(crate) fn absolute_points(e: &Element) -> Vec<(f64, f64)> {
    e.points.iter().flatten().map(|p| (e.x + p[0], e.y + p[1])).collect()
}

impl DisplayList {
    /// The box every op paints, in scene coordinates. `None` when the list paints nothing.
    ///
    /// This is what a viewer frames: a scene whose elements start at `x: 40` has its left edge at
    /// 40, not at 0, so a board never needs a margin guessed around it.
    pub fn bounds(&self) -> Option<Bounds> {
        let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
        let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut grow = |x: f64, y: f64| {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        };
        for op in &self.ops {
            match op {
                DrawOp::Rect(s, _) | DrawOp::Ellipse(s, _) | DrawOp::Diamond(s, _) => {
                    // A rotated box is framed by the box around its corners, not by its own sides.
                    let (hw, hh) = (s.width / 2.0, s.height / 2.0);
                    let (cx, cy) = (s.x + hw, s.y + hh);
                    let (sin, cos) = s.angle.sin_cos();
                    for (dx, dy) in [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)] {
                        grow(cx + dx * cos - dy * sin, cy + dx * sin + dy * cos);
                    }
                }
                DrawOp::Polyline(p) => {
                    for &(x, y) in &p.points {
                        grow(x, y);
                    }
                }
                DrawOp::TextRun(t) => {
                    grow(t.x, t.y);
                    grow(t.x + t.width, t.y + t.height);
                }
                DrawOp::ImageRef(i) => {
                    grow(i.x, i.y);
                    grow(i.x + i.width, i.y + i.height);
                }
            }
        }
        if !x0.is_finite() || !y0.is_finite() {
            return None;
        }
        Some(Bounds { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }

    /// A stable one-line-per-op text rendering, used by the tests and pasted into reviews.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        if let Some(bg) = self.background {
            out.push_str(&format!("background {bg}\n"));
        }
        for op in &self.ops {
            out.push_str(&op.dump());
            out.push('\n');
        }
        for (id, skip) in &self.skipped {
            out.push_str(&format!("skip {id} {}\n", skip.dump()));
        }
        out
    }
}

impl Skip {
    fn dump(&self) -> String {
        match self {
            Skip::Deleted => "deleted".into(),
            Skip::Type(name) => format!("type={name}"),
            Skip::EmptyText => "empty-text".into(),
            Skip::NoGeometry => "no-geometry".into(),
            Skip::MissingFile(id) => format!("missing-file={id}"),
        }
    }
}

impl DrawOp {
    fn dump(&self) -> String {
        match self {
            DrawOp::Rect(s, st) => format!("rect {} {}", s.dump(), st.dump()),
            DrawOp::Ellipse(s, st) => format!("ellipse {} {}", s.dump(), st.dump()),
            DrawOp::Diamond(s, st) => format!("diamond {} {}", s.dump(), st.dump()),
            DrawOp::Polyline(p) => p.dump(),
            DrawOp::TextRun(t) => t.dump(),
            DrawOp::ImageRef(i) => i.dump(),
        }
    }
}

impl Shape {
    fn dump(&self) -> String {
        format!("{}x{} at {},{} angle={}", n(self.width), n(self.height), n(self.x), n(self.y), n(self.angle))
    }
}

impl Style {
    fn dump(&self) -> String {
        let stroke = match self.stroke {
            Some(c) => format!("{c}/{}/{}", n(self.stroke_width), self.stroke_style.as_str()),
            None => format!("none/{}/{}", n(self.stroke_width), self.stroke_style.as_str()),
        };
        let fill = match self.fill {
            Some(c) => format!("{c}/{}", self.fill_style.as_str()),
            None => "none".to_string(),
        };
        let sketchy = if self.sketchy { " sketchy" } else { "" };
        format!("stroke={stroke} fill={fill} opacity={}{sketchy}", n(self.opacity))
    }
}

impl Polyline {
    fn dump(&self) -> String {
        let points: Vec<String> = self.points.iter().map(|(x, y)| format!("({},{})", n(*x), n(*y))).collect();
        let head = |h: &Option<Arrowhead>| h.as_ref().map_or("none".to_string(), |h| h.as_str().to_string());
        format!(
            "polyline {} heads={}:{} {}",
            points.join(" "),
            head(&self.start_head),
            head(&self.end_head),
            self.style.dump()
        )
    }
}

impl TextRun {
    fn dump(&self) -> String {
        format!(
            "text {}x{} at {},{} angle={} size={} line={} font={} align={}/{} colour={} opacity={} {:?}",
            n(self.width),
            n(self.height),
            n(self.x),
            n(self.y),
            n(self.angle),
            n(self.size),
            n(self.line_height),
            self.family.as_str(),
            self.align.as_str(),
            self.vertical.as_str(),
            self.colour.map_or("none".to_string(), |c| c.to_string()),
            n(self.opacity),
            self.text
        )
    }
}

impl ImageRef {
    fn dump(&self) -> String {
        format!(
            "image {}x{} at {},{} angle={} file={} mime={} opacity={}",
            n(self.width),
            n(self.height),
            n(self.x),
            n(self.y),
            n(self.angle),
            self.file_id,
            self.mime_type.as_deref().unwrap_or("unknown"),
            n(self.opacity)
        )
    }
}

/// Whole numbers without a decimal part, everything else rounded to three decimals.
fn n(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
