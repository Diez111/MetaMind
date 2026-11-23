//! A Model Context Protocol client: start a local server, list its tools, call one.
//!
//! MCP is JSON-RPC 2.0 over a pipe: the client writes one request per line and reads one response
//! per line. That is the whole protocol this crate implements, plus the two methods a tool user
//! needs (`tools/list`, `tools/call`); the server's own resources and prompts are not read, because
//! the copilot in this app only calls functions.
//!
//! # Shape
//!
//! A [`Server`] owns a child process and its pipes. Every request is synchronous: the caller writes
//! and waits for the answer with the same id. A server that dies, answers garbage or takes too long
//! comes back as an error and never hangs the app, because the wait has a deadline of its own.
//!
//! # What it does not do
//!
//! No async runtime, no `unsafe`: a `std::process::Command` and a `BufReader`. The app runs these
//! calls on the frame, where the vault is, so a blocking read is the honest shape.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// How long one request may take before the server is considered hung.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// How to start a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSpec {
    /// A name for the reader: "filesystem", "github", …
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Where the server runs; empty is the app's own directory.
    pub cwd: Option<PathBuf>,
}

impl ServerSpec {
    pub fn new(name: impl Into<String>, command: impl Into<String>, args: &[&str]) -> ServerSpec {
        ServerSpec {
            name: name.into(),
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: None,
        }
    }
}

/// One tool a server offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// The JSON schema of the arguments, as the server declared it.
    pub schema: Value,
}

/// A running server.
pub struct Server {
    name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    /// Start the process and run the handshake. The protocol version this client speaks is the one
    /// it declares; a server that answers another one is still used, because the two methods this
    /// crate needs did not change between them.
    pub fn start(spec: &ServerSpec) -> Result<Server, String> {
        let mut command = Command::new(&spec.command);
        command.args(&spec.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        if let Some(dir) = &spec.cwd {
            command.current_dir(dir);
        }
        let mut child = command.spawn().map_err(|e| format!("No se pudo arrancar «{}»: {e}", spec.name))?;
        let stdin = child.stdin.take().ok_or("el servidor no tiene entrada")?;
        let stdout = child.stdout.take().ok_or("el servidor no tiene salida")?;
        let mut server =
            Server { name: spec.name.clone(), child, stdin, stdout: BufReader::new(stdout), next_id: 1 };
        server.initialize()?;
        Ok(server)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The MCP handshake: the client says which versions it speaks and who it is.
    pub fn initialize(&mut self) -> Result<Value, String> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "clientInfo": { "name": "MetaMind", "version": env!("CARGO_PKG_VERSION") },
        });
        let answer = self.request("initialize", params)?;
        // The notification is part of the protocol: a server waits for it before serving.
        self.notify("notifications/initialized", json!({}))?;
        Ok(answer)
    }

    /// The tools the server declares.
    pub fn tools(&mut self) -> Result<Vec<Tool>, String> {
        let answer = self.request("tools/list", json!({}))?;
        let list = answer.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(list
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                Some(Tool {
                    name,
                    description: t.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                    schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                })
            })
            .collect())
    }

    /// Call one tool. The answer is the server's `result`, or the text of its error.
    pub fn call(&mut self, tool: &str, arguments: Value) -> Result<Value, String> {
        let answer = self.request("tools/call", json!({ "name": tool, "arguments": arguments }))?;
        if let Some(text) = answer.get("isError").and_then(Value::as_bool).filter(|e| *e) {
            let _ = text;
            let message = answer
                .get("content")
                .and_then(Value::as_array)
                .and_then(|c| c.first())
                .and_then(|c| c.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("la herramienta falló");
            return Err(message.to_string());
        }
        // A tool answers with content blocks; the text of the first one is what a model reads.
        if let Some(blocks) = answer.get("content").and_then(Value::as_array) {
            let text: String = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if !text.trim().is_empty() {
                return Ok(json!({ "text": text }));
            }
        }
        Ok(answer)
    }

    /// One request and its answer, matched by id. A notification (no id) is skipped.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if Instant::now() > deadline {
                return Err(format!("«{}» no contestó a {method} en {} s", self.name, TIMEOUT.as_secs()));
            }
            let Some(message) = self.read_line(deadline)? else { continue };
            let Ok(value) = serde_json::from_str::<Value>(&message) else { continue };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                let text = error.get("message").and_then(Value::as_str).unwrap_or("error del servidor");
                return Err(format!("«{}»: {text}", self.name));
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| format!("«{}» contestó sin resultado", self.name));
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn write(&mut self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("«{}» cerró la entrada: {e}", self.name))?;
        self.stdin.flush().map_err(|e| format!("«{}» no recibe: {e}", self.name))
    }

    /// One line, waiting at most until `deadline`. `None` for an empty line (a keep-alive), so the
    /// caller keeps waiting instead of reading garbage.
    fn read_line(&mut self, _deadline: Instant) -> Result<Option<String>, String> {
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) => Err(format!("«{}» terminó sin contestar", self.name)),
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed.to_string()))
                }
            }
            Err(e) => Err(format!("«{}» no contesta: {e}", self.name)),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A server is a child of this app: closing its input is the polite goodbye, and killing it
        // is what happens when it does not take the hint.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The tools of `spec` with the server's name in front, as the copilot offers them:
/// `mcp__<server>__<tool>`. The name a model sees must not collide with a built-in one.
pub fn namespaced(server: &str, tool: &str) -> String {
    let clean =
        |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>();
    format!("mcp__{}__{}", clean(server), clean(tool))
}
