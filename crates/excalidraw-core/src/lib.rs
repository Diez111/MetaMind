//! Excalidraw scenes (`.excalidraw`, file format 2) read into a model, a display list and a JSON
//! Canvas. No JavaScript, no fonts, no UI: painting the commands is the caller's job.
//!
//! A scene is the document Excalidraw's *Save to file* writes:
//!
//! ```json
//! {"type":"excalidraw","version":2,"source":"https://excalidraw.com",
//!  "elements":[…],"appState":{"viewBackgroundColor":"#ffffff"},
//!  "files":{"<fileId>":{"dataURL":"data:image/png;base64,…","mimeType":"image/png","created":…}}}
//! ```
//!
//! Three rules shape this crate:
//!
//! - **Unknown fields survive.** Every struct keeps the keys this version does not know in a
//!   flattened `extra` map, and there is no `deny_unknown_fields` anywhere: the format grows with
//!   every Excalidraw release. The JSON Canvas reader in `canvas-core` follows the same rule.
//! - **The model is the file.** [`Scene::to_json`] writes back what was read; numbers are held as
//!   `f64`, so an integral `60` comes back as `60.0`. The model round-trips, the bytes need not. The
//!   `float_roundtrip` feature of `serde_json` is on: without it its parser is a unit in the last
//!   place off for some 17-digit decimals, which would show up as a scene that changes every time it
//!   is opened.
//! - **Nothing is emulated.** Hand-drawn ("sketchy") strokes, hachure and cross-hatch fills, rounded
//!   corners, arrowhead geometry and freedraw pressure are carried as data or as flags; the display
//!   list itself is straight boxes, ellipses, diamonds, polylines and text runs.
//!
//! # Where the display list stops, and why
//!
//! [`Scene::display_list`] reports, per element, either drawing commands or a [`Skip`] reason. An
//! element that is deleted, of a type this crate does not paint (`frame`, `embed`, `iframe`, …), or
//! missing the data it needs (an image whose `fileId` is not in `files`, an arrow without points)
//! produces no commands and one skip entry, so a viewer can say what it dropped instead of quietly
//! showing a smaller board.
//!
//! The text runs carry the element's own box: Excalidraw lays text out (wrapping, fonts, container
//! centring) and stores the result in `x`/`y`/`width`/`height`, so nothing here wraps or measures.
//!
//! # Export to JSON Canvas
//!
//! [`to_canvas`] maps a scene onto the format `canvas-core` reads (`.canvas`). The mapping is lossy
//! by construction and the full table is in the `canvas` module: JSON Canvas has nodes and edges
//! only, so a shape becomes an empty or labelled text node, a bound arrow becomes an edge, and
//! freedraw strokes, images (embedded data URLs are not vault paths) and unbound arrows are dropped.
//!
//! # Not covered yet
//!
//! - **Frames and embeds** (`frame`, `embed`, `iframe`): they parse and round-trip, and the display
//!   list reports them in [`DisplayList::skipped`]. Frame clipping is not applied — elements paint at
//!   their own coordinates whether or not a frame holds them.
//! - **Sketchy strokes and fills**: `roughness` and `fillStyle: hachure | cross-hatch` are flags on
//!   [`Style`]; nothing is hatched or jittered, and rounded corners (`roundness`) are not drawn.
//! - **Arrowheads and elbow routes**: [`Polyline`] says which mark each end asked for; the geometry of
//!   the mark and `elbowed: true` routing belong to the renderer.
//! - **Freedraw pressure**: `pressures` is kept in the model, but the stroke is a constant-width
//!   polyline.
//! - **Images**: the op points at the `fileId`; the bytes stay in [`Scene::files`] as a data URL.
//!   Decoding, `crop` and the natural size behind `scale` are left to the caller.
//! - **Grouping, locking and links**: `groupIds`, `locked` and `link` are in the model; the display
//!   list paints in file order and does not group, lock or draw anything for a link.
//! - **Format 1**: a version-1 file parses as far as its fields match the model, but no v1 file was
//!   tested here — the fixtures are all version 2.
//! - **Editing**: [`new_element`], [`Element::contains`], [`Element::bump`] and the mutating
//!   methods of [`Scene`] are what a board editor needs — a scene can be built, hit-tested, moved
//!   and written back without leaving this crate. Grouping, locking and frame containment are not
//!   part of that: an editor selects one element at a time and paints in file order.
//! - **Wiring**: MetaMind opens a `.excalidraw` file in its own tab (`crates/app/src/
//!   excalidrawview.rs`) and draws `![[…]]` embeds read-only (`crates/app/src/excalidraw.rs`).

