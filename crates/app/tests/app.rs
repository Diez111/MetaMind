use metamind_app::shell::{Focus, ViewMode};
use metamind_app::{render, App, Frame, Key};
use metamind_ui::{Cmd, Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    w("Home.md", "# Home\n\nSee [[Second]] and **bold** text.\n");
    w("dir/Second.md", "Second note links [[Home]].\n");
    w("dir/Third.md", "third\n");
    w(
        ".obsidian/hotkeys.json",
        r#"{"editor:toggle-highlight":[{"modifiers":["Mod","Shift"],"key":"H"}],"editor:toggle-bold":[]}"#,
    );
    w(".obsidian/appearance.json", r#"{"theme":"moonstone","enabledCssSnippets":["s"]}"#);
    w(".obsidian/snippets/s.css", ".theme-light { --background-primary: #fefefe }");
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.text_input(&c.to_string());
    }
}

#[test]
fn edit_save_and_conflict() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    assert_eq!(app.tab().unwrap().path, "Home.md");
    app.key(key("Ctrl+End"));
    typed(&mut app, "new line");
    assert!(app.tab().unwrap().editor.buf.is_modified());
    app.key(key("Ctrl+S"));
    assert!(fs::read_to_string(d.path().join("Home.md")).unwrap().ends_with("new line"));
    assert!(!app.tab().unwrap().editor.buf.is_modified());
    // An external edit must not be overwritten.
    typed(&mut app, "!");
    fs::write(d.path().join("Home.md"), "changed elsewhere, different length").unwrap();
    assert!(!app.save());
    assert!(app.notice.as_deref().unwrap().contains("fuera de MetMind"));
    assert_eq!(fs::read_to_string(d.path().join("Home.md")).unwrap(), "changed elsewhere, different length");
    assert!(
        app.tab().unwrap().editor.buf.text().ends_with("new line!"),
        "the user's text is still in the tab"
    );
}

#[test]
fn external_change_reloads_clean_tab() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("dir/Third.md");
    fs::write(d.path().join("dir/Third.md"), "rewritten by another program\n").unwrap();
    app.files_changed(&["dir/Third.md".to_string()]);
    assert_eq!(app.tab().unwrap().editor.buf.text(), "rewritten by another program\n");
    fs::write(d.path().join("New.md"), "x").unwrap();
    app.files_changed(&["New.md".to_string()]);
    assert!(app.store.index.id("New.md").is_some());
}

#[test]
fn palette_switcher_and_hotkeys() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    assert!(!app.dark, "appearance.json theme=moonstone is light");
    app.key(key("Ctrl+P"));
    assert_eq!(app.focus, Focus::Prompt);
    typed(&mut app, "modo claro");
    assert_eq!(app.prompt.as_ref().unwrap().items[0].action, "theme:toggle-light-dark");
    app.key(key("Enter"));
    assert!(app.dark && app.prompt.is_none());

    app.key(key("Ctrl+O"));
    typed(&mut app, "secnd");
    assert_eq!(app.prompt.as_ref().unwrap().items[0].action, "open:dir/Second.md");
    app.key(key("Enter"));
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");

    app.key(key("Ctrl+O"));
    typed(&mut app, "Brand New");
    app.key(key("ArrowDown"));
    app.key(key("ArrowDown"));
    app.key(key("ArrowDown"));
    app.key(key("Enter"));
    assert_eq!(app.tab().unwrap().path, "Brand New.md");
    assert!(d.path().join("Brand New.md").exists());

    // hotkeys.json: highlight rebound, bold unbound.
    typed(&mut app, "word");
    app.key(key("Ctrl+Shift+H"));
    assert_eq!(app.tab().unwrap().editor.buf.text(), "==word==");
    app.key(key("Ctrl+B"));
    assert_eq!(app.tab().unwrap().editor.buf.text(), "==word==", "Ctrl+B was unbound by hotkeys.json");
    assert_eq!(app.hotkey_for("editor:toggle-highlight"), "Ctrl+Shift+H");

    // Navigation history and tabs.
    app.key(key("Ctrl+Alt+ArrowLeft"));
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");
    app.key(key("Ctrl+Alt+ArrowRight"));
    assert_eq!(app.tab().unwrap().path, "Brand New.md");
    app.key(key("Ctrl+W"));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(
        fs::read_to_string(d.path().join("Brand New.md")).unwrap(),
        "==word==",
        "closing a tab saves it"
    );
}

#[test]
fn follow_link_search_and_modes() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    let at = app.tab().unwrap().editor.buf.text().find("Second").unwrap();
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(at)]);
    app.run("editor:follow-link");
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");

    app.key(key("Ctrl+Shift+F"));
    assert_eq!(app.focus, Focus::Search);
    typed(&mut app, "links -third");
    app.key(key("Enter"));
    assert_eq!(app.search_results.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), vec!["dir/Second.md"]);
    app.search_query = "(".into();
    app.run_search();
    assert!(app.search_error.is_some());

    app.key(key("Escape"));
    app.key(key("Ctrl+E"));
    assert_eq!(app.tab().unwrap().mode, ViewMode::Reading);
    typed(&mut app, "ignored in reading mode");
    assert!(!app.tab().unwrap().editor.buf.is_modified());
}

#[test]
fn rendering_and_mouse() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 600).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // The snippet's light background is applied.
    let p = pm.pixel(700, 300).unwrap();
    assert_eq!((p.red(), p.green(), p.blue()), (254, 254, 254));
    // Click the "Home" row in the file explorer.
    let row = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("open:Home.md")).unwrap().rect;
    app.click(&frame, &mut fonts, row.x + 20.0, row.y + 5.0, false);
    app.release();
    assert_eq!(app.tab().unwrap().path, "Home.md");

    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // Live preview hides the `**` of "**bold**" while the cursor is elsewhere.
    let para = frame.info.blocks.iter().find(|b| b.display.contains("bold")).unwrap();
    assert_eq!(para.display, "See Second and bold text.");
    // Clicking on the word "bold" puts the cursor inside it in the *source*.
    let node = &frame.dom.nodes[para.node];
    let cb = frame.dom.content_rect(para.node);
    let rects = fonts.range_rects(
        &para.display,
        &node.el.spans,
        &node.style.text,
        Some(cb.w),
        para.display.find("old").unwrap(),
        para.display.find("old").unwrap() + 1,
    );
    app.click(&frame, &mut fonts, cb.x + rects[0].x + 1.0, cb.y + rects[0].y + 5.0, false);
    let head = app.tab().unwrap().editor.buf.sels[0].head;
    let src = app.tab().unwrap().editor.buf.text();
    assert_eq!(&src[head..head + 3], "old", "click maps through hidden markup");
    // With the cursor inside, the markup is revealed.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display.contains("**bold**")));
    // The tree starts folded, so opening a folder is what shows its files.
    let before = app.tree_rows().len();
    app.run("toggle-folder:dir");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(app.tree_rows().len(), before + 2);
}

