//! strop-grammar: the pure operator-pending resolver (0001 §5.2).
//!
//! One resolver, two consumers: the app executes what this resolves, the
//! renderer previews what this resolves. No UI code in here, ever.

mod parse;
mod resolve;
#[cfg(test)]
mod tests;

pub use parse::parse;
pub use resolve::{
    cursor_after, plan, resolve, search_all, search_backward, search_forward, ActionPlan,
    PlannedTarget,
};

pub use types::*;
mod types;
