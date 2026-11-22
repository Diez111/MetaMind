//! `dataview` code blocks: the query runs against the vault index and its table takes the place
//! of the fence in reading view and in live preview while the cursor is outside the block, the
//! same way a table turns back into source when the cursor enters it.
//!
//! The result is cached per query and vault revision, so the scan over every note's front matter
//! runs again only when the notes change.

use crate::shell::App;
use dataview_core::{Opts, Table};
use md_engine::{NodeKind, Tree};
use metamind_ui::{Dom, El, NodeId};
use std::collections::HashMap;

/// Computed blocks of the open vault: query text -> (vault revision, table or error).
pub(crate) type Cached = std::cell::RefCell<HashMap<String, (u64, Result<Table, String>)>>;

/// The query a `dataview` fence holds, or `None` when the block is another language.
pub fn query_source(tree: &Tree, src: &str, root_start: u32) -> Option<String> {
    let NodeKind::CodeBlock { info, content, .. } = tree.root().kind else { return None };
    let rel = |v: u32| v.checked_sub(root_start).map(|v| v as usize);
    let lang = highlight_core::lang_of_info(src.get(rel(info.start)?..rel(info.end)?)?);
    if lang != "dataview" {
        return None;
    }
    Some(src.get(rel(content.start)?..rel(content.end)?)?.trim().to_string())
}

/// A number that changes when the notes change; the index itself has no version.
pub(crate) fn revision(app: &App) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (_, f) in app.store.index.iter() {
        for v in [f.path.len() as u64, f.mtime as u64, f.size] {
            h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// The table `query` builds over the vault, from the cache when nothing changed.
pub fn table(app: &App, query: &str) -> Result<Table, String> {
    let rev = revision(app);
    if let Some((cached, table)) = app.dataview.borrow().get(query) {
        if *cached == rev {
            return table.clone();
        }
    }
    let opts = Opts { utc_offset: crate::datetime::local_offset(vault_core::fsio::now_ms() / 1000) };
    let result = match dataview_core::parse(query) {
        Ok(q) => Ok(dataview_core::table(&app.store.index, &q, opts)),
        Err(e) => Err(format!("{e}")),
    };
    app.store.release_cache_pages();
    app.dataview.borrow_mut().insert(query.to_string(), (rev, result.clone()));
    result
}

/// Add the result of `query` to `view` as a table. An error comes back for the caller to show
/// under the block's source.
pub fn block(app: &App, dom: &mut Dom, view: NodeId, query: &str) -> Result<(), String> {
    let t = table(app, query)?;
    // Column widths follow the longest cell, as the table renderer does.
    let weights: Vec<f32> = (0..t.headers.len())
        .map(|c| {
            let chars = t
                .rows
                .iter()
                .filter_map(|r| r.cells.get(c))
                .map(|cell| cell.text.chars().count())
                .chain([t.headers[c].chars().count()])
                .max()
                .unwrap_or(0);
            chars.clamp(3, 60) as f32 * 10.0 + 22.0
        })
        .collect();
    let total: f32 = weights.iter().sum();
    if total <= 0.0 {
        return Ok(());
    }
    let width = |c: usize| format!("width: {:.3}%", weights[c] / total * 100.0);
    let grid = dom.add(
        view,
        El::div().class("markdown-table dataview-table").style(&format!("max-width: {:.0}px", total + 2.0)),
    );
    let head = dom.add(grid, El::div().class("table-row is-header"));
    for (c, h) in t.headers.iter().enumerate() {
        dom.add(head, El::div().class("table-cell").style(&width(c)).text(h.clone()));
    }
    for row in &t.rows {
        let tr = dom.add(grid, El::div().class("table-row"));
        for (c, cell) in row.cells.iter().enumerate() {
            let el = El::div()
                .class("table-cell")
                .class_if(cell.link.is_some(), "internal-link")
                .text(cell.text.clone());
            let el = match &cell.link {
                Some(path) => el.action(format!("open:{path}")),
                None => el,
            };
            dom.add(tr, el.style(&width(c)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dataview_fences_hold_a_query() {
        let src = "```dataview\nTABLE tipo FROM \"x\"\n```\n";
        let doc = md_engine::parse(src);
        let b = &doc.blocks[0];
        assert_eq!(query_source(&b.tree, src, 0), Some("TABLE tipo FROM \"x\"".to_string()));
        let src = "```js\nconsole.log(1)\n```\n";
        let doc = md_engine::parse(src);
        assert_eq!(query_source(&doc.blocks[0].tree, src, 0), None);
    }
}
