use rustsidian_app::App;
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
