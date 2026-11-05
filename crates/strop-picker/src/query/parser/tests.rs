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
    let q = SearchQuery::parse("language:rust glob:\"src/my files/**/*.rs\" text:\"request-id\"");
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
fn insensitive_alias_matches_ignore_and_changes_scope_identity() {
    for input in ["case:ignore needle", "case:insensitive needle"] {
        let query = SearchQuery::parse(input);
        assert_eq!(query.state, QueryState::Ready, "{input}");
        assert_eq!(query.case, Some(CaseMode::Ignore));
        assert_eq!(query.case_mode(), CaseMode::Ignore);
        assert!(query
            .highlights
            .iter()
            .all(|span| span.role != super::super::Role::Error));
    }
    let default = SearchQuery::parse("path:src needle");
    let sensitive = SearchQuery::parse("case:sensitive path:src needle");
    assert_eq!(
        SearchQuery::parse("case:ignore case:insensitive needle").state,
        QueryState::Ready
    );
    assert_eq!(default.case_mode(), CaseMode::Ignore);
    assert!(!default.same_scope(&sensitive));
    assert!(default.same_scope(&SearchQuery::parse("case:ignore path:src needle")));
    assert!(default.same_scope(&SearchQuery::parse("case:insensitive path:src needle")));
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
fn boolean_content_qualifier_negation_is_rejected() {
    for input in ["-text:secret AND other", "-regex:secret AND other"] {
        let query = SearchQuery::parse(input);
        assert_eq!(query.state, QueryState::Invalid, "{input}");
        let qualifier_end = input.find(" AND ").unwrap();
        assert!(
            query.diagnostics.iter().any(|diagnostic| {
                (diagnostic.range == (0..qualifier_end))
                    && diagnostic.message.ends_with(": cannot be negated")
            }),
            "{input}: {:?}",
            query.diagnostics
        );
    }
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
