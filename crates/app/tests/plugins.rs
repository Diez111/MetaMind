use ext_api::{Api, Event, Extension, Permissions, PluginInfo};
use metamind_app::App;
use serde_json::json;
use std::fs;

struct Demo(PluginInfo);

impl Demo {
    fn call(&self, api: &mut Api, method: &str, params: serde_json::Value) -> serde_json::Value {
        serde_json::from_str(&api.call(&self.0, &json!({"method": method, "params": params}).to_string()))
            .unwrap()
    }
}

impl Extension for Demo {
    fn load(&mut self, api: &mut Api) -> Result<(), String> {
        self.call(
            api,
            "commands.register",
            json!({"id": "tag", "name": "Append tag", "hotkey": "Ctrl+Alt+T"}),
        );
        self.call(api, "commands.register", json!({"id": "mk", "name": "Make note"}));
        self.call(api, "commands.register", json!({"id": "evil", "name": "Escape"}));
        self.call(api, "events.subscribe", json!({"event": "file-open"}));
        self.call(api, "ui.status", json!({"id": "s", "text": "demo ready"}));
        self.call(api, "ui.ribbon", json!({"id": "r", "icon": "D", "title": "Demo", "command": "tag"}));
        Ok(())
    }
    fn event(&mut self, e: &Event, api: &mut Api) -> Result<(), String> {
        if let Event::FileOpen { path } = e {
            self.call(api, "ui.status", json!({"id": "s", "text": format!("opened {path}")}));
        }
        Ok(())
    }
    fn command(&mut self, id: &str, api: &mut Api) -> Result<(), String> {
        match id {
            "tag" => {
                let t = self.call(api, "editor.text", json!({}))["ok"].as_str().unwrap().len();
                self.call(api, "editor.replace", json!({"from": t, "to": t, "text": " #fromplugin"}));
            }
            "mk" => {
                self.call(api, "vault.write", json!({"path": "Made.md", "text": "[[Home]] #made"}));
            }
            _ => {
                let r = self.call(api, "vault.write", json!({"path": "../evil.md", "text": "x"}));
                assert!(r.get("error").is_some());
            }
        }
        Ok(())
    }
}

fn setup() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), "# Home\n").unwrap();
    fs::create_dir_all(d.path().join(".obsidian/plugins/wasm-x")).unwrap();
    fs::write(
        d.path().join(".obsidian/plugins/wasm-x/manifest.json"),
        r#"{"id":"wasm-x","name":"X","version":"1"}"#,
    )
    .unwrap();
    fs::write(d.path().join(".obsidian/plugins/wasm-x/main.wasm"), b"junk").unwrap();
    fs::write(d.path().join(".obsidian/community-plugins.json"), r#"["wasm-x"]"#).unwrap();
    let app = App::open(d.path()).unwrap();
    (d, app)
}

#[test]
fn plugin_commands_editor_vault_events_and_ui() {
    let (d, mut app) = setup();
    let info = PluginInfo {
        id: "demo".into(),
        name: "Demo".into(),
        perms: Permissions::all(),
        data_path: ".obsidian/plugins/demo/data.json".into(),
    };
    app.add_extension(info.clone(), Box::new(Demo(info))).unwrap();
    app.open_note("Home");
    app.run("demo:tag");
    assert!(app.tab().unwrap().editor.buf.text().ends_with(" #fromplugin"));
    app.run("demo:mk");
    assert_eq!(fs::read_to_string(d.path().join("Made.md")).unwrap(), "[[Home]] #made");
    assert!(app.store.index.id("Made.md").is_some());
    app.run("demo:evil");
    assert!(!d.path().parent().unwrap().join("evil.md").exists());
    app.pump_plugins();
    assert!(app.plugins.ext.reg.status.iter().any(|s| s.text == "opened Home.md"));
    assert!(app.plugin_palette().iter().any(|(id, _)| id == "demo:tag"));
    assert!(app.keymap.iter().any(|(_, id)| id == "demo:tag"));
}

#[test]
fn plugin_list_and_reload_report_unloadable_plugins() {
    let (_d, mut app) = setup();
    let list = app.plugin_list();
    assert_eq!(list.len(), 1);
    assert!(list[0].enabled && !list[0].loaded);
    app.run("plugins:reload");
    let list = app.plugin_list();
    assert!(list[0].error.is_some(), "junk wasm must report an error, not crash");
    app.run("plugins:list");
    assert!(app.notice.as_deref().unwrap().contains("wasm-x"));
    app.set_plugin_enabled("wasm-x", false).unwrap();
    app.run("plugins:reload");
    assert!(!app.plugin_list()[0].enabled);
}

