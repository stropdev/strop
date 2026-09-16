//! The semantic compilation step (0051 §3): one `FileSelectionPlan`
//! consumed by file find, grep and replace, and a `ContentPlan` for the
//! search expression. Filters compile once per query revision; changing
//! only the needle never rebuilds the file catalog.

use strop_core::languages;
use strop_syntax::symbols::SymbolKind;

use super::parser::{BooleanExpr, CaseMode, ContentExpr, QueryDiagnostic, QueryState, SearchQuery};

/// One compiled file-selection policy: which paths are eligible.
/// Predicates are structured data — the adapters never re-parse text.
#[derive(Debug, Clone, Default)]
pub struct FileSelectionPlan {
    /// Extension sets of the included languages (empty = all languages).
    pub extensions: Vec<&'static [&'static str]>,
    pub exclude_extensions: Vec<&'static [&'static str]>,
    /// Literal path substrings (positive = OR within the family).
    pub paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    /// Precompiled at plan time — `allows` runs per candidate.
    include_set: globset::GlobSet,
    exclude_set: globset::GlobSet,
    /// None = surface/session default; Some = explicit override.
    pub hidden: Option<bool>,
    pub ignored: Option<bool>,
}

impl FileSelectionPlan {
    /// Compile the query's file filters. Invalid globs are located
    /// diagnostics, never silently ignored scope.
    pub fn compile(query: &SearchQuery) -> Result<Self, QueryDiagnostic> {
        if query.state != super::QueryState::Ready {
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
        let mut includes = globset::GlobSetBuilder::new();
        let mut excludes = globset::GlobSetBuilder::new();
        for (negated, values) in [(false, &query.globs), (true, &query.exclude_globs)] {
            for value in values {
                let glob = globset::Glob::new(value).map_err(|error| QueryDiagnostic {
                    range: query.qualifier_range("glob", value, negated),
                    message: format!("invalid glob {value}: {error}"),
                    suggestion: None,
                })?;
                if negated {
                    excludes.add(glob);
                } else {
                    includes.add(glob);
                }
            }
        }
        let set_error = |error| QueryDiagnostic {
            range: 0..query.source.len(),
            message: format!("glob set: {error}"),
            suggestion: None,
        };
        Ok(Self {
            extensions: query
                .languages
                .iter()
                .filter_map(|name| languages::extensions_for_language(name))
                .collect(),
            exclude_extensions: query
                .exclude_languages
                .iter()
                .filter_map(|name| languages::extensions_for_language(name))
                .collect(),
            paths: query.paths.clone(),
            exclude_paths: query.exclude_paths.clone(),
            include_set: includes.build().map_err(set_error)?,
            exclude_set: excludes.build().map_err(set_error)?,
            hidden: query.hidden,
            ignored: query.ignored,
        })
    }

    /// One extension's language membership check.
    fn extension_allowed(&self, ext: &str) -> bool {
        if self.exclude_extensions.iter().any(|set| set.contains(&ext)) {
            return false;
        }
        self.extensions.is_empty() || self.extensions.iter().any(|set| set.contains(&ext))
    }

    /// Whether a workspace-relative path is eligible. Directory parts
    /// are evaluated with the same predicates (`.git` is handled by the
    /// walk policy, not by query filters).
    pub fn allows(&self, rel_path: &str) -> bool {
        if self
            .exclude_paths
            .iter()
            .any(|needle| rel_path.contains(needle))
        {
            return false;
        }
        if !self.paths.is_empty() && !self.paths.iter().any(|needle| rel_path.contains(needle)) {
            return false;
        }
        if !self.include_set.is_empty() && !self.include_set.is_match(rel_path) {
            return false;
        }
        if self.exclude_set.is_match(rel_path) {
            return false;
        }
        match rel_path.rsplit('.').next() {
            Some(ext) if rel_path.contains('.') => self.extension_allowed(ext),
            _ => self.extensions.is_empty(),
        }
    }

    /// No file predicates at all (the walk passes everything eligible
    /// by policy).
    pub fn is_unfiltered(&self) -> bool {
        self.extensions.is_empty()
            && self.paths.is_empty()
            && self.include_set.is_empty()
            && self.exclude_paths.is_empty()
            && self.exclude_set.is_empty()
            && self.exclude_extensions.is_empty()
    }
}

/// The compiled search expression and its case behavior.
#[derive(Debug, Clone)]
pub struct ContentPlan {
    pub expr: ContentExpr,
    /// The provider prefilter: one regex matching every line the AST can
    /// admit (the union of positive content atoms). Exact admission is
    /// `matches_line`/`admits` — the prefilter only overfetches (0063 §4).
    pub regex: regex::Regex,
    pub case: CaseMode,
    boolean: Option<BooleanPlan>,
    /// The pattern a line provider (rg) runs: the exact expression for
    /// simple queries, the positive-atom union prefilter for Boolean
    /// ones (0063 §4).
    provider: String,
}

impl ContentPlan {
    /// The provider pattern (`rg -e`). `-F` only for simple literals.
    pub fn provider_pattern(&self) -> &str {
        &self.provider
    }
    pub fn is_literal(&self) -> bool {
        self.boolean.is_none() && matches!(self.expr, ContentExpr::Literal(_))
    }
}

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
        self.root.uses_kind()
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
                    "glob" => MetadataKey::Glob,
                    "repo" => MetadataKey::Repo,
                    // Decided by the syntax-fallback symbol index when
                    // the search built one (0063 §2); otherwise the
                    // atom stays Unknown and admits.
                    "kind" => MetadataKey::Kind,
                    _ => return Ok(CompiledExpr::Unknown),
                };
                CompiledExpr::Metadata {
                    key,
                    value: atom.value.clone(),
                    negated: atom.negated,
                }
            }
        })
    }

    /// The provider prefilter pattern: alternation of every positive
    /// content atom. Negative terms contribute nothing (0063 §4); a
    /// query with no positive content atom matches every line.
    fn prefilter_pattern(expr: &BooleanExpr, out: &mut Vec<String>) {
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
                    MetadataKey::Path => path.contains(value.as_str()),
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

    fn uses_kind(&self) -> bool {
        match self {
            Self::And(operands) | Self::Or(operands) => operands.iter().any(Self::uses_kind),
            Self::Not(inner) => inner.uses_kind(),
            Self::Content(_) => false,
            Self::Metadata { key, .. } => *key == MetadataKey::Kind,
            Self::Unknown => false,
        }
    }
}

