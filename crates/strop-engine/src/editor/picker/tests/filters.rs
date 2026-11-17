//! One filter language for every list (0109): each list takes exactly
//! the keys its schema declares, each declared key changes the list,
//! and a key outside the schema is a located error that leaves the list
//! as it was. Driven by `Kind::schema()`, so a key added to a schema
//! without a case here fails `every_declared_field_has_a_narrowing_case`.

use super::super::*;
use strop_core::frontend_input::Key;
use strop_core::symbol_kind::SymbolKind;
use strop_core::Buffer;
use strop_picker::query::Field;

/// Every list that takes filters, except Search (its own suite).
const LISTS: [Kind; 7] = [
    Kind::Files,
    Kind::WorkspaceSymbols,
    Kind::Symbols,
    Kind::Buffers,
    Kind::Jumps,
    Kind::Diagnostics,
    Kind::Locations,
];

/// A Rust file, a Python file, a dotfile and an ignored file.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "struct Config;\nfn parse_config() {}\nfn wrap() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("tests/check.py"), "def check():\n    pass\n").unwrap();
    std::fs::write(root.join(".hidden.rs"), "fn secret() {}\n").unwrap();
    std::fs::write(root.join("ignored.rs"), "fn skipped() {}\n").unwrap();
    std::fs::write(root.join(".ignore"), "ignored.rs\n").unwrap();
    dir
}

fn diagnostic(e: &Editor, message: &str) -> crate::editor::DocumentDiagnostics {
    crate::editor::DocumentDiagnostics {
        revision: e.buf().revision(),
        items: vec![strop_lsp::ResolvedDiag {
            line: strop_core::id::LineIndex::new(0),
            col: strop_core::id::ByteColumn::new(0),
            end_line: strop_core::id::LineIndex::new(0),
            end_col: strop_core::id::ByteColumn::new(1),
            severity: strop_lsp::Severity::Error,
            message: message.into(),
        }],
    }
}

/// The list open over the workspace, its rows ranked.
fn open(kind: Kind, root: &std::path::Path) -> Editor {
    let mut e = Editor::new_in(Buffer::from_text(""), root.to_path_buf());
    let lib = root.join("src/lib.rs");
    let check = root.join("tests/check.py");
    match kind {
        Kind::Symbols => {
            e.open_fixture(&lib).unwrap();
            e.feed_text(" s");
            e.wait_symbols();
        }
        Kind::Buffers | Kind::Jumps | Kind::Diagnostics => {
            e.open_fixture(&lib).unwrap();
            e.feed_text("G");
            let first = (e.current(), diagnostic(&e, "unused config"));
            e.open_fixture(&check).unwrap();
            e.feed_text("G");
            if kind == Kind::Diagnostics {
                e.fixture_insert_diagnostics(first.0, first.1);
                let second = diagnostic(&e, "check is undefined");
                e.fixture_insert_diagnostics(e.current(), second);
                e.open_diagnostics_picker();
            } else {
                e.open_picker(kind);
            }
        }
        Kind::Locations => {
            e.locations_fixture(
                &[(lib, 1), (check, 0)],
                vec!["fn parse_config() {}".into(), "def check():".into()],
            );
        }
        _ => e.open_picker(kind),
    }
    e.wait_picker();
    e
}

fn visible(e: &Editor) -> Vec<String> {
    let picker = &e.picker.as_ref().unwrap().picker;
    picker
        .rows
        .iter()
        .map(|row| picker.items[row.item].text.clone())
        .collect()
}

fn query(e: &mut Editor, input: &str) -> Vec<String> {
    e.picker.as_mut().unwrap().picker.input.set_text(input);
    e.picker_input_changed();
    e.wait_picker();
    let error = e.picker.as_ref().unwrap().picker.error.clone();
    assert_eq!(error, None, "{input}");
    visible(e)
}

/// What a declared key does on a list: the list `with` it differs from
/// the list `without` it, and every row kept satisfies `keeps`.
struct Case {
    with: String,
    without: String,
    keeps: fn(&str) -> bool,
}

