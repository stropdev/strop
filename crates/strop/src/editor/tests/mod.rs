//! Editing-behavior contract tests (0003 §5, 0006 tier 1): state
//! assertions drive the real Editor — no terminal, no timing.

pub use super::*;

mod alignment_tests;
mod edit_tests;
mod hardening_tests;
mod indent_tests;
mod keybinds_tests;
mod register_tests;
mod reviewer_battery;
mod scratch_tests;
mod search_tests;
mod smartindent_tests;
mod surround_tests;
mod transaction_tests;
mod undo_tests;
mod visual_object_tests;
