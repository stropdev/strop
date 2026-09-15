//! The semantic compilation step (0051 §3): one `FileSelectionPlan`
//! consumed by file find, grep and replace, and a `ContentPlan` for the
//! search expression. Filters compile once per query revision; changing
//! only the needle never rebuilds the file catalog.

use strop_core::languages;

use super::parser::{BooleanExpr, CaseMode, ContentExpr, QueryDiagnostic, SearchQuery};

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
                } else {
                    Decision::No
                }
            }
            Self::Metadata {
                key,
                value,
                negated,
            } => {
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
                        let Some(symbols) = evidence.symbols else {
                            return Decision::Unknown;
                        };
                        let Some(kinds) =
                            crate::source::symbols::SymbolIndex::kinds_for_value(value)
                        else {
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
mod tests {
    use super::*;
    use crate::query::parser::SearchQuery;

    fn plan(input: &str) -> (SearchQuery, FileSelectionPlan) {
        let query = SearchQuery::parse(input);
        let plan = FileSelectionPlan::compile(&query).unwrap();
        (query, plan)
    }

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
        let query = SearchQuery::parse("glob:\"[bad\" x");
        let diagnostic = FileSelectionPlan::compile(&query).unwrap_err();
        assert!(diagnostic.message.contains("invalid glob"));
        assert_eq!(&query.source[diagnostic.range], "glob:\"[bad\"");
    }

    #[test]
    fn filter_only_content_surface_needs_an_expression() {
        let query = SearchQuery::parse("language:rust");
        let content = ContentPlan::compile(&query).unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn literal_is_escaped_and_case_smart_default() {
        let query = SearchQuery::parse("a.b");
        let plan = ContentPlan::compile(&query).unwrap().unwrap();
        let re = plan.regex;
        assert!(re.is_match("a.b"));
        assert!(!re.is_match("aXb"));
        // smart case: lowercase needle matches either case
        assert!(re.is_match("A.B"));
    }

    #[test]
    fn explicit_regex_and_case() {
        let query = SearchQuery::parse("case:sensitive regex:\"\\brequest_(id|name)\\b\"");
        let plan = ContentPlan::compile(&query).unwrap().unwrap();
        let re = plan.regex;
        assert!(re.is_match("request_id"));
        assert!(!re.is_match("REQUEST_ID"));
    }

    #[test]
    fn invalid_regex_is_a_diagnostic() {
        let query = SearchQuery::parse("regex:\"(unclosed\"");
        assert!(ContentPlan::compile(&query).is_err());
    }

    fn content_plan(input: &str) -> ContentPlan {
        let query = SearchQuery::parse(input);
        assert_eq!(
            query.state,
            super::super::QueryState::Ready,
            "{input}: {:?}",
            query.diagnostics
        );
        ContentPlan::compile(&query)
            .unwrap()
            .expect("content required")
    }

    fn admits_line(plan: &ContentPlan, path: &str, line: &str) -> bool {
        plan.admits(
            Evidence {
                path: Some(path),
                ..Evidence::default()
            },
            line,
        )
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
                        plan.regex.is_match(line),
                        "{input}: prefilter missed admitted line {line:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn type_alias_normalizes_to_kind() {
        let query = SearchQuery::parse("type:class AND parser");
        assert_eq!(query.state, super::super::QueryState::Ready);
        assert!(matches!(query.boolean, Some(BooleanExpr::And(_))));
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
        let plan = content_plan("(repo:engine OR repo:tools) NOT glob:**/vendor/** parser");
        // Exact admission with the catalog: branch-sensitive.
        assert!(plan.admits(
            Evidence {
                catalog: Some(&catalog),
                path: Some("engine/src/a.rs"),
                ..Evidence::default()
            },
            "parser here"
        ));
        assert!(plan.admits(
            Evidence {
                catalog: Some(&catalog),
                path: Some("tools/b.py"),
                ..Evidence::default()
            },
            "parser here"
        ));
        assert!(!plan.admits(
            Evidence {
                catalog: Some(&catalog),
                path: Some("other/c.rs"),
                ..Evidence::default()
            },
            "parser here"
        ));
        assert!(!plan.admits(
            Evidence {
                catalog: Some(&catalog),
                path: Some("engine/vendor/x.rs"),
                ..Evidence::default()
            },
            "parser here"
        ));
        // NOT repo: inverts exactly with a catalog.
        let outside = content_plan("NOT repo:engine AND parser");
        assert!(!outside.admits(
            Evidence {
                catalog: Some(&catalog),
                path: Some("engine/src/a.rs"),
                ..Evidence::default()
            },
            "parser"
        ));
        assert!(outside.admits(
            Evidence {
                catalog: Some(&catalog),
                path: Some("tools/b.py"),
                ..Evidence::default()
            },
            "parser"
        ));
        // Without a catalog (remote until 0058's worker): repo: is
        // Unknown — the line admits when the decided atoms allow it.
        assert!(admits_line(&plan, "other/c.rs", "parser here"));
        assert!(!admits_line(&plan, "other/c.rs", "nothing"));
    }
    #[test]
    fn flat_kind_and_repo_are_implicit_ands() {
        // Flat spellings upgrade to the Boolean parse (0063 §2):
        // `kind:class needle` narrows exactly like `kind:class AND
        // needle` — the evidence exists now — and `type:` aliases.
        let query = SearchQuery::parse("kind:class needle");
        assert_eq!(query.state, super::super::QueryState::Ready);
        assert!(query.boolean.is_some());
        let aliased = SearchQuery::parse("type:class needle");
        assert_eq!(aliased.state, super::super::QueryState::Ready);
        assert_eq!(
            aliased.boolean.as_ref().unwrap().to_query_string(),
            query.boolean.as_ref().unwrap().to_query_string()
        );
        let repo = SearchQuery::parse("repo:engine parser");
        assert_eq!(repo.state, super::super::QueryState::Ready);
        assert!(repo.boolean.is_some());
        // Without evidence both still admit (Unknown, 0063 §4).
        let plan = ContentPlan::compile(&query).unwrap().unwrap();
        assert!(admits_line(&plan, "src/a.rs", "class Parser { needle }"));
        assert!(!admits_line(&plan, "src/a.rs", "class Parser {}"));
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
        fn evidence_for<'a>(
            index: &'a crate::source::symbols::SymbolIndex,
            path: &'a str,
            line: usize,
        ) -> Evidence<'a> {
            Evidence {
                symbols: Some(index),
                path: Some(path),
                line: Some(line),
                ..Evidence::default()
            }
        }
        let index = crate::source::symbols::SymbolIndex::build(
            root,
            &[std::path::PathBuf::from("src/lib.rs")],
            &|| false,
        );

        let plan = ContentPlan::compile(&SearchQuery::parse("kind:function parser"))
            .unwrap()
            .unwrap();
        assert!(plan.needs_symbols());
        // Line 3 (inside `wrap`): content and kind both Yes.
        assert!(plan.admits(evidence_for(&index, "src/lib.rs", 3), "    let parser = 1;"));
        // Line 5 (`bare`, outside every function): complete evidence
        // answers No — the line drops.
        assert!(!plan.admits(evidence_for(&index, "src/lib.rs", 5), "let bare = parser;"));
        // Unknown file: overfetch, never a silent drop.
        assert!(plan.admits(evidence_for(&index, "src/other.rs", 1), "parser"));
        // No index at all: Unknown admits.
        assert!(plan.admits(
            Evidence {
                path: Some("src/lib.rs"),
                line: Some(5),
                ..Evidence::default()
            },
            "let bare = parser;"
        ));

        // NOT inverts exactly under complete evidence (0063 §4): a
        // parser line outside functions now matches.
        let negated = ContentPlan::compile(&SearchQuery::parse("NOT kind:function AND parser"))
            .unwrap()
            .unwrap();
        assert!(negated.admits(evidence_for(&index, "src/lib.rs", 5), "let bare = parser;"));
        assert!(!negated.admits(evidence_for(&index, "src/lib.rs", 3), "    let parser = 1;"));

        // Branches stay sensitive: struct line matches the OR, function
        // line does not.
        let branches = ContentPlan::compile(&SearchQuery::parse("(kind:class OR kind:struct) x"))
            .unwrap()
            .unwrap();
        assert!(branches.admits(evidence_for(&index, "src/lib.rs", 1), "struct St; // x"));
        assert!(!branches.admits(evidence_for(&index, "src/lib.rs", 3), "let x = 1;"));

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
        let methods = ContentPlan::compile(&SearchQuery::parse("kind:method AND k"))
            .unwrap()
            .unwrap();
        assert!(methods.admits(evidence_for(&index, "src/impl.rs", 4), "        let k = 1;"));
        let banana = ContentPlan::compile(&SearchQuery::parse("kind:banana AND k"))
            .unwrap()
            .unwrap();
        // An unrecognized kind value stays Unknown — admit, explain
        // via suggestions, never drop.
        assert!(banana.admits(evidence_for(&index, "src/lib.rs", 5), "let k = 2;"));
    }

    #[test]
    fn plain_queries_never_build_the_symbol_index() {
        let plan = ContentPlan::compile(&SearchQuery::parse("parser AND request"))
            .unwrap()
            .unwrap();
        assert!(!plan.needs_symbols());
        let flat = ContentPlan::compile(&SearchQuery::parse("parser"))
            .unwrap()
            .unwrap();
        assert!(!flat.needs_symbols());
        let repo_only = ContentPlan::compile(&SearchQuery::parse("repo:engine AND parser"))
            .unwrap()
            .unwrap();
        assert!(!repo_only.needs_symbols());
    }

    #[test]
    fn boolean_case_modes_apply_to_atoms() {
        let smart = content_plan("text:Parser AND text:Request");
        assert!(admits_line(&smart, "x.rs", "Parser Request"));
        assert!(!admits_line(&smart, "x.rs", "parser request"));
        let ignore = content_plan("case:ignore text:Parser AND text:Request");
        assert!(admits_line(&ignore, "x.rs", "parser request"));
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
            ContentPlan::compile(&SearchQuery::parse(input))
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
    }
}
