use metamind_app::shell::Key;
use metamind_app::suggest::SuggestKind;
use metamind_app::{render, App};
use metamind_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("dir")).unwrap();
    fs::write(d.path().join("Home.md"), "").unwrap();
    fs::write(d.path().join("dir/Second note.md"), "---\ntags: [x]\n---\n# Second\nfirst line\n").unwrap();
    d
}

fn key(app: &mut App, s: &str) {
    app.key(Key::parse(s).unwrap());
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.text_input(&c.to_string());
    }
}

fn text(app: &App) -> String {
    app.tab().unwrap().editor.buf.slice(0, app.tab().unwrap().editor.buf.len()).into_owned()
}

#[test]
fn link_suggestion_completes_with_preview() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Home.md");
    typed(&mut app, "see [[sec");
    let s = app.suggest.as_ref().expect("suggestion open");
    assert_eq!(s.kind, SuggestKind::Link);
    assert_eq!(s.items[0].label, "Second note");
    assert_eq!(s.preview, "# Second\nfirst line");

    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let row = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("suggest:0")).unwrap();
    assert!(row.rect.w > 0.0 && row.rect.h > 0.0);

    key(&mut app, "Enter");
    assert_eq!(text(&app), "see [[Second note]]");
    assert!(app.suggest.is_none());
    assert_eq!(app.tab().unwrap().editor.buf.sels[0].head, text(&app).len());
}

#[test]
fn escape_dismisses_until_the_trigger_changes() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Home.md");
    typed(&mut app, "[[");
    assert!(app.suggest.is_some());
    key(&mut app, "Escape");
    assert!(app.suggest.is_none());
    typed(&mut app, "s");
    assert!(app.suggest.is_none());
}

#[test]
fn slash_runs_a_command_in_place_of_the_query() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Home.md");
    typed(&mut app, "x /toggle bold");
    // A space ends the query, so nothing is offered.
    assert!(app.suggest.is_none());
    key(&mut app, "Enter");
    typed(&mut app, "/negrita");
    assert_eq!(app.suggest.as_ref().map(|s| s.kind), Some(SuggestKind::Slash));
    assert_eq!(app.suggest.as_ref().unwrap().items[0].action, "editor:toggle-bold");
    key(&mut app, "Enter");
    assert!(text(&app).ends_with("\n****"), "{:?}", text(&app));
}

#[test]
fn nothing_is_suggested_in_code_blocks() {
    let d = vault();
    fs::write(d.path().join("Home.md"), "```\ncd \n```\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Home.md");
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(7)]);
    typed(&mut app, "/tog");
    assert!(app.suggest.is_none());
}

#[test]
fn ctrl_hover_on_a_link_previews_its_note() {
    let d = vault();
    fs::write(d.path().join("Home.md"), "intro\n\nsee [[Second note]] here\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Home.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let block = frame.info.blocks.iter().find(|b| b.display.contains("here")).unwrap();
    let r = frame.dom.nodes[block.node].rect;
    let y = r.y + r.h / 2.0;
    let xs: Vec<f32> = (0..r.w as usize).step_by(4).map(|dx| r.x + dx as f32).collect();
    assert!(!xs.iter().any(|&x| app.hover_link(&frame, &mut fonts, x, y, false)));
    assert!(xs.iter().any(|&x| app.hover_link(&frame, &mut fonts, x, y, true)));
    let h = app.hover_preview.as_ref().unwrap();
    assert_eq!((h.path.as_str(), h.text.as_str()), ("dir/Second note.md", "# Second\nfirst line\n"));
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.dom.nodes.iter().any(
        |n| n.el.text.as_deref() == Some("first line") && n.el.classes.iter().any(|c| c == "embed-line")
    ));
    // Over plain text the preview closes.
    assert!(app.hover_link(&frame, &mut fonts, r.x + 2.0, y, true));
    assert!(app.hover_preview.is_none());
}
