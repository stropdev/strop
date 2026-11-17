//! 0063 §6.1/§6.2 verification: deterministic property corpora and an
//! independent reference interpreter. Everything here is hermetic — a
//! fixed-seed LCG (no RNG dependency, no wall clock), tempdirs for the
//! evidence sources, bounded iteration counts.

use super::facts::RowFacts;
use super::parser::{BooleanExpr, ContentAtom, ContentExpr, MetadataAtom, QueryState};
use super::plan::{ContentPlan, Evidence};
use super::schema::{self, Field, SURFACES};
use super::{SearchModes, SearchQuery};

/// A tiny deterministic generator (Knuth LCG): reproducible corpora in
/// CI, release builds and replay alike.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    fn flag(&mut self) -> bool {
        self.next() & 1 == 0
    }
}

// ---- §6.1: parser properties over generated inputs ------------------------

/// The alphabet deliberately includes the grammar's sharp edges:
/// operator words, parentheses, quotes, escapes, colons and multibyte
/// text — whatever a user can type must parse without panicking and
/// without producing an unrankable state.
const ALPHABET: &[&str] = &[
    "a",
    "parser",
    "request",
    "retry",
    "AND",
    "OR",
    "NOT",
    "and",
    "(",
    ")",
    "\"",
    "'",
    ":",
    "kind:",
    "repo:",
    "type:",
    "language:",
    "glob:",
    "path:",
    "fpath:",
    "text:",
    "case:",
    "hidden:",
    "ignored:",
    "-",
    "!",
    "|",
    "\\",
    "日本語",
    "résumé",
    " ",
    "  ",
];

