//! Query results over the fixture vault: the notes in `tests/fixtures`, parsed with the same
//! metadata pass the app uses.

use dataview_core::{markdown, parse, table, Cell, Opts};
use std::fs;
use std::path::Path;
use vault_core::path::{self, FileKind};
use vault_core::Vault;

/// Every fixture note gets this stamp, so `file.mtime` reads the same on every machine.
/// 2024-03-04T00:00:00Z in nanoseconds.
const MTIME: i64 = 1_709_510_400_000_000_000;

/// A vault built from a folder of notes, as the app's store builds its index.
fn load() -> Vault {
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
            let text = fs::read_to_string(&abs).unwrap();
            let rel = abs.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let meta = vault_core::meta::extract(&text);
            v.upsert(&rel, MTIME, text.len() as u64, Some(meta));
        }
    }
    let mut v = Vault::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    walk(&root, &root, &mut v);
    v
}

fn run(src: &str) -> String {
    render(src, Opts::default())
}

fn render(src: &str, opts: Opts) -> String {
    markdown(&table(&load(), &parse(src).unwrap(), opts))
}

fn files(t: &dataview_core::Table) -> Vec<&str> {
    t.rows.iter().map(|r| r.path.as_str()).collect()
}

#[test]
fn a_folder_query_builds_the_table_markdown() {
    let got = run("TABLE tipo AS \"Tipo\", unidad AS \"Unidad\", file.mtime AS \"Editado\"\n\
         FROM \"Facultad/Materias\"\n\
         WHERE tipo != \"mapa\"\n\
         SORT unidad ASC, file.name ASC");
    assert_eq!(
        got,
        "| File | Tipo | Unidad | Editado |\n\
         | --- | --- | --- | --- |\n\
         | [[Facultad/Materias/Unidades/U1.md\\|U1]] | unidad | 1 | 2024-03-04 |\n\
         | [[Facultad/Materias/Materia Uno.md\\|Materia Uno]] | materia | 2 | 2024-03-04 |\n\
         | [[Facultad/Materias/Unidades/U10.md\\|U10]] | unidad | 10 | 2024-03-04 |\n\
         | [[Facultad/Materias/Materia Dos.md\\|Materia Dos]] | materia |  | 2024-03-04 |\n\
         | [[Facultad/Materias/Unidades/Sin-Fecha.md\\|Sin-Fecha]] | unidad |  | 2024-03-04 |\n"
    );
}

#[test]
fn from_is_recursive_and_skips_other_folders() {
    let q = parse("TABLE x FROM \"Vida\"").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        [
            "Vida/Diario/2026-11-10.md",
            "Vida/Diario/2026-11-12.md",
            "Vida/Diario/Sin-Datos.md",
            "Vida/Salud/Mensual/2026-11.md"
        ]
    );
    let q = parse("TABLE x FROM \"vida/diario/\"").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        ["Vida/Diario/2026-11-10.md", "Vida/Diario/2026-11-12.md", "Vida/Diario/Sin-Datos.md"]
    );
    let q = parse("TABLE x FROM \"Vida/Diario\" WHERE tipo = \"diario\"").unwrap();
    assert_eq!(table(&load(), &q, Opts::default()).rows.len(), 3);
    // A folder that only shares a prefix is not included.
    let q = parse("TABLE x FROM \"Vid\"").unwrap();
    assert!(table(&load(), &q, Opts::default()).rows.is_empty());
}

#[test]
fn dates_compare_by_day_and_missing_dates_drop_out() {
    let got = run("TABLE sueno AS \"Sueño\", peso AS \"Peso\"\n\
         FROM \"Vida/Diario\"\n\
         WHERE tipo = \"diario\" AND fecha >= date(2026-11-11)\n\
         SORT fecha ASC");
    assert_eq!(
        got,
        "| File | Sueño | Peso |\n\
         | --- | --- | --- |\n\
         | [[Vida/Diario/2026-11-12.md\\|2026-11-12]] | 8 | 79.8 |\n"
    );
    let q = parse("TABLE x FROM \"Vida/Diario\" WHERE fecha < date(2026-11-11)").unwrap();
    assert_eq!(files(&table(&load(), &q, Opts::default())), ["Vida/Diario/2026-11-10.md"]);
    let q = parse("TABLE x FROM \"Vida/Diario\" WHERE fecha < date(2026-11-11) AND sueno > 7").unwrap();
    assert_eq!(files(&table(&load(), &q, Opts::default())), ["Vida/Diario/2026-11-10.md"]);
}

#[test]
fn null_is_the_test_for_a_missing_key() {
    let q = parse("TABLE x FROM \"Facultad/Materias\" WHERE tipo != null").unwrap();
    assert_eq!(table(&load(), &q, Opts::default()).rows.len(), 6);
    let q = parse("TABLE x FROM \"Facultad/Materias\" WHERE unidad = null").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        [
            "Facultad/Materias/Mapa.md",
            "Facultad/Materias/Materia Dos.md",
            "Facultad/Materias/Unidades/Sin-Fecha.md"
        ]
    );
    let q = parse("TABLE x FROM \"Facultad/Materias\" WHERE unidad != null").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        [
            "Facultad/Materias/Materia Uno.md",
            "Facultad/Materias/Unidades/U1.md",
            "Facultad/Materias/Unidades/U10.md"
        ]
    );
}

