//! Command-line interface. Every subcommand except `open` is headless.

use crate::shell::{App, Key};
use search_core::{Corpus, Query, RankWeights, Sort};
use std::path::{Path, PathBuf};
use std::time::Instant;
use vault_core::config::VaultConfig;
use vault_core::{path, VaultStore};

pub const USAGE: &str = "Rustsidian

USO:
  rustsidian                                    Muestra la pantalla de bienvenida para elegir o crear una bóveda
  rustsidian [open] <bóveda> [nota]             Abre una bóveda en una ventana
  rustsidian <archivo.md>                       Abre una nota de la bóveda que lo contiene
  rustsidian <rustsidian://enlace>              Abre o crea una nota desde un enlace (acciones: open, new)
  rustsidian index <bóveda>                     Crea o actualiza el índice e imprime estadísticas
  rustsidian search <bóveda> <consulta> [--limit N] Busca con la sintaxis de consultas
  rustsidian export <bóveda> <nota> [--format html|md|pdf] [-o <ruta>]
  rustsidian doctor <bóveda>                    Diagnostica la configuración, el índice y las notas
  rustsidian screenshot <bóveda> <salida.png> [--note <ruta>] [--size WxH] [--keys \"Ctrl+P ...\"] [--type <texto>] [--run <id de comando>,...] [--frames N] [--scroll-to <texto>] [--light]
  rustsidian welcome-screenshot <salida.png> [--run <acción>,...] [--type <texto>]
  rustsidian commands                           Lista los ids de comando y sus atajos predeterminados
  rustsidian update [--check]                   Instala la última versión o solo la informa
  rustsidian version                            Imprime la versión

OPCIONES:
  --profile    Imprime un desglose de tiempos de inicio (con `open` e `index`)

No implementado en esta compilación: `rustsidian plugin`, `--config-dir`, modo portátil.";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

fn parse_args(args: &[String]) -> Args {
    const WITH_VALUE: &[&str] = &[
        "--limit",
        "--format",
        "-o",
        "--note",
        "--size",
        "--keys",
        "--type",
        "--run",
        "--frames",
        "--scroll-to",
    ];
    let mut out = Args { pos: Vec::new(), flags: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if WITH_VALUE.contains(&a.as_str()) {
            out.flags.push((a.clone(), it.next().cloned()));
        } else if a.starts_with("--") {
            out.flags.push((a.clone(), None));
        } else {
            out.pos.push(a.clone());
        }
    }
    out
}

impl Args {
    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }
    fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }
}

pub enum Outcome {
    /// Finished; exit with this code.
    Exit(i32),
    /// No vault was named: show the welcome screen to pick one.
    Welcome,
    /// Open a window on this vault, optionally with a note.
    Open { vault: PathBuf, note: Option<String>, profile: bool },
}

fn fail(msg: &str) -> Outcome {
    eprintln!("rustsidian: {msg}");
    Outcome::Exit(2)
}

