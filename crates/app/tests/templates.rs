use metamind_app::App;
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::create_dir_all(d.path().join("Tpl")).unwrap();
    fs::write(d.path().join("Tpl/Meeting.md"), "# {{title}}\nattendees:\n").unwrap();
    fs::write(d.path().join("Tpl/Day.md"), "Daily {{title}} {{unknown}}\n").unwrap();
    fs::write(d.path().join("Note.md"), "").unwrap();
    fs::write(d.path().join(".obsidian/templates.json"), r#"{"folder":"Tpl"}"#).unwrap();
    d
}

#[test]
fn template_picker_inserts_filled_template() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:Note.md");
    app.run("templates:insert-template");
    let items: Vec<String> = app.prompt.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert_eq!(items, ["Day", "Meeting"]);
    app.run("templates:insert:Tpl/Meeting.md");
    assert!(app.prompt.is_none());
    assert_eq!(app.tab().unwrap().editor.buf.text(), "# Note\nattendees:\n");
}

#[test]
fn daily_note_uses_folder_format_and_template() {
    let d = vault();
    fs::write(
        d.path().join(".obsidian/daily-notes.json"),
        r#"{"folder":"Journal/","format":"[Day] YYYY","template":"Tpl/Day"}"#,
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("daily-notes");
    let p = app.tab().unwrap().path.clone();
    assert!(p.starts_with("Journal/Day 20"), "{p}");
    let name = p.trim_start_matches("Journal/").trim_end_matches(".md").to_string();
    assert_eq!(app.tab().unwrap().editor.buf.text(), format!("Daily {name} {{{{unknown}}}}\n"));
    // Running it again opens the same note rather than creating another.
    app.run("daily-notes");
    assert_eq!(app.tab().unwrap().path, p);
}

#[test]
fn unique_note_uses_settings_and_never_reuses_a_name() {
    let d = vault();
    fs::write(
        d.path().join(".obsidian/zk-prefixer.json"),
        r#"{"folder":"Zettel","format":"[Z]YYYY","template":"Tpl/Day"}"#,
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("zk-prefixer");
    let first = app.tab().unwrap().path.clone();
    assert!(first.starts_with("Zettel/Z20") && first.ends_with(".md"), "{first}");
    assert!(app.tab().unwrap().editor.buf.text().starts_with("Daily Z20"));
    app.run("zk-prefixer");
    let second = app.tab().unwrap().path.clone();
    assert_eq!(second, first.replace(".md", " 1.md"));
    assert!(d.path().join(&second).is_file());
}

#[test]
fn previous_and_next_daily_notes_skip_missing_days() {
    let d = vault();
    fs::write(d.path().join(".obsidian/daily-notes.json"), r#"{"folder":"Daily"}"#).unwrap();
    fs::create_dir_all(d.path().join("Daily")).unwrap();
    fs::write(d.path().join("Daily/2020-01-01.md"), "one\n").unwrap();
    fs::write(d.path().join("Daily/2020-01-05.md"), "five\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("daily-notes");
    let path = |app: &App| app.tab().unwrap().path.clone();
    app.run("daily-notes:goto-prev");
    assert_eq!(path(&app), "Daily/2020-01-05.md");
    app.run("daily-notes:goto-prev");
    assert_eq!(path(&app), "Daily/2020-01-01.md");
    app.run("daily-notes:goto-prev");
    assert_eq!(app.notice.as_deref(), Some("No hay nota diaria anterior"));
    assert_eq!(path(&app), "Daily/2020-01-01.md");
    app.run("daily-notes:goto-next");
    assert_eq!(path(&app), "Daily/2020-01-05.md");
}
