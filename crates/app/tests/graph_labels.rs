use metamind_app::App;
use metamind_ui::{Cmd, Color, Rect, TextStyle};
use std::fs;

#[test]
fn crowded_labels_do_not_overlap() {
    let d = tempfile::tempdir().unwrap();
    let mut hub = String::new();
    for i in 0..80 {
        let name = format!("{i:02} - A fairly long note title about topic {i}");
        hub.push_str(&format!("[[{name}]]\n"));
        fs::write(d.path().join(format!("{name}.md")), "x").unwrap();
    }
    fs::write(d.path().join("Hub.md"), hub).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("graph:open");
    let g = app.graph_view.as_mut().unwrap();
    // The settled star, zoomed out until its labels crowd each other but still show.
    let n = g.graph.nodes.len();
    g.settle();
    assert!(!g.animating(), "the layout settles");
    g.settings.text_fade = 3.0;
    g.auto_fit = false;
    g.zoom = 0.3;
    g.cam = [
        g.pos.iter().map(|p| p[0]).sum::<f32>() / n as f32,
        g.pos.iter().map(|p| p[1]).sum::<f32>() / n as f32,
    ];
    let hub = g.graph.nodes.iter().position(|n| &*n.label == "Hub").unwrap();
    g.hover = Some(hub as u32);
    let style = TextStyle {
        size: 12.0,
        line_height: 1.2,
        color: Color::rgb(200, 200, 200),
        bold: false,
        italic: false,
        mono: false,
        wrap: false,
        family: None,
    };
    let cmds = g.paint(Rect::new(0.0, 0.0, 800.0, 600.0), &[Color::rgb(128, 128, 128); 9], &style);
    let labels: Vec<(String, Rect)> = cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::Text { x, y, text, style, .. } => Some((
                text.clone(),
                Rect::new(*x, *y, text.chars().count() as f32 * style.size * 0.52, style.size * 1.2),
            )),
            _ => None,
        })
        .collect();
    assert!(labels.iter().any(|(t, _)| t == "Hub"), "the hovered node is always labelled");
    assert!(labels.len() > 3 && labels.len() < n, "{} of {n} labelled", labels.len());
    for (i, (ta, a)) in labels.iter().enumerate() {
        for (tb, b) in &labels[i + 1..] {
            let hit = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
            assert!(!hit, "`{ta}` overlaps `{tb}`");
        }
    }
}
