//! Run a `TABLE` query against a folder of notes and print the GFM table.
//!
//! `cargo run -p dataview-core --example query -- <vault-folder> <query-file>`
//!
//! The folder is read as the app reads a vault: Markdown files, dot-folders skipped. Dates of
//! `file.mtime` are UTC here, so the output of the same vault is the same on every machine.

use std::fs;
use std::path::Path;
use std::process::ExitCode;
use vault_core::path::{self, FileKind};
use vault_core::Vault;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Ok(root), [_, query]) =
        (Path::new(args.first().map_or("", String::as_str)).canonicalize(), &args[..])
    else {
        eprintln!("usage: query <vault-folder> <query-file>");
        return ExitCode::FAILURE;
    };
    let Ok(src) = fs::read_to_string(query) else {
        eprintln!("cannot read the query file");
        return ExitCode::FAILURE;
    };
    let vault = load(&root);
    match dataview_core::render(&vault, &src, dataview_core::Opts::default()) {
        Ok(table) => {
            print!("{table}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("dataview: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A vault index built from the Markdown files `root` holds.
fn load(root: &Path) -> Vault {
    fn walk(root: &Path, dir: &Path, v: &mut Vault) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let abs = e.path();
            if abs.is_dir() {
                walk(root, &abs, v);
                continue;
            }
            if path::kind_of(&name) != FileKind::Markdown {
                continue;
            }
            let Ok(text) = fs::read_to_string(&abs) else { continue };
            let Ok(rel) = abs.strip_prefix(root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as i64);
            let meta = vault_core::meta::extract(&text);
            v.upsert(&rel, mtime, text.len() as u64, Some(meta));
        }
    }
    let mut v = Vault::new();
    walk(root, root, &mut v);
    v
}
