//! Lightweight query suggestions (0051 §4): Ctrl-Space in a query
//! field offers the keys its surface declares (0109), language names
//! and aliases, kind values and enumerated options from static metadata
//! and the current parse position — never an LSP or a filesystem scan.

use strop_core::{languages, symbol_kind};

use super::lexer::{self, TokenKind};
use super::schema::Schema;

/// One offered completion: the replacement text and the byte range it
/// replaces. Suggestions replace the exact token/value span — never
/// text after the caret or the rest of the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub insert: String,
    pub range: std::ops::Range<usize>,
    pub detail: String,
}

/// Suggest completions for the raw query at `caret` (byte offset), from
/// what `schema` declares.
pub fn suggest(query: &super::SearchQuery, caret: usize, schema: &Schema) -> Vec<Suggestion> {
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
        return qualifier_suggestions(schema, "", caret..caret);
    };
    match &token.kind {
        TokenKind::UnclosedQuote | TokenKind::Operator(_) | TokenKind::Paren { .. } => Vec::new(),
        TokenKind::Word { text, quoted } => {
            if *quoted {
                return Vec::new();
            }
            // completing a qualifier key? "lang" → "language:"
            if !text.contains(':') {
                return qualifier_suggestions(
                    schema,
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
                    schema,
                    &input[key_range.start..caret],
                    key_range.start..value_start,
                );
            }
            let prefix = input[value_start..caret].trim_start_matches(['"', char::from(39)]);
            value_suggestions(key, prefix, value_start..token.range.end)
        }
    }
}

/// The surface's keys matching a partial key.
fn qualifier_suggestions(
    schema: &Schema,
    prefix: &str,
    range: std::ops::Range<usize>,
) -> Vec<Suggestion> {
    schema
        .fields
        .iter()
        .filter(|field| field.key().starts_with(prefix))
        .map(|field| Suggestion {
            insert: format!("{}:", field.key()),
            range: range.clone(),
            detail: field.help().into(),
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
        "kind" => {
            // a value is offered when any word naming it fits the prefix
            for spec in symbol_kind::KINDS {
                if spec
                    .words()
                    .any(|word| word.starts_with(&prefix.to_ascii_lowercase()))
                {
                    out.push(Suggestion {
                        insert: spec.value.into(),
                        range: range.clone(),
                        detail: format!("symbol kind · {}", spec.chip),
                    });
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
            push("insensitive".into(), "value");
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::schema::{self, Field};
    use super::*;

    fn at(input: &str, caret: usize) -> Vec<Suggestion> {
        suggest(&crate::query::parse_search(input), caret, &schema::SEARCH)
    }

    #[test]
    fn empty_field_offers_every_declared_key() {
        let suggestions = at("", 0);
        assert_eq!(suggestions.len(), Field::ALL.len());
        assert!(suggestions.iter().any(|s| s.insert == "language:"));
        let files = suggest(&crate::query::parse_search(""), 0, &schema::FILES);
        assert!(!files.iter().any(|s| s.insert == "kind:"));
    }

    #[test]
    fn partial_key_filters() {
        let suggestions = at("lang", 4);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].insert, "language:");
    }

    #[test]
    fn language_values_and_aliases() {
        let suggestions = at("language:", 9);
        assert!(suggestions.iter().any(|s| s.insert == "rust"));
        assert!(suggestions.iter().any(|s| s.insert == "rs"));
        assert!(suggestions.iter().any(|s| s.detail == "alias"));
    }

    #[test]
    fn case_values() {
        let suggestions = at("case:sm", 7);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].insert, "smart");
        assert_eq!(suggestions[0].range, 5..7);
        let suggestions = at("case:ins", 8);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].insert, "insensitive");
        let all = at("case:", 5);
        assert!(all.iter().any(|s| s.insert == "ignore"));
        assert!(all.iter().any(|s| s.insert == "insensitive"));
        assert!(all.iter().any(|s| s.insert == "sensitive"));
    }

    #[test]
    fn quoted_spans_offer_nothing() {
        assert!(at("\"lang", 3).is_empty());
    }

    #[test]
    fn kind_values_come_from_the_kind_table() {
        let suggestions = at("kind:fun", 8);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].insert, "function");
        assert_eq!(suggestions[0].detail, "symbol kind · fn");
        // any word naming a kind offers its value: a chip, a habit
        let traits = at("kind:tra", 8);
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].insert, "interface");
        let all = at("kind:", 5);
        assert!(all
            .iter()
            .map(|s| s.insert.as_str())
            .eq(strop_core::symbol_kind::KINDS.iter().map(|spec| spec.value)));
    }
}
