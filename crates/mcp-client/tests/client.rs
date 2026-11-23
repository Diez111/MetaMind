//! The client against a real server: the mock that ships with this crate.

use mcp_client::{namespaced, Server, ServerSpec};
use serde_json::json;

/// The mock server this crate builds: `CARGO_BIN_EXE_mock-mcp` is the path of that binary.
fn mock() -> ServerSpec {
    ServerSpec::new("mock", env!("CARGO_BIN_EXE_mock-mcp"), &[])
}

#[test]
fn a_server_is_started_and_its_tools_are_listed() {
    let mut server = Server::start(&mock()).expect("el mock arranca");
    let tools = server.tools().expect("lista de herramientas");
    assert_eq!(tools.len(), 1, "{tools:?}");
    assert_eq!(tools[0].name, "eco");
    assert!(tools[0].description.contains("Devuelve"));
    assert_eq!(tools[0].schema["type"], "object");
}

#[test]
fn a_tool_is_called_and_its_text_comes_back() {
    let mut server = Server::start(&mock()).expect("el mock arranca");
    let answer = server.call("eco", json!({ "text": "hola" })).expect("la llamada");
    assert_eq!(answer["text"], "hola");
    // A tool that says it failed comes back as an error, not as an answer.
    let err = server.call("eco", json!({ "text": "fallar" }));
    assert!(err.is_err(), "la herramienta falló y el cliente lo dice");
}

#[test]
fn a_server_that_cannot_start_is_an_error_with_its_name() {
    let spec = ServerSpec::new("fantasma", "/no/existe/tal/binario", &[]);
    let err = Server::start(&spec).err().expect("no arranca");
    assert!(err.contains("fantasma"), "{err}");
}

#[test]
fn a_remote_tool_gets_a_name_that_cannot_collide() {
    assert_eq!(namespaced("filesystem", "read_file"), "mcp__filesystem__read_file");
    assert_eq!(namespaced("mi servidor", "abrir-archivo"), "mcp__mi_servidor__abrir_archivo");
}
