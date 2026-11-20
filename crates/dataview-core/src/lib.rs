//! Dataview query blocks: `TABLE` queries over the front matter of the notes in a vault.
//!
//! Supported subset, as the notes in the reference vault use it:
//!
//! ```text
//! TABLE [WITHOUT ID] [<field> [AS "<header>"], …]
//! FROM "<folder>"
//! WHERE <field> <op> <literal> [AND …]
//! SORT <field> [ASC|DESC], …
//! ```
//!
//! - Columns are front matter keys, case-insensitive, or the file fields `file.name`,
//!   `file.link` and `file.mtime`. Without `WITHOUT ID` the first column is a `File` link.
//! - `FROM` is a folder; notes in its subfolders are included. Without `FROM`, every note.
//! - `WHERE` compares with `=`, `!=`, `<`, `<=`, `>`, `>=` against a string, a number, `null`
//!   or `date(YYYY-MM-DD)`, and joins the tests with `AND`.
//! - `SORT` takes one or more keys; ties are broken by the note path, so the output is stable.
//!
//! Values: missing keys and empty lists and strings read as `null`; `a != null` is the test for
//! "the key has a value". Text compares without case. ISO dates (`YYYY-MM-DD`), in the note or
//! as `date(…)`, compare and sort by day. Numbers compare by value. A list renders as its items
//! separated by commas. Notes with an empty or missing sort key go last. Nested maps have no
//! value; `key.sub` reaches into one when it is compared or shown, but a map is not displayed.
//!
//! Results are a table that [`render::markdown`] writes as GFM (links as `[[path\|Name]]`) and
//! the app draws with its own widget tree.

mod eval;
pub mod query;
pub mod render;
mod value;

pub use eval::{table, Cell, Opts, Row, Table};
pub use query::{parse, Cmp, Column, Dir, Expr, Field, Lit, ParseError, Query, SortKey};
pub use render::markdown;
pub use value::{Date, Val};

use vault_core::Vault;

/// Parse `src`, run it against `vault` and return the result as a GFM table.
pub fn render(vault: &Vault, src: &str, opts: Opts) -> Result<String, ParseError> {
    Ok(markdown(&table(vault, &parse(src)?, opts)))
}
