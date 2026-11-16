use rustsidian_app::uri::{parse, resolve, Target};
use std::fs;

#[test]
fn parses_action_and_decoded_params() {
    let (action, params) = parse("rustsidian://open?vault=My%20Vault&file=dir%2FNote").unwrap();
    assert_eq!(action, "open");
    assert_eq!(params, vec![("vault".into(), "My Vault".into()), ("file".into(), "dir/Note".into())]);
    assert_eq!(parse("RUSTSIDIAN:new/").unwrap().0, "new");
    assert!(parse("https://example.com").is_err());
}

#[test]
fn opens_by_vault_name_and_by_path() {
    let d = tempfile::tempdir().unwrap();
    let vault = d.path().join("Notes");
    fs::create_dir_all(vault.join("dir")).unwrap();
    fs::write(vault.join("dir/A.md"), "a").unwrap();
    let vaults = vec![vault.clone()];

    let t = resolve("rustsidian://open?vault=notes&file=dir/A", &vaults).unwrap();
    assert_eq!(t, Target { vault: vault.clone(), note: Some("dir/A".into()) });
    assert!(resolve("rustsidian://open?vault=Other", &vaults).is_err());

    let file = vault.join("dir/A.md");
    let link = format!("rustsidian://open?path={}", file.to_string_lossy());
    assert_eq!(
        resolve(&link, &vaults).unwrap(),
        Target { vault: vault.clone(), note: Some("dir/A.md".into()) }
    );

    // An unknown vault is found by its settings folder.
    fs::create_dir_all(vault.join(".obsidian")).unwrap();
    assert_eq!(resolve(&link, &[]).unwrap().vault, vault);

    let folder = format!("rustsidian://open?path={}", vault.to_string_lossy());
    assert_eq!(resolve(&folder, &[]).unwrap(), Target { vault: vault.clone(), note: None });
    assert!(resolve("rustsidian://frobnicate", &vaults).is_err());
}

#[test]
fn new_writes_a_note_without_overwriting() {
    let d = tempfile::tempdir().unwrap();
    let vault = d.path().to_path_buf();
    let vaults = vec![vault.clone()];
    let t = resolve("rustsidian://new?name=Inbox%2FIdea&content=hello%20world", &vaults).unwrap();
    assert_eq!(t.note.as_deref(), Some("Inbox/Idea.md"));
    assert_eq!(fs::read_to_string(vault.join("Inbox/Idea.md")).unwrap(), "hello world");

    let t = resolve("rustsidian://new?name=Inbox/Idea.md&content=again", &vaults).unwrap();
    assert_eq!(t.note.as_deref(), Some("Inbox/Idea 1.md"));
    assert_eq!(fs::read_to_string(vault.join("Inbox/Idea.md")).unwrap(), "hello world");

    assert!(resolve("rustsidian://new?name=../escape", &vaults).is_err());
    assert!(!d.path().parent().unwrap().join("escape.md").exists());
}

#[test]
fn local_file_accepts_paths_and_file_urls() {
    use rustsidian_app::uri::local_file;
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("My Note.md");
    fs::write(&file, "x").unwrap();
    let canon = rustsidian_app::recent::clean(&fs::canonicalize(&file).unwrap());
    assert_eq!(local_file(&file.to_string_lossy()), Some(canon.clone()));
    // `file:///home/x` on Unix, `file:///C:/x` on Windows.
    let slashed = file.to_string_lossy().replace('\\', "/").replace(' ', "%20");
    let url = format!("file:///{}", slashed.trim_start_matches('/'));
    assert_eq!(local_file(&url), Some(canon));
    assert_eq!(local_file(&d.path().to_string_lossy()), None, "a folder is not a file");
    assert_eq!(local_file("file://elsewhere/x.md"), None);
}