#[test]
fn cli_commands() {
    use metamind_app::cli::{run, Outcome};
    let d = vault();
    let v = d.path().to_string_lossy().to_string();
    let call = |args: &[&str]| {
        let mut out = Vec::new();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let code = match run(&args, &mut out) {
            Outcome::Exit(c) => c,
            Outcome::Open { .. } | Outcome::Welcome => -1,
        };
        (code, String::from_utf8(out).unwrap())
    };
    let (code, out) = call(&["index", &v]);
    assert_eq!(code, 0);
    assert!(out.contains("notas: 3") && out.contains("enlaces resueltos: 2"), "{out}");
    let (code, out) = call(&["search", &v, "path:dir links"]);
    assert_eq!((code, out.lines().next()), (0, Some("dir/Second.md")));
    assert_eq!(call(&["search", &v, "(unclosed"]).0, 2);
    let html = d.path().join("out.html");
    let (code, _) = call(&["export", &v, "Home", "-o", &html.to_string_lossy()]);
    assert_eq!(code, 0);
    let page = fs::read_to_string(&html).unwrap();
    assert!(
        page.contains("<h1 data-heading=\"Home\">") && page.contains("href=\"dir/Second.html\""),
        "{page}"
    );
    let pdf = d.path().join("out.pdf");
    let (code, out) = call(&["export", &v, "Home", "--format", "pdf", "-o", &pdf.to_string_lossy()]);
    assert_eq!(code, 0, "{out}");
    assert!(fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    let (code, out) = call(&["doctor", &v]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(call(&["plugin", "new"]).0, 2);
    assert_eq!(call(&[&v]).0, -1, "a bare vault path opens a window");
    assert_eq!(call(&[&format!("{v}/Home.md")]).0, -1, "a note file opens its vault");
    assert_eq!(call(&["nonsense"]).0, 2);
    assert_eq!(call(&["metamind://frobnicate"]).0, 2, "links are parsed, not taken as folders");
    let (_, out) = call(&["commands"]);
    assert!(out.contains("command-palette:open"));
}

#[test]
fn block_widgets() {
    let d = tempfile::tempdir().unwrap();
    // A 2x1 opaque red PNG.
    let mut img = Pixmap::new(2, 1).unwrap();
    img.fill(tiny_skia_color(255, 0, 0));
    fs::write(d.path().join("red.png"), img.encode_png().unwrap()).unwrap();
    fs::write(d.path().join("Other.md"), "# Other\n\n## Part\n\nEmbedded **part**.\n\n## Tail\n\nhidden\n")
        .unwrap();
    fs::write(
        d.path().join("Main.md"),
        "Top\n\n> quoted\n> lines\n\n> [!warning]- Folded title\n> folded body\n\n> [!tip]\n> tip body\n\n\
         ![[Other#Part]] ![[red.png|40x20]]\n\n| A | B |\n|---|:-:|\n| **x** | y z |\n\nEnd\n",
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Main.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 800).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let shown: Vec<&str> = frame.info.blocks.iter().map(|b| b.display.as_str()).collect();
    // Quote markers are hidden, and the wrap the source used is a space: it is one paragraph.
    assert!(shown.contains(&"quoted lines"), "{shown:?}");
    // A folded callout shows only its title; an untitled one shows its type, then its body.
    assert!(shown.contains(&"Folded title"), "{shown:?}");
    assert!(shown.contains(&"tip\ntip body"), "{shown:?}");
    // The table is a grid of cells without the delimiter row or pipes.
    for cell in ["A", "B", "x", "y z"] {
        assert!(shown.contains(&cell), "{shown:?}");
    }
    assert!(!shown.iter().any(|t| t.contains('|') || t.contains("---")), "{shown:?}");
    let texts: Vec<&str> = frame.dom.nodes.iter().filter_map(|n| n.el.text.as_deref()).collect();
    // The embed shows the addressed section only, formatted.
    assert!(texts.contains(&"Embedded part."), "{texts:?}");
    assert!(!texts.contains(&"hidden"));
    // The image is drawn at the requested size, in its own colour.
    let image = frame.dom.nodes.iter().find(|n| n.el.image.is_some()).unwrap();
    assert_eq!((image.rect.w, image.rect.h), (40.0, 20.0));
    let p = pm.pixel((image.rect.x + 20.0) as u32, (image.rect.y + 10.0) as u32).unwrap();
    assert_eq!((p.red(), p.green(), p.blue()), (255, 0, 0));

    // Clicking a cell puts the cursor in that cell's source, which turns the table back into text.
    let cell = frame.info.blocks.iter().find(|b| b.display == "y z").unwrap();
    let cb = frame.dom.content_rect(cell.node);
    app.click(&frame, &mut fonts, cb.x + 2.0, cb.y + 5.0, false);
    let head = app.tab().unwrap().editor.buf.sels[0].head;
    let src = app.tab().unwrap().editor.buf.text();
    assert!(src[head..].starts_with("y z"), "cursor at {:?}", &src[head..]);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display.contains("|---|:-:|")));

    // Source mode shows everything as written and draws no widgets.
    app.tabs[0].mode = ViewMode::Source;
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display == "> quoted\n> lines"));
    assert!(!frame.dom.nodes.iter().any(|n| n.el.image.is_some()));

    // Reading mode renders widgets even where the cursor is.
    app.tabs[0].mode = ViewMode::Reading;
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display == "y z"));
}

