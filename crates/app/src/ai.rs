//! In-app AI copilot: a chat panel in the right sidebar that talks to any OpenAI-compatible
//! `chat/completions` endpoint.
//!
//! The request runs on a worker thread and its server-sent events come back over a channel, so
//! the window keeps drawing while an answer streams in. The panel sends the markdown of the note
//! that is open as context (truncated, see [`MAX_CONTEXT_CHARS`]) and, for a note whose front
//! matter says `tipo: materia`, asks for a more pedagogical answer.
//!
//! The user may configure as many providers as they want: `ai.profiles` holds one
//! [`AiProfile`] per provider and `ai.active` names the one the panel sends to. A vault written
//! by an older build only has a single endpoint (`ai.base_url`, `ai.model`, …); those keys are
//! folded into the "Predeterminado" profile the first time they are read.

use crate::shell::{App, Key, RightPanel, Tab};
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;
use vault_core::config::JsonFile;

/// Where requests go when a profile leaves `base_url` unset.
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
/// The model used by the default profile and the DeepSeek preset: a cheap one, as the panel is
/// for quick questions.
pub const DEFAULT_MODEL: &str = "deepseek-v4.1-flash";
/// The instructions the panel sends when a profile has none, and the starting points the settings
/// form offers. Each preset writes a different voice, not a different model.
pub const PROMPT_PRESETS: [(&str, &str); 5] = [
    (
        "Estudio (predeterminado)",
        "Un tutor que explica el porqué antes del resultado, con un ejemplo mínimo y una pregunta \
         de comprobación al final.",
    ),
    (
        "Matemática paso a paso",
        "Desarrolla cada paso, nombra la regla que aplica y cierra con el resultado destacado.",
    ),
    (
        "Diagramas y tablas",
        "Prefiere mostrar antes que contar: tabla para comparar, Mermaid para el flujo.",
    ),
    (
        "Economía y trading",
        "Distingue hecho de interpretación, da el supuesto y el riesgo antes de la conclusión.",
    ),
    (
        "Fuentes verificadas",
        "Separa lo que está en la nota de lo que viene de fuera, y nombra la fuente de cada dato.",
    ),
];

/// The instructions behind each entry of [`PROMPT_PRESETS`].
pub fn preset_prompt(index: usize) -> Option<&'static str> {
    Some(match index {
        0 => DEFAULT_SYSTEM_PROMPT,
        1 => "\
Eres un tutor de matemática dentro de Obsidian. Responde en español y desarrolla el cálculo \
completo: cada igualdad es un paso y antes de cada una nombrás la regla que aplicás (derivada del \
producto, regla de la cadena, sustitución). Usa $...$ en línea y $$...$$ para lo destacado. \
Cierra con el resultado en una línea aparte. Si el dato no alcanza para resolver, decilo y pedí \
lo que falta en vez de inventar un número.",
        2 => "\
Eres un asistente técnico dentro de Obsidian. Respondé en español y preferí mostrar antes que \
contar: una tabla markdown cuando compares dos o más cosas y un bloque ```mermaid cuando haya un \
flujo, estados o una secuencia. Explicá el diagrama en una línea, no repitas en prosa lo que el \
dibujo ya dice. Usa LaTeX para las fórmulas.",
        3 => "\
Eres analista de mercados dentro de Obsidian. Respondé en español y separá siempre tres cosas: lo \
que es un hecho observable, lo que es tu interpretación y lo que es un supuesto que el lector \
debe aceptar. Da el riesgo y el escenario en contra antes de la conclusión. Nada de \
recomendaciones de compra o venta: describí el mecanismo. Si falta un dato, pedilo.",
        4 => "\
Eres un asistente de investigación dentro de Obsidian. Respondé en español y marcá el origen de \
cada afirmación: «en tu nota…» cuando sale de la nota abierta, «según la fuente que citás…» \
cuando viene de fuera, y una línea aparte al final con lo que no pudiste verificar. Nunca \
presentes una estimación como un dato.",
        _ => return None,
    })
}

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
pub const KEY_PROFILES: &str = "ai.profiles";
/// Name of the profile the panel sends to.
pub const KEY_ACTIVE: &str = "ai.active";
/// The single-endpoint keys of older vaults; they are read once and folded into a profile.
pub const KEY_BASE_URL: &str = "ai.base_url";
pub const KEY_API_KEY: &str = "ai.api_key";
pub const KEY_MODEL: &str = "ai.model";
pub const KEY_SYSTEM_PROMPT: &str = "ai.system_prompt";
pub const KEY_TIMEOUT: &str = "ai.timeout";
/// The profile the legacy single-endpoint keys turn into.
pub const LEGACY_PROFILE_NAME: &str = "Predeterminado";

