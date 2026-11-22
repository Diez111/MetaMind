//! A Kanban board end to end: the note that opens as a board, the three columns of the plugin's
//! format, a card dragged from one column to another with the file on disk following it, the
//! checkbox that writes `[x]`, a card added at the foot of a column and one deleted from the
//! keyboard — every step through the public input path (`click`, `drag`, `key`, `text_input`,
//! `run`), the way a mouse and a keyboard do it.
//!
//! The second half of the file is the roundtrip gate: the markdown the plugin writes is read into
//! a board and written back byte for byte, and the board of a file that is already canonical is
//! stable under any number of reads and writes.

use metamind_app::kanbanview::Board;
use metamind_app::{render, App, Frame, Key};
use metamind_ui::{Cmd, Fonts, Pixmap, Rect};
use std::fs;

/// The board of the brief: three columns, cards with a tag and a date, one done.
const BOARD: &str = "%% kanban: settings\n\
{\"kanban-plugin\": \"basic\"} %%\n\
# Tablero de areas\n\
\n\
## Pendiente\n\
- [ ] Primera tarea\n\
- [ ] Segunda @{2026-11-25} tarea con fecha #area\n\
\n\
## Haciendo\n\
- [ ] En curso\n\
\n\
## Hecho\n\
- [x] Terminada\n";

const NOTA: &str = "# Otra nota\n\nno es un tablero\n";

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Tablero.md"), BOARD).unwrap();
    fs::write(d.path().join("Otra.md"), NOTA).unwrap();
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn frame(app: &mut App, fonts: &mut Fonts) -> Frame {
    let mut pm = Pixmap::new(1200, 900).unwrap();
    render(app, fonts, &mut pm, 1.0)
}

fn on_disk(d: &tempfile::TempDir) -> String {
    fs::read_to_string(d.path().join("Tablero.md")).unwrap()
}

/// The card on disk under the heading `col`, in order, as `(done, text)`.
fn column_of(text: &str, col: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            inside = name.trim() == col;
            continue;
        }
        if !inside {
            continue;
        }
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("- [") {
            let done = rest.starts_with('x');
            let text = rest.splitn(2, "] ").nth(1).unwrap_or("").to_string();
            out.push((done, text));
        }
    }
    out
}

/// An application with the board open, and the frame it painted.
fn open_board() -> (tempfile::TempDir, App, Fonts, Frame) {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    app.open_note("Tablero.md");
    assert!(app.kanban_active(), "a note with the kanban settings opens as a board");
    let f = frame(&mut app, &mut fonts);
    (d, app, fonts, f)
}