#[test]
fn code_blocks_are_highlighted() {
    let (_d, mut app) = note("intro\n\n```rust\nfn main() { let s = \"x\"; } // c\n```\n", "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let block = frame.info.blocks.iter().find(|b| b.display.contains("fn main")).unwrap();
    let spans = &frame.dom.nodes[block.node].el.spans;
    let mut colors: Vec<_> = spans.iter().filter_map(|s| s.color).map(|c| (c.r, c.g, c.b)).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() >= 4, "{colors:?}");
}

#[test]
fn math_blocks_are_typeset_and_editable() {
    let (_d, mut app) = note("intro\n\n$$\n\\frac{a}{b}\n$$\n\nAfter $x^2$ here.\n", "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // The display block is a widget; the inline formula is drawn into the flow of its line.
    assert_eq!(frame.info.rich.len(), 1);
    assert_eq!(frame.info.inline.len(), 1);
    for it in &frame.info.rich {
        assert!(!frame.dom.nodes[it.node].draw.is_empty());
    }
    for it in &frame.info.inline {
        assert!(!frame.dom.nodes[it.node].draw.is_empty());
    }
    let block = &frame.info.rich[0];
    let r = frame.dom.content_rect(block.node);
    assert!(r.h > 30.0);
    app.click(&frame, &mut fonts, r.x + r.w / 2.0, r.y + r.h / 2.0, false);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display.contains("\\frac{a}{b}")));
    assert!(frame.info.rich.is_empty());
    assert_eq!(frame.info.inline.len(), 1);
}

/// The box a command covers, with the glyph size as the ink height of a run.
fn cmd_box(c: &Cmd) -> Option<(f32, f32, f32, f32)> {
    match c {
        Cmd::Text { x, y, style, .. } => Some((*x, *y, style.size * 0.9, style.size)),
        Cmd::Lines { segs, .. } => {
            let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for s in segs {
                x0 = x0.min(s[0]);
                x1 = x1.max(s[2]);
                y0 = y0.min(s[1]);
                y1 = y1.max(s[3]);
            }
            (x1 > x0).then_some((x0, y0 - 1.0, x1 - x0, (y1 - y0) + 2.0))
        }
        _ => None,
    }
}

fn leaf_rect(frame: &Frame) -> metamind_ui::Rect {
    frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "workspace-leaf"))
        .map(|n| n.rect)
        .expect("a pane is open")
}

#[test]
fn inline_math_is_drawn_inside_its_line() {
    let (_d, mut app) = note(
        "intro\n\nTexto con $\\frac{1}{2}\\ln 2$ y $\\dfrac{du}{2}$ y $\\sum_{n=1}^{\\infty}\\frac{1}{n^2}$.\n",
        "{}",
    );
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(frame.info.inline.len(), 1);
    let it = &frame.info.inline[0];
    let node = &frame.dom.nodes[it.node];
    let r = node.rect;
    assert!(!node.draw.is_empty(), "the formulas of the line are not drawn");
    // The line grew to hold the tallest formula instead of drawing it over the line below.
    assert!(r.h >= 30.0, "line is {:.1} px tall, too short for its formula", r.h);
    let leaf = leaf_rect(&frame);
    for c in &node.draw {
        let Some((x, y, w, h)) = cmd_box(c) else { continue };
        // The formula sits in the blanks of its own line, not at the top left of the window where
        // the pane's clip would take it out (the commands are in scene coordinates).
        assert!(x >= r.x, "{c:?} starts left of its line ({:.1})", r.x);
        assert!(x + w <= r.x + r.w, "{c:?} runs past its line ({:.1})", r.x + r.w);
        assert!(y >= r.y - 1.0 && y + h <= r.y + r.h + 1.0, "{c:?} does not fit the line {r:?}");
        assert!(x >= leaf.x && x + w <= leaf.x + leaf.w, "{c:?} is drawn outside the pane");
        assert!(y >= leaf.y && y + h <= leaf.y + leaf.h, "{c:?} is drawn outside the pane");
    }
}