/// The question «Explicar selección» asks; the selection itself follows, quoted.
pub const EXPLAIN_SELECTION_PROMPT: &str =
    "Explica el texto que cité: qué significa, para qué sirve y un ejemplo mínimo.";
/// The question «Resumir nota» asks about the open note.
pub const SUMMARIZE_NOTE_PROMPT: &str = "Resume la nota abierta: la idea central en una frase y \
    después hasta cinco puntos clave, en una lista.";

/// The provider presets offered in Ajustes → IA: picking one fills the URL and a typical model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    DeepSeek,
    OpenAi,
    OpenRouter,
    Groq,
    Ollama,
    /// Keeps whatever URL and model the profile has.
    Custom,
}

impl Preset {
    /// The presets in the order the settings dropdown walks them.
    pub const ALL: [Preset; 6] =
        [Preset::DeepSeek, Preset::OpenAi, Preset::OpenRouter, Preset::Groq, Preset::Ollama, Preset::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Preset::DeepSeek => "DeepSeek",
            Preset::OpenAi => "OpenAI",
            Preset::OpenRouter => "OpenRouter",
            Preset::Groq => "Groq",
            Preset::Ollama => "Ollama local",
            Preset::Custom => "Personalizado",
        }
    }

    /// The origin the preset writes. [`Preset::Custom`] has none: it keeps what is there.
    pub fn base_url(self) -> &'static str {
        match self {
            Preset::DeepSeek => "https://api.deepseek.com",
            Preset::OpenAi => "https://api.openai.com",
            Preset::OpenRouter => "https://openrouter.ai/api/v1",
            Preset::Groq => "https://api.groq.com/openai/v1",
            Preset::Ollama => "http://localhost:11434/v1",
            Preset::Custom => "",
        }
    }

    /// A model the provider usually serves.
    pub fn model(self) -> &'static str {
        match self {
            Preset::DeepSeek => DEFAULT_MODEL,
            Preset::OpenAi => "gpt-4o-mini",
            Preset::OpenRouter => "openai/gpt-4o-mini",
            Preset::Groq => "llama-3.3-70b-versatile",
            Preset::Ollama => "llama3.2",
            Preset::Custom => "",
        }
    }

    /// The preset a saved `base_url` belongs to; [`Preset::Custom`] when none matches.
    pub fn of_base_url(base_url: &str) -> Preset {
        let norm = |u: &str| u.trim().trim_end_matches('/').to_ascii_lowercase();
        let url = norm(base_url);
        Preset::ALL
            .into_iter()
            .find(|p| *p != Preset::Custom && norm(p.base_url()) == url)
            .unwrap_or(Preset::Custom)
    }

    /// The next preset in the dropdown.
    pub fn next(self) -> Preset {
        let at = Preset::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Preset::ALL[(at + 1) % Preset::ALL.len()]
    }
}

