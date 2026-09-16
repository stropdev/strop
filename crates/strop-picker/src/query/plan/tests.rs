use super::*;
use crate::query::parser::SearchQuery;

fn plan(input: &str) -> (SearchQuery, FileSelectionPlan) {
    let query = SearchQuery::parse(input);
    let plan = FileSelectionPlan::compile(&query).unwrap();
    (query, plan)
}

#[test]
fn language_filters_by_extension_family() {
    let (_, plan) = plan("language:rust retry");
    assert!(plan.allows("src/main.rs"));
    assert!(!plan.allows("src/main.cpp"));
    assert!(!plan.allows("build/out.txt"));
}

#[test]
fn path_substring_and_negation() {
    let (_, plan) = plan("path:src/ -path:generated retry");
    assert!(plan.allows("src/a.rs"));
    assert!(!plan.allows("src/generated/b.rs"));
    assert!(!plan.allows("tests/a.rs"));
}

#[test]
fn globs_and_exclusions_win() {
    let (_, plan) = plan("glob:\"**/*.rs\" -glob:\"**/*-test.rs\" x");
    assert!(plan.allows("src/a.rs"));
    assert!(!plan.allows("tests/a-test.rs"));
}

#[test]
fn same_family_ors_different_families_and() {
    let (_, plan) = plan("language:rust language:cpp path:src x");
    assert!(plan.allows("src/a.rs"));
    assert!(plan.allows("src/a.cpp"));
    assert!(!plan.allows("src/a.py"));
    assert!(!plan.allows("other/a.rs"));
}

#[test]
fn invalid_glob_is_a_diagnostic() {
    let query = SearchQuery::parse("glob:\"[bad\" x");
    let diagnostic = FileSelectionPlan::compile(&query).unwrap_err();
    assert!(diagnostic.message.contains("invalid glob"));
    assert_eq!(&query.source[diagnostic.range], "glob:\"[bad\"");
}

#[test]
fn filter_only_content_surface_needs_an_expression() {
    let query = SearchQuery::parse("language:rust");
    let content = ContentPlan::compile(&query).unwrap();
    assert!(content.is_none());
}

#[test]
fn literal_is_escaped_and_case_smart_default() {
    let query = SearchQuery::parse("a.b");
    let plan = ContentPlan::compile(&query).unwrap().unwrap();
    let re = plan.regex;
    assert!(re.is_match("a.b"));
    assert!(!re.is_match("aXb"));
    // smart case: lowercase needle matches either case
    assert!(re.is_match("A.B"));
}

#[test]
fn explicit_regex_and_case() {
    let query = SearchQuery::parse("case:sensitive regex:\"\\brequest_(id|name)\\b\"");
    let plan = ContentPlan::compile(&query).unwrap().unwrap();
    let re = plan.regex;
    assert!(re.is_match("request_id"));
    assert!(!re.is_match("REQUEST_ID"));
}

#[test]
fn invalid_regex_is_a_diagnostic() {
    let query = SearchQuery::parse("regex:\"(unclosed\"");
    assert!(ContentPlan::compile(&query).is_err());
}

fn content_plan(input: &str) -> ContentPlan {
    let query = SearchQuery::parse(input);
    assert_eq!(
        query.state,
        super::super::QueryState::Ready,
        "{input}: {:?}",
        query.diagnostics
    );
    ContentPlan::compile(&query)
        .unwrap()
        .expect("content required")
}

fn admits_line(plan: &ContentPlan, path: &str, line: &str) -> bool {
    plan.admits(
        Evidence {
            path: Some(path),
            ..Evidence::default()
        },
        line,
    )
}

#[test]
fn boolean_admits_same_line_conjunction_and_exclusion() {
    // 0063 §4: AND means both literals on ONE logical line; NOT
    // excludes the line. Git-grep line semantics, not file-level.
    let plan = content_plan("text:\"retry\" AND text:\"request\" NOT text:\"test\"");
    assert!(admits_line(
        &plan,
        "src/a.rs",
        "let request = retry(request);"
    ));
    assert!(!admits_line(
        &plan,
        "src/a.rs",
        "let request = retry(request); // test"
    ));
    assert!(!admits_line(&plan, "src/a.rs", "let request = x;"));
    // Different lines are different candidates: neither line alone
    // satisfies the conjunction.
    assert!(!admits_line(&plan, "src/a.rs", "let retry = retry(x);"));
    assert!(!admits_line(&plan, "src/a.rs", "let request = y;"));
}

