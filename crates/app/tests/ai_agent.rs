//! The copilot with tools: the model asks for a function, the app runs it on the vault, and the
//! answer that comes back is built on what the function found.
//!
//! The server is a local socket that answers two requests in order: one that asks for a tool and
//! one with the answer. Everything else is the real app: the real store, the real config, the real
//! loop that runs on the frame.

use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Pixmap};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

/// A server that answers each request with the next reply of the list, and hands back the bodies
/// it received so a test can check what was asked.
fn server(replies: Vec<&'static str>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut bodies = Vec::new();
        for reply in replies {
            let Ok((mut sock, _)) = listener.accept() else { break };
            sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let len: usize = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(str::to_string))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            bodies.push(serde_json::from_slice(&body).unwrap_or(Value::Null));
            let out = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = sock.write_all(out.as_bytes());
        }
        bodies
    });
    (format!("http://{addr}"), handle)
}

/// An app with the copilot pointed at `base` and one frame rendered, which is what lets the tool
/// loop run (it lives on the frame, where the store is).
fn app_with_ai(dir: &tempfile::TempDir, base: &str) -> App {
    std::fs::create_dir_all(dir.path().join(".obsidian")).unwrap();
    let config = format!(
        r#"{{"ai.base_url":"{base}","ai.model":"test","ai.api_key":"sk-test"}}"#
    );
    std::fs::write(dir.path().join(".obsidian/app.json"), config).unwrap();
    let mut app = App::open(dir.path()).unwrap();
    app.run("ai:open");
    app
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1100, 760).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

#[test]
fn the_model_asks_for_a_note_and_the_app_writes_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Home.md"), "# Home\n\nUna nota.\n").unwrap();
    let (base, server) = server(vec![
        // First answer: a call, and no text.
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"create_note\",\"arguments\":\"{\\\"path\\\":\\\"Resumen.md\\\",\\\"content\\\":\\\"# Resumen\\\\n\\\\nLo que salió.\\\\n\\\"}\"}}]}}]}\n\ndata: [DONE]\n\n",
        // Second answer: the text the reader sees.
        "data: {\"choices\":[{\"delta\":{\"content\":\"Listo, \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"creé Resumen.\"}}]}\n\ndata: [DONE]\n\n",
    ]);
    let mut app = app_with_ai(&dir, &base);
    let mut fonts = Fonts::new();
    app.ai.input = "Armá una nota con el resumen".into();
    app.run("ai:send");
    // Frames until the turn settles: the stream, then the tool, then the second stream.
    for _ in 0..40 {
        frame(&mut app, &mut fonts);
        if !app.ai.streaming && app.ai.tool_calls.is_empty() && app.ai.messages.len() >= 4 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let made = dir.path().join("Resumen.md");
    assert!(made.exists(), "la herramienta escribió la nota");
    let text = std::fs::read_to_string(made).unwrap();
    assert!(text.starts_with("# Resumen"), "{text}");
    assert_eq!(app.ai.tool_log.len(), 1, "una herramienta, una fila: {:?}", app.ai.tool_log);
    assert_eq!(app.ai.tool_log[0].0, "create_note");
    assert!(app.ai.tool_log[0].2, "y salió bien: {:?}", app.ai.tool_log[0]);
    let answer: String = app.ai.messages.iter().filter(|m| m.role == "assistant").map(|m| m.content.clone()).collect();
    assert!(answer.contains("creé Resumen"), "la respuesta final está en el panel: {answer:?}");
    // What the second request carried: the assistant turn with its call, and the tool's answer.
    let bodies = server.join().unwrap();
    assert_eq!(bodies.len(), 2, "dos pedidos: el que pide y el que sigue");
    let messages = &bodies[1]["messages"];
    assert!(
        messages.as_array().is_some_and(|m| m.iter().any(|x| x["role"] == "tool")),
        "el segundo pedido lleva el resultado de la herramienta: {messages}"
    );
    assert!(
        messages.as_array().is_some_and(|m| m.iter().any(|x| x["tool_calls"].is_array())),
        "y el turno que la pidió: {messages}"
    );
    assert!(!bodies[0]["tools"].as_array().is_none_or(Vec::is_empty), "el primer pedido ofrece las herramientas");
}

#[test]
fn a_tool_that_cannot_run_answers_an_error_and_the_turn_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Home.md"), "# Home\n").unwrap();
    let (base, server) = server(vec![
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read_note\",\"arguments\":\"{\\\"path\\\":\\\"../fuera.md\\\"}\"}}]}}]}\n\ndata: [DONE]\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"No pude leerlo.\"}}]}\n\ndata: [DONE]\n\n",
    ]);
    let mut app = app_with_ai(&dir, &base);
    let mut fonts = Fonts::new();
    app.ai.input = "Leé ../fuera.md".into();
    app.run("ai:send");
    for _ in 0..40 {
        frame(&mut app, &mut fonts);
        if !app.ai.streaming && app.ai.tool_calls.is_empty() && !app.ai.tool_log.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.ai.tool_log.len(), 1);
    assert!(!app.ai.tool_log[0].2, "la ruta inválida no se lee: {:?}", app.ai.tool_log[0]);
    let bodies = server.join().unwrap();
    let tool = bodies[1]["messages"]
        .as_array()
        .and_then(|m| m.iter().find(|x| x["role"] == "tool"))
        .cloned()
        .expect("el resultado viaja al modelo");
    assert!(tool["content"].as_str().is_some_and(|c| c.contains("no es válida")), "{tool}");
}

#[test]
fn the_tool_log_is_part_of_the_answer_the_reader_sees() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Home.md"), "# Home\n").unwrap();
    // A turn with no tool: the panel does not grow a tool line.
    let (base, server) = server(vec!["data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\ndata: [DONE]\n\n"]);
    let mut app = app_with_ai(&dir, &base);
    let mut fonts = Fonts::new();
    app.ai.input = "hola".into();
    app.run("ai:send");
    for _ in 0..30 {
        frame(&mut app, &mut fonts);
        if !app.ai.streaming {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(app.ai.tool_log.is_empty(), "sin herramientas no hay filas: {:?}", app.ai.tool_log);
    let _ = server.join().unwrap();
}
