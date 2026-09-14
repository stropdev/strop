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
}

impl BooleanPlan {
    /// Exact admission for one candidate line of one file (workspace-
    /// relative path). `None` path treats path metadata as not matching.
    pub fn admits(&self, path: Option<&str>, line: &str) -> bool {
        self.root.matches(path, line)
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
            BooleanExpr::And(operands) => collapse_and(
                operands
                    .iter()
                    .map(|operand| Self::compile(operand, case, range.clone()))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            BooleanExpr::Or(branches) => collapse_or(
                branches
                    .iter()
                    .map(|branch| Self::compile(branch, case, range.clone()))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            BooleanExpr::Not(inner) => {
                let inner = Self::compile(inner, case, range.clone())?;
                if contains_unknown(&inner) {
                    CompiledExpr::Unknown
                } else {
                    CompiledExpr::Not(Box::new(inner))
                }
            }
            BooleanExpr::Content(atom_spec) => {
                CompiledExpr::Content(atom(atom_spec.regex, &atom_spec.text)?)
            }
            BooleanExpr::Metadata(atom) => {
                let key = match atom.key.as_str() {
                    "language" => MetadataKey::Language,
                    "path" => MetadataKey::Path,
                    "glob" => MetadataKey::Glob,
                    // `kind:`/`repo:` await project discovery (0063 §2).
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

/// Unknown inside AND drops out (the decided operands still bind);
/// inside OR it widens to Unknown; under NOT it neutralizes.
fn collapse_and(mut operands: Vec<CompiledExpr>) -> CompiledExpr {
    operands.retain(|operand| !matches!(operand, CompiledExpr::Unknown));
    CompiledExpr::And(operands)
}

fn collapse_or(branches: Vec<CompiledExpr>) -> CompiledExpr {
    if branches
        .iter()
        .any(|branch| matches!(branch, CompiledExpr::Unknown))
    {
        CompiledExpr::Unknown
    } else {
        CompiledExpr::Or(branches)
    }
}

fn contains_unknown(expr: &CompiledExpr) -> bool {
    match expr {
        CompiledExpr::Unknown => true,
        CompiledExpr::And(operands) => operands.iter().any(contains_unknown),
        CompiledExpr::Or(branches) => branches.iter().any(contains_unknown),
        CompiledExpr::Not(inner) => contains_unknown(inner),
        CompiledExpr::Content(_) | CompiledExpr::Metadata { .. } => false,
    }
}

impl CompiledExpr {
    fn matches(&self, path: Option<&str>, line: &str) -> bool {
        match self {
            Self::Unknown => true,
            Self::And(operands) => operands.iter().all(|operand| operand.matches(path, line)),
            Self::Or(branches) => branches.iter().any(|branch| branch.matches(path, line)),
            Self::Not(inner) => !inner.matches(path, line),
            Self::Content(regex) => regex.is_match(line),
            Self::Metadata {
                key,
                value,
                negated,
            } => {
                let Some(path) = path else {
                    return *negated;
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
                };
                matched != *negated
            }
        }
    }
}

/// Bounded literal-glob semantics for admission: `*` and `**` span path
/// separators or characters, everything else is literal.
fn glob_literal_match(pattern: &str, path: &str) -> bool {
    let mut pattern = pattern;
    let mut path = path;
    loop {
        match pattern.find('*') {
            None => break pattern == path,
            Some(star) => {
                let (literal, rest) = pattern.split_at(star);
                let Some(found) = path.find(literal) else {
                    return false;
                };
                path = &path[found + literal.len()..];
                pattern = rest.trim_start_matches('*');
                if pattern.is_empty() {
                    return true;
                }
            }
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

    /// Exact per-line admission (0063 §4). Simple queries are the regex
    /// itself; Boolean queries evaluate the AST with path metadata.
    pub fn matches_line(&self, line: &str) -> bool {
        match &self.boolean {
            Some(exact) => exact.admits(None, line),
            None => self.regex.is_match(line),
        }
    }

    /// Exact admission with the file's path for metadata atoms.
    pub fn admits(&self, path: &str, line: &str) -> bool {
        match &self.boolean {
            Some(exact) => exact.admits(Some(path), line),
            None => self.regex.is_match(line),
        }
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

    #[test]
    fn boolean_admits_same_line_conjunction_and_exclusion() {
        // 0063 §4: AND means both literals on ONE logical line; NOT
        // excludes the line. Git-grep line semantics, not file-level.
        let plan = content_plan("text:\"retry\" AND text:\"request\" NOT text:\"test\"");
        assert!(plan.admits("src/a.rs", "let request = retry(request);"));
        assert!(!plan.admits("src/a.rs", "let request = retry(request); // test"));
        assert!(!plan.admits("src/a.rs", "let request = x;"));
        // Different lines are different candidates: neither line alone
        // satisfies the conjunction.
        assert!(!plan.admits("src/a.rs", "let retry = retry(x);"));
        assert!(!plan.admits("src/a.rs", "let request = y;"));
    }

    #[test]
    fn boolean_metadata_narrows_by_path_and_language() {
        let plan = content_plan("(language:python OR language:cpp) parser");
        assert!(plan.admits("pkg/main.py", "class Parser:"));
        assert!(plan.admits("src/main.cpp", "Parser parser;"));
        assert!(!plan.admits("src/main.rs", "struct Parser;"));
    }

    #[test]
    fn boolean_branches_do_not_flatten() {
        // (path:a AND foo) OR (path:b AND bar) — the branch relationship
        // is preserved: a-with-bar or b-with-foo do not match (0063 §4).
        let plan = content_plan("(path:alpha AND foo) OR (path:beta AND bar)");
        assert!(plan.admits("alpha/x.rs", "fn foo() {}"));
        assert!(plan.admits("beta/y.rs", "fn bar() {}"));
        assert!(!plan.admits("alpha/x.rs", "fn bar() {}"));
        assert!(!plan.admits("beta/y.rs", "fn foo() {}"));
    }

    #[test]
    fn not_never_turns_failure_into_a_match() {
        // Unknown evidence (a path with no extension) must not become
        // false inside NOT and admit the line (0063 §4).
        let plan = content_plan("NOT language:rust AND parser");
        assert!(!plan.admits("src/main.rs", "struct Parser;"));
        assert!(plan.admits("src/main.py", "class Parser:"));
        assert!(plan.admits("noext", "parser here"));
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
                if plan.admits(path, line) {
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
        assert!(plan.admits("any/x.rs", "a parser here"));
        assert!(!plan.admits("any/x.rs", "nothing relevant"));
    }

    #[test]
    fn not_never_drops_lines_through_a_pending_qualifier() {
        // The soundness core (0063 §4): unknown evidence under NOT must
        // not become false and silently drop every line.
        let plan = content_plan("NOT kind:function AND parser");
        assert!(plan.admits("any/x.rs", "a parser here"));
        assert!(!plan.admits("any/x.rs", "nothing relevant"));
        let widened = content_plan("(repo:engine OR repo:tools) AND parser");
        assert!(widened.admits("elsewhere/y.py", "parser"));
    }

    #[test]
    fn flat_kind_or_repo_explains_itself() {
        let query = SearchQuery::parse("kind:class needle");
        assert_ne!(query.state, super::super::QueryState::Ready);
        assert!(query
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("workspace symbols")));
    }

    #[test]
    fn boolean_case_modes_apply_to_atoms() {
        let smart = content_plan("text:Parser AND text:Request");
        assert!(smart.admits("x.rs", "Parser Request"));
        assert!(!smart.admits("x.rs", "parser request"));
        let ignore = content_plan("case:ignore text:Parser AND text:Request");
        assert!(ignore.admits("x.rs", "parser request"));
    }
}
