use super::*;

fn draw(src: &str) -> Diagram {
    render(src, &mut Approx).unwrap()
}

fn texts(d: &Diagram) -> Vec<&str> {
    d.items
        .iter()
        .filter_map(|i| match i {
            Item::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn text_pos(d: &Diagram, t: &str) -> (f32, f32) {
    d.items
        .iter()
        .find_map(|i| match i {
            Item::Text { text, x, y, .. } if text == t => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text {t}"))
}

#[test]
fn top_down_layers_stack_vertically() {
    let d =
        draw("graph TD\n  A[Start] --> B(Next)\n  B --> C{Choice}\n  C -->|yes| D((Done))\n  C -->|no| A");
    for t in ["Start", "Next", "Choice", "Done", "yes", "no"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    let (a, b, c) = (text_pos(&d, "Start"), text_pos(&d, "Next"), text_pos(&d, "Choice"));
    assert!(a.1 < b.1 && b.1 < c.1);
    assert!(d.items.iter().any(|i| matches!(i, Item::Diamond { .. })));
    assert!(d.items.iter().any(|i| matches!(i, Item::Ellipse { .. })));
    assert!(d.items.iter().filter(|i| matches!(i, Item::Arrow { .. })).count() >= 4);
    assert!(d.width > 0.0 && d.height > 0.0);
}

#[test]
fn left_right_flows_horizontally() {
    let d = draw("flowchart LR\n A --> B --> C");
    let (a, c) = (text_pos(&d, "A"), text_pos(&d, "C"));
    assert!(a.0 < c.0 && (a.1 - c.1).abs() < 1.0);
    let d = draw("flowchart RL\n A --> B");
    assert!(text_pos(&d, "A").0 > text_pos(&d, "B").0);
    let d = draw("flowchart BT\n A --> B");
    assert!(text_pos(&d, "A").1 > text_pos(&d, "B").1);
}

#[test]
fn edge_syntaxes() {
    let d = draw("graph LR\n A -- text --> B\n B -.-> C\n C ==> D\n D --- E\n E & F --> G\n G <--> H");
    assert!(texts(&d).contains(&"text"));
    let dashed = d.items.iter().any(|i| matches!(i, Item::Line { dashed: true, .. }));
    let thick = d.items.iter().any(|i| matches!(i, Item::Line { width, .. } if *width > 2.0));
    assert!(dashed && thick);
    assert!(texts(&d).contains(&"H"));
}

#[test]
fn cycles_and_self_loops_terminate() {
    let d = draw("graph TD\n A --> B\n B --> C\n C --> A\n A --> A\n A --> C");
    assert!(d.width.is_finite() && d.height.is_finite());
}

#[test]
fn long_edges_get_routed_around() {
    let d = draw("graph TD\n A --> B --> C --> D\n A --> D");
    let routed = d.items.iter().any(|i| matches!(i, Item::Line { pts, .. } if pts.len() > 2));
    assert!(routed);
}

#[test]
fn subgraphs_and_styles_are_tolerated() {
    let d = draw("graph TD\n subgraph one [First]\n A --> B\n end\n classDef x fill:#f9f\n class A x\n style B fill:#333\n B --> C");
    assert!(texts(&d).contains(&"First"));
}

#[test]
fn multiline_and_quoted_labels() {
    let d = draw("graph TD\n A[\"Hello (world)\"] --> B[line one<br/>line two]");
    assert!(texts(&d).contains(&"Hello (world)"));
    assert!(texts(&d).contains(&"line one") && texts(&d).contains(&"line two"));
}

#[test]
fn sequence_diagram() {
    let d = draw("sequenceDiagram\n participant A as Alice\n actor B\n A->>B: Hello\n B-->>A: Hi back\n Note over A,B: thinking\n loop every day\n A->>A: self\n end");
    for t in ["Alice", "B", "Hello", "Hi back", "thinking", "self"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    let (h, r) = (text_pos(&d, "Hello"), text_pos(&d, "Hi back"));
    assert!(h.1 < r.1);
    assert!(d.items.iter().any(|i| matches!(i, Item::Line { dashed: true, .. })));
}

#[test]
fn unsupported_and_invalid_are_reported() {
    assert_eq!(render("pie title x\n \"a\": 1", &mut Approx).unwrap_err(), Error::Unsupported("pie".into()));
    assert!(matches!(render("graph TD\n A --> ", &mut Approx), Err(Error::Syntax(_))));
    assert!(render("", &mut Approx).is_err());
    assert!(matches!(render("graph XX\n A-->B", &mut Approx), Err(Error::Syntax(_))));
}

#[test]
fn frontmatter_and_comments_are_skipped() {
    let d = draw("---\ntitle: t\n---\n%% note\ngraph TD\n A --> B");
    assert!(texts(&d).contains(&"B"));
}

/// The boxes of the nodes, which is what a reader sees as cards.
fn node_boxes(d: &Diagram) -> Vec<(f32, f32, f32, f32)> {
    d.items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { x, y, w, h, fill: Fill::Node, .. } => Some((*x, *y, *w, *h)),
            Item::Diamond { cx, cy, w, h, .. } => Some((cx - w / 2.0, cy - h / 2.0, *w, *h)),
            Item::Ellipse { cx, cy, rx, ry, .. } => Some((cx - rx, cy - ry, rx * 2.0, ry * 2.0)),
            _ => None,
        })
        .collect()
}

fn overlap(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

#[test]
fn node_boxes_never_overlap() {
    // The shape of the decision tree that overflowed in the app: many long labels over several
    // layers of a top-down flow.
    let d = draw(
        "graph TD\n\
           Q[Que te dan en el enunciado?] --> A[La formula de la funcion]\n\
           Q --> B[El polinomio ya armado]\n\
           A --> C[Derivar el polinomio en el centro]\n\
           B --> C\n\
           C --> D[Piden error o tolerancia?]\n\
           D -->|Si| E[Centro 0: Maclaurin]\n\
           D -->|No| F[Centro c distinto de 0]\n\
           E --> G[No: alcanza con el polinomio y un termino]\n\
           F --> G\n\
           G --> H[Es composicion o producto?]\n\
           H -->|Composicion| I[Regla de la cadena]\n\
           H -->|Producto| J[Regla del producto]\n\
           H -->|Ninguna| K[Funcion simple]\n",
    );
    let boxes = node_boxes(&d);
    assert!(boxes.len() >= 10, "{boxes:?}");
    for (i, a) in boxes.iter().enumerate() {
        for (j, b) in boxes.iter().enumerate().skip(i + 1) {
            assert!(!overlap(*a, *b), "boxes {i} and {j} overlap: {a:?} {b:?}");
        }
    }
}

#[test]
fn edges_are_painted_before_the_boxes_they_join() {
    // Draw order is the order of the commands: a box has to come after every line that reaches
    // it, or the line is drawn over its text.
    let d = draw("graph LR\n  A[one] --> B[two]\n  B --> C[three]\n  B --> A\n");
    let last_line = d.items.iter().rposition(|i| matches!(i, Item::Line { .. })).unwrap();
    let first_box = d.items.iter().position(|i| matches!(i, Item::Rect { fill: Fill::Node, .. })).unwrap();
    assert!(last_line < first_box, "lines at {last_line}, boxes from {first_box}");
}

#[test]
fn a_diagram_of_long_labels_is_narrow_enough_to_fit_a_column() {
    // Labels wrap instead of growing without end: a diagram of prose labels has to stay inside
    // the width of a note column at a size the text can still be read at.
    let d = draw(
        "graph TD\n\
           A[Derivar el polinomio en el centro y evaluar] --> B[Piden error o tolerancia explicita]\n\
           B --> C[No: alcanza con el polinomio y un termino]\n\
           C --> D[Comprobar el resultado con la formula]\n",
    );
    // Wide enough for the labels, narrow enough that fitting it to 644 px keeps the text over
    // 11 px (`TEXT_SIZE` is 14).
    assert!(d.width <= 644.0 / (11.0 / TEXT_SIZE), "a {} px wide diagram is too wide", d.width);
}
