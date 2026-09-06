//! Schema vocabulary shared by producers; payloads retain their domain's types.
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStart,
    SessionEnd,
    Input,
    Paste,
    State,
    Document,
    Mutation,
    History,
    Render,
    Resize,
    JobStarted,
    JobFinished,
    JobRejected,
    LspMessage,
    Error,
    Panic,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ContentPolicy {
    /// Keys, paths and bounded previews are still sensitive; this is not redaction.
    #[default]
    Metadata,
    /// Include whole document/paste payloads for reproduction, explicitly opted in.
    Full,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TraceOptions {
    pub content: ContentPolicy,
}

/// UTF-8-safe bounded preview. The byte length and full text are separate fields.
pub fn preview(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
