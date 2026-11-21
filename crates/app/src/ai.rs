//! In-app AI copilot: a chat panel in the right sidebar that talks to any OpenAI-compatible
//! `chat/completions` endpoint.
//!
//! The request runs on a worker thread and its server-sent events come back over a channel, so
//! the window keeps drawing while an answer streams in. The panel sends the markdown of the note
//! that is open as context (truncated, see [`MAX_CONTEXT_CHARS`]) and, for a note whose front
//! matter says `tipo: materia`, asks for a more pedagogical answer.

use crate::shell::{App, Key, Tab};
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;

/// Where requests go when `ai.base_url` is unset.
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
/// The model used when `ai.model` is unset: a cheap one, as the panel is for quick questions.
pub const DEFAULT_MODEL: &str = "deepseek-v4.1-flash";
/// The instructions the panel sends unless `ai.system_prompt` says otherwise: short, LaTeX for
/// mathematics, tables to compare and Mermaid for diagrams.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
Eres un tutor que ayuda a estudiar dentro de Obsidian.
Responde en español, breve: menos de 200 palabras, sin repetir la pregunta ni parlotear.
Empieza por la idea en una o dos frases y añade solo el desarrollo mínimo necesario.
Usa LaTeX con $...$ para las fórmulas en línea y $$...$$ para las que van destacadas.
Usa una tabla markdown cuando compares opciones, ventajas o pasos en paralelo.
Usa un bloque ```mermaid para los diagramas de flujo.
Si la nota no basta para responder, dilo y pide el dato que falta en vez de inventarlo.
Cuando uses la nota abierta, dilo: «en tu nota…».";
/// Added to the context of a note whose front matter says `tipo: materia`.
pub const MATERIA_HINT: &str = "\
La nota abierta es material de estudio: explica el porqué antes del resultado, da un ejemplo
mínimo y termina con una pregunta de comprobación.";
/// How much of the open note travels with each question, in characters. Long notes are cut: a
/// whole note would cost more tokens than the answer is worth.
pub const MAX_CONTEXT_CHARS: usize = 4000;
/// How much of the user's selection is quoted on its own.
const MAX_SELECTION_CHARS: usize = 1500;
/// Messages of the conversation sent to the model, the newest ones, on top of the system message.
const MAX_HISTORY: usize = 20;

/// Settings keys, written to `.obsidian/app.json` like every other setting.
pub const KEY_BASE_URL: &str = "ai.base_url";
pub const KEY_API_KEY: &str = "ai.api_key";
pub const KEY_MODEL: &str = "ai.model";
pub const KEY_SYSTEM_PROMPT: &str = "ai.system_prompt";
pub const KEY_TIMEOUT: &str = "ai.timeout";

/// What the panel needs to reach a model.
#[derive(Clone, Debug, PartialEq)]
pub struct AiConfig {
    /// Origin of the API, with or without its `/v1`. See [`chat_url`].
    pub base_url: String,
    /// Bearer token. Empty means the panel refuses to send.
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    /// Connect, response-header and per-read timeout of the request, in seconds.
    pub timeout_s: u64,
}

impl Default for AiConfig {
    fn default() -> AiConfig {
        AiConfig {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            model: DEFAULT_MODEL.to_string(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            timeout_s: 30,
        }
    }
}

impl AiConfig {
    /// Read the settings, falling back to the defaults for what is unset or blank.
    pub fn from_config(config: &vault_core::config::VaultConfig) -> AiConfig {
        let a = &config.app;
        let d = AiConfig::default();
        let text = |key: &str, default: &str| {
            a.str(key).map(str::trim).filter(|v| !v.is_empty()).unwrap_or(default).to_string()
        };
        AiConfig {
            base_url: text(KEY_BASE_URL, &d.base_url),
            api_key: a.str(KEY_API_KEY).map(str::trim).unwrap_or("").to_string(),
            model: text(KEY_MODEL, &d.model),
            system_prompt: text(KEY_SYSTEM_PROMPT, &d.system_prompt),
            timeout_s: a.get(KEY_TIMEOUT).and_then(Value::as_u64).map_or(d.timeout_s, |n| n.clamp(1, 600)),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "base_url": self.base_url,
            "api_key": self.api_key,
            "model": self.model,
            "system_prompt": self.system_prompt,
            "timeout_s": self.timeout_s,
        })
    }

    /// Read a serialized config (see [`AiConfig::to_json`]), with defaults for missing fields.
    pub fn from_json(v: &Value) -> AiConfig {
        let d = AiConfig::default();
        let text = |key: &str, default: &str| {
            v.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .to_string()
        };
        AiConfig {
            base_url: text("base_url", &d.base_url),
            api_key: v.get("api_key").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            model: text("model", &d.model),
            system_prompt: text("system_prompt", &d.system_prompt),
            timeout_s: v.get("timeout_s").and_then(Value::as_u64).map_or(d.timeout_s, |n| n.clamp(1, 600)),
        }
    }
}

/// The URL of the chat endpoint for `base`, which may already end in `/v1`.
pub fn chat_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let base = if base.is_empty() { DEFAULT_BASE_URL } else { base };
    if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

/// One turn of the conversation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    /// `user` or `assistant`.
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Message {
        Message { role: "user".to_string(), content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Message {
        Message { role: "assistant".to_string(), content: content.into() }
    }
}

/// What the worker thread reports while it reads the stream.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// A piece of the answer.
    Delta(String),
    /// The stream ended well.
    Done,
    /// The request failed; the text is shown under the conversation.
    Error(String),
}

/// The system message: the instructions plus the note context, when there is one.
pub fn system_content(cfg: &AiConfig, note_context: &str) -> String {
    let mut out = cfg.system_prompt.trim_end().to_string();
    if !note_context.trim().is_empty() {
        out.push_str("\n\n## Contexto de la bóveda\n\n");
        out.push_str(note_context.trim());
    }
    out
}

/// The OpenAI request body: system message, prior turns and the question.
pub fn build_payload(cfg: &AiConfig, system: &str, history: &[Message], user_input: &str) -> Value {
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(json!({ "role": "system", "content": system }));
    for m in history {
        if !m.content.trim().is_empty() {
            messages.push(json!({ "role": m.role, "content": m.content }));
        }
    }
    messages.push(json!({ "role": "user", "content": user_input }));
    json!({ "model": cfg.model, "messages": messages, "stream": true })
}

/// `s` cut to at most `max` characters, with an ellipsis when something was dropped.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The part of the open note, and of its selection, that travels as context. The front matter
/// says whether the note is course material, which switches the answer to a pedagogical one.
pub fn context_from_note(tab: &Tab) -> String {
    let text = tab.editor.buf.text();
    let mut ctx = String::new();
    if let Some(sel) = selection(tab, &text) {
        ctx.push_str("Texto que el usuario tiene seleccionado:\n\"\"\"\n");
        ctx.push_str(&sel);
        ctx.push_str("\n\"\"\"\n\n");
    }
    ctx.push_str("Nota abierta: ");
    ctx.push_str(vault_core::path::strip_md(&tab.path));
    ctx.push('\n');
    if tipo(tab).is_some_and(|t| t.eq_ignore_ascii_case("materia")) {
        ctx.push_str(MATERIA_HINT);
        ctx.push('\n');
    }
    ctx.push_str("---\n");
    ctx.push_str(&text);
    truncate_chars(&ctx, MAX_CONTEXT_CHARS)
}

/// The note's front matter `tipo`, when it has one.
fn tipo(tab: &Tab) -> Option<String> {
    let (_, meta) = tab.meta.as_ref()?;
    meta.property("tipo")?.as_str().map(str::to_string)
}

/// The selected text of the note, trimmed and cut to [`MAX_SELECTION_CHARS`].
fn selection(tab: &Tab, text: &str) -> Option<String> {
    let sel = tab.editor.buf.sels.first()?;
    let (a, b) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
    if a == b {
        return None;
    }
    let slice = text.get(a..b)?.trim();
    (!slice.is_empty()).then(|| truncate_chars(slice, MAX_SELECTION_CHARS))
}

/// A reply or a question broken into the parts the view draws differently.
#[derive(Debug, PartialEq)]
pub enum Chunk {
    Text(String),
    /// A `$$...$$` formula, drawn as a display block.
    Display(String),
}

/// Split `text` at its `$$...$$` formulas. A formula that is never closed stays text.
pub fn split_display_math(text: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with("$$") && !text[..i].ends_with('\\') {
            match text[i + 2..].find("$$") {
                Some(len) => {
                    if i > text_start {
                        out.push(Chunk::Text(text[text_start..i].to_string()));
                    }
                    out.push(Chunk::Display(text[i + 2..i + 2 + len].trim().to_string()));
                    i += 2 + len + 2;
                    text_start = i;
                    continue;
                }
                None => break,
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    if text_start < text.len() {
        out.push(Chunk::Text(text[text_start..].to_string()));
    }
    out
}

/// The conversation and the state of the panel.
#[derive(Default)]
pub struct AiPanel {
    pub messages: Vec<Message>,
    /// What the user has typed but not sent yet.
    pub input: String,
    /// The input has the keyboard: characters and Enter go to it.
    pub editing: bool,
    /// A worker thread is reading an answer.
    pub streaming: bool,
    /// The last failure, shown under the conversation.
    pub error: Option<String>,
    /// The view scrolls the conversation to the end on the next frame.
    pub scroll_to_end: bool,
    rx: Option<Receiver<Event>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AiPanel {
    /// Ask the model, streaming the answer into an empty assistant message. `note_context` is
    /// what [`context_from_note`] returned, or empty when no note is open.
    pub fn send(&mut self, cfg: &AiConfig, note_context: &str, user_input: &str) {
        let question = user_input.trim();
        if self.streaming || question.is_empty() {
            return;
        }
        self.error = None;
        if cfg.api_key.trim().is_empty() {
            self.error = Some("Falta la clave de API: Ajustes → IA.".to_string());
            return;
        }
        // The newest turns are the ones the model needs; the oldest fall off.
        let start = self.messages.len().saturating_sub(MAX_HISTORY);
        let mut history: Vec<Message> = self.messages[start..].to_vec();
        let payload =
            build_payload(cfg, &system_content(cfg, note_context), &history, question).to_string();
        history.push(Message::user(question));
        history.push(Message::assistant(String::new()));
        self.messages = history;
        self.input.clear();
        self.scroll_to_end = true;
        let cfg = cfg.clone();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.thread = Some(std::thread::spawn(move || stream_request(&cfg, &payload, &tx)));
        self.streaming = true;
    }

    /// Take everything the worker thread produced since the last frame. Returns whether the
    /// conversation changed and the window must redraw.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.rx.take() else { return false };
        let mut new = String::new();
        let mut done = false;
        loop {
            match rx.try_recv() {
                Ok(Event::Delta(d)) => new.push_str(&d),
                Ok(Event::Done) => {
                    done = true;
                    break;
                }
                Ok(Event::Error(e)) => {
                    self.error = Some(e);
                    done = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
            }
        }
        if !new.is_empty() {
            if let Some(last) = self.messages.last_mut().filter(|m| m.role == "assistant") {
                last.content.push_str(&new);
            }
            self.scroll_to_end = true;
        }
        if done {
            self.streaming = false;
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
            if self.messages.last().is_some_and(|m| m.role == "assistant" && m.content.is_empty()) {
                self.messages.pop();
            }
        } else {
            self.rx = Some(rx);
        }
        !new.is_empty() || done
    }

    /// Whether a request is still running.
    pub fn busy(&self) -> bool {
        self.streaming
    }

    /// Drop the conversation. A running request is abandoned: its next event finds no receiver.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.input.clear();
        self.error = None;
        self.streaming = false;
        self.scroll_to_end = true;
        self.rx = None;
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// POST `payload` to the endpoint of `cfg` and forward the streamed deltas to `tx`.
pub(crate) fn stream_request(cfg: &AiConfig, payload: &str, tx: &Sender<Event>) {
    let secs = cfg.timeout_s.clamp(1, 600);
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(secs)))
        .timeout_recv_response(Some(Duration::from_secs(secs)))
        .timeout_recv_body(Some(Duration::from_secs(secs)))
        // The status is inspected below: its body carries the explanation of a failure.
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let auth = format!("Bearer {}", cfg.api_key.trim());
    let sent = agent
        .post(&chat_url(&cfg.base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Authorization", auth.as_str())
        .send(payload);
    let mut res = match sent {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Event::Error(format!("No se pudo contactar la API: {e}")));
            return;
        }
    };
    let status = res.status();
    if !status.is_success() {
        let body = res.body_mut().with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
        let body = body.trim();
        let body = truncate_chars(body, 300);
        let _ = tx.send(Event::Error(format!("La API respondió {}: {body}", status.as_u16())));
        return;
    }
    let mut reader = std::io::BufReader::new(res.body_mut().as_reader());
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::BufRead::read_line(&mut reader, &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                let _ = tx.send(Event::Error(format!("Se cortó la respuesta: {e}")));
                return;
            }
        }
        let Some(data) = line.trim_end().strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            break;
        }
        match parse_delta(data) {
            Ok(Some(text)) => {
                if tx.send(Event::Delta(text)).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(e) => {
                let _ = tx.send(Event::Error(e));
                return;
            }
        }
    }
    let _ = tx.send(Event::Done);
}

/// The text of one `data:` line of an OpenAI stream. `Ok(None)` for events that carry no text
/// (a role announcement, or the final usage-only chunk); `Err` for a server-side error.
pub(crate) fn parse_delta(data: &str) -> Result<Option<String>, String> {
    let v: Value = serde_json::from_str(data).map_err(|e| format!("Respuesta inesperada de la API: {e}"))?;
    if let Some(msg) = v.get("error").and_then(|e| e.get("message")).and_then(Value::as_str) {
        return Err(format!("La API devolvió un error: {msg}"));
    }
    let choices = v.get("choices").and_then(Value::as_array);
    Ok(choices
        .and_then(|c| c.first())
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string))
}

impl App {
    /// Send what the panel's input holds, with the open note as context.
    pub(crate) fn ai_send(&mut self) {
        if self.ai.input.trim().is_empty() {
            return;
        }
        let cfg = AiConfig::from_config(&self.config);
        let context = self.tab().map(context_from_note).unwrap_or_default();
        let input = self.ai.input.clone();
        self.ai.send(&cfg, &context, &input);
        if let Some(e) = &self.ai.error {
            self.notice = Some(e.clone());
        }
    }

    /// Keys for the AI input. `None` lets the regular shortcuts see the key.
    pub(crate) fn ai_key(&mut self, key: &Key) -> Option<bool> {
        if !self.ai.editing {
            return None;
        }
        match key.key.as_str() {
            "Escape" => self.ai.editing = false,
            "Enter" if key.shift || key.alt => self.ai.input.push('\n'),
            "Enter" if !key.ctrl => self.ai_send(),
            "Backspace" if key.ctrl => self.ai.input.clear(),
            "Backspace" => {
                self.ai.input.pop();
            }
            _ if key.ctrl || key.alt => return None,
            _ => {}
        }
        Some(true)
    }

    /// Characters typed while the AI input has the keyboard.
    pub(crate) fn ai_text(&mut self, text: &str) -> bool {
        if !self.ai.editing || text.is_empty() {
            return false;
        }
        self.ai.input.push_str(text);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    fn tab(text: &str) -> Tab {
        Tab::new(
            "Nota.md".to_string(),
            text,
            vault_core::fsio::Stamp { mtime: 0, size: 0 },
            false,
        )
    }

    #[test]
    fn config_serializes_and_defaults() {
        let cfg = AiConfig {
            base_url: "http://localhost:8080/v1".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            system_prompt: "p".into(),
            timeout_s: 12,
        };
        assert_eq!(AiConfig::from_json(&cfg.to_json()), cfg);
        let empty = AiConfig::from_json(&json!({}));
        assert_eq!(empty, AiConfig::default());
        assert_eq!(AiConfig::from_json(&json!({ "model": "  " })).model, DEFAULT_MODEL);
        assert_eq!(AiConfig::from_json(&json!({ "timeout_s": 0 })).timeout_s, 1);
        assert_eq!(chat_url("https://api.deepseek.com/"), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(chat_url("http://127.0.0.1:1/v1"), "http://127.0.0.1:1/v1/chat/completions");
    }

    #[test]
    fn context_truncates_the_note() {
        let t = tab(&"a".repeat(5000));
        let ctx = context_from_note(&t);
        assert_eq!(ctx.chars().count(), MAX_CONTEXT_CHARS);
        assert!(ctx.ends_with('…'), "a cut context says so");
        assert!(ctx.starts_with("Nota abierta: Nota"));
        let short = context_from_note(&tab("hola"));
        assert!(short.ends_with("hola") && !short.contains('…'));
    }

    #[test]
    fn context_quotes_the_selection_and_flags_materia() {
        let mut t = tab("primera linea\nsegunda linea\n");
        t.editor.buf.set_sels(vec![editor_core::Sel::new(0, 13)]);
        t.meta = Some((0, vault_core::meta::extract("---\ntipo: materia\n---\ncuerpo\n")));
        let ctx = context_from_note(&t);
        assert!(ctx.contains("primera linea"), "{ctx}");
        assert!(ctx.contains("Texto que el usuario tiene seleccionado"));
        assert!(ctx.contains(MATERIA_HINT), "{ctx}");
        assert!(ctx.len() <= MAX_CONTEXT_CHARS * 4, "context stays small");
    }

    #[test]
    fn system_prompt_carries_the_context() {
        let cfg = AiConfig::default();
        let ctx = context_from_note(&tab("la nota"));
        let system = system_content(&cfg, &ctx);
        assert!(system.starts_with(&cfg.system_prompt));
        assert!(system.contains("la nota"), "{system}");
        assert!(system.contains("$...$") || system.contains("LaTeX"));
        assert_eq!(system_content(&cfg, "   "), cfg.system_prompt);
    }

    #[test]
    fn payload_is_openai_shaped() {
        let cfg = AiConfig::default();
        let history = vec![Message::user("antes"), Message::assistant("respuesta")];
        let p = build_payload(&cfg, "SYS", &history, "hola");
        assert_eq!(p["model"], cfg.model);
        assert_eq!(p["stream"], true);
        let m = p["messages"].as_array().unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[0]["content"], "SYS");
        assert_eq!(m[1]["role"], "user");
        assert_eq!(m[1]["content"], "antes");
        assert_eq!(m[2]["role"], "assistant");
        assert_eq!(m[2]["content"], "respuesta");
        assert_eq!(m[3]["role"], "user");
        assert_eq!(m[3]["content"], "hola");
    }

    #[test]
    fn sse_lines_are_parsed() {
        assert_eq!(
            parse_delta(r#"{"choices":[{"delta":{"content":"Hola"}}]}"#).unwrap(),
            Some("Hola".to_string())
        );
        assert_eq!(
            parse_delta(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#).unwrap(),
            None
        );
        assert_eq!(parse_delta(r#"{"choices":[]}"#).unwrap(), None);
        assert!(parse_delta(r#"{"error":{"message":"sin saldo"}}"#).unwrap_err().contains("sin saldo"));
        assert!(parse_delta("{").is_err());
    }

    #[test]
    fn display_math_is_split_out() {
        let parts = split_display_math("antes $$x^2$$ despues");
        assert_eq!(
            parts,
            vec![Chunk::Text("antes ".into()), Chunk::Display("x^2".into()), Chunk::Text(" despues".into())]
        );
        assert_eq!(split_display_math("sin formulas"), vec![Chunk::Text("sin formulas".into())]);
        assert_eq!(split_display_math("$$abierto"), vec![Chunk::Text("$$abierto".into())]);
    }

    /// A loopback server that answers one request with a canned SSE stream.
    fn mock_server(reply: &'static str) -> (String, std::thread::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                    break;
                }
                head.push_str(&l);
            }
            let len: usize = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(str::to_string))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = sock.write_all(reply.as_bytes());
            serde_json::from_slice(&body).unwrap()
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn streams_a_mock_openai_answer() {
        let (base, server) = mock_server(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\n\
             data: {\"choices\":[{\"delta\":{\"content\":\" mundo\"}}]}\n\n\
             data: [DONE]\n\n",
        );
        let cfg = AiConfig { base_url: base, api_key: "sk-test".into(), ..Default::default() };
        let payload = build_payload(&cfg, "SYS", &[], "hello").to_string();
        let (tx, rx) = channel();
        let worker = std::thread::spawn(move || stream_request(&cfg, &payload, &tx));
        let mut out = String::new();
        let mut done = false;
        for e in rx {
            match e {
                Event::Delta(d) => out.push_str(&d),
                Event::Done => done = true,
                Event::Error(e) => panic!("{e}"),
            }
        }
        worker.join().unwrap();
        let request = server.join().unwrap();
        assert_eq!(out, "Hola mundo");
        assert!(done, "the stream reports its end");
        assert_eq!(request["model"], DEFAULT_MODEL);
        assert_eq!(request["messages"][1]["content"], "hello");
    }

    #[test]
    fn send_requires_a_key_and_fills_the_panel() {
        let mut panel = AiPanel::default();
        panel.input = "hola".into();
        let cfg = AiConfig::default();
        panel.send(&cfg, "", "hola");
        assert!(panel.error.as_deref().unwrap().contains("clave"));
        assert!(panel.messages.is_empty());
        assert_eq!(panel.input, "hola", "the text is kept so it can be sent later");

        // A configured panel starts a request; the answer goes into the last message.
        let (base, _server) = mock_server("data: [DONE]\n\n");
        let cfg = AiConfig { base_url: base, api_key: "sk".into(), ..Default::default() };
        panel.send(&cfg, "contexto", "¿qué es una derivada?");
        assert!(panel.streaming);
        assert_eq!(panel.messages.len(), 2);
        assert_eq!(panel.messages[0].role, "user");
        assert_eq!(panel.messages[1].role, "assistant");
        for _ in 0..200 {
            if !panel.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            panel.poll();
        }
        assert!(!panel.busy());
        assert_eq!(panel.messages.len(), 1, "an empty answer is dropped");
        assert!(panel.error.is_none());
    }
}
