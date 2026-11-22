//! `metamind://` links, so other programs and web pages can open or create notes.
//!
//! - `metamind://open?vault=<name or path>&file=<note>`
//! - `metamind://open?path=<absolute path to a note or vault folder>`
//! - `metamind://new?vault=<name or path>&name=<note path>&content=<text>`
//!
//! A vault given by name is looked up among the recently opened vaults by folder name.

use std::path::{Path, PathBuf};
use vault_core::path;

pub const SCHEME: &str = "metamind:";

/// What a link asks for, once resolved: the vault to open and the note to show in it.
#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub vault: PathBuf,
    pub note: Option<String>,
}

pub fn is_link(s: &str) -> bool {
    s.get(..SCHEME.len()).is_some_and(|p| p.eq_ignore_ascii_case(SCHEME))
}

/// The action name and its decoded query parameters.
pub fn parse(uri: &str) -> Result<(String, Vec<(String, String)>), String> {
    if !is_link(uri) {
        return Err(format!("no es un enlace {SCHEME}//: {uri}"));
    }
    let rest = uri[SCHEME.len()..].trim_start_matches('/');
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    let action = action.trim_end_matches('/').to_ascii_lowercase();
    let params = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (path::percent_decode(k), path::percent_decode(v))
        })
        .collect();
    Ok((action, params))
}

/// Resolve a link against the known vaults. `new` writes the note before returning.
pub fn resolve(uri: &str, vaults: &[PathBuf]) -> Result<Target, String> {
    let (action, params) = parse(uri)?;
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    match action.as_str() {
        "open" => {
            if let Some(p) = get("path") {
                return for_path(Path::new(p), vaults);
            }
            let vault = vault(get("vault"), vaults)?;
            Ok(Target { vault, note: get("file").map(str::to_string) })
        }
        "new" => {
            let vault = vault(get("vault"), vaults)?;
            let name = get("name").or(get("file")).unwrap_or("Untitled");
            let note = create(&vault, name, get("content").unwrap_or(""))?;
            Ok(Target { vault, note: Some(note) })
        }
        "" => Err("el enlace no indica ninguna acción".into()),
        other => Err(format!("acción de enlace no compatible `{other}`")),
    }
}

fn vault(name: Option<&str>, vaults: &[PathBuf]) -> Result<PathBuf, String> {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return vaults
            .first()
            .cloned()
            .ok_or_else(|| "el enlace no indica ninguna bóveda y antes no se abrió ninguna".into());
    };
    let p = Path::new(name);
    if p.is_absolute() && p.is_dir() {
        return Ok(p.to_path_buf());
    }
    vaults
        .iter()
        .find(|v| v.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name)))
        .cloned()
        .ok_or_else(|| format!("ninguna bóveda abierta recientemente se llama `{name}`"))
}

/// A command-line argument naming an existing file, as a path or a `file://` URL (desktop
/// launchers may pass either).
pub fn local_file(arg: &str) -> Option<PathBuf> {
    let p = match arg.strip_prefix("file://") {
        Some(rest) => {
            // `file://host/path` names another machine; only an empty host is local.
            let rest = path::percent_decode(rest.strip_prefix("localhost").unwrap_or(rest));
            let abs = rest.strip_prefix('/')?;
            // `file:///C:/x` on Windows; `file:///home/x` elsewhere.
            if cfg!(windows) && abs.get(1..2) == Some(":") {
                PathBuf::from(abs)
            } else {
                PathBuf::from(format!("/{abs}"))
            }
        }
        None => PathBuf::from(arg),
    };
    // Without `\\?\`, so the path compares equal to the recent-vault list.
    p.is_file().then(|| std::fs::canonicalize(&p).map(|c| crate::recent::clean(&c)).unwrap_or(p))
}

/// A folder opens as a vault; a file opens in the known vault that contains it, or else in the
/// nearest enclosing folder that has a `.obsidian` settings folder, or else its own folder.
pub fn for_path(p: &Path, vaults: &[PathBuf]) -> Result<Target, String> {
    if p.is_dir() {
        return Ok(Target { vault: p.to_path_buf(), note: None });
    }
    if !p.is_file() {
        return Err(format!("{} no existe", p.display()));
    }
    let known = vaults.iter().filter(|v| p.starts_with(v)).max_by_key(|v| v.components().count());
    let vault = known
        .cloned()
        .or_else(|| p.ancestors().skip(1).find(|d| d.join(".obsidian").is_dir()).map(Path::to_path_buf))
        .or_else(|| p.parent().map(Path::to_path_buf))
        .ok_or_else(|| format!("{} no está dentro de una bóveda", p.display()))?;
    let rel = p.strip_prefix(&vault).map_err(|_| format!("{} no está dentro de una bóveda", p.display()))?;
    Ok(Target { vault, note: Some(rel.to_string_lossy().replace('\\', "/")) })
}

/// Write a new note, numbering the name when it is taken. Returns its vault-relative path.
fn create(vault: &Path, name: &str, content: &str) -> Result<String, String> {
    if name.split(['/', '\\']).any(|s| s == "..") {
        return Err(format!("`{name}` leaves the vault"));
    }
    let base = path::normalize(path::strip_md(name));
    if base.is_empty() {
        return Err("the link gives an empty note name".into());
    }
    let mut rel = format!("{base}.md");
    let mut n = 1;
    while vault.join(&rel).exists() {
        rel = format!("{base} {n}.md");
        n += 1;
    }
    let file = vault.join(&rel);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&file, content).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(rel)
}
