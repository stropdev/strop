use strop_core::languages;
use strop_syntax::symbols::SymbolKind;

use super::super::parser::{
    BooleanExpr, CaseMode, MetadataAtom, QueryDiagnostic, QueryState, SearchQuery,
};
use super::{build_regex, content::ReplacementTarget};

/// The exact Boolean evaluator (0063 §3/§4): one typed AST with
/// precompiled atom regexes, admitted per (path, line). Content atoms
/// evaluate against the complete logical line; metadata atoms evaluate
/// against the file's path/extension. NOT never turns failed evidence
/// into a match: unknown or empty operands are errors at compile, not
/// false at runtime.
#[derive(Debug, Clone)]
pub struct BooleanPlan {
    root: CompiledExpr,
}

#[derive(Debug, Clone)]
enum CompiledExpr {
    And(Vec<CompiledExpr>),
    Or(Vec<CompiledExpr>),
    Not(Box<CompiledExpr>),
    Content(regex::Regex),
    Metadata {
        key: MetadataKey,
        value: String,
        negated: bool,
        path_pattern: Option<regex::Regex>,
        case: CaseMode,
    },
    /// An atom this build cannot decide yet (`kind:`, `repo:` before
    /// project discovery). It admits its line — overfetch, never a
    /// silent drop — and neutralizes any NOT above it, because unknown
    /// evidence must not become false (0063 §4).
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataKey {
    Language,
    Path,
    Glob,
    FuzzyPath,
    Repo,
    Kind,
}

impl BooleanPlan {
    /// Exact admission for one candidate line. `Evidence::default()`
    /// (a line-less surface) keeps every metadata atom Unknown.
    pub fn admits(&self, evidence: &Evidence<'_>, line: &str) -> bool {
        self.root.decide(evidence, line) != Decision::No
    }

    /// Whether the AST consults `kind:` — the signal to build the
    /// syntax-fallback symbol index (0063 §2).
    pub fn uses_kind(&self) -> bool {
        self.root.uses(MetadataKey::Kind)
    }

    /// Whether the AST consults `repo:` — the signal to discover the
    /// scope's project boundaries before searching (0093 §4.3).
    pub fn uses_repo(&self) -> bool {
        self.root.uses(MetadataKey::Repo)
    }

    pub(super) fn compile_plan(
        expr: &BooleanExpr,
        case: CaseMode,
        range: std::ops::Range<usize>,
    ) -> Result<Self, QueryDiagnostic> {
        Ok(Self {
            root: Self::compile(expr, case, range)?,
        })
    }

