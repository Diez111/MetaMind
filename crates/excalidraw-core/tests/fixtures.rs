//! The fixtures: the two boards in the vault and one hand-written scene with a sample of every
//! element type. Each one has to come back unchanged.

use excalidraw_core::{ElementType, Scene};

const AREAS: &str = include_str!("fixtures/Tablero-Areas.excalidraw");
const SERIES: &str = include_str!("fixtures/Tablero-Series.excalidraw");
const KINDS: &str = include_str!("fixtures/kinds.excalidraw");

fn fixtures() -> [(&'static str, &'static str); 3] {
    [("Tablero-Areas", AREAS), ("Tablero-Series", SERIES), ("kinds", KINDS)]
}

#[test]
fn every_fixture_round_trips() {
    for (name, src) in fixtures() {
        let scene = Scene::parse(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = scene.to_json();
        let again = Scene::parse(&json).unwrap_or_else(|e| panic!("{name} read back: {e}"));
        assert_eq!(again, scene, "{name}: the model changed on the way out");
        assert_eq!(again.to_json(), json, "{name}: a second write differs from the first");
    }
}

#[test]
fn the_two_real_boards_are_text_and_a_frame() {
    for (name, src) in [("Tablero-Areas", AREAS), ("Tablero-Series", SERIES)] {
        let scene = Scene::parse(src).unwrap();
        let kinds: Vec<ElementType> = scene.elements.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(kinds.iter().filter(|k| **k == ElementType::Text).count() + 1, kinds.len());
        assert_eq!(kinds.iter().filter(|k| **k == ElementType::Rectangle).count(), 1);
        assert!(scene.files.is_empty());
        assert!(scene.elements.iter().all(|e| e.group_ids.is_empty() && e.points.is_none()));
        assert!(scene.source.is_some(), "{name} was written by Excalidraw itself");
        assert!(scene.elements.iter().all(|e| e.is_deleted != Some(true)));
    }
    assert_eq!(Scene::parse(AREAS).unwrap().elements.len(), 8);
    assert_eq!(Scene::parse(SERIES).unwrap().elements.len(), 7);
}

#[test]
fn the_hand_written_fixture_covers_the_hard_parts() {
    let scene = Scene::parse(KINDS).unwrap();
    let kinds: Vec<ElementType> = scene.elements.iter().map(|e| e.kind.clone()).collect();
    for kind in [
        ElementType::Rectangle,
        ElementType::Diamond,
        ElementType::Ellipse,
        ElementType::Text,
        ElementType::Arrow,
        ElementType::Line,
        ElementType::Freedraw,
        ElementType::Image,
    ] {
        assert!(kinds.contains(&kind), "{kind:?} is missing from the fixture");
    }
    // Rotated text.
    assert_eq!(scene.element("text-rot").unwrap().angle, 0.5);
    // Groups, bindings and an embedded image.
    assert_eq!(scene.element("rect-a").unwrap().group_ids, vec!["g1"]);
    assert_eq!(scene.element("freedraw-a").unwrap().group_ids, vec!["g2", "g3"]);
    let arrow = scene.element("arrow-a").unwrap();
    assert_eq!(arrow.start_binding.as_ref().unwrap().element_id, "rect-a");
    assert_eq!(arrow.end_binding.as_ref().unwrap().element_id, "diamond-a");
    let image = scene.element("image-a").unwrap();
    assert_eq!(image.file_id.as_deref(), Some("file-red"));
    assert!(scene.files["file-red"].data_url.starts_with("data:image/png;base64,iVBORw0KGgo"));
    // Unknown fields at every level, kept for the next release of the format.
    assert_eq!(scene.element("rect-a").unwrap().extra["customData"]["future"], true);
    assert_eq!(scene.element("frame-a").unwrap().extra["name"], "Marco");
    assert_eq!(scene.extra["futureTopLevelKey"]["keep"], "me");
    assert_eq!(scene.app_state.extra["zoom"]["value"], 1);
    // A deleted element and a type this crate does not paint are still in the model.
    assert_eq!(scene.element("gone").unwrap().is_deleted, Some(true));
    assert_eq!(scene.element("frame-a").unwrap().kind, ElementType::Other("frame".into()));
}
