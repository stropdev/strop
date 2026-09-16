//! 0063 §6.1/§6.2 verification: deterministic property corpora and an
//! independent reference interpreter. Everything here is hermetic — a
//! fixed-seed LCG (no RNG dependency, no wall clock), tempdirs for the
//! evidence sources, bounded iteration counts.

use super::parser::{BooleanExpr, ContentAtom, MetadataAtom, SearchQuery};
use super::plan::{ContentPlan, Evidence};

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
        let query = SearchQuery::parse(&raw);
        assert!(
            query.state == super::parser::QueryState::Ready
                || query.state == super::parser::QueryState::Incomplete
                || query.state == super::parser::QueryState::Invalid,
            "a parse must settle: {raw:?}"
        );
        // Compiling a Ready query must also hold: regex bounds are the
        // parser's own diagnostics, never a panic.
        if query.state == super::parser::QueryState::Ready {
            let _ = ContentPlan::compile(&query);
        }
    }
}

#[test]
fn canonical_formatting_is_a_fixed_point_for_ready_queries() {
    let mut rand = Lcg(0x0FF1CE);
    for _ in 0..2000 {
        let raw = random_query(&mut rand);
        let query = SearchQuery::parse(&raw);
        if query.state != super::parser::QueryState::Ready {
            continue;
        }
        let Some(expr) = query.boolean.clone() else {
            continue;
        };
        let once = expr.to_query_string();
        let reparsed = SearchQuery::parse(&once);
        assert_eq!(
            reparsed.state,
            super::parser::QueryState::Ready,
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
        // Only the documented exception upgrades a flat query: the
        // kind:/repo:/type: evidence qualifiers (0063 §2). Inputs with
        // standalone operators or parens are legitimately Boolean.
        let evidence_bearing =
            raw.contains("kind:") || raw.contains("repo:") || raw.contains("type:");
        let operator_bearing = raw.contains(['(', ')'])
            || raw
                .split_whitespace()
                .any(|word| matches!(word, "AND" | "OR" | "NOT"));
        if evidence_bearing || operator_bearing {
            continue;
        }
        let query = SearchQuery::parse(&raw);
        assert!(
            query.boolean.is_none(),
            "no operators, no evidence qualifiers: the query must parse byte-identically to the legacy grammar: {raw:?}"
        );
    }
}

// ---- §6.2: the reference interpreter --------------------------------------

/// The kind values the query language recognizes (0063 §2) — the
/// reference's own table, mirroring the documented contract.
const KIND_VALUES: &[&str] = &[
    "function",
    "method",
    "class",
    "struct",
    "enum",
    "interface",
    "module",
    "constant",
];

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
                "glob" => Some(reference_glob(&atom.value, path)),
                "kind" => {
                    if KIND_VALUES.contains(&atom.value.as_str()) {
                        kind(atom.value.as_str(), path, line_number)
                    } else {
                        None
                    }
                }
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
                ("kind", "banana"),
                ("repo", "a"),
                ("repo", "b"),
                ("repo", "zz"),
                ("language", "rust"),
                ("language", "python"),
                ("path", "src"),
                ("path", "zz"),
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
            // The file declares exactly one function (lines 1-2);
            // every other recognized kind is definitely absent, and
            // `function` subsumes methods (0063 §2).
            "a/src/x.rs" => Some(matches!(value, "function" | "method") && (1..=3).contains(&line)),
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
        let query = SearchQuery::parse(&source);
        assert_eq!(
            query.state,
            super::parser::QueryState::Ready,
            "generated canonical query must parse: {source}"
        );
        let plan = ContentPlan::compile(&query).unwrap().unwrap();
        let boolean = query.boolean.clone().expect("operator-bearing");
        for path in PATHS {
            for (line_index, line) in LINES.iter().enumerate() {
                let line_number = line_index + 1;
                let admitted = plan.admits(
                    Evidence {
                        catalog: Some(&catalog),
                        symbols: Some(&index),
                        path: Some(path),
                        line: Some(line_number),
                        symbol: None,
                    },
                    line,
                );
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
