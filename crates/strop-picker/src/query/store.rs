//! Stored-query records (0063 §3): a saved query carries the syntax
//! version it was written in, so a grammar change never silently
//! re-reads it. Version 1 is the pre-Boolean flat literal semantics
//! (0051 §3 — `AND`/`OR`/`NOT` were ordinary words and
//! `kind:`/`type:`/`repo:` were unknown qualifiers); version 2 is the
//! Boolean AST grammar. The wire shape follows strop-core's
//! `path_serde` precedent: an explicit integer version field, typed
//! rejection of versions this build cannot read, and a serde-level
//! wire-shape pin. Queries are not persisted anywhere yet; this is the
//! record every future persistence (search history, sessions) uses.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::lexer::{self, TokenKind};
use super::parser::{CaseMode, QueryState, SearchQuery};

/// Syntax version 1: the pre-Boolean flat literal semantics (0051 §3).
pub const SYNTAX_VERSION_FLAT: u32 = 1;
/// Syntax version 2: the current grammar — the 0063 §3 Boolean AST with
/// `kind:`/`repo:` narrowing. Bump this when a grammar change could
/// reclassify a stored token, and add a migration beside [`migrate_v1`].
pub const SYNTAX_VERSION_CURRENT: u32 = 2;

/// A persisted query string plus the syntax version it was written in.
/// Deserialization rejects unknown versions; a version 1 source migrates
/// at parse time via [`migrate_v1`], never silently re-reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredQuery {
    pub syntax_version: u32,
    pub source: String,
}

impl<'de> Deserialize<'de> for StoredQuery {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            syntax_version: u32,
            source: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::checked(wire.syntax_version, wire.source).map_err(serde::de::Error::custom)
    }
}

/// A stored query this build cannot read. Carries the offending version;
/// the caller surfaces it instead of guessing at the text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoredQueryError {
    #[error(
        "stored query syntax version {found} is unsupported (this build reads {SYNTAX_VERSION_FLAT}..={SYNTAX_VERSION_CURRENT})"
    )]
    UnsupportedVersion { found: u32 },
}

impl StoredQuery {
    /// Record a query string written in the current syntax.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            syntax_version: SYNTAX_VERSION_CURRENT,
            source: source.into(),
        }
    }

    /// Record a parsed query in canonical form: query-wide options in a
    /// leading preamble, the Boolean expression fully parenthesized.
    pub fn from_query(query: &SearchQuery) -> Self {
        Self::new(canonical_source(query))
    }

    /// Checked construction: the one validation gate every decode path
    /// funnels through.
    pub fn checked(syntax_version: u32, source: String) -> Result<Self, StoredQueryError> {
        match syntax_version {
            SYNTAX_VERSION_FLAT | SYNTAX_VERSION_CURRENT => Ok(Self {
                syntax_version,
                source,
            }),
            found => Err(StoredQueryError::UnsupportedVersion { found }),
        }
    }

    /// The source in the current syntax: borrowed verbatim for current
    /// records, migrated for version 1 records.
    pub fn current_source(&self) -> Result<Cow<'_, str>, StoredQueryError> {
        match self.syntax_version {
            SYNTAX_VERSION_CURRENT => Ok(Cow::Borrowed(&self.source)),
            SYNTAX_VERSION_FLAT => Ok(Cow::Owned(migrate_v1(&self.source))),
            found => Err(StoredQueryError::UnsupportedVersion { found }),
        }
    }

    /// Parse the stored query with its recorded version's semantics.
    pub fn parse(&self) -> Result<SearchQuery, StoredQueryError> {
        Ok(SearchQuery::parse(self.current_source()?.as_ref()))
    }
}