/// One provider the user configured: where it lives, the key it uses and the model to ask for.
#[derive(Clone, Debug, PartialEq)]
pub struct AiProfile {
    /// How the panel lists it; `ai.active` points at a profile by this name.
    pub name: String,
    /// Origin of the API, with or without its `/v1`. See [`chat_url`].
    pub base_url: String,
    /// Bearer token. Empty means the panel refuses to send.
    pub api_key: String,
    /// Set, the panel may only use these models; empty means every model the provider serves.
    pub enabled: Vec<String>,
    /// The model the panel asks for by default.
    pub model: String,
    /// Instructions for this profile. Empty means [`DEFAULT_SYSTEM_PROMPT`].
    pub system_prompt: String,
    /// Connect, response-header and per-read timeout of the request, in seconds.
    pub timeout_s: u64,
}

impl Default for AiProfile {
    fn default() -> AiProfile {
        AiProfile {
            name: LEGACY_PROFILE_NAME.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            model: DEFAULT_MODEL.to_string(),
            enabled: Vec::new(),
            system_prompt: String::new(),
            timeout_s: 30,
        }
    }
}

impl AiProfile {
    /// A profile named `name` with the default endpoint, model and no key.
    pub fn named(name: impl Into<String>) -> AiProfile {
        AiProfile { name: name.into(), ..AiProfile::default() }
    }

    /// The instructions to send: this profile's, or the default ones when it has none.
    pub fn instructions(&self) -> &str {
        if self.system_prompt.trim().is_empty() {
            DEFAULT_SYSTEM_PROMPT
        } else {
            &self.system_prompt
        }
    }

    /// The preset this profile's URL belongs to.
    pub fn preset(&self) -> Preset {
        Preset::of_base_url(&self.base_url)
    }

    /// Take the endpoint and the model from `preset`; [`Preset::Custom`] changes nothing.
    pub fn apply_preset(&mut self, preset: Preset) {
        if preset == Preset::Custom {
            return;
        }
        self.base_url = preset.base_url().to_string();
        self.model = preset.model().to_string();
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "base_url": self.base_url,
            "api_key": self.api_key,
            "model": self.model,
            "models": self.enabled,
            "system_prompt": self.system_prompt,
            "timeout_s": self.timeout_s,
        })
    }

    /// Read a profile (see [`AiProfile::to_json`]), with defaults for the missing fields.
    pub fn from_json(v: &Value) -> AiProfile {
        let d = AiProfile::default();
        let text = |key: &str, default: &str| {
            v.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .to_string()
        };
        AiProfile {
            name: text("name", &d.name),
            base_url: text("base_url", &d.base_url),
            api_key: v.get("api_key").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            model: text("model", &d.model),
            enabled: v
                .get("models")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            system_prompt: v.get("system_prompt").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            timeout_s: v.get("timeout_s").and_then(Value::as_u64).map_or(d.timeout_s, |n| n.clamp(1, 600)),
        }
    }
}

/// Every provider the user configured, and the name of the one the panel talks to.
///
/// A vault that only has the older single-endpoint keys has no `ai.profiles`; the first read
/// builds the "Predeterminado" profile from them, so the panel keeps working untouched.
#[derive(Clone, Debug, PartialEq)]
pub struct AiConfig {
    /// At least one profile: the constructors and [`AiConfig::normalize`] see to it.
    pub profiles: Vec<AiProfile>,
    /// The active profile's name. A name that matches no profile falls back to the first one;
    /// [`AiConfig::normalize`] leaves this field pointing at an existing profile.
    pub active: String,
}

impl Default for AiConfig {
    fn default() -> AiConfig {
        let p = AiProfile::default();
        AiConfig { active: p.name.clone(), profiles: vec![p] }
    }
}

impl AiConfig {
    /// Read `ai.profiles` and `ai.active`, folding the legacy single-endpoint keys into a profile
    /// when the vault has none.
    pub fn from_config(config: &vault_core::config::VaultConfig) -> AiConfig {
        let a = &config.app;
        let profiles = a
            .get(KEY_PROFILES)
            .and_then(Value::as_array)
            .map(|list| list.iter().map(AiProfile::from_json).collect())
            .unwrap_or_default();
        let active = a.str(KEY_ACTIVE).unwrap_or("").trim().to_string();
        let mut cfg = AiConfig { profiles, active };
        if cfg.profiles.is_empty() {
            cfg.profiles.push(legacy_profile(a));
        }
        cfg.normalize();
        cfg
    }

