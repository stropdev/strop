//! The row decision the ranking pass applies to kept lists (0109
//! §3.2d): flat filters and the Boolean AST over borrowed row facts.

use super::*;
use crate::query::schema;
use crate::query::SearchModes;
use strop_core::symbol_kind::SymbolKind;

fn compile(input: &str, on: &schema::Schema) -> Option<Admission> {
    let query = SearchQuery::parse(input, on, SearchModes::default());
    assert_eq!(
        query.state,
        QueryState::Ready,
        "{input}: {:?}",
        query.diagnostics
    );
    Admission::compile(&query).unwrap()
}

fn symbol<'a>(kind: SymbolKind, name: &'a str, container: Option<&'a str>) -> RowFacts<'a> {
    RowFacts {
        path: Some("src/lib.rs"),
        line: Some(1),
        kind: Some(kind),
        text: name,
        container,
    }
}

fn keeps(admission: &Admission, row: RowFacts<'_>) -> bool {
    admission.admits(&row, &Evidence::default())
}

#[test]
fn flat_filters_narrow_like_file_find() {
    // an implicit conjunction, OR inside a family
    let admission = compile("language:rust path:tests", &schema::JUMPS).unwrap();
    assert!(keeps(&admission, line("tests/a.rs", "fn one() {}")));
    assert!(!keeps(&admission, line("src/a.rs", "fn one() {}")), "path");
    assert!(!keeps(&admission, line("tests/a.py", "x = 1")), "language");
    // a row with no path (a scratch buffer) cannot be judged: it stays
    assert!(keeps(&admission, RowFacts::default()));
    let either = compile("language:rust language:python", &schema::BUFFERS).unwrap();
    assert!(keeps(&either, line("a.py", "")));
    assert!(!keeps(&either, line("a.c", "")));
}

#[test]
fn every_exclusion_holds_on_its_own() {
    // two exclusions are both applied, never one-or-the-other
    let admission = compile("-path:vendor -path:generated", &schema::BUFFERS).unwrap();
    assert!(keeps(&admission, line("src/a.rs", "")));
    assert!(!keeps(&admission, line("vendor/a.rs", "")));
    assert!(!keeps(&admission, line("src/generated/a.rs", "")));
    let rust = compile("-language:rust", &schema::BUFFERS).unwrap();
    assert!(!keeps(&rust, line("a.rs", "x")));
    assert!(keeps(&rust, line("a.py", "x")));
}

#[test]
fn bare_words_never_enter_the_row_decision() {
    // free text is the ranked needle, filters or not (0109 decision 5)
    assert!(compile("parse config", &schema::SYMBOLS).is_none());
    let query = crate::query::parse_search("kind:function parse");
    assert_eq!(query.boolean, None);
    assert_eq!(
        query.content,
        Some(crate::query::ContentExpr::Literal("parse".into()))
    );
    let admission = compile("kind:function pcfg", &schema::SYMBOLS).unwrap();
    assert!(keeps(
        &admission,
        symbol(SymbolKind::Function, "parse_config", None)
    ));
    // an invalid query never compiles
    let query = crate::query::parse_search("language:");
    assert!(Admission::compile(&query).is_err());
}

#[test]
fn kind_judges_every_row_kind() {
    // 0109 cause B: fields, variants and type parameters are kinds too
    let function = compile("kind:function", &schema::SYMBOLS).unwrap();
    for kind in [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Constructor,
    ] {
        assert!(keeps(&function, symbol(kind, "f", None)), "{kind:?}");
    }
    for kind in [
        SymbolKind::Field,
        SymbolKind::Variant,
        SymbolKind::Type,
        SymbolKind::Struct,
        SymbolKind::Impl,
        SymbolKind::Other,
    ] {
        assert!(!keeps(&function, symbol(kind, "f", None)), "{kind:?}");
    }
    let excluded = compile("-kind:fn", &schema::SYMBOLS).unwrap();
    assert!(!keeps(&excluded, symbol(SymbolKind::Method, "m", None)));
    assert!(keeps(&excluded, symbol(SymbolKind::Field, "x", None)));
    // the language spellings name the kinds they mean
    let traits = compile("kind:trait", &schema::SYMBOLS).unwrap();
    assert!(keeps(&traits, symbol(SymbolKind::Interface, "T", None)));
    // a row with no kind and no index cannot be judged: it stays
    assert!(keeps(&function, line("src/a.rs", "text")));
    let not = compile("NOT kind:function AND x", &schema::SYMBOLS).unwrap();
    assert!(keeps(&not, line("src/a.rs", "x")));
}

#[test]
fn content_atoms_match_the_name_or_the_container() {
    let admission = compile("kind:method AND Parser", &schema::SYMBOLS).unwrap();
    assert!(keeps(
        &admission,
        symbol(SymbolKind::Method, "parse", Some("Parser"))
    ));
    assert!(!keeps(
        &admission,
        symbol(SymbolKind::Method, "parse", None)
    ));
    assert!(!keeps(
        &admission,
        symbol(SymbolKind::Function, "parse", Some("Parser"))
    ));
    // negative content guards apply to symbols too
    let guarded = compile("parse NOT regex:test", &schema::SYMBOLS).unwrap();
    assert!(!keeps(
        &guarded,
        symbol(SymbolKind::Function, "test_parse", None)
    ));
    assert!(keeps(&guarded, symbol(SymbolKind::Function, "parse", None)));
}

#[test]
fn path_filters_share_the_selected_case() {
    for input in ["path:src/", "case:insensitive path:SRC/"] {
        let admission = compile(input, &schema::JUMPS).unwrap();
        assert!(keeps(&admission, line("src/lib.rs", "anything")), "{input}");
        assert!(
            !keeps(&admission, line("tests/lib.rs", "anything")),
            "{input}"
        );
    }
    let sensitive = compile("case:sensitive path:SRC/", &schema::JUMPS).unwrap();
    assert!(keeps(&sensitive, line("SRC/lib.rs", "anything")));
    assert!(!keeps(&sensitive, line("src/lib.rs", "anything")));
}

#[test]
fn one_glob_engine_decides_everywhere() {
    // the same `glob:` keeps the same paths in a walk and on a row
    for glob in ["*.rs", "**/x.rs", "a/*", "src/**/*.rs"] {
        let query = crate::query::parse_search(&format!("glob:\"{glob}\""));
        let walk = FileSelectionPlan::compile(&query).unwrap();
        let rows = Admission::compile(&query).unwrap().unwrap();
        for path in [
            "a/src/x.rs",
            "a/src/x.py",
            "deep/nest/x.rs",
            "src/a/b.rs",
            "x.rs",
        ] {
            assert_eq!(
                walk.allows(path),
                keeps(&rows, line(path, "")),
                "{glob} on {path}"
            );
        }
    }
}
