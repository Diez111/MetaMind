use super::*;

const TINY: &str = r##"{
 "type": "excalidraw",
 "version": 2,
 "source": "https://example.org/export",
 "elements": [
  {"id": "r", "type": "rectangle", "x": 10, "y": 20, "width": 30, "height": 40, "angle": 0.5,
   "strokeColor": "#1971c2", "backgroundColor": "#a5d8ff", "fillStyle": "cross-hatch",
   "strokeWidth": 4, "strokeStyle": "dashed", "roughness": 2, "opacity": 50, "seed": 7,
   "groupIds": ["g1"], "frameId": "frm", "boundElements": [{"type": "text", "id": "t", "future": 1}],
   "updated": 1700000000000, "link": "https://example.org", "locked": true,
   "roundness": {"type": 3, "value": 32}, "customData": {"future": true}},
  {"id": "t", "type": "text", "x": 12, "y": 22, "width": 20, "height": 10, "text": "hola",
   "fontSize": 24, "fontFamily": 5, "textAlign": "center", "verticalAlign": "bottom",
   "containerId": "r", "originalText": "hola!", "lineHeight": 1.1, "autoResize": false},
  {"id": "a", "type": "arrow", "x": 100, "y": 100, "width": 40, "height": 20,
   "points": [[0, 0], [40, 20]], "startBinding": {"elementId": "r", "focus": 0.5, "gap": 4, "mode": "inside"},
   "endBinding": {"elementId": "t", "focus": 0, "gap": 2}, "startArrowhead": "triangle",
   "endArrowhead": "crowfoot", "elbowed": false}
 ],
 "appState": {"viewBackgroundColor": "#fafafa", "gridSize": 10, "zoom": {"value": 2}},
 "files": {"f1": {"dataURL": "data:image/png;base64,AAAA", "mimeType": "image/png", "created": 5, "futureFileKey": 1}},
 "futureTopLevel": {"keep": true}
}"##;

fn tiny() -> Scene {
    Scene::parse(TINY).unwrap()
}

#[test]
fn parse_keeps_every_field_it_models() {
    let scene = tiny();
    assert_eq!((scene.kind.as_str(), scene.version), ("excalidraw", 2));
    assert_eq!(scene.source.as_deref(), Some("https://example.org/export"));
    assert_eq!(scene.elements.len(), 3);

    let r = scene.element("r").unwrap();
    assert_eq!(r.kind, ElementType::Rectangle);
    assert_eq!((r.x, r.y, r.width, r.height, r.angle), (10.0, 20.0, 30.0, 40.0, 0.5));
    assert_eq!(r.fill_style, FillStyle::CrossHatch);
    assert_eq!(r.stroke_style, StrokeStyle::Dashed);
    assert_eq!((r.stroke_width, r.roughness, r.opacity, r.seed), (4.0, 2.0, 50.0, 7));
    assert_eq!(r.group_ids, vec!["g1"]);
    assert_eq!(r.frame_id.as_deref(), Some("frm"));
    assert_eq!(r.updated, Some(1700000000000));
    assert_eq!(r.link.as_deref(), Some("https://example.org"));
    assert!(r.locked);
    assert_eq!(r.bound_elements.as_ref().unwrap()[0].id, "t");
    assert_eq!(r.bound_elements.as_ref().unwrap()[0].kind, "text");

    let t = scene.element("t").unwrap();
    assert_eq!(t.kind, ElementType::Text);
    assert_eq!(t.font_family, Some(FontFamily::Other(5)));
    assert_eq!(t.text_align, Some(TextAlign::Center));
    assert_eq!(t.vertical_align, Some(VerticalAlign::Bottom));
    assert_eq!(t.container_id.as_deref(), Some("r"));
    assert_eq!(t.original_text.as_deref(), Some("hola!"));
    assert_eq!(t.auto_resize, Some(false));

    let a = scene.element("a").unwrap();
    assert_eq!(a.points, Some(vec![[0.0, 0.0], [40.0, 20.0]]));
    assert_eq!(a.start_binding.as_ref().unwrap().element_id, "r");
    assert_eq!(a.start_binding.as_ref().unwrap().focus, 0.5);
    assert_eq!(a.end_binding.as_ref().unwrap().gap, 2.0);
    assert_eq!(a.start_arrowhead, Some(Arrowhead::Triangle));
    assert_eq!(a.end_arrowhead, Some(Arrowhead::Other("crowfoot".into())));

    assert_eq!(scene.app_state.view_background_color.as_deref(), Some("#fafafa"));
    assert_eq!(scene.app_state.grid_size.as_ref().and_then(Value::as_f64), Some(10.0));
    assert_eq!(scene.files["f1"].mime_type, "image/png");
    assert_eq!(scene.files["f1"].created, 5);
}

