use rustsidian_app::{render, App};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

#[test]
fn unlinked_mentions_are_listed_and_linked() {
    let d = tempfile::tempdir().unwrap();
    for (p, t) in [
        ("Plan.md", "---\naliases: [Roadmap]\n---\nbody\n"),
        ("Notes.md", "The plan and the roadmap. Planning is not a mention.\n"),
        ("Linked.md", "See [[Plan]]; the plan again.\n"),
        ("Other.md", "nothing here\n"),
    ] {
        fs::write(d.path().join(p), t).unwrap();
    }
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Plan.md");
    app.right_open = true;
    app.run("backlinks:toggle-unlinked");
    app.refresh_derived();
    let files: Vec<&str> = app.mentions.as_ref().unwrap().files.iter().map(|f| f.0.as_str()).collect();
    // Notes that already link are listed as backlinks instead.
    assert_eq!(files, ["Notes.md"]);

    app.run("backlinks:link:Notes.md");
    assert_eq!(
        fs::read_to_string(d.path().join("Notes.md")).unwrap(),
        "The [[Plan|plan]] and the [[Plan|roadmap]]. Planning is not a mention.\n"
    );
    app.refresh_derived();
    assert!(app.mentions.as_ref().unwrap().files.is_empty());
}

#[test]
fn linked_mentions_show_context_and_panels_switch() {
    let d = tempfile::tempdir().unwrap();
    for (p, t) in [
        ("Plan.md", "Goes to [[Missing]] and [[Other]].\n"),
        ("Alpha.md", "first line\nSee [[Plan|the plan]] today.\n"),
        ("Zed.md", "[[Plan]]\n"),
        ("Other.md", "nothing here\n"),
    ] {
        fs::write(d.path().join(p), t).unwrap();
    }
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Plan.md");
    app.right_open = true;
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1200, 800).unwrap();
    let texts = |frame: &rustsidian_app::Frame| -> Vec<String> {
        frame.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect()
    };
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let t = texts(&frame);
    assert!(t.iter().any(|s| s == "See [[Plan|the plan]] today."), "{t:?}");
    let order = |t: &[String]| (t.iter().rposition(|s| s == "Alpha"), t.iter().rposition(|s| s == "Zed"));
    let (a, z) = order(&t);
    assert!(a < z);

    app.run("backlinks:toggle-sort");
    app.run("backlinks:toggle-collapse");
    let t = texts(&render(&mut app, &mut fonts, &mut pm, 1.0));
    let (a, z) = order(&t);
    assert!(z < a);
    assert!(!t.iter().any(|s| s.contains("today")), "collapsed results hide context");

    app.run("backlinks:toggle-collapse");
    app.run("backlinks:toggle-filter");
    app.text_input("TODAY");
    let t = texts(&render(&mut app, &mut fonts, &mut pm, 1.0));
    let count = |name: &str| t.iter().filter(|s| *s == name).count();
    // Each name is also in the file list.
    assert_eq!((count("Alpha"), count("Zed")), (2, 1), "the filter keeps notes whose snippets match");
    app.run("backlinks:toggle-context");
    app.refresh_derived();
    let linked = &app.linked.as_ref().unwrap().files;
    assert_eq!(linked[0].1, ["See [[Plan|the plan]] today."]);
    app.key(rustsidian_app::Key::parse("Escape").unwrap());
    app.text_input("x");
    assert_eq!(
        app.backlinks_filter.as_deref(),
        Some("TODAY"),
        "typing goes elsewhere once the field is left"
    );

    app.run("outgoing-links:open");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some("open:Other.md")));
    assert!(frame.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some("create:Missing")));
}
