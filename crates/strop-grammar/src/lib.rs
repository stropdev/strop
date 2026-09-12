//! strop-grammar: the pure operator-pending resolver (0001 §5.2) and
//! the compiled search query (0031 R5).
//!
//! One resolver, two consumers: the app executes what this resolves, the
//! renderer previews what this resolves. No UI code in here, ever.

mod parse;
mod query;
mod resolve;
#[cfg(test)]
mod tests;

pub use parse::parse;
pub use query::{
    search_all, search_backward, search_forward, search_visit, CompiledQuery, QueryError,
    SearchMatch,
};
pub use resolve::{
    cursor_after, delimiter_pair, find_character, literal_from, matching_delimiter_at, plan,
    resolve, resolve_many, resolve_many_cancellable, word_run, ActionPlan, MatchCancelled,
    PlannedTarget,
};

pub use types::*;
mod types;
