//! The schema contract (0109 §4): what each list declares is exactly
//! what its field accepts, suggests and advertises.

use super::lexer::{self, TokenKind};
use super::schema::{self, Field, Schema, SURFACES};
use super::suggest::suggest;
use super::{QueryState, SearchModes, SearchQuery};
use crate::Kind;

const KINDS: [Kind; 14] = [
    Kind::Jumps,
    Kind::Symbols,
    Kind::WorkspaceSymbols,
    Kind::Files,
    Kind::Buffers,
    Kind::Search,
    Kind::Diagnostics,
    Kind::Locations,
    Kind::RemoteHosts,
    Kind::RemoteAddress,
    Kind::Actions,
    Kind::TabSize,
    Kind::Theme,
    Kind::Branches,
];

fn parse(input: &str, schema: &Schema) -> SearchQuery {
    SearchQuery::parse(input, schema, SearchModes::default())
}

/// A value each key accepts, so only the schema decides Ready.
fn valid(field: Field) -> &'static str {
    match field {
        Field::Language => "language:rust",
        Field::Path => "path:src",
        Field::FuzzyPath => "fpath:src",
        Field::Glob => "glob:*.rs",
        Field::Kind => "kind:fn",
        Field::Repo => "repo:core",
        Field::Hidden => "hidden:include",
        Field::Ignored => "ignored:exclude",
        Field::Case => "case:sensitive",
        Field::Text => "text:x",
        Field::Regex => "regex:x",
    }
}

#[test]
fn every_picker_kind_declares_a_schema() {
    let plain = [
        Kind::RemoteHosts,
        Kind::RemoteAddress,
        Kind::Actions,
        Kind::TabSize,
        Kind::Theme,
        Kind::Branches,
    ];
    for kind in KINDS {
        let schema = kind.schema();
        assert_eq!(schema.is_plain(), plain.contains(&kind), "{kind:?}");
        if !schema.is_plain() {
            assert!(SURFACES.contains(&schema), "{kind:?} is listed in help");
            assert!(!schema.surface.is_empty() && !schema.noun.is_empty());
        }
    }
    // the four lists the owner named take path:, and their rows say so
    for kind in [
        Kind::Buffers,
        Kind::Diagnostics,
        Kind::Locations,
        Kind::Jumps,
    ] {
        for field in [Field::Path, Field::Language, Field::Case] {
            assert!(kind.schema().declares(field), "{kind:?} {field:?}");
        }
        assert!(
            !kind.schema().declares(Field::Kind),
            "{kind:?} rows carry no kind"
        );
    }
    assert!(Kind::Symbols.schema().declares(Field::Kind));
    assert!(Kind::WorkspaceSymbols.schema().declares(Field::Kind));
    assert!(Kind::Search.schema().declares(Field::Kind));
    assert!(!Kind::Files.schema().declares(Field::Kind));
}

#[test]
fn every_field_key_lexes_as_a_qualifier() {
    for field in Field::ALL {
        let input = valid(field);
        match &lexer::lex(input)[0].kind {
            TokenKind::Qualifier { key, .. } => assert_eq!(key, field.key()),
            other => panic!("{input}: {other:?}"),
        }
        assert_eq!(Field::from_key(field.key()), Some(field));
    }
    assert_eq!(Field::from_key("type"), None, "type: is retired (0109)");
}

#[test]
fn a_key_outside_the_schema_is_a_located_error() {
    for schema in SURFACES {
        for field in Field::ALL {
            let input = valid(field);
            let query = parse(input, schema);
            if schema.declares(field) {
                assert_eq!(query.state, QueryState::Ready, "{input} on {}", schema.noun);
                continue;
            }
            assert_eq!(
                query.state,
                QueryState::Invalid,
                "{input} on {}",
                schema.noun
            );
            let diagnostic = &query.diagnostics[0];
            assert_eq!(diagnostic.range, 0..input.len(), "located under the key");
            assert_eq!(
                diagnostic.message,
                format!("{}: does not filter {}", field.key(), schema.noun)
            );
            let elsewhere = diagnostic.suggestion.as_deref().unwrap_or_default();
            assert!(elsewhere.contains(field.key()), "{elsewhere}");
        }
    }
}

#[test]
fn type_is_not_a_key() {
    let query = parse("type:function parse", &schema::SYMBOLS);
    assert_eq!(query.state, QueryState::Invalid);
    assert_eq!(query.diagnostics[0].message, "unknown filter: type");
    assert_eq!(
        query.diagnostics[0].suggestion.as_deref(),
        Some("did you mean kind:?")
    );
    // a correction only names a key the list takes
    let files = parse("type:function", &schema::FILES);
    assert_eq!(files.state, QueryState::Invalid);
    assert_eq!(files.diagnostics[0].suggestion, None);
    // ext: is a habit for the language, not a misspelt text:
    let ext = parse("ext:rs", &schema::FILES);
    assert_eq!(
        ext.diagnostics[0].suggestion.as_deref(),
        Some("did you mean language:?")
    );
}

#[test]
fn suggestions_offer_only_declared_keys() {
    for kind in KINDS {
        let schema = kind.schema();
        if schema.is_plain() {
            continue;
        }
        let offered: Vec<String> = suggest(&parse("", schema), 0, schema)
            .into_iter()
            .map(|suggestion| suggestion.insert)
            .collect();
        let declared: Vec<String> = schema
            .fields
            .iter()
            .map(|field| format!("{}:", field.key()))
            .collect();
        assert_eq!(offered, declared, "{kind:?}");
    }
}

#[test]
fn hints_come_from_the_schema() {
    let hint = schema::SYMBOLS.hint();
    assert!(hint.starts_with("kind:fn "), "{hint}");
    assert!(hint.contains("ctrl-space"));
    assert!(!schema::BUFFERS.hint().contains("kind:"));
    assert_eq!(schema::PLAIN.hint(), "");
    for schema in SURFACES {
        for field in schema.fields {
            assert!(schema.examples().contains(field.example()));
        }
    }
}

/// The docs say exactly what each list takes (0109 decision 7): the
/// table in `docs/search.md` is the schema, row for row, and its kind
/// table is the kind table.
#[test]
fn the_docs_list_what_each_list_takes() {
    let docs =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/search.md"))
            .unwrap();
    let row = |lead: &str| {
        docs.lines()
            .find(|line| line.starts_with(lead))
            .unwrap_or_else(|| panic!("docs/search.md has no row {lead}"))
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>()
    };
    for schema in SURFACES {
        let lead = format!("| `{}` {} |", schema.surface, schema.noun);
        let keys: Vec<String> = schema
            .fields
            .iter()
            .map(|field| format!("`{}:`", field.key()))
            .collect();
        assert_eq!(row(&lead)[2], keys.join(" "), "{lead}");
    }
    for spec in strop_core::symbol_kind::KINDS {
        let cells = row(&format!("| `{}` | `{}` |", spec.value, spec.chip));
        let aliases: Vec<String> = spec.aliases.iter().map(|a| format!("`{a}`")).collect();
        assert_eq!(cells[3], aliases.join(" "), "{}", spec.value);
        assert_eq!(cells[4], spec.lsp.join(", "), "{}", spec.value);
    }
}
