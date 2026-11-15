use rustsidian_app::shell::Panel;
use rustsidian_app::{render, App};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("dir")).unwrap();
    for (p, t) in [
        ("A.md", "see [[B]] #topic #Area/sub\n"),
        ("B.md", "---\ntags: [topic]\n---\nb\n"),
        ("dir/C.md", "alone\n"),
    ] {
        fs::write(d.path().join(p), t).unwrap();
    }
    d
}

fn texts(app: &mut App) -> Vec<String> {
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 700).unwrap();
    let frame = render(app, &mut fonts, &mut pm, 1.0);
    frame.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect()
}

fn bookmarks(d: &tempfile::TempDir) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(d.path().join(".obsidian/bookmarks.json")).unwrap()).unwrap()
}

#[test]
fn bookmark_toggle_follows_rename_and_removes() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:dir/C.md");
    app.run("bookmarks:bookmark-current-view");
    assert_eq!(bookmarks(&d)["items"][0]["path"], "dir/C.md");
    app.run("bookmarks:open");
    assert_eq!(app.panel, Panel::Bookmarks);
    assert!(texts(&mut app).iter().any(|t| t == "C"));

    app.run("rename:dir/C.md|Renamed");
    assert_eq!(bookmarks(&d)["items"][0]["path"], "dir/Renamed.md");

    app.search_query = "topic".into();
    app.run("bookmarks:bookmark-current-search");
    assert_eq!(bookmarks(&d)["items"][1]["query"], "topic");
    app.run("bookmarks:search:topic");
    assert_eq!(app.panel, Panel::Search);
    assert_eq!(app.search_results.len(), 2);

    app.run("bookmarks:remove:0");
    assert_eq!(bookmarks(&d)["items"].as_array().unwrap().len(), 1);
    // Toggling twice leaves no bookmark behind.
    app.run("bookmark:A.md");
    app.run("bookmark:A.md");
    assert_eq!(bookmarks(&d)["items"].as_array().unwrap().len(), 1);
}

#[test]
fn tags_pane_counts_notes_per_tag() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("tag-pane:open");
    let t = texts(&mut app);
    let at = t.iter().position(|x| x == "#topic").expect("topic listed");
    assert_eq!(t[at + 1], "2");
    assert!(t.iter().any(|x| x == "#sub"));
    app.run("tags:search:topic");
    assert_eq!(app.search_query, "tag:#topic");
    assert_eq!(app.search_results.len(), 2);
}
