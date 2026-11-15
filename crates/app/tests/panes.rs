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

#[test]
fn explorer_sort_orders() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let files = |app: &mut App| -> Vec<String> {
        app.refresh_derived();
        app.tree_rows().iter().map(|r| r.path.clone()).collect()
    };
    assert_eq!(files(&mut app), ["dir", "dir/C.md", "A.md", "B.md"]);
    app.run("file-explorer:sort:alphabeticalReverse");
    // Folders stay first and in name order.
    assert_eq!(files(&mut app), ["dir", "dir/C.md", "B.md", "A.md"]);
    let saved = fs::read_to_string(d.path().join(".obsidian/app.json")).unwrap();
    assert!(saved.contains("\"fileSortOrder\": \"alphabeticalReverse\""), "{saved}");
}

#[test]
fn explorer_drag_moves_files_and_a_still_press_opens() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 700).unwrap();
    // Centre of the explorer row labelled `label`.
    let row = |app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap, label: &str| {
        let frame = render(app, fonts, pm, 1.0);
        let r = frame.dom.nodes.iter().find(|n| n.el.text.as_deref() == Some(label)).unwrap().rect;
        (frame, r.x + 4.0, r.y + r.h / 2.0)
    };

    let (frame, x, y) = row(&mut app, &mut fonts, &mut pm, "A");
    app.click(&frame, &mut fonts, x, y, false);
    app.release();
    assert_eq!(app.tab().map(|t| t.path.as_str()), Some("A.md"));

    let (frame, x, y) = row(&mut app, &mut fonts, &mut pm, "A");
    let (_, dx, dy) = row(&mut app, &mut fonts, &mut pm, "dir");
    app.click(&frame, &mut fonts, x, y, false);
    app.drag(&frame, &mut fonts, dx, dy);
    app.release();
    assert!(d.path().join("dir/A.md").exists());
    assert_eq!(app.tab().map(|t| t.path.as_str()), Some("dir/A.md"));

    // Dropping on a root file moves into the root; dropping a folder into itself does nothing.
    let (frame, x, y) = row(&mut app, &mut fonts, &mut pm, "C");
    let (_, bx, by) = row(&mut app, &mut fonts, &mut pm, "B");
    app.click(&frame, &mut fonts, x, y, false);
    app.drag(&frame, &mut fonts, bx, by);
    app.release();
    assert!(d.path().join("C.md").exists());
    let (frame, x, y) = row(&mut app, &mut fonts, &mut pm, "dir");
    let (_, ax, ay) = row(&mut app, &mut fonts, &mut pm, "A");
    app.click(&frame, &mut fonts, x, y, false);
    app.drag(&frame, &mut fonts, ax, ay);
    app.release();
    assert!(d.path().join("dir/A.md").exists());
}
