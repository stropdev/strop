use strop_core::languages;
use strop_core::symbol_kind::{self, KindSpec};

use super::super::facts::RowFacts;
use super::super::parser::{
    BooleanExpr, CaseMode, MetadataAtom, QueryDiagnostic, QueryState, SearchQuery,
};
use super::super::schema::Field;
use super::{build_regex, content::ReplacementTarget};

/// The one row decision (0109 §3.2d): the Boolean AST and the flat
/// filters as one conjunction, with every atom compiled once, judged
/// per row over borrowed [`RowFacts`]. Content atoms match the row's
/// text (or a symbol's container); metadata atoms its path, kind and
/// line. NOT never turns missing evidence into a match: a row a filter
/// cannot judge stays (0063 §4).
#[derive(Debug, Clone)]
pub struct Admission {
    root: CompiledExpr,
}

#[derive(Debug, Clone)]
enum CompiledExpr {
    And(Vec<CompiledExpr>),
    Or(Vec<CompiledExpr>),
    Not(Box<CompiledExpr>),
    Content(regex::Regex),
    Metadata { test: Test, negated: bool },
}

#[derive(Debug, Clone)]
enum Test {
    Language(&'static [&'static str]),
    Path(regex::Regex),
    FuzzyPath(String, CaseMode),
    Glob(globset::GlobMatcher),
    Repo(String),
    Kind(&'static KindSpec),
}

/// What a worker owns beyond the row: the evidence `repo:` and a
/// line's enclosing `kind:` are decided from. Absent evidence keeps
/// those atoms undecided.
#[derive(Default, Clone, Copy)]
pub struct Evidence<'a> {
    /// The project catalog deciding `repo:` atoms.
    pub catalog: Option<&'a crate::source::catalog::ProjectCatalog>,
    /// The syntax-fallback symbol index deciding `kind:` for a line.
    pub symbols: Option<&'a crate::source::symbols::SymbolIndex>,
}

/// Three-valued admission outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Yes,
    No,
    Unknown,
}

impl Decision {
    fn of(matched: bool, negated: bool) -> Self {
        if matched != negated {
            Decision::Yes
        } else {
            Decision::No
        }
    }
}