/// Run a CLI invocation. Output goes to `out` so tests can capture it.
pub fn run(args: &[String], out: &mut dyn std::io::Write) -> Outcome {
    let a = parse_args(args);
    let Some(first) = a.pos.first().map(String::as_str) else {
        if a.flag("--version") {
            let _ = writeln!(out, "rustsidian {}", crate::update::CURRENT);
            return Outcome::Exit(0);
        }
        if !a.flag("--help") && cfg!(feature = "window") {
            return Outcome::Welcome;
        }
        let _ = writeln!(out, "{USAGE}");
        return Outcome::Exit(if a.flag("--help") { 0 } else { 2 });
    };
    let vault_arg = |i: usize| a.pos.get(i).map(PathBuf::from).filter(|p| p.is_dir());
    match first {
        "help" => {
            let _ = writeln!(out, "{USAGE}");
            Outcome::Exit(0)
        }
        "version" => {
            let _ = writeln!(out, "rustsidian {}", crate::update::CURRENT);
            Outcome::Exit(0)
        }
        "update" => {
            let r = match crate::update::check() {
                Ok(Some(r)) => r,
                Ok(None) => {
                    let _ = writeln!(out, "rustsidian {} es la última versión", crate::update::CURRENT);
                    return Outcome::Exit(0);
                }
                Err(e) => return fail(&format!("update: {e}")),
            };
            let _ = writeln!(out, "{} está disponible (instalada: v{})", r.tag, crate::update::CURRENT);
            if a.flag("--check") {
                return Outcome::Exit(0);
            }
            let exe = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => return fail(&format!("update: no se pudo encontrar el archivo del programa: {e}")),
            };
            match crate::update::install(&r, &exe) {
                Ok(()) => {
                    let _ = writeln!(out, "se instaló {} en {}", r.tag, exe.display());
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("update: {e}")),
            }
        }
        "commands" => {
            for (id, name, key) in crate::shell::COMMANDS {
                let _ = writeln!(out, "{id:36} {key:22} {name}");
            }
            Outcome::Exit(0)
        }
        "plugin" => fail("el sistema de plugins no está implementado en esta compilación (ver README)"),
        "index" => {
            let Some(root) = vault_arg(1) else { return fail("index: se esperaba una carpeta de bóveda") };
            match VaultStore::open(&root) {
                Ok((mut store, st)) => {
                    let t = Instant::now();
                    let saved = store.save_cache().unwrap_or(false);
                    let graph = store.index.graph();
                    let links: usize =
                        graph.out.iter().map(|o| o.iter().map(|x| x.1 as usize).sum::<usize>()).sum();
                    let unresolved: usize = graph.unresolved.iter().map(Vec::len).sum();
                    let _ = writeln!(out, "archivos: {}\nnotas: {}\nanalizados: {}\nreutilizados del índice: {}\nenlaces resueltos: {links}\ndestinos de enlace sin resolver: {unresolved}", st.files, st.notes, st.parsed, st.reused);
                    let _ = writeln!(out, "índice escrito: {saved}");
                    if a.flag("--profile") {
                        let _ = writeln!(
                            out,
                            "apertura: {:?} (recorrido {:?}, carga del índice {:?}, análisis {:?}); guardado + gráfico: {:?}",
                            st.total,
                            st.walk,
                            st.load_cache,
                            st.parse,
                            t.elapsed()
                        );
                    }
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("index: {e}")),
            }
        }
        "search" => {
            let (Some(root), Some(q)) = (vault_arg(1), a.pos.get(2)) else {
                return fail("search: se esperaba <bóveda> <consulta>");
            };
            let query = match Query::parse(q) {
                Ok(q) => q,
                Err(e) => return fail(&format!("search: {e}")),
            };
            let store = match VaultStore::open(&root) {
                Ok((s, _)) => s,
                Err(e) => return fail(&format!("search: {e}")),
            };
            let limit = a.value("--limit").and_then(|v| v.parse().ok()).unwrap_or(20);
            let corpus = Corpus::build(&store.index, &root);
            for hit in
                corpus.search(&store.index, &root, &query, Sort::Relevance, RankWeights::default(), limit)
            {
                let Some(f) = store.index.entry(hit.file) else { continue };
                let _ = writeln!(out, "{}", f.path);
                let text = std::fs::read_to_string(root.join(&*f.path)).unwrap_or_default();
                for m in hit.matches.iter().take(3) {
                    let line = text[..(m.start as usize).min(text.len())].matches('\n').count() + 1;
                    let _ = writeln!(out, "  {line}: {}", search_core::snippet(&text, *m, 50).0.trim());
                }
            }
            Outcome::Exit(0)
        }
        "export" => {
            let (Some(root), Some(note)) = (vault_arg(1), a.pos.get(2)) else {
                return fail("export: se esperaba <bóveda> <nota>");
            };
            export(&root, note, a.value("--format").unwrap_or("html"), a.value("-o"), out)
        }
        "doctor" => {
            let Some(root) = vault_arg(1) else { return fail("doctor: se esperaba una carpeta de bóveda") };
            doctor(&root, out)
        }
        "screenshot" => {
            let (Some(root), Some(png)) = (vault_arg(1), a.pos.get(2)) else {
                return fail("screenshot: se esperaba <bóveda> <salida.png>");
            };
            let (w, h) = a
                .value("--size")
                .and_then(|s| s.split_once('x'))
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or((1280u32, 800u32));
            let mut app = match App::open(&root) {
                Ok(app) => app,
                Err(e) => return fail(&format!("screenshot: {e}")),
            };
            crate::spell::load();
            if a.flag("--light") {
                app.dark = false;
            }
            if let Some(n) = a.value("--note") {
                app.open_note(n);
            }
            for k in a.value("--keys").unwrap_or("").split_whitespace() {
                match Key::parse(k) {
                    Some(key) => {
                        app.key(key);
                    }
                    None => return fail(&format!("screenshot: tecla no válida `{k}`")),
                }
            }
            if let Some(t) = a.value("--type") {
                for c in t.chars() {
                    app.text_input(&c.to_string());
                }
            }
            for c in a.value("--run").unwrap_or("").split(',').filter(|c| !c.is_empty()) {
                app.run(c.trim());
            }
            // A long note is laid out around the block at the top of the view, so a shot of a
            // note's last section needs the anchor moved there before the first frame.
            if let (Some(needle), Some(tab)) = (a.value("--scroll-to"), app.tab_mut()) {
                let doc = tab.editor.buf.doc();
                let at = doc
                    .blocks
                    .iter()
                    .position(|b| tab.editor.buf.slice(b.start, b.end()).contains(needle))
                    .unwrap_or(0);
                tab.top_block = at;
                tab.scroll_px = 0.0;
            }
            if let Some(g) = &mut app.graph_view {
                g.settle();
            }
            let mut fonts = rustsidian_ui::Fonts::new();
            let Some(mut pm) = rustsidian_ui::Pixmap::new(w.clamp(64, 8192), h.clamp(64, 8192)) else {
                return fail("screenshot: tamaño no válido");
            };
            // Two frames so the perf overlay shows a warm-cache frame time; `--frames` renders
            // more, which is what a scroll animation and the block window need to settle.
            let frames = a.value("--frames").and_then(|v| v.parse().ok()).unwrap_or(2).max(1);
            for _ in 0..frames {
                crate::render(&mut app, &mut fonts, &mut pm, 1.0);
            }
            match pm.save_png(png) {
                Ok(()) => {
                    let _ = writeln!(
                        out,
                        "se escribió {png} ({w}x{h}), fotograma {:.2} ms",
                        app.last_frame_us as f64 / 1000.0
                    );
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("screenshot: {e}")),
            }
        }
        "welcome-screenshot" => {
            let Some(png) = a.pos.get(1) else { return fail("welcome-screenshot: se esperaba <salida.png>") };
            let mut w = crate::welcome::Welcome::new();
            for c in a.value("--run").unwrap_or("").split(',').filter(|c| !c.is_empty()) {
                w.run(c.trim());
            }
            if let Some(t) = a.value("--type") {
                w.text_input(t);
            }
            let mut fonts = rustsidian_ui::Fonts::new();
            let Some(mut pm) = rustsidian_ui::Pixmap::new(1280, 800) else {
                return fail("welcome-screenshot: tamaño no válido");
            };
            w.render(&mut fonts, &mut pm, 1.0);
            match pm.save_png(png) {
                Ok(()) => {
                    let _ = writeln!(out, "se escribió {png}");
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("welcome-screenshot: {e}")),
            }
        }
        "open" => match vault_arg(1) {
            Some(vault) => Outcome::Open { vault, note: a.pos.get(2).cloned(), profile: a.flag("--profile") },
            None => fail("open: se esperaba una carpeta de bóveda"),
        },
        _ if crate::uri::is_link(first) => match crate::uri::resolve(first, &crate::recent::load()) {
            Ok(t) => Outcome::Open { vault: t.vault, note: t.note, profile: false },
            Err(e) => fail(&e),
        },
        _ => {
            if let Some(file) = crate::uri::local_file(first) {
                return match crate::uri::for_path(&file, &crate::recent::load()) {
                    Ok(t) => Outcome::Open { vault: t.vault, note: t.note, profile: a.flag("--profile") },
                    Err(e) => fail(&e),
                };
            }
            match vault_arg(0) {
                Some(vault) => {
                    Outcome::Open { vault, note: a.pos.get(1).cloned(), profile: a.flag("--profile") }
                }
                None => fail(&format!(
                    "`{first}` no es un comando, un archivo ni una carpeta. Ejecuta `rustsidian help`."
                )),
            }
        }
    }
}

