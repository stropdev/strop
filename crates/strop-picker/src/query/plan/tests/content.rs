//! The content plan: which lines a search admits, its provider
//! pattern and its replacement target.

use super::*;

#[test]
fn boolean_path_scopes_share_the_selected_case() {
    for input in [
        "path:src/ AND needle",
        "case:insensitive path:SRC/ AND needle",
    ] {
        let plan = content_plan(input);
        assert!(admits_line(&plan, "src/lib.rs", "NEEDLE"), "{input}");
        assert!(admits_line(&plan, "Src/lib.rs", "needle"), "{input}");
        assert!(!admits_line(&plan, "tests/lib.rs", "needle"), "{input}");
    }
    let sensitive = content_plan("case:sensitive path:SRC/ AND NEEDLE");
    assert!(admits_line(&sensitive, "SRC/lib.rs", "NEEDLE"));
    assert!(!admits_line(&sensitive, "src/lib.rs", "NEEDLE"));
    assert!(!admits_line(&sensitive, "SRC/lib.rs", "needle"));
}

#[test]
fn fuzzy_path_is_the_same_predicate_in_boolean_content_search() {
    let plan = content_plan("fpath:modules/jweb/ AND parser");
    assert!(admits_line(
        &plan,
        "src/modules/jweb/Parser.java",
        "class Parser {}"
    ));
    assert!(!admits_line(
        &plan,
        "src/modules/other/Parser.java",
        "class Parser {}"
    ));
    let excluded = content_plan("NOT fpath:modules/jweb/ AND parser");
    assert!(admits_line(
        &excluded,
        "src/modules/other/Parser.java",
        "class Parser {}"
    ));
    assert!(!admits_line(
        &excluded,
        "src/modules/jweb/Parser.java",
        "class Parser {}"
    ));
}

#[test]
fn filter_only_content_surface_needs_an_expression() {
    let query = crate::query::parse_search("language:rust");
    let content = ContentPlan::compile(&query).unwrap();
    assert!(content.is_none());
}

#[test]
fn literal_is_escaped_and_smart_case_by_default() {
    // one capital makes case count; an all-lowercase literal folds
    let capital = content_plan("A.b");
    assert!(capital.matches_line("A.b"));
    assert!(!capital.matches_line("a.b"));
    for input in ["a.b", "case:ignore A.B", "case:insensitive A.b"] {
        let query = crate::query::parse_search(input);
        let plan = ContentPlan::compile(&query).unwrap().unwrap();
        assert!(plan.matches_line("a.b"), "{input}");
        assert!(plan.matches_line("A.B"), "{input}");
        assert!(!plan.matches_line("aXb"), "{input}");
    }
    let sensitive = content_plan("case:sensitive A.B");
    assert!(sensitive.matches_line("A.B"));
    assert!(!sensitive.matches_line("a.b"));
}

#[test]
fn explicit_regex_and_case() {
    let query = crate::query::parse_search("case:sensitive regex:\"\\brequest_(id|name)\\b\"");
    let plan = ContentPlan::compile(&query).unwrap().unwrap();
    assert!(plan.matches_line("request_id"));
    assert!(!plan.matches_line("REQUEST_ID"));
}

#[test]
fn invalid_regex_is_a_diagnostic() {
    let query = crate::query::parse_search("regex:\"(unclosed\"");
    assert!(ContentPlan::compile(&query).is_err());
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
        let provider =
            super::super::build_regex(plan.provider_pattern(), plan.provider_case()).unwrap();
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
                    provider.is_match(line),
                    "{input}: prefilter missed admitted line {line:?}"
                );
            }
        }
    }
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
    let with_catalog = Evidence {
        catalog: Some(&catalog),
        ..Evidence::default()
    };
    let plan = content_plan("(repo:engine OR repo:tools) NOT glob:**/vendor/** parser");
    // Exact admission with the catalog: branch-sensitive.
    assert!(plan.admits(&line("engine/src/a.rs", "parser here"), &with_catalog));
    assert!(plan.admits(&line("tools/b.py", "parser here"), &with_catalog));
    assert!(!plan.admits(&line("other/c.rs", "parser here"), &with_catalog));
    assert!(!plan.admits(&line("engine/vendor/x.rs", "parser here"), &with_catalog));
    // NOT repo: inverts exactly with a catalog.
    let outside = content_plan("NOT repo:engine AND parser");
    assert!(!outside.admits(&line("engine/src/a.rs", "parser"), &with_catalog));
    assert!(outside.admits(&line("tools/b.py", "parser"), &with_catalog));
    // Without a catalog (remote until 0058's worker): repo: is
    // Unknown — the line admits when the decided atoms allow it.
    assert!(admits_line(&plan, "other/c.rs", "parser here"));
    assert!(!admits_line(&plan, "other/c.rs", "nothing"));
}