#[test]
fn unknown_fields_survive_every_round_trip() {
    let scene = tiny();
    assert_eq!(scene.element("r").unwrap().extra["customData"]["future"], true);
    assert_eq!(scene.element("a").unwrap().extra["elbowed"], false);
    assert_eq!(scene.app_state.extra["zoom"]["value"], 2);
    assert_eq!(scene.extra["futureTopLevel"]["keep"], true);
    assert_eq!(scene.files["f1"].extra["futureFileKey"], 1);
    assert_eq!(scene.element("r").unwrap().bound_elements.as_ref().unwrap()[0].extra["future"], 1);
    assert_eq!(scene.element("a").unwrap().start_binding.as_ref().unwrap().extra["mode"], "inside");

    let json = scene.to_json();
    for key in ["customData", "futureTopLevel", "futureFileKey", "elbowed", "\"mode\""] {
        assert!(json.contains(key), "{key} was dropped");
    }
    let again = Scene::parse(&json).unwrap();
    assert_eq!(again, scene, "the model round-trips");
    assert_eq!(again.to_json(), json, "writing twice gives the same bytes");
}

#[test]
fn an_empty_file_is_an_empty_scene() {
    let scene = Scene::parse("  \n").unwrap();
    assert_eq!(scene, Scene::default());
    assert_eq!((scene.kind.as_str(), scene.version), ("excalidraw", 2));
    assert_eq!(Scene::parse(&scene.to_json()).unwrap(), scene);
}

#[test]
fn missing_keys_get_excalidraw_defaults() {
    let scene = Scene::parse(r#"{"elements":[{"id":"e","type":"rectangle","x":1,"y":2}]}"#).unwrap();
    let e = scene.element("e").unwrap();
    assert_eq!((e.width, e.height, e.angle), (0.0, 0.0, 0.0));
    assert_eq!(e.stroke_color, "#1e1e1e");
    assert_eq!(e.background_color, "transparent");
    assert_eq!((e.stroke_width, e.roughness, e.opacity), (2.0, 1.0, 100.0));
    assert_eq!(e.fill_style, FillStyle::Solid);
    assert!(!e.locked && e.is_deleted.is_none());
}

#[test]
fn values_this_version_does_not_know_are_kept() {
    let scene = Scene::parse(
        r#"{"elements":[
            {"id":"h","type":"hologram","x":0,"y":0},
            {"id":"r","type":"rectangle","fillStyle":"sparkle","strokeStyle":"wavy","fontFamily":9}
        ]}"#,
    )
    .unwrap();
    assert_eq!(scene.element("h").unwrap().kind, ElementType::Other("hologram".into()));
    let r = scene.element("r").unwrap();
    assert_eq!(r.fill_style, FillStyle::Other("sparkle".into()));
    assert_eq!(r.stroke_style, StrokeStyle::Other("wavy".into()));
    assert_eq!(r.font_family, Some(FontFamily::Other(9)));
    let again = Scene::parse(&scene.to_json()).unwrap();
    assert_eq!(again, scene);
    assert!(again.to_json().contains("\"sparkle\"") && again.to_json().contains("\"hologram\""));
}

