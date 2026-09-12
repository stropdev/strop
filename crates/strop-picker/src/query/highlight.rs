//! Highlight roles from the SAME parse result (0051 §4): qualifier key,
//! punctuation, value, literal, regex, negation, incomplete, error —
//! covering the raw input without dropping characters. Quoted text is
//! never colored as an active filter.

use super::lexer::{self, TokenKind};

/// One styled span over the raw input, as (byte range, role).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: std::ops::Range<usize>,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    QualifierKey,
    Punctuation,
    Value,
    Literal,
    Regex,
    Negation,
    Incomplete,
    Error,
}

/// Style the raw input from its tokens and diagnostics. One call per
/// revision; the spans partition the tokens exactly.
pub(super) fn from_tokens(
    tokens: &[lexer::Token],
    diagnostics: &[super::QueryDiagnostic],
    state: super::QueryState,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    for token in tokens {
        match &token.kind {
            TokenKind::UnclosedQuote => spans.push(HighlightSpan {
                range: token.range.clone(),
                role: Role::Incomplete,
            }),
            TokenKind::Word { text, quoted } => {
                let role = if qualifier_shaped(text, *quoted) {
                    Role::Error
                } else {
                    Role::Literal
                };
                spans.push(HighlightSpan {
                    range: token.range.clone(),
                    role,
                });
            }
            TokenKind::Qualifier {
                key,
                key_range,
                value,
                negated,
                ..
            } => {
                if *negated {
                    spans.push(HighlightSpan {
                        range: token.range.start..key_range.start,
                        role: Role::Negation,
                    });
                }
                spans.push(HighlightSpan {
                    range: key_range.clone(),
                    role: Role::QualifierKey,
                });
                spans.push(HighlightSpan {
                    range: key_range.end..key_range.end + 1,
                    role: Role::Punctuation,
                });
                let value_start = key_range.end + 1;
                if value_start < token.range.end {
                    spans.push(HighlightSpan {
                        range: value_start..token.range.end,
                        role: if key == "regex" {
                            Role::Regex
                        } else {
                            Role::Value
                        },
                    });
                } else if value.is_empty() {
                    spans.push(HighlightSpan {
                        range: token.range.clone(),
                        role: Role::Incomplete,
                    });
                }
            }
        }
    }
    // error diagnostics overpaint their ranges
    for diagnostic in diagnostics {
        spans.push(HighlightSpan {
            range: diagnostic.range.clone(),
            role: if state == super::QueryState::Incomplete {
                Role::Incomplete
            } else {
                Role::Error
            },
        });
    }
    spans
}

fn qualifier_shaped(text: &str, quoted: bool) -> bool {
    if quoted {
        return false;
    }
    let Some((key, value)) = text.split_once(':') else {
        return false;
    };
    // the SAME shape rule as the parser (one parse, one identity):
    // `ssh://…` and `std::…` are literal text, not qualifier errors
    if value.starts_with("//") || value.starts_with(':') {
        return false;
    }
    key.len() >= 3
        && key.chars().all(|c| c.is_ascii_alphabetic())
        && !lexer::QUALIFIERS.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(input: &str) -> Vec<Role> {
        super::super::SearchQuery::parse(input)
            .highlights
            .into_iter()
            .map(|s| s.role)
            .collect()
    }

    #[test]
    fn qualifier_parts_get_roles() {
        assert_eq!(
            roles("language:rust"),
            [Role::QualifierKey, Role::Punctuation, Role::Value]
        );
    }

    #[test]
    fn negation_is_its_own_role() {
        assert_eq!(
            roles("-language:rust"),
            [
                Role::Negation,
                Role::QualifierKey,
                Role::Punctuation,
                Role::Value
            ]
        );
    }

    #[test]
    fn quoted_reserved_words_stay_literal() {
        assert_eq!(roles("\"language:rust\""), [Role::Literal]);
    }

    #[test]
    fn unclosed_quote_is_incomplete() {
        assert!(roles("text:\"abc")
            .iter()
            .all(|role| *role == Role::Incomplete));
    }
}
