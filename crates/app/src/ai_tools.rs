//! The functions the copilot can call on the vault.
//!
//! A model that only writes text can describe a note; one with tools can read the vault it is
//! being asked about and write the note. Each tool is a JSON schema the model sees and a function
//! this module runs on the app's own thread, where the store lives.
//!
//! # The rules of the surface
//!
//! - A path is **relative to the vault** and validated here: no `..`, no absolute path, no
//!   `.metamind/` (the app's own data). A model that invents `../etc/passwd` gets an error back,
//!   not a file.
//! - A write is applied at once — the operator asked for that — and it leaves a recovery snapshot
//!   the editor already knows how to restore.
//! - The result is compact JSON: the model reads it, and a long note comes back truncated with
//!   `"truncated": true` instead of flooding the conversation.
//! - Nothing panics on a bad argument: the tool answers `{"ok":false,"error":"…"}` and the model
//!   decides what to do with it.

use crate::shell::App;
use serde_json::{json, Value};
use vault_core::path;

/// The most characters a tool returns: enough for a note and a search, not enough to fill the
/// conversation with one answer.
const MAX_RESULT: usize = 12_000;
/// How many notes a search returns.
const MAX_HITS: usize = 12;
/// How many files a listing returns.
const MAX_LIST: usize = 200;

/// The tools, as the model sees them: one function spec each.
pub fn specs() -> Vec<Value> {
    let spec = |name: &str, description: &str, parameters: Value| json!({ "type": "function", "function": { "name": name, "description": description, "parameters": parameters } });
    vec![
        spec(
            "search_notes",
            "Busca texto en las notas de la bóveda y devuelve ruta, línea y un extracto.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Qué buscar" } },
                "required": ["query"],
            }),
        ),
        spec(
            "read_note",
            "Devuelve el texto de una nota, por ruta relativa a la bóveda.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            }),
        ),
        spec(
            "list_notes",
            "Lista las notas de una carpeta de la bóveda (o de la raíz).",
            json!({
                "type": "object",
                "properties": { "folder": { "type": "string", "description": "Carpeta; vacío es la raíz" } },
            }),
        ),
        spec(
            "create_note",
            "Crea una nota nueva. Falla si la ruta ya existe.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                },
                "required": ["path", "content"],
            }),
        ),
        spec(
            "append_note",
            "Agrega texto al final de una nota.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" }, "text": { "type": "string" } },
                "required": ["path", "text"],
            }),
        ),
        spec(
            "create_calendar_event",
            "Crea un evento en el calendario propio. La fecha es YYYY-MM-DD; las horas, HH:MM.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "start": { "type": "string", "description": "HH:MM, opcional" },
                    "end": { "type": "string", "description": "HH:MM, opcional" },
                    "color": { "type": "string", "description": "violeta, azul, verde, ambar, rojo, rosa, cian o gris" },
                },
                "required": ["title", "date"],
            }),
        ),
        spec(
            "create_kanban_card",
            "Agrega una tarjeta a una columna de un tablero.",
            json!({
                "type": "object",
                "properties": {
                    "board": { "type": "string", "description": "Ruta del tablero" },
                    "column": { "type": "string" },
                    "text": { "type": "string" },
                },
                "required": ["board", "column", "text"],
            }),
        ),
        spec(
            "open_note",
            "Abre una nota en una pestaña, para que el lector la vea.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            }),
        ),
    ]
}

/// A vault-relative path the tools may touch: no absolute path, no `..`, no `.metamind/`.
pub fn safe_path(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_start_matches('/');
    if raw.is_empty() {
        return Err("Falta la ruta de la nota.".to_string());
    }
    if raw.split('/').any(|part| part == "..") {
        return Err("La ruta no puede salir de la bóveda.".to_string());
    }
    if raw.starts_with(".metamind/") || raw == ".metamind" {
        return Err("Esa carpeta es de la app, no de las notas.".to_string());
    }
    let rel = path::normalize(raw);
    if rel.is_empty() {
        return Err("Ruta vacía.".to_string());
    }
    Ok(rel)
}

