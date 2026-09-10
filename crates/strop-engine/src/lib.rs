//! strop editor engine: documents, grammar dispatch, services, sessions — no
//! terminal. The binary (`crates/strop`) owns the physical terminal, cell-grid
//! rendering, the CLI, self-update, bench and headless frame rendering; it
//! drives this engine and injects frame capture at the composition roots.
pub mod config;
pub mod editor;
pub mod files;
pub mod keymap;
pub mod session;
