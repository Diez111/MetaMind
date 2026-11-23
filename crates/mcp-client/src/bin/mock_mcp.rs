//! A Model Context Protocol server for the client's tests: one tool, `eco`, that returns its text.
//!
//! It is a real server, not a stub: it speaks JSON-RPC on stdin/stdout, answers the handshake, the
//! notification that follows it, `tools/list`, and `tools/call`. Writing it in Rust keeps the tests
//! free of a Python or Node dependency, and `env!("CARGO_BIN_EXE_mock-mcp")` gives the test the
//! path of this very binary.

use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        // A notification has no id and takes no answer.
        let Some(id) = message.get("id").cloned() else { continue };
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mock", "version": "0.0.0" },
            }),
            "tools/list" => json!({
                "tools": [{
                    "name": "eco",
                    "description": "Devuelve el texto que recibe.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "text": { "type": "string" } },
                        "required": ["text"],
                    },
                }],
            }),
            "tools/call" => {
                let args =
                    message.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or_default();
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                if text == "fallar" {
                    json!({
                        "isError": true,
                        "content": [{ "type": "text", "text": "el pedido falló" }],
                    })
                } else {
                    json!({ "content": [{ "type": "text", "text": text }] })
                }
            }
            other => {
                let answer = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("método desconocido: {other}") },
                });
                let _ = writeln!(stdout, "{answer}");
                let _ = stdout.flush();
                continue;
            }
        };
        let answer = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        if writeln!(stdout, "{answer}").is_err() || stdout.flush().is_err() {
            return;
        }
    }
}