#[test]
fn boolean_metadata_narrows_by_path_and_language() {
    let plan = content_plan("(language:python OR language:cpp) parser");
    assert!(admits_line(&plan, "pkg/main.py", "class Parser:"));
    assert!(admits_line(&plan, "src/main.cpp", "Parser parser;"));
    assert!(!admits_line(&plan, "src/main.rs", "struct Parser;"));
}

#[test]
fn boolean_branches_do_not_flatten() {
    // (path:a AND foo) OR (path:b AND bar) — the branch relationship
    // is preserved: a-with-bar or b-with-foo do not match (0063 §4).
    let plan = content_plan("(path:alpha AND foo) OR (path:beta AND bar)");
    assert!(admits_line(&plan, "alpha/x.rs", "fn foo() {}"));
    assert!(admits_line(&plan, "beta/y.rs", "fn bar() {}"));
    assert!(!admits_line(&plan, "alpha/x.rs", "fn bar() {}"));
    assert!(!admits_line(&plan, "beta/y.rs", "fn foo() {}"));
}

#[test]
fn not_never_turns_failure_into_a_match() {
    // Unknown evidence (a path with no extension) must not become
    // false inside NOT and admit the line (0063 §4).
    let plan = content_plan("NOT language:rust AND parser");
    assert!(!admits_line(&plan, "src/main.rs", "struct Parser;"));
    assert!(admits_line(&plan, "src/main.py", "class Parser:"));
    assert!(admits_line(&plan, "noext", "parser here"));
}

#[test]
fn provider_prefilter_is_sound_for_every_admitted_line() {
    // Prefilter soundness (0063 §6.2): every line the exact plan
    // admits, the provider pattern also matches — overfetch only.
    for input in [
        "text:\"retry\" AND text:\"request\" NOT text:\"test\"",
        "(path:alpha AND foo) OR (path:beta AND bar)",
        "(language:python OR language:cpp) parser",
        "NOT language:rust AND parser",
        "foo OR bar AND NOT baz",
    ] {
        let plan = content_plan(input);
        let corpus = [
            ("src/a.rs", "let request = retry(request); // test"),
            ("src/a.rs", "let retry = retry(x); let request = y;"),
            ("pkg/main.py", "class Parser:"),
            ("alpha/x.rs", "fn foo() {}"),
            ("beta/y.rs", "fn bar() {}"),
            ("src/main.cpp", "Parser parser;"),
            ("noext", "parser here"),
            ("src/b.rs", "baz foo bar"),
        ];
        for (path, line) in corpus {
            if admits_line(&plan, path, line) {
                assert!(
                    plan.regex.is_match(line),
                    "{input}: prefilter missed admitted line {line:?}"
                );
            }
        }
    }
}

#[test]
fn type_alias_normalizes_to_kind() {
    let query = SearchQuery::parse("type:class AND parser");
    assert_eq!(query.state, super::super::QueryState::Ready);
    assert!(matches!(query.boolean, Some(BooleanExpr::And(_))));
}

#[test]
fn pending_qualifiers_overfetch_inside_and() {
    // kind:/repo: await project discovery: they cannot narrow yet, so
    // they admit (overfetch) while the decided operand still binds.
    let plan = content_plan("kind:class AND parser");
    assert!(admits_line(&plan, "any/x.rs", "a parser here"));
    assert!(!admits_line(&plan, "any/x.rs", "nothing relevant"));
}

#[test]
fn not_never_drops_lines_through_a_pending_qualifier() {
    // The soundness core (0063 §4): unknown evidence under NOT must
    // not become false and silently drop every line.
    let plan = content_plan("NOT kind:function AND parser");
    assert!(admits_line(&plan, "any/x.rs", "a parser here"));
    assert!(!admits_line(&plan, "any/x.rs", "nothing relevant"));
    let widened = content_plan("(repo:engine OR repo:tools) AND parser");
    assert!(admits_line(&widened, "elsewhere/y.py", "parser"));
}

