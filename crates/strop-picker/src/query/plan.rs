//! The semantic compilation step (0051 §3): one `FileSelectionPlan`
//! consumed by file find, grep and replace, and a `ContentPlan` for the
//! search expression. Filters compile once per query revision; changing
//! only the needle never rebuilds the file catalog.

mod boolean;
mod content;
mod file;

pub use boolean::{BooleanPlan, Evidence, SymbolEvidence};
pub use content::{ContentPlan, ReplacementTarget};
pub use file::FileSelectionPlan;

use super::parser::{CaseMode, QueryDiagnostic};

fn build_regex(pattern: &str, case: CaseMode) -> Result<regex::Regex, QueryDiagnostic> {
    let insensitive = match case {
        CaseMode::Ignore => true,
        CaseMode::Smart => !has_capital(pattern),
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

/// Smart case asks whether the pattern *spells* a capital: the letter
/// after a backslash is an escape (`\S`, `\W`, `\B`), not text.
fn has_capital(pattern: &str) -> bool {
    let mut escaped = false;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c.is_uppercase() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests;