#[test]
fn display_math_block_is_as_tall_as_its_formula() {
    let src =
        "intro\n\n$$\n\\frac{a}{b}\n$$\n\n$$\n\\sum_{n=1}^{\\infty}\\frac{1}{n^2}+\\int_0^1 x^2\\,dx\n$$\n\n";
    let (_d, mut app) = note(src, "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 900).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(frame.info.rich.len(), 2);
    for it in &frame.info.rich {
        let node = &frame.dom.nodes[it.node];
        let (mut y0, mut y1) = (f32::MAX, f32::MIN);
        for c in &node.draw {
            if let Some((_, y, _, h)) = cmd_box(c) {
                y0 = y0.min(y);
                y1 = y1.max(y + h);
            }
        }
        assert!(y1 > y0, "the block has no command below it");
        // The block reserves the typeset box: a formula with a fraction (or a sum with limits) is
        // never crushed into a band of a handful of pixels.
        assert!(node.rect.h >= 30.0, "block is {:.1} px tall", node.rect.h);
        assert!(
            node.rect.h >= y1 - y0 - 1.0,
            "block is {:.1} px tall, its formula {:.1}",
            node.rect.h,
            y1 - y0
        );
        assert!(y0 >= node.rect.y - 1.0 && y1 <= node.rect.y + node.rect.h + 1.0, "{node:?}");
    }
}

#[test]
fn a_display_formula_too_wide_for_the_pane_is_not_crushed() {
    // A formula about four panes wide: it is drawn as large as the pane allows and the rest runs
    // under the right edge, instead of the whole block collapsing into an unreadable strip.
    let mut f = String::from("$$");
    for _ in 0..60 {
        f.push_str("\\frac{1}{2}x+");
    }
    f.push_str("0$$");
    let (_d, mut app) = note(&format!("intro\n\n{f}\n\n"), "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(frame.info.rich.len(), 1);
    let node = &frame.dom.nodes[frame.info.rich[0].node];
    let size = node
        .draw
        .iter()
        .filter_map(|c| match c {
            Cmd::Text { style, .. } => Some(style.size),
            _ => None,
        })
        .fold(0.0f32, f32::max);
    assert!(size >= 10.5, "the formula is set at {size:.1} px");
    let h = node.rect.h;
    assert!(h >= 30.0, "block is {h:.1} px tall");
}

#[test]
fn a_display_formula_wider_than_its_block_is_shrunk_to_fit() {
    // 90-plus characters of amsmath with a nested fraction, a radical and a sum: at 20px it is
    // twice as wide as the reading column.
    let src = r"$$\int_1^{x^3}\frac{\ln t}{t}dt+3\ln(x^3)+\frac{2x(x-1)(x+1)}{-2(1-x)}+\sqrt[3]{x^2+1}+\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}$$";
    let (_d, mut app) = note(&format!("intro\n\n{src}\n"), "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let node = frame.info.rich[0].node;
    let r = frame.dom.content_rect(node);
    let ink = |frame: &metamind_app::Frame, node: metamind_ui::NodeId| {
        let mut max_x = f32::MIN;
        let mut max_size = 0.0f32;
        for cmd in &frame.dom.nodes[node].draw {
            if let Cmd::Text { x, style, .. } = cmd {
                max_x = max_x.max(*x);
                max_size = max_size.max(style.size);
            }
        }
        (max_x, max_size)
    };
    let (max_x, size) = ink(&frame, node);
    assert!(max_x <= r.x + r.w, "the formula is drawn inside its block: {max_x} vs {}", r.x + r.w);
    // A wider window has room for the formula at full size: the narrow one is smaller, so it was
    // shrunk to fit instead of being clipped.
    let mut wide = Pixmap::new(2400, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut wide, 1.0);
    let (_, wide_size) = ink(&frame, frame.info.rich[0].node);
    assert!(size < wide_size, "shrunk: {size} vs {wide_size}");
}

#[test]
fn mermaid_blocks_are_drawn_or_explained() {
    let src = "intro\n\n```mermaid\ngraph LR\n  A --> B\n```\n\n```mermaid\npie title x\n  \"a\": 1\n```\n";
    let (_d, mut app) = note(src, "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 900).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // Both kinds are drawn since mermaid-core learned `pie`: no block falls back to an
    // explanation any more.
    assert_eq!(frame.info.rich.len(), 2);
    for it in &frame.info.rich {
        assert!(frame.dom.nodes[it.node].draw.len() > 2, "the diagram paints something");
    }
    let texts: Vec<&str> = frame.dom.nodes.iter().filter_map(|n| n.el.text.as_deref()).collect();
    assert!(!texts.iter().any(|t| t.contains("no son compatibles")), "{texts:?}");
    // Source mode shows the fence untouched.
    app.tabs[0].mode = ViewMode::Source;
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.rich.is_empty());
}

fn tiny_skia_color(r: u8, g: u8, b: u8) -> metamind_ui::SkColor {
    metamind_ui::SkColor::from_rgba8(r, g, b, 255)
}

fn note(text: &str, app_json: &str) -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), app_json).unwrap();
    fs::write(d.path().join("N.md"), text).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("N.md");
    (d, app)
}

fn shown(app: &mut App) -> Vec<String> {
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    render(app, &mut fonts, &mut pm, 1.0).info.blocks.iter().map(|b| b.display.clone()).collect()
}

fn cursor_at(app: &mut App, needle: &str) {
    let at = app.tab().unwrap().editor.buf.text().find(needle).unwrap();
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(at)]);
}

#[test]
fn folding() {
    let (_d, mut app) = note("intro\n\n# A\nalpha\n\n## B\nbeta\n\n# C\n- item\n  - child\n- solo\n", "{}");
    cursor_at(&mut app, "alpha");
    // Folding from inside a section folds the section and moves the cursor to its heading.
    app.run("editor:toggle-fold");
    let s = shown(&mut app);
    // The heading keeps its rendering while the caret sits in it: the `#`s are markup, not text.
    assert!(s.iter().any(|t| t.starts_with("A") && t.ends_with('⋯')), "{s:?}");
    assert!(!s.iter().any(|t| t.contains("# A")), "{s:?}");
    assert!(!s.iter().any(|t| t.contains("alpha") || t.contains("beta")), "{s:?}");
    assert!(s.iter().any(|t| t.contains("C")), "{s:?}");
    assert_eq!(app.tab().unwrap().editor.buf.sels[0].head, 10);
    // Moving down skips the folded text.
    app.key(key("ArrowDown"));
    let head = app.tab().unwrap().editor.buf.sels[0].head;
    let text = app.tab().unwrap().editor.buf.text();
    assert!(head > text.find("beta").unwrap() + 4 && head <= text.find("# C").unwrap(), "head {head}");
    // Typing before a fold keeps it attached to its heading.
    cursor_at(&mut app, "intro");
    typed(&mut app, "x");
    let s = shown(&mut app);
    assert!(!s.iter().any(|t| t.contains("alpha")), "{s:?}");
    // Undo of that edit keeps it too; clicking the indicator unfolds.
    app.run("editor:undo");
    assert!(!shown(&mut app).iter().any(|t| t.contains("alpha")));
    app.run("fold:7");
    assert!(shown(&mut app).iter().any(|t| t.contains("alpha")));
    // A list item folds its children inside the same block.
    cursor_at(&mut app, "item");
    app.run("editor:toggle-fold");
    let s = shown(&mut app);
    assert!(s.iter().any(|t| t.contains("item") && !t.contains("child") && t.contains("solo")), "{s:?}");
    app.run("editor:fold-all");
    let s = shown(&mut app);
    assert_eq!(s.iter().filter(|t| !t.is_empty()).count(), 3, "{s:?}");
    app.run("editor:unfold-all");
    assert!(shown(&mut app).iter().any(|t| t.contains("child")));
}

