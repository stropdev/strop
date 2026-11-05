use strop_core::languages;

use super::super::parser::{CaseMode, QueryDiagnostic, QueryState, SearchQuery};
use super::build_regex;

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
    path_patterns: Vec<regex::Regex>,
    exclude_path_patterns: Vec<regex::Regex>,
    /// Explicit fuzzy path scope (positive = OR within the family).
    pub fuzzy_paths: Vec<String>,
    pub exclude_fuzzy_paths: Vec<String>,
    case: CaseMode,
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
        let case = query.case_mode();
        let compile_paths = |values: &[String], negated: bool| {
            values
                .iter()
                .map(|value| {
                    build_regex(&regex::escape(value), case).map_err(|error| QueryDiagnostic {
                        range: query.qualifier_range("path", value, negated),
                        message: error.message,
                        suggestion: None,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
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
            path_patterns: compile_paths(&query.paths, false)?,
            exclude_path_patterns: compile_paths(&query.exclude_paths, true)?,
            fuzzy_paths: query.fuzzy_paths.clone(),
            exclude_fuzzy_paths: query.exclude_fuzzy_paths.clone(),
            case,
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
            .exclude_path_patterns
            .iter()
            .any(|pattern| pattern.is_match(rel_path))
        {
            return false;
        }
        if !self.path_patterns.is_empty()
            && !self
                .path_patterns
                .iter()
                .any(|pattern| pattern.is_match(rel_path))
        {
            return false;
        }
        if self.exclude_fuzzy_paths.iter().any(|needle| {
            crate::score::fuzzy_score_with_case(needle, rel_path, self.case).is_some()
        }) {
            return false;
        }
        if !self.fuzzy_paths.is_empty()
            && !self.fuzzy_paths.iter().any(|needle| {
                crate::score::fuzzy_score_with_case(needle, rel_path, self.case).is_some()
            })
        {
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
            && self.fuzzy_paths.is_empty()
            && self.exclude_fuzzy_paths.is_empty()
            && self.include_set.is_empty()
            && self.exclude_paths.is_empty()
            && self.exclude_set.is_empty()
            && self.exclude_extensions.is_empty()
    }
}
