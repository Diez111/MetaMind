//! The four render bugs of the QA pass, in the shapes the reader reported them: display math
//! written with the literal snippets of the note (also inside a callout), a raw `<svg>` block
//! dumped as prose, empty callouts of different heights, and a note that opens or ends under the
//! edges of its viewport.
//!
//! Everything here is asserted on what a frame really holds: the display text of each block, the
//! formulas handed to the block painter, and the rectangles the layout produced.
use metamind_app::shell::ViewMode;
use metamind_app::{render, App, Frame};
use metamind_ui::{Cmd, Fonts, Pixmap};
use std::fs;

/// The two `$$` formulas of the unit notes, byte for byte: `{,}` is the decimal comma the note
/// uses, `\lvert`/`\rvert`/`\le`/`\cdot` the commands around it.
const S1: &str = r"$$P_4(0{,}3)=1-0{,}045+\frac{0{,}0081}{24}=1-0{,}045+0{,}0003375=0{,}9553375$$";
const S2: &str = r"$$\lvert R_4(0{,}3)\rvert\le\frac{0{,}00243}{120}=2{,}03\cdot 10^{-5}$$";

/// A block of raw HTML, as the notes embed their figures.
const SVG: &str = concat!(
    r#"<svg width="560" height="90" viewBox="0 0 560 90" xmlns="http://www.w3.org/2000/svg">"#,
    r#"<text x="10" y="40">Dominio</text></svg>"#
);

fn vault(text: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("N.md"), text).unwrap();
    d
}

fn open(d: &tempfile::TempDir, mode: ViewMode) -> (App, Fonts) {
    let mut app = App::open(d.path()).unwrap();
    app.open_note("N.md");
    app.tab_mut().unwrap().mode = mode;
    (app, Fonts::new())
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 900).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn shown(f: &Frame) -> Vec<String> {
    f.info.blocks.iter().map(|b| b.display.clone()).collect()
}

