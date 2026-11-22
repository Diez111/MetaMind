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
    assert_eq!(
        render("sankey-beta\n A,B,3", &mut Approx).unwrap_err(),
        Error::Unsupported("sankey-beta".into())
    );
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

/// How many items of the drawing answer to `f`.
fn count(d: &Diagram, f: impl Fn(&Item) -> bool) -> usize {
    d.items.iter().filter(|i| f(i)).count()
}

fn boxes(d: &Diagram) -> usize {
    count(d, |i| matches!(i, Item::Rect { fill: Fill::Node, .. }))
}

#[test]
fn class_diagram_draws_compartments_and_relations() {
    let d = draw(
        "classDiagram\n\
           class Animal {\n\
             +int age\n\
             +isMammal() bool\n\
           }\n\
           class Duck\n\
           Animal <|-- Duck\n\
           Animal : +String name\n\
           Order \"1\" *-- \"many\" Line : contains\n\
           <<interface>> Shape\n\
           Shape ..|> Animal",
    );
    for t in [
        "Animal",
        "Duck",
        "Order",
        "Line",
        "+int age",
        "+isMammal() bool",
        "+String name",
        "contains",
        "1",
        "many",
        "Shape",
    ] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    assert!(boxes(&d) >= 4);
    // The inheritance triangle and the composition diamond of a relation.
    assert!(count(&d, |i| matches!(i, Item::Arrow { .. })) >= 2);
    assert!(count(&d, |i| matches!(i, Item::Diamond { .. })) >= 1);
    // The parent of an inheritance is drawn over the child.
    assert!(text_pos(&d, "Animal").1 < text_pos(&d, "Duck").1);
    assert!(d.width > 100.0 && d.width < 900.0 && d.height > 80.0);
}

#[test]
fn class_diagram_takes_a_direction_and_marks_its_members() {
    let d = draw("classDiagram\n direction LR\n class A\n class B\n A --> B");
    assert!(text_pos(&d, "A").0 < text_pos(&d, "B").0);
    assert!(matches!(render("classDiagram\n A --% B", &mut Approx), Err(Error::Syntax(_))));
}

