use metamind_app::uri::{parse, resolve, Target};
use metamind_app::App;
use std::fs;

#[test]
fn parses_action_and_decoded_params() {
    let (action, params) = parse("metamind://open?vault=My%20Vault&file=dir%2FNote").unwrap();
    assert_eq!(action, "open");
    assert_eq!(params, vec![("vault".into(), "My Vault".into()), ("file".into(), "dir/Note".into())]);
    assert_eq!(parse("METAMIND:new/").unwrap().0, "new");
    assert!(parse("https://example.com").is_err());
}

#[test]
fn opens_by_vault_name_and_by_path() {
    let d = tempfile::tempdir().unwrap();
    let vault = d.path().join("Notes");
    fs::create_dir_all(vault.join("dir")).unwrap();
    fs::write(vault.join("dir/A.md"), "a").unwrap();
    let vaults = vec![vault.clone()];

    let t = resolve("metamind://open?vault=notes&file=dir/A", &vaults).unwrap();
    assert_eq!(t, Target { vault: vault.clone(), note: Some("dir/A".into()) });
    assert!(resolve("metamind://open?vault=Other", &vaults).is_err());

    let file = vault.join("dir/A.md");
    let link = format!("metamind://open?path={}", file.to_string_lossy());
    assert_eq!(
        resolve(&link, &vaults).unwrap(),
        Target { vault: vault.clone(), note: Some("dir/A.md".into()) }
    );

    // An unknown vault is found by its settings folder.
    fs::create_dir_all(vault.join(".obsidian")).unwrap();
    assert_eq!(resolve(&link, &[]).unwrap().vault, vault);

    let folder = format!("metamind://open?path={}", vault.to_string_lossy());
    assert_eq!(resolve(&folder, &[]).unwrap(), Target { vault: vault.clone(), note: None });
    assert!(resolve("metamind://frobnicate", &vaults).is_err());
}

#[test]
fn new_writes_a_note_without_overwriting() {
    let d = tempfile::tempdir().unwrap();
    let vault = d.path().to_path_buf();
    let vaults = vec![vault.clone()];
    let t = resolve("metamind://new?name=Inbox%2FIdea&content=hello%20world", &vaults).unwrap();
    assert_eq!(t.note.as_deref(), Some("Inbox/Idea.md"));
    assert_eq!(fs::read_to_string(vault.join("Inbox/Idea.md")).unwrap(), "hello world");

    let t = resolve("metamind://new?name=Inbox/Idea.md&content=again", &vaults).unwrap();
    assert_eq!(t.note.as_deref(), Some("Inbox/Idea 1.md"));
    assert_eq!(fs::read_to_string(vault.join("Inbox/Idea.md")).unwrap(), "hello world");

    assert!(resolve("metamind://new?name=../escape", &vaults).is_err());
    assert!(!d.path().parent().unwrap().join("escape.md").exists());
}

#[test]
fn local_file_accepts_paths_and_file_urls() {
    use metamind_app::uri::local_file;
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("My Note.md");
    fs::write(&file, "x").unwrap();
    let canon = metamind_app::recent::clean(&fs::canonicalize(&file).unwrap());
    assert_eq!(local_file(&file.to_string_lossy()), Some(canon.clone()));
    // `file:///home/x` on Unix, `file:///C:/x` on Windows.
    let slashed = file.to_string_lossy().replace('\\', "/").replace(' ', "%20");
    let url = format!("file:///{}", slashed.trim_start_matches('/'));
    assert_eq!(local_file(&url), Some(canon));
    assert_eq!(local_file(&d.path().to_string_lossy()), None, "a folder is not a file");
    assert_eq!(local_file("file://elsewhere/x.md"), None);
}

/// A link written as a relative path — `[grabación](Videos/U1.mp4)`, which is how the vault's
/// tables name their recordings — opens the file instead of answering "no es un enlace web".
#[test]
fn a_relative_link_opens_the_file_it_names() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Clase.md"), "# Clase\n").unwrap();
    std::fs::write(d.path().join("clip.mp4"), b"no es un video").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Clase.md");
    let before = app.tab().map(|t| t.path.clone());
    app.run("open-url:clip.mp4");
    let notice = app.notice.clone().unwrap_or_default();
    assert!(!notice.contains("No es un enlace web"), "{notice}");
    // The video goes to the player; a markdown file opens as a note.
    assert!(app.video.is_open() || app.notice.is_some(), "el archivo se abrió o dijo por qué no");
    std::fs::write(d.path().join("Otra.md"), "# Otra\n").unwrap();
    app.files_changed(&["Otra.md".to_string()]);
    app.run("open-url:Otra.md");
    assert_eq!(
        app.tab().map(|t| t.path.clone()),
        Some("Otra.md".to_string()),
        "el enlace relativo abre la nota"
    );
    let _ = before;
}