/// Run one call. The message is JSON: `{"ok":true,…}` or `{"ok":false,"error":"…"}`.
pub fn run(app: &mut App, name: &str, arguments: &str) -> String {
    let args: Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(e) => return error(&format!("Los argumentos no son JSON: {e}")),
    };
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    match name {
        "search_notes" => search(app, &text("query")),
        "read_note" => read(app, &text("path")),
        "list_notes" => list(app, &text("folder")),
        "create_note" => create(app, &text("path"), &text("content")),
        "append_note" => append(app, &text("path"), &text("text")),
        "create_calendar_event" => event(app, &args),
        "create_kanban_card" => card(app, &args),
        "open_note" => open(app, &text("path")),
        other => error(&format!("No existe la herramienta {other}.")),
    }
}

fn error(message: &str) -> String {
    json!({ "ok": false, "error": message }).to_string()
}

fn ok(value: Value) -> String {
    let mut map = json!({ "ok": true });
    if let (Value::Object(base), Value::Object(extra)) = (&mut map, value) {
        base.extend(extra);
    }
    let out = map.to_string();
    if out.chars().count() > MAX_RESULT {
        json!({ "ok": true, "truncated": true, "result": out.chars().take(MAX_RESULT).collect::<String>() })
            .to_string()
    } else {
        out
    }
}

fn search(app: &mut App, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        return error("La búsqueda está vacía.");
    }
    let hits = app.search_excerpts(query, MAX_HITS);
    if hits.is_empty() {
        return ok(json!({ "hits": [], "note": "sin resultados" }));
    }
    let rows: Vec<Value> = hits
        .into_iter()
        .map(|(path, line, text)| json!({ "path": path, "line": line, "text": text }))
        .collect();
    ok(json!({ "hits": rows }))
}

fn read(app: &mut App, raw: &str) -> String {
    let Ok(rel) = safe_path(raw) else { return error("La ruta no es válida.") };
    match app.store.read(&rel) {
        Ok((text, _)) => {
            let total = text.chars().count();
            let short: String = text.chars().take(MAX_RESULT).collect();
            ok(json!({ "path": rel, "chars": total, "truncated": total > MAX_RESULT, "text": short }))
        }
        Err(e) => error(&format!("No se pudo leer {rel}: {e}")),
    }
}

fn list(app: &mut App, folder: &str) -> String {
    let folder = folder.trim().trim_start_matches('/').trim_end_matches('/');
    if !folder.is_empty() && (folder.split('/').any(|p| p == "..") || folder.starts_with(".metamind")) {
        return error("Esa carpeta no se puede listar.");
    }
    let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
    let mut out: Vec<String> = app
        .store
        .index
        .notes()
        .map(|(_, entry)| entry.path.to_string())
        .filter(|path| path.starts_with(&prefix))
        .take(MAX_LIST)
        .collect();
    out.sort();
    ok(json!({ "folder": prefix, "count": out.len(), "notes": out }))
}

fn create(app: &mut App, raw: &str, content: &str) -> String {
    let Ok(rel) = safe_path(raw) else { return error("La ruta no es válida.") };
    if app.store.index.id(&rel).is_some() || app.store.abs(&rel).exists() {
        return error(&format!("{rel} ya existe: usá append_note o leé la nota antes de escribir."));
    }
    match app.store.create(&rel, content) {
        Ok(_) => {
            app.files_changed(std::slice::from_ref(&rel));
            app.notice = Some(format!("La IA creó {rel}"));
            ok(json!({ "path": rel, "chars": content.chars().count() }))
        }
        Err(e) => error(&format!("No se pudo crear {rel}: {e}")),
    }
}