#[test]
fn table_editing() {
    let (_d, mut app) = note("intro\n\n|a|b|\n|-|-:|\n|1|2|\n\nend\n", "{}");
    let text = |app: &App| app.tab().unwrap().editor.buf.text();
    cursor_at(&mut app, "1|2");
    // Tab moves to the next cell and re-aligns the source; past the last cell it adds a row.
    app.key(key("Tab"));
    assert_eq!(text(&app), "intro\n\n| a   |   b |\n| --- | --: |\n| 1   |   2 |\n\nend\n");
    typed(&mut app, "0");
    app.key(key("Tab"));
    typed(&mut app, "x");
    app.key(key("Shift+Tab"));
    typed(&mut app, "!");
    assert_eq!(text(&app), "intro\n\n| a   |   b |\n| --- | --: |\n| 1   |  20! |\n| x   |     |\n\nend\n");
    // Enter goes down a row in the same column.
    app.key(key("Enter"));
    typed(&mut app, "y");
    // Typing does not re-align; the next table operation does.
    assert!(text(&app).contains("| x   | y    |"), "{}", text(&app));
    app.run("editor:table-col-after");
    typed(&mut app, "new");
    app.run("editor:table-row-up");
    app.run("editor:table-col-left");
    app.run("editor:table-col-align-center");
    assert_eq!(
        text(&app),
        "intro\n\n| a   |     |   b |\n| --- | :-: | --: |\n| x   | new |   y |\n| 1   |     | 20! |\n\nend\n"
    );
    app.run("editor:table-row-delete");
    app.run("editor:table-col-delete");
    app.run("editor:table-sort-descending");
    assert_eq!(text(&app), "intro\n\n| a   |   b |\n| --- | --: |\n| 1   | 20! |\n\nend\n");
    // One undo step per operation that changed the text (the sort did not).
    app.run("editor:undo");
    assert!(text(&app).contains("| 1   |     | 20! |"), "{}", text(&app));
    app.run("editor:redo");
    // Outside a table, Tab indents as before and table commands do nothing.
    cursor_at(&mut app, "end");
    let before = text(&app);
    app.run("editor:table-row-after");
    assert_eq!(text(&app), before);
    app.run("editor:insert-table");
    assert!(text(&app).ends_with("end\n\n|     |     |\n| --- | --- |\n|     |     |\n"), "{:?}", text(&app));
}

#[test]
fn vim_keys() {
    let (_d, mut app) = note("one two\nthree\n", r#"{"vimMode":true}"#);
    let text = |app: &App| app.tab().unwrap().editor.buf.text();
    assert_eq!(app.tab().unwrap().vim_label(), Some("NORMAL"));
    // Normal mode: typed characters are commands.
    typed(&mut app, "dw");
    assert_eq!(text(&app), "two\nthree\n");
    typed(&mut app, "ihi ");
    assert_eq!(app.tab().unwrap().vim_label(), Some("INSERT"));
    assert_eq!(text(&app), "hi two\nthree\n");
    app.key(key("Escape"));
    assert_eq!(app.tab().unwrap().vim_label(), Some("NORMAL"));
    app.key(key("Enter"));
    typed(&mut app, "dd");
    assert_eq!(text(&app), "hi two\n");
    typed(&mut app, "u");
    assert_eq!(text(&app), "hi two\nthree\n");
    // The command turns vim keys off for open and new tabs.
    app.run("editor:toggle-vim");
    assert_eq!(app.tab().unwrap().vim_label(), None);
    typed(&mut app, "dd");
    assert!(text(&app).contains("dd"));
}

#[test]
fn graph_view() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 600).unwrap();
    app.key(key("Ctrl+G"));
    assert!(app.show_graph && app.tab().is_none());
    app.graph_view.as_mut().unwrap().settle();
    assert!(!app.graph_animating(), "the layout thread stops once settled");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let pane = frame.dom.nodes[frame.info.graph.unwrap()].rect;
    let g = app.graph_view.as_ref().unwrap();
    assert_eq!((g.graph.nodes.len(), g.graph.edges.len()), (3, 2));
    // Something other than the background is drawn inside the pane.
    let bg = pm.pixel(pane.x as u32 + 2, pane.y as u32 + 2).unwrap();
    let second = g.graph.nodes.iter().position(|n| &*n.label == "Second").unwrap();
    let (x, y) = g.node_screen(&pane, second).unwrap();
    assert!(pane.contains(x, y));
    assert_ne!(pm.pixel(x as u32, y as u32).unwrap(), bg);

    // Typing does not reach the hidden note.
    typed(&mut app, "zzz");
    assert!(!app.tabs[0].editor.buf.is_modified());

    // Hover highlights, wheel zooms about the pointer, dragging the background pans.
    assert!(app.hover(&frame, x, y));
    assert!(!app.hover(&frame, x, y));
    let zoom = app.graph_view.as_ref().unwrap().zoom;
    app.wheel(&frame, x, y, -3.0);
    let g = app.graph_view.as_ref().unwrap();
    assert!(g.zoom > zoom && !g.auto_fit);
    let (x2, y2) = g.node_screen(&pane, second).unwrap();
    assert!((x2 - x).abs() < 0.5 && (y2 - y).abs() < 0.5, "the point under the pointer stays put");
    app.run("graph:reset-view");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);

    // Filters are saved to graph.json and rebuild the graph.
    app.run("graph:toggle-orphans");
    let frame2 = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(app.graph_view.as_ref().unwrap().graph.nodes.len(), 2);
    assert!(fs::read_to_string(d.path().join(".obsidian/graph.json"))
        .unwrap()
        .contains("\"showOrphans\": false"));
    drop(frame2);
    app.run("graph:toggle-orphans");
    app.graph_view.as_mut().unwrap().settle();
    let frame = {
        drop(frame);
        render(&mut app, &mut fonts, &mut pm, 1.0)
    };

    // A click on a node opens the note; the graph tab stays available.
    let g = app.graph_view.as_ref().unwrap();
    let second = g.graph.nodes.iter().position(|n| &*n.label == "Second").unwrap();
    let (x, y) = g.node_screen(&pane, second).unwrap();
    app.click(&frame, &mut fonts, x, y, false);
    app.release();
    assert!(!app.show_graph);
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");
    app.run("graph:focus");
    assert!(app.show_graph);
    app.run("workspace:close");
    assert!(app.graph_view.is_none() && app.tabs.len() == 2);

    // Local graph: the active note and its neighbours.
    app.run("graph:open-local");
    let g = app.graph_view.as_ref().unwrap();
    assert_eq!(g.local.as_deref(), Some("dir/Second.md"));
    assert_eq!(g.graph.nodes.len(), 2);
}