#[test]
fn repo_atoms_decide_with_a_catalog_and_admit_without() {
    let directory = tempfile::tempdir().unwrap();
    let scope = directory.path();
    std::fs::create_dir_all(scope.join("engine/src")).unwrap();
    std::fs::create_dir(scope.join("engine/.git")).unwrap();
    std::fs::create_dir_all(scope.join("tools")).unwrap();
    std::fs::create_dir(scope.join("tools/.git")).unwrap();
    let catalog = crate::source::catalog::ProjectCatalog::discover(scope, &|| false);
    let plan = content_plan("(repo:engine OR repo:tools) NOT glob:**/vendor/** parser");
    // Exact admission with the catalog: branch-sensitive.
    assert!(plan.admits(
        Evidence {
            catalog: Some(&catalog),
            path: Some("engine/src/a.rs"),
            ..Evidence::default()
        },
        "parser here"
    ));
    assert!(plan.admits(
        Evidence {
            catalog: Some(&catalog),
            path: Some("tools/b.py"),
            ..Evidence::default()
        },
        "parser here"
    ));
    assert!(!plan.admits(
        Evidence {
            catalog: Some(&catalog),
            path: Some("other/c.rs"),
            ..Evidence::default()
        },
        "parser here"
    ));
    assert!(!plan.admits(
        Evidence {
            catalog: Some(&catalog),
            path: Some("engine/vendor/x.rs"),
            ..Evidence::default()
        },
        "parser here"
    ));
    // NOT repo: inverts exactly with a catalog.
    let outside = content_plan("NOT repo:engine AND parser");
    assert!(!outside.admits(
        Evidence {
            catalog: Some(&catalog),
            path: Some("engine/src/a.rs"),
            ..Evidence::default()
        },
        "parser"
    ));
    assert!(outside.admits(
        Evidence {
            catalog: Some(&catalog),
            path: Some("tools/b.py"),
            ..Evidence::default()
        },
        "parser"
    ));
    // Without a catalog (remote until 0058's worker): repo: is
    // Unknown — the line admits when the decided atoms allow it.
    assert!(admits_line(&plan, "other/c.rs", "parser here"));
    assert!(!admits_line(&plan, "other/c.rs", "nothing"));
}
#[test]
fn flat_kind_and_repo_are_implicit_ands() {
    // Flat spellings upgrade to the Boolean parse (0063 §2):
    // `kind:class needle` narrows exactly like `kind:class AND
    // needle` — the evidence exists now — and `type:` aliases.
    let query = SearchQuery::parse("kind:class needle");
    assert_eq!(query.state, super::super::QueryState::Ready);
    assert!(query.boolean.is_some());
    let aliased = SearchQuery::parse("type:class needle");
    assert_eq!(aliased.state, super::super::QueryState::Ready);
    assert_eq!(
        aliased.boolean.as_ref().unwrap().to_query_string(),
        query.boolean.as_ref().unwrap().to_query_string()
    );
    let repo = SearchQuery::parse("repo:engine parser");
    assert_eq!(repo.state, super::super::QueryState::Ready);
    assert!(repo.boolean.is_some());
    // Without evidence both still admit (Unknown, 0063 §4).
    let plan = ContentPlan::compile(&query).unwrap().unwrap();
    assert!(admits_line(&plan, "src/a.rs", "class Parser { needle }"));
    assert!(!admits_line(&plan, "src/a.rs", "class Parser {}"));
}

#[test]
fn kind_atoms_narrow_with_symbol_evidence_and_admit_without() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "struct St;\nfn wrap() {\n    let parser = 1;\n}\nlet bare = 2;\n",
    )
    .unwrap();
    fn evidence_for<'a>(
        index: &'a crate::source::symbols::SymbolIndex,
        path: &'a str,
        line: usize,
    ) -> Evidence<'a> {
        Evidence {
            symbols: Some(index),
            path: Some(path),
            line: Some(line),
            ..Evidence::default()
        }
    }
    let index = crate::source::symbols::SymbolIndex::build(
        root,
        &[std::path::PathBuf::from("src/lib.rs")],
        &|| false,
    );

    let plan = ContentPlan::compile(&SearchQuery::parse("kind:function parser"))
        .unwrap()
        .unwrap();
    assert!(plan.needs_symbols());
    // Line 3 (inside `wrap`): content and kind both Yes.
    assert!(plan.admits(evidence_for(&index, "src/lib.rs", 3), "    let parser = 1;"));
    // Line 5 (`bare`, outside every function): complete evidence
    // answers No — the line drops.
    assert!(!plan.admits(evidence_for(&index, "src/lib.rs", 5), "let bare = parser;"));
    // Unknown file: overfetch, never a silent drop.
    assert!(plan.admits(evidence_for(&index, "src/other.rs", 1), "parser"));
    // No index at all: Unknown admits.
    assert!(plan.admits(
        Evidence {
            path: Some("src/lib.rs"),
            line: Some(5),
            ..Evidence::default()
        },
        "let bare = parser;"
    ));

    // NOT inverts exactly under complete evidence (0063 §4): a
    // parser line outside functions now matches.
    let negated = ContentPlan::compile(&SearchQuery::parse("NOT kind:function AND parser"))
        .unwrap()
        .unwrap();
    assert!(negated.admits(evidence_for(&index, "src/lib.rs", 5), "let bare = parser;"));
    assert!(!negated.admits(evidence_for(&index, "src/lib.rs", 3), "    let parser = 1;"));

    // Branches stay sensitive: struct line matches the OR, function
    // line does not.
    let branches = ContentPlan::compile(&SearchQuery::parse("(kind:class OR kind:struct) x"))
        .unwrap()
        .unwrap();
    assert!(branches.admits(evidence_for(&index, "src/lib.rs", 1), "struct St; // x"));
    assert!(!branches.admits(evidence_for(&index, "src/lib.rs", 3), "let x = 1;"));

    // Methods narrow under kind:function and kind:method alike.
    std::fs::write(
        root.join("src/impl.rs"),
        "struct O;\nimpl O {\n    fn m(&self) {\n        let k = 1;\n    }\n}\n",
    )
    .unwrap();
    let index = crate::source::symbols::SymbolIndex::build(
        root,
        &[
            std::path::PathBuf::from("src/lib.rs"),
            std::path::PathBuf::from("src/impl.rs"),
        ],
        &|| false,
    );
    let methods = ContentPlan::compile(&SearchQuery::parse("kind:method AND k"))
        .unwrap()
        .unwrap();
    assert!(methods.admits(evidence_for(&index, "src/impl.rs", 4), "        let k = 1;"));
    let banana = ContentPlan::compile(&SearchQuery::parse("kind:banana AND k"))
        .unwrap()
        .unwrap();
    // An unrecognized kind value stays Unknown — admit, explain
    // via suggestions, never drop.
    assert!(banana.admits(evidence_for(&index, "src/lib.rs", 5), "let k = 2;"));
}

