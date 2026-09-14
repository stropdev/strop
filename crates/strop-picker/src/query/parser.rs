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

/// The Boolean expression tree (0063 §3). Present only when the query
/// carries an operator — operator-free queries keep the flat lowering
/// and their existing phrase behavior unchanged. Precedence on parse:
/// NOT > AND > OR; juxtaposition of units is an implicit AND, and a run
/// of bare words stays one phrase atom.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BooleanExpr {
    And(Vec<BooleanExpr>),
    Or(Vec<BooleanExpr>),
    Not(Box<BooleanExpr>),
    Content(ContentAtom),
    Metadata(MetadataAtom),
}

/// One content predicate: a phrase (literal) or an explicit `regex:`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContentAtom {
    pub regex: bool,
    pub text: String,
}

/// One metadata predicate, e.g. `language:python` / `-glob:vendor`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MetadataAtom {
    pub key: String,
    pub value: String,
    pub negated: bool,
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
    /// Some when the query carries a Boolean operator (0063 §3); then
    /// the flat `content` stays None and execution compiles from the AST.
    pub boolean: Option<BooleanExpr>,
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
        let boolean = tokens.iter().any(|token| match &token.kind {
            TokenKind::Operator(_) | TokenKind::Paren { .. } => true,
            // `kind:`/`repo:` narrow through the AST's evidence
            // (symbol index, project catalog — 0063 §2); a flat
            // spelling is an implicit AND, not a failure.
            TokenKind::Qualifier { key, value, .. } => {
                !value.is_empty() && matches!(key.as_str(), "kind" | "type" | "repo")
            }
            _ => false,
        });
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
                TokenKind::Operator(_) | TokenKind::Paren { .. } => {}
                TokenKind::Word { text, quoted } => {
                    if let Some(diag) = qualifier_shaped_error(text, *quoted, &token.range) {
                        query.state = QueryState::Invalid;
                        query.diagnostics.push(diag);
                    } else if !boolean {
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
                    } else if !boolean || matches!(key.as_str(), "hidden" | "ignored" | "case") {
                        // Query-wide options stay outside Boolean branches
                        // and keep their flat meaning (0063 §3); selection
                        // qualifiers live in the AST only.
                        let key = if key == "type" { "kind" } else { key };
                        query.apply_qualifier(key, value, *negated, &token.range);
                    }
                }
            }
        }
        if boolean {
            query.boolean = parse_boolean(&tokens, &mut query);
        } else {
            query.resolve_content(free_text);
        }
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

/// Recursive descent over the token stream (0063 §3): precedence
/// NOT > AND > OR, juxtaposition of units is an implicit AND, and a run
/// of bare words is one phrase atom. Located diagnostics for dangling
/// operators and unbalanced groups; the parse recovers instead of
/// stopping at the first error.
fn parse_boolean(tokens: &[lexer::Token], query: &mut SearchQuery) -> Option<BooleanExpr> {
    let mut parser = BooleanParser { tokens, at: 0 };
    let expr = parser.or_level(query);
    while parser.at < tokens.len() {
        let token = &tokens[parser.at];
        match &token.kind {
            TokenKind::Paren { open: false } => query.fail(
                &token.range,
                "unbalanced )".into(),
                Some("remove it or open the group with (".into()),
            ),
            TokenKind::Operator(operator) => query.fail(
                &token.range,
                format!("{} without a right operand", operator.as_str()),
                Some("add an operand after it".into()),
            ),
            _ => {}
        }
        parser.at += 1;
    }
    expr
}

struct BooleanParser<'a> {
    tokens: &'a [lexer::Token],
    at: usize,
}

