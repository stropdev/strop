//! The query parser (0051 §3): tokens → a typed `SearchQuery` with
//! located diagnostics and an explicit state. Highlighting, compilation
//! and suggestions all consume this one result per revision — there is
//! no second parser.

use strop_core::languages;

use super::lexer::{self, TokenKind};

/// Case behavior for the search expression, resolved once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    Smart,
    Sensitive,
    Ignore,
}

/// The search expression (content surface): literal by default, regex
/// only by explicit `regex:` (0051 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentExpr {
    Literal(String),
    Regex(String),
}

/// A located parse diagnostic; `suggestion` carries the closest valid
/// alternative where one exists.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueryDiagnostic {
    pub range: std::ops::Range<usize>,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Parse outcome. Incomplete typing is normal — but it is never
/// permission to silently broaden the search (0051 §4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QueryState {
    #[default]
    Ready,
    /// Unclosed quote / trailing qualifier with an empty value.
    Incomplete,
    /// Any located error.
    Invalid,
}

/// The typed query: filters for file selection, visibility policy, case
/// behavior and an optional content expression.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub source: String,
    pub languages: Vec<&'static str>,
    pub paths: Vec<String>,
    pub globs: Vec<String>,
    pub exclude_languages: Vec<&'static str>,
    pub exclude_paths: Vec<String>,
    pub exclude_globs: Vec<String>,
    /// None = session default; Some = this query's explicit override.
    pub hidden: Option<bool>,
    pub ignored: Option<bool>,
    pub case: Option<CaseMode>,
    pub content: Option<ContentExpr>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub state: QueryState,
    pub highlights: Vec<super::HighlightSpan>,
    pub(super) tokens: Vec<lexer::Token>,
    /// Explicit text/regex and wholly quoted input are exact file expressions;
    /// ordinary bare file input remains fuzzy without operator metacharacters.
    pub exact_file_expression: bool,
}

impl SearchQuery {
    pub fn same_scope(&self, other: &Self) -> bool {
        self.languages == other.languages
            && self.exclude_languages == other.exclude_languages
            && self.paths == other.paths
            && self.exclude_paths == other.exclude_paths
            && self.globs == other.globs
            && self.exclude_globs == other.exclude_globs
            && self.hidden == other.hidden
            && self.ignored == other.ignored
    }

    pub fn diagnostic_range_from(
        &self,
        previous: &Self,
        diagnostic: &QueryDiagnostic,
    ) -> std::ops::Range<usize> {
        let token = previous
            .tokens
            .iter()
            .find(|token| token.range == diagnostic.range);
        match token.map(|token| &token.kind) {
            Some(TokenKind::Qualifier {
                key,
                value,
                negated,
                ..
            }) => self.qualifier_range(key, value, *negated),
            _ => self.content_range(),
        }
    }
    pub(super) fn qualifier_range(
        &self,
        name: &str,
        wanted: &str,
        negative: bool,
    ) -> std::ops::Range<usize> {
        self.tokens.iter().find(|token| matches!(&token.kind,
            TokenKind::Qualifier { key, value, negated, .. } if key == name && value == wanted && *negated == negative))
            .map(|token| token.range.clone()).unwrap_or(0..0)
    }

    pub fn content_range(&self) -> std::ops::Range<usize> {
        self.tokens
            .iter()
            .filter(|token| match &token.kind {
                TokenKind::Word { .. } => true,
                TokenKind::Qualifier { key, .. } => matches!(key.as_str(), "text" | "regex"),
                _ => false,
            })
            .map(|token| token.range.clone())
            .reduce(|first, next| first.start..next.end)
            .unwrap_or(0..0)
    }

    /// Parse one input string (one parse per revision; the lexer and
    /// this pass are one logical step).
    pub fn parse(input: &str) -> Self {
        let tokens = lexer::lex(input);
        let mut query = SearchQuery {
            source: input.to_owned(),
            state: QueryState::Ready,
            ..Default::default()
        };
        let mut free_text: Vec<(String, bool, std::ops::Range<usize>)> = Vec::new();
        for token in &tokens {
            match &token.kind {
                TokenKind::UnclosedQuote => {
                    if query.state != QueryState::Invalid {
                        query.state = QueryState::Incomplete;
                    }
                    query.diagnostics.push(QueryDiagnostic {
                        range: token.range.clone(),
                        message: "unclosed quote".into(),
                        suggestion: Some("close the quote to finish the value".into()),
                    });
                }
                TokenKind::Word { text, quoted } => {
                    if let Some(diag) = qualifier_shaped_error(text, *quoted, &token.range) {
                        query.state = QueryState::Invalid;
                        query.diagnostics.push(diag);
                    } else {
                        free_text.push((text.clone(), *quoted, token.range.clone()));
                    }
                }
                TokenKind::Qualifier {
                    key,
                    value,
                    negated,
                    ..
                } => {
                    if value.is_empty() {
                        if query.state != QueryState::Invalid {
                            query.state = QueryState::Incomplete;
                        }
                        query.diagnostics.push(QueryDiagnostic {
                            range: token.range.clone(),
                            message: format!("{key}: expects a value"),
                            suggestion: Some(format!("finish {key}:… or delete it")),
                        });
                    } else {
                        query.apply_qualifier(key, value, *negated, &token.range);
                    }
                }
            }
        }
        query.resolve_content(free_text);
        query.highlights = super::highlight::from_tokens(&tokens, &query.diagnostics, query.state);
        query.tokens = tokens;
        query
    }

