//! Light property tests: rotation travels untouched, any scene survives a write and a read, and the
//! export never points at a node that is not there.

use canvas_core::Canvas;
use excalidraw_core::{to_canvas, DrawOp, Scene};
use proptest::prelude::*;
use serde_json::{json, Value};

fn parse(elements: Value) -> Scene {
    let src = json!({"type": "excalidraw", "version": 2, "elements": elements}).to_string();
    Scene::parse(&src).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// The angle of an element reaches the display list unchanged, and nothing else moves.
    #[test]
    fn rotation_travels_from_the_file_to_the_display_list(
        angle in -10.0f64..10.0,
        x in -5_000.0f64..5_000.0,
        y in -5_000.0f64..5_000.0,
        width in 1.0f64..2_000.0,
        height in 1.0f64..2_000.0,
        text in "[a-zA-Z0-9 áéíóúñ]{1,16}",
    ) {
        let scene = parse(json!([{"id": "t", "type": "text", "x": x, "y": y,
            "width": width, "height": height, "angle": angle, "text": text}]));
        let list = scene.display_list();
        prop_assert_eq!(list.ops.len(), 1);
        let DrawOp::TextRun(run) = &list.ops[0] else { panic!("a text element paints a text run") };
        prop_assert_eq!(run.angle, angle);
        prop_assert_eq!((run.x, run.y, run.width, run.height), (x, y, width, height));
        prop_assert_eq!(&run.text, &text);
    }

    /// Rotating an element leaves every other field of its op alone.
    #[test]
    fn only_the_angle_changes(a in -6.0f64..6.0, b in -6.0f64..6.0) {
        let build = |angle: f64| {
            parse(json!([{"id": "r", "type": "rectangle", "x": 10, "y": 20, "width": 30, "height": 40,
                "angle": angle, "strokeColor": "#1971c2", "backgroundColor": "#a5d8ff"}]))
            .display_list()
            .dump()
        };
        let (one, other) = (build(a), build(b));
        prop_assert_eq!(
            one.replace(&format!("angle={}", trimmed(a)), ""),
            other.replace(&format!("angle={}", trimmed(b)), ""),
            "only the angle may differ"
        );
    }

    /// Whatever a scene holds, writing it and reading it back gives the same model.
    #[test]
    fn writing_and_reading_again_keeps_the_model(
        kind in prop::sample::select(vec!["rectangle", "diamond", "ellipse", "text", "arrow", "line", "freedraw", "image", "hologram"]),
        angle in -6.0f64..6.0,
        opacity in 0.0f64..100.0,
        unknown_field in "[a-z]{1,6}",
    ) {
        let key = format!("future_{unknown_field}");
        let mut element = json!({"id": "e1", "type": kind, "x": 1.5, "y": -2.25, "width": 100, "height": 50,
            "angle": angle, "opacity": opacity, "seed": 42, "groupIds": ["g1"], "stand": "in"});
        element[&key] = json!({"kept": true});
        let scene = parse(json!([element]));
        let again = Scene::parse(&scene.to_json()).unwrap();
        prop_assert_eq!(&again, &scene);
        prop_assert_eq!(again.to_json(), scene.to_json());
        prop_assert_eq!(again.elements[0].extra[&key]["kept"].as_bool(), Some(true));
    }

    /// Edges only ever join two nodes, even when bindings point nowhere.
    #[test]
    fn the_export_never_points_at_a_missing_node(
        shapes in prop::collection::vec((-1_000.0f64..1_000.0, -1_000.0f64..1_000.0), 1..4),
        arrows in prop::collection::vec((0usize..4, 0usize..4), 0..4),
    ) {
        let mut elements: Vec<Value> = shapes.iter().enumerate().map(|(i, (x, y))| json!({
            "id": format!("s{i}"), "type": "rectangle", "x": x, "y": y, "width": 80, "height": 40,
        })).collect();
        let ends = ["s0", "s1", "s2", "nowhere"];
        for (i, (from, to)) in arrows.iter().enumerate() {
            elements.push(json!({"id": format!("a{i}"), "type": "arrow", "points": [[0, 0], [10, 10]],
                "startBinding": {"elementId": ends[from % ends.len()]},
                "endBinding": {"elementId": ends[to % ends.len()]}}));
        }
        let scene = parse(Value::Array(elements.clone()));
        let canvas = to_canvas(&scene);
        for edge in &canvas.edges {
            prop_assert!(canvas.node(&edge.from_node).is_some());
            prop_assert!(canvas.node(&edge.to_node).is_some());
        }
        prop_assert!(canvas.nodes.len() <= elements.len(), "an arrow never becomes a node");
        let json = canvas.to_json();
        prop_assert_eq!(Canvas::parse(&json).unwrap().nodes.len(), canvas.nodes.len());
    }
}

/// The same rounding the display list dump uses, so the angle can be cut out of it.
fn trimmed(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
