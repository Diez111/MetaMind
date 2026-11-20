//! A query result as a GFM table, the text the block stands for.

use crate::eval::Table;

/// The table as Markdown: a header row, a separator row and one row per note. Cells join with
/// `|`, a pipe in a value is escaped, and links are written `[[path\|Name]]` as tables need.
pub fn markdown(t: &Table) -> String {
    let mut out = String::new();
    for (i, h) in t.headers.iter().enumerate() {
        out.push_str(if i == 0 { "| " } else { " " });
        push_text(&mut out, h);
        out.push_str(" |");
        if i + 1 == t.headers.len() {
            out.push('\n');
        }
    }
    for i in 0..t.headers.len() {
        out.push_str(if i == 0 { "| --- " } else { " --- " });
        out.push('|');
        if i + 1 == t.headers.len() {
            out.push('\n');
        }
    }
    for row in &t.rows {
        for (i, cell) in row.cells.iter().enumerate() {
            out.push_str(if i == 0 { "| " } else { " " });
            match &cell.link {
                Some(path) => {
                    out.push_str("[[");
                    push_text(&mut out, path);
                    out.push_str("\\|");
                    push_text(&mut out, &cell.text);
                    out.push_str("]]");
                }
                None => push_text(&mut out, &cell.text),
            }
            out.push_str(" |");
        }
        out.push('\n');
    }
    out
}

/// Text safe inside a table cell: pipes escaped, newlines as `<br>`.
fn push_text(out: &mut String, s: &str) {
    for c in s.trim().chars() {
        match c {
            '|' => out.push_str("\\|"),
            '\n' => out.push_str("<br>"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{Cell, Row};

    fn cell(text: &str) -> Cell {
        Cell { text: text.into(), link: None }
    }

    #[test]
    fn a_table_becomes_gfm() {
        let t = Table {
            headers: vec!["File".into(), "Tipo".into()],
            rows: vec![Row {
                path: "Facultad/Materias/Uno.md".into(),
                cells: vec![
                    Cell { text: "Uno".into(), link: Some("Facultad/Materias/Uno.md".into()) },
                    cell("materia"),
                ],
            }],
        };
        assert_eq!(
            markdown(&t),
            "| File | Tipo |\n| --- | --- |\n| [[Facultad/Materias/Uno.md\\|Uno]] | materia |\n"
        );
    }

    #[test]
    fn pipes_and_lines_in_a_value_do_not_break_the_row() {
        let t = Table {
            headers: vec!["a|b".into()],
            rows: vec![Row { path: "x.md".into(), cells: vec![cell("one\ntwo")] }],
        };
        assert_eq!(markdown(&t), "| a\\|b |\n| --- |\n| one<br>two |\n");
    }

    #[test]
    fn no_rows_keeps_the_header() {
        let t = Table { headers: vec!["File".into()], rows: Vec::new() };
        assert_eq!(markdown(&t), "| File |\n| --- |\n");
    }
}