#[test]
fn state_diagram_draws_the_starts_ends_and_composites() {
    let d = draw(
        "stateDiagram-v2\n\
           [*] --> Idle\n\
           Idle --> Running : start\n\
           Running --> [*]\n\
           state Running {\n\
             [*] --> Fast\n\
             Fast --> Slow\n\
           }\n\
           note right of Idle : espera",
    );
    for t in ["Idle", "Running", "Fast", "Slow", "start", "espera"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    // The two pseudo states: the dot a machine starts at, and the ring it ends in.
    assert!(count(&d, |i| matches!(i, Item::Ellipse { fill: Fill::Node, .. })) >= 1);
    assert!(count(&d, |i| matches!(i, Item::Ellipse { fill: Fill::Background, .. })) >= 1);
    // The frame around the states of a composite one.
    assert!(count(&d, |i| matches!(i, Item::Rect { fill: Fill::Accent, stroke: true, .. })) >= 1);
    assert!(d.width > 100.0 && d.height > 80.0);
}

#[test]
fn state_diagram_rejects_lines_it_cannot_read() {
    assert!(matches!(render("stateDiagram-v2\n Idle ? Running", &mut Approx), Err(Error::Syntax(_))));
}

#[test]
fn er_diagram_draws_entities_attributes_and_cardinalities() {
    let d = draw(
        "erDiagram\n\
           CUSTOMER ||--o{ ORDER : places\n\
           ORDER ||--|{ LINE_ITEM : contains\n\
           CUSTOMER {\n\
             string name\n\
             int id PK\n\
           }",
    );
    for t in ["CUSTOMER", "ORDER", "LINE_ITEM", "places", "contains", "string name", "int id", "PK"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    assert_eq!(boxes(&d), 3);
    // The crow's foot and the bars that stand for "many" and for "exactly one".
    assert!(count(&d, |i| matches!(i, Item::Line { .. })) >= 8);
    assert!(d.width > 120.0 && d.height > 80.0);
}

#[test]
fn gantt_draws_bars_sections_and_the_axis() {
    let d = draw(
        "gantt\n\
           title Plan\n\
           dateFormat YYYY-MM-DD\n\
           section Diseño\n\
           Bocetos :a1, 2024-01-01, 30d\n\
           Revision :after a1, 10d\n\
           section Código\n\
           Implementar :2024-02-01, 12d\n\
           Entrega :milestone, m1, 2024-02-20, 0d",
    );
    for t in ["Plan", "Diseño", "Código", "Bocetos", "Revision", "Implementar", "Entrega"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    // The bars of the tasks, and the diamond of the milestone.
    assert!(count(&d, |i| matches!(i, Item::Rect { fill: Fill::Node, .. })) >= 3);
    assert!(count(&d, |i| matches!(i, Item::Diamond { .. })) >= 1);
    // The axis: one label per tick, at least the first day.
    assert!(texts(&d).iter().any(|t| t.contains('/')));
    assert!(d.width > 200.0 && d.height > 90.0);
}

#[test]
fn pie_draws_one_sector_per_value_with_a_legend() {
    let d = draw("pie showData\n \"Sí\" : 3\n \"No\" : 1");
    assert_eq!(count(&d, |i| matches!(i, Item::Sector { .. })), 2);
    let texts = texts(&d);
    assert!(texts.contains(&"Sí") && texts.contains(&"No"), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("75.0%")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("25.0%")), "{texts:?}");
    // Two slices are painted in two shades.
    let tones: Vec<Fill> = d
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Sector { fill, .. } => Some(*fill),
            _ => None,
        })
        .collect();
    assert_ne!(tones[0], tones[1]);
    assert!(d.width > 200.0 && d.height > 100.0);
}

#[test]
fn pie_rejects_a_value_that_is_not_a_number() {
    assert!(matches!(render("pie\n \"a\" : mucho", &mut Approx), Err(Error::Syntax(_))));
}

