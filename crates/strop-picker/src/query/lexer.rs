//! The query lexer (0051 §3/§4): one span-producing tokenizer for the
//! bounded qualifier language. Every token carries its source range;
//! quotes and escapes are preserved in the raw input — the lexer never
//! rewrites a character. Highlighting, parsing and suggestions consume
//! this same token stream; there is no second scanner.

/// A lexed token with its byte range in the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range in the raw query string.
    pub range: std::ops::Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// `language:rust` / `-language:rust`: a recognized qualifier with
    /// key and value. The key range excludes the leading `-`.
    Qualifier {
        key: String,
        key_range: std::ops::Range<usize>,
        value: String,
        negated: bool,
    },
    /// A bare word — literal text. `quoted` when it arrived inside
    /// quotes (quoted words are never qualifiers, e.g. `"language:rust"`).
    Word { text: String, quoted: bool },
    /// An unclosed quote: the word is incomplete (typing in progress).
    UnclosedQuote,
}

/// Recognized qualifier keys (0051 §3 vocabulary).
pub const QUALIFIERS: &[&str] = &[
    "language", "path", "glob", "hidden", "ignored", "case", "text", "regex",
];

/// Lex the whole input. Never fails: an unclosed quote is a token, not
/// an exception.
pub fn lex(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < input.len() {
        let rest = &input[at..];
        let ws = rest
            .char_indices()
            .take_while(|(_, c)| c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .last()
            .unwrap_or(0);
        at += ws;
        if at >= input.len() {
            break;
        }
        let start = at;
        let mut text = String::new();
        // a word STARTING with a quote is never a qualifier; a quoted
        // value inside an unquoted word keeps the word's meaning
        // (`glob:"my files/**/*.rs"` IS a glob qualifier).
        let mut starts_quoted = false;
        let mut unclosed = false;
        while at < input.len() {
            let c = input[at..].chars().next().unwrap();
            if c.is_whitespace() {
                break;
            }
            if c == '"' || c == '\'' {
                starts_quoted = starts_quoted || text.is_empty();
                at += c.len_utf8();
                match take_quoted(input, at, c) {
                    Quoted::Closed { value, end } => {
                        text.push_str(&value);
                        at = end;
                    }
                    Quoted::Unclosed { value } => {
                        text.push_str(&value);
                        at = input.len();
                        unclosed = true;
                    }
                }
            } else {
                text.push(c);
                at += c.len_utf8();
            }
        }
        let range = start..at;
        if unclosed {
            tokens.push(Token {
                kind: TokenKind::UnclosedQuote,
                range,
            });
            continue;
        }
        tokens.push(Token {
            kind: classify(&text, range.clone(), starts_quoted),
            range,
        });
    }
    tokens
}

enum Quoted {
    Closed { value: String, end: usize },
    Unclosed { value: String },
}

/// Consume a quoted span at `at` (after the opening quote). Only the
/// active quote and the backslash itself escape (0051 §3: no C/JSON
/// escapes — `C:\temp` stays a path).
fn take_quoted(input: &str, at: usize, quote: char) -> Quoted {
    let mut value = String::new();
    let mut i = at;
    while i < input.len() {
        let c = input[i..].chars().next().unwrap();
        if c == quote {
            return Quoted::Closed {
                value,
                end: i + quote.len_utf8(),
            };
        }
        if c == '\\' {
            let next = input[i + 1..].chars().next();
            match next {
                Some(n) if n == quote || n == '\\' => {
                    value.push(n);
                    i += 1 + n.len_utf8();
                }
                _ => {
                    value.push(c);
                    i += 1;
                }
            }
        } else {
            value.push(c);
            i += c.len_utf8();
        }
    }
    Quoted::Unclosed { value }
}

/// Word → token kind: qualifier recognition happens only for an exact
/// known key at an unquoted token start, with `-` only negating a known
/// qualifier (0051 §3: `-test.rs` and `--notes.rs` are literal text).
fn classify(text: &str, range: std::ops::Range<usize>, quoted: bool) -> TokenKind {
    if !quoted {
        let (negated, rest) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if let Some((key, value)) = rest.split_once(':') {
            if QUALIFIERS.contains(&key) {
                let key_start = range.start + usize::from(negated);
                return TokenKind::Qualifier {
                    key: key.to_string(),
                    key_range: key_start..key_start + key.len(),
                    value: value.to_string(),
                    negated,
                };
            }
        }
    }
    TokenKind::Word {
        text: text.to_string(),
        quoted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<TokenKind> {
        lex(input).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn qualifiers_and_words() {
        assert_eq!(
            kinds("language:rust parser"),
            vec![
                TokenKind::Qualifier {
                    key: "language".into(),
                    key_range: 0..8,
                    value: "rust".into(),
                    negated: false,
                },
                TokenKind::Word {
                    text: "parser".into(),
                    quoted: false
                },
            ]
        );
    }

    #[test]
    fn negation_only_for_known_qualifiers() {
        match &kinds("-language:rust")[0] {
            TokenKind::Qualifier { negated, .. } => assert!(negated),
            other => panic!("{other:?}"),
        }
        // -test.rs and --notes.rs are literal text, never options
        assert_eq!(
            kinds("-test.rs --notes.rs"),
            vec![
                TokenKind::Word {
                    text: "-test.rs".into(),
                    quoted: false
                },
                TokenKind::Word {
                    text: "--notes.rs".into(),
                    quoted: false
                },
            ]
        );
    }

    #[test]
    fn unknown_qualifier_shaped_words_stay_literal() {
        // the parser reports them; the lexer never drops them
        assert_eq!(
            kinds("langauge:rust"),
            vec![TokenKind::Word {
                text: "langauge:rust".into(),
                quoted: false
            }]
        );
    }

    #[test]
    fn quoted_words_are_never_qualifiers() {
        match &kinds("\"language:rust\"")[0] {
            TokenKind::Word { text, quoted } => {
                assert!(quoted);
                assert_eq!(text, "language:rust");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn quotes_preserve_spaces_and_paths() {
        assert_eq!(
            kinds("glob:\"src/my files/**/*.rs\""),
            vec![TokenKind::Qualifier {
                key: "glob".into(),
                key_range: 0..4,
                value: "src/my files/**/*.rs".into(),
                negated: false,
            }]
        );
        assert_eq!(
            kinds("C:\\temp\\file.rs"),
            vec![TokenKind::Word {
                text: "C:\\temp\\file.rs".into(),
                quoted: false
            }]
        );
    }

    #[test]
    fn escapes_are_only_the_active_quote_and_backslash() {
        match &kinds(r#"text:"a\"b\n""#)[0] {
            TokenKind::Qualifier { value, .. } => assert_eq!(value, "a\"b\\n"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unclosed_quote_is_a_token_not_an_error() {
        assert_eq!(kinds("text:\"abc"), vec![TokenKind::UnclosedQuote]);
    }

    #[test]
    fn colon_in_uri_is_not_a_qualifier() {
        assert_eq!(
            kinds("ssh://host/path"),
            vec![TokenKind::Word {
                text: "ssh://host/path".into(),
                quoted: false
            }]
        );
    }

    #[test]
    fn spans_cover_the_input() {
        let input = "language:rust  retry \"two words\"";
        let tokens = lex(input);
        let mut cursor = 0;
        for token in &tokens {
            assert!(token.range.start >= cursor);
            cursor = token.range.end;
        }
        assert_eq!(tokens[2].range, 21..32);
    }
}