fn centre(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// The board as the last frame painted it: the texts of every command, in order.
fn painted(f: &Frame) -> Vec<String> {
    let node = f.info.kanban.expect("the board paints into the pane");
    f.dom.nodes[node]
        .draw
        .iter()
        .filter_map(|c| match c {
            Cmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The geometry of the card at (column, index), on the last frame.
fn card(f: &App, col: usize, idx: usize) -> Rect {
    app_kanban(f).card_rect(col, idx).unwrap_or_else(|| panic!("the card {col}/{idx} is on the board"))
}

fn app_kanban(app: &App) -> &metamind_app::kanbanview::KanbanView {
    app.kanban_view.as_ref().expect("the board is open")
}

#[test]
fn the_tree_opens_the_board_of_a_note_and_the_paint_shows_its_columns() {
    let (_d, app, _fonts, f) = open_board();
    assert!(app.kanban_active(), "the board has the pane");
    assert!(!app.exc_active());
    assert!(app.tab().is_none(), "the board is not a text tab of the note it came from");
    let texts = painted(&f);
    for expected in ["Pendiente", "Haciendo", "Hecho", "Primera tarea", "+ Añadir tarjeta"] {
        assert!(
            texts.iter().any(|t| t.contains(expected)),
            "the board paints {expected:?}: {texts:?}"
        );
    }
    // The three columns are side by side, a fixed width apart, and the cards inside them.
    let c0 = app_kanban(&app).column_rect(0).unwrap();
    let c1 = app_kanban(&app).column_rect(1).unwrap();
    assert!((c1.x - c0.x - 292.0).abs() < 0.5, "the columns are 280px + a 12px gap apart: {c0:?} {c1:?}");
    let first = card(&app, 0, 0);
    let second = card(&app, 0, 1);
    assert!(second.y > first.y + first.h - 1.0, "the cards stack: {first:?} {second:?}");
    assert!(first.x >= c0.x && first.x + first.w <= c0.x + c0.w, "a card stays inside its column");

    // The pane takes the whole height: the columns scroll rather than clip their cards.
    let pane = f.dom.nodes[f.info.kanban.unwrap()].rect;
    assert!(c0.h <= pane.h, "a column is as tall as its content, not taller than the pane");
}

#[test]
fn a_card_dragged_to_another_column_is_written_to_the_file_where_it_lands() {
    let (d, mut app, mut fonts, f) = open_board();
    assert_eq!(column_of(&on_disk(&d), "Pendiente").len(), 2);
    assert_eq!(column_of(&on_disk(&d), "Haciendo").len(), 1);

    // Grab the second card of Pendiente and drop it at the foot of Haciendo.
    let from = card(&app, 0, 1);
    let (gx, gy) = (from.x + from.w * 0.6, from.y + from.h / 2.0);
    app.click(&f, &mut fonts, gx, gy, false);
    assert_eq!(app_kanban(&app).selected, Some(app_kanban(&app).board.columns[0].cards[1].id));
    let to = app_kanban(&app).column_rect(1).unwrap();
    let (tx, ty) = (to.x + to.w / 2.0, to.y + to.h - 4.0);
    app.drag(&f, &mut fonts, gx + 10.0, gy + 10.0);
    app.drag(&f, &mut fonts, tx, ty);
    app.release();

    // The file on disk follows the drop, with no save command in between.
    let text = on_disk(&d);
    assert_eq!(column_of(&text, "Pendiente"), vec![(false, "Primera tarea".to_string())], "{text}");
    let done = column_of(&text, "Haciendo");
    assert_eq!(done.len(), 2, "{text}");
    assert_eq!(done[1].1, "Segunda @{2026-11-25} tarea con fecha #area", "the card landed last");

    // And the next frame paints it there.
    let f = frame(&mut app, &mut fonts);
    let pend = app_kanban(&app).column_rect(0).unwrap();
    let terc = app_kanban(&app).column_rect(2).unwrap();
    assert!(
        app_kanban(&app).card_rect(1, 1).is_some(),
        "the moved card is in Haciendo: {:?} {:?}",
        pend,
        terc
    );
    let f2 = frame(&mut app, &mut fonts);
    assert!(painted(&f2).iter().any(|t| t.contains("Segunda")), "the card still paints");
    let _ = f;
}

#[test]
fn the_checkbox_of_a_card_writes_x_and_takes_it_back() {
    let (d, mut app, mut fonts, f) = open_board();
    let check = app_kanban(&app).check_rect(0, 0).unwrap();
    let (x, y) = centre(check);
    app.click(&f, &mut fonts, x, y, false);
    assert!(column_of(&on_disk(&d), "Pendiente")[0].0, "the first card is done on disk: {}", on_disk(&d));

    let f = frame(&mut app, &mut fonts);
    let check = app_kanban(&app).check_rect(0, 0).unwrap();
    let (x, y) = centre(check);
    app.click(&f, &mut fonts, x, y, false);
    assert!(!column_of(&on_disk(&d), "Pendiente")[0].0, "and pressing again clears it");

    // A done card is painted faint and struck through: the file, the view.
    let f = frame(&mut app, &mut fonts);
    assert!(painted(&f).iter().any(|t| t.contains("Terminada")), "the done card is on the board");
}

#[test]
fn a_card_typed_at_the_foot_of_a_column_and_delete_removes_one() {
    let (d, mut app, mut fonts, f) = open_board();
    let add = app_kanban(&app).add_rect(2).unwrap();
    let (x, y) = centre(add);
    app.click(&f, &mut fonts, x, y, false);
    assert!(app_kanban(&app).is_editing(), "the press starts a card to type into");
    for c in "Revisar el PR".chars() {
        app.text_input(&c.to_string());
    }
    app.key(key("Enter"));
    assert!(!app_kanban(&app).is_editing(), "Enter commits the text");
    let text = on_disk(&d);
    assert!(
        column_of(&text, "Hecho").iter().any(|(_, t)| t == "Revisar el PR"),
        "the new card is in the file: {text}"
    );

    // Select it (the last card of the last column) and press Delete.
    let f = frame(&mut app, &mut fonts);
    let last = app_kanban(&app).board.columns[2].cards.len() - 1;
    let r = app_kanban(&app).card_rect(2, last).unwrap();
    let (x, y) = (r.x + r.w * 0.6, r.y + r.h / 2.0);
    app.click(&f, &mut fonts, x, y, false);
    app.key(key("Delete"));
    let text = on_disk(&d);
    assert!(
        !column_of(&text, "Hecho").iter().any(|(_, t)| t == "Revisar el PR"),
        "the card is gone from the file: {text}"
    );
    assert_eq!(column_of(&text, "Hecho"), vec![(true, "Terminada".to_string())], "{text}");
}

#[test]
fn the_code_view_writes_the_same_markdown_the_plugin_reads() {
    let (d, mut app, mut fonts, _f) = open_board();
    app.run("kanban:source");
    let f = frame(&mut app, &mut fonts);
    let texts = painted(&f);
    assert!(
        texts.iter().any(|t| t.contains("%% kanban: settings")),
        "the code view shows the file: {texts:?}"
    );
    assert!(texts.iter().any(|t| t.contains("## Pendiente")), "with its columns: {texts:?}");

    // Leaving the code view, the board is back and the file untouched.
    let pane = f.dom.nodes[f.info.kanban.unwrap()].rect;
    let (x, y) = (pane.x + pane.w / 2.0, pane.y + pane.h / 2.0);
    app.click(&f, &mut fonts, x, y, false);
    app.click(&f, &mut fonts, x, y, false);
    let f = frame(&mut app, &mut fonts);
    assert!(painted(&f).iter().any(|t| t.contains("Pendiente")), "the columns are back");
    assert_eq!(on_disk(&d), BOARD, "browsing wrote nothing");
}

#[test]
fn a_column_added_from_the_toolbar_is_renamed_inline_and_saved() {
    let (d, mut app, _fonts, _f) = open_board();
    app.run("kanban:add-column");
    assert!(app_kanban(&app).is_editing(), "the new column starts with its name being typed");
    for c in "Bloqueado".chars() {
        app.text_input(&c.to_string());
    }
    app.key(key("Enter"));
    let text = on_disk(&d);
    assert!(text.contains("## Bloqueado"), "the new column is in the file: {text}");
    assert!(text.ends_with('\n'), "the file keeps its final newline");
}

#[test]
fn the_markdown_of_a_board_roundtrips_through_the_model() {
    // The file the plugin writes is the file the board writes back.
    let board = Board::parse(BOARD);
    assert_eq!(board.title.as_deref(), Some("Tablero de areas"));
    assert_eq!(board.columns.len(), 3);
    assert_eq!(board.columns[0].name, "Pendiente");
    assert_eq!(board.columns[0].cards.len(), 2);
    assert!(board.columns[2].cards[0].done);
    assert_eq!(board.render(), BOARD, "read and written, the file is the same");

    // A file that needs normalising is stable after the first write.
    let messy = "%% kanban: settings\n\
{\"kanban-plugin\": \"basic\"} %%\n\
\n\
# Sin titulo de columna todavia\n\
\n\
* [X] hecha con asterisco\n\
linea suelta que no es tarjeta\n\
\n\
## Otra\n\
- una tarjeta sin casilla\n";
    let once = Board::parse(messy).render();
    let twice = Board::parse(&once).render();
    assert_eq!(once, twice, "a second write changes nothing:\n{once}");
    // `* [X]` becomes the canonical `- [x]`, and a plain item stays a plain item.
    assert!(once.contains("- una tarjeta sin casilla"), "a plain item stays plain: {once}");
    // A list item above the first column is not part of a board: it is kept as it came.
    assert!(once.contains("* [X] hecha con asterisco"), "{once}");
    assert!(once.contains("linea suelta que no es tarjeta"), "the line under the cards is kept");
}

#[test]
fn a_note_without_the_settings_is_still_a_note() {
    let (_d, mut app, mut fonts) = {
        let d = vault();
        let app = App::open(d.path()).unwrap();
        (d, app, Fonts::new())
    };
    app.open_note("Otra.md");
    assert!(!app.kanban_active(), "a plain note does not become a board");
    let f = frame(&mut app, &mut fonts);
    assert!(f.info.kanban.is_none());
}

#[test]
fn a_new_board_is_the_template_of_three_columns_and_opens_ready_to_fill() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    app.run("kanban:new");
    assert!(app.kanban_active(), "the new board opens as a board");
    let board = app_kanban(&app);
    assert_eq!(board.board.columns.len(), 3);
    assert_eq!(board.path, "Tablero 2.md", "the name of a board already taken is not reused");
    let f = frame(&mut app, &mut fonts);
    let texts = painted(&f);
    for expected in ["Pendiente", "Haciendo", "Hecho"] {
        assert!(texts.iter().any(|t| t.contains(expected)), "the template paints {expected}: {texts:?}");
    }
    // The file on disk is the template the plugin reads back.
    let text = fs::read_to_string(d.path().join("Tablero 2.md")).unwrap();
    assert!(text.starts_with("%% kanban: settings"), "{text}");
    assert_eq!(Board::parse(&text).render(), text, "the template roundtrips");
}

#[test]
fn a_card_dragged_within_its_column_changes_the_order_on_disk() {
    let (d, mut app, mut fonts, f) = open_board();
    let first = card(&app, 0, 0);
    let second = card(&app, 0, 1);
    let (gx, gy) = (first.x + first.w * 0.6, first.y + first.h / 2.0);
    app.click(&f, &mut fonts, gx, gy, false);
    // Below the middle of the second card: the first card goes after it.
    let (tx, ty) = (second.x + second.w * 0.6, second.y + second.h - 1.0);
    app.drag(&f, &mut fonts, gx + 8.0, gy + 8.0);
    app.drag(&f, &mut fonts, tx, ty);
    app.release();
    let text = on_disk(&d);
    let pend = column_of(&text, "Pendiente");
    assert_eq!(pend[0].1, "Segunda @{2026-11-25} tarea con fecha #area", "{text}");
    assert_eq!(pend[1].1, "Primera tarea", "{text}");
}