#[test]
fn new_canvas_gets_a_free_name_and_opens() {
    let d = vault();
    fs::write(d.path().join("Untitled.canvas"), "{}").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("canvas:new");
    assert!(app.canvas_active());
    let text = fs::read_to_string(d.path().join("Untitled 1.canvas")).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap()["nodes"], serde_json::json!([]));
}

#[test]
fn copy_screenshot_crops_the_open_pane() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    app.run("workspace:copy-screenshot");
    assert!(app.screenshot);
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 2.0);
    let pane =
        frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "markdown-source-view")).unwrap();
    let r = pane.rect;
    let shot = metamind_app::pane_image(&frame, &pm, 2.0).unwrap();
    assert_eq!((shot.width(), shot.height()), ((r.w * 2.0).round() as u32, (r.h * 2.0).round() as u32));
    let (x, y) = ((r.x * 2.0).round() as u32, (r.y * 2.0).round() as u32);
    assert_eq!(shot.pixel(5, 5), pm.pixel(x + 5, y + 5));
    app.copy_screenshot(&frame, &pm, 2.0);
    assert!(!app.screenshot);
    assert_eq!(
        app.notice.as_deref(),
        Some("No se pudo copiar la captura de pantalla: el portapapeles del sistema está desactivado")
    );
}

#[test]
fn canvas_view() {
    let d = vault();
    let w = |p: &str, t: &str| fs::write(d.path().join(p), t).unwrap();
    w(
        "Board.canvas",
        r##"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"# Hi","extra":1},{"id":"b","type":"file","file":"Home.md","x":400,"y":0,"width":200,"height":100,"color":"#ff0000"}],"edges":[{"id":"e","fromNode":"a","toNode":"b","label":"x"}],"keep":true}"##,
    );
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    app.run("open:Board.canvas");
    assert!(app.canvas_active() && app.tab().is_none());
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 600).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let pane = frame.dom.nodes[frame.info.canvas.unwrap()].rect;

    typed(&mut app, "zzz");
    assert!(!app.tabs[0].editor.buf.is_modified());

    let v = app.canvas_view.as_ref().unwrap();
    let (x, y) = v.to_screen(&pane, 100.0, 50.0);
    let zoom = v.zoom;
    app.wheel(&frame, x, y, -3.0);
    let v = app.canvas_view.as_ref().unwrap();
    assert!(v.zoom > zoom && !v.auto_fit);
    let (x2, y2) = v.to_screen(&pane, 100.0, 50.0);
    assert!((x2 - x).abs() < 0.5 && (y2 - y).abs() < 0.5);

    app.click(&frame, &mut fonts, x2, y2, false);
    app.drag(&frame, &mut fonts, x2 + 30.0, y2);
    app.release();
    let n = app.canvas_view.as_ref().unwrap().canvas.node("a").unwrap().x;
    assert!(n > 0.0);
    let saved = fs::read_to_string(d.path().join("Board.canvas")).unwrap();
    assert!(saved.contains("\"extra\":1") && saved.contains("\"keep\":true"), "{saved}");

    app.run("canvas:add-text");
    typed(&mut app, "new");
    app.key(key("Escape"));
    assert_eq!(app.canvas_view.as_ref().unwrap().canvas.nodes.len(), 3);
    assert!(fs::read_to_string(d.path().join("Board.canvas")).unwrap().contains("\"text\":\"new\""));
    app.run("canvas:delete-selected");
    assert_eq!(app.canvas_view.as_ref().unwrap().canvas.nodes.len(), 2);
    render(&mut app, &mut fonts, &mut pm, 1.0);
}

#[test]
fn status_bar_shows_word_goal_and_reading_time() {
    let d = tempfile::tempdir().unwrap();
    let body = "word ".repeat(450);
    fs::write(d.path().join("Goal.md"), format!("---\nword-goal: 1000\n---\n{body}\n")).unwrap();
    fs::write(d.path().join("Plain.md"), "one two three\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 600).unwrap();
    let mut status = |app: &mut App| -> Vec<String> {
        let frame = render(app, &mut fonts, &mut pm, 1.0);
        frame
            .dom
            .nodes
            .iter()
            .filter_map(|n| n.el.text.clone())
            .filter(|t| t.contains("palabra") || t.contains("min de lectura"))
            .collect()
    };
    app.run("open:Goal.md");
    let s = status(&mut app);
    assert!(
        s.contains(&"450 / 1000 palabras".to_string()) && s.contains(&"3 min de lectura".to_string()),
        "{s:?}"
    );
    app.run("open:Plain.md");
    let s = status(&mut app);
    assert!(s.contains(&"3 palabras".to_string()) && s.contains(&"1 min de lectura".to_string()), "{s:?}");
}

#[test]
fn jpeg_embeds_are_shown() {
    // The same hand-built 8x8 baseline JPEG as the decoder test in metamind-ui.
    let mut j = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
    j.extend([1u8; 64]);
    j.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00]);
    for class in [0x00, 0x10] {
        j.extend([0xFF, 0xC4, 0x00, 0x14, class, 1]);
        j.extend([0u8; 15]);
        j.push(0);
    }
    j.extend([0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x3F, 0xFF, 0xD9]);
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("photo.JPG"), &j).unwrap();
    fs::write(d.path().join("N.md"), "text\n\n![[photo.JPG]]\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:N.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 600).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let img = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "image-embed")).unwrap();
    assert_eq!((img.rect.w, img.rect.h), (8.0, 8.0));
}

#[test]
fn reading_view_draws_inline_backgrounds_and_strikes() {
    let (_d, mut app) = note("a ==marked== b `code` c ~~gone~~ d\n", "{}");
    app.tabs[0].mode = ViewMode::Reading;
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let bv = frame.info.blocks.iter().find(|b| b.display.contains("marked")).unwrap();
    let node = &frame.dom.nodes[bv.node];
    assert_eq!(node.overlays.len(), 2, "highlight and inline code backgrounds");
    assert_eq!(node.overlays_top.len(), 1, "strikethrough line");
}

