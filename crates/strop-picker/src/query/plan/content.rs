use super::super::parser::{
    BooleanExpr, CaseMode, ContentExpr, QueryDiagnostic, QueryState, SearchQuery,
};
use super::{
    boolean::{BooleanPlan, Evidence},
    build_regex,
};

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

    /// Smart-case exact admission is per atom. A Boolean provider union must
    /// not become case-sensitive merely because one sibling has uppercase:
    /// folding the prefilter is sound because exact admission remains per atom.
    pub fn provider_case(&self) -> CaseMode {
        if self.boolean.is_some() && self.case == CaseMode::Smart {
            CaseMode::Ignore
        } else {
            self.case
        }
    }

    /// The plan's replaceable target. Simple queries target their
    /// content expression; Boolean queries target the unique positive
    /// content atom (parity of `NOT` respected — `NOT NOT x` is x).
    pub fn replacement_target(&self) -> ReplacementTarget {
        match &self.boolean {
            None => ReplacementTarget::Single(self.regex.clone()),
            Some(exact) => exact.replacement_target(),
        }
    }

    /// Compile the content expression. A content surface REQUIRES an
    /// expression (0051 §3: a filter-only grep explains itself instead
    /// of searching every byte).
    pub fn compile(query: &SearchQuery) -> Result<Option<Self>, QueryDiagnostic> {
        let case = query.case_mode();
        if let Some(boolean) = &query.boolean {
            if query.state != QueryState::Ready {
                return Err(query.diagnostics.first().cloned().unwrap_or_else(|| {
                    QueryDiagnostic {
                        range: 0..query.source.len(),
                        message: "incomplete query".into(),
                        suggestion: None,
                    }
                }));
            }
            let exact = BooleanPlan::compile_plan(boolean, case, query.content_range())?;
            let mut positive = Vec::new();
            BooleanPlan::prefilter_pattern(boolean, &mut positive);
            let pattern = if positive.is_empty() {
                // Pure-negative or metadata-only: providers must surface
                // every line; exact admission decides.
                "()".to_string()
            } else {
                positive.join("|")
            };
            let regex = build_regex(
                &pattern,
                if case == CaseMode::Smart {
                    CaseMode::Ignore
                } else {
                    case
                },
            )
            .map_err(|error| QueryDiagnostic {
                range: query.content_range(),
                message: error.message,
                suggestion: None,
            })?;
            return Ok(Some(ContentPlan {
                expr: ContentExpr::Regex(boolean.to_query_string()),
                regex,
                case,
                boolean: Some(exact),
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

    /// Whether discovering project boundaries pays off for this query —
    /// only `repo:` atoms consult the catalog (0063 §2).
    pub fn needs_catalog(&self) -> bool {
        self.boolean.as_ref().is_some_and(|exact| exact.uses_repo())
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