#[test]
fn a_bad_known_field_names_the_element() {
    let err =
        Scene::parse(r#"{"elements":[{"id":"a","type":"rectangle"},{"id":"bad","type":"text","x":"no"}]}"#);
    let message = err.unwrap_err().to_string();
    assert!(message.contains("element 1 (id bad)"), "{message}");
    assert!(Scene::parse(r#"{"type":"obsidian","elements":[]}"#).is_err());
    assert!(Scene::parse(r#"{"elements":{}}"#).is_err());
    assert!(Scene::parse("[1]").is_err());
    assert!(Scene::parse("{nope").is_err());
}

#[test]
fn a_stale_app_state_field_costs_nothing() {
    // Excalidraw itself writes `gridSize: "small"` in older files: a string must not cost the
    // rest of the app state, so the field is carried as a `Value` and resolved by the viewer.
    let scene = Scene::parse(r##"{"appState":{"gridSize":"big","viewBackgroundColor":"#ffffff"}}"##).unwrap();
    assert_eq!(scene.app_state.view_background_color.as_deref(), Some("#ffffff"));
    assert_eq!(scene.app_state.grid_size.as_ref().and_then(Value::as_str), Some("big"));
    assert!(!scene.app_state.extra.contains_key("gridSize"), "a known key is typed, not extra");
    assert_eq!(Scene::parse(&scene.to_json()).unwrap(), scene);
}

#[test]
fn a_broken_app_state_keeps_every_field() {
    // A field of the wrong shape falls back to `extra` instead of losing the object.
    let scene = Scene::parse(r##"{"appState":{"viewBackgroundColor":42,"zoom":{"value":2}}}"##).unwrap();
    assert_eq!(scene.app_state.view_background_color, None);
    assert_eq!(scene.app_state.extra["viewBackgroundColor"], 42);
    assert_eq!(scene.app_state.extra["zoom"]["value"], 2);
}

#[test]
fn colours() {
    let hex = |r, g, b, a| Rgba { r, g, b, a };
    assert_eq!(Rgba::parse("#1e1e1e"), Some(hex(30, 30, 30, 255)));
    assert_eq!(Rgba::parse("#abc"), Some(hex(170, 187, 204, 255)));
    assert_eq!(Rgba::parse("#1e1e1e80"), Some(hex(30, 30, 30, 128)));
    assert_eq!(Rgba::parse("transparent"), Some(hex(0, 0, 0, 0)));
    assert_eq!(Rgba::parse("#12345"), None);
    assert_eq!(Rgba::parse("var(--color)"), None);
    assert_eq!(hex(30, 30, 30, 255).to_string(), "#1e1e1e");
    assert_eq!(hex(30, 30, 30, 128).to_string(), "#1e1e1e80");
    assert_eq!(hex(0, 0, 0, 0).to_string(), "transparent");
}

#[test]
fn the_display_list_reports_what_it_drops() {
    let scene = Scene::parse(
        r##"{"appState":{"viewBackgroundColor":"#ffffff"},"elements":[
            {"id":"ok","type":"ellipse","x":0,"y":0,"width":10,"height":10,"strokeColor":"#ff0000"},
            {"id":"gone","type":"rectangle","isDeleted":true},
            {"id":"frame","type":"frame"},
            {"id":"empty","type":"text","text":""},
            {"id":"one","type":"line","points":[[0,0]]},
            {"id":"img","type":"image","fileId":"nope"}
        ]}"##,
    )
    .unwrap();
    let list = scene.display_list();
    assert_eq!(list.background, Some(Rgba { r: 255, g: 255, b: 255, a: 255 }));
    assert_eq!(list.ops.len(), 1);
    assert_eq!(
        list.skipped,
        vec![
            ("gone".to_string(), Skip::Deleted),
            ("frame".to_string(), Skip::Type("frame".into())),
            ("empty".to_string(), Skip::EmptyText),
            ("one".to_string(), Skip::NoGeometry),
            ("img".to_string(), Skip::MissingFile("nope".into())),
        ]
    );
    assert_eq!(
        list.dump(),
        "background #ffffff\n\
         ellipse 10x10 at 0,0 angle=0 stroke=#ff0000/2/solid fill=none opacity=1 sketchy\n\
         skip gone deleted\n\
         skip frame type=frame\n\
         skip empty empty-text\n\
         skip one no-geometry\n\
         skip img missing-file=nope\n"
    );
}

#[test]
fn style_is_carried_not_resolved() {
    let scene = tiny();
    let list = scene.display_list();
    let DrawOp::Rect(shape, style) = &list.ops[0] else { panic!("first op is the rectangle") };
    assert_eq!((shape.width, shape.height, shape.angle), (30.0, 40.0, 0.5));
    assert_eq!(style.stroke, Some(Rgba { r: 0x19, g: 0x71, b: 0xc2, a: 255 }));
    assert_eq!(style.fill, Some(Rgba { r: 0xa5, g: 0xd8, b: 0xff, a: 255 }));
    assert_eq!(style.fill_style, FillStyle::CrossHatch);
    assert_eq!(style.stroke_style, StrokeStyle::Dashed);
    assert_eq!((style.stroke_width, style.opacity), (4.0, 0.5));
    assert!(style.sketchy, "roughness 2 asks for a hand-drawn stroke");

    // Excalidraw stores `points` relative to the element, the display list wants scene coordinates.
    let DrawOp::Polyline(line) = &list.ops[2] else { panic!("third op is the arrow") };
    assert_eq!(line.points, vec![(100.0, 100.0), (140.0, 120.0)]);
    assert_eq!(line.start_head, Some(Arrowhead::Triangle));
    assert_eq!(line.end_head, Some(Arrowhead::Other("crowfoot".into())));

    // Text runs keep the box Excalidraw laid out, plus the family and alignment.
    let DrawOp::TextRun(text) = &list.ops[1] else { panic!("second op is the text") };
    assert_eq!(
        (
            text.text.as_str(),
            text.size,
            text.line_height,
            text.family,
            text.align.clone(),
            text.vertical.clone()
        ),
        ("hola", 24.0, 1.1, FontFamily::Other(5), TextAlign::Center, VerticalAlign::Bottom)
    );
    assert_eq!(text.colour, Some(Rgba { r: 30, g: 30, b: 30, a: 255 }), "text uses the stroke colour");
    assert!(list.skipped.is_empty());
}

#[test]
fn an_image_op_points_at_its_embedded_file() {
    let scene = Scene::parse(
        r##"{"elements":[{"id":"i","type":"image","x":5,"y":6,"width":7,"height":8,"fileId":"f1","scale":[2,3]}],
            "files":{"f1":{"dataURL":"data:image/png;base64,AAAA","mimeType":"image/png","created":1}}}"##,
    )
    .unwrap();
    let list = scene.display_list();
    let DrawOp::ImageRef(image) = &list.ops[0] else { panic!("the image produced an op") };
    assert_eq!((image.file_id.as_str(), image.mime_type.as_deref()), ("f1", Some("image/png")));
    assert_eq!((image.width, image.height, image.scale), (7.0, 8.0, Some((2.0, 3.0))));
    assert_eq!(scene.files["f1"].data_url, "data:image/png;base64,AAAA");
}

#[test]
fn a_missing_opacity_stays_opaque() {
    let scene = Scene::parse(r#"{"elements":[{"id":"e","type":"rectangle"}]}"#).unwrap();
    let DrawOp::Rect(_, style) = &scene.display_list().ops[0] else { panic!("one rect") };
    assert_eq!(style.opacity, 1.0, "Excalidraw's own default is 100%, not 0");
}