fn random_query(rand: &mut Lcg) -> String {
    let words = rand.below(8) + 1;
    (0..words)
        .map(|_| *rand.pick(ALPHABET))
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn arbitrary_input_never_panics_and_settles_into_a_known_state() {
    let mut rand = Lcg(0x0063);
    for _ in 0..4000 {
        let raw = random_query(&mut rand);
        for schema in SURFACES {
            let query = SearchQuery::parse(&raw, schema, SearchModes::default());
            assert!(
                query.state == QueryState::Ready
                    || query.state == QueryState::Incomplete
                    || query.state == QueryState::Invalid,
                "a parse must settle: {raw:?}"
            );
            // A settled non-Ready query always says why, at a place.
            if query.state != QueryState::Ready {
                assert!(!query.diagnostics.is_empty(), "{raw:?}");
            }
            // Compiling a Ready query must also hold: regex bounds are
            // the parser's own diagnostics, never a panic.
            if query.state == QueryState::Ready {
                let _ = ContentPlan::compile(&query);
                let _ = super::plan::Admission::compile(&query);
            }
        }
    }
}

#[test]
fn every_ready_query_uses_only_declared_fields() {
    let mut rand = Lcg(0x0109);
    for _ in 0..3000 {
        let raw = random_query(&mut rand);
        for schema in SURFACES {
            let query = SearchQuery::parse(&raw, schema, SearchModes::default());
            if query.state != QueryState::Ready {
                continue;
            }
            for token in &query.tokens {
                if let super::lexer::TokenKind::Qualifier { key, .. } = &token.kind {
                    let field = Field::from_key(key).expect("a lexed key is a field");
                    assert!(schema.declares(field), "{raw:?} on {}", schema.noun);
                }
            }
        }
    }
}

#[test]
fn canonical_formatting_is_a_fixed_point_for_ready_queries() {
    let mut rand = Lcg(0x0FF1CE);
    for _ in 0..2000 {
        let raw = random_query(&mut rand);
        let query = crate::query::parse_search(&raw);
        if query.state != QueryState::Ready {
            continue;
        }
        let Some(expr) = query.boolean.clone() else {
            continue;
        };
        let once = expr.to_query_string();
        let reparsed = crate::query::parse_search(&once);
        assert_eq!(
            reparsed.state,
            QueryState::Ready,
            "canonical form must re-parse ready: {raw:?} -> {once:?}"
        );
        let twice = reparsed
            .boolean
            .clone()
            .expect("operator-bearing canonical form stays Boolean")
            .to_query_string();
        assert_eq!(once, twice, "canonicalization is idempotent: {raw:?}");
    }
}

#[test]
fn operator_free_queries_stay_flat() {
    let mut rand = Lcg(0xC0FFEE);
    for _ in 0..2000 {
        let raw = random_query(&mut rand);
        // Only operators make a query Boolean; a filter never does
        // (0109 decision 5). Standalone operators or parens are.
        let operator_bearing = raw.contains(['(', ')'])
            || raw
                .split_whitespace()
                .any(|word| matches!(word, "AND" | "OR" | "NOT"));
        if operator_bearing {
            continue;
        }
        let query = crate::query::parse_search(&raw);
        assert!(
            query.boolean.is_none(),
            "no operators: the query must parse flat: {raw:?}"
        );
    }
}

/// The owner's bug (0109 cause C), as a property: for any needle and
/// any filter every row passes, the list is exactly the list the needle
/// alone gives — same rows, same order, same scores.
#[test]
fn a_qualifier_never_changes_how_bare_words_match() {
    use crate::rank::{rank, FilterRequest, MatchMode};
    use strop_core::symbol_kind::SymbolKind;
    let names = [
        "parse_config",
        "ParseConfig",
        "wrap",
        "write_all",
        "parser",
        "config_path",
        "résumé",
        "PCFG",
        "apply",
        "new",
    ];
    let items: Vec<crate::Item> = names
        .iter()
        .enumerate()
        .map(|(at, name)| {
            crate::Item::symbol(crate::SymbolRow {
                name,
                container: Some("Config"),
                detail: format_args!("· :{}", at + 1),
                kind: SymbolKind::Function,
                path: Some("src/lib.rs".into()),
                payload: crate::Payload::Grep {
                    location: strop_workspace::ResourceLocation::local("/w/src/lib.rs".into()),
                    line: at + 1,
                    col: 1,
                    match_len: 1,
                    line_text: "".into(),
                },
            })
        })
        .collect();
    let catalog = crate::Catalog::from(items);
    // filters every row passes: a kind it is, a path it has
    let always = [
        "kind:function",
        "kind:fn",
        "kind:FUNC",
        "path:src",
        "language:rust",
        "-kind:struct",
        "glob:\"**/*.rs\"",
    ];
    let letters = ["p", "c", "f", "g", "w", "a", "r", "n", "C", "e", "_", "é"];
    let ranked = |input: &str| {
        let query = std::sync::Arc::new(SearchQuery::parse(
            input,
            &schema::SYMBOLS,
            SearchModes::default(),
        ));
        assert_eq!(query.state, QueryState::Ready, "{input}");
        let needle = match &query.content {
            Some(ContentExpr::Literal(text)) => text.clone(),
            _ => String::new(),
        };
        let request = FilterRequest {
            catalog: catalog.clone(),
            query: needle,
            upstream_filtered: false,
            pinned_tail: 0,
            mode: MatchMode::Fuzzy(query.case_mode()),
            filter: Some(query),
            paths: false,
            priors: Default::default(),
        };
        let ranking = rank(&request, || false).unwrap().unwrap();
        ranking
            .rows
            .iter()
            .map(|row| (row.item, row.score, ranking.columns(row).to_vec()))
            .collect::<Vec<_>>()
    };
    let mut rand = Lcg(0xBA5E);
    for _ in 0..400 {
        let needle: String = (0..rand.below(4) + 1)
            .map(|_| *rand.pick(&letters))
            .collect();
        let bare = ranked(&needle);
        let filter = *rand.pick(&always);
        for input in [format!("{filter} {needle}"), format!("{needle} {filter}")] {
            assert_eq!(ranked(&input), bare, "{input:?} against {needle:?}");
        }
    }
    // and the owner's own case: `kind:function pcfg` finds parse_config
    assert!(!ranked("kind:function pcfg").is_empty());
}

// ---- §6.2: the reference interpreter --------------------------------------

/// An independent three-valued evaluator (`None` = Unknown), written
/// from the spec (0063 §4) rather than from `plan.rs`: different
/// structure (monadic recursion), different glob implementation, and
/// evidence arrives as ground-truth closures fixed by the fixture.
fn reference(
    expr: &BooleanExpr,
    line: &str,
    path: &str,
    line_number: usize,
    kind: &dyn Fn(&str, &str, usize) -> Option<bool>,
    repo: &dyn Fn(&str, &str) -> Option<bool>,
) -> Option<bool> {
    match expr {
        BooleanExpr::And(operands) => {
            let mut result = Some(true);
            for operand in operands {
                match reference(operand, line, path, line_number, kind, repo) {
                    Some(false) => return Some(false),
                    None => result = None,
                    Some(true) => {}
                }
            }
            result
        }
        BooleanExpr::Or(branches) => {
            let mut result = Some(false);
            for branch in branches {
                match reference(branch, line, path, line_number, kind, repo) {
                    Some(true) => return Some(true),
                    None => result = None,
                    Some(false) => {}
                }
            }
            result
        }
        BooleanExpr::Not(inner) => {
            reference(inner, line, path, line_number, kind, repo).map(|v| !v)
        }
        BooleanExpr::Content(atom) => {
            // The corpus generates literals only; a literal contains.
            Some(line.contains(atom.text.as_str()))
        }
        BooleanExpr::Metadata(atom) => {
            let decided = match atom.key.as_str() {
                "language" => Some(match path.rsplit('.').next().unwrap_or("") {
                    "rs" => atom.value == "rust",
                    "py" => atom.value == "python",
                    _ => false,
                }),
                "path" => Some(path.contains(atom.value.as_str())),
                "fpath" => Some(crate::fuzzy_score(&atom.value, path).is_some()),
                "glob" => Some(reference_glob(&atom.value, path)),
                "kind" => kind(atom.value.as_str(), path, line_number),
                "repo" => repo(&atom.value, path),
                _ => None,
            };
            decided.map(|value| value != atom.negated)
        }
    }
}

/// Independent glob semantics — the textbook definition: `*` spans
/// any (possibly empty) run including `/`, everything else compares
/// literally. Exponential in principle, corpus-bounded in practice
/// (short paths, few stars).
fn reference_glob(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if let Some(rest) = pattern.strip_prefix('*') {
        let rest = rest.trim_start_matches('*');
        if rest.is_empty() {
            return true;
        }
        return (0..=path.len())
            .filter(|split| path.is_char_boundary(*split))
            .any(|split| reference_glob(rest, &path[split..]));
    }
    let head = pattern.chars().next().expect("nonempty");
    match path.chars().next() {
        Some(got) if got == head => {
            reference_glob(&pattern[head.len_utf8()..], &path[head.len_utf8()..])
        }
        _ => false,
    }
}

const WORDS: &[&str] = &["parser", "request", "retry", "let", "fn", "class"];
const LINES: &[&str] = &[
    "fn parser() { let retry = request; }",
    "class Request: pass",
    "let plain = 2;",
    "function mod:method() end",
    "parser_request_retry",
];
const PATHS: &[&str] = &[
    "a/src/x.rs",
    "a/y.py",
    "b/loose/z.lua",
    "notes.txt",
    "q.cpp",
];

fn random_expr(rand: &mut Lcg, depth: usize) -> BooleanExpr {
    if depth == 0 || rand.flag() {
        if rand.flag() {
            BooleanExpr::Content(ContentAtom {
                regex: false,
                text: (*rand.pick(WORDS)).to_string(),
            })
        } else {
            let (key, value) = *rand.pick(&[
                ("kind", "function"),
                ("kind", "struct"),
                ("kind", "fn"),
                ("kind", "trait"),
                ("repo", "a"),
                ("repo", "b"),
                ("repo", "zz"),
                ("language", "rust"),
                ("language", "python"),
                ("path", "src"),
                ("path", "zz"),
                ("fpath", "src"),
                ("glob", "a/*"),
                ("glob", "*.rs"),
                ("glob", "**/x.rs"),
            ]);
            BooleanExpr::Metadata(MetadataAtom {
                key: key.to_string(),
                value: value.to_string(),
                negated: rand.flag(),
            })
        }
    } else {
        match rand.below(3) {
            0 => BooleanExpr::And(
                (0..rand.below(3) + 1)
                    .map(|_| random_expr(rand, depth - 1))
                    .collect(),
            ),
            1 => BooleanExpr::Or(
                (0..rand.below(3) + 1)
                    .map(|_| random_expr(rand, depth - 1))
                    .collect(),
            ),
            _ => BooleanExpr::Not(Box::new(random_expr(rand, depth - 1))),
        }
    }
}

#[test]
fn admission_agrees_with_the_reference_interpreter() {
    let directory = tempfile::tempdir().unwrap();
    // The plan's evidence is built for real: one indexed rust file and
    // a two-project catalog.
    std::fs::create_dir_all(directory.path().join("a/src")).unwrap();
    std::fs::write(
        directory.path().join("a/src/x.rs"),
        "fn covered() {\n    let parser = 1;\n}\nlet bare = 2;\n",
    )
    .unwrap();
    std::fs::create_dir(directory.path().join("a/.git")).unwrap();
    std::fs::create_dir_all(directory.path().join("b/.git")).unwrap();
    let index = crate::source::symbols::SymbolIndex::build(
        directory.path(),
        &[std::path::PathBuf::from("a/src/x.rs")],
        &|| false,
    );
    let catalog = crate::source::catalog::ProjectCatalog::discover(directory.path(), &|| false);

    // The reference's evidence is the fixture's GROUND TRUTH, stated
    // by reading the fixture — never by calling the code under test:
    // `fn covered` spans exactly lines 1-3 (closing brace included) of the only indexed file;
    // projects a and b are named by their directory basenames, and the
    // scope itself encloses everything else (a decided miss for any
    // other repo name, 0063 §2 — never Unknown with a catalog).
    let scope_name = directory
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let kind_truth = |path: &str, value: &str, line: usize| -> Option<bool> {
        match path {
            // The file declares exactly one function (lines 1-3);
            // every other kind is definitely absent. `fn` is the
            // function chip (0109).
            "a/src/x.rs" => Some(matches!(value, "function" | "fn") && (1..=3).contains(&line)),
            _ => None,
        }
    };
    let repo_truth = |path: &str, value: &str| -> Option<bool> {
        let name = if path.starts_with("a/") {
            "a"
        } else if path.starts_with("b/") {
            "b"
        } else {
            scope_name.as_str()
        };
        Some(name == value)
    };

    let mut rand = Lcg(0x5EED_0063);
    for _ in 0..5000 {
        let expr = random_expr(&mut rand, 3);
        // case:sensitive keeps content comparison identical on both
        // sides (smart case would duplicate case rules in the
        // reference for no algebraic gain).
        // The double negation guarantees the Boolean parse for any
        // generated tree — parens alone stay literal in operator-free
        // queries (byte-identical compat, 0063 §3) — while evaluating
        // to exactly the wrapped expression.
        let source = format!("case:sensitive NOT (NOT ({}))", expr.to_query_string());
        let query = crate::query::parse_search(&source);
        assert_eq!(
            query.state,
            QueryState::Ready,
            "generated canonical query must parse: {source}"
        );
        let plan = ContentPlan::compile(&query).unwrap().unwrap();
        let boolean = query.boolean.clone().expect("operator-bearing");
        for path in PATHS {
            for (line_index, line) in LINES.iter().enumerate() {
                let line_number = line_index + 1;
                let row = RowFacts {
                    path: Some(path),
                    line: Some(line_number),
                    text: line,
                    ..RowFacts::default()
                };
                let evidence = Evidence {
                    catalog: Some(&catalog),
                    symbols: Some(&index),
                };
                let admitted = plan.admits(&row, &evidence);
                let expected = reference(
                    &boolean,
                    line,
                    path,
                    line_number,
                    &|value, path, line| kind_truth(path, value, line),
                    &|value, path| repo_truth(path, value),
                );
                assert_eq!(
                    admitted,
                    expected != Some(false),
                    "AST {}\npath {path} line {line:?}",
                    boolean.to_query_string()
                );
            }
        }
    }
}