/// Migrate a version 1 (flat literal) source to the current syntax.
///
/// The invariant: every token keeps its version 1 classification, so the
/// old literal meaning survives verbatim. Standalone `AND`/`OR`/`NOT`
/// words (literal text in v1, operators in v2) are quoted, as are
/// `kind:`/`type:`/`repo:`-shaped words (unknown qualifiers — a located
/// error — in v1, narrowing qualifiers in v2; quoting keeps them the
/// literal text the user typed instead of silently acquiring scope).
/// Quoting the operator words also keeps zero-depth parens literal:
/// grouping mode triggers only on unquoted operator words, and none
/// survive. Already-quoted forms pass through untouched. Token
/// boundaries come from the one shared lexer — there is no second
/// grammar. Pure: same input, same output.
pub fn migrate_v1(source: &str) -> String {
    let tokens = lexer::lex(source);
    let mut migrated = String::with_capacity(source.len());
    let mut cursor = 0;
    for token in &tokens {
        migrated.push_str(&source[cursor..token.range.start]);
        let slice = &source[token.range.clone()];
        match &token.kind {
            TokenKind::Operator(_) => quote_literal(&mut migrated, slice),
            TokenKind::Qualifier { key, .. }
                if matches!(key.as_str(), "kind" | "type" | "repo") =>
            {
                quote_literal(&mut migrated, slice)
            }
            _ => migrated.push_str(slice),
        }
        cursor = token.range.end;
    }
    migrated.push_str(&source[cursor..]);
    migrated
}