#[test]
fn a_numbered_heading_keeps_its_number_in_the_heading_style() {
    // Live preview hides the `## ` marker, so the display of this heading starts with `10.`,
    // which a rendered list would take for an ordered-item marker. It is part of the heading:
    // it has to share the heading's run, weight, size and baseline.
    let (_d, mut app) = note("intro\n\n## 10. Gates (all prefixed)\n\nThe body.\n", "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(
        !frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "list-item-marker")),
        "a heading number is not a list marker"
    );
    let block = frame.info.blocks.iter().find(|b| b.display.contains("Gates")).unwrap();
    let node = &frame.dom.nodes[block.node];
    assert_eq!(node.el.text.as_deref(), Some("10. Gates (all prefixed)"));
    assert!(
        node.el.attrs.iter().any(|(k, v)| k == "aria-level" && v == "2"),
        "still an H2: {:?}",
        node.el.attrs
    );
    assert!(!node.el.spans.is_empty(), "the heading has styled runs");
    assert!(
        node.el.spans.iter().all(|s| s.bold && s.scale > 1.0),
        "every run of the heading is bold and scaled: {:?}",
        node.el.spans
    );
}

/// The box a node's painted commands occupy, in window coordinates.
fn draw_box(frame: &Frame, node: usize) -> Option<(f32, f32, f32, f32)> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    let mut grow = |x: f32, y: f32| {
        b = Some(match b {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    };
    for c in &frame.dom.nodes[node].draw {
        match c {
            Cmd::Text { x, y, .. } => grow(*x, *y),
            Cmd::Fill { rect, .. } => {
                grow(rect.x, rect.y);
                grow(rect.x + rect.w, rect.y + rect.h);
            }
            Cmd::Lines { segs, .. } => {
                for s in segs {
                    grow(s[0], s[1]);
                    grow(s[2], s[3]);
                }
            }
            _ => {}
        }
    }
    b
}

/// The action of the innermost node under a point, the same way a click finds it.
fn action_at(frame: &Frame, x: f32, y: f32) -> Option<String> {
    frame.dom.action_at(x, y).map(|(_, a)| a.to_string())
}

#[test]
fn a_click_on_a_folded_callout_shows_its_body_in_place() {
    let src = "intro\n\n> [!success]- Solución\n> Cuerpo con $x^2$ aqui.\n\nAfter\n";
    let (_d, mut app) = note(src, "{}");
    cursor_at(&mut app, "After");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();

    // Folded: the title is all there is, and the formula of the hidden body is not collected.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let shown: Vec<String> = frame.info.blocks.iter().map(|b| b.display.clone()).collect();
    assert!(shown.contains(&"Solución".to_string()), "{shown:?}");
    assert!(!shown.iter().any(|t| t.contains("Cuerpo")), "the body is hidden: {shown:?}");
    assert!(frame.info.inline.is_empty(), "math of a folded body is not typeset");

    // The folded callout carries the toggle on the box the reader sees.
    let node = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.action.as_deref().is_some_and(|a| a.starts_with("callout:toggle:")))
        .expect("a folded callout is clickable");
    let action = frame.dom.nodes[node].el.action.clone().unwrap();
    let r = frame.dom.nodes[node].rect;
    assert_eq!(action_at(&frame, r.x + 4.0, r.y + 4.0).as_deref(), Some(action.as_str()));

    let caret = app.tab().unwrap().editor.buf.sels[0].head;
    app.click(&frame, &mut fonts, r.x + 4.0, r.y + 4.0, false);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let shown: Vec<String> = frame.info.blocks.iter().map(|b| b.display.clone()).collect();
    // The body is on screen, in place, and the block is still rendered: no markdown showed up.
    assert!(shown.iter().any(|t| t.contains("Cuerpo con")), "{shown:?}");
    assert!(!shown.iter().any(|t| t.contains("[!success]")), "the source never shows: {shown:?}");
    assert!(!shown.iter().any(|t| t.trim_start().starts_with('>')), "{shown:?}");
    // Its formula is typeset again, because the body it sits in is visible.
    assert_eq!(frame.info.inline.len(), 1, "the formula of the shown body is collected");
    // The caret did not move into the block: the click was the handle, not the text.
    assert_eq!(app.tab().unwrap().editor.buf.sels[0].head, caret);

    // The same click folds it back: the title line carries the toggle while the body is shown.
    let title = frame.info.blocks.iter().find(|b| b.display == "Solución").unwrap().node;
    let tr = frame.dom.nodes[title].rect;
    app.click(&frame, &mut fonts, tr.x + 2.0, tr.y + 2.0, false);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let shown: Vec<String> = frame.info.blocks.iter().map(|b| b.display.clone()).collect();
    assert!(shown.contains(&"Solución".to_string()), "{shown:?}");
    assert!(!shown.iter().any(|t| t.contains("Cuerpo")), "folded again: {shown:?}");
    assert!(frame.info.inline.is_empty());

    // In reading mode the same click toggles it, and no caret is involved.
    app.tabs[0].mode = ViewMode::Reading;
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let node = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.action.as_deref().is_some_and(|a| a.starts_with("callout:toggle:")))
        .expect("clickable in reading mode too");
    let r = frame.dom.nodes[node].rect;
    app.click(&frame, &mut fonts, r.x + 4.0, r.y + 4.0, false);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let shown: Vec<String> = frame.info.blocks.iter().map(|b| b.display.clone()).collect();
    assert!(shown.iter().any(|t| t.contains("Cuerpo con")), "{shown:?}");
}

#[test]
fn the_callout_toggle_is_kept_per_note() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), "{}").unwrap();
    // The same offsets in both notes, so an offset-keyed toggle would leak between them.
    let text = "intro\n\n> [!success]- Solución\n> Cuerpo.\n";
    fs::write(d.path().join("A.md"), text).unwrap();
    fs::write(d.path().join("B.md"), text).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let action = frame
        .dom
        .nodes
        .iter()
        .find_map(|n| n.el.action.clone().filter(|a| a.starts_with("callout:toggle:")))
        .expect("the callout can be opened");
    app.run(&action);
    let shown: Vec<String> =
        render(&mut app, &mut fonts, &mut pm, 1.0).info.blocks.iter().map(|b| b.display.clone()).collect();
    assert!(shown.iter().any(|t| t.contains("Cuerpo")), "{shown:?}");

    // Another note with the same layout opens folded: the toggle did not follow the file.
    app.open_note("B.md");
    let shown: Vec<String> =
        render(&mut app, &mut fonts, &mut pm, 1.0).info.blocks.iter().map(|b| b.display.clone()).collect();
    assert!(shown.contains(&"Solución".to_string()), "{shown:?}");
    assert!(!shown.iter().any(|t| t.contains("Cuerpo")), "{shown:?}");
}

