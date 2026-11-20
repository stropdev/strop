use super::super::facts::RowFacts;
use super::super::parser::{
    BooleanExpr, CaseMode, ContentExpr, QueryDiagnostic, QueryState, SearchQuery,
};
use super::{
    admission::{Admission, Evidence},
    build_regex,
};

/// The compiled search expression and its case behavior.
#[derive(Debug, Clone)]
pub struct ContentPlan {
    pub expr: ContentExpr,
    atoms: Atoms,
    pub case: CaseMode,
    /// The row decision beyond the expression: the Boolean AST, or the
    /// flat `kind:`/`repo:` filters the file selection cannot apply.
    admission: Option<Admission>,
    /// The pattern a line provider (rg) runs: the exact expression for
    /// simple queries, the positive-atom union prefilter for Boolean
    /// ones (0063 §4).
    provider: String,
}

#[derive(Debug, Clone)]
enum Atoms {
    Simple(regex::Regex),
    Boolean(Vec<regex::Regex>),
}

impl ContentPlan {
    /// The provider pattern (`rg -e`). `-F` only for simple literals.
    pub fn provider_pattern(&self) -> &str {
        &self.provider
    }

    pub fn is_literal(&self) -> bool {
        matches!(self.atoms, Atoms::Simple(_)) && matches!(self.expr, ContentExpr::Literal(_))
    }

    /// Smart-case exact admission is per atom. A Boolean provider union must
    /// not become case-sensitive merely because one sibling has uppercase:
    /// folding the prefilter is sound because exact admission remains per atom.
    pub fn provider_case(&self) -> CaseMode {
        if matches!(self.atoms, Atoms::Boolean(_)) && self.case == CaseMode::Smart {
            CaseMode::Ignore
        } else {
            self.case
        }
    }

    fn atoms(&self) -> &[regex::Regex] {
        match &self.atoms {
            Atoms::Simple(regex) => std::slice::from_ref(regex),
            Atoms::Boolean(atoms) => atoms,
        }
    }

    /// Hits come from the atoms, not their provider alternation: even a
    /// legitimate match of one atom can hide an overlapping sibling.
    pub(crate) fn spans<'a>(
        &'a self,
        text: &'a str,
    ) -> impl Iterator<Item = regex::Match<'a>> + 'a {
        let atoms = self.atoms();
        let mut seen = (atoms.len() > 1).then(std::collections::HashSet::new);
        atoms
            .iter()
            .flat_map(move |atom| atom.find_iter(text))
            .filter(move |hit| {
                seen.as_mut()
                    .is_none_or(|seen| seen.insert((hit.start(), hit.end())))
            })
    }

    /// The plan's replaceable target. Simple queries target their
    /// content expression; Boolean queries target the unique positive
    /// content atom (parity of `NOT` respected — `NOT NOT x` is x).
    pub fn replacement_target(&self) -> ReplacementTarget {
        match self.atoms() {
            [] => ReplacementTarget::Missing,
            [atom] => ReplacementTarget::Single(atom.clone()),
            many => ReplacementTarget::Ambiguous(many.len()),
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
            let exact = Admission::compile_expr(boolean, case, query.content_range())?;
            let positive = exact.positive_atoms();
            let mut pattern = String::new();
            for atom in &positive {
                if !pattern.is_empty() {
                    pattern.push('|');
                }
                pattern.push_str("(?:");
                pattern.push_str(atom.as_str());
                pattern.push(')');
            }
            if positive.is_empty() {
                // Line-only surfaces can admit rows without a content hit.
                pattern.push_str("()");
            }
            return Ok(Some(ContentPlan {
                expr: ContentExpr::Regex(boolean.to_query_string()),
                atoms: Atoms::Boolean(positive),
                case,
                admission: Some(exact),
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
        // The file selection already applies the path family; only the
        // filters it cannot judge make a per-line decision.
        let admission = if query.kinds.is_empty()
            && query.exclude_kinds.is_empty()
            && query.repos.is_empty()
            && query.exclude_repos.is_empty()
        {
            None
        } else {
            Admission::compile(query)?
        };
        Ok(Some(ContentPlan {
            expr: expr.clone(),
            atoms: Atoms::Simple(regex),
            case,
            admission,
            provider,
        }))
    }

    /// Exact admission of one name on a surface with no other facts
    /// (a folder's entries): the name is both the text and the path.
    pub fn matches_line(&self, line: &str) -> bool {
        let row = RowFacts {
            path: Some(line),
            text: line,
            ..RowFacts::default()
        };
        self.admits(&row, &Evidence::default())
    }

    /// Exact admission of one line with its path and line number.
    pub fn admits(&self, row: &RowFacts<'_>, evidence: &Evidence<'_>) -> bool {
        let matches = match &self.atoms {
            Atoms::Simple(regex) => regex.is_match(row.text),
            Atoms::Boolean(_) => true, // The admission tree already tests the atoms.
        };
        matches
            && self
                .admission
                .as_ref()
                .is_none_or(|admission| admission.admits(row, evidence))
    }

    /// Whether building the syntax-fallback symbol index pays off for
    /// this query — only `kind:` atoms consult it (0063 §2).
    pub fn needs_symbols(&self) -> bool {
        self.admission.as_ref().is_some_and(Admission::uses_kind)
    }

    /// Whether discovering project boundaries pays off for this query —
    /// only `repo:` atoms consult the catalog (0063 §2).
    pub fn needs_catalog(&self) -> bool {
        self.admission.as_ref().is_some_and(Admission::uses_repo)
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
    /// Whether the query names something to look for: free text, a
    /// `text:`/`regex:`, or a content atom outside any `NOT`. Filters
    /// alone select files; they are not a search.
    pub fn has_positive_content(&self) -> bool {
        match &self.boolean {
            Some(expr) => has_positive_atom(expr, false),
            None => self.content.is_some(),
        }
    }

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

fn has_positive_atom(expr: &BooleanExpr, negated: bool) -> bool {
    match expr {
        BooleanExpr::And(operands) | BooleanExpr::Or(operands) => operands
            .iter()
            .any(|operand| has_positive_atom(operand, negated)),
        BooleanExpr::Not(inner) => has_positive_atom(inner, !negated),
        BooleanExpr::Content(_) => !negated,
        BooleanExpr::Metadata(_) => false,
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
