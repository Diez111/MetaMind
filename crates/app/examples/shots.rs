//! Screenshots of every surface, at the sizes the app ships at.
//!
//! `cargo run --release -p metamind --example shots -- [--dir /tmp/shots] [--only calview]`
//!
//! Each view is rendered headless (no window, no GPU) and saved as a PNG, so a design pass can
//! look at what a reader sees instead of guessing from the CSS. The vault is a fixture written to
//! a temporary folder: one note, a board, a drawing and a dashboard, so every surface has content
//! that looks like the real thing.

use metamind_app::{render, App};
use metamind_ui::{Fonts, Pixmap};
use std::path::{Path, PathBuf};

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next();
        }
    }
    None
}

/// The sizes the surfaces ship at: a phone, a tablet and a laptop.
const SIZES: [(u32, u32, &str); 3] = [(390, 844, "phone"), (1024, 768, "tablet"), (1280, 800, "laptop")];

const DRAWING: &str = r##"{"type":"excalidraw","version":2,"elements":[
{"id":"t","type":"text","x":40,"y":30,"width":420,"height":28,"strokeColor":"#1971c2","text":"Taylor y MacLaurin","fontSize":24,"fontFamily":1},
{"id":"b","type":"rectangle","x":40,"y":90,"width":260,"height":140,"strokeColor":"#1e1e1e","backgroundColor":"#a5d8ff","strokeWidth":2,"fillStyle":"solid","seed":7},
{"id":"e","type":"ellipse","x":340,"y":100,"width":180,"height":120,"strokeColor":"#e03131","backgroundColor":"transparent","strokeWidth":2,"fillStyle":"hachure","seed":8}
],"appState":{"viewBackgroundColor":"#ffffff"},"files":{}}"##;

const BOARD: &str = "%% kanban: settings\n{\"kanban-plugin\": \"basic\"} %%\n# Proyecto\n\n## Pendiente\n- [ ] Definir el alcance\n- [ ] Listar dudas\n\n## Haciendo\n- [ ] Escribir el capítulo 1\n\n## Hecho\n- [x] Armar el índice\n";

/// The vault every picture starts from.
fn fixture(root: &Path) {
    let w = |p: &str, t: &str| {
        let abs = root.join(p);
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(abs, t).unwrap();
    };
    w(
        "Home.md",
        "# MetaMind\n\nUna nota de trabajo con **negrita**, `código` y una lista:\n\n- primero\n- segundo\n\nUna fórmula: $P_n(x)=\\sum f^{(k)}(c)/k!\\,(x-c)^k$.\n",
    );
    w("Tablero.md", BOARD);
    w("Pizarra.excalidraw", DRAWING);
    w(
        "Dinero.md",
        "---\ntipo: finanzas\n---\n# Finanzas\n\n## Ingresos\n- 2026-11-01 · sueldo · 1.250.000\n- 2026-10-01 · sueldo · 1.180.000\n\n## Gastos\n- 2026-11-03 · supermercado · 45.200\n- 2026-11-05 · transporte · 12.000\n- 2026-11-10 · alquiler · 240.000\n- 2026-10-04 · supermercado · 38.000\n- 2026-10-10 · alquiler · 240.000\n",
    );
    w(
        "Finanzas.dashboard.md",
        "---\ntipo: dashboard\n---\n# Tablero\n\n## Hoy\n- [ ] Revisar gastos\n\n## Semana\n- [x] Pagar alquiler\n",
    );
    w(
        ".metamind/calendar/events.json",
        r#"{"version":1,"events":[
{"id":"a","title":"Parcial AM I","start":"2026-11-27","startMinute":540,"endMinute":660,"color":"violeta"},
{"id":"b","title":"Práctica 8","start":"2026-11-24","startMinute":600,"endMinute":720,"color":"azul"},
{"id":"c","title":"Feriado","start":"2026-11-30","color":"verde"}]}"#,
    );
}

fn main() {
    let dir = PathBuf::from(arg("--dir").unwrap_or_else(|| "/tmp/metamind-shots".into()));
    std::fs::create_dir_all(&dir).expect("the output folder");
    let vault = dir.join("vault");
    let _ = std::fs::remove_dir_all(&vault);
    fixture(&vault);
    let only = arg("--only");
    for (w, h, label) in SIZES {
        for (name, commands) in views() {
            if only.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
                continue;
            }
            // Every picture starts from the same vault, so a command cannot leak into the next.
            let mut app = App::open(&vault).expect("the vault opens");
            app.open_note("Home");
            let mut fonts = Fonts::new();
            for command in commands {
                app.run(command);
            }
            let mut pm = Pixmap::new(w, h).expect("a pixmap");
            // Two frames: the first lays the tree out, the second is the one a reader sees.
            render(&mut app, &mut fonts, &mut pm, 1.0);
            render(&mut app, &mut fonts, &mut pm, 1.0);
            let path = dir.join(format!("{name}-{label}.png"));
            if let Err(e) = pm.save_png(&path) {
                eprintln!("{}: {e}", path.display());
            } else {
                println!("{} ({w}x{h})", path.display());
            }
        }
    }
}

/// Every surface worth a picture, as the commands that put the window in it.
fn views() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("nota", vec![]),
        ("calendario", vec!["calendar:open"]),
        ("calendario-editor", vec!["calendar:open", "cal:new"]),
        ("calview-mes", vec!["calview:open"]),
        ("calview-semana", vec!["calview:open", "calview:mode:week"]),
        ("calview-dia", vec!["calview:open", "calview:mode:day"]),
        ("calview-agenda", vec!["calview:open", "calview:mode:agenda"]),
        ("tablero", vec!["open:Tablero.md"]),
        ("tablero-nuevo", vec!["kanban:new"]),
        ("excalidraw", vec!["open:Pizarra.excalidraw"]),
        ("excalidraw-estilo", vec!["open:Pizarra.excalidraw", "exc:style"]),
        ("ajustes", vec!["app:open-settings"]),
        ("ia", vec!["ai:open"]),
        ("dashboard", vec!["open:Finanzas.dashboard.md"]),
        ("finanzas", vec!["open:Dinero.md"]),
    ]
}
