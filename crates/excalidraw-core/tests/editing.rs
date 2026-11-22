//! Editing a scene: the element a drawing tool creates, what a click on the board hits, and the
//! file that comes out of a board edited in the app. The serializer is the writer half of the
//! round-trip `fixtures.rs` checks for files Excalidraw wrote; here it is checked for elements this
//! crate creates, which is what the editor saves.

use excalidraw_core::{new_element, Element, ElementType, Scene};

const NOW: u64 = 1_700_000_000_000;

const KINDS: [ElementType; 8] = [
    ElementType::Rectangle,
    ElementType::Diamond,
    ElementType::Ellipse,
    ElementType::Text,
    ElementType::Arrow,
    ElementType::Line,
    ElementType::Freedraw,
    ElementType::Image,
];

fn scene_with(kind: ElementType) -> Scene {
    let mut scene = Scene::default();
    let mut e = new_element(kind, 40.0, 60.0, 120.0, 80.0, 0xabcd_ef01, NOW);
    if e.kind == ElementType::Text {
        e.text = Some("hola".into());
        e.original_text = Some("hola".into());
    }
    if e.kind == ElementType::Image {
        // An image only paints when the file it points at is in the scene; a 1×1 PNG is enough.
        e.file_id = Some("file-1".into());
        scene.files.insert(
            "file-1".into(),
            excalidraw_core::BinaryFile {
                data_url: "data:image/png;base64,iVBORw0KGgo=".into(),
                mime_type: "image/png".into(),
                created: NOW,
                ..excalidraw_core::BinaryFile::default()
            },
        );
    }
    scene.elements.push(e);
    scene
}

#[test]
fn every_created_element_round_trips_through_the_file() {
    for kind in KINDS {
        let scene = scene_with(kind.clone());
        let json = scene.to_json();
        let again = Scene::parse(&json).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        assert_eq!(again, scene, "{kind:?}: the model changed on the way out");
        assert_eq!(again.to_json(), json, "{kind:?}: a second write differs from the first");
        // And a created element still paints, so the board is not silently empty after a save.
        assert_eq!(again.display_list().ops.len(), 1, "{kind:?} produced no op");
    }
}

#[test]
fn a_created_element_carries_the_keys_excalidraw_writes() {
    let scene = scene_with(ElementType::Rectangle);
    let e = &scene.elements[0];
    assert_eq!(e.id, "00000000abcdef01");
    assert_eq!(e.stroke_color, "#1e1e1e");
    assert_eq!(e.background_color, "transparent");
    assert_eq!(e.stroke_width, 2.0);
    assert_eq!(e.opacity, 100.0);
    assert_eq!(e.angle, 0.0);
    assert_eq!(e.updated, Some(NOW));
    let json: serde_json::Value = serde_json::from_str(&scene.to_json()).unwrap();
    let el = &json["elements"][0];
    assert_eq!(el["id"], "00000000abcdef01");
    assert_eq!(el["type"], "rectangle");
    assert_eq!(el["version"], 1);
    assert_eq!(el["versionNonce"], 0xabcd_ef01u64 as u32);
    assert_eq!(el["isDeleted"], false);
    assert_eq!(el["x"], 40.0);
    assert_eq!(el["width"], 120.0);
    // The top level keeps the three keys Excalidraw's own reader requires.
    assert_eq!(json["type"], "excalidraw");
    assert_eq!(json["version"], 2);
    assert!(json["appState"].is_object() && json["files"].is_object());
}

#[test]
fn a_created_text_and_arrow_carry_their_own_fields() {
    let text = scene_with(ElementType::Text);
    let t = &text.elements[0];
    assert_eq!(t.text.as_deref(), Some("hola"));
    assert_eq!(t.font_size, Some(20.0));
    assert_eq!(t.line_height, Some(1.25));
    assert!(text.display_list().ops.len() == 1);

    let arrow = scene_with(ElementType::Arrow);
    let a = &arrow.elements[0];
    assert_eq!(a.points.as_deref(), Some(&[[0.0, 0.0], [120.0, 80.0]][..]));
    let list = arrow.display_list();
    let excalidraw_core::DrawOp::Polyline(p) = &list.ops[0] else { panic!("an arrow is a polyline") };
    assert_eq!(p.points, vec![(40.0, 60.0), (160.0, 140.0)]);
    assert!(p.end_head.is_some() && p.start_head.is_none());

    let line = scene_with(ElementType::Line);
    let list = line.display_list();
    let excalidraw_core::DrawOp::Polyline(p) = &list.ops[0] else { panic!("a line is a polyline") };
    assert!(p.start_head.is_none() && p.end_head.is_none());
}

