//! Lightweight query suggestions (0051 §4): Ctrl-Space in a query
//! field offers qualifier names, language names/aliases and enumerated
//! values from static metadata and the current parse position — never
//! an LSP or a filesystem scan.

use strop_core::languages;

use super::lexer::{self, TokenKind};

/// One offered completion: the replacement text and the byte range it
/// replaces. Suggestions replace the exact token/value span — never
/// text after the caret or the rest of the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub insert: String,
    pub range: std::ops::Range<usize>,
    pub detail: String,
}

/// Suggest completions for the raw query at `caret` (byte offset).
pub fn suggest(query: &super::SearchQuery, caret: usize) -> Vec<Suggestion> {
    let input = &query.source;
    let mut caret = caret.min(input.len());
    while !input.is_char_boundary(caret) {
        caret -= 1;
    }
    let tokens = &query.tokens;
    // find the token containing the caret (or the one starting at it)
    let mut at_token: Option<&lexer::Token> = None;
    for token in tokens {
        if token.range.start <= caret && caret <= token.range.end {
            at_token = Some(token);
            break;
        }
        if token.range.start > caret {
            break;
        }
    }
    let Some(token) = at_token else {
        // caret in whitespace after the last token: offer qualifiers
        return qualifier_suggestions("", caret..caret);
    };
    match &token.kind {
        TokenKind::UnclosedQuote => Vec::new(),
        TokenKind::Word { text, quoted } => {
            if *quoted {
                return Vec::new();
            }
            // completing a qualifier key? "lang" → "language:"
            if !text.contains(':') {
                return qualifier_suggestions(
                    &input[token.range.start..caret],
                    token.range.clone(),
                );
            }
            Vec::new()
        }
        TokenKind::Qualifier { key, key_range, .. } => {
            let value_start = key_range.end + 1;
            if caret <= key_range.end {
                return qualifier_suggestions(
                    &input[key_range.start..caret],
                    key_range.start..value_start,
                );
            }
            let prefix = input[value_start..caret].trim_start_matches(['"', char::from(39)]);
            value_suggestions(key, prefix, value_start..token.range.end)
        }
    }
}

/// All qualifiers matching a partial key.
fn qualifier_suggestions(prefix: &str, range: std::ops::Range<usize>) -> Vec<Suggestion> {
    lexer::QUALIFIERS
        .iter()
        .filter(|q| prefix.is_empty() || q.starts_with(prefix))
        .map(|q| Suggestion {
            insert: format!("{q}:"),
            range: range.clone(),
            detail: "qualifier".into(),
        })
        .collect()
}

/// Values for the qualifier at its value position.
fn value_suggestions(key: &str, prefix: &str, range: std::ops::Range<usize>) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    let mut push = |insert: String, detail: &str| {
        if insert.starts_with(prefix) {
            out.push(Suggestion {
                insert,
                range: range.clone(),
                detail: detail.into(),
            });
        }
    };
    match key {
        "language" => {
            for language in languages::LANGUAGES {
                push(language.name.to_string(), "language");
                for alias in language.aliases {
                    push((*alias).to_string(), "alias");
                }
            }
        }
        "hidden" | "ignored" => {
            push("include".into(), "value");
            push("exclude".into(), "value");
        }
        "case" => {
            push("smart".into(), "value");
            push("sensitive".into(), "value");
            push("ignore".into(), "value");
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_field_offers_all_qualifiers() {
        let suggestions = suggest(&super::super::SearchQuery::parse(""), 0);
        assert_eq!(suggestions.len(), lexer::QUALIFIERS.len());
        assert!(suggestions.iter().any(|s| s.insert == "language:"));
    }

    #[test]
    fn partial_key_filters() {
        let suggestions = suggest(&super::super::SearchQuery::parse("lang"), 4);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].insert, "language:");
    }

    #[test]
    fn language_values_and_aliases() {
        let suggestions = suggest(&super::super::SearchQuery::parse("language:"), 9);
        assert!(suggestions.iter().any(|s| s.insert == "rust"));
        assert!(suggestions.iter().any(|s| s.insert == "rs"));
        assert!(suggestions.iter().any(|s| s.detail == "alias"));
    }

    #[test]
    fn case_values() {
        let suggestions = suggest(&super::super::SearchQuery::parse("case:sm"), 7);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].insert, "smart");
        assert_eq!(suggestions[0].range, 5..7);
    }

    #[test]
    fn quoted_spans_offer_nothing() {
        assert!(suggest(&super::super::SearchQuery::parse("\"lang"), 3).is_empty());
    }
}