#[test]
fn flat_kind_and_repo_stay_flat_beside_bare_text() {
    // 0109 decision 5: a filter never turns the words beside it into a
    // Boolean phrase; the content stays the expression.
    let query = crate::query::parse_search("kind:class needle");
    assert_eq!(query.state, QueryState::Ready);
    assert_eq!(query.boolean, None);
    assert_eq!(query.kinds.len(), 1);
    let repo = crate::query::parse_search("repo:engine parser");
    assert_eq!(repo.boolean, None);
    assert_eq!(repo.repos, ["engine"]);
    // Without evidence the filter keeps the line (Unknown, 0063 §4);
    // the expression still binds.
    let plan = ContentPlan::compile(&query).unwrap().unwrap();
    assert!(plan.needs_symbols());
    assert!(admits_line(&plan, "src/a.rs", "class Parser { needle }"));
    assert!(!admits_line(&plan, "src/a.rs", "class Parser {}"));
    // Filters alone are not a search.
    assert!(!crate::query::parse_search("kind:function").has_positive_content());
    assert!(!crate::query::parse_search("path:src NOT x").has_positive_content());
    assert!(crate::query::parse_search("kind:function x").has_positive_content());
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
    fn inside(
        plan: &ContentPlan,
        index: &crate::source::symbols::SymbolIndex,
        path: &str,
        at: usize,
        text: &str,
    ) -> bool {
        let row = RowFacts {
            line: Some(at),
            ..line(path, text)
        };
        let evidence = Evidence {
            symbols: Some(index),
            ..Evidence::default()
        };
        plan.admits(&row, &evidence)
    }
    let index = crate::source::symbols::SymbolIndex::build(
        root,
        &[std::path::PathBuf::from("src/lib.rs")],
        &|| false,
    );

    let plan = ContentPlan::compile(&crate::query::parse_search("kind:function parser"))
        .unwrap()
        .unwrap();
    assert!(plan.needs_symbols());
    // Line 3 (inside `wrap`): content and kind both Yes.
    assert!(inside(
        &plan,
        &index,
        "src/lib.rs",
        3,
        "    let parser = 1;"
    ));
    // Line 5 (`bare`, outside every function): complete evidence
    // answers No — the line drops.
    assert!(!inside(
        &plan,
        &index,
        "src/lib.rs",
        5,
        "let bare = parser;"
    ));
    // Unknown file: overfetch, never a silent drop.
    assert!(inside(&plan, &index, "src/other.rs", 1, "parser"));
    // No index at all: Unknown admits.
    assert!(admits_line(&plan, "src/lib.rs", "let bare = parser;"));

    // NOT inverts exactly under complete evidence (0063 §4): a
    // parser line outside functions now matches.
    let negated = ContentPlan::compile(&crate::query::parse_search("NOT kind:function AND parser"))
        .unwrap()
        .unwrap();
    assert!(inside(
        &negated,
        &index,
        "src/lib.rs",
        5,
        "let bare = parser;"
    ));
    assert!(!inside(
        &negated,
        &index,
        "src/lib.rs",
        3,
        "    let parser = 1;"
    ));

    // Branches stay sensitive: struct line matches the OR, function
    // line does not.
    let branches =
        ContentPlan::compile(&crate::query::parse_search("(kind:class OR kind:struct) x"))
            .unwrap()
            .unwrap();
    assert!(inside(
        &branches,
        &index,
        "src/lib.rs",
        1,
        "struct St; // x"
    ));
    assert!(!inside(&branches, &index, "src/lib.rs", 3, "let x = 1;"));

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
    let methods = ContentPlan::compile(&crate::query::parse_search("kind:method AND k"))
        .unwrap()
        .unwrap();
    assert!(inside(
        &methods,
        &index,
        "src/impl.rs",
        4,
        "        let k = 1;"
    ));
    // An unknown kind value is an error, never a filter that keeps
    // everything (0109 cause A).
    let banana = crate::query::parse_search("kind:banana AND k");
    assert_eq!(banana.state, QueryState::Invalid);
    assert_eq!(banana.diagnostics[0].message, "unknown kind: banana");
}

#[test]
fn plain_queries_never_build_the_symbol_index() {
    let plan = ContentPlan::compile(&crate::query::parse_search("parser AND request"))
        .unwrap()
        .unwrap();
    assert!(!plan.needs_symbols());
    let flat = ContentPlan::compile(&crate::query::parse_search("parser"))
        .unwrap()
        .unwrap();
    assert!(!flat.needs_symbols());
    let repo_only = ContentPlan::compile(&crate::query::parse_search("repo:engine AND parser"))
        .unwrap()
        .unwrap();
    assert!(!repo_only.needs_symbols());
}

#[test]
fn boolean_case_modes_apply_to_atoms() {
    let smart = content_plan("text:Parser AND text:request");
    assert!(admits_line(&smart, "x.rs", "Parser REQUEST"));
    assert!(!admits_line(&smart, "x.rs", "parser request"));
    for input in [
        "text:parser AND text:request",
        "case:ignore text:Parser AND text:Request",
        "case:insensitive text:Parser AND text:Request",
    ] {
        let insensitive = content_plan(input);
        assert!(
            admits_line(&insensitive, "x.rs", "Parser Request"),
            "{input}"
        );
        assert!(
            admits_line(&insensitive, "x.rs", "parser request"),
            "{input}"
        );
        assert!(
            admits_line(&insensitive, "x.rs", "PARSER REQUEST"),
            "{input}"
        );
    }
    let sensitive = content_plan("case:sensitive text:Parser AND text:Request");
    assert!(admits_line(&sensitive, "x.rs", "Parser Request"));
    assert!(!admits_line(&sensitive, "x.rs", "parser request"));
    let smart = content_plan("case:smart text:Parser AND text:Request");
    assert!(admits_line(&smart, "x.rs", "Parser Request"));
    assert!(!admits_line(&smart, "x.rs", "parser request"));
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
        ContentPlan::compile(&crate::query::parse_search(input))
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
    let ReplacementTarget::Single(re) = target("needle") else {
        panic!("single");
    };
    assert!(re.is_match("needle"));
    assert!(re.is_match("Needle"));
    assert!(re.is_match("NEEDLE"));
    let ReplacementTarget::Single(re) = target("Needle") else {
        panic!("single");
    };
    assert!(re.is_match("Needle"));
    assert!(!re.is_match("needle"));
}

#[test]
fn wsymbols_probe_carries_only_bare_content_text() {
    let probe = |input: &str| crate::query::parse_search(input).wsymbols_probe();
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