/// Everything the block painter draws inside a node, as glyph runs.
fn drawn(f: &Frame, node: metamind_ui::NodeId) -> String {
    f.dom.nodes[node]
        .draw
        .iter()
        .filter_map(|c| match c {
            Cmd::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The `$$` formulas the frame hands to the block painter.
fn math_blocks(f: &Frame) -> Vec<(String, String)> {
    f.info
        .rich
        .iter()
        .filter_map(|it| match &it.kind {
            metamind_app::rich::Kind::Math { src, display: true } => Some((src.clone(), drawn(f, it.node))),
            _ => None,
        })
        .collect()
}

#[test]
fn display_math_is_typeset_and_never_shown_as_source() {
    let d = vault(&format!("Antes\n\n{S1}\n\n{S2}\n\nDespués\n"));
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let f = frame(&mut app, &mut fonts);

    let texts = shown(&f);
    assert!(!texts.iter().any(|t| t.contains("$$")), "display math shown raw: {texts:?}");
    assert!(!texts.iter().any(|t| t.contains("\\frac")), "LaTeX shown raw: {texts:?}");
    assert!(texts.contains(&"Antes".to_string()), "{texts:?}");

    // Both formulas are drawn, with the commands they were written with resolved to glyphs.
    let blocks = math_blocks(&f);
    assert_eq!(blocks.len(), 2, "two display formulas: {blocks:?}");
    for (src, glyphs) in &blocks {
        assert!(!src.contains("$$"), "{src:?}");
        assert!(src.starts_with("P_4") || src.starts_with("\\lvert R_4"), "{src:?}");
        assert!(!glyphs.is_empty(), "the formula drew nothing: {src:?}");
        // A command the engine cannot read is drawn as its own source: none may survive.
        assert!(!glyphs.contains('\\'), "{src:?} drew its own source: {glyphs:?}");
    }
}

#[test]
fn display_math_inside_a_callout_is_typeset_folded_and_unfolded() {
    // One titled, unfolded callout (`+`) whose body is the two snippets of the report.
    let d = vault(&format!("intro\n\n> [!solucion]+ Solución\n> {S1}\n>\n> {S2}\n\nFin\n"));
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let f = frame(&mut app, &mut fonts);

    let texts = shown(&f);
    assert!(texts.contains(&"Solución".to_string()), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("$$")), "raw display math in a callout: {texts:?}");
    assert!(!texts.iter().any(|t| t.contains("\\frac")), "raw LaTeX in a callout: {texts:?}");

    // The body's formulas are painted as blocks, inside the callout's own box.
    let blocks = math_blocks(&f);
    assert_eq!(blocks.len(), 2, "both snippets of the callout are typeset: {blocks:?}");
    for (src, glyphs) in &blocks {
        assert!(!glyphs.contains('\\'), "{src:?} drew its own source: {glyphs:?}");
        assert!(!glyphs.is_empty(), "{src:?} drew nothing");
    }

    // Folded: the body is gone and nothing of it is left behind as text.
    let action = f
        .dom
        .nodes
        .iter()
        .find_map(|n| n.el.action.clone().filter(|a| a.starts_with("callout:toggle:")))
        .expect("the callout can be folded");
    app.run(&action);
    let f = frame(&mut app, &mut fonts);
    let texts = shown(&f);
    assert!(texts.contains(&"Solución".to_string()), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("$$")), "raw math from a hidden body: {texts:?}");
    assert!(math_blocks(&f).is_empty(), "a folded body is not painted");

    // Unfolded again: the formulas come back, still typeset.
    app.run(&action);
    let f = frame(&mut app, &mut fonts);
    assert_eq!(math_blocks(&f).len(), 2, "the body is typeset again");
}

#[test]
fn a_raw_html_block_is_shown_as_code() {
    let d = vault(&format!("intro\n\n{SVG}\n\nFin\n"));
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let f = frame(&mut app, &mut fonts);

    // The block is one text run of the source, tags and all: nothing was stripped from it.
    let node = f
        .dom
        .nodes
        .iter()
        .position(|n| n.el.text.as_deref() == Some(SVG))
        .expect("the svg block is on screen, whole");
    let st = &f.dom.nodes[node].style;
    assert!(st.text.mono, "an HTML block is code: monospace");
    assert!(!st.text.wrap, "a tag is one token: the line never wraps inside it");
    assert_eq!(st.background.a, 255, "a code block has a background of its own");
    let classes = &f.dom.nodes[node].el.classes;
    assert!(classes.iter().any(|c| c == "cm-codeblock"), "{classes:?}");
    assert!(classes.iter().any(|c| c == "cm-htmlblock"), "{classes:?}");
    // The dots of `www.w3.org` and the tag ends stay in one run: no mid-token break.
    assert!(f.info.blocks.iter().any(|b| b.display == SVG));
}

#[test]
fn empty_callouts_have_one_height() {
    // The two empty callouts of the report: one plain, one with a quoted blank line under it.
    let d = vault("> [!solucion] Solución\n\n> [!solucion] Solución\n>\n");
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let f = frame(&mut app, &mut fonts);

    // The box each callout draws around its line: a one-line callout is the box itself, a callout
    // with a body is a `.cm-callout-block` with one child per line.
    let boxes: Vec<f32> = f
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "cm-callout" || c == "cm-callout-block"))
        .map(|n| n.rect.h)
        .collect();
    assert_eq!(boxes.len(), 2, "two callouts: {boxes:?}");
    assert!(
        (boxes[0] - boxes[1]).abs() < 0.5,
        "an empty callout keeps one height: {boxes:?} (a trailing blank line adds a paragraph)"
    );
    assert!(boxes[0] < 70.0, "an empty callout is one line tall: {boxes:?}");
}

#[test]
fn a_note_opens_at_its_first_line_and_ends_above_the_status_bar() {
    let mut text = String::from("# Inicio\n\n");
    for i in 0..60 {
        text.push_str(&format!("Párrafo {i} con algo de texto para llenar la vista.\n\n"));
    }
    let d = vault(&text);
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let f = frame(&mut app, &mut fonts);

    let view = f.info.editor.expect("the note is on screen");
    let cb = f.dom.content_rect(view);
    let vr = f.dom.nodes[view].rect;
    let status = f
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "status-bar"))
        .expect("the window has a status bar");
    // The note scrolls in the area above the footer, and the footer is not painted over it.
    assert!(
        (vr.y + vr.h - status.rect.y).abs() < 0.5,
        "the note's view ends where the status bar starts: view {:?} against {:?}",
        vr,
        status.rect
    );
    assert_eq!(f.dom.nodes[view].scroll_y, 0.0, "a note opens at its top");
    let first = f.info.blocks[0].node;
    assert!(
        f.dom.nodes[first].rect.y >= cb.y,
        "the first line is not above the view: {:?} against {:?}",
        f.dom.nodes[first].rect,
        cb
    );

    // A stale anchor from a longer version of the note opens it at the top, not at its end.
    app.tab_mut().unwrap().top_block = 500;
    let f = frame(&mut app, &mut fonts);
    let view = f.info.editor.unwrap();
    assert!(f.dom.nodes[view].scroll_y <= f.dom.max_scroll(view) + 0.5, "{:?}", f.dom.nodes[view].rect);

    // Scrolled to the end, the last line is inside the view, above the footer.
    let d = vault(&text);
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let mut f = frame(&mut app, &mut fonts);
    let view = f.info.editor.unwrap();
    let r = f.dom.nodes[view].rect;
    for _ in 0..40 {
        app.wheel(&f, r.x + r.w / 2.0, r.y + r.h / 2.0, 10.0);
        f = frame(&mut app, &mut fonts);
    }
    let view = f.info.editor.unwrap();
    let cb = f.dom.content_rect(view);
    let vb = f.dom.nodes[view].rect;
    let status = f.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "status-bar")).unwrap();
    let last = f.info.blocks.last().map(|b| b.node).expect("the end of the note is laid out");
    let lr = f.dom.nodes[last].rect;
    assert!(lr.y + lr.h <= cb.y + cb.h + 0.5, "the last line is inside the view: {lr:?} against {cb:?}");
    assert!(lr.y + lr.h <= status.rect.y, "the last line is above the status bar: {lr:?}");
    assert!(vb.y + vb.h <= status.rect.y + 0.5, "{vb:?} against {:?}", status.rect);
}

