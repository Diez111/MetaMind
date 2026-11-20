//! The `.canvas` export: what a scene becomes for `canvas-core`, and what the format cannot hold.

use canvas_core::{Canvas, NodeKind};
use excalidraw_core::{to_canvas, Scene};
use std::collections::HashSet;

const AREAS: &str = include_str!("fixtures/Tablero-Areas.excalidraw");
const SERIES: &str = include_str!("fixtures/Tablero-Series.excalidraw");
const KINDS: &str = include_str!("fixtures/kinds.excalidraw");

/// A readable rendering of the export without the hashed ids: nodes by index, edges by the text of
/// the nodes they join.
fn dump(c: &Canvas) -> String {
    let text = |id: &str| {
        let node = c.node(id).expect("an edge end that is a node");
        match &node.kind {
            NodeKind::Text { text } => text.clone(),
            other => panic!("unexpected node {other:?}"),
        }
    };
    let mut out = String::new();
    for (i, n) in c.nodes.iter().enumerate() {
        out.push_str(&format!(
            "node {i} {:?} at {},{} {}x{} color={}\n",
            text(&n.id),
            n.x,
            n.y,
            n.width,
            n.height,
            n.color.as_deref().unwrap_or("none")
        ));
    }
    for e in &c.edges {
        out.push_str(&format!(
            "edge {:?} -> {:?} label={} color={}\n",
            text(&e.from_node),
            text(&e.to_node),
            e.label.as_deref().unwrap_or("none"),
            e.color.as_deref().unwrap_or("none")
        ));
    }
    out
}

#[test]
fn tablero_areas_becomes_a_column_of_text_nodes() {
    let canvas = to_canvas(&Scene::parse(AREAS).unwrap());
    assert_eq!((canvas.nodes.len(), canvas.edges.len()), (8, 0), "seven texts and the empty box");
    assert_eq!(
        dump(&canvas),
        r##"node 0 "Áreas y TFC — tablero (dibujá la región antes de integrar)" at 60,40 900x40 color=#1971c2
node 1 "Pasos: graficar · TODAS las intersecciones · quién está arriba · integrar (arriba−abajo) · chequear signo" at 60,100 900x25 color=#868e96
node 2 "D1.  ∫₀² |x−1| dx" at 60,150 900x25 color=#1e1e1e
node 3 "D5.  área entre y = x³ − x y el eje x" at 60,184 900x25 color=#1e1e1e
node 4 "D6.  hallar a > 0 con área 36 entre y = 2x−6 y el eje x en [0; a]" at 60,218 900x25 color=#1e1e1e
node 5 "D4.  d/dx ∫₀^(x²) sen t dt" at 60,252 900x25 color=#1e1e1e
node 6 "" at 60,306 900x620 color=#1e1e1e
node 7 "Resolvé acá" at 80,316 400x20 color=#adb5bd
"##
    );
}

#[test]
fn tablero_series_becomes_a_column_of_text_nodes() {
    let canvas = to_canvas(&Scene::parse(SERIES).unwrap());
    assert_eq!((canvas.nodes.len(), canvas.edges.len()), (7, 0));
    assert_eq!(
        dump(&canvas),
        r##"node 0 "Series de potencias — tablero" at 60,40 900x40 color=#1971c2
node 1 "4 pasos: 1) a_n (x−c)^n  2) R = 1/L  3) (c−R; c+R)  4) los dos extremos" at 60,100 900x25 color=#868e96
node 2 "A1.  Σ x^n / (n·3^n)          → radio, intervalo y los DOS extremos" at 60,150 900x25 color=#1e1e1e
node 3 "A5.  Σ (x−1)^n / (2^n·√n)     → idem (ojo: los extremos pueden dar distinto)" at 60,184 900x25 color=#1e1e1e
node 4 "A6.  Σ (x−1)^n / (k^n·n)  con R = 3     → hallar k > 0" at 60,218 900x25 color=#1e1e1e
node 5 "" at 60,272 900x620 color=#1e1e1e
node 6 "Resolvé acá" at 80,282 400x20 color=#adb5bd
"##
    );
}

#[test]
fn shapes_become_labelled_nodes_and_bound_arrows_become_edges() {
    let canvas = to_canvas(&Scene::parse(KINDS).unwrap());
    assert_eq!(
        dump(&canvas),
        r##"node 0 "Etiqueta\nrectángulo" at 40,60 260x120 color=#1971c2
node 1 "" at 360,60 200x140 color=#e03131
node 2 "" at 640,80 180x100 color=#2f9e44
node 3 "Texto rotado" at 380,300 300x60 color=#f08c00
edge "Etiqueta\nrectángulo" -> "" label=flecha color=#e03131
"##
    );
    // The label of a shape or an arrow is part of its host, not a node of its own.
    assert!(canvas.nodes.iter().all(|n| match &n.kind {
        NodeKind::Text { text } => text != "flecha",
        _ => false,
    }));
    // Freedraw, image and frame have no JSON Canvas form.
    assert!(canvas.nodes.iter().all(|n| n.width != 120.0), "the image was dropped");
    assert_eq!(canvas.edges[0].label.as_deref(), Some("flecha"));
    assert!(canvas.edges[0].to_arrow, "canvas edges point at their target");
}

#[test]
fn every_export_is_a_canvas_file_canvas_core_can_read() {
    for (name, src) in [("Tablero-Areas", AREAS), ("Tablero-Series", SERIES), ("kinds", KINDS)] {
        let canvas = to_canvas(&Scene::parse(src).unwrap());
        let json = canvas.to_json();
        let again = Canvas::parse(&json)
            .unwrap_or_else(|e| panic!("{name}: canvas-core refused our own output: {e}"));
        assert_eq!(again.to_json(), json, "{name}: the file does not round-trip through canvas-core");
        assert_eq!(
            (again.nodes.len(), again.edges.len()),
            (canvas.nodes.len(), canvas.edges.len()),
            "{name}"
        );

        // What JSON Canvas asks of an id: 16 hex digits, one per node or edge.
        let ids: Vec<&str> = canvas
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .chain(canvas.edges.iter().map(|e| e.id.as_str()))
            .collect();
        assert_eq!(ids.len(), ids.iter().collect::<HashSet<_>>().len(), "{name}: duplicate ids");
        assert!(
            ids.iter().all(|id| id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())),
            "{name}: ids are not 16 hex digits"
        );
        for edge in &canvas.edges {
            assert!(canvas.node(&edge.from_node).is_some(), "{name}: edge start is not a node");
            assert!(canvas.node(&edge.to_node).is_some(), "{name}: edge end is not a node");
        }
    }
}

#[test]
fn the_export_is_stable() {
    let scene = Scene::parse(KINDS).unwrap();
    let first = to_canvas(&scene).to_json();
    assert_eq!(to_canvas(&Scene::parse(&scene.to_json()).unwrap()).to_json(), first);
}