mod canvas;
mod display;
mod style;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use style::string_enum;

pub use canvas::to_canvas;
pub use display::{Bounds, DisplayList, DrawOp, ImageRef, Polyline, Shape, Skip, Style, TextRun};
pub use style::{Arrowhead, FillStyle, FontFamily, Rgba, StrokeStyle, TextAlign, VerticalAlign};

string_enum! {
    /// The element `type` values this crate knows. Anything else — `frame`, `embed`, `iframe`,
    /// `magicframe`, a type a future release adds — is kept in `Other` and skipped when painting.
    ElementType {
        Rectangle => "rectangle", Diamond => "diamond", Ellipse => "ellipse", Text => "text",
        Arrow => "arrow", Line => "line", Freedraw => "freedraw", Image => "image",
    } default ElementType::Other(String::new())
}

/// A scene file. `elements` are painted in order; later elements sit on top.
#[derive(Clone, PartialEq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub elements: Vec<Element>,
    pub app_state: AppState,
    /// Embedded images, keyed by the `fileId` the image elements point at.
    pub files: BTreeMap<String, BinaryFile>,
    /// Top-level keys this crate does not know.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One element. The fields of every type live side by side and stay `None` when the element does not
/// use them, which is what lets a single struct cover a format with nine shapes. A key the file does
/// not have gets Excalidraw's own default (stroke `#1e1e1e`, opacity 100, roughness 1), not zero.
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Element {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ElementType,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Radians, clockwise, around the centre of the element.
    pub angle: f64,
    /// `#rrggbb`, `#rrggbbaa` or `"transparent"`, kept verbatim.
    pub stroke_color: String,
    pub background_color: String,
    pub fill_style: FillStyle,
    pub stroke_width: f64,
    pub stroke_style: StrokeStyle,
    /// 0 = crisp, 1 = hand-drawn (default), 2 = more so. Kept as a flag, not reproduced.
    pub roughness: f64,
    /// 0..100, as Excalidraw writes it.
    pub opacity: f64,
    pub seed: u64,
    #[serde(rename = "groupIds")]
    pub group_ids: Vec<String>,
    pub frame_id: Option<String>,
    pub bound_elements: Option<Vec<BoundElement>>,
    /// Milliseconds since the epoch.
    pub updated: Option<u64>,
    pub link: Option<String>,
    pub locked: bool,
    pub is_deleted: Option<bool>,
    /// `{"type":2,"value":32}`, kept as is: rounded corners are not reproduced.
    pub roundness: Option<Value>,

    // text
    pub text: Option<String>,
    pub font_size: Option<f64>,
    pub font_family: Option<FontFamily>,
    pub text_align: Option<TextAlign>,
    pub vertical_align: Option<VerticalAlign>,
    /// Set when this text is the label of a container (a shape or an arrow).
    pub container_id: Option<String>,
    pub original_text: Option<String>,
    pub line_height: Option<f64>,
    pub auto_resize: Option<bool>,

    // arrow, line, freedraw
    /// Offsets from `x`/`y`: `[[0,0],[120,40]]`.
    pub points: Option<Vec<[f64; 2]>>,
    pub start_binding: Option<Binding>,
    pub end_binding: Option<Binding>,
    pub start_arrowhead: Option<Arrowhead>,
    pub end_arrowhead: Option<Arrowhead>,
    /// One 0..1 pressure per point; carried, not used by the display list.
    pub pressures: Option<Vec<f64>>,
    pub simulate_pressure: Option<bool>,

    // image
    pub file_id: Option<String>,
    /// Natural size multiplied into `width`/`height`; carried, not used by the display list.
    pub scale: Option<[f64; 2]>,
    pub status: Option<String>,
    /// Crop rectangle, kept as is: crops are not applied.
    pub crop: Option<Value>,

    /// Element keys this crate does not know (`version`, `versionNonce`, `elbowed`, …).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An element or arrow a shape/text is bound to, from `boundElements`.
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoundElement {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One end of an arrow: which element it is attached to, and how tightly.
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Binding {
    pub element_id: String,
    pub focus: f64,
    /// Distance in pixels between the arrowhead and the element.
    pub gap: f64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The canvas state saved with the scene. The grid and the page colour are the two fields a
/// renderer needs; the rest (zoom, scroll, selection, current style) is kept in `extra`.
///
/// `grid_size` is a `Value` on purpose: Excalidraw writes a number of pixels, older files write
/// `"small"`/`"medium"`/`"large"`, and a typed `f64` would fail on the string and throw the whole
/// app state into `extra`.
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    /// Pixels between grid lines, or one of `small`, `medium`, `large`.
    pub grid_size: Option<Value>,
    /// `dots`, `grid`, `ruled` or `none`. Absent in a file written by Excalidraw itself, which
    /// only has the switch "grid on/off".
    pub grid_mode: Option<String>,
    pub view_background_color: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An image embedded in the scene as a data URL.
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BinaryFile {
    #[serde(rename = "dataURL")]
    pub data_url: String,
    pub mime_type: String,
    /// Milliseconds since the epoch, when the image was added.
    pub created: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A scene that could not be read, with the element or key that stopped it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid Excalidraw scene: {}", self.0)
    }
}
impl std::error::Error for Error {}

impl Default for Element {
    fn default() -> Element {
        Element {
            id: String::new(),
            kind: ElementType::default(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            angle: 0.0,
            stroke_color: "#1e1e1e".into(),
            background_color: "transparent".into(),
            fill_style: FillStyle::default(),
            stroke_width: 2.0,
            stroke_style: StrokeStyle::default(),
            roughness: 1.0,
            opacity: 100.0,
            seed: 0,
            group_ids: Vec::new(),
            frame_id: None,
            bound_elements: None,
            updated: None,
            link: None,
            locked: false,
            is_deleted: None,
            roundness: None,
            text: None,
            font_size: None,
            font_family: None,
            text_align: None,
            vertical_align: None,
            container_id: None,
            original_text: None,
            line_height: None,
            auto_resize: None,
            points: None,
            start_binding: None,
            end_binding: None,
            start_arrowhead: None,
            end_arrowhead: None,
            pressures: None,
            simulate_pressure: None,
            file_id: None,
            scale: None,
            status: None,
            crop: None,
            extra: Map::new(),
        }
    }
}

impl Default for Scene {
    fn default() -> Scene {
        Scene {
            kind: "excalidraw".into(),
            version: 2,
            source: None,
            elements: Vec::new(),
            app_state: AppState::default(),
            files: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}

impl Scene {
    /// Read a `.excalidraw` file. An empty file is an empty scene (the editor writes one when a board
    /// is created and nothing is drawn yet).
    ///
    /// A known field holding the wrong type fails here, with the element id in the message: dropping
    /// that element instead would silently lose it on the next save.
    pub fn parse(src: &str) -> Result<Scene, Error> {
        if src.trim().is_empty() {
            return Ok(Scene::default());
        }
        let v: Value = serde_json::from_str(src).map_err(|e| Error(e.to_string()))?;
        let Value::Object(mut top) = v else {
            return Err(Error("top level must be an object".into()));
        };
        if let Some(kind) = top.get("type").and_then(Value::as_str) {
            if kind != "excalidraw" {
                return Err(Error(format!("\"type\" is {kind:?}, not \"excalidraw\"")));
            }
        }
        let mut scene = Scene::default();
        if let Some(version) = top.get("version") {
            scene.version = version.as_f64().map(|v| v as u32).unwrap_or(2);
        }
        scene.source = top.get("source").and_then(Value::as_str).map(str::to_string);
        if let Some(list) = top.get("elements") {
            let list = list.as_array().ok_or_else(|| Error("\"elements\" must be an array".into()))?;
            scene.elements = list
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    serde_json::from_value(e.clone()).map_err(|err| {
                        let id = e.get("id").and_then(Value::as_str).unwrap_or("?");
                        Error(format!("element {i} (id {id}): {err}"))
                    })
                })
                .collect::<Result<Vec<Element>, Error>>()?;
        }
        if let Some(state) = top.get("appState") {
            // A stale field must not cost the whole app state: keep the object untouched instead.
            scene.app_state = serde_json::from_value(state.clone()).unwrap_or_else(|_| AppState {
                grid_size: None,
                grid_mode: None,
                view_background_color: None,
                extra: state.as_object().cloned().unwrap_or_default(),
            });
        }
        if let Some(files) = top.get("files") {
            scene.files =
                serde_json::from_value(files.clone()).map_err(|e| Error(format!("\"files\": {e}")))?;
        }
        for key in ["type", "version", "source", "elements", "appState", "files"] {
            top.shift_remove(key);
        }
        scene.extra = top;
        Ok(scene)
    }

    /// Write the scene back as Excalidraw writes it: two-space indentation, unknown fields kept.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The element with this id, if the scene has one.
    pub fn element(&self, id: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// The element with this id, to change it.
    pub fn element_mut(&mut self, id: &str) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    /// The topmost element painted at a scene point, which is what a click selects.
    pub fn hit(&self, x: f64, y: f64) -> Option<&Element> {
        self.elements.iter().rev().find(|e| e.painted() && e.contains(x, y))
    }

    /// Drop an element from the scene. False when no element has that id.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.elements.len();
        self.elements.retain(|e| e.id != id);
        self.elements.len() != before
    }

    /// The box everything in the scene paints, in scene coordinates.
    pub fn bounds(&self) -> Option<Bounds> {
        self.display_list().bounds()
    }

    /// Drawing commands for the scene, in paint order.
    pub fn display_list(&self) -> DisplayList {
        display::build(self)
    }
}

/// A new element, with the fields and the defaults Excalidraw itself writes for a freshly drawn
/// shape: the style defaults, `version: 1`, a `versionNonce` and the `updated` stamp. `seed` fills
/// the id and the nonce (drawing programs pass a pseudorandom number so two identical shapes
/// differ).
///
/// The points of a `line`, `arrow` or `freedraw` are its own box: `[[0, 0], [width, height]]`,
/// offsets from `x`/`y` the way the format stores them.
pub fn new_element(
    kind: ElementType,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    seed: u64,
    now_ms: u64,
) -> Element {
    let mut e = Element {
        id: format!("{seed:016x}"),
        kind: kind.clone(),
        x,
        y,
        width,
        height,
        seed,
        updated: Some(now_ms),
        ..Element::default()
    };
    e.extra.insert("version".into(), Value::from(1));
    e.extra.insert("versionNonce".into(), Value::from(seed as u32));
    // A new element is live: `isDeleted` is a field of the model, so it is written once.
    e.is_deleted = Some(false);
    match kind {
        ElementType::Text => {
            e.text = Some(String::new());
            e.original_text = Some(String::new());
            e.font_size = Some(20.0);
            e.font_family = Some(FontFamily::HandDrawn);
            e.text_align = Some(TextAlign::Left);
            e.vertical_align = Some(VerticalAlign::Top);
            e.line_height = Some(1.25);
            e.auto_resize = Some(true);
        }
        ElementType::Arrow | ElementType::Line | ElementType::Freedraw => {
            e.points = Some(vec![[0.0, 0.0], [width, height]]);
            if matches!(kind, ElementType::Freedraw) {
                e.simulate_pressure = Some(true);
            } else {
                e.end_arrowhead = (kind == ElementType::Arrow).then_some(Arrowhead::Arrow);
            }
        }
        _ => {}
    }
    e
}

impl Element {
    /// Whether the display list would paint this element. A deleted element, a type this crate
    /// does not paint and an empty text produce no op, so they are not selectable either.
    pub fn painted(&self) -> bool {
        if self.is_deleted == Some(true) {
            return false;
        }
        match &self.kind {
            ElementType::Other(_) => false,
            ElementType::Text => !self.text().is_empty(),
            ElementType::Arrow | ElementType::Line | ElementType::Freedraw => {
                self.points.iter().flatten().count() >= 2
            }
            _ => true,
        }
    }

    /// The element's text, from `text` or, for an element whose text was never edited, from
    /// `originalText`. Empty for every other type.
    pub fn text(&self) -> String {
        self.text.as_deref().or(self.original_text.as_deref()).unwrap_or("").to_string()
    }

    /// Whether a scene point is on the element: inside its box, turned the way `angle` says, or —
    /// for a line, an arrow and a freedraw stroke — near the polyline itself, so that a thin
    /// diagonal is not selectable by the whole box around it.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match &self.kind {
            ElementType::Arrow | ElementType::Line | ElementType::Freedraw => {
                let pts = display::absolute_points(self);
                if pts.len() < 2 {
                    return false;
                }
                let tol = (self.stroke_width * 0.5).max(6.0);
                pts.windows(2).any(|w| segment_distance(x, y, w[0], w[1]) <= tol)
            }
            _ => {
                // Undo the rotation around the box centre, then test the plain box.
                let (cx, cy) = (self.x + self.width / 2.0, self.y + self.height / 2.0);
                let (sin, cos) = (-self.angle).sin_cos();
                let (dx, dy) = (x - cx, y - cy);
                let (lx, ly) = (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos);
                lx >= self.x - 1.0
                    && lx <= self.x + self.width + 1.0
                    && ly >= self.y - 1.0
                    && ly <= self.y + self.height + 1.0
            }
        }
    }

    /// Mark the element as edited the way Excalidraw does: `version` goes up and `versionNonce` and
    /// `updated` are stamped. The file then looks like one Excalidraw wrote, which is what its own
    /// synchronisation uses to decide what changed.
    pub fn bump(&mut self, now_ms: u64, nonce: u64) {
        let version = self.extra.get("version").and_then(Value::as_u64).unwrap_or(1) + 1;
        self.extra.insert("version".into(), Value::from(version));
        self.extra.insert("versionNonce".into(), Value::from(nonce as u32));
        self.updated = Some(now_ms);
    }
}

/// Distance from a point to a segment, so a line is selected by its stroke and not by its box.
fn segment_distance(x: f64, y: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (px, py) = (a.0 + dx * t, a.1 + dy * t);
    ((x - px).powi(2) + (y - py).powi(2)).sqrt()
}

#[cfg(test)]
mod tests;

/// The Excalidraw scene inside a Markdown note, as the Obsidian plugin writes it: a `%%…%%`
/// comment that holds a ```` ```json ```` fence with the whole scene in it.
///
/// Returns the byte range of the JSON text, so a caller can parse the scene and write it back
/// without touching a byte of the prose around it. `None` for a note that is not one of those,
/// which is every note that is not a drawing.
pub fn scene_in_markdown(text: &str) -> Option<std::ops::Range<usize>> {
    let mut at = 0usize;
    let mut in_comment = false;
    let mut open: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let trimmed = line.trim_end_matches(['\n', '\r']).trim();
        if !in_comment {
            if trimmed.starts_with("%%") {
                in_comment = true;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            match open {
                None => open = Some(at),
                Some(from) => {
                    let body = text.get(from..start)?;
                    if body.contains("\"excalidraw\"") {
                        // Trim the trailing newline: the scene ends where the fence begins.
                        let end = text[from..start].trim_end().len() + from;
                        return Some(from..end);
                    }
                    open = None;
                }
            }
            continue;
        }
        if trimmed.ends_with("%%") && !trimmed.starts_with("%%") {
            in_comment = false;
        }
    }
    None
}