#[test]
fn plain_queries_never_build_the_symbol_index() {
    let plan = ContentPlan::compile(&SearchQuery::parse("parser AND request"))
        .unwrap()
        .unwrap();
    assert!(!plan.needs_symbols());
    let flat = ContentPlan::compile(&SearchQuery::parse("parser"))
        .unwrap()
        .unwrap();
    assert!(!flat.needs_symbols());
    let repo_only = ContentPlan::compile(&SearchQuery::parse("repo:engine AND parser"))
        .unwrap()
        .unwrap();
    assert!(!repo_only.needs_symbols());
}

#[test]
fn boolean_case_modes_apply_to_atoms() {
    let smart = content_plan("text:Parser AND text:Request");
    assert!(admits_line(&smart, "x.rs", "Parser Request"));
    assert!(!admits_line(&smart, "x.rs", "parser request"));
    let ignore = content_plan("case:ignore text:Parser AND text:Request");
    assert!(admits_line(&ignore, "x.rs", "parser request"));
}

#[test]
fn glob_stars_span_unanchored_suffixes() {
    // Found by the 0063 §6.2 reference interpreter: the old
    // first-match scan rejected `*.rs` against `a/src/x.rs` — an
    // unanchored star must resume, not surrender.
    let plan = content_plan("glob:*.rs AND parser");
    assert!(admits_line(&plan, "a/src/x.rs", "parser here"));
    assert!(!admits_line(&plan, "a/src/x.py", "parser here"));
    let nested = content_plan("glob:**/x.rs AND parser");
    assert!(admits_line(&nested, "deep/nest/x.rs", "parser"));
    assert!(!admits_line(&nested, "deep/nest/x.py", "parser"));
    let prefix = content_plan("glob:a/* AND parser");
    assert!(admits_line(&prefix, "a/anything/x.rs", "parser"));
    assert!(!admits_line(&prefix, "b/anything/x.rs", "parser"));
}

#[test]
fn replacement_targets_are_single_positive_atoms_only() {
    use super::ReplacementTarget;
    let target = |input: &str| {
        ContentPlan::compile(&SearchQuery::parse(input))
            .unwrap()
            .unwrap()
            .replacement_target()
    };
    // Simple queries: the content expression is the target.
    assert!(matches!(target("needle"), ReplacementTarget::Single(_)));
    // Boolean with one positive atom: that atom, guarded by
    // negatives and metadata at will.
    assert!(matches!(
        target("needle NOT test"),
        ReplacementTarget::Single(_)
    ));
    assert!(matches!(
        target("language:rust AND kind:function AND needle NOT vendor"),
        ReplacementTarget::Single(_)
    ));
    // Double negation restores positivity.
    assert!(matches!(
        target("NOT (NOT needle)"),
        ReplacementTarget::Single(_)
    ));
    // Several positive atoms: a valid search, an ambiguous replace.
    assert!(matches!(
        target("retry AND request"),
        ReplacementTarget::Ambiguous(2)
    ));
    assert!(matches!(
        target("a OR b OR c"),
        ReplacementTarget::Ambiguous(3)
    ));
    assert!(matches!(
        target("(a OR b) AND c NOT d"),
        ReplacementTarget::Ambiguous(3)
    ));
    // No positive content atom: nothing identifiable to replace.
    assert!(matches!(
        target("language:rust AND NOT kind:function"),
        ReplacementTarget::Missing
    ));
    // The single target carries the query's case behavior.
    let ReplacementTarget::Single(re) = target("case:sensitive needle") else {
        panic!("single");
    };
    assert!(re.is_match("needle"));
    assert!(!re.is_match("Needle"));
}

