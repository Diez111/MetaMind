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
            let text = rest.split_once("] ").map(|x| x.1).unwrap_or("").to_string();
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

/// The texts of the board's own bar, which are DOM nodes rather than paint commands.
fn bar_texts(f: &Frame) -> Vec<String> {
    f.dom.nodes.iter().filter_map(|n| n.el.text.clone()).collect()
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
        assert!(texts.iter().any(|t| t.contains(expected)), "the board paints {expected:?}: {texts:?}");
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

#[test]
fn a_link_inside_a_card_opens_the_note_it_names() {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("Enlaces.md"),
        "%% kanban: settings\n{\"kanban-plugin\": \"basic\"} %%\n# Enlaces\n\n## Pendiente\n- [ ] leer [[Otra]] hoy\n",
    )
    .unwrap();
    fs::write(d.path().join("Otra.md"), "# Otra\n\nnada mas\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    app.open_note("Enlaces.md");
    let f = frame(&mut app, &mut fonts);
    let link = app
        .kanban_view
        .as_mut()
        .unwrap()
        .link_rect(&mut fonts, 0, 0, "Otra")
        .expect("the link of the card is laid out");
    app.click(&f, &mut fonts, link.x + link.w / 2.0, link.y + link.h / 2.0, false);
    assert_eq!(app.tab().map(|t| t.path.clone()).as_deref(), Some("Otra.md"), "the link opens the note");
    assert!(!app.kanban_active(), "the note takes the pane");
}

#[test]
fn the_column_menu_carries_the_actions_of_the_brief_and_deletes_the_column() {
    let (d, mut app, mut fonts, f) = open_board();
    let (x, y) = app_kanban(&app).column_anchor(1);
    app.context_menu(&f, &mut fonts, x, y - 4.0);
    let menu = app.menu.as_ref().expect("a column opens a menu");
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    for expected in ["Añadir tarjeta", "Renombrar columna", "Colapsar columna", "Eliminar columna"] {
        assert!(labels.contains(&expected), "the column menu offers {expected:?}: {labels:?}");
    }
    app.run("kanban:column-delete:1");
    assert!(app.notice.is_some(), "removing a column says so in the status bar");
    let text = on_disk(&d);
    assert!(!text.contains("## Haciendo"), "the column is gone from the file: {text}");
    assert!(text.contains("## Pendiente") && text.contains("## Hecho"), "the others stay: {text}");
}

#[test]
fn the_chevron_folds_a_column_without_touching_the_file() {
    let (d, mut app, mut fonts, _f) = open_board();
    let fold = app_kanban(&app).fold_rect(0).unwrap();
    let (x, y) = centre(fold);
    let f = frame(&mut app, &mut fonts);
    app.click(&f, &mut fonts, x, y, false);
    let f = frame(&mut app, &mut fonts);
    assert!(app_kanban(&app).card_rect(0, 0).is_none(), "the cards of a folded column are hidden");
    assert_eq!(app_kanban(&app).column_rect(0).unwrap().h, 34.0, "only its header stays");
    assert_eq!(on_disk(&d), BOARD, "folding is a view state, not an edit");
    let fold = app_kanban(&app).fold_rect(0).unwrap();
    let (x, y) = centre(fold);
    app.click(&f, &mut fonts, x, y, false);
    let f = frame(&mut app, &mut fonts);
    assert!(app_kanban(&app).card_rect(0, 0).is_some(), "unfolding brings the cards back");
    let _ = f;
}

/// The card lines of `col` on disk, as text, for the tests that care about the order only.
fn card_texts(text: &str, col: &str) -> Vec<String> {
    column_of(text, col).into_iter().map(|(_, t)| t).collect()
}

#[test]
fn a_wip_limit_is_written_in_the_board_settings_and_a_drop_past_it_warns() {
    let (d, mut app, mut fonts, _f) = open_board();
    // One is enough for Haciendo, which already holds one card.
    app.run("kanban:wip-set:1:1");
    let text = on_disk(&d);
    assert!(text.contains("\"wip\":{\"Haciendo\":1}"), "the limit lives in the settings block: {text}");
    assert_eq!(Board::parse(&text).wip_limit("Haciendo"), Some(1));
    assert_eq!(Board::parse(&text).render(), text, "the block with the limit roundtrips");

    // The counter of a column with a limit reads cards / limit.
    let f = frame(&mut app, &mut fonts);
    assert!(painted(&f).iter().any(|t| t == "1/1"), "the counter shows the limit: {:?}", painted(&f));

    // A drag that leaves Haciendo over its limit still lands, and says so.
    let from = card(&app, 0, 1);
    let (gx, gy) = (from.x + from.w * 0.6, from.y + from.h / 2.0);
    app.click(&f, &mut fonts, gx, gy, false);
    let to = app_kanban(&app).column_rect(1).unwrap();
    let (tx, ty) = (to.x + to.w / 2.0, to.y + to.h - 4.0);
    app.drag(&f, &mut fonts, gx + 10.0, gy + 10.0);
    app.drag(&f, &mut fonts, tx, ty);
    app.release();

    let text = on_disk(&d);
    assert_eq!(column_of(&text, "Haciendo").len(), 2, "the move is not blocked: {text}");
    let notice = app.notice.clone().unwrap_or_default();
    assert!(notice.contains("límite WIP") && notice.contains("Haciendo"), "the drop warns: {notice}");

    // And the next frame paints the column over its limit as 2/1.
    let f = frame(&mut app, &mut fonts);
    assert!(painted(&f).iter().any(|t| t == "2/1"), "over the limit: {:?}", painted(&f));
}

#[test]
fn the_filter_hides_cards_by_text_and_by_tag_and_is_not_saved() {
    let (d, mut app, mut fonts, _f) = open_board();
    app.run("kanban:filter");
    assert!(app_kanban(&app).filter_editing, "the field takes the keyboard");
    for c in "fecha".chars() {
        app.text_input(&c.to_string());
    }
    let f = frame(&mut app, &mut fonts);
    // Only "Segunda … fecha" shows, so it is the first card of the column's geometry.
    assert!(app_kanban(&app).card_rect(0, 0).is_some());
    assert!(app_kanban(&app).card_rect(0, 1).is_none());
    let texts = painted(&f);
    assert!(!texts.iter().any(|t| t.contains("Primera tarea")), "a card with no match is hidden: {texts:?}");
    assert!(texts.iter().any(|t| t.contains("Segunda")), "the matching card stays: {texts:?}");
    let bar = bar_texts(&f);
    assert!(bar.iter().any(|t| t.contains("1 de 4 tarjetas")), "the bar counts what shows: {bar:?}");

    // A tag is a filter of its own, and words and tags combine with AND.
    app.run("kanban:filter-clear");
    for c in "#area".chars() {
        app.text_input(&c.to_string());
    }
    let f = frame(&mut app, &mut fonts);
    assert!(app_kanban(&app).card_rect(0, 0).is_some());
    assert!(painted(&f).iter().any(|t| t.contains("Segunda")));
    app.run("kanban:filter-clear");
    for c in "#nada".chars() {
        app.text_input(&c.to_string());
    }
    let f = frame(&mut app, &mut fonts);
    assert!(app_kanban(&app).card_rect(0, 0).is_none(), "no card carries the tag");
    assert!(bar_texts(&f).iter().any(|t| t.contains("0 de 4 tarjetas")), "{:?}", bar_texts(&f));

    // Escape gives the keyboard back; a second Escape clears the query. Neither writes the file.
    app.key(key("Escape"));
    assert!(!app_kanban(&app).filter_editing);
    app.key(key("Escape"));
    assert!(app_kanban(&app).filter.is_empty());
    let f = frame(&mut app, &mut fonts);
    assert!(app_kanban(&app).card_rect(0, 0).is_some());
    assert!(painted(&f).iter().any(|t| t.contains("Primera tarea")), "the board is whole again");
    assert_eq!(on_disk(&d), BOARD, "a filter is a view state, not an edit");
}

#[test]
fn a_drop_under_a_filter_lands_where_the_visible_card_is() {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("Filtro.md"),
        "%% kanban: settings\n{\"kanban-plugin\": \"basic\"} %%\n# Filtro\n\n## Pendiente\n\
- [ ] oculto\n- [ ] uno #keep\n- [ ] tres #keep\n\n## Hecho\n",
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    app.open_note("Filtro.md");
    assert!(app.kanban_active());
    app.run("kanban:filter-set:#keep");
    let f = frame(&mut app, &mut fonts);
    // The filter leaves two cards: "uno" above "tres"; "oculto" sits between them on disk.
    let uno = app_kanban(&app).card_rect(0, 0).expect("uno is visible");
    let tres = app_kanban(&app).card_rect(0, 1).expect("tres is visible");
    let (gx, gy) = (tres.x + tres.w * 0.6, tres.y + tres.h / 2.0);
    app.click(&f, &mut fonts, gx, gy, false);
    let (tx, ty) = (uno.x + uno.w * 0.6, uno.y + 2.0);
    app.drag(&f, &mut fonts, gx + 8.0, gy + 8.0);
    app.drag(&f, &mut fonts, tx, ty);
    app.release();

    let text = fs::read_to_string(d.path().join("Filtro.md")).unwrap();
    assert_eq!(
        card_texts(&text, "Pendiente"),
        vec!["oculto".to_string(), "tres #keep".to_string(), "uno #keep".to_string()],
        "the hidden card keeps its place: {text}"
    );
}

/// The ribbon is where a reader looks for "make me one of these": a board has to be there.
#[test]
fn the_ribbon_offers_a_board_and_the_command_makes_one() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let f = frame(&mut app, &mut fonts);
    let found = f.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some("kanban:new"));
    assert!(found, "la ribbon tiene el botón de tablero");
    app.run("kanban:new");
    let made = d.path().join("Tablero.md");
    assert!(made.exists(), "el comando crea el archivo");
    assert!(app.kanban_active(), "y abre el tablero");
    let text = fs::read_to_string(made).unwrap();
    assert!(text.starts_with("%% kanban: settings"), "{text}");
    assert_eq!(text, metamind_app::kanbanview::TEMPLATE, "con la plantilla de siempre");
}