impl Admission {
    /// The whole query's row decision: the Boolean AST when the query
    /// carries operators, and the flat filters (`language:`, `path:`,
    /// `fpath:`, `glob:`, `kind:`, `repo:` and their negatives) as an
    /// implicit conjunction. Flat bare words never enter it — they are
    /// the ranked needle. `Ok(None)` when the query filters nothing.
    pub fn compile(query: &SearchQuery) -> Result<Option<Self>, QueryDiagnostic> {
        if query.state != QueryState::Ready {
            return Err(query
                .diagnostics
                .first()
                .cloned()
                .unwrap_or_else(|| QueryDiagnostic {
                    range: 0..query.source.len(),
                    message: "incomplete query".into(),
                    suggestion: None,
                }));
        }
        let mut conjuncts: Vec<BooleanExpr> = query.boolean.iter().cloned().collect();
        let mut family = |values: Vec<String>, negated: bool, field: Field| {
            let mut atoms = values.into_iter().map(|value| {
                BooleanExpr::Metadata(MetadataAtom {
                    key: field.key().into(),
                    value,
                    negated,
                })
            });
            if negated {
                // every exclusion holds on its own
                conjuncts.extend(atoms);
            } else if let (Some(first), rest) = (atoms.next(), atoms.collect::<Vec<_>>()) {
                // any one inclusion of a family admits
                conjuncts.push(if rest.is_empty() {
                    first
                } else {
                    BooleanExpr::Or(std::iter::once(first).chain(rest).collect())
                });
            }
        };
        let names = |languages: &[&'static str]| languages.iter().map(|l| l.to_string()).collect();
        let values =
            |kinds: &[&'static KindSpec]| kinds.iter().map(|k| k.value.to_string()).collect();
        family(names(&query.languages), false, Field::Language);
        family(names(&query.exclude_languages), true, Field::Language);
        family(query.paths.clone(), false, Field::Path);
        family(query.exclude_paths.clone(), true, Field::Path);
        family(query.fuzzy_paths.clone(), false, Field::FuzzyPath);
        family(query.exclude_fuzzy_paths.clone(), true, Field::FuzzyPath);
        family(query.globs.clone(), false, Field::Glob);
        family(query.exclude_globs.clone(), true, Field::Glob);
        family(values(&query.kinds), false, Field::Kind);
        family(values(&query.exclude_kinds), true, Field::Kind);
        family(query.repos.clone(), false, Field::Repo);
        family(query.exclude_repos.clone(), true, Field::Repo);
        let expr = match conjuncts.len() {
            0 => return Ok(None),
            1 => conjuncts.remove(0),
            _ => BooleanExpr::And(conjuncts),
        };
        Self::compile_expr(&expr, query.case_mode(), 0..query.source.len()).map(Some)
    }

    pub(super) fn compile_expr(
        expr: &BooleanExpr,
        case: CaseMode,
        range: std::ops::Range<usize>,
    ) -> Result<Self, QueryDiagnostic> {
        Ok(Self {
            root: compile(expr, case, &range)?,
        })
    }

    /// Whether the row is kept.
    pub fn admits(&self, row: &RowFacts<'_>, evidence: &Evidence<'_>) -> bool {
        self.root.decide(row, evidence) != Decision::No
    }

    /// Whether `kind:` is judged here — the signal to build the
    /// syntax-fallback index for lines (0063 §2).
    pub fn uses_kind(&self) -> bool {
        self.root.uses(&|test| matches!(test, Test::Kind(_)))
    }

    /// Whether `repo:` is judged here — the signal to discover the
    /// scope's project boundaries first (0093 §4.3).
    pub fn uses_repo(&self) -> bool {
        self.root.uses(&|test| matches!(test, Test::Repo(_)))
    }

    /// The provider prefilter pattern: alternation of every positive
    /// content atom. Negative terms contribute nothing (0063 §4); a
    /// query with no positive content atom matches every line.
    pub(super) fn prefilter_pattern(expr: &BooleanExpr, out: &mut Vec<String>) {
        match expr {
            BooleanExpr::And(operands) | BooleanExpr::Or(operands) => {
                for inner in operands {
                    Self::prefilter_pattern(inner, out);
                }
            }
            BooleanExpr::Not(_) | BooleanExpr::Metadata(_) => {}
            BooleanExpr::Content(atom) => out.push(if atom.regex {
                format!("(?:{})", atom.text)
            } else {
                regex::escape(&atom.text)
            }),
        }
    }

    pub(super) fn replacement_target(&self) -> ReplacementTarget {
        let mut atoms: Vec<&regex::Regex> = Vec::new();
        collect_positive_atoms(&self.root, false, &mut atoms);
        match atoms.len() {
            0 => ReplacementTarget::Missing,
            1 => ReplacementTarget::Single(atoms[0].clone()),
            count => ReplacementTarget::Ambiguous(count),
        }
    }
}

fn compile(
    expr: &BooleanExpr,
    case: CaseMode,
    range: &std::ops::Range<usize>,
) -> Result<CompiledExpr, QueryDiagnostic> {
    let located = |message: String| QueryDiagnostic {
        range: range.clone(),
        message,
        suggestion: None,
    };
    let all = |operands: &[BooleanExpr]| {
        operands
            .iter()
            .map(|operand| compile(operand, case, range))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match expr {
        BooleanExpr::And(operands) => CompiledExpr::And(all(operands)?),
        BooleanExpr::Or(branches) => CompiledExpr::Or(all(branches)?),
        BooleanExpr::Not(inner) => CompiledExpr::Not(Box::new(compile(inner, case, range)?)),
        BooleanExpr::Content(atom) => {
            let pattern = if atom.regex {
                atom.text.clone()
            } else {
                regex::escape(&atom.text)
            };
            CompiledExpr::Content(build_regex(&pattern, case).map_err(|e| located(e.message))?)
        }
        BooleanExpr::Metadata(atom) => {
            let value = atom.value.as_str();
            let test = match Field::from_key(&atom.key) {
                Some(Field::Language) => Test::Language(
                    languages::extensions_for_language(value)
                        .ok_or_else(|| located(format!("unknown language: {value}")))?,
                ),
                Some(Field::Path) => Test::Path(
                    build_regex(&regex::escape(value), case).map_err(|e| located(e.message))?,
                ),
                Some(Field::FuzzyPath) => Test::FuzzyPath(value.to_string(), case),
                Some(Field::Glob) => Test::Glob(
                    globset::Glob::new(value)
                        .map_err(|error| located(format!("invalid glob {value}: {error}")))?
                        .compile_matcher(),
                ),
                Some(Field::Repo) => Test::Repo(value.to_string()),
                Some(Field::Kind) => match symbol_kind::resolve(value) {
                    symbol_kind::Resolved::Kind(spec) => Test::Kind(spec),
                    _ => return Err(located(format!("unknown kind: {value}"))),
                },
                _ => return Err(located(format!("{}: is not a row filter", atom.key))),
            };
            CompiledExpr::Metadata {
                test,
                negated: atom.negated,
            }
        }
    })
}

impl CompiledExpr {
    /// Three-valued evaluation (0063 §4): missing evidence is Unknown —
    /// never false, so `NOT` cannot turn a miss into a match, and a
    /// top-level Unknown keeps the row.
    fn decide(&self, row: &RowFacts<'_>, evidence: &Evidence<'_>) -> Decision {
        match self {
            Self::And(operands) => {
                let mut result = Decision::Yes;
                for operand in operands {
                    match operand.decide(row, evidence) {
                        Decision::No => return Decision::No,
                        Decision::Unknown => result = Decision::Unknown,
                        Decision::Yes => {}
                    }
                }
                result
            }
            Self::Or(branches) => {
                let mut result = Decision::No;
                for branch in branches {
                    match branch.decide(row, evidence) {
                        Decision::Yes => return Decision::Yes,
                        Decision::Unknown => result = Decision::Unknown,
                        Decision::No => {}
                    }
                }
                result
            }
            Self::Not(inner) => match inner.decide(row, evidence) {
                Decision::Yes => Decision::No,
                Decision::No => Decision::Yes,
                Decision::Unknown => Decision::Unknown,
            },
            Self::Content(regex) => Decision::of(
                regex.is_match(row.text) || row.container.is_some_and(|c| regex.is_match(c)),
                false,
            ),
            Self::Metadata { test, negated } => match test.judge(row, evidence) {
                Some(matched) => Decision::of(matched, *negated),
                None => Decision::Unknown,
            },
        }
    }

    fn uses(&self, wanted: &dyn Fn(&Test) -> bool) -> bool {
        match self {
            Self::And(operands) | Self::Or(operands) => {
                operands.iter().any(|operand| operand.uses(wanted))
            }
            Self::Not(inner) => inner.uses(wanted),
            Self::Content(_) => false,
            Self::Metadata { test, .. } => wanted(test),
        }
    }
}

impl Test {
    /// `None` when this row carries no evidence for the test.
    fn judge(&self, row: &RowFacts<'_>, evidence: &Evidence<'_>) -> Option<bool> {
        if let Test::Kind(spec) = self {
            // A symbol row decides from its own kind; a line decides
            // from the declaration around it, when an index was built.
            return match row.kind {
                Some(kind) => Some(spec.admits(kind)),
                None => evidence
                    .symbols?
                    .kind_decides(row.path?, row.line, spec.includes),
            };
        }
        let path = row.path?;
        Some(match self {
            Test::Language(extensions) => path
                .rsplit_once('.')
                .is_some_and(|(_, extension)| extensions.contains(&extension)),
            Test::Path(pattern) => pattern.is_match(path),
            Test::FuzzyPath(needle, case) => {
                crate::score::fuzzy_score_with_case(needle, path, *case).is_some()
            }
            Test::Glob(matcher) => matcher.is_match(path),
            Test::Repo(name) => return evidence.catalog.map(|c| c.repo_matches(path, name)),
            Test::Kind(_) => return None,
        })
    }
}

/// Positive content atoms by NOT-parity: an atom under an even number
/// of negations is a candidate target.
fn collect_positive_atoms<'a>(
    expr: &'a CompiledExpr,
    negated: bool,
    out: &mut Vec<&'a regex::Regex>,
) {
    match expr {
        CompiledExpr::And(operands) | CompiledExpr::Or(operands) => {
            for operand in operands {
                collect_positive_atoms(operand, negated, out);
            }
        }
        CompiledExpr::Not(inner) => collect_positive_atoms(inner, !negated, out),
        CompiledExpr::Content(regex) => {
            if !negated {
                out.push(regex);
            }
        }
        CompiledExpr::Metadata { .. } => {}
    }
}