    fn fail(
        &mut self,
        range: &std::ops::Range<usize>,
        message: String,
        suggestion: Option<String>,
    ) {
        self.state = QueryState::Invalid;
        self.diagnostics.push(QueryDiagnostic {
            range: range.clone(),
            message,
            suggestion,
        });
    }

    fn apply_qualifier(
        &mut self,
        key: &str,
        value: &str,
        negated: bool,
        range: &std::ops::Range<usize>,
    ) {
        if negated && !matches!(key, "language" | "path" | "glob") {
            self.fail(
                range,
                format!("{key}: cannot be negated"),
                Some("use the explicit option value".into()),
            );
            return;
        }
        match key {
            "language" => match languages::extensions_for_language(value) {
                Some(_) => {
                    let name = languages::language_names()
                        .find(|name| *name == value)
                        .unwrap_or_else(|| {
                            languages::LANGUAGES
                                .iter()
                                .find(|l| l.aliases.contains(&value))
                                .map(|l| l.name)
                                .unwrap_or(value)
                        });
                    let name = languages::LANGUAGES
                        .iter()
                        .find(|l| l.name == name)
                        .map(|l| l.name)
                        .unwrap();
                    if negated {
                        self.exclude_languages.push(name);
                    } else {
                        self.languages.push(name);
                    }
                }
                None => self.fail(
                    range,
                    format!("unknown language: {value}"),
                    closest_language(value).map(|lang| format!("did you mean language:{lang}?")),
                ),
            },
            "path" => {
                if negated {
                    self.exclude_paths.push(value.to_string());
                } else {
                    self.paths.push(value.to_string());
                }
            }
            "glob" => {
                if negated {
                    self.exclude_globs.push(value.to_string());
                } else {
                    self.globs.push(value.to_string());
                }
            }
            "hidden" | "ignored" => {
                let parsed = match value {
                    "include" => Some(true),
                    "exclude" => Some(false),
                    _ => None,
                };
                let Some(parsed) = parsed else {
                    return self.fail(
                        range,
                        format!("{key}: expects include or exclude"),
                        Some(format!("{key}:include or {key}:exclude")),
                    );
                };
                let slot = if key == "hidden" {
                    &mut self.hidden
                } else {
                    &mut self.ignored
                };
                if let Some(existing) = *slot {
                    if existing != parsed {
                        self.fail(
                            range,
                            format!("conflicting {key} options"),
                            Some("keep one".into()),
                        );
                    }
                } else {
                    *slot = Some(parsed);
                }
            }
            "case" => {
                let mode = match value {
                    "smart" => CaseMode::Smart,
                    "sensitive" => CaseMode::Sensitive,
                    "ignore" => CaseMode::Ignore,
                    _ => {
                        return self.fail(
                            range,
                            "case: expects smart, sensitive or ignore".into(),
                            Some("case:smart".into()),
                        )
                    }
                };
                if let Some(existing) = self.case {
                    if existing != mode {
                        self.fail(
                            range,
                            "conflicting case options".into(),
                            Some("keep one".into()),
                        );
                    }
                } else {
                    self.case = Some(mode);
                }
            }
            "text" | "regex" => {
                self.exact_file_expression = true;
                if self.content.is_some() {
                    return self.fail(
                        range,
                        "one search expression only".into(),
                        Some("remove one of them".into()),
                    );
                }
                self.content = Some(if key == "text" {
                    ContentExpr::Literal(value.to_string())
                } else {
                    ContentExpr::Regex(value.to_string())
                });
            }
            _ => unreachable!("the lexer only emits known qualifiers"),
        }
    }

    /// Bare words become one literal phrase (0051 §3: ordinary
    /// inter-token whitespace; quoted text keeps its own spaces). An
    /// explicit `text:`/`regex:` must not mix with free text.
    fn resolve_content(&mut self, free_text: Vec<(String, bool, std::ops::Range<usize>)>) {
        if free_text.is_empty() {
            return;
        }
        if self.content.is_some() {
            let range = free_text[0].2.start..free_text.last().map(|t| t.2.end).unwrap_or(0);
            self.state = QueryState::Invalid;
            self.diagnostics.push(QueryDiagnostic {
                range,
                message: "free text cannot mix with an explicit text:/regex: expression".into(),
                suggestion: Some("quote it or move it into the expression".into()),
            });
            return;
        }
        self.exact_file_expression = free_text.iter().all(|(_, quoted, _)| *quoted);
        let phrase = free_text
            .iter()
            .map(|(text, _, _)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        self.content = Some(ContentExpr::Literal(phrase));
    }
}

/// A word shaped `name:value` (unquoted) that names no known qualifier
/// is an error with a correction, never silently ignored scope (0051 §3).
fn qualifier_shaped_error(
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

fn closest_language(value: &str) -> Option<&'static str> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_canonical_query() {
        let q = SearchQuery::parse("language:rust path:src/ retry_request");
        assert_eq!(q.state, QueryState::Ready);
        assert_eq!(q.languages, ["rust"]);
        assert_eq!(q.paths, ["src/"]);
        assert_eq!(
            q.content,
            Some(ContentExpr::Literal("retry_request".into()))
        );
    }