impl<'a> BooleanParser<'a> {
    fn peek(&self) -> Option<&'a lexer::Token> {
        self.tokens.get(self.at)
    }

    fn or_level(&mut self, query: &mut SearchQuery) -> Option<BooleanExpr> {
        let first = self.and_level(query)?;
        let mut branches = vec![first];
        while matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::Operator(lexer::Operator::Or))
        ) {
            let range = self.peek().map(|token| token.range.clone());
            self.at += 1;
            match self.and_level(query) {
                Some(branch) => branches.push(branch),
                None => {
                    if let Some(range) = range {
                        query.fail(
                            &range,
                            "OR without a right operand".into(),
                            Some("add a term after OR".into()),
                        );
                    }
                    break;
                }
            }
        }
        match branches.len() {
            1 => branches.pop(),
            _ => Some(BooleanExpr::Or(branches)),
        }
    }

    fn and_level(&mut self, query: &mut SearchQuery) -> Option<BooleanExpr> {
        let mut operands = Vec::new();
        loop {
            let joined = matches!(
                self.peek().map(|token| &token.kind),
                Some(TokenKind::Operator(lexer::Operator::And))
            );
            if joined {
                self.at += 1;
            }
            if operands.is_empty() && joined {
                // AND at operand position: a dangling operator.
                if let Some(token) = self.peek() {
                    query.fail(
                        &token.range.clone(),
                        "AND without a left operand".into(),
                        Some("start with a term".into()),
                    );
                }
            }
            let before = self.at;
            match self.unit(query) {
                Some(operand) => operands.push(operand),
                None => {
                    if joined && before < self.tokens.len() {
                        // Consumed AND, no operand followed.
                    } else if joined {
                        if let Some(token) = self.tokens.get(before.saturating_sub(1)) {
                            query.fail(
                                &token.range.clone(),
                                "AND without a right operand".into(),
                                Some("add a term after AND".into()),
                            );
                        }
                    }
                    break;
                }
            }
        }
        match operands.len() {
            0 => None,
            1 => operands.pop(),
            _ => Some(BooleanExpr::And(operands)),
        }
    }

    /// One operand: NOT-unit, group, qualifier atom, or a bare-word
    /// phrase run (adjacent words stay one phrase, 0063 §3).
    fn unit(&mut self, query: &mut SearchQuery) -> Option<BooleanExpr> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Operator(lexer::Operator::Not) => {
                let range = token.range.clone();
                self.at += 1;
                match self.unit(query) {
                    Some(operand) => Some(BooleanExpr::Not(Box::new(operand))),
                    None => {
                        query.fail(
                            &range,
                            "NOT without an operand".into(),
                            Some("add a term after NOT".into()),
                        );
                        None
                    }
                }
            }
            TokenKind::Paren { open: true } => {
                let range = token.range.clone();
                self.at += 1;
                let inner = self.or_level(query);
                match self.peek().map(|token| &token.kind) {
                    Some(TokenKind::Paren { open: false }) => {
                        self.at += 1;
                    }
                    _ => query.fail(
                        &range,
                        "unclosed group".into(),
                        Some("close it with )".into()),
                    ),
                }
                inner
            }
            TokenKind::Paren { open: false }
            | TokenKind::Operator(lexer::Operator::And)
            | TokenKind::Operator(lexer::Operator::Or) => None,
            TokenKind::UnclosedQuote => None,
            TokenKind::Qualifier {
                key,
                value,
                negated,
                ..
            } => {
                self.at += 1;
                if value.is_empty() {
                    return None;
                }
                // `type:` is the documented input alias of `kind:` (0063 §3).
                let key = if key == "type" { "kind" } else { key };
                if key == "text" {
                    Some(BooleanExpr::Content(ContentAtom {
                        regex: false,
                        text: value.clone(),
                    }))
                } else if key == "regex" {
                    Some(BooleanExpr::Content(ContentAtom {
                        regex: true,
                        text: value.clone(),
                    }))
                } else {
                    Some(BooleanExpr::Metadata(MetadataAtom {
                        key: key.to_string(),
                        value: value.clone(),
                        negated: *negated,
                    }))
                }
            }
            TokenKind::Word { .. } => {
                let mut words = Vec::new();
                while let Some(token) = self.peek() {
                    match &token.kind {
                        TokenKind::Word { text, .. } => {
                            words.push(text.clone());
                            self.at += 1;
                        }
                        _ => break,
                    }
                }
                Some(BooleanExpr::Content(ContentAtom {
                    regex: false,
                    text: words.join(" "),
                }))
            }
        }
    }
}

