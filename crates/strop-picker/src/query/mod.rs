//! The query language (0051 §3): one bounded qualifier grammar shared
//! by file find (`Space f`), content search (`Space /`) and replace
//! (`Space R`). One lexer, one parser, one compiler — the parse result
//! drives execution, highlighting and suggestions from the same spans.

pub mod highlight;
pub mod lexer;
pub mod parser;
pub mod plan;
pub mod suggest;

pub use highlight::{HighlightSpan, Role};
pub use parser::{CaseMode, ContentExpr, QueryDiagnostic, QueryState, SearchQuery};
pub use plan::{ContentPlan, FileSelectionPlan};