#[test]
fn wsymbols_probe_carries_only_bare_content_text() {
    let probe = |input: &str| SearchQuery::parse(input).wsymbols_probe();
    // A flat literal crosses as itself — today's behavior.
    assert_eq!(probe("parse").as_deref(), Some("parse"));
    // Regex patterns stay local: empty probe, never metacharacters.
    assert_eq!(probe("regex:par.*").as_deref(), Some(""));
    // Qualifiers and operators never reach the wire.
    assert_eq!(probe("kind:function").as_deref(), Some(""));
    assert_eq!(probe("kind:function parse").as_deref(), Some("parse"));
    assert_eq!(probe("repo:engine AND wrap").as_deref(), Some("wrap"));
    // Positive atoms in tree order; negated and regex atoms drop.
    assert_eq!(probe("foo AND NOT bar").as_deref(), Some("foo"));
    assert_eq!(probe("NOT NOT x").as_deref(), Some("x"));
    assert_eq!(probe("regex:a.* OR b").as_deref(), Some("b"));
    // A broken query asks nothing.
    assert_eq!(probe("NOT"), None);
}

fn candidate(kind: Option<SymbolKind>, qualified: Option<&str>) -> Evidence<'_> {
    Evidence {
        symbol: Some(SymbolEvidence { kind, qualified }),
        ..Evidence::default()
    }
}

#[test]
fn symbol_candidates_decide_kind_from_their_own_classification() {
    let compile = |input: &str| {
        ContentPlan::compile(&SearchQuery::parse(input))
            .unwrap()
            .unwrap()
    };
    let plan = compile("kind:function");
    assert!(plan.admits(candidate(Some(SymbolKind::Function), None), "wrap"));
    // `function` subsumes methods (0063 §2).
    assert!(plan.admits(candidate(Some(SymbolKind::Method), None), "method"));
    assert!(!plan.admits(candidate(Some(SymbolKind::Struct), None), "St"));
    // Negation inverts exactly, never through missing evidence.
    let plan = compile("NOT kind:function");
    assert!(!plan.admits(candidate(Some(SymbolKind::Function), None), "wrap"));
    assert!(plan.admits(candidate(Some(SymbolKind::Struct), None), "St"));
    // An unrecognized kind VALUE stays Unknown and admits.
    let plan = compile("kind:bogus");
    assert!(plan.admits(candidate(Some(SymbolKind::Struct), None), "St"));
    // A provider kind with no canonical mapping stays Unknown.
    let plan = compile("kind:function");
    assert!(plan.admits(candidate(None, None), "anything"));
    let plan = compile("NOT kind:function");
    assert!(plan.admits(candidate(None, None), "anything"));
}

#[test]
fn symbol_content_atoms_match_name_or_qualified_form() {
    let compile = |input: &str| {
        ContentPlan::compile(&SearchQuery::parse(input))
            .unwrap()
            .unwrap()
    };
    // Content + kind combine across the AST.
    let plan = compile("kind:function parse");
    assert!(plan.admits(candidate(Some(SymbolKind::Function), None), "parse_file"));
    assert!(!plan.admits(candidate(Some(SymbolKind::Struct), None), "parse"));
    assert!(!plan.admits(candidate(Some(SymbolKind::Function), None), "wrap"));
    // The qualified/container form carries content evidence where
    // the provider reports one (0063 §4).
    assert!(plan.admits(
        candidate(Some(SymbolKind::Function), Some("Parser::parse")),
        "wrap"
    ));
    // and the qualified form narrows exactly in a Boolean query:
    let plan = compile("kind:method Parser::parse");
    assert!(plan.admits(
        candidate(Some(SymbolKind::Method), Some("Parser::parse")),
        "parse"
    ));
    assert!(!plan.admits(candidate(Some(SymbolKind::Method), None), "parse"));
    // Negative content guards apply to symbols too.
    let plan = compile("parse NOT regex:test");
    assert!(!plan.admits(candidate(Some(SymbolKind::Function), None), "test_parse"));
    assert!(plan.admits(candidate(Some(SymbolKind::Function), None), "parse"));
}