impl BooleanExpr {
    /// Canonical fully-parenthesized formatting. Always re-parses to the
    /// same tree (parse-format-parse property, 0063 §6.1).
    pub fn to_query_string(&self) -> String {
        match self {
            Self::Content(atom) => {
                let prefix = if atom.regex { "regex:" } else { "" };
                // Quote anything the lexer would re-shape: whitespace,
                // quotes, parens, backslashes, colons (a bare
                // `kind:…` inside content would lex as a qualifier)
                // and the operator words themselves.
                let hostile = atom.text.is_empty()
                    || atom.text.starts_with('-')
                    || atom.text.chars().any(|c| {
                        c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '\\' | ':')
                    })
                    || lexer::Operator::from_word(&atom.text).is_some();
                if hostile {
                    let escaped = atom.text.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("{prefix}\"{escaped}\"")
                } else {
                    format!("{prefix}{}", atom.text)
                }
            }
            Self::Metadata(atom) => {
                let negation = if atom.negated { "-" } else { "" };
                let hostile = atom.value.is_empty()
                    || atom.value.starts_with('-')
                    || atom.value.chars().any(|c| {
                        c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '\\' | ':')
                    });
                if hostile {
                    let escaped = atom.value.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("{negation}{}:\"{escaped}\"", atom.key)
                } else {
                    format!("{negation}{}:{}", atom.key, atom.value)
                }
            }
            Self::Not(inner) => format!("NOT ({})", inner.to_query_string()),
            Self::And(operands) => format!(
                "({})",
                operands
                    .iter()
                    .map(BooleanExpr::to_query_string)
                    .collect::<Vec<_>>()
                    .join(" AND ")
            ),
            Self::Or(branches) => format!(
                "({})",
                branches
                    .iter()
                    .map(BooleanExpr::to_query_string)
                    .collect::<Vec<_>>()
                    .join(" OR ")
            ),
        }
    }
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

    fn boolean(input: &str) -> Option<BooleanExpr> {
        let query = SearchQuery::parse(input);
        assert_eq!(
            query.state,
            QueryState::Ready,
            "diagnostics: {:?}",
            query.diagnostics
        );
        query.boolean
    }

    fn content(text: &str) -> BooleanExpr {
        BooleanExpr::Content(ContentAtom {
            regex: false,
            text: text.into(),
        })
    }

    #[test]
    fn boolean_precedence_not_and_or() {
        assert_eq!(
            boolean("a OR b AND c"),
            Some(BooleanExpr::Or(vec![
                content("a"),
                BooleanExpr::And(vec![content("b"), content("c")]),
            ]))
        );
        assert_eq!(
            boolean("NOT a AND b"),
            Some(BooleanExpr::And(vec![
                BooleanExpr::Not(Box::new(content("a"))),
                content("b"),
            ]))
        );
        assert_eq!(
            boolean("NOT (a AND b)"),
            Some(BooleanExpr::Not(Box::new(BooleanExpr::And(vec![
                content("a"),
                content("b")
            ]))))
        );
    }

    #[test]
    fn boolean_grouping_and_juxtaposition() {
        assert_eq!(
            boolean("(a OR b) AND c"),
            Some(BooleanExpr::And(vec![
                BooleanExpr::Or(vec![content("a"), content("b")]),
                content("c"),
            ]))
        );
        // Juxtaposition is an implicit AND; a bare-word run stays one phrase.
        assert_eq!(
            boolean("(a OR b) parser"),
            Some(BooleanExpr::And(vec![
                BooleanExpr::Or(vec![content("a"), content("b")]),
                content("parser"),
            ]))
        );
        assert_eq!(
            boolean("foo bar AND baz"),
            Some(BooleanExpr::And(vec![content("foo bar"), content("baz")]))
        );
    }

    #[test]
    fn operator_free_queries_are_untouched() {
        // Lowercase operator words, punctuation, parens and quoted
        // operators stay literal content (0063 §3 compatibility).
        for literal in [
            "foo and bar",
            "not a test",
            "!important",
            "a|b",
            "func(x)",
            "foo(1).txt",
        ] {
            let query = SearchQuery::parse(literal);
            assert_eq!(query.state, QueryState::Ready, "{literal}");
            assert!(query.boolean.is_none(), "{literal}");
            assert_eq!(
                query.content,
                Some(ContentExpr::Literal(literal.to_string())),
                "{literal}"
            );
        }
    }

    #[test]
    fn quoted_operator_word_stays_literal_content() {
        let query = SearchQuery::parse("foo \"AND\" bar");
        assert_eq!(query.state, QueryState::Ready);
        assert!(query.boolean.is_none());
        // Quotes are syntax: the phrase carries the word, not the quotes.
        assert_eq!(
            query.content,
            Some(ContentExpr::Literal("foo AND bar".into()))
        );
    }

    #[test]
    fn qualifier_values_stay_opaque_to_grouping() {
        let query = SearchQuery::parse("glob:**/(1)/*.rs AND parser");
        assert_eq!(query.state, QueryState::Ready);
        let expr = query.boolean.expect("operators present");
        assert_eq!(
            expr,
            BooleanExpr::And(vec![
                BooleanExpr::Metadata(MetadataAtom {
                    key: "glob".into(),
                    value: "**/(1)/*.rs".into(),
                    negated: false,
                }),
                content("parser"),
            ])
        );
    }

    #[test]
    fn quoted_operands_are_atoms() {
        let query = SearchQuery::parse("text:\"retry request\" NOT text:\"test\"");
        assert_eq!(query.state, QueryState::Ready);
        assert_eq!(
            query.boolean,
            Some(BooleanExpr::And(vec![
                BooleanExpr::Content(ContentAtom {
                    regex: false,
                    text: "retry request".into(),
                }),
                BooleanExpr::Not(Box::new(BooleanExpr::Content(ContentAtom {
                    regex: false,
                    text: "test".into(),
                }))),
            ]))
        );
    }

    #[test]
    fn dangling_operators_and_unbalanced_groups_diagnose_located() {
        for (input, message) in [
            ("foo AND", "AND without a right operand"),
            ("AND foo", "AND without a left operand"),
            ("a OR", "OR without a right operand"),
            ("NOT", "NOT without an operand"),
            ("(foo AND bar", "unclosed group"),
            ("foo OR bar)", "unbalanced )"),
        ] {
            let query = SearchQuery::parse(input);
            assert_ne!(query.state, QueryState::Ready, "{input}");
            assert!(
                query
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(message)),
                "{input}: {:?}",
                query.diagnostics
            );
        }
    }

    #[test]
    fn parse_format_parse_round_trips() {
        for input in [
            "a OR b AND c",
            "NOT a AND b",
            "(a OR b) AND c",
            "(a AND b) OR (c AND NOT d)",
            "text:\"retry request\" NOT text:\"test\"",
            "language:rust AND NOT glob:**/vendor/** AND parser",
            "(language:python OR language:cpp OR language:lua) parser",
        ] {
            let first = SearchQuery::parse(input);
            assert_eq!(first.state, QueryState::Ready, "{input}");
            let canonical = first
                .boolean
                .as_ref()
                .expect("operators present")
                .to_query_string();
            let second = SearchQuery::parse(&canonical);
            assert_eq!(second.state, QueryState::Ready, "{input} -> {canonical}");
            assert_eq!(first.boolean, second.boolean, "{input} -> {canonical}");
        }
    }
}