#[test]
fn a_diagram_opens_pans_zooms_and_swaps_to_its_source() {
    let src = "intro\n\n```mermaid\ngraph LR\n  A --> B\n  B --> C\n```\n";
    let (_d, mut app) = note(src, "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let block = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.action.as_deref() == Some("mermaid:zoom"))
        .expect("the block opens the full-screen view");
    let br = frame.dom.nodes[block].rect;
    app.click(&frame, &mut fonts, br.x + 6.0, br.y + 6.0, false);
    assert!(app.diagram_zoom.is_some(), "a click on the block opens it");

    // The drawing gets a surface of its own: a click on it grabs it instead of closing the view.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let zoom = frame
        .info
        .rich
        .iter()
        .find(|it| frame.dom.nodes[it.node].el.action.as_deref() == Some("mermaid:drag"))
        .map(|it| it.node)
        .expect("the drawing has its own action");
    let zr = frame.dom.nodes[zoom].rect;
    let (cx, cy) = (zr.x + zr.w / 2.0, zr.y + zr.h / 2.0);
    let fitted = draw_box(&frame, zoom).expect("the diagram is painted");
    app.click(&frame, &mut fonts, cx, cy, false);
    assert!(app.diagram_zoom.is_some(), "a click on the diagram closed the view");

    // The same press grabs it: dragging moves it, and the painter draws it where it was left.
    app.drag(&frame, &mut fonts, cx + 40.0, cy + 24.0);
    app.release();
    assert_eq!(app.diagram_zoom_pan, (40.0, 24.0));
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let moved = draw_box(&frame, zoom).expect("the diagram is painted");
    assert!((moved.0 - fitted.0 - 40.0).abs() < 1.5, "{moved:?} vs {fitted:?}");
    assert!((moved.1 - fitted.1 - 24.0).abs() < 1.5, "{moved:?} vs {fitted:?}");

    // The wheel zooms it, and the drawing grows with the factor.
    app.wheel(&frame, cx, cy, -6.0);
    assert!(app.diagram_zoom_scale > 1.0, "scale {}", app.diagram_zoom_scale);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let zoomed = draw_box(&frame, zoom).expect("the diagram is painted");
    assert!(zoomed.2 - zoomed.0 > moved.2 - moved.0, "{zoomed:?} vs {moved:?}");
    // Zooming out runs into a floor instead of turning the diagram inside out.
    for _ in 0..40 {
        app.wheel(&frame, cx, cy, 20.0);
    }
    assert_eq!(app.diagram_zoom_scale, metamind_app::MIN_DIAGRAM_ZOOM);

    // A double click inside the view shows its source, in the same box. The view has been open
    // for a while by now, so the pair of presses is its own and not the block's.
    std::thread::sleep(std::time::Duration::from_millis(420));
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    app.click(&frame, &mut fonts, cx, cy, false);
    app.click(&frame, &mut fonts, cx, cy, false);
    assert!(app.diagram_zoom_source, "the view did not swap to its source");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let texts: Vec<&str> = frame.dom.nodes.iter().filter_map(|n| n.el.text.as_deref()).collect();
    assert!(texts.iter().any(|t| t.contains("graph LR")), "the source is not on screen: {texts:?}");
    assert!(frame.info.rich.len() < 2, "the drawing is not painted while the source shows");

    // Escape closes it and forgets how it was left.
    app.key(key("Escape"));
    assert!(app.diagram_zoom.is_none());
    assert_eq!(app.diagram_zoom_pan, (0.0, 0.0));
    assert_eq!(app.diagram_zoom_scale, 1.0);
}

#[test]
fn a_double_click_swaps_a_diagram_for_its_source_in_place() {
    let src = "intro\n\n```mermaid\ngraph LR\n  A --> B\n  B --> C\n```\n\nAfter\n";
    let (_d, mut app) = note(src, "{}");
    cursor_at(&mut app, "After");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let block = frame
        .dom
        .nodes
        .iter()
        .position(|n| n.el.action.as_deref() == Some("mermaid:zoom"))
        .expect("the block opens the full-screen view");
    let br = frame.dom.nodes[block].rect;
    let (x, y) = (br.x + 6.0, br.y + 6.0);
    // Two presses in the same spot: the second is the double click.
    app.click(&frame, &mut fonts, x, y, false);
    app.click(&frame, &mut fonts, x, y, false);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.rich.is_empty(), "the drawing is gone");
    let shown: Vec<String> = frame.info.blocks.iter().map(|b| b.display.clone()).collect();
    assert!(shown.iter().any(|t| t.contains("graph LR")), "the source is shown: {shown:?}");
    assert!(app.diagram_zoom.is_none(), "the double click did not leave the overlay open");

    // And back: the same two presses on the source return the drawing.
    let block = frame.info.blocks.iter().find(|b| b.display.contains("graph LR")).unwrap().node;
    let br = frame.dom.nodes[block].rect;
    let (x, y) = (br.x + 6.0, br.y + 6.0);
    app.click(&frame, &mut fonts, x, y, false);
    app.click(&frame, &mut fonts, x, y, false);
    assert!(app.mermaid_source.is_empty(), "the block draws its diagram again");
    // The caret went into the block on the first press, which is what shows it as source: move
    // it away and the drawing is back.
    cursor_at(&mut app, "After");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(frame.info.rich.len(), 1, "the drawing is back");

    // A slow pair of clicks is two clicks: the view opens and the block is untouched.
    let br = frame.dom.nodes[frame.info.rich[0].node].rect;
    app.click(&frame, &mut fonts, br.x + 6.0, br.y + 6.0, false);
    std::thread::sleep(std::time::Duration::from_millis(450));
    app.click(&frame, &mut fonts, br.x + 6.0, br.y + 6.0, false);
    assert!(app.diagram_zoom.is_some(), "the view stays open");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(frame.info.rich.len(), 2, "the overlay still draws the diagram");
}
