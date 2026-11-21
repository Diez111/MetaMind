//! The AI copilot in the right sidebar: its tab, its input, and the LaTeX of its answers.

use rustsidian_app::shell::RightPanel;
use rustsidian_app::{render, App, Frame, Key};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

const NOTE: &str = "# Home\n\nUna nota cualquiera.\n";

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), NOTE).unwrap();
    d
}

fn render_frame(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap) -> Frame {
    render(app, fonts, pm, 1.0)
}

/// Indices of the nodes carrying `class`.
fn classed(frame: &Frame, class: &str) -> Vec<usize> {
    (0..frame.dom.nodes.len())
        .filter(|&i| frame.dom.nodes[i].el.classes.iter().any(|c| c == class))
        .collect()
}

#[test]
fn the_copilot_opens_takes_input_and_reports_a_missing_key() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home.md");
    app.run("ai:open");
    assert!(app.right_open);
    assert_eq!(app.right_panel, RightPanel::Ai);

    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let mut frame = render_frame(&mut app, &mut fonts, &mut pm);
    let panel = classed(&frame, "ai-panel");
    assert_eq!(panel.len(), 1, "the panel is in the right sidebar");
    assert!(frame.dom.nodes[panel[0]].rect.w > 0.0 && frame.dom.nodes[panel[0]].rect.h > 0.0);
    assert!(frame.dom.by_id("ai-messages").is_some());
    assert_eq!(classed(&frame, "ai-input-container").len(), 1);

    // A press on the input gives it the keyboard; the characters are not the note's.
    let input = frame.dom.nodes[frame.dom.by_id("ai-messages").unwrap()].rect;
    let field = frame.dom.nodes[classed(&frame, "ai-input")[0]].rect;
    assert!(field.y >= input.y + input.h - 1.0, "the input sits under the conversation");
    app.click(&frame, &mut fonts, field.x + 4.0, field.y + 4.0, false);
    assert!(app.ai.editing);
    for c in "¿Qué es $x^2$?".chars() {
        app.text_input(&c.to_string());
    }
    assert_eq!(app.ai.input, "¿Qué es $x^2$?");
    assert_eq!(app.tab().unwrap().editor.buf.text(), NOTE, "the note keeps its text");

    // Shift+Enter starts a new line; Enter sends, and without a key the panel says so.
    app.key(Key::parse("Shift+Enter").unwrap());
    assert!(app.ai.input.contains('\n'));
    app.key(Key::parse("Enter").unwrap());
    assert!(app.ai.error.as_deref().is_some_and(|e| e.contains("clave")));
    assert!(app.ai.input.contains('\n'), "the question stays in the input");
    frame = render_frame(&mut app, &mut fonts, &mut pm);
    assert!(classed(&frame, "ai-error").len() == 1);

    // A press outside gives the keyboard back to the editor.
    let pane = frame.dom.nodes[frame.info.editor.unwrap()].rect;
    app.click(&frame, &mut fonts, pane.x + 10.0, pane.y + 10.0, false);
    assert!(!app.ai.editing);
}

#[test]
fn answers_typeset_their_latex() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home.md");
    app.run("ai:open");
    app.ai.messages.push(rustsidian_app::ai::Message::user("¿Y la energía?"));
    app.ai.messages.push(rustsidian_app::ai::Message::assistant(
        "La energía en reposo es $E = mc^2$.\n\n$$\\int_0^1 x^2\\,dx = \\frac{1}{3}$$",
    ));
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render_frame(&mut app, &mut fonts, &mut pm);

    // The display formula became a rich block and the inline one kept blanks in its line.
    assert!(!frame.info.rich.is_empty(), "$$…$$ is a display block");
    let inline = frame.info.inline.iter().find(|it| {
        frame.dom.nodes[it.node].el.text.as_deref().is_some_and(|t| t.contains('\u{a0}'))
    });
    assert!(inline.is_some(), "$…$ is typeset inside the line");
    let line = frame.dom.nodes[inline.unwrap().node].el.text.clone().unwrap();
    assert!(line.starts_with("La energía en reposo es"));
    assert!(!line.contains('$'), "the dollars are replaced by the formula's place");

    // Both are painted after layout.
    let display = frame.info.rich[0].node;
    assert!(!frame.dom.nodes[display].draw.is_empty(), "the display formula is drawn");
    let inline = frame.info.inline[0].node;
    assert!(!frame.dom.nodes[inline].draw.is_empty(), "the inline formula is drawn");
}