#[test]
fn without_id_drops_the_link_column() {
    let got = run("TABLE WITHOUT ID tipo AS \"Tipo\", unidad AS \"Unidad\"\n\
         FROM \"anon\"\n\
         WHERE tipo != \"materia\"\n\
         SORT unidad ASC, file.name ASC");
    assert_eq!(got, "| Tipo | Unidad |\n| --- | --- |\n| unidad | 1 |\n| diario |  |\n");
    let t = table(&load(), &parse("TABLE WITHOUT ID tipo").unwrap(), Opts::default());
    assert_eq!(t.headers, ["tipo"]);
    assert!(t.rows.iter().all(|r| r.cells.len() == 1));
}

#[test]
fn file_link_is_the_link_column_and_headers_default_to_the_field() {
    let t =
        table(&load(), &parse("TABLE WITHOUT ID file.link, tipo FROM \"anon\"").unwrap(), Opts::default());
    assert_eq!(t.headers, ["file.link", "tipo"]);
    let row = &t.rows[0];
    assert_eq!(row.cells[0], Cell { text: "2026-11-11".into(), link: Some("anon/2026-11-11.md".into()) });
    let row = t.rows.iter().find(|r| r.path == "anon/Unidad Ejemplo.md").unwrap();
    assert_eq!(row.cells[0], Cell { text: "Unidad Ejemplo".into(), link: Some(row.path.clone()) });
}

#[test]
fn lists_render_as_their_items() {
    let t = table(
        &load(),
        &parse(
            "TABLE WITHOUT ID tags FROM \"Facultad/Materias\" WHERE materia = \"Materia Uno\" SORT file.name",
        )
        .unwrap(),
        Opts::default(),
    );
    let cell = |name: &str| {
        let row = t.rows.iter().find(|r| r.path.ends_with(name)).unwrap();
        row.cells[0].text.clone()
    };
    assert_eq!(cell("Materia Uno.md"), "materia/uno, anio/1, tipo/materia");
    assert_eq!(cell("Sin-Fecha.md"), "");
}

#[test]
fn sort_is_numeric_case_insensitive_and_ties_by_path() {
    let q = parse("TABLE unidad FROM \"Facultad/Materias\" WHERE tipo = \"unidad\" SORT unidad ASC").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        [
            "Facultad/Materias/Unidades/U1.md",
            "Facultad/Materias/Unidades/U10.md",
            "Facultad/Materias/Unidades/Sin-Fecha.md"
        ]
    );
    let q =
        parse("TABLE unidad FROM \"Facultad/Materias\" WHERE tipo = \"unidad\" SORT unidad DESC").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        [
            "Facultad/Materias/Unidades/U10.md",
            "Facultad/Materias/Unidades/U1.md",
            "Facultad/Materias/Unidades/Sin-Fecha.md"
        ]
    );
    let q = parse("TABLE tipo FROM \"anon\" SORT tipo DESC").unwrap();
    assert_eq!(
        files(&table(&load(), &q, Opts::default())),
        ["anon/Unidad Ejemplo.md", "anon/Materia Ejemplo.md", "anon/2026-11-11.md"]
    );
}

#[test]
fn mtime_is_a_date_in_the_local_zone() {
    let q = parse("TABLE WITHOUT ID file.mtime FROM \"anon\" WHERE tipo = \"unidad\"").unwrap();
    let t = table(&load(), &q, Opts::default());
    assert_eq!(t.rows[0].cells[0].text, "2024-03-04");
    let west = Opts { utc_offset: -5 * 3600 };
    assert_eq!(table(&load(), &q, west).rows[0].cells[0].text, "2024-03-03");
    let east = Opts { utc_offset: 2 * 3600 };
    assert_eq!(table(&load(), &q, east).rows[0].cells[0].text, "2024-03-04");
}

#[test]
fn a_query_that_matches_nothing_keeps_its_headers() {
    let got = run("TABLE tipo AS \"Tipo\" FROM \"anon\" WHERE tipo = \"no existe\"");
    assert_eq!(got, "| File | Tipo |\n| --- | --- |\n");
}

#[test]
fn the_reference_queries_are_the_ones_this_vault_uses() {
    // The three shapes the notes in the reference vault are written with.
    for src in [
        "TABLE tipo AS \"Tipo\", unidad AS \"Unidad\", file.mtime AS \"Editado\"\nFROM \"Facultad/Materias/Unidades\"\nWHERE tipo != \"materia\"\nSORT unidad ASC, file.name ASC",
        "TABLE tipo AS \"Tipo\", materia AS \"Materia\", file.mtime AS \"Editado\"\nFROM \"Facultad/Materias\"\nWHERE tipo != \"anio\"\nSORT materia ASC, tipo ASC, file.name ASC",
        "TABLE sueno AS \"Sueño\", peso AS \"Peso\"\nFROM \"Vida/Diario\"\nWHERE tipo = \"diario\" AND fecha >= date(2026-11-11)\nSORT fecha ASC",
        "TABLE anio AS \"Año\", materia AS \"Materia\", tipo AS \"Tipo\"\nFROM \"Facultad/Materias\"\nWHERE tipo != \"mapa\" AND tipo != null\nSORT anio ASC, materia ASC, file.name ASC",
        "TABLE WITHOUT ID file.name AS \"Nota\", tipo AS \"Tipo\"\nFROM \"anon\"\nSORT file.name ASC",
    ] {
        let q = parse(src).unwrap();
        assert_eq!(q.to_string(), src.trim(), "the canonical form keeps the query as written");
        let t = table(&load(), &q, Opts::default());
        assert_eq!(t.headers.len(), t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(t.headers.len()));
    }
}
