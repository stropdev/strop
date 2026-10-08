//! Source-owned code completion. Provider state remains outside input/render.
pub mod acceptance;
mod admission;
mod apply;
mod context;
mod delivery;
mod interaction;
pub(crate) use interaction::KeyDisposition;
mod lifecycle;
mod mailbox;
mod model;
mod presentation;
pub use presentation::{
    CompletionDocumentation, CompletionMenu, CompletionProviderStatus, CompletionRow,
};
mod provider;
mod session;
mod worker;
pub(crate) use session::CompletionState;
#[cfg(test)]
mod tests;
mod words;