/// What a With/Review flow may replace (0063 §4): the single
/// identifiable positive target. Negative terms and metadata constrain
/// admission but are never targets; more than one positive atom is an
/// explicit ambiguity, not a silent first-match.
#[derive(Debug, Clone)]
pub enum ReplacementTarget {
    /// Exactly one positive content atom, compiled with the query's
    /// case behavior.
    Single(regex::Regex),
    /// N positive content atoms: a valid search, an ambiguous replace.
    Ambiguous(usize),
    /// No positive content atom: nothing identifiable to replace.
    Missing,
}

impl ContentPlan {
    /// The plan's replaceable target. Simple queries target their
    /// content expression; Boolean queries target the unique positive
    /// content atom (parity of `NOT` respected — `NOT NOT x` is x).
    pub fn replacement_target(&self) -> ReplacementTarget {
        match &self.boolean {
            None => ReplacementTarget::Single(self.regex.clone()),
            Some(exact) => exact.replacement_target(),
        }
    }
}

impl BooleanPlan {
    fn replacement_target(&self) -> ReplacementTarget {
        let mut atoms: Vec<&regex::Regex> = Vec::new();
        collect_positive_atoms(&self.root, false, &mut atoms);
        match atoms.len() {
            0 => ReplacementTarget::Missing,
            1 => ReplacementTarget::Single(atoms[0].clone()),
            count => ReplacementTarget::Ambiguous(count),
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
                let head = mark.chars().next().expect("nonempty mark");
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
            let head = mark.chars().next().expect("nonempty mark");
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

impl ContentPlan {
    /// Compile the content expression. A content surface REQUIRES an
    /// expression (0051 §3: a filter-only grep explains itself instead
    /// of searching every byte).
    pub fn compile(query: &SearchQuery) -> Result<Option<Self>, QueryDiagnostic> {
        let case = query.case.unwrap_or(CaseMode::Smart);
        if let Some(boolean) = &query.boolean {
            if query.state != super::QueryState::Ready {
                return Err(query.diagnostics.first().cloned().unwrap_or_else(|| {
                    QueryDiagnostic {
                        range: 0..query.source.len(),
                        message: "incomplete query".into(),
                        suggestion: None,
                    }
                }));
            }
            let exact = BooleanPlan::compile(boolean, case, query.content_range())?;
            let mut positive = Vec::new();
            BooleanPlan::prefilter_pattern(boolean, &mut positive);
            let pattern = if positive.is_empty() {
                // Pure-negative or metadata-only: providers must surface
                // every line; exact admission decides.
                "()".to_string()
            } else {
                positive.join("|")
            };
            let regex = build_regex(&pattern, case).map_err(|error| QueryDiagnostic {
                range: query.content_range(),
                message: error.message,
                suggestion: None,
            })?;
            return Ok(Some(ContentPlan {
                expr: ContentExpr::Regex(boolean.to_query_string()),
                regex,
                case,
                boolean: Some(BooleanPlan { root: exact }),
                provider: pattern,
            }));
        }
        let Some(expr) = &query.content else {
            return Ok(None);
        };
        let regex = Self::compile_expression(expr, case).map_err(|mut diagnostic| {
            diagnostic.range = query.content_range();
            diagnostic
        })?;
        let provider = match expr {
            ContentExpr::Literal(text) | ContentExpr::Regex(text) => text.clone(),
        };
        Ok(Some(ContentPlan {
            expr: expr.clone(),
            regex,
            case,
            boolean: None,
            provider,
        }))
    }

    /// Exact per-line admission on a line-less surface (directory
    /// names): metadata atoms stay Unknown (0063 §4).
    pub fn matches_line(&self, line: &str) -> bool {
        self.admits(Evidence::default(), line)
    }

    /// Exact admission with the file's path for metadata atoms.
    pub fn admits(&self, evidence: Evidence<'_>, line: &str) -> bool {
        match &self.boolean {
            Some(exact) => exact.admits(&evidence, line),
            None => self.regex.is_match(line),
        }
    }

    /// Whether building the syntax-fallback symbol index pays off for
    /// this query — only `kind:` atoms consult it (0063 §2).
    pub fn needs_symbols(&self) -> bool {
        self.boolean.as_ref().is_some_and(|exact| exact.uses_kind())
    }

    pub fn compile_expression(
        expr: &ContentExpr,
        case: CaseMode,
    ) -> Result<regex::Regex, QueryDiagnostic> {
        match expr {
            ContentExpr::Literal(text) => build_regex(&regex::escape(text), case),
            ContentExpr::Regex(pattern) => build_regex(pattern, case),
        }
    }
}

impl SearchQuery {
    /// The `workspace/symbol` wire probe (0063 §4): only bare content
    /// text crosses — the positive content atoms' literal text joined
    /// by spaces, or empty when the query has none. Qualifiers,
    /// operators and regex patterns never reach the LSP wire; returned
    /// candidates are filtered locally against the full AST. `None`
    /// when the query is not Ready — a broken query asks nothing.
    pub fn wsymbols_probe(&self) -> Option<String> {
        if self.state != QueryState::Ready {
            return None;
        }
        Some(match &self.boolean {
            Some(expr) => {
                let mut atoms = Vec::new();
                positive_literal_atoms(expr, false, &mut atoms);
                atoms.join(" ")
            }
            None => match &self.content {
                Some(ContentExpr::Literal(text)) => text.clone(),
                Some(ContentExpr::Regex(_)) | None => String::new(),
            },
        })
    }
}

/// Positive non-regex content atoms by NOT-parity, in tree order:
/// `NOT NOT x` is x again; regex patterns stay local.
fn positive_literal_atoms(expr: &BooleanExpr, negated: bool, out: &mut Vec<String>) {
    match expr {
        BooleanExpr::And(operands) | BooleanExpr::Or(operands) => {
            for operand in operands {
                positive_literal_atoms(operand, negated, out);
            }
        }
        BooleanExpr::Not(inner) => positive_literal_atoms(inner, !negated, out),
        BooleanExpr::Content(atom) => {
            if !negated && !atom.regex {
                out.push(atom.text.clone());
            }
        }
        BooleanExpr::Metadata(_) => {}
    }
}

fn build_regex(pattern: &str, case: CaseMode) -> Result<regex::Regex, QueryDiagnostic> {
    let insensitive = match case {
        CaseMode::Ignore => true,
        // smart: an all-lowercase pattern matches either case
        CaseMode::Smart => !pattern.chars().any(|c| c.is_uppercase()),
        CaseMode::Sensitive => false,
    };
    regex::RegexBuilder::new(pattern)
        .case_insensitive(insensitive)
        .build()
        .map_err(|error| QueryDiagnostic {
            range: 0..0,
            message: format!("invalid regex: {error}"),
            suggestion: None,
        })
}

#[cfg(test)]
mod tests;