    /// The settings this struct owns, ready to be written to `.obsidian/app.json`.
    pub fn to_json(&self) -> Value {
        json!({
            "profiles": self.profiles.iter().map(AiProfile::to_json).collect::<Vec<_>>(),
            "active": self.active,
        })
    }

    /// Read a serialized config (see [`AiConfig::to_json`]), with defaults for what is missing.
    pub fn from_json(v: &Value) -> AiConfig {
        let profiles = v
            .get("profiles")
            .and_then(Value::as_array)
            .map(|list| list.iter().map(AiProfile::from_json).collect::<Vec<_>>())
            .unwrap_or_default();
        let active = v.get("active").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let mut cfg = AiConfig { profiles, active };
        cfg.normalize();
        cfg
    }

    /// Enforce what every reader relies on: at least one profile, every profile named and no two
    /// sharing a name (the name is how `ai.active` points at one), and `active` naming a profile
    /// that exists.
    pub(crate) fn normalize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles.push(AiProfile::default());
        }
        let mut taken: Vec<String> = Vec::with_capacity(self.profiles.len());
        for p in &mut self.profiles {
            let base =
                if p.name.trim().is_empty() { "Perfil".to_string() } else { p.name.trim().to_string() };
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name) {
                name = format!("{base} {n}");
                n += 1;
            }
            p.name = name.clone();
            taken.push(name);
        }
        if !self.profiles.iter().any(|p| p.name == self.active) {
            self.active = self.profiles[0].name.clone();
        }
    }

    /// Position of the active profile; the first one when `active` names none.
    pub fn active_index(&self) -> usize {
        self.profiles.iter().position(|p| p.name == self.active).unwrap_or(0)
    }

    /// The profile the panel sends to.
    pub fn active_profile(&self) -> &AiProfile {
        &self.profiles[self.active_index()]
    }

    /// A name no profile uses yet: `base`, else `base 2`, `base 3`…
    pub fn free_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Perfil" } else { base };
        let mut n = 1;
        loop {
            let name = if n == 1 { base.to_string() } else { format!("{base} {n}") };
            if !self.profiles.iter().any(|p| p.name == name) {
                return name;
            }
            n += 1;
        }
    }
}

/// The profile built from the single-endpoint keys of older vaults.
fn legacy_profile(a: &JsonFile) -> AiProfile {
    let d = AiProfile::default();
    let text = |key: &str, default: &str| {
        a.str(key).map(str::trim).filter(|v| !v.is_empty()).unwrap_or(default).to_string()
    };
    AiProfile {
        name: LEGACY_PROFILE_NAME.to_string(),
        base_url: text(KEY_BASE_URL, &d.base_url),
        api_key: a.str(KEY_API_KEY).map(str::trim).unwrap_or("").to_string(),
        model: text(KEY_MODEL, &d.model),
        enabled: Vec::new(),
        system_prompt: text(KEY_SYSTEM_PROMPT, ""),
        timeout_s: a.get(KEY_TIMEOUT).and_then(Value::as_u64).map_or(d.timeout_s, |n| n.clamp(1, 600)),
    }
}

/// The URL of the chat endpoint for `base`, which may already end in `/v1`.
/// The address that lists the models of a provider, next to [`chat_url`].
pub fn models_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let base = if base.is_empty() { DEFAULT_BASE_URL } else { base };
    if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

