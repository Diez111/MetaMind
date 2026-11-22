use metamind_app::instance::{forward, same_vault, Instance};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn forwards_only_to_the_instance_showing_the_vault() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("instances");
    let (vault, other) = (d.path().join("v"), d.path().join("w"));
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::create_dir_all(&other).unwrap();

    assert!(!forward(&dir, &vault, None), "nothing is running");

    let (tx, rx) = mpsc::channel();
    let shown = vault.clone();
    Instance::register(&dir).unwrap().serve(move |v, note| {
        let mine = same_vault(&shown, v);
        if mine {
            tx.send(note).unwrap();
        }
        mine
    });
    assert!(forward(&dir, &vault.join("."), Some("dir/A.md")));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().as_deref(), Some("dir/A.md"));
    assert!(forward(&dir, &vault, None));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), None);
    assert!(!forward(&dir, &other, Some("x")), "another vault starts its own window");

    // A record left by a process that is gone is cleared, and a bad token is refused.
    std::fs::write(dir.join("999999999"), "1\nnope\n").unwrap();
    let record = std::fs::read_dir(&dir).unwrap().flatten().find(|e| e.file_name() != "999999999").unwrap();
    let port = std::fs::read_to_string(record.path()).unwrap().lines().next().unwrap().to_string();
    std::fs::write(dir.join("forged"), format!("{port}\nwrong\n")).unwrap();
    std::fs::remove_file(record.path()).unwrap();
    assert!(!forward(&dir, &vault, Some("x")), "a wrong token is refused");
    assert!(!dir.join("999999999").exists());
    assert!(rx.try_recv().is_err());
}
