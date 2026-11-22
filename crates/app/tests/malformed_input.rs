//! Parsers that read a file a user can put in a vault: a truncated or malformed file must give an
//! error (or a default), never a panic.
//!
//! This is the light fuzz pass from the security audit (see `SECURITY.md`): every parser gets the
//! hostile documents in [`CORPUS`] and every character-boundary prefix of them, which is where a
//! hand-written parser typically indexes out of bounds. The point is not coverage of the grammar
//! but that no input from disk takes the app down.

use math_core::Measure as MathMeasure;
use mermaid_core::Measure as DiagramMeasure;

/// Documents that a note, a board, a canvas or a query can contain: broken JSON, wrong types where
/// an object or a list is expected, absurd numbers, escapes that do not decode, front matter that
/// is not there, and text that is not UTF-8-able JSON at all.
const CORPUS: &[&str] = &[
    "",
    "{",
    "{}",
    "[]",
    "null",
    "0",
    "\"x\"",
    "{\"type\":\"excalidraw\",\"elements\":[{",
    "{\"type\":\"excalidraw\",\"elements\":[{\"id\":1}]}",
    "{\"type\":\"excalidraw\",\"elements\":{},\"files\":3,\"appState\":[]}",
    "{\"type\":\"canvas\",\"nodes\":[{\"x\":\"no\"}],\"edges\":[{}],\"extra\":null}",
    "{\"elements\":[{\"width\":-1e400,\"height\":1e400,\"x\":null}]}",
    "{\"type\":\"not-excalidraw\",\"elements\":[]}",
    "{\"type\":\"excalidraw\",\"elements\":[{\"id\":\"a\",\"type\":\"rectangle\",\"unk\":[[[[[[[[]]]]]]]]}]}",
    "{\"files\":{\"k\":{\"dataURL\":\"data:image/png;base64,%%%\"}}}",
    "---\ntags: [a, b\nkey: \"unterminated\n---\n# Title\n[[link]] [[..]] [[/abs]]\n",
    "---\ntitle: 2024-13-45\n\nkind: -\n---\n",
    " ```js\nlet x = ;\n```\n",
    "```\nunclosed",
    "TABLE\nWITHOUT\nFROM \"\"\"\nWHERE x = date(not-a-date) AND\nSORT",
    "TABLE a AS \"b\nFROM \"f\nWHERE x != null AND y > 1e999 SORT a DESC, b",
    "# Heading ### with ###### #\n[link](<a b>) [x](javascript:alert(1)) <script>alert(1)</script>",
    "\\frac{1}{",
    "$$\\begin{matrix} a & b \\end{matrix}$$",
    "graph TD\n  A-->B\n  B-->A\n  A-->A\n  %% comment\n  A[unclosed",
    "\u{feff}\u{0}\u{7f}\u{1b}[31m\\",
    "%%%%%",
    "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
];

/// Every prefix that ends on a character boundary, which is where slicing parsers break.
fn prefixes(src: &str) -> impl Iterator<Item = &str> {
    src.char_indices().map(|(i, _)| &src[..i]).chain(std::iter::once(src))
}

struct Approx;

impl DiagramMeasure for Approx {
    fn width(&mut self, text: &str, size: f32, _bold: bool) -> f32 {
        text.chars().count() as f32 * size * 0.6
    }
}

/// One parser at a time, so a panic in one names the crate it came from.
fn parse_all(src: &str) {
    let _ = excalidraw_core::Scene::parse(src);
    let _ = canvas_core::Canvas::parse(src);
    let _ = dataview_core::query::parse(src);
    let _ = search_core::query::parse(src);
    let doc = md_engine::parse(src);
    let _ = vault_core::meta::extract(src);
    let _ = vault_core::meta::extract_from(&doc, src);
    let lang = highlight_core::lang_of_info(src);
    let _ = highlight_core::highlight(&lang, src);
    let mut approx = math_core::Approx;
    let _ = math_core::render(src, 14.0, false, &mut approx as &mut dyn MathMeasure);
    let mut diagram = Approx;
    let _ = mermaid_core::render(src, &mut diagram);
    let _ = vault_core::path::normalize(src);
    let _ = vault_core::path::percent_decode(src);
    let _ = metamind_app::uri::parse(src);
    let _ = metamind_app::uri::is_link(src);
    let _ = metamind_app::youtube::find(src);
    let _ = metamind_app::youtube::is_youtube_url(src);
}

#[test]
fn every_prefix_of_a_hostile_file_parses_or_errors_without_panicking() {
    for src in CORPUS {
        for prefix in prefixes(src) {
            parse_all(prefix);
        }
    }
}

#[test]
fn a_board_with_absurd_numbers_does_not_panic() {
    for src in [
        r#"{"type":"excalidraw","elements":[{"id":"a","type":"rectangle","x":1e308,"y":-1e308,"width":1e308,"height":1e308,"angle":1e308}]}"#,
        r#"{"type":"excalidraw","elements":[{"id":"a","type":"text","fontSize":1e-308,"width":-0.0,"height":0}]}"#,
        r#"{"type":"excalidraw","version":1e308,"appState":{"zoom":1e308}}"#,
        r#"{"nodes":[{"id":"n","x":1e308,"y":1e308,"width":1e308,"height":1e308}]}"#,
    ] {
        parse_all(src);
    }
}

#[test]
fn deeply_nested_json_is_rejected_instead_of_overflowing_the_stack() {
    let deep = format!(
        "{{\"type\":\"excalidraw\",\"elements\":{}{}}}",
        "[".repeat(512),
        "]".repeat(512)
    );
    assert!(excalidraw_core::Scene::parse(&deep).is_err());
    let deep_object = format!("{}{}{}", "{\"a\":".repeat(2000), "0", "}".repeat(2000));
    assert!(canvas_core::Canvas::parse(&deep_object).is_err());
}

/// A `.md` file with front matter is YAML-ish, not JSON: the vault parser must not trust it.
#[test]
fn hostile_front_matter_is_kept_or_ignored_but_never_panics() {
    for src in [
        "---\n---\n",
        "---\n: : :\n---\n",
        "---\ntags: !include /etc/passwd\n---\n[[a]]",
        "---\ntitle: \"\\u0000\\uD800\"\n---\n",
        "---\nlist:\n  - a\n - b\n---\n",
        "\u{0}\u{0}\u{0}",
    ] {
        let meta = vault_core::meta::extract(src);
        // Whatever the front matter says, the links and tags that come back are plain strings.
        let _ = meta.links.len() + meta.tags.len();
    }
}
