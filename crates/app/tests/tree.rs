//! The explorer tree: which folders are open, how deep each row really is, and the folders a
//! session brings back.

use metamind_app::App;
use std::fs;

fn w(root: &std::path::Path, p: &str, t: &str) {
    let abs = root.join(p);
    fs::create_dir_all(abs.parent().unwrap()).unwrap();
    fs::write(abs, t).unwrap();
}

/// A vault two years deep: `1° Año` sits under two different parents, and there is a file next
/// to the folders so the root has one too.
fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    w(r, "Facultad/Materias/1° Año/Algebra.md", "a\n");
    w(r, "Facultad/Materias/1° Año/Analisis.md", "a\n");
    w(r, "Facultad/Años/1° Año/Fisica.md", "f\n");
    w(r, "Facultad/Años/2° Año/Quimica.md", "q\n");
    w(r, "Facultad/Años/3° Año/Historia.md", "h\n");
    w(r, "Facultad/Años/4° Año/Matematica.md", "m\n");
    d
}

fn rows(app: &mut App) -> Vec<(usize, String)> {
    app.refresh_derived();
    app.tree_rows().iter().map(|r| (r.depth, r.label.clone())).collect()
}

fn open_all(app: &mut App) {
    for f in ["Facultad", "Facultad/Años", "Facultad/Materias"] {
        app.expanded.insert(f.to_string());
    }
    for y in ["1° Año", "2° Año", "3° Año", "4° Año"] {
        app.expanded.insert(format!("Facultad/Años/{y}"));
    }
    app.expanded.insert("Facultad/Materias/1° Año".into());
}

fn labels(rows: &[(usize, String)]) -> Vec<(usize, &str)> {
    rows.iter().map(|(d, l)| (*d, l.as_str())).collect()
}

#[test]
fn tree_starts_with_every_folder_folded() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    assert_eq!(labels(&rows(&mut app)), [(0, "Facultad")]);
    assert!(app.tree_rows()[0].collapsed);
}

#[test]
fn same_folder_name_under_two_parents_sits_at_its_own_depth() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    open_all(&mut app);
    // Both `1° Año` folders are listed once each, at the depth their path really has, and their
    // files hang under the right one.
    assert_eq!(
        labels(&rows(&mut app)),
        [
            (0, "Facultad"),
            (1, "Años"),
            (2, "1° Año"),
            (3, "Fisica"),
            (2, "2° Año"),
            (3, "Quimica"),
            (2, "3° Año"),
            (3, "Historia"),
            (2, "4° Año"),
            (3, "Matematica"),
            (1, "Materias"),
            (2, "1° Año"),
            (3, "Algebra"),
            (3, "Analisis"),
        ]
    );
    let ones: Vec<&str> = app
        .tree_rows()
        .iter()
        .filter(|r| r.label == "1° Año")
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(ones, ["Facultad/Años/1° Año", "Facultad/Materias/1° Año"]);
}

#[test]
fn one_folder_spelled_two_ways_is_listed_once() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    // A case-sensitive disk allows both spellings of the same branch; the walk folds case when it
    // groups folders, so the branch is not listed twice.
    w(r, "Facultad/Materias/1° Año/Algebra.md", "a\n");
    w(r, "facultad/materias/1° año/Fisica.md", "f\n");
    let mut app = App::open(r).unwrap();
    for f in ["Facultad", "Facultad/Materias", "Facultad/Materias/1° Año"] {
        app.expanded.insert(f.to_string());
    }
    let mut seen = std::collections::HashSet::new();
    let dup: Vec<String> = app
        .tree_rows()
        .iter()
        .map(|x| x.path.clone())
        .filter(|p| !seen.insert(p.clone()))
        .collect();
    assert!(dup.is_empty(), "a path is listed twice: {dup:?}");
    let built = rows(&mut app);
    for name in ["facultad", "materias", "1° año"] {
        let n = built.iter().filter(|(_, l)| l.to_lowercase() == name).count();
        assert_eq!(n, 1, "`{name}` is a folder row once: {:?}", labels(&rows(&mut app)));
    }
    let mut files: Vec<(usize, &str)> =
        app.tree_rows().iter().filter(|x| !x.folder).map(|x| (x.depth, x.label.as_str())).collect();
    files.sort_unstable();
    assert_eq!(files, [(3, "Algebra"), (3, "Fisica")]);
}

