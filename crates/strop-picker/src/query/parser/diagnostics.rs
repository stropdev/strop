//! Located-diagnostic helpers: misspelled keys, unknown languages and
//! unknown kinds get a correction ranked by edit distance (0051 §3,
//! 0109 §3.2c). A correction only ever names something the surface takes.

use strop_core::{languages, symbol_kind};

use super::super::schema::{Field, Schema};
use super::QueryDiagnostic;

/// Habits from other tools, read as the key strop spells differently.
const SYNONYMS: &[(&str, Field)] = &[
    ("type", Field::Kind),
    ("lang", Field::Language),
    ("ext", Field::Language),
    ("dir", Field::Path),
    ("file", Field::Path),
];

/// A word shaped `name:value` (unquoted) that names no key is an error
/// with a correction, never silently ignored scope (0051 §3).
pub(super) fn qualifier_shaped_error(
    text: &str,
    quoted: bool,
    range: &std::ops::Range<usize>,
    schema: &Schema,
) -> Option<QueryDiagnostic> {
    if quoted {
        return None;
    }
    let (key, value) = text.split_once(':')?;
    if Field::from_key(key).is_some() {
        return None;
    }
    // `ssh://host`, `std::fmt` and `C:\temp` are literal text, never
    // qualifier-shaped input (0051 §3: URI schemes, drive prefixes and
    // C++ `::` are not split into qualifiers — searching `fmt::` or a
    // URL is ordinary literal text, not a misspelled filter).
    if value.starts_with("//") || value.starts_with(':') {
        return None;
    }
    if key.len() >= 3 && key.chars().all(|c| c.is_ascii_alphabetic()) {
        let suggestion = closest_key(key, schema).map(|q| format!("did you mean {q}:?"));
        return Some(QueryDiagnostic {
            range: range.clone(),
            message: format!("unknown filter: {key}"),
            suggestion,
        });
    }
    None
}

fn closest_key(key: &str, schema: &Schema) -> Option<&'static str> {
    let lowered = key.to_ascii_lowercase();
    if let Some((_, field)) = SYNONYMS.iter().find(|(word, _)| *word == lowered) {
        return schema.declares(*field).then(|| field.key());
    }
    schema
        .fields
        .iter()
        .map(|field| field.key())
        .min_by_key(|candidate| edit_distance(&lowered, candidate))
        .filter(|candidate| edit_distance(&lowered, candidate) <= 2)
}

pub(super) fn closest_language(value: &str) -> Option<&'static str> {
    languages::language_names()
        .min_by_key(|name| edit_distance(value, name))
        .filter(|name| edit_distance(value, name) <= 2)
}

pub(super) fn closest_kind(value: &str) -> Option<&'static str> {
    let lowered = value.to_ascii_lowercase();
    symbol_kind::kind_words()
        .min_by_key(|word| edit_distance(&lowered, word))
        .filter(|word| edit_distance(&lowered, word) <= 2)
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