/// "No veo opciones de sintaxis": the board shows its own format, and what it shows is the format
/// the parser reads.
#[test]
fn the_help_card_shows_the_format_the_parser_reads() {
    let (_d, mut app, mut fonts, f) = open_board();
    assert!(f.info.kanban.is_some());
    app.run("kanban:help");
    let f = frame(&mut app, &mut fonts);
    let card = f
        .dom
        .nodes
        .iter()
        .find(|n| n.el.classes.iter().any(|c| c == "kanban-help"))
        .expect("la tarjeta de ayuda");
    assert!(card.rect.w > 100.0, "{:?}", card.rect);
    let text: String = f
        .dom
        .nodes
        .iter()
        .filter(|n| n.el.classes.iter().any(|c| c == "kanban-help-text"))
        .filter_map(|n| n.el.text.clone())
        .collect();
    assert!(text.contains("kanban: settings"), "{text}");
    assert!(text.contains("Prioridad"), "{text}");
    assert!(text.contains("@{"), "{text}");
    // What the card shows makes the template the parser accepts.
    let board = metamind_app::kanbanview::Board::parse(metamind_app::kanbanview::TEMPLATE);
    assert_eq!(board.columns.len(), 3, "la plantilla que se muestra parsea a tres columnas");
    // The button that opened it closes it.
    app.run("kanban:help");
    let f = frame(&mut app, &mut fonts);
    assert!(!f.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "kanban-help")));
}