fn case(kind: Kind, field: Field) -> Option<Case> {
    // the word some rows hold: a symbol name, or the python file
    let word = if kind == Kind::Symbols {
        "wrap"
    } else {
        "check"
    };
    let make = |with: &str, without: &str, keeps: fn(&str) -> bool| {
        Some(Case {
            with: with.into(),
            without: without.into(),
            keeps,
        })
    };
    let python = |row: &str| row.contains("check") || row.contains("tests/");
    match field {
        Field::Language => make("language:python", "", python),
        Field::Path => make("path:tests", "", python),
        Field::FuzzyPath => make("fpath:tsts", "", python),
        Field::Glob => make("glob:\"**/*.py\"", "", python),
        Field::Kind => make("kind:struct", "", |row| row.contains("Config")),
        Field::Case => make(
            &format!("case:ignore {}", word.to_uppercase()),
            &word.to_uppercase(),
            |row| row.contains("check") || row.contains("wrap"),
        ),
        Field::Text => make(&format!("text:{word}"), "", |row| {
            row.contains("check") || row.contains("wrap")
        }),
        Field::Regex => make(
            if kind == Kind::Symbols {
                "regex:\"wr.p\""
            } else {
                "regex:\"ch.ck\""
            },
            "",
            |row| row.contains("check") || row.contains("wrap"),
        ),
        Field::Hidden => make("hidden:exclude", "hidden:include", |row| {
            !row.contains("hidden") && !row.contains("secret")
        }),
        Field::Ignored => make("ignored:include", "ignored:exclude", |_| true),
        Field::Repo => None,
    }
}

#[test]
fn every_declared_field_has_a_narrowing_case() {
    let root = workspace();
    for kind in LISTS {
        for &field in kind.schema().fields {
            let Some(case) = case(kind, field) else {
                panic!("{kind:?} declares {field:?} with no case here");
            };
            let mut e = open(kind, root.path());
            let without = query(&mut e, &case.without);
            let with = query(&mut e, &case.with);
            assert_ne!(with, without, "{kind:?} {}", case.with);
            for row in &with {
                assert!((case.keeps)(row), "{kind:?} {} kept {row:?}", case.with);
            }
        }
    }
}

#[test]
fn a_key_a_list_does_not_take_is_an_error_and_the_list_stays() {
    let root = workspace();
    for kind in LISTS {
        let schema = kind.schema();
        let Some(field) = Field::ALL
            .into_iter()
            .find(|field| !schema.declares(*field))
        else {
            continue;
        };
        let mut e = open(kind, root.path());
        let before = query(&mut e, "");
        let input = format!("{}:x", field.key());
        e.picker.as_mut().unwrap().picker.input.set_text(&input);
        e.picker_input_changed();
        e.wait_picker();
        let glue = e.picker.as_ref().unwrap();
        let error = glue.picker.error.clone().unwrap_or_default();
        assert!(
            error.starts_with(&format!("{}: does not filter {}", field.key(), schema.noun)),
            "{kind:?}: {error}"
        );
        assert!(glue.query_diagnosed());
        assert_eq!(visible(&e), before, "{kind:?}: the list stays as it was");
    }
}

#[test]
fn symbols_kind_keeps_fuzzy_words() {
    // the owner's bug, cause C: `kind:` no longer turns the words beside
    // it into an exact phrase
    let root = workspace();
    let mut e = open(Kind::Symbols, root.path());
    assert_eq!(query(&mut e, "pcfg"), ["parse_config  · :2"]);
    assert_eq!(query(&mut e, "kind:function pcfg"), ["parse_config  · :2"]);
    assert_eq!(
        query(&mut e, "kind:fn parse config"),
        ["parse_config  · :2"]
    );
    assert!(query(&mut e, "kind:struct pcfg").is_empty());
}

