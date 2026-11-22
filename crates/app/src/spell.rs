//! Spellcheck against the bundled en_US Hunspell dictionary (SCOWL, see dict/LICENSE) plus the
//! user's own word list, kept as `dictionary.txt` in the configuration folder.

use spellbook::Dictionary;
use std::path::PathBuf;
use std::sync::RwLock;

static DICT: RwLock<Option<Dictionary>> = RwLock::new(None);

/// Build the dictionary. Parsing takes tens of milliseconds, so the window calls this on a
/// worker thread at startup; until it finishes nothing is marked.
pub fn load() {
    if DICT.read().is_ok_and(|d| d.is_some()) {
        return;
    }
    let mut dict = match Dictionary::new(include_str!("dict/en_US.aff"), include_str!("dict/en_US.dic")) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("metamind: spellcheck dictionary: {e}");
            return;
        }
    };
    if let Some(path) = user_path() {
        match std::fs::read_to_string(&path) {
            Ok(words) => {
                for w in words.lines().map(str::trim).filter(|w| !w.is_empty()) {
                    if let Err(e) = dict.add(w) {
                        eprintln!("metamind: {}: {w}: {e}", path.display());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("metamind: {}: {e}", path.display()),
        }
    }
    if let Ok(mut d) = DICT.write() {
        *d = Some(dict);
    }
}

fn user_path() -> Option<PathBuf> {
    crate::recent::config_dir().map(|d| d.join("dictionary.txt"))
}

/// Byte ranges of the misspelled words in `text`. Empty while the dictionary is loading.
pub fn misspelled(text: &str) -> Vec<(usize, usize)> {
    let Ok(guard) = DICT.read() else { return Vec::new() };
    let Some(dict) = guard.as_ref() else { return Vec::new() };
    words(text).filter(|&(a, b)| !dict.check(&normalize(&text[a..b]))).collect()
}

/// Up to five replacements for `word`.
pub fn suggest(word: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(dict) = DICT.read().ok().as_ref().and_then(|d| d.as_ref()) {
        dict.suggest(&normalize(word), &mut out);
    }
    out.truncate(5);
    out
}

/// Accept `word` from now on and remember it in the user's word list.
pub fn add(word: &str) -> std::io::Result<()> {
    let word = normalize(word);
    if let Some(dict) = DICT.write().ok().as_mut().and_then(|d| d.as_mut()) {
        dict.add(&word).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
    }
    let path = user_path().ok_or_else(|| std::io::Error::other("no hay carpeta de configuración"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(f, "{word}")
}

fn normalize(word: &str) -> String {
    word.replace('\u{2019}', "'")
}

/// Words worth checking: letters with inner apostrophes, at least two letters long. Runs that
/// touch digits, underscores or slashes, and mixed-case runs like `camelCase`, are skipped since
/// they are usually identifiers, not prose.
fn words(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let is_quote = |c: char| c == '\'' || c == '\u{2019}';
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || loop {
        let (start, c) = chars.next()?;
        if !c.is_alphanumeric() {
            continue;
        }
        let mut end = start + c.len_utf8();
        let mut skip = !c.is_alphabetic();
        let mut upper_after_first = false;
        while let Some(&(i, c)) = chars.peek() {
            if c.is_alphabetic() {
                upper_after_first |= c.is_uppercase();
            } else if c.is_numeric() || c == '_' || c == '/' || c == '\\' {
                skip = true;
            } else if is_quote(c) && text[i + c.len_utf8()..].starts_with(char::is_alphabetic) {
                // An apostrophe inside a word; a trailing one is closing punctuation.
            } else {
                break;
            }
            end = i + c.len_utf8();
            chars.next();
        }
        let w = &text[start..end];
        let all_upper = !w.chars().any(char::is_lowercase);
        if skip || w.chars().count() < 2 || (upper_after_first && !all_upper) {
            continue;
        }
        return Some((start, end));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<&str> {
        words(text).map(|(a, b)| &text[a..b]).collect()
    }

    #[test]
    fn splits_prose_into_words() {
        assert_eq!(found("Don't stop—it’s fine."), ["Don't", "stop", "it’s", "fine"]);
        assert_eq!(found("the dogs' bowls"), ["the", "dogs", "bowls"]);
    }

    #[test]
    fn skips_identifiers() {
        assert_eq!(found("a camelCase snake_case v2 path/to NASA ok"), ["NASA", "ok"]);
    }

    #[test]
    fn marks_misspellings() {
        load();
        let text = "This sentance is mostly corect, isn’t it?";
        let bad: Vec<&str> = misspelled(text).into_iter().map(|(a, b)| &text[a..b]).collect();
        assert_eq!(bad, ["sentance", "corect"]);
        assert!(suggest("sentance").iter().any(|s| s == "sentence"));
    }
}