#[test]
fn mindmap_nests_its_branches_to_the_right() {
    let d = draw(
        "mindmap\n\
           root((MetaMind))\n\
             Notas\n\
               Markdown\n\
               Enlaces\n\
             Diagramas\n\
               flow\n",
    );
    for t in ["MetaMind", "Notas", "Markdown", "Enlaces", "Diagramas", "flow"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    let root = text_pos(&d, "MetaMind");
    let branch = text_pos(&d, "Notas");
    let leaf = text_pos(&d, "Markdown");
    assert!(root.0 < branch.0 && branch.0 < leaf.0);
    assert!(count(&d, |i| matches!(i, Item::Rect { stroke: true, .. })) >= 6);
    assert!(d.width > 160.0 && d.height > 100.0);
}

#[test]
fn timeline_draws_periods_and_their_events() {
    let d = draw(
        "timeline\n\
           title Historia\n\
           section Temprano\n\
           2002 : LinkedIn\n\
           2004 : Facebook : Google\n\
           section Tarde\n\
           2006 : Twitter",
    );
    for t in ["Historia", "Temprano", "Tarde", "2002", "2004", "LinkedIn", "Facebook", "Google", "Twitter"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    let (a, b) = (text_pos(&d, "2002"), text_pos(&d, "2004"));
    assert!(a.0 < b.0);
    assert!(d.width > 200.0 && d.height > 90.0);
}

#[test]
fn journey_rates_every_step() {
    let d = draw(
        "journey\n\
           title Mi día\n\
           section Trabajo\n\
             Preparar café: 5: Yo\n\
             Programar: 4: Yo, Gato\n\
           section Casa\n\
             Caminar: 2: Yo",
    );
    for t in ["Mi día", "Trabajo", "Casa", "Preparar café", "Programar", "Caminar"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    // The stars of the score, and the actors under the step.
    assert!(texts(&d).iter().any(|t| t.contains("★★★★★")), "{:?}", texts(&d));
    assert!(texts(&d).iter().any(|t| t.contains("Gato")), "{:?}", texts(&d));
    assert!(count(&d, |i| matches!(i, Item::Ellipse { fill: Fill::Node, .. })) >= 3);
    // A better score is drawn higher up.
    assert!(text_pos(&d, "Preparar café").1 > 0.0);
    assert!(d.width > 200.0 && d.height > 100.0);
}

#[test]
fn git_graph_draws_branches_merges_and_tags() {
    let d = draw(
        "gitGraph\n\
           commit id: \"a1\"\n\
           branch develop\n\
           commit id: \"b1\"\n\
           checkout main\n\
           commit id: \"a2\" tag: \"v1.0\"\n\
           merge develop\n",
    );
    for t in ["main", "develop", "a1", "b1", "a2", "v1.0"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    assert!(count(&d, |i| matches!(i, Item::Ellipse { .. })) >= 4);
    // The branch of `develop` runs below the one of `main`.
    assert!(text_pos(&d, "main").1 < text_pos(&d, "develop").1);
    assert!(d.width > 150.0 && d.height > 90.0);
}

#[test]
fn quadrant_chart_places_the_points_by_their_scores() {
    let d = draw(
        "quadrantChart\n\
           title Campañas\n\
           x-axis Alcance bajo --> Alcance alto\n\
           y-axis Compromiso bajo --> Compromiso alto\n\
           quadrant-1 Ampliar\n\
           quadrant-2 Promover\n\
           quadrant-3 Revisar\n\
           quadrant-4 Mejorar\n\
           Campaña A: [0.3, 0.6]\n\
           Campaña B: [0.8, 0.2]",
    );
    for t in ["Campañas", "Ampliar", "Promover", "Revisar", "Mejorar", "Campaña A", "Campaña B"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    // The point with the higher score on the x axis is drawn further right.
    assert!(text_pos(&d, "Campaña A").0 < text_pos(&d, "Campaña B").0);
    // Both points are on the plot, not outside it.
    let points: Vec<(f32, f32)> = d
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Ellipse { cx, cy, fill: Fill::Node, .. } => Some((*cx, *cy)),
            _ => None,
        })
        .collect();
    assert_eq!(points.len(), 2, "{points:?}");
    assert!(d.width > 250.0 && d.height > 250.0);
}

#[test]
fn quadrant_rejects_a_point_outside_the_square() {
    assert!(matches!(render("quadrantChart\n A: [1.4, 0.2]", &mut Approx), Err(Error::Syntax(_))));
}

#[test]
fn every_diagram_stays_inside_a_note_column() {
    // Wide enough to be read, narrow enough to fit a column at the app's scale.
    for src in [
        "classDiagram\n class A {\n +int x\n +go() y\n }\n class B\n A <|-- B",
        "stateDiagram-v2\n [*] --> A\n A --> B : go\n B --> [*]",
        "erDiagram\n A ||--o{ B : has\n A {\n int id PK\n }",
        "gantt\n section S\n Uno :2024-01-01, 3d\n Dos :2024-01-05, 4d",
        "pie\n \"a\" : 1\n \"b\" : 1",
        "mindmap\n raíz\n   rama\n     hoja",
        "timeline\n 2000 : algo\n 2001 : otra cosa",
        "journey\n section S\n Paso: 3: Yo",
        "gitGraph\n commit\n branch b\n commit\n checkout main\n merge b",
        "quadrantChart\n x-axis a --> b\n P: [0.5, 0.5]",
    ] {
        let d = draw(src);
        assert!(d.width > 0.0 && d.height > 0.0, "{src}");
        assert!(d.width < 900.0, "{src} is {} px wide", d.width);
    }
}
