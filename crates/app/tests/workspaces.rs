use metamind_app::shell::PromptKind;
use metamind_app::App;
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("A.md"), "alpha\n").unwrap();
    fs::write(d.path().join("B.md"), "beta\n").unwrap();
    d
}

fn labels(app: &App) -> Vec<String> {
    app.prompt.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect()
}

#[test]
fn a_saved_workspace_brings_back_its_panes_and_tabs() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A.md");
    app.run("workspace:split-vertical");
    app.open_note("B.md");
    app.run("workspace:toggle-stacked-tabs");
    app.run("workspaces:save");
    assert_eq!(app.prompt.as_ref().map(|p| p.kind), Some(PromptKind::Workspace));
    app.text_input("Writing");
    assert_eq!(labels(&app), ["Guardar como \u{201c}Writing\u{201d}"]);
    app.key(metamind_app::Key::new("Enter"));
    assert_eq!(app.workspace_names().unwrap(), ["Writing"]);

    app.run("workspace:close-all");
    app.focus_pane(0);
    app.run("workspace:close-all");
    app.load_workspace("Writing");
    assert_eq!(app.notice.as_deref(), Some("Espacio de trabajo \u{201c}Writing\u{201d} cargado"));
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.pane, 1);
    assert!(app.panes[1].stacked);
    let paths = |p: usize| app.pane_tabs(p).0.iter().map(|t| t.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(0), ["A.md"]);
    assert_eq!(paths(1), ["A.md", "B.md"]);

    app.run("workspaces:load");
    assert_eq!(labels(&app), ["Writing"]);
    app.run("workspaces:delete:Writing");
    assert!(app.workspace_names().unwrap().is_empty());
    app.run("workspaces:load");
    assert_eq!(labels(&app), ["No hay espacios de trabajo guardados"]);
}

#[test]
fn a_broken_workspace_file_is_reported_not_ignored() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    fs::create_dir_all(&app.config.dir).unwrap();
    fs::write(app.config.dir.join("metamind-workspaces.json"), "{oops").unwrap();
    app.run("workspaces:load");
    assert!(labels(&app)[0].starts_with("No se pudieron leer los espacios de trabajo"), "{:?}", labels(&app));
    app.save_workspace("x");
    assert!(app.notice.as_deref().unwrap().starts_with("No se pudieron leer los espacios de trabajo"));
}