/// The ids a `/models` body lists, sorted and without repeats. Providers answer the
/// OpenAI-compatible shape (`{"data":[{"id":…}]}`); anything else yields an empty list instead
/// of an error, so an unexpected body reads as "no models" rather than a failure to parse.
pub fn parse_models(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let mut ids: Vec<String> = v
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str).or_else(|| m.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Ask a provider which models it serves. Blocking: the settings panel runs it on a thread.
pub fn list_models(profile: &AiProfile) -> Result<Vec<String>, String> {
    if profile.api_key.trim().is_empty() {
        return Err("Falta la clave de API: cargala y volvé a consultar".to_string());
    }
    let secs = profile.timeout_s.clamp(1, 600);
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(secs)))
        .timeout_recv_response(Some(Duration::from_secs(secs)))
        .timeout_recv_body(Some(Duration::from_secs(secs)))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let auth = format!("Bearer {}", profile.api_key.trim());
    let res = agent
        .get(&models_url(&profile.base_url))
        .header("Accept", "application/json")
        .header("Authorization", auth.as_str())
        .call()
        .map_err(|e| format!("No se pudo contactar la API: {e}"))?;
    let status = res.status();
    let body = res.into_body().read_to_string().unwrap_or_default();
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 | 403 => "La clave fue rechazada".to_string(),
            404 => "El proveedor no expone /models: escribí el modelo a mano".to_string(),
            429 => "Demasiadas consultas: probá en un rato".to_string(),
            s => format!("El proveedor respondió {s}"),
        };
        return Err(hint);
    }
    let ids = parse_models(&body);
    if ids.is_empty() {
        return Err("La respuesta no trae modelos: escribí el nombre a mano".to_string());
    }
    Ok(ids)
}

/// The models a provider listed, fetched on a thread so the panel never blocks on the network.
#[derive(Default)]
pub struct ModelList {
    pending: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// The profile the list belongs to, so an answer for another profile is not applied.
    pub profile: String,
    pub list: Vec<String>,
    pub error: Option<String>,
}

impl ModelList {
    /// Starts the query for `profile`. A second call replaces the first.
    pub fn fetch(&mut self, profile: &AiProfile) {
        let (tx, rx) = std::sync::mpsc::channel();
        let p = profile.clone();
        std::thread::spawn(move || {
            let _ = tx.send(list_models(&p));
        });
        self.profile = profile.name.clone();
        self.list.clear();
        self.error = None;
        self.pending = Some(rx);
    }

    /// Whether a query is in flight for `profile`.
    pub fn pending_for(&self, profile: &str) -> bool {
        self.pending.is_some() && self.profile == profile
    }

    /// Takes an answer if one arrived. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        match rx.try_recv() {
            Ok(Ok(list)) => {
                self.list = list;
                self.error = None;
                self.pending = None;
                true
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.pending = None;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.error = Some("La consulta de modelos se cortó".to_string());
                self.pending = None;
                true
            }
        }
    }
}

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
pub fn system_content(cfg: &AiProfile, note_context: &str) -> String {
    let mut out = cfg.instructions().trim_end().to_string();
    if !note_context.trim().is_empty() {
        out.push_str("\n\n## Contexto de la bóveda\n\n");
        out.push_str(note_context.trim());
    }
    out
}

/// The OpenAI request body: system message, prior turns and the question.
pub fn build_payload(cfg: &AiProfile, system: &str, history: &[Message], user_input: &str) -> Value {
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

/// What the editor has selected, trimmed and cut to [`MAX_SELECTION_CHARS`].
pub fn selected_text(tab: &Tab) -> Option<String> {
    let text = tab.editor.buf.text();
    selection(tab, &text)
}

/// What a quick action of the panel asks about the open note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAsk {
    /// The editor's selection, quoted in the question.
    Selection,
    /// The whole open note.
    Summary,
}

