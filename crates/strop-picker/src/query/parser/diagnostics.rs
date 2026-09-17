//! Located-diagnostic helpers: misspelled qualifiers and unknown
//! language names get a correction ranked by edit distance (0051 §3).

use strop_core::languages;

use super::super::lexer;
use super::QueryDiagnostic;

/// A word shaped `name:value` (unquoted) that names no known qualifier
/// is an error with a correction, never silently ignored scope (0051 §3).
pub(super) fn qualifier_shaped_error(
    text: &str,
    quoted: bool,
    range: &std::ops::Range<usize>,
) -> Option<QueryDiagnostic> {
    if quoted {
        return None;
    }
    let (key, value) = text.split_once(':')?;
    if lexer::QUALIFIERS.contains(&key) {
        return None;
    }
    // `ssh://host`, `std::fmt` and `C:\temp` are literal text, never
    // qualifier-shaped input (0051 §3: URI schemes, drive prefixes and
    // C++ `::` are not split into qualifiers — searching `fmt::` or a
    // URL is ordinary literal text, not a misspelled filter).
    if value.starts_with("//") || value.starts_with(':') {
        return None;
    }
    // `langauge:rust` needs an error and a correction, never silently
    // ignored scope.
    if key.len() >= 3 && key.chars().all(|c| c.is_ascii_alphabetic()) {
        let suggestion = closest_qualifier(key).map(|q| format!("did you mean {q}:?"));
        return Some(QueryDiagnostic {
            range: range.clone(),
            message: format!("unknown qualifier: {key}"),
            suggestion,
        });
    }
    None
}

fn closest_qualifier(key: &str) -> Option<&'static str> {
    lexer::QUALIFIERS
        .iter()
        .min_by_key(|q| edit_distance(key, q))
        .filter(|q| edit_distance(key, q) <= 2)
        .copied()
}

pub(super) fn closest_language(value: &str) -> Option<&'static str> {
    languages::language_names()
        .min_by_key(|name| edit_distance(value, name))
        .filter(|name| edit_distance(value, name) <= 2)
}

fn edit_distance(a: &str, b: &str) -> usize {
    // small bounded Levenshtein for suggestion ranking
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}