    fn compile(
        expr: &BooleanExpr,
        case: CaseMode,
        range: std::ops::Range<usize>,
    ) -> Result<CompiledExpr, QueryDiagnostic> {
        let atom = |regex: bool, text: &str| {
            let pattern = if regex {
                text.to_string()
            } else {
                regex::escape(text)
            };
            build_regex(&pattern, case).map_err(|error| QueryDiagnostic {
                range: range.clone(),
                message: error.message,
                suggestion: None,
            })
        };
        Ok(match expr {
            BooleanExpr::And(operands) => CompiledExpr::And(
                operands
                    .iter()
                    .map(|operand| Self::compile(operand, case, range.clone()))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            BooleanExpr::Or(branches) => CompiledExpr::Or(
                branches
                    .iter()
                    .map(|branch| Self::compile(branch, case, range.clone()))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            BooleanExpr::Not(inner) => {
                CompiledExpr::Not(Box::new(Self::compile(inner, case, range.clone())?))
            }
            BooleanExpr::Content(atom_spec) => {
                CompiledExpr::Content(atom(atom_spec.regex, &atom_spec.text)?)
            }
            BooleanExpr::Metadata(atom) => {
                let key = match atom.key.as_str() {
                    "language" => MetadataKey::Language,
                    "path" => MetadataKey::Path,
                    "fpath" => MetadataKey::FuzzyPath,
                    "glob" => MetadataKey::Glob,
                    "repo" => MetadataKey::Repo,
                    // Decided by the syntax-fallback symbol index when
                    // the search built one (0063 §2); otherwise the
                    // atom stays Unknown and admits.
                    "kind" => MetadataKey::Kind,
                    _ => return Ok(CompiledExpr::Unknown),
                };
                let path_pattern = if key == MetadataKey::Path {
                    Some(
                        build_regex(&regex::escape(&atom.value), case).map_err(|error| {
                            QueryDiagnostic {
                                range: range.clone(),
                                message: error.message,
                                suggestion: None,
                            }
                        })?,
                    )
                } else {
                    None
                };
                CompiledExpr::Metadata {
                    key,
                    value: atom.value.clone(),
                    negated: atom.negated,
                    path_pattern,
                    case,
                }
            }
        })
    }

    /// The provider prefilter pattern: alternation of every positive
    /// content atom. Negative terms contribute nothing (0063 §4); a
    /// query with no positive content atom matches every line.
    pub(super) fn prefilter_pattern(expr: &BooleanExpr, out: &mut Vec<String>) {
        match expr {
            BooleanExpr::And(operands) => {
                for inner in operands {
                    Self::prefilter_pattern(inner, out);
                }
            }
            BooleanExpr::Or(branches) => {
                for inner in branches {
                    Self::prefilter_pattern(inner, out);
                }
            }
            BooleanExpr::Not(_) => {}
            BooleanExpr::Content(atom) => out.push(if atom.regex {
                format!("(?:{})", atom.text)
            } else {
                regex::escape(&atom.text)
            }),
            BooleanExpr::Metadata(_) => {}
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

    /// Whole-query admission for retained-list surfaces (0063 §4):
    /// the Boolean AST when the query carries operators, else the flat
    /// selection qualifiers (`language:`/`path:`/`fpath:`/`glob:` and
    /// their negatives) as an implicit conjunction — the narrowing file
    /// find already gives. Free text never enters admission here (the
    /// local ranker narrows it, 0063 §3 literal compatibility);
    /// `Ok(None)` when the query narrows nothing.
    pub fn compile_retained(query: &SearchQuery) -> Result<Option<Self>, QueryDiagnostic> {
        let case = query.case_mode();
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
        if let Some(boolean) = &query.boolean {
            return Ok(Some(Self {
                root: Self::compile(boolean, case, 0..query.source.len())?,
            }));
        }
        let mut families: Vec<BooleanExpr> = Vec::new();
        let mut family = |values: &[String], negated: bool, key: &'static str| {
            if values.is_empty() {
                return;
            }
            let atoms = values
                .iter()
                .map(|value| {
                    BooleanExpr::Metadata(MetadataAtom {
                        key: key.into(),
                        value: value.clone(),
                        negated,
                    })
                })
                .collect::<Vec<_>>();
            // Negation rides the atom's own flag, exactly like the
            // parser's lowering (`-language:`): the flag flips the
            // decision, no BooleanExpr::Not on top.
            if atoms.len() == 1 {
                let Some(atom) = atoms.into_iter().next() else {
                    panic!("one atom");
                };
                families.push(atom);
            } else {
                families.push(BooleanExpr::Or(atoms));
            }
        };
        let owned: Vec<String> = query
            .languages
            .iter()
            .map(|lang| lang.to_string())
            .collect();
        family(&owned, false, "language");
        let owned: Vec<String> = query
            .exclude_languages
            .iter()
            .map(|lang| lang.to_string())
            .collect();
        family(&owned, true, "language");
        family(&query.paths, false, "path");
        family(&query.exclude_paths, true, "path");
        family(&query.fuzzy_paths, false, "fpath");
        family(&query.exclude_fuzzy_paths, true, "fpath");
        family(&query.globs, false, "glob");
        family(&query.exclude_globs, true, "glob");
        if families.is_empty() {
            return Ok(None);
        }
        let conjunction = if families.len() == 1 {
            let Some(family) = families.into_iter().next() else {
                panic!("one family");
            };
            family
        } else {
            BooleanExpr::And(families)
        };
        Ok(Some(Self {
            root: Self::compile(&conjunction, case, 0..query.source.len())?,
        }))
    }
}

impl CompiledExpr {
    /// Three-valued evaluation (0063 §4): failed or unavailable evidence
    /// is Unknown — never false, so `NOT` cannot turn a miss into a
    /// match, and a top-level Unknown admits (overfetch).
    fn decide(&self, evidence: &Evidence<'_>, line: &str) -> Decision {
        match self {
            Self::Unknown => Decision::Unknown,
            Self::And(operands) => {
                let mut result = Decision::Yes;
                for operand in operands {
                    match operand.decide(evidence, line) {
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
                    match branch.decide(evidence, line) {
                        Decision::Yes => return Decision::Yes,
                        Decision::Unknown => result = Decision::Unknown,
                        Decision::No => {}
                    }
                }
                result
            }
            Self::Not(inner) => match inner.decide(evidence, line) {
                Decision::Yes => Decision::No,
                Decision::No => Decision::Yes,
                Decision::Unknown => Decision::Unknown,
            },
            Self::Content(regex) => {
                if regex.is_match(line) {
                    Decision::Yes
                } else if evidence
                    .symbol
                    .and_then(|symbol| symbol.qualified)
                    .is_some_and(|qualified| regex.is_match(qualified))
                {
                    // Symbol candidates also admit on their
                    // qualified/container name (0063 §4).
                    Decision::Yes
                } else {
                    Decision::No
                }
            }
            Self::Metadata {
                key,
                value,
                negated,
                path_pattern,
                case,
            } => {
                // A symbol candidate decides `kind:` from its own
                // classification — no path needed; a provider kind
                // with no canonical mapping stays Unknown (0063 §4).
                if *key == MetadataKey::Kind {
                    if let Some(symbol) = evidence.symbol {
                        let Some(kinds) =
                            crate::source::symbols::SymbolIndex::kinds_for_value(value)
                        else {
                            return Decision::Unknown;
                        };
                        return match symbol.kind {
                            Some(kind) => {
                                if kinds.contains(&kind) != *negated {
                                    Decision::Yes
                                } else {
                                    Decision::No
                                }
                            }
                            None => Decision::Unknown,
                        };
                    }
                }
                let Some(path) = evidence.path else {
                    return Decision::Unknown;
                };
                let matched = match key {
                    MetadataKey::Language => path
                        .rsplit('.')
                        .next()
                        .and_then(languages::language_for_extension_name)
                        .is_some_and(|name| {
                            languages::extensions_for_language(value)
                                .is_some_and(|known| known == extensions_for_language_name(name))
                        }),
                    MetadataKey::Path => path_pattern
                        .as_ref()
                        .is_some_and(|pattern| pattern.is_match(path)),
                    MetadataKey::FuzzyPath => {
                        crate::score::fuzzy_score_with_case(value, path, *case).is_some()
                    }
                    MetadataKey::Glob => glob_literal_match(value, path),
                    MetadataKey::Repo => {
                        let Some(catalog) = evidence.catalog else {
                            return Decision::Unknown;
                        };
                        catalog.repo_matches(path, value, *negated)
                    }
                    MetadataKey::Kind => {
                        let Some(kinds) =
                            crate::source::symbols::SymbolIndex::kinds_for_value(value)
                        else {
                            return Decision::Unknown;
                        };
                        let Some(symbols) = evidence.symbols else {
                            return Decision::Unknown;
                        };
                        return match symbols.kind_decides(path, evidence.line, kinds) {
                            Some(true) => {
                                if *negated {
                                    Decision::No
                                } else {
                                    Decision::Yes
                                }
                            }
                            Some(false) => {
                                if *negated {
                                    Decision::Yes
                                } else {
                                    Decision::No
                                }
                            }
                            None => Decision::Unknown,
                        };
                    }
                };
                if matched != *negated {
                    Decision::Yes
                } else {
                    Decision::No
                }
            }
        }
    }

    fn uses(&self, wanted: MetadataKey) -> bool {
        match self {
            Self::And(operands) | Self::Or(operands) => {
                operands.iter().any(|operand| operand.uses(wanted))
            }
            Self::Not(inner) => inner.uses(wanted),
            Self::Content(_) => false,
            Self::Metadata { key, .. } => *key == wanted,
            Self::Unknown => false,
        }
    }
}

/// Symbol-candidate evidence (0063 §4): present when the candidate IS
/// a declaration rather than a line inside a file — `kind:` decides
/// from the candidate's own classification and content atoms also try
/// the qualified/container form of the name. Absent fields stay
/// Unknown — admitting, never false.
#[derive(Debug, Clone, Copy, Default)]
pub struct SymbolEvidence<'a> {
    /// The declaration's classified kind; `None` when the provider's
    /// vocabulary has no canonical mapping.
    pub kind: Option<SymbolKind>,
    /// Qualified form of the name (e.g. `Class::method`), when the
    /// provider reports a container.
    pub qualified: Option<&'a str>,
}

/// Positive content atoms by NOT-parity: a atom under an even number
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
        CompiledExpr::Metadata { .. } | CompiledExpr::Unknown => {}
    }
}

/// Per-candidate evidence for exact admission (0063 §4): everything
/// the AST may consult beyond the line text. Missing evidence keeps
/// its atoms Unknown — admitting, never false.
#[derive(Default)]
pub struct Evidence<'a> {
    /// The project catalog deciding `repo:` atoms.
    pub catalog: Option<&'a crate::source::catalog::ProjectCatalog>,
    /// The syntax-fallback symbol index deciding `kind:` atoms.
    pub symbols: Option<&'a crate::source::symbols::SymbolIndex>,
    /// Workspace-relative path of the candidate's file.
    pub path: Option<&'a str>,
    /// 1-based line number; `None` on line-less surfaces.
    pub line: Option<usize>,
    /// Symbol-candidate evidence; `None` on line-oriented surfaces.
    pub symbol: Option<SymbolEvidence<'a>>,
}

/// Three-valued admission outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Yes,
    No,
    Unknown,
}