/// Wrap a raw source slice in double quotes, escaping the two characters
/// the lexer resolves inside quotes (the active quote and the backslash,
/// 0051 §3). The re-lexed word text equals the slice.
fn quote_literal(out: &mut String, slice: &str) {
    out.push('"');
    for c in slice.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// Canonical form of a Ready query: the query-wide options
/// (`case:`/`hidden:`/`ignored:`) in a leading preamble — they stay
/// outside Boolean branches (0063 §3) — followed by the canonical
/// Boolean expression, or the flat source with the option tokens
/// removed. Re-parses to the same query and is a fixed point.
/// Non-Ready queries keep their source verbatim: there is nothing to
/// canonicalize while typing or on an error.
pub fn canonical_source(query: &SearchQuery) -> String {
    if query.state != QueryState::Ready {
        return query.source.clone();
    }
    let mut canonical = String::new();
    if let Some(mode) = query.case {
        canonical.push_str("case:");
        canonical.push_str(match mode {
            CaseMode::Smart => "smart",
            CaseMode::Sensitive => "sensitive",
            CaseMode::Ignore => "ignore",
        });
    }
    for (key, value) in [("hidden", query.hidden), ("ignored", query.ignored)] {
        if let Some(include) = value {
            if !canonical.is_empty() {
                canonical.push(' ');
            }
            canonical.push_str(key);
            canonical.push(':');
            canonical.push_str(if include { "include" } else { "exclude" });
        }
    }
    let body = match &query.boolean {
        Some(boolean) => boolean.to_query_string(),
        None => flat_body(query),
    };
    if !body.is_empty() {
        if !canonical.is_empty() {
            canonical.push(' ');
        }
        canonical.push_str(&body);
    }
    canonical
}

/// The flat source without the query-wide option tokens (they moved to
/// the preamble), one space between the kept tokens. Slices are verbatim:
/// quotes and escapes survive untouched.
fn flat_body(query: &SearchQuery) -> String {
    query
        .tokens
        .iter()
        .filter(|token| {
            !matches!(&token.kind, TokenKind::Qualifier { key, negated, .. } if !negated && matches!(key.as_str(), "case" | "hidden" | "ignored"))
        })
        .map(|token| &query.source[token.range.clone()])
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::ContentExpr;

    fn parse_ready(source: &str) -> SearchQuery {
        let query = SearchQuery::parse(source);
        assert_eq!(
            query.state,
            QueryState::Ready,
            "{source}: {:?}",
            query.diagnostics
        );
        query
    }

    /// The v1→v2 migration corpus: previously literal operator words,
    /// zero-depth parens, quoted forms and newly recognized qualifiers
    /// all keep their v1 meaning (0063 §6.1 grammar tier).
    #[test]
    fn v1_migration_corpus_preserves_literal_meaning() {
        let cases: &[(&str, &str, &str)] = &[
            // (v1 source, migrated source, v1 literal phrase)
            ("foo AND bar", "foo \"AND\" bar", "foo AND bar"),
            ("NOT ready", "\"NOT\" ready", "NOT ready"),
            ("x OR y AND z", "x \"OR\" y \"AND\" z", "x OR y AND z"),
            ("AND", "\"AND\"", "AND"),
            // Zero-depth parens stay literal once operators are quoted.
            ("(a OR b)", "(a \"OR\" b)", "(a OR b)"),
            ("func(x) OR", "func(x) \"OR\"", "func(x) OR"),
            ("foo) AND bar", "foo) \"AND\" bar", "foo) AND bar"),
            // No operator word: grouping never triggers, parens pass through.
            ("foo) bar", "foo) bar", "foo) bar"),
            ("foo(1).txt", "foo(1).txt", "foo(1).txt"),
            // Quoted operator forms were already literal: untouched.
            ("\"AND\" but", "\"AND\" but", "AND but"),
            // Lowercase operator words are literal in both versions.
            ("foo and bar", "foo and bar", "foo and bar"),
            // Newly recognized qualifiers stay the literal text typed in
            // v1 (an unquoted `kind:` was a located error, never scope).
            (
                "kind:function handle",
                "\"kind:function\" handle",
                "kind:function handle",
            ),
            ("-repo:main x", "\"-repo:main\" x", "-repo:main x"),
            // A trailing backslash escapes inside quotes instead of
            // swallowing the closing quote.
            ("kind:a\\", "\"kind:a\\\\\"", "kind:a\\"),
        ];
        for &(v1, migrated, phrase) in cases {
            assert_eq!(migrate_v1(v1), migrated, "{v1}");
            let query = parse_ready(migrated);
            assert_eq!(query.boolean, None, "{v1}: operators must not survive");
            assert_eq!(
                query.content,
                Some(ContentExpr::Literal(phrase.to_string())),
                "{v1}"
            );
            // Migration is a fixed point.
            assert_eq!(migrate_v1(migrated), migrated, "{v1}");
        }
    }

    #[test]
    fn v1_migration_keeps_flat_qualifiers_and_typing_states() {
        // A qualifier known in v1 keeps its qualifier role; the operator
        // word beside it becomes literal text.
        let query = parse_ready(&migrate_v1("language:rust OR"));
        assert_eq!(query.languages, ["rust"]);
        assert_eq!(query.content, Some(ContentExpr::Literal("OR".into())));
        // An unclosed quote is typing in progress in both versions.
        let source = "foo \"AND";
        assert_eq!(migrate_v1(source), source);
        assert_eq!(SearchQuery::parse(source).state, QueryState::Incomplete);
    }

    /// §6.1: parse → format → parse over migrated output is a fixed
    /// point, and the round-trip preserves the query.
    #[test]
    fn migrated_output_parse_format_parse() {
        for v1 in [
            "foo AND bar",
            "(a OR b) NOT c",
            "kind:function handle",
            "language:rust x OR y",
            "case:smart needle AND thread",
        ] {
            let migrated = StoredQuery {
                syntax_version: SYNTAX_VERSION_FLAT,
                source: v1.to_string(),
            }
            .parse()
            .unwrap();
            assert_eq!(migrated.state, QueryState::Ready, "{v1}");
            let canonical = canonical_source(&migrated);
            let reparsed = parse_ready(&canonical);
            assert_eq!(reparsed.content, migrated.content, "{v1} -> {canonical}");
            assert_eq!(reparsed.boolean, migrated.boolean, "{v1} -> {canonical}");
            assert!(migrated.same_scope(&reparsed), "{v1} -> {canonical}");
            assert_eq!(reparsed.case, migrated.case, "{v1} -> {canonical}");
            assert_eq!(
                canonical_source(&reparsed),
                canonical,
                "fixed point: {v1} -> {canonical}"
            );
        }
    }

    #[test]
    fn unknown_versions_are_typed_rejections_never_silent_reads() {
        // One past the end is rejected at the serde boundary.
        let future = serde_json::json!({
            "syntax_version": SYNTAX_VERSION_CURRENT + 1,
            "source": "foo",
        });
        let error = serde_json::from_value::<StoredQuery>(future)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&(SYNTAX_VERSION_CURRENT + 1).to_string()),
            "{error}"
        );
        // Version 0 never existed.
        let zero = serde_json::json!({"syntax_version": 0, "source": "foo"});
        assert!(serde_json::from_value::<StoredQuery>(zero).is_err());
        // A hand-built record fails at use with the same typed error.
        let record = StoredQuery {
            syntax_version: 99,
            source: "foo".to_string(),
        };
        assert_eq!(
            record.parse().unwrap_err(),
            StoredQueryError::UnsupportedVersion { found: 99 }
        );
        assert_eq!(
            record.current_source().unwrap_err(),
            StoredQueryError::UnsupportedVersion { found: 99 }
        );
    }

    /// Serde-level wire-shape pin (session/tests.rs precedent): the
    /// version field is present and current, unknown fields are
    /// rejected, and a record round-trips.
    #[test]
    fn wire_shape_pins_the_version_field() {
        let record = StoredQuery::new("case:smart (foo AND bar)");
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "syntax_version": SYNTAX_VERSION_CURRENT,
                "source": "case:smart (foo AND bar)",
            })
        );
        let with_extra = serde_json::json!({
            "syntax_version": SYNTAX_VERSION_CURRENT,
            "source": "x",
            "surprise": true,
        });
        assert!(serde_json::from_value::<StoredQuery>(with_extra).is_err());
        let missing_version = serde_json::json!({"source": "x"});
        assert!(serde_json::from_value::<StoredQuery>(missing_version).is_err());
        let back: StoredQuery = serde_json::from_value(value).unwrap();
        assert_eq!(back, record);
    }

    /// The canonical formatter places query-wide options in a leading
    /// preamble, outside Boolean branches (0063 §3).
    #[test]
    fn canonical_form_places_query_wide_options_in_a_leading_preamble() {
        let query = parse_ready("foo AND case:ignore bar hidden:include");
        assert_eq!(query.case, Some(CaseMode::Ignore));
        assert_eq!(query.hidden, Some(true));
        let canonical = canonical_source(&query);
        assert_eq!(canonical, "case:ignore hidden:include (foo AND bar)");
        let reparsed = parse_ready(&canonical);
        assert_eq!(reparsed.boolean, query.boolean);
        assert_eq!(reparsed.case, query.case);
        assert_eq!(reparsed.hidden, query.hidden);
        assert_eq!(canonical_source(&reparsed), canonical);

        // Flat queries hoist the same options; kept tokens are verbatim.
        let query = parse_ready("needle case:sensitive  glob:**/*.rs");
        let canonical = canonical_source(&query);
        assert_eq!(canonical, "case:sensitive needle glob:**/*.rs");
        let reparsed = parse_ready(&canonical);
        assert_eq!(reparsed.content, query.content);
        assert_eq!(reparsed.globs, query.globs);
        assert_eq!(reparsed.case, query.case);
        assert_eq!(canonical_source(&reparsed), canonical);

        // Options only: the body is empty, the preamble is the query.
        let query = parse_ready("hidden:exclude");
        assert_eq!(canonical_source(&query), "hidden:exclude");

        // Nothing to canonicalize while typing or on an error.
        let incomplete = SearchQuery::parse("foo \"bar");
        assert_eq!(canonical_source(&incomplete), "foo \"bar");
    }

    /// Query-wide options are never branch operands (0063 §3): an
    /// option beside an operator does not satisfy its arity, and a
    /// trailing option does not turn the operator into an error.
    #[test]
    fn options_are_never_boolean_branch_operands() {
        let query = parse_ready("foo AND case:smart");
        assert_eq!(
            query.boolean,
            Some(crate::query::parser::BooleanExpr::Content(
                crate::query::parser::ContentAtom {
                    regex: false,
                    text: "foo".into(),
                }
            ))
        );
        assert_eq!(canonical_source(&query), "case:smart foo");

        for (input, message) in [
            ("case:smart AND foo", "AND without a left operand"),
            ("foo OR case:smart", "OR without a right operand"),
        ] {
            let query = SearchQuery::parse(input);
            assert_eq!(query.state, QueryState::Invalid, "{input}");
            assert!(
                query
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains(message)),
                "{input}: {:?}",
                query.diagnostics
            );
        }
    }

    #[test]
    fn stored_query_parses_with_its_recorded_versions_semantics() {
        // The same source means different things per version — that is
        // the point of carrying one.
        let v1 = StoredQuery {
            syntax_version: SYNTAX_VERSION_FLAT,
            source: "foo AND bar".to_string(),
        };
        let flat = v1.parse().unwrap();
        assert_eq!(flat.boolean, None);
        assert_eq!(
            flat.content,
            Some(ContentExpr::Literal("foo AND bar".into()))
        );
        assert_eq!(v1.current_source().unwrap().as_ref(), "foo \"AND\" bar");

        let v2 = StoredQuery::new("foo AND bar");
        assert!(v2.parse().unwrap().boolean.is_some());
        assert_eq!(v2.current_source().unwrap().as_ref(), "foo AND bar");

        // The write side stores canonical form in the current version.
        let record = StoredQuery::from_query(&parse_ready("foo AND bar case:smart"));
        assert_eq!(record.syntax_version, SYNTAX_VERSION_CURRENT);
        assert_eq!(record.source, "case:smart (foo AND bar)");
    }
}
