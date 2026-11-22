use metamind_app::shell::{Panel, ViewMode};
use metamind_app::App;
use std::fs;

#[test]
fn reopening_a_vault_restores_tabs_and_sidebar() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("dir")).unwrap();
    fs::write(d.path().join("A.md"), "# One\nx\n# Two\ny\n").unwrap();
    fs::write(d.path().join("dir/B.md"), "b\n").unwrap();
    fs::write(d.path().join("C.md"), "c\n").unwrap();

    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.tabs[0].mode = ViewMode::Source;
    app.tabs[0].folds.fold(0);
    app.open_note("C.md");
    app.open_note("dir/B.md");
    app.open_note("A.md");
    app.panel = Panel::Tags;
    app.collapsed.insert("dir".into());
    app.save_all();
    // A note deleted between sessions is skipped.
    fs::remove_file(d.path().join("C.md")).unwrap();

    let mut app = App::open(d.path()).unwrap();
    app.restore_session();
    let paths: Vec<&str> = app.tabs.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(paths, ["A.md", "dir/B.md"]);
    assert_eq!(app.active, 0);
    assert_eq!(app.tabs[0].mode, ViewMode::Source);
    assert!(app.tabs[0].folds.is_folded(0));
    assert_eq!(app.panel, Panel::Tags);
    assert!(app.collapsed.contains("dir"));
}
