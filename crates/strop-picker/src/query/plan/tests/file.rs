//! The file selection: which paths a walk admits.

use super::*;

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
fn path_scopes_default_to_smart_case_and_honor_overrides() {
    // smart: a capital in the scope makes its case count
    let (_, smart) = plan("path:SRC/");
    assert!(smart.allows("SRC/lib.rs"));
    assert!(!smart.allows("src/lib.rs"));
    for input in [
        "path:src/",
        "case:ignore path:SRC/",
        "case:insensitive path:SRC/",
    ] {
        let (_, plan) = plan(input);
        assert!(plan.allows("src/lib.rs"), "{input}");
        assert!(plan.allows("Src/lib.rs"), "{input}");
        assert!(!plan.allows("tests/lib.rs"), "{input}");
    }
    let (_, sensitive) = plan("case:sensitive path:SRC/");
    assert!(sensitive.allows("SRC/lib.rs"));
    assert!(!sensitive.allows("src/lib.rs"));

    for input in ["fpath:src/lib", "case:insensitive fpath:SRC/LIB"] {
        let (_, plan) = plan(input);
        assert!(plan.allows("src/lib.rs"), "{input}");
        assert!(plan.allows("Src/Lib.rs"), "{input}");
        assert!(!plan.allows("src/main.rs"), "{input}");
    }
    let (_, sensitive) = plan("case:sensitive fpath:SRC/LIB");
    assert!(sensitive.allows("SRC/LIB.rs"));
    assert!(!sensitive.allows("src/lib.rs"));

    let (_, excluded) = plan("-path:src/");
    assert!(!excluded.allows("Src/lib.rs"));
    assert!(excluded.allows("tests/lib.rs"));
    let (_, excluded) = plan("-fpath:src/lib");
    assert!(!excluded.allows("src/lib.rs"));
    assert!(excluded.allows("src/main.rs"));
}

#[test]
fn fuzzy_path_scope_matches_parent_components_without_reinterpreting_path() {
    let (query, fuzzy) = plan("fpath:modules/jweb/ parser");
    assert_eq!(query.fuzzy_paths, ["modules/jweb/"]);
    assert!(fuzzy.allows("src/modules/jweb/Parser.java"));
    assert!(fuzzy.allows("modules/legacy/jweb/parser.java"));
    assert!(!fuzzy.allows("src/modules/other/parser.java"));
    assert!(!fuzzy.is_unfiltered());
    let (query, literal) = plan("path:modules/jweb/ parser");
    assert!(query.fuzzy_paths.is_empty());
    assert!(literal.allows("src/modules/jweb/Parser.java"));
    assert!(!literal.allows("src/modules/other/Parser.java"));
    let (_, excluded) = plan("-fpath:modules/jweb/ parser");
    assert!(!excluded.allows("src/modules/jweb/Parser.java"));
    assert!(excluded.allows("src/modules/other/parser.java"));
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
    let query = crate::query::parse_search("glob:\"[bad\" x");
    let diagnostic = FileSelectionPlan::compile(&query).unwrap_err();
    assert!(diagnostic.message.contains("invalid glob"));
    assert_eq!(&query.source[diagnostic.range], "glob:\"[bad\"");
}