#[test]
fn symbols_kind_typed_one_key_at_a_time() {
    // typing, not a paste: until the colon `kind` is a word like any
    // other; then the list stays as it was while the value is partial,
    // and filters once the value names one kind
    let root = workspace();
    let mut e = open(Kind::Symbols, root.path());
    for c in "kind".chars() {
        e.feed(Key::Char(c));
    }
    e.wait_picker();
    let word = visible(&e);
    for c in ":f".chars() {
        e.feed(Key::Char(c));
        e.wait_picker();
        assert!(e.picker.as_ref().unwrap().query_diagnosed());
        assert_eq!(visible(&e), word, "the list stays while typing");
    }
    let error = e.picker.as_ref().unwrap().picker.error.clone().unwrap();
    assert!(error.starts_with("kind:f could be"), "{error}");
    e.feed(Key::Char('u'));
    e.wait_picker();
    assert_eq!(e.picker.as_ref().unwrap().picker.error, None);
    assert_eq!(visible(&e), ["parse_config  · :2", "wrap  · :3"]);
    for c in "nction".chars() {
        e.feed(Key::Char(c));
    }
    e.wait_picker();
    assert_eq!(visible(&e), ["parse_config  · :2", "wrap  · :3"]);
}

#[test]
fn symbols_kind_filters_every_lsp_kind() {
    // the owner's bug, cause B: fields, variants and type parameters
    // used to pass every `kind:` filter
    let root = workspace();
    let mut e = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    e.open_picker(Kind::Symbols);
    let row = |name: &str, kind: SymbolKind, line: usize| {
        strop_picker::Item::symbol(strop_picker::SymbolRow {
            name,
            container: Some("Config"),
            detail: format_args!("· :{line}"),
            kind,
            path: Some("src/lib.rs".into()),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(root.path().join("src/lib.rs")),
                line,
                col: 1,
                match_len: 1,
                line_text: "".into(),
            },
        })
    };
    e.picker_items_fixture(vec![
        row("field", SymbolKind::Field, 1),
        row("Variant", SymbolKind::Variant, 2),
        row("T", SymbolKind::Type, 3),
        row("new", SymbolKind::Constructor, 4),
        row("run", SymbolKind::Method, 5),
        row("Config", SymbolKind::Struct, 6),
    ]);
    assert_eq!(
        query(&mut e, "kind:function"),
        ["new  Config · :4", "run  Config · :5"]
    );
    assert_eq!(query(&mut e, "kind:field"), ["field  Config · :1"]);
    assert_eq!(query(&mut e, "kind:variant"), ["Variant  Config · :2"]);
    assert_eq!(query(&mut e, "kind:type"), ["T  Config · :3"]);
    assert_eq!(query(&mut e, "-kind:fn -kind:field").len(), 3);
}

#[test]
fn files_boolean_query_filters_paths() {
    let root = workspace();
    let mut e = open(Kind::Files, root.path());
    let not_src = query(&mut e, "NOT path:src");
    assert!(!not_src.is_empty() && not_src.iter().all(|row| !row.starts_with("src/")));
    let either = query(&mut e, "path:src OR path:tests");
    assert_eq!(either.len(), 2, "{either:?}");
    assert!(query(&mut e, "check AND NOT py").is_empty());
}

#[test]
fn search_metadata_only_query_asks_for_text() {
    let root = workspace();
    let mut e = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    e.open_search(false);
    e.paste_bracketed("kind:function");
    e.wait_picker();
    let picker = &e.picker.as_ref().unwrap().picker;
    assert_eq!(
        picker.error.as_deref(),
        Some("content search needs a text or regex expression")
    );
    assert!(picker.items.is_empty(), "nothing searched");
}

#[test]
fn plain_lists_read_the_field_as_text() {
    // action titles hold colons and a theme name is a name: nothing is
    // parsed, nothing is an error
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(":theme\r");
    e.wait_picker();
    assert_eq!(e.picker.as_ref().unwrap().picker.kind, Kind::Theme);
    e.paste_bracketed("kind:x");
    e.wait_picker();
    let glue = e.picker.as_ref().unwrap();
    assert_eq!(glue.picker.error, None);
    assert!(!glue.query_diagnosed());
    assert!(glue.picker.rows.is_empty(), "matched as text, not a filter");
    e.feed(Key::CtrlSpace);
    assert!(
        e.picker.as_ref().unwrap().suggestions.is_none(),
        "no filters to suggest"
    );
}