/// The question a quick action sends about `tab`. `Err` is the notice to show instead: a
/// selection action with nothing selected, for instance.
pub fn quick_question(kind: QuickAsk, tab: &Tab) -> Result<String, &'static str> {
    match kind {
        QuickAsk::Selection => {
            let sel = selected_text(tab).ok_or("Seleccioná texto en el editor para explicarlo.")?;
            Ok(format!("{EXPLAIN_SELECTION_PROMPT}\n\n\"\"\"\n{sel}\n\"\"\""))
        }
        QuickAsk::Summary => Ok(SUMMARIZE_NOTE_PROMPT.to_string()),
    }
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
    pub fn send(&mut self, cfg: &AiProfile, note_context: &str, user_input: &str) {
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
        let payload = build_payload(cfg, &system_content(cfg, note_context), &history, question).to_string();
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
pub(crate) fn stream_request(cfg: &AiProfile, payload: &str, tx: &Sender<Event>) {
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
    /// The AI settings as stored in `.obsidian/app.json`.
    pub(crate) fn ai_config(&self) -> AiConfig {
        AiConfig::from_config(&self.config)
    }

    /// Write the profiles and the active one to `.obsidian/app.json`.
    pub(crate) fn set_ai_config(&mut self, cfg: &AiConfig) {
        let v = cfg.to_json();
        self.config.app.set(KEY_PROFILES, v["profiles"].clone());
        self.config.app.set(KEY_ACTIVE, v["active"].clone());
        if let Err(e) = self.config.app.save() {
            self.notice = Some(format!("No se pudieron guardar los perfiles de IA: {e}"));
        }
    }

    /// Send what the panel's input holds, with the open note as context and the active profile.
    pub(crate) fn ai_send(&mut self) {
        if self.ai.input.trim().is_empty() {
            return;
        }
        let profile = self.ai_config().active_profile().clone();
        let context = self.tab().map(context_from_note).unwrap_or_default();
        let input = self.ai.input.clone();
        self.ai.send(&profile, &context, &input);
        if let Some(e) = &self.ai.error {
            self.notice = Some(e.clone());
        }
    }

    /// Run a quick action: the panel opens and asks a prepared question about the open note.
    pub(crate) fn ai_quick(&mut self, kind: QuickAsk) {
        if self.ai.busy() {
            self.notice = Some("Esperá a que termine la respuesta.".to_string());
            return;
        }
        let asked = match self.tab() {
            Some(tab) => quick_question(kind, tab).map(|q| (q, context_from_note(tab))),
            None => Err("Abrí una nota para usar la IA sobre ella."),
        };
        let (question, context) = match asked {
            Ok(asked) => asked,
            Err(notice) => {
                self.notice = Some(notice.to_string());
                return;
            }
        };
        let profile = self.ai_config().active_profile().clone();
        self.right_open = true;
        self.right_panel = RightPanel::Ai;
        self.ai.send(&profile, &context, &question);
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
        Tab::new("Nota.md".to_string(), text, vault_core::fsio::Stamp { mtime: 0, size: 0 }, false)
    }

    fn profile(name: &str, url: &str, model: &str) -> AiProfile {
        AiProfile {
            name: name.into(),
            base_url: url.into(),
            api_key: "sk-test".into(),
            model: model.into(),
            enabled: Vec::new(),
            system_prompt: "Sé breve.".into(),
            timeout_s: 20,
        }
    }

    #[test]
    fn profiles_roundtrip_through_json() {
        let cfg = AiConfig {
            profiles: vec![
                profile("Local", "http://localhost:11434/v1", "llama3.2"),
                profile("Nube", "https://api.openai.com", "gpt-4o-mini"),
            ],
            active: "Nube".into(),
        };
        let json = cfg.to_json();
        assert_eq!(json["active"], "Nube");
        assert_eq!(json["profiles"].as_array().unwrap().len(), 2);
        assert_eq!(AiConfig::from_json(&json), cfg);

        // A profile with missing fields takes the defaults.
        assert_eq!(AiProfile::from_json(&json!({})), AiProfile::default());
        assert_eq!(AiProfile::from_json(&json!({ "model": "  " })).model, DEFAULT_MODEL);
        assert_eq!(AiProfile::from_json(&json!({ "timeout_s": 0 })).timeout_s, 1);
        assert_eq!(AiConfig::from_json(&json!({})), AiConfig::default());
        assert_eq!(chat_url("https://api.deepseek.com/"), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(chat_url("http://127.0.0.1:1/v1"), "http://127.0.0.1:1/v1/chat/completions");
    }

    #[test]
    fn legacy_single_endpoint_settings_become_a_profile() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".obsidian")).unwrap();
        std::fs::write(
            d.path().join(".obsidian/app.json"),
            r#"{"ai.base_url":"https://api.groq.com/openai/v1","ai.api_key":"sk-legacy",
                "ai.model":"llama-3.3-70b-versatile","ai.system_prompt":"Sé breve.","ai.timeout":45}"#,
        )
        .unwrap();
        let config = vault_core::config::VaultConfig::load(d.path()).unwrap();
        let cfg = AiConfig::from_config(&config);
        assert_eq!(cfg.profiles.len(), 1, "the installed keys become a single profile");
        assert_eq!(cfg.active, LEGACY_PROFILE_NAME, "and that profile is the active one");
        let p = cfg.active_profile();
        assert_eq!(p.name, LEGACY_PROFILE_NAME);
        assert_eq!(p.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(p.api_key, "sk-legacy");
        assert_eq!(p.model, "llama-3.3-70b-versatile");
        assert_eq!(p.instructions(), "Sé breve.");
        assert_eq!(p.timeout_s, 45);
    }

    #[test]
    fn a_vault_without_ai_settings_gets_one_default_profile() {
        let d = tempfile::tempdir().unwrap();
        let config = vault_core::config::VaultConfig::load(d.path()).unwrap();
        let cfg = AiConfig::from_config(&config);
        assert_eq!(cfg.profiles, vec![AiProfile::default()]);
        assert_eq!(cfg.active, LEGACY_PROFILE_NAME);
        assert_eq!(cfg.active_profile().instructions(), DEFAULT_SYSTEM_PROMPT);
        assert_eq!(cfg.active_profile().model, DEFAULT_MODEL);
    }

    #[test]
    fn the_active_profile_falls_back_to_the_first() {
        let cfg = AiConfig::from_json(&json!({
            "profiles": [{ "name": "A" }, { "name": "B" }],
            "active": "B"
        }));
        assert_eq!(cfg.active_index(), 1);
        assert_eq!(cfg.active_profile().name, "B");

        let dangling = AiConfig::from_json(&json!({
            "profiles": [{ "name": "A" }, { "name": "B" }],
            "active": "Z"
        }));
        assert_eq!(dangling.active, "A", "a name that matches nothing falls back to the first");
        assert_eq!(dangling.active_profile().name, "A");

        // The name is the key `ai.active` points at, so blanks and repeats are fixed.
        let dup = AiConfig::from_json(&json!({
            "profiles": [{ "name": "A" }, { "name": "A" }, { "name": "  " }]
        }));
        let names: Vec<_> = dup.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A", "A 2", "Predeterminado"], "a nameless profile takes the default name");
        assert_eq!(dup.free_name("A"), "A 3");
        assert_eq!(dup.free_name("Local"), "Local");
    }

    #[test]
    fn presets_fill_the_endpoint_and_a_model() {
        let mut p = AiProfile::named("Local");
        p.apply_preset(Preset::Ollama);
        assert_eq!(p.base_url, "http://localhost:11434/v1");
        assert_eq!(p.model, Preset::Ollama.model());
        assert_eq!(p.preset(), Preset::Ollama);
        p.apply_preset(Preset::Custom);
        assert_eq!(p.base_url, "http://localhost:11434/v1", "Personalizado keeps what is there");
        assert_eq!(Preset::of_base_url("https://openrouter.ai/api/v1/"), Preset::OpenRouter);
        assert_eq!(Preset::of_base_url("http://127.0.0.1:8080"), Preset::Custom);
        assert_eq!(Preset::DeepSeek.next(), Preset::OpenAi);
        assert_eq!(Preset::Custom.next(), Preset::DeepSeek);
        let labels: Vec<_> = Preset::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels, ["DeepSeek", "OpenAI", "OpenRouter", "Groq", "Ollama local", "Personalizado"]);
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
        let cfg = AiProfile::default();
        let ctx = context_from_note(&tab("la nota"));
        let system = system_content(&cfg, &ctx);
        assert!(system.starts_with(cfg.instructions()));
        assert!(system.contains("la nota"), "{system}");
        assert!(system.contains("$...$") || system.contains("LaTeX"));
        assert_eq!(system_content(&cfg, "   "), cfg.instructions());
        let custom = AiProfile { system_prompt: "Solo respuestas cortas.".into(), ..Default::default() };
        assert!(system_content(&custom, "").starts_with("Solo respuestas cortas."));
    }

    #[test]
    fn quick_actions_ask_about_the_selection_and_the_note() {
        let mut t = tab("primera linea\nsegunda linea\n");
        let err = quick_question(QuickAsk::Selection, &t).unwrap_err();
        assert!(err.contains("Seleccioná"), "{err}");
        t.editor.buf.set_sels(vec![editor_core::Sel::new(0, 13)]);
        let q = quick_question(QuickAsk::Selection, &t).unwrap();
        assert!(q.starts_with(EXPLAIN_SELECTION_PROMPT), "{q}");
        assert!(q.contains("primera linea"), "the selection travels in the question");
        assert_eq!(quick_question(QuickAsk::Summary, &t).unwrap(), SUMMARIZE_NOTE_PROMPT);
    }

    #[test]
    fn payload_is_openai_shaped() {
        let cfg = AiProfile::default();
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
        assert_eq!(parse_delta(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#).unwrap(), None);
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
        let cfg = AiProfile { base_url: base, api_key: "sk-test".into(), ..Default::default() };
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
    fn models_url_follows_the_same_rule_as_chat_url() {
        assert_eq!(models_url("https://api.deepseek.com"), "https://api.deepseek.com/v1/models");
        assert_eq!(models_url("http://localhost:11434/v1/"), "http://localhost:11434/v1/models");
        assert_eq!(models_url(""), "https://api.deepseek.com/v1/models");
    }

    #[test]
    fn a_models_body_becomes_a_sorted_list_of_ids() {
        let body = r#"{"object":"list","data":[{"id":"deepseek-v4.1-flash"},{"id":"deepseek-v4-pro"},{"id":"deepseek-v4.1-flash"}]}"#;
        // Sorted as bytes: `-` comes before `.`, so the v4 names go first and the v4.1 ones after.
        assert_eq!(parse_models(body), vec!["deepseek-v4-pro", "deepseek-v4.1-flash"]);
        // A provider that answers plain strings is still read.
        assert_eq!(parse_models(r#"{"data":["b","a"]}"#), vec!["a", "b"]);
        // Anything unexpected reads as "no models", not as a parse failure.
        assert!(parse_models("no es json").is_empty());
        assert!(parse_models(r#"{"data":[]}"#).is_empty());
    }

    #[test]
    fn send_requires_a_key_and_fills_the_panel() {
        let mut panel = AiPanel { input: "hola".into(), ..Default::default() };
        let cfg = AiProfile::default();
        panel.send(&cfg, "", "hola");
        assert!(panel.error.as_deref().unwrap().contains("clave"));
        assert!(panel.messages.is_empty());
        assert_eq!(panel.input, "hola", "the text is kept so it can be sent later");

        // A configured panel starts a request; the answer goes into the last message.
        let (base, _server) = mock_server("data: [DONE]\n\n");
        let cfg = AiProfile { base_url: base, api_key: "sk".into(), ..Default::default() };
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