#[test]
fn a_formula_without_delimiters_is_left_as_the_reader_wrote_it() {
    // No `$..$`, so this is the reader's own text: it must not be typeset, rewritten or hidden.
    let src = "La derivada queda g(x)=\\operatorname{sen}(3x) en texto plano.\n";
    let d = vault(src);
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let f = frame(&mut app, &mut fonts);
    let texts = shown(&f);
    assert!(
        texts.iter().any(|t| t == "La derivada queda g(x)=\\operatorname{sen}(3x) en texto plano."),
        "{texts:?}"
    );
    assert!(f.info.rich.is_empty(), "no block formula was invented");
    assert!(f.info.inline.is_empty(), "no inline formula was invented");
}

#[test]
fn a_row_of_a_callout_is_as_tall_as_the_text_it_holds() {
    // The bullets of the EDO note, as the reader reported them: an item whose text wraps over
    // four lines with two formulas in it, in a panel narrow enough that the squeeze is what
    // decides the last line. The row used to keep the height of a wider measurement, so the
    // next bullet was drawn over the formula at the end of this one.
    let text = concat!(
        "> [!abstract] Al terminar esta unidad vas a poder\n",
        "> - Clasificar una EDO de primer orden como separable o lineal y aplicar el método que le ",
        "corresponde: separar e integrar, o factor integrante $\\mu(x)=e^{\\int P(x)\\,dx}$.\n",
        "> - Convertir una ecuación integral en una EDO derivando los dos miembros con la regla del ",
        "producto y el TFC, agregando el factor $g'(x)$ cuando el límite superior es $g(x)$.\n",
        "> - Resolver el ejercicio del parcial completo: derivar, simplificar hasta que quede $f'$ y ",
        "$f$, integrar y fijar $C$ con la condición inicial.\n",
        "> - Plantear un modelo (enfriamiento de Newton, desintegración, bola de nieve) y elegir la ",
        "variable que linealiza la EDO.\n",
    );
    let d = vault(text);
    let (mut app, mut fonts) = open(&d, ViewMode::Reading);
    let mut pm = Pixmap::new(520, 900).unwrap();
    let f = render(&mut app, &mut fonts, &mut pm, 1.0);

    // Every row of the callout, in the order it is stacked, with the text it holds.
    let rows: Vec<(usize, metamind_ui::Rect)> = f
        .dom
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.el.classes.iter().any(|c| c == "cm-list-item"))
        .map(|(i, n)| (i, n.rect))
        .collect();
    assert!(rows.len() >= 4, "the four bullets are laid out: {}", rows.len());
    for (i, (node, r)) in rows.iter().enumerate() {
        // The text of the row fits inside it: it is what the next row is stacked after.
        let text = f.dom.nodes[*node]
            .children
            .iter()
            .copied()
            .find(|c| f.dom.nodes[*c].el.classes.iter().any(|k| k == "list-item-text"))
            .expect("a row of a list item holds its text");
        let tr = f.dom.nodes[text].rect;
        assert!(
            tr.h <= r.h + 0.5,
            "row {i} is {:.1} tall but its text is {:.1}: the next row is drawn over it",
            r.h,
            tr.h
        );
        if let Some((_, prev)) = rows.get(i.wrapping_sub(1)).filter(|_| i > 0) {
            assert!(
                r.y >= prev.y + prev.h - 0.5,
                "row {i} starts at {:.1}, inside the row above it ({:.1}..{:.1})",
                r.y,
                prev.y,
                prev.y + prev.h
            );
        }
    }
}