    #[test]
    fn multiword_bare_text_is_one_literal_phrase() {
        let q = SearchQuery::parse("retry request");
        assert_eq!(
            q.content,
            Some(ContentExpr::Literal("retry request".into()))
        );
    }

    #[test]
    fn quoted_glob_with_spaces() {
        let q =
            SearchQuery::parse("language:rust glob:\"src/my files/**/*.rs\" text:\"request-id\"");
        assert_eq!(q.globs, ["src/my files/**/*.rs"]);
        assert_eq!(q.content, Some(ContentExpr::Literal("request-id".into())));
    }

    #[test]
    fn negations() {
        let q = SearchQuery::parse("language:rust -glob:\"**/*-test.rs\" text:x");
        assert_eq!(q.exclude_globs, ["**/*-test.rs"]);
    }

    #[test]
    fn unknown_language_is_a_located_error_with_a_suggestion() {
        let q = SearchQuery::parse("language:rusty x");
        assert_eq!(q.state, QueryState::Invalid);
        assert!(q.diagnostics[0].message.contains("unknown language"));
        assert!(q.diagnostics[0]
            .suggestion
            .as_ref()
            .unwrap()
            .contains("rust"));
    }

    #[test]
    fn misspelled_qualifier_is_a_correction_not_silence() {
        let q = SearchQuery::parse("langauge:rust");
        assert_eq!(q.state, QueryState::Invalid);
        assert!(q.diagnostics[0].message.contains("unknown qualifier"));
        assert_eq!(
            q.diagnostics[0].suggestion.as_deref(),
            Some("did you mean language:?")
        );
    }

    #[test]
    fn conflicting_case_options_are_located_errors() {
        let q = SearchQuery::parse("case:smart case:ignore x");
        assert_eq!(q.state, QueryState::Invalid);
        assert!(q.diagnostics[0].message.contains("conflicting case"));
    }

    #[test]
    fn identical_singletons_are_harmless() {
        let q = SearchQuery::parse("case:smart case:smart x");
        assert_eq!(q.state, QueryState::Ready);
    }

    #[test]
    fn text_and_free_text_conflict() {
        let q = SearchQuery::parse("text:\"a\" b");
        assert_eq!(q.state, QueryState::Invalid);
    }

    #[test]
    fn unclosed_quote_is_incomplete_not_invalid() {
        let q = SearchQuery::parse("text:\"abc");
        assert_eq!(q.state, QueryState::Incomplete);
    }

    #[test]
    fn alias_rs_resolves_to_rust() {
        let q = SearchQuery::parse("language:rs x");
        assert_eq!(q.state, QueryState::Ready);
        assert_eq!(q.languages, ["rust"]);
    }

    #[test]
    fn visibility_options() {
        let q = SearchQuery::parse("hidden:exclude ignored:exclude language:cpp text:x");
        assert_eq!(q.hidden, Some(false));
        assert_eq!(q.ignored, Some(false));
    }

    #[test]
    fn quoted_reserved_filename_is_literal() {
        let q = SearchQuery::parse("text:\"language:rust\"");
        assert_eq!(q.state, QueryState::Ready);
        assert_eq!(
            q.content,
            Some(ContentExpr::Literal("language:rust".into()))
        );
    }
    #[test]
    fn whitespace_runs_preserve_the_next_token_and_quoted_content() {
        let query = SearchQuery::parse("  language:rust \t path:src/  text:\"a  b\"");
        assert_eq!(query.state, QueryState::Ready);
        assert_eq!(query.languages, ["rust"]);
        assert_eq!(query.paths, ["src/"]);
        assert_eq!(query.content, Some(ContentExpr::Literal("a  b".into())));
    }
    #[test]
    fn uri_schemes_and_scoped_paths_are_literal_text() {
        // 0051 §3: URI schemes and C++/Rust `::` paths are not split
        // into qualifiers — searching `fmt::` or a URL is ordinary text
        for input in [
            "ssh://host/path",
            "https://example.com/x",
            "std::fmt::Debug",
            "fmt::println",
        ] {
            let q = SearchQuery::parse(input);
            assert_eq!(q.state, QueryState::Ready, "{input}: {q:?}");
            assert_eq!(
                q.content,
                Some(ContentExpr::Literal((*input).into())),
                "{input}"
            );
        }
        // but a plain qualifier-shaped typo still errors with a correction
        let q = SearchQuery::parse("langauge:rust");
        assert_eq!(q.state, QueryState::Invalid);
    }

    #[test]
    fn unsupported_negative_options_cannot_silently_change_policy() {
        let query = SearchQuery::parse("-hidden:include needle");
        assert_eq!(query.state, QueryState::Invalid);
        assert_eq!(query.hidden, None);
    }
}
