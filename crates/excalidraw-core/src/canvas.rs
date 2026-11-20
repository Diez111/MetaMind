//! Export a scene as JSON Canvas, the format `canvas-core` reads (`.canvas` files).
//!
//! JSON Canvas has two things — nodes and edges — and a node is text, a file, a link or a group.
//! The mapping below is therefore lossy, and every loss is named here so nobody has to guess:
//!
//! | Excalidraw | becomes | loss |
//! |---|---|---|
//! | `text` | a `text` node with the element's box | font, size and alignment |
//! | `rectangle`, `diamond`, `ellipse` | a `text` node holding the shape's label, or an empty one | the shape: a diamond and a rectangle are both an empty box |
//! | `arrow`, `line` bound at both ends | an `edge` between the two nodes | the route: an elbow or a curve becomes a straight edge |
//! | a text element inside a shape (`containerId`) | the shape's node text | — |
//! | a text element on an arrow | the edge label | — |
//! | `arrow` or `line` without both bindings | nothing | free-standing lines have no JSON Canvas form |
//! | `freedraw` | nothing | — |
//! | `image` | nothing | a canvas `file` node needs a vault path; Excalidraw embeds a data URL |
//! | `frame`, `embed`, `iframe` | nothing | — |
//! | colours | the node or edge `color` | alpha, and any colour that does not parse |
//! | stroke width, dash pattern, fill pattern, opacity, `angle` | nothing | JSON Canvas has no style beyond
//!   a colour, and its nodes are axis-aligned |
//! | `appState.viewBackgroundColor` | nothing | JSON Canvas has no page colour |
//!
//! Element ids are hashed into the 16 hex digits JSON Canvas uses, so the same board always exports
//! to the same file.

use std::collections::HashMap;

use canvas_core::{Canvas, NodeKind};

use crate::{Binding, Element, ElementType, Rgba, Scene};

/// Map a scene onto a JSON Canvas document.
pub fn to_canvas(scene: &Scene) -> Canvas {
    let mut canvas = Canvas::default();
    let mut nodes: HashMap<&str, String> = HashMap::new();

    // Text elements that label a shape or an arrow travel with their host instead of becoming nodes.
    let mut labels: HashMap<&str, &str> = HashMap::new();
    for e in &scene.elements {
        if e.kind == ElementType::Text && e.is_deleted != Some(true) {
            if let (Some(host), Some(text)) = (&e.container_id, &e.text) {
                labels.insert(host.as_str(), text.as_str());
            }
        }
    }
    let labelled = |e: &Element| {
        e.container_id.as_deref().is_some_and(|host| {
            scene.element(host).is_some_and(|h| {
                matches!(
                    h.kind,
                    ElementType::Rectangle
                        | ElementType::Diamond
                        | ElementType::Ellipse
                        | ElementType::Arrow
                        | ElementType::Line
                )
            })
        })
    };

    for e in &scene.elements {
        if e.is_deleted == Some(true) {
            continue;
        }
        let text = match &e.kind {
            ElementType::Rectangle | ElementType::Diamond | ElementType::Ellipse => {
                labels.get(e.id.as_str()).copied().unwrap_or("").to_string()
            }
            ElementType::Text if !labelled(e) => match (&e.text, &e.original_text) {
                (Some(t), _) | (None, Some(t)) => t.clone(),
                _ => String::new(),
            },
            _ => continue,
        };
        let id = canvas.add_node(NodeKind::Text { text }, e.x, e.y, e.width, e.height, seed(&e.id));
        if let Some(node) = canvas.node_mut(&id) {
            node.color = colour(e);
        }
        nodes.insert(e.id.as_str(), id);
    }

    for e in &scene.elements {
        if e.is_deleted == Some(true) || !matches!(e.kind, ElementType::Arrow | ElementType::Line) {
            continue;
        }
        let (Some(from), Some(to)) = (binding(&e.start_binding), binding(&e.end_binding)) else {
            continue;
        };
        let (Some(from), Some(to)) = (nodes.get(from), nodes.get(to)) else { continue };
        let Some(id) = canvas.add_edge(from, to, seed(&e.id)) else { continue };
        if let Some(edge) = canvas.edges.iter_mut().find(|edge| edge.id == id) {
            edge.color = colour(e);
            edge.label = labels.get(e.id.as_str()).filter(|t| !t.is_empty()).map(|t| t.to_string());
        }
    }

    canvas
}

fn binding(b: &Option<Binding>) -> Option<&str> {
    b.as_ref().map(|b| b.element_id.as_str()).filter(|id| !id.is_empty())
}

/// A node or edge colour, but only when it is opaque: JSON Canvas colours have no alpha channel.
fn colour(e: &Element) -> Option<String> {
    Rgba::parse(&e.stroke_color).filter(|c| c.a == 255).map(|c| c.to_string())
}

/// FNV-1a of the element id, which is what `canvas-core` turns into its 16 hex digit ids.
fn seed(id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