struct VaultLinks<'a> {
    store: &'a VaultStore,
    source: &'a str,
}

impl md_engine::html::LinkResolver for VaultLinks<'_> {
    fn resolve(&self, target: &str) -> Option<String> {
        let id = self.store.index.resolve(target, self.source)?;
        let f = self.store.index.entry(id)?;
        let rel = path::relative(path::parent(self.source), &f.path);
        Some(if f.kind == vault_core::FileKind::Markdown {
            format!("{}.html", path::strip_md(&rel))
        } else {
            rel
        })
    }
}

fn export(
    root: &Path,
    note: &str,
    format: &str,
    dest: Option<&str>,
    out: &mut dyn std::io::Write,
) -> Outcome {
    let store = match VaultStore::open(root) {
        Ok((s, _)) => s,
        Err(e) => return fail(&format!("export: {e}")),
    };
    let Some(id) = store.index.resolve(note, "") else {
        return fail(&format!("export: no se encontró la nota `{note}`"));
    };
    let rel = store.index.entry(id).map(|f| f.path.to_string()).unwrap_or_default();
    let text = match store.read(&rel) {
        Ok((t, _)) => t,
        Err(e) => return fail(&format!("export: {e}")),
    };
    match format {
        "html" => {
            let doc = md_engine::parse(&text);
            let body = md_engine::html::render_with(&doc, &text, &VaultLinks { store: &store, source: &rel });
            let mut title = String::new();
            md_engine::html::escape(path::stem(&rel), &mut title);
            let html = format!(
                "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n<style>\nbody {{ max-width: 46em; margin: 2em auto; padding: 0 1em; font-family: system-ui, sans-serif; line-height: 1.6 }}\npre {{ background: #f4f4f4; padding: 1em; overflow: auto }}\n.callout {{ border-left: 4px solid #8a5cf5; background: #f6f3ff; padding: .5em 1em; margin: 1em 0 }}\n.callout-title {{ font-weight: 600 }}\n.is-unresolved {{ opacity: .6 }}\ntable {{ border-collapse: collapse }} td, th {{ border: 1px solid #ccc; padding: .3em .6em }}\nmark {{ background: #ffe066 }}\n</style>\n</head>\n<body>\n{body}</body>\n</html>\n"
            );
            let dest = dest
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{}.html", path::stem(&rel))));
            match vault_core::fsio::atomic_write(&dest, html.as_bytes()) {
                Ok(()) => {
                    let _ = writeln!(out, "se escribió {}", dest.display());
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("export: {e}")),
            }
        }
        "md" => {
            // Markdown bundle: the note plus every attachment it embeds or links, flat in one folder.
            let dest = dest
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{}-bundle", path::stem(&rel))));
            let meta = vault_core::meta::extract(&text);
            let mut copied = 0;
            let mut edits = Vec::new();
            for l in &meta.links {
                let Some(t) = store.index.resolve(&l.target, &rel).and_then(|t| store.index.entry(t)) else {
                    continue;
                };
                if t.kind == vault_core::FileKind::Markdown {
                    continue;
                }
                let name = path::file_name(&t.path).to_string();
                if std::fs::create_dir_all(&dest)
                    .and_then(|()| std::fs::copy(root.join(&*t.path), dest.join(&name)))
                    .is_ok()
                {
                    copied += 1;
                    let text = if l.kind == vault_core::meta::LinkKind::Markdown {
                        path::percent_encode(&name)
                    } else {
                        name
                    };
                    edits.push(vault_core::TextEdit {
                        start: l.target_start as usize,
                        end: l.target_end as usize,
                        text,
                    });
                }
            }
            let bundled = vault_core::apply_edits(&text, &edits);
            match vault_core::fsio::atomic_write(&dest.join(path::file_name(&rel)), bundled.as_bytes()) {
                Ok(()) => {
                    let _ = writeln!(out, "se escribió {} (1 nota, {copied} adjuntos)", dest.display());
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("export: {e}")),
            }
        }
        "pdf" => {
            let dest =
                dest.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{}.pdf", path::stem(&rel))));
            let mut app = match App::open(root) {
                Ok(app) => app,
                Err(e) => return fail(&format!("export: {e}")),
            };
            app.open_note(&rel);
            match crate::pdf::export(&mut app, &mut rustsidian_ui::Fonts::new(), &dest) {
                Ok(pages) => {
                    let _ = writeln!(out, "se escribió {} ({pages} páginas)", dest.display());
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("export: {e}")),
            }
        }
        other => fail(&format!("export: formato desconocido `{other}`")),
    }
}

