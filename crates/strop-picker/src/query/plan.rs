//! The semantic compilation step (0051 §3): one `FileSelectionPlan`
//! consumed by file find, grep and replace, and a `ContentPlan` for the
//! search expression. Filters compile once per query revision; changing
//! only the needle never rebuilds the file catalog.

use strop_core::languages;

use super::parser::{CaseMode, ContentExpr, QueryDiagnostic, SearchQuery};

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
    pub regex: regex::Regex,
    pub case: CaseMode,
}

impl ContentPlan {
    /// Compile the content expression. A content surface REQUIRES an
    /// expression (0051 §3: a filter-only grep explains itself instead
    /// of searching every byte).
    pub fn compile(query: &SearchQuery) -> Result<Option<Self>, QueryDiagnostic> {
        let Some(expr) = &query.content else {
            return Ok(None);
        };
        let case = query.case.unwrap_or(CaseMode::Smart);
        let regex = Self::compile_expression(expr, case).map_err(|mut diagnostic| {
            diagnostic.range = query.content_range();
            diagnostic
        })?;
        Ok(Some(ContentPlan {
            expr: expr.clone(),
            regex,
            case,
        }))
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
}
