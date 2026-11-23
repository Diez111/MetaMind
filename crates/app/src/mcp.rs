//! The MCP servers this app is allowed to use, and their tools.
//!
//! The servers are declared in `.obsidian/app.json` under `mcp.servers`:
//!
//! ```json
//! {"mcp": {"servers": [{"name": "filesystem", "command": "npx",
//!   "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/me"]}]}}
//! ```
//!
//! Nothing is started until the copilot needs a tool: the first turn that asks for the list starts
//! every enabled server, reads its `tools/list` and keeps the process for the rest of the session.
//! A server that fails to start is reported once and skipped, so a broken entry in the config does
//! not take the copilot down with it.
//!
//! A remote tool is offered as `mcp__<server>__<tool>`, which cannot collide with a built-in name.

use mcp_client::{namespaced, Server, ServerSpec};
use serde_json::{json, Value};

/// The MCP servers of this session.
#[derive(Default)]
pub struct Mcp {
    servers: Vec<(String, Server)>,
    /// `(namespaced name, server index, tool name, schema)`.
    tools: Vec<(String, usize, String, Value)>,
    /// Why a server could not be started, one line per server.
    pub errors: Vec<String>,
    /// The config was already read: a vault with no servers is asked once.
    started: bool,
}

impl Mcp {
    /// Start every server the config declares, the first time this is called. Idempotent.
    pub fn ensure(&mut self, config: &vault_core::config::JsonFile) {
        if self.started {
            return;
        }
        self.started = true;
        let Some(list) = config.get("mcp").and_then(|m| m.get("servers")).and_then(Value::as_array) else {
            return;
        };
        for entry in list {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("mcp").to_string();
            let command = entry.get("command").and_then(Value::as_str).unwrap_or("");
            if command.trim().is_empty() {
                self.errors.push(format!("«{name}» no tiene comando"));
                continue;
            }
            let args: Vec<String> = entry
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let spec = ServerSpec { name: name.clone(), command: command.to_string(), args, cwd: None };
            match Server::start(&spec) {
                Ok(mut server) => match server.tools() {
                    Ok(tools) => {
                        let index = self.servers.len();
                        for tool in tools {
                            let full = namespaced(&name, &tool.name);
                            self.tools.push((full, index, tool.name, tool.schema));
                        }
                        self.servers.push((name, server));
                    }
                    Err(e) => self.errors.push(e),
                },
                Err(e) => self.errors.push(e),
            }
        }
    }

    /// The tools, in the shape the chat payload wants.
    pub fn specs(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|(name, _, _, schema)| {
                json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": format!("Herramienta del servidor MCP ({name})."),
                        "parameters": schema,
                    }
                })
            })
            .collect()
    }

    /// True when `name` is one of the remote tools.
    pub fn owns(&self, name: &str) -> bool {
        name.starts_with("mcp__")
    }

    /// Call a remote tool. `None` when the name is not one of them.
    pub fn call(&mut self, name: &str, arguments: &str) -> Option<Result<Value, String>> {
        let (_, index, tool, _) = self.tools.iter().find(|(full, ..)| full == name)?;
        let (index, tool) = (*index, tool.clone());
        let args: Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return Some(Err(format!("Los argumentos no son JSON: {e}"))),
        };
        let Some((_, server)) = self.servers.get_mut(index) else {
            return Some(Err(format!("El servidor de «{name}» ya no está")));
        };
        Some(server.call(&tool, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mock server of `mcp-client`, next to the test binary. A missing binary is a failure,
    /// not a reason to skip: a test that passes because it did nothing is worse than no test.
    fn mock_path() -> std::path::PathBuf {
        let exe = std::env::current_exe().expect("the test binary");
        let path = exe.parent().and_then(|d| d.parent()).expect("the target folder").join("mock-mcp");
        assert!(path.exists(), "falta el servidor de prueba: {}", path.display());
        path
    }

    #[test]
    fn a_server_in_the_config_is_started_once_and_its_tools_are_namespaced() {
        let path = mock_path();
        let d = tempfile::tempdir().unwrap();
        let config = vault_core::config::JsonFile::load(
            &d.path().join("app.json"),
            serde_json::json!({"mcp": {"servers": [{"name": "mock", "command": path, "args": []}]}}),
        )
        .unwrap();
        let mut mcp = Mcp::default();
        mcp.ensure(&config);
        assert!(mcp.errors.is_empty(), "{:?}", mcp.errors);
        let specs = mcp.specs();
        assert_eq!(specs.len(), 1, "{specs:?}");
        assert_eq!(specs[0]["function"]["name"], "mcp__mock__eco");
        assert!(mcp.owns("mcp__mock__eco"));
        let answer =
            mcp.call("mcp__mock__eco", "{\"text\":\"hola\"}").expect("la herramienta").expect("la llamada");
        assert_eq!(answer["text"], "hola");
        // A second ensure does not start the server twice.
        mcp.ensure(&config);
        assert_eq!(mcp.specs().len(), 1);
    }

    #[test]
    fn a_server_that_cannot_start_is_reported_and_the_rest_keeps_working() {
        let d = tempfile::tempdir().unwrap();
        let config = vault_core::config::JsonFile::load(
            &d.path().join("app.json"),
            serde_json::json!({"mcp": {"servers": [{"name": "roto", "command": "/no/existe"}]}}),
        )
        .unwrap();
        let mut mcp = Mcp::default();
        mcp.ensure(&config);
        assert_eq!(mcp.errors.len(), 1, "{:?}", mcp.errors);
        assert!(mcp.errors[0].contains("roto"), "{:?}", mcp.errors);
        assert!(mcp.specs().is_empty());
    }
}