fn doctor(root: &Path, out: &mut dyn std::io::Write) -> Outcome {
    let mut problems = 0;
    let mut line = |ok: bool, msg: String| {
        if !ok {
            problems += 1;
        }
        let _ = writeln!(out, "[{}] {msg}", if ok { " ok " } else { "WARN" });
    };
    match VaultConfig::load(root) {
        Ok(c) => {
            line(
                true,
                format!(
                    "configuración de .obsidian legible (formato de enlaces {:?}, tema {})",
                    c.link_format(),
                    c.css_theme().unwrap_or("default")
                ),
            );
            if let Some(t) = c.css_theme() {
                line(
                    c.dir.join("themes").join(t).join("theme.css").is_file(),
                    format!("el tema `{t}` tiene theme.css"),
                );
            }
            for s in c.enabled_snippets() {
                let p = c.dir.join("snippets").join(format!("{s}.css"));
                match std::fs::read_to_string(&p) {
                    Ok(css) => {
                        let sheet = theme_engine::css::parse(&css);
                        line(
                            sheet.diagnostics.is_empty(),
                            format!(
                                "fragmento `{s}`: {} reglas, {} construcciones no compatibles o no válidas",
                                sheet.rules.len(),
                                sheet.diagnostics.len()
                            ),
                        );
                    }
                    Err(_) => {
                        line(false, format!("el fragmento `{s}` está activado pero falta {}", p.display()))
                    }
                }
            }
        }
        Err(e) => line(false, format!("configuración de .obsidian: {e}")),
    }
    let cache = VaultStore::cache_path(root);
    line(
        !cache.exists() || vault_core::cache::load(&cache).is_some(),
        "el índice guardado es válido (o no existe; se reconstruye automáticamente)".into(),
    );
    match VaultStore::open(root) {
        Ok((mut store, st)) => {
            line(true, format!("{} archivos, {} notas indexadas en {:?}", st.files, st.notes, st.total));
            let bad: Vec<String> = store
                .index
                .notes()
                .filter(|(_, f)| f.meta().is_some_and(|m| m.frontmatter_error.is_some()))
                .map(|(_, f)| f.path.to_string())
                .collect();
            line(
                bad.is_empty(),
                format!(
                    "{} notas con front matter no analizable{}",
                    bad.len(),
                    bad.first().map(|p| format!(" (primera: {p})")).unwrap_or_default()
                ),
            );
            let g = store.index.graph();
            let unresolved: usize = g.unresolved.iter().map(Vec::len).sum();
            line(true, format!("{unresolved} destinos de enlace sin resolver (informativo)"));
            let tmp = root.join(".rustsidian").join("doctor-write-test");
            let writable = vault_core::fsio::atomic_write(&tmp, b"ok").is_ok();
            let _ = std::fs::remove_file(&tmp);
            line(writable, "la bóveda es escribible".into());
        }
        Err(e) => line(false, format!("no se pudo abrir la bóveda: {e}")),
    }
    let fonts = rustsidian_ui::Fonts::new();
    line(fonts.font_count() > 0, format!("{} tipos de letra disponibles", fonts.font_count()));
    let _ = writeln!(
        out,
        "{}",
        if problems == 0 {
            "No se encontraron problemas.".to_string()
        } else {
            format!("{problems} problema(s) encontrado(s).")
        }
    );
    Outcome::Exit(i32::from(problems > 0))
}