/// Bounded literal-glob semantics for admission: `*` and `**` span path
/// separators or characters, everything else is literal.
fn glob_literal_match(pattern: &str, path: &str) -> bool {
    // Classic single-resume wildcard match: every `*` spans any run
    // of characters (including `/`); everything else compares
    // literally. A mismatch backtracks to the most recent star and
    // advances its match point one character — the old first-match
    // scan silently rejected unanchored stars like `*.rs`.
    let mut pattern = pattern;
    let mut path = path;
    // The most recent star: the pattern tail after it, and the path
    // position it currently matches from (`mark`). A mismatch rewinds
    // here with `mark` advanced one character — the classic resume.
    let mut star: Option<&str> = None;
    let mut mark = "";
    loop {
        if let Some(at) = pattern.find('*') {
            let (literal, rest) = pattern.split_at(at);
            if path.starts_with(literal) {
                mark = &path[literal.len()..];
                pattern = rest.trim_start_matches('*');
                path = mark;
                star = Some(pattern);
            } else if let Some(after) = star {
                if mark.is_empty() {
                    return false;
                }
                let Some(head) = mark.chars().next() else {
                    panic!("nonempty mark");
                };
                mark = &mark[head.len_utf8()..];
                pattern = after;
                path = mark;
            } else {
                return false;
            }
        } else if pattern == path {
            return true;
        } else if let Some(after) = star {
            if mark.is_empty() {
                return false;
            }
            let Some(head) = mark.chars().next() else {
                panic!("nonempty mark");
            };
            mark = &mark[head.len_utf8()..];
            pattern = after;
            path = mark;
        } else {
            return false;
        }
    }
}

fn extensions_for_language_name(name: &str) -> &'static [&'static str] {
    languages::extensions_for_language(name).unwrap_or(&[])
}
