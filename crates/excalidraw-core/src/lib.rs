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
//! - **Wiring**: nothing in the app opens `.excalidraw` files yet.

mod canvas;
mod display;
mod style;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use style::string_enum;

pub use canvas::to_canvas;
pub use display::{DisplayList, DrawOp, ImageRef, Polyline, Shape, Skip, Style, TextRun};
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

/// The canvas state saved with the scene. Only the two fields a renderer needs are typed; the rest
/// (zoom, scroll, selection, current style) is kept in `extra`.
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    pub grid_size: Option<f64>,
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

    /// Drawing commands for the scene, in paint order.
    pub fn display_list(&self) -> DisplayList {
        display::build(self)
    }
}

#[cfg(test)]
mod tests;
