//! `flex-wrap: wrap` on a row: line breaking, per-line growth and the gaps between lines.
//!
//! The layout engine breaks a row into lines only when it has a definite width to break against:
//! that is the case here (the root is laid out at a fixed size), and it is the case of every
//! popover and board panel that wraps its colour swatches.

use metamind_ui::{Dom, El, Env, Fonts, Theme};

const CSS: &str = "
body { background: #1e1e1e; color: #ddd; font-size: 14px }
.wrap { display: flex; flex-wrap: wrap; width: 250px; gap: 10px }
.cell { width: 100px; height: 20px }
.wide { width: 300px; height: 20px }
.grower { width: 60px; height: 20px; flex-grow: 1 }
.rows { display: flex; flex-wrap: wrap; width: 250px; row-gap: 7px; column-gap: 10px }
.col { display: flex; flex-direction: column; flex-wrap: wrap; height: 100px }
";

fn render(css: &str, w: f32, h: f32, build: impl FnOnce(&mut Dom) -> Vec<usize>) -> (Dom, Vec<usize>) {
    let mut theme = Theme::new();
    theme.add_sheet("test", css);
    let mut fonts = Fonts::new();
    let mut d = Dom::new();
    let ids = build(&mut d);
    d.style(&theme, Env { dark: true, ..Env::default() });
    d.layout(&mut fonts, w, h);
    (d, ids)
}

#[test]
fn five_cells_of_100_in_250_break_two_per_line() {
    let (d, ids) = render(CSS, 400.0, 300.0, |d| {
        let row = d.add(0, El::div().class("wrap"));
        (0..5).map(|_| d.add(row, El::div().class("cell"))).collect()
    });
    let r: Vec<_> = ids.iter().map(|&i| d.nodes[i].rect).collect();
    // 2 + 10 + 100 = 210 fits; a third cell would need 320.
    assert_eq!((r[0].x, r[1].x), (0.0, 110.0));
    assert_eq!(r[2].x, 0.0, "the third cell starts the second line");
    assert_eq!(r[2].y, r[0].y + 20.0 + 10.0, "row-gap separates the lines: {:?}", r[2]);
    assert_eq!((r[4].x, r[4].y), (0.0, r[2].y + 30.0), "two lines of two and a line of one");
}

#[test]
fn a_cell_wider_than_the_line_gets_a_line_to_itself() {
    let (d, ids) = render(CSS, 400.0, 300.0, |d| {
        let row = d.add(0, El::div().class("wrap"));
        vec![
            d.add(row, El::div().class("cell")),
            d.add(row, El::div().class("wide")),
            d.add(row, El::div().class("cell")),
        ]
    });
    let r: Vec<_> = ids.iter().map(|&i| d.nodes[i].rect).collect();
    assert_eq!(r[1].y, r[0].y + 30.0, "the wide cell took a line of its own: {r:?}");
    assert_eq!(r[2].y, r[1].y + 30.0, "and the next cell starts a new line: {r:?}");
}

#[test]
fn grow_is_shared_inside_the_line_only() {
    let (d, ids) = render(CSS, 400.0, 300.0, |d| {
        let row = d.add(0, El::div().class("wrap"));
        vec![
            d.add(row, El::div().class("cell")),
            d.add(row, El::div().class("cell")),
            // Two growers on the second line: they share the free space of that line.
            d.add(row, El::div().class("grower")),
            d.add(row, El::div().class("grower")),
        ]
    });
    let r: Vec<_> = ids.iter().map(|&i| d.nodes[i].rect).collect();
    assert_eq!(r[0].w, 100.0, "a cell that does not grow keeps its width");
    assert_eq!(r[2].y, r[0].y + 30.0, "growers went to the second line");
    // 250 - (60 + 60 + 10) = 120 of free space, half each.
    assert!((r[2].w - 120.0).abs() < 0.5, "first grower: {:?}", r[2]);
    assert!((r[3].w - 120.0).abs() < 0.5, "second grower: {:?}", r[3]);
    assert!((r[3].x + r[3].w - 250.0).abs() < 0.5, "the line is filled: {:?}", r[3]);
}

#[test]
fn row_gap_and_column_gap_are_different() {
    let (d, ids) = render(CSS, 400.0, 300.0, |d| {
        let row = d.add(0, El::div().class("rows"));
        (0..4).map(|_| d.add(row, El::div().class("cell"))).collect()
    });
    let r: Vec<_> = ids.iter().map(|&i| d.nodes[i].rect).collect();
    assert_eq!(r[1].x - r[0].x, 110.0, "column-gap is the gap between cells");
    assert_eq!(r[2].y - r[0].y, 20.0 + 7.0, "row-gap is the gap between lines");
}

#[test]
fn wrap_on_a_column_is_ignored() {
    let (d, ids) = render(CSS, 400.0, 300.0, |d| {
        let col = d.add(0, El::div().class("col"));
        (0..4).map(|_| d.add(col, El::div().class("cell"))).collect()
    });
    let r: Vec<_> = ids.iter().map(|&i| d.nodes[i].rect).collect();
    assert_eq!(r[1].y, r[0].y + 20.0, "a column stacks without breaking: {r:?}");
    assert_eq!(r[3].y, r[0].y + 60.0);
}

#[test]
fn a_wrapped_row_inside_a_wrapped_row_keeps_its_own_lines() {
    let css = format!("{CSS}\n.inner {{ display: flex; flex-wrap: wrap; width: 150px; gap: 5px }}");
    let (d, ids) = render(&css, 400.0, 300.0, |d| {
        let row = d.add(0, El::div().class("wrap"));
        let inner = d.add(row, El::div().class("inner"));
        let a = d.add(inner, El::div().class("cell"));
        let b = d.add(inner, El::div().class("cell"));
        vec![a, b]
    });
    let r: Vec<_> = ids.iter().map(|&i| d.nodes[i].rect).collect();
    assert_eq!(r[1].y, r[0].y + 20.0 + 5.0, "the inner row wrapped on its own: {r:?}");
}

#[test]
fn the_intrinsic_height_of_a_wrapping_row_counts_every_line() {
    // A popover sizes itself to its content: the swatch grid must report three lines, not one.
    let css = format!("{CSS}\n.pop {{ display: flex; flex-direction: column; width: 250px }}");
    let (d, ids) = render(&css, 400.0, 600.0, |d| {
        let pop = d.add(0, El::div().class("pop"));
        let row = d.add(pop, El::div().class("wrap"));
        let mut cells = vec![row];
        cells.extend((0..5).map(|_| d.add(row, El::div().class("cell"))));
        let after = d.add(pop, El::div().class("cell"));
        cells.push(after);
        cells
    });
    let row = d.nodes[ids[0]].rect;
    let after = d.nodes[*ids.last().unwrap()].rect;
    assert_eq!(row.h, 20.0 * 3.0 + 10.0 * 2.0, "three lines with two gaps: {row:?}");
    assert_eq!(after.y, row.y + row.h, "the next block starts under the wrapped row: {after:?}");
}
