//! Temporary: draws the diagrams of the new kinds to SVG so they can be looked at.
use std::fmt::Write as _;

fn mix(bg: (f32, f32, f32), fg: (f32, f32, f32), t: f32) -> String {
    let f = |a: f32, b: f32| (a + (b - a) * t).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", f(bg.0, fg.0), f(bg.1, fg.1), f(bg.2, fg.2))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn svg(d: &mermaid_core::Diagram) -> String {
    let bg = (30.0, 30.0, 30.0);
    let fg = (220.0, 220.0, 220.0);
    let node = mix(bg, fg, 0.12);
    let accent = mix(bg, fg, 0.05);
    let line_c = mix(bg, fg, 0.8);
    let stroke = mix(bg, fg, 0.55);
    let fill = |f: mermaid_core::Fill| match f {
        mermaid_core::Fill::Node => node.clone(),
        mermaid_core::Fill::Accent => accent.clone(),
        mermaid_core::Fill::Tone(n) => mix(bg, fg, 0.06 + 0.06 * f32::from(n.min(6))),
        _ => mix(bg, fg, 0.0),
    };
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {:.0} {:.0}"><rect width="100%" height="100%" fill="{}"/><g font-family="DejaVu Sans">"#,
        d.width, d.height, d.width, d.height, mix(bg, fg, 0.0)
    );
    for it in &d.items {
        match it {
            mermaid_core::Item::Rect { x, y, w, h, radius, fill: f, stroke: st } => {
                let _ = write!(
                    s,
                    r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="{radius:.1}" fill="{}" stroke="{}" stroke-width="1.2"/>"#,
                    fill(*f),
                    if *st { stroke.clone() } else { "none".to_string() }
                );
            }
            mermaid_core::Item::Diamond { cx, cy, w, h, fill: f } => {
                let (top, bottom, left, right) = (cy - h / 2.0, cy + h / 2.0, cx - w / 2.0, cx + w / 2.0);
                let _ = write!(
                    s,
                    r#"<polygon points="{cx:.1},{top:.1} {right:.1},{cy:.1} {cx:.1},{bottom:.1} {left:.1},{cy:.1}" fill="{}" stroke="{}" stroke-width="1.2"/>"#,
                    fill(*f),
                    stroke
                );
            }
            mermaid_core::Item::Ellipse { cx, cy, rx, ry, fill: f } => {
                let _ = write!(
                    s,
                    r#"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{rx:.1}" ry="{ry:.1}" fill="{}" stroke="{}" stroke-width="1.2"/>"#,
                    fill(*f),
                    stroke
                );
            }
            mermaid_core::Item::Sector { cx, cy, r, from, to, fill: f } => {
                let large = if (to - from).abs() > std::f32::consts::PI { 1 } else { 0 };
                let _ = write!(
                    s,
                    r#"<path d="M {cx:.1} {cy:.1} L {:.1} {:.1} A {r:.1} {r:.1} 0 {large} 1 {:.1} {:.1} Z" fill="{}" stroke="{}" stroke-width="1"/>"#,
                    cx + r * from.cos(),
                    cy + r * from.sin(),
                    cx + r * to.cos(),
                    cy + r * to.sin(),
                    fill(*f),
                    stroke
                );
            }
            mermaid_core::Item::Line { pts, dashed, width } => {
                let path: Vec<String> = pts.iter().map(|p| format!("{:.1},{:.1}", p.0, p.1)).collect();
                let _ = write!(
                    s,
                    r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="{width:.1}"{}/>"#,
                    path.join(" "),
                    line_c,
                    if *dashed { r#" stroke-dasharray="5 4""# } else { "" }
                );
            }
            mermaid_core::Item::Arrow { x, y, angle, .. } => {
                let (c, sn) = (angle.cos(), angle.sin());
                let len = 9.0;
                let half = 3.8;
                let (bx, by) = (x - c * len, y - sn * len);
                let _ = write!(
                    s,
                    r#"<polygon points="{x:.1},{y:.1} {:.1},{:.1} {:.1},{:.1}" fill="{}"/>"#,
                    bx - sn * half,
                    by + c * half,
                    bx + sn * half,
                    by - c * half,
                    line_c
                );
            }
            mermaid_core::Item::Text { x, y, text, bold, size } => {
                let _ = write!(
                    s,
                    r#"<text x="{x:.1}" y="{y:.1}" font-size="{size:.1}" fill="{}" text-anchor="middle" dominant-baseline="central"{}>{}</text>"#,
                    mix(bg, fg, 1.0),
                    if *bold { r#" font-weight="bold""# } else { "" },
                    esc(text)
                );
            }
        }
    }
    s.push_str("</g></svg>");
    s
}

fn main() {
    let dir = std::path::Path::new("/tmp/mm-svg");
    std::fs::create_dir_all(dir).unwrap();
    let cases: &[(&str, &str)] = &[
        (
            "class",
            "classDiagram\n\
             class Animal {\n +int age\n +String gender\n +isMammal() bool\n +mate()\n }\n\
             class Duck {\n +String beakColor\n +swim()\n +quack()\n }\n\
             class Fish {\n -int sizeInFeet\n -canEat()\n }\n\
             class Zebra {\n +bool is_wild\n +run()\n }\n\
             Animal <|-- Duck\n Animal <|-- Fish\n Animal <|-- Zebra\n\
             Animal : +int age\n Duck \"1\" *-- \"many\" Fish : nada\n",
        ),
        (
            "state",
            "stateDiagram-v2\n [*] --> Still\n Still --> Moving\n Moving --> Still\n\
             Moving --> Crash\n Crash --> [*]\n state Moving {\n [*] --> Lento\n Lento --> Rapido\n }\n\
             note right of Still : quieto\n",
        ),
        (
            "er",
            "erDiagram\n CUSTOMER ||--o{ ORDER : places\n ORDER ||--|{ LINE-ITEM : contains\n\
             CUSTOMER {\n string name\n int id PK\n string email UK\n }\n ORDER {\n int id PK\n date at\n }\n",
        ),
        (
            "gantt",
            "gantt\n title Un proyecto\n dateFormat YYYY-MM-DD\n section Diseño\n\
             Bocetos :a1, 2024-01-01, 20d\n Revision :after a1, 8d\n section Código\n\
             Implementar :crit, 2024-02-01, 25d\n Entrega :milestone, m1, 2024-03-01, 0d\n",
        ),
        ("pie", "pie showData\n \"Notas\" : 42.96\n \"Diagramas\" : 30\n \"Búsqueda\" : 20\n \"Otros\" : 7.04\n"),
        (
            "mindmap",
            "mindmap\n root((MetaMind))\n   Notas\n     Markdown\n     Enlaces\n     Tareas\n\
             \x20  Diagramas\n     Mermaid\n     Matemáticas\n   Búsqueda\n     Índice\n",
        ),
        (
            "timeline",
            "timeline\n title Historia\n section Temprano\n 2002 : LinkedIn\n\
             2004 : Facebook : Google\n section Tarde\n 2005 : YouTube\n 2006 : Twitter\n",
        ),
        (
            "journey",
            "journey\n title Mi día\n section Trabajo\n   Preparar café: 5: Yo\n\
             \x20  Programar: 4: Yo, Gato\n   Revisar: 3: Yo\n section Casa\n\
             \x20  Caminar: 2: Yo\n   Dormir: 5: Yo\n",
        ),
        (
            "gitgraph",
            "gitGraph\n commit id: \"a1\"\n commit id: \"a2\" tag: \"v0.1\"\n branch develop\n\
             commit id: \"b1\"\n checkout main\n commit id: \"a3\"\n merge develop tag: \"v1.0\"\n\
             commit id: \"a4\" type: HIGHLIGHT\n",
        ),
        (
            "quadrant",
            "quadrantChart\n title Campañas\n x-axis Alcance bajo --> Alcance alto\n\
             y-axis Compromiso bajo --> Compromiso alto\n quadrant-1 Ampliar\n quadrant-2 Promover\n\
             quadrant-3 Revisar\n quadrant-4 Mejorar\n Campaña A: [0.3, 0.6]\n Campaña B: [0.45, 0.23]\n\
             Campaña C: [0.75, 0.8]\n",
        ),
    ];
    for (name, src) in cases {
        match mermaid_core::render(src, &mut mermaid_core::Approx) {
            Ok(d) => {
                println!("{name}: {:.0}x{:.0} ({} items)", d.width, d.height, d.items.len());
                std::fs::write(dir.join(format!("{name}.svg")), svg(&d)).unwrap();
            }
            Err(e) => println!("{name}: ERROR {e:?}"),
        }
    }
}
