//! Values crossing the completion worker boundary. Ownership is query-local;
//! neither a candidate label nor a live language-server connection is authority.
use super::acceptance::AcceptanceRefusal;
use super::context::CompletionContext;
use super::words::WordSuggestions;
use std::sync::Arc;
use strop_core::worker::{Outcome, WorkerId};
use strop_lsp::completion::{CompletionList, CompletionReply};
use strop_lsp::ReplyContext;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionQuery {
    pub request: WorkerId,
    pub context: Arc<CompletionContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum CandidateKey {
    Word(Arc<str>),
    Language {
        request: strop_lsp::RequestStamp,
        ordinal: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum DeliveryScope {
    Query,
    Selection(WorkerId),
    Preparation { selection: WorkerId, work: WorkerId },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct LanguageSuggestions {
    /// The original server identity survives safe complete-list refiltering.
    /// The containing query owns the CURRENT source/selection revision.
    pub origin: ReplyContext,
    pub owner: Arc<super::context::LanguageContext>,
    pub items: CompletionList,
    pub resolve_required: bool,
    pub prefix: Arc<str>,
    pub suffix: Arc<str>,
}
impl LanguageSuggestions {
    /// Cache only complete, self-contained plain insertions. Explicit source
    /// ranges/imports and lazy edits require a fresh server operation rather
    /// than pretending an old range is valid after typing.
    pub fn reusable(&self) -> bool {
        !self.resolve_required
            && !self.items.server_incomplete
            && !self.items.client_truncated
            && self.items.omitted_oversized == 0
            && self.items.items.iter().all(|entry| {
                let item = &entry.item.protocol();
                item.text_edit.is_none()
                    && item
                        .additional_text_edits
                        .as_ref()
                        .is_none_or(Vec::is_empty)
                    && item.command.is_none()
                    && item.insert_text_format.is_none_or(|format| {
                        format == strop_lsp::completion::InsertTextFormat::PLAIN_TEXT
                    })
                    && item
                        .insert_text_mode
                        .is_none_or(|mode| mode == strop_lsp::completion::InsertTextMode::AS_IS)
            })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedDocumentation {
    pub text: Arc<str>,
    pub spans: Vec<strop_syntax::Span>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionWorkStats {
    pub builds: u64,
    pub incremental_updates: u64,
    pub scanned_bytes: u64,
    pub queries: u64,
    pub indexed_words: usize,
    pub indexed_word_bytes: usize,
    pub retired_snapshots: u64,
    pub retirement_high_water_bytes: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum CompletionPayload {
    Words {
        outcome: Outcome<WordSuggestions>,
        work: CompletionWorkStats,
    },
    Language(Outcome<LanguageSuggestions>),
    Resolved {
        candidate: CandidateKey,
        reply: CompletionReply,
    },
    Documentation {
        candidate: CandidateKey,
        outcome: Outcome<PreparedDocumentation>,
    },
    Acceptance {
        candidate: CandidateKey,
        prepared: Result<(), AcceptanceRefusal>,
    },
}
impl CompletionPayload {
    pub fn slot(&self) -> usize {
        match self {
            Self::Words { .. } => 0,
            Self::Language(_) => 1,
            Self::Resolved { .. } => 2,
            Self::Documentation { .. } => 3,
            Self::Acceptance { .. } => 4,
        }
    }
    /// Conservative admission charges, not an RSS estimate. Language values
    /// already passed the protocol's text/node limits; charging maximum node
    /// storage avoids traversing opaque server data on the interactive thread.
    pub fn charged_bytes(&self) -> usize {
        let payload = match self {
            Self::Words { .. } => 40 * 1024,
            Self::Language(_) => 5 * 1024 * 1024,
            Self::Resolved { .. } => 576 * 1024,
            Self::Documentation { .. } => 128 * 1024,
            Self::Acceptance { .. } => 144 * 1024,
        };
        // At most 64 bounded source identities, one cwd and 64 caret records.
        payload + 1024 * 1024
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionDelivery {
    pub query: Arc<CompletionQuery>,
    pub scope: DeliveryScope,
    pub payload: CompletionPayload,
}