#[test]
fn an_edited_element_is_stamped_and_the_unknown_keys_survive() {
    let mut scene = Scene::parse(
        r##"{"type":"excalidraw","version":2,"elements":[
            {"id":"r","type":"rectangle","x":10,"y":10,"width":50,"height":50,"version":3,
             "customData":{"keep":true},"strokeColor":"#e03131"}
        ],"appState":{"viewBackgroundColor":"#ffffff"},"files":{},"future":{"keep":"me"}}"##,
    )
    .unwrap();
    let e = scene.element_mut("r").unwrap();
    e.x = 70.0;
    e.bump(NOW, 7);
    assert_eq!(e.extra["version"], 4);
    assert_eq!(e.extra["versionNonce"], 7);
    assert_eq!(e.updated, Some(NOW));
    let again = Scene::parse(&scene.to_json()).unwrap();
    assert_eq!(again.element("r").unwrap().x, 70.0);
    assert_eq!(again.element("r").unwrap().extra["customData"]["keep"], true);
    assert_eq!(again.element("r").unwrap().stroke_color, "#e03131");
    assert_eq!(again.extra["future"]["keep"], "me");
    assert_eq!(again.app_state.view_background_color.as_deref(), Some("#ffffff"));
}

#[test]
fn bounds_frame_the_elements_not_the_origin() {
    let mut scene = Scene::default();
    let mut text = new_element(ElementType::Text, 200.0, 120.0, 90.0, 20.0, 1, NOW);
    text.text = Some("hola".into());
    scene.elements.push(text);
    scene.elements.push(new_element(ElementType::Rectangle, 40.0, 300.0, 100.0, 60.0, 2, NOW));
    let b = scene.bounds().unwrap();
    assert_eq!((b.x, b.y), (40.0, 120.0));
    assert_eq!((b.width, b.height), (250.0, 240.0));
    // Nothing painted is nothing to frame.
    assert!(Scene::default().bounds().is_none());
}

#[test]
fn bounds_follow_a_rotated_box() {
    let mut scene = Scene::default();
    let mut e = new_element(ElementType::Rectangle, 0.0, 0.0, 100.0, 0.0, 1, NOW);
    e.height = 20.0;
    // A quarter turn around the centre turns the 100×20 box on its side.
    e.angle = std::f64::consts::FRAC_PI_2;
    scene.elements.push(e);
    let b = scene.bounds().unwrap();
    assert!((b.width - 20.0).abs() < 1e-6, "{b:?}");
    assert!((b.height - 100.0).abs() < 1e-6, "{b:?}");
    assert!((b.x - 40.0).abs() < 1e-6 && (b.y + 40.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn a_click_hits_the_topmost_element_and_the_topmost_only() {
    let mut scene = Scene::default();
    scene.elements.push(new_element(ElementType::Rectangle, 0.0, 0.0, 100.0, 100.0, 1, NOW));
    scene.elements.push(new_element(ElementType::Ellipse, 50.0, 50.0, 100.0, 100.0, 2, NOW));
    assert_eq!(scene.hit(60.0, 60.0).map(|e| e.seed), Some(2), "the later element is on top");
    assert_eq!(scene.hit(10.0, 10.0).map(|e| e.seed), Some(1), "outside the ellipse, the box");
    assert!(scene.hit(500.0, 500.0).is_none());
    // A deleted element is not there to be clicked, even where it would paint.
    scene.elements[1].is_deleted = Some(true);
    assert_eq!(scene.hit(60.0, 60.0).map(|e| e.seed), Some(1));
}

#[test]
fn a_line_is_selected_by_its_stroke_not_by_the_box_around_it() {
    let mut scene = Scene::default();
    let mut line = new_element(ElementType::Line, 0.0, 0.0, 100.0, 100.0, 1, NOW);
    line.points = Some(vec![[0.0, 0.0], [100.0, 100.0]]);
    scene.elements.push(line);
    assert!(scene.hit(50.0, 50.0).is_some(), "on the diagonal");
    assert!(scene.hit(96.0, 3.0).is_none(), "the empty corner of the box is not the line");
}

#[test]
fn removed_elements_leave_the_scene_and_the_board() {
    let mut scene = scene_with(ElementType::Rectangle);
    scene.elements.push(new_element(ElementType::Ellipse, 200.0, 60.0, 80.0, 80.0, 2, NOW));
    assert_eq!(scene.display_list().ops.len(), 2);
    assert!(scene.remove(&scene.elements[0].id.clone()));
    assert_eq!(scene.display_list().ops.len(), 1);
    assert!(!scene.remove("nowhere"));
}

#[test]
fn a_scene_of_new_elements_survives_a_write_and_a_read() {
    let mut scene = Scene::default();
    for (i, kind) in KINDS.into_iter().enumerate() {
        let mut e: Element = new_element(kind.clone(), i as f64 * 30.0, 0.0, 25.0, 25.0, i as u64 + 1, NOW);
        if e.kind == ElementType::Text {
            e.text = Some(format!("t{i}"));
            e.original_text = e.text.clone();
        }
        scene.elements.push(e);
    }
    // The image is the one element whose op needs a file, so it is checked on its own above.
    let json = scene.to_json();
    let again = Scene::parse(&json).unwrap();
    assert_eq!(again.elements.len(), KINDS.len());
    assert_eq!(again, scene);
    assert_eq!(again.display_list().ops.len(), KINDS.len() - 1);
    assert_eq!(again.display_list().skipped.len(), 1);
}