#[test]
fn sidebars_and_open_folders_survive_a_restart() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.left_w = 320.0;
    app.right_w = 412.0;
    app.right_open = true;
    app.run("toggle-folder:Facultad");
    app.run("toggle-folder:Facultad/Materias");
    app.run("toggle-folder:Facultad/Materias/1° Año");
    assert!(app.expanded.contains("Facultad/Materias"));
    app.save_all();

    let mut app = App::open(d.path()).unwrap();
    app.restore_session();
    assert_eq!((app.left_w, app.right_w), (320.0, 412.0), "sidebar widths are remembered");
    assert!(app.right_open);
    assert!(app.expanded.contains("Facultad/Materias"), "open folders are remembered");
    assert!(app.expanded.contains("Facultad"));
    let shown = rows(&mut app);
    let open = labels(&shown);
    assert!(open.contains(&(2, "1° Año")), "the remembered folder comes back open: {open:?}");
    assert!(open.contains(&(3, "Algebra")));
    // Everything the session did not name is folded.
    let years = app.tree_rows().iter().find(|r| r.label == "Años").expect("Años is listed");
    assert!(years.collapsed, "a folder the session did not open comes back folded");
}

#[test]
fn setting_off_always_starts_folded_and_saves_nothing() {
    let d = vault();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"rememberExpandedFolders": false}"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    assert!(!app.remember_expanded(), "the setting is off in app.json");

    app.run("toggle-folder:Facultad");
    assert!(app.expanded.contains("Facultad"));
    app.save_all();
    let saved = fs::read_to_string(d.path().join(".obsidian/metamind-session.json")).unwrap();
    assert!(!saved.contains("expanded"), "nothing is recorded while the setting is off: {saved}");

    let mut app = App::open(d.path()).unwrap();
    app.restore_session();
    assert!(app.expanded.is_empty());
    assert_eq!(labels(&rows(&mut app)), [(0, "Facultad")]);
}

#[test]
fn sidebar_widths_are_clamped_when_restored() {
    let d = vault();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    let session = r#"{"leftWidth": 9000, "rightWidth": 3}"#;
    fs::write(d.path().join(".obsidian/metamind-session.json"), session).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.restore_session();
    assert_eq!(app.left_w, metamind_app::shell::SIDE_W_MAX);
    assert_eq!(app.right_w, metamind_app::shell::SIDE_W_MIN);
}

#[test]
fn dragging_a_sidebar_edge_resizes_it() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = metamind_ui::Fonts::new();
    let mut pm = metamind_ui::Pixmap::new(1200, 700).unwrap();
    let (frame, x, y) = edge(&mut app, &mut fonts, &mut pm, "side-resize:left");
    app.click(&frame, &mut fonts, x, y, false);
    app.drag(&frame, &mut fonts, x + 90.0, y);
    app.release();
    assert_eq!(app.left_w, 340.0);
    // The panel is drawn at the width it was dragged to.
    let frame = metamind_app::render(&mut app, &mut fonts, &mut pm, 1.0);
    let split = frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "mod-left-split"))
        .expect("left sidebar");
    assert_eq!(split.rect.w, 340.0);

    // Past its limits the sidebar stops: the drag is clamped, not the pointer.
    let (frame, x, y) = edge(&mut app, &mut fonts, &mut pm, "side-resize:left");
    app.click(&frame, &mut fonts, x, y, false);
    app.drag(&frame, &mut fonts, x + 5_000.0, y);
    app.release();
    assert_eq!(app.left_w, metamind_app::shell::SIDE_W_MAX);

    // The right sidebar grows toward the left.
    app.right_open = true;
    let (frame, x, y) = edge(&mut app, &mut fonts, &mut pm, "side-resize:right");
    app.click(&frame, &mut fonts, x, y, false);
    app.drag(&frame, &mut fonts, x - 60.0, y);
    app.release();
    assert_eq!(app.right_w, 310.0);
}

/// The frame, and the middle of the 6px strip on a sidebar's inner edge.
fn edge(
    app: &mut App,
    fonts: &mut metamind_ui::Fonts,
    pm: &mut metamind_ui::Pixmap,
    action: &str,
) -> (metamind_app::Frame, f32, f32) {
    let frame = metamind_app::render(app, fonts, pm, 1.0);
    let r = frame
        .dom
        .nodes
        .iter()
        .find(|n| n.el.action.as_deref() == Some(action))
        .expect("the sidebar edge is drawn")
        .rect;
    (frame, r.x + r.w / 2.0, r.y + 40.0)
}
