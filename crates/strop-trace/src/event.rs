//! Schema vocabulary shared by producers; payloads retain their domain's types.
//!
//! Schema 2 adds the closed forensic substream (`Replay` nodes, captured only
//! under the `Full` content policy) and the always-present terminal `TraceEnd`
//! marker that distinguishes a complete capture from a capped or failed one.
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

/// Hard upper bounds a capture may use. They exist so a runaway producer
/// cannot fill the disk; `start` refuses anything outside them, and the
/// reader applies the same totals, so writer and reader agree everywhere.
pub const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CAPTURE_EVENTS: u64 = 100_000;
pub const MAX_RECORD_BYTES: usize = 256 * 1024;
/// The writer always reserves this much of the byte budget for the
/// terminal marker, so a capture that hits its cap still ends legibly.
pub const TERMINAL_RESERVE: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    Replay,
    TraceEnd,
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

/// Bounded capture: total bytes, total events, per-record bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub bytes: usize,
    pub events: u64,
    pub record_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: MAX_CAPTURE_BYTES,
            events: MAX_CAPTURE_EVENTS,
            record_bytes: MAX_RECORD_BYTES,
        }
    }
}

impl Limits {
    /// Hard bounds only — there is no CLI knob that lifts them.
    pub fn valid(&self) -> bool {
        self.bytes >= TERMINAL_RESERVE * 2
            && self.bytes <= MAX_CAPTURE_BYTES
            && self.events > 0
            && self.events <= MAX_CAPTURE_EVENTS
            && self.record_bytes > 0
            && self.record_bytes <= MAX_RECORD_BYTES
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TraceOptions {
    pub content: ContentPolicy,
    pub limits: Limits,
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