#[cfg(feature = "scripting")]
#[test]
fn user_script_registers_a_command_and_edits_a_note() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), "# Home\n").unwrap();
    fs::create_dir_all(d.path().join(".obsidian/scripts")).unwrap();
    fs::write(
        d.path().join(".obsidian/scripts/log.rhai"),
        r#"register_command("add", "Add line", "add"); fn add() { write("Home.md", read("Home.md") + "added\n"); notice("done"); }"#,
    )
    .unwrap();
    fs::write(d.path().join(".obsidian/community-plugins.json"), r#"["script-log"]"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("plugins:reload");
    assert!(app.plugin_list()[0].loaded, "{:?}", app.plugin_list());
    app.run("script-log:add");
    app.pump_plugins();
    assert_eq!(fs::read_to_string(d.path().join("Home.md")).unwrap(), "# Home\nadded\n");
    assert_eq!(app.notice.as_deref(), Some("done"));
}

#[cfg(feature = "wasm-plugins")]
#[test]
fn example_wasm_plugin_loads_and_runs_in_the_app() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), "# Home\n").unwrap();
    let p = d.path().join(".obsidian/plugins/hello-wasm");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("manifest.json"), include_str!("../../../examples/plugins/hello-wasm/manifest.json"))
        .unwrap();
    fs::write(p.join("main.wasm"), include_bytes!("../../../examples/plugins/hello-wasm/main.wasm")).unwrap();
    fs::write(d.path().join(".obsidian/community-plugins.json"), r#"["hello-wasm"]"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    app.run("plugins:reload");
    assert!(app.plugin_list()[0].loaded, "{:?}", app.plugin_list());
    app.run("hello-wasm:hello");
    app.pump_plugins();
    assert_eq!(app.notice.as_deref(), Some("Hello from a WASM plugin"));
    app.run("hello-wasm:stamp");
    assert!(app.tab().unwrap().editor.buf.text().starts_with("stamped\n"));
    assert!(app.plugins.ext.reg.status.iter().any(|s| s.text.starts_with("hello-wasm")));
}

/// An Obsidian plugin of the JS kind has no native port yet. That is not an error of the plugin
/// nor of the app, so the launch notice says it once, in one line for all of them, with the
/// reassuring wording the owner asked for — not the technical sentence it used to print, and not
/// one toast per plugin.
#[test]
fn js_plugins_without_a_native_port_are_announced_once_and_as_one_line() {
    metamind_app::plugins::reset_no_port_notice();
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), "# Home\n").unwrap();
    for id in ["calendar-extra", "kanban-board"] {
        let p = d.path().join(".obsidian/plugins").join(id);
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("manifest.json"),
            format!(r#"{{"id":"{id}","name":"{id}","version":"1.2.3","minAppVersion":"0.15.0"}}"#),
        )
        .unwrap();
        fs::write(p.join("main.js"), "module.exports = {};").unwrap();
    }
    fs::write(d.path().join(".obsidian/community-plugins.json"), r#"["calendar-extra","kanban-board"]"#)
        .unwrap();
    let mut app = App::open(d.path()).unwrap();
    // The window loads the plugins itself after the first frame; `plugins:reload` would replace
    // the launch notice with its own "Plugins recargados" line.
    app.plugins_load();
    let list = app.plugin_list();
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|p| p.enabled), "{list:?}");
    assert!(list.iter().all(|p| p.error.as_deref() == Some(metamind_app::plugins::NO_PORT)), "{list:?}");
    // One line for both, counting them, and the toast knows it informs instead of warning.
    let notice = app.notice.clone().expect("the launch notice is shown");
    assert_eq!(notice, "2 plugins de Obsidian sin port nativo aún; los necesarios llegarán con el tiempo.");
    assert_eq!(app.plugins.no_port_notice.as_deref(), Some(notice.as_str()));
    assert!(!notice.contains("compilación"), "the old technical wording is gone: {notice}");
    // Once per session: a reload (or another vault) does not repeat it.
    app.notice = None;
    app.plugins_load();
    assert_eq!(app.notice, None, "the notice is not repeated after the first time");
}