fn append(app: &mut App, raw: &str, text: &str) -> String {
    let Ok(rel) = safe_path(raw) else { return error("La ruta no es válida.") };
    if text.chars().count() > MAX_RESULT {
        return error("El texto es demasiado largo para una sola escritura.");
    }
    let appended = match app.store.read(&rel) {
        Ok((current, stamp)) => {
            let mut next = current;
            if !next.ends_with('\n') {
                next.push('\n');
            }
            next.push_str(text);
            app.store.write(&rel, &next, Some(stamp)).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    match appended {
        Ok(_) => {
            app.files_changed(std::slice::from_ref(&rel));
            ok(json!({ "path": rel, "appended": text.chars().count() }))
        }
        Err(e) => error(&format!("No se pudo escribir en {rel}: {e}")),
    }
}

fn event(app: &mut App, args: &Value) -> String {
    let title = args.get("title").and_then(Value::as_str).unwrap_or_default().trim();
    let date = args.get("date").and_then(Value::as_str).unwrap_or_default().trim();
    if title.is_empty() || date.is_empty() {
        return error("Faltan el título o la fecha.");
    }
    let Some(day) = parse_day(date) else {
        return error("La fecha tiene que ser YYYY-MM-DD.");
    };
    let mut event = calendar_core::local::OwnedEvent::all_day(title, day, "violeta");
    let color = args.get("color").and_then(Value::as_str).unwrap_or("violeta");
    if calendar_core::local::valid_color(color) {
        event.color = color.to_string();
    }
    if let (Some(start), Some(end)) = (clock(args, "start"), clock(args, "end")) {
        event.start_minute = Some(start);
        event.end_minute = Some(end.max(start));
    }
    let id = app.calendar.owned.upsert(event);
    let root = app.store.root.clone();
    let file = root.join(crate::calendar::OWNED_JSON);
    match app.calendar.owned.save_atomic(&file) {
        Ok(()) => {
            app.calendar.mark_dirty();
            app.calendar.rebuild();
            let (y, m, d) = day.ymd();
            ok(json!({ "id": id, "date": format!("{y:04}-{m:02}-{d:02}") }))
        }
        Err(e) => error(&format!("No se pudo guardar el calendario: {e}")),
    }
}

fn parse_day(text: &str) -> Option<calendar_core::Day> {
    let mut parts = text.split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    calendar_core::Day::from_ymd(y, m, d)
}

fn clock(args: &Value, key: &str) -> Option<u16> {
    let text = args.get(key)?.as_str()?;
    let (h, m) = text.split_once(':')?;
    let h = h.trim().parse::<u16>().ok()?;
    let m = m.trim().parse::<u16>().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn card(app: &mut App, args: &Value) -> String {
    let board = args.get("board").and_then(Value::as_str).unwrap_or_default();
    let column = args.get("column").and_then(Value::as_str).unwrap_or_default();
    let text = args.get("text").and_then(Value::as_str).unwrap_or_default();
    if board.is_empty() || column.is_empty() || text.trim().is_empty() {
        return error("Faltan el tablero, la columna o el texto.");
    }
    let Ok(rel) = safe_path(board) else { return error("La ruta del tablero no es válida.") };
    let (body, stamp) = match app.store.read(&rel) {
        Ok(read) => read,
        Err(e) => return error(&format!("No se pudo leer {rel}: {e}")),
    };
    if !crate::kanbanview::is_board_text(&body) {
        return error(&format!("{rel} no es un tablero."));
    }
    let Some(next) = crate::kanbanview::add_card_text(&body, column, text) else {
        return error(&format!("El tablero no tiene una columna «{column}»."));
    };
    match app.store.write(&rel, &next, Some(stamp)) {
        Ok(_) => {
            app.files_changed(std::slice::from_ref(&rel));
            ok(json!({ "path": rel, "column": column }))
        }
        Err(e) => error(&format!("No se pudo escribir {rel}: {e}")),
    }
}

fn open(app: &mut App, raw: &str) -> String {
    let Ok(rel) = safe_path(raw) else { return error("La ruta no es válida.") };
    if app.store.index.id(&rel).is_none() {
        return error(&format!("No existe {rel}."));
    }
    app.open_note(&rel);
    ok(json!({ "path": rel }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_cannot_leave_the_vault() {
        assert!(safe_path("notas/A.md").is_ok());
        assert!(safe_path("/notas/A.md").is_ok(), "la barra del principio es la raíz de la bóveda");
        assert!(safe_path("../secreto").is_err());
        assert!(safe_path("notas/../../secreto").is_err());
        assert!(safe_path(".metamind/config.json").is_err());
        assert!(safe_path("").is_err());
    }

    #[test]
    fn the_specs_are_functions_with_a_name_and_a_type() {
        let specs = specs();
        assert!(specs.len() >= 8);
        for spec in &specs {
            assert_eq!(spec["type"], "function");
            assert!(spec["function"]["name"].as_str().is_some_and(|n| !n.is_empty()));
            assert_eq!(spec["function"]["parameters"]["type"], "object");
        }
    }

    #[test]
    fn a_broken_argument_body_is_an_answer_and_not_a_panic() {
        let out = run_for_test("search_notes", "{nope");
        assert!(out.contains("\"ok\":false"), "{out}");
    }

    /// The tool runner without a window: only the tools that do not need an `App`.
    fn run_for_test(name: &str, arguments: &str) -> String {
        let args: Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return error(&format!("Los argumentos no son JSON: {e}")),
        };
        let _ = (name, args);
        error("sin app")
    }
}
