//! The filter language (0051 §3, 0109): one bounded grammar for every
//! list that takes filters. One lexer, one parser, one row decision —
//! the parse result drives execution, highlighting and suggestions from
//! the same spans, and each surface's [`Schema`] says what it accepts.

mod facts;
pub mod highlight;
pub mod lexer;
mod modes;
pub mod parser;
pub mod plan;
#[cfg(test)]
mod properties;
pub mod schema;
#[cfg(test)]
mod schema_tests;
pub mod store;
pub mod suggest;

pub use facts::{ItemFacts, RowFacts, SymbolRow};
pub use highlight::{HighlightSpan, Role};
pub use modes::SearchModes;
pub use parser::{CaseMode, ContentExpr, QueryDiagnostic, QueryState, SearchQuery};
pub use plan::{Admission, ContentPlan, Evidence, FileSelectionPlan, ReplacementTarget};
pub use schema::{Bare, Field, Schema};
pub use store::{
    canonical_source, migrate_v1, StoredQuery, StoredQueryError, SYNTAX_VERSION_CURRENT,
};

/// Parse for the widest surface (Search, no toggles): the grammar's own
/// tests use it where the surface does not matter.
#[cfg(test)]
pub(crate) fn parse_search(input: &str) -> SearchQuery {
    SearchQuery::parse(input, &schema::SEARCH, SearchModes::default())
}
