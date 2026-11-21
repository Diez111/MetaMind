use rustsidian_app::App;
use std::fs;

#[test]
fn dropped_files_are_copied_and_embedded() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("notes")).unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"attachmentFolderPath":"./assets"}"#).unwrap();
    fs::write(d.path().join("notes/N.md"), "").unwrap();
    fs::write(d.path().join("Other.md"), "").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let pic = outside.path().join("pic.png");
    fs::write(&pic, b"not really a png").unwrap();

    let mut app = App::open(d.path()).unwrap();
    app.run("open:notes/N.md");
    app.import_files(&[&pic]);
    app.import_files(&[&pic, &d.path().join("Other.md")]);
    assert_eq!(fs::read(d.path().join("notes/assets/pic.png")).unwrap(), b"not really a png");
    assert!(d.path().join("notes/assets/pic 1.png").is_file());
    assert_eq!(app.tab().unwrap().editor.buf.text(), "![[pic.png]]![[pic 1.png]]\n[[Other]]");

    // A directory is refused with a notice and nothing is inserted.
    app.import_files(&[outside.path()]);
    assert!(app.notice.as_deref().unwrap().contains("solo se pueden añadir archivos"));
    assert_eq!(app.tab().unwrap().editor.buf.text(), "![[pic.png]]![[pic 1.png]]\n[[Other]]");
}

#[test]
fn markdown_link_setting_and_no_open_note() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(
        d.path().join(".obsidian/app.json"),
        r#"{"attachmentFolderPath":"files","useMarkdownLinks":true}"#,
    )
    .unwrap();
    fs::write(d.path().join("N.md"), "").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let doc = outside.path().join("my doc.pdf");
    fs::write(&doc, b"%PDF").unwrap();

    let mut app = App::open(d.path()).unwrap();
    app.import_files(&[&doc]);
    assert_eq!(app.notice.as_deref(), Some("Añadido files/my doc.pdf"));
    app.run("open:N.md");
    app.import_files(&[&doc]);
    assert_eq!(app.tab().unwrap().editor.buf.text(), "![my doc 1](my%20doc%201.pdf)");
}

#[test]
fn pasted_images_are_saved_and_embedded() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"attachmentFolderPath":"img"}"#).unwrap();
    fs::write(d.path().join("N.md"), "").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("open:N.md");
    app.import_png(b"png bytes");
    app.import_png(b"second");
    let names: Vec<String> = fs::read_dir(d.path().join("img"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2);
    let text = app.tab().unwrap().editor.buf.text();
    for name in &names {
        assert!(name.starts_with("Pasted image ") && name.ends_with(".png"), "{name}");
        assert!(text.contains(&format!("![[{name}]]")), "{text}");
    }
}
