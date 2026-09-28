use super::super::acceptance;
use super::super::mailbox::CompletionMailbox;
use super::super::model::{
    CandidateKey, CompletionDelivery, CompletionPayload, CompletionQuery, DeliveryScope,
    LanguageSuggestions, PreparedDocumentation,
};
use ropey::Rope;
use std::sync::Arc;
use strop_core::id::BufferRevision;
use strop_core::worker::{CancelReason, FailureKind, Outcome, WorkerId};
use strop_lsp::completion::{CompletionItem, Documentation, MarkupKind};

const MAX_DOCUMENTATION_BYTES: usize = 8 * 1024;
const MAX_DOCUMENTATION_SPANS: usize = 4096;

pub(in crate::editor::completion) struct Preparation {
    pub query: Arc<CompletionQuery>,
    pub candidate: CandidateKey,
    pub selection: WorkerId,
    pub work: WorkerId,
    pub operation: PreparationOperation,
}
pub(in crate::editor::completion) enum PreparationOperation {
    Documentation {
        original: Arc<CompletionItem>,
        resolved: Option<Arc<CompletionItem>>,
    },
    Word {
        text: Arc<str>,
        recorded_chars: usize,
    },
    Language {
        source: Rope,
        original: Arc<CompletionItem>,
        resolved: Option<Arc<CompletionItem>>,
        encoding: strop_lsp::PositionEncoding,
        resolve_required: bool,
        recorded_chars: usize,
    },
}
impl Preparation {
    pub fn scope(&self) -> DeliveryScope {
        DeliveryScope::Preparation {
            selection: self.selection,
            work: self.work,
        }
    }
    pub fn charged_bytes(&self) -> usize {
        let source = match &self.operation {
            PreparationOperation::Language { source, .. } => source.len_bytes(),
            _ => 0,
        };
        // Two bounded opaque items plus bounded query ownership metadata.
        source + 3 * 1024 * 1024
    }
}

#[derive(Default)]
pub(super) struct DocumentationWorker {
    markdown: Option<strop_syntax::Highlighter>,
}
impl DocumentationWorker {
    pub fn prepare(&mut self, work: Preparation, mailbox: &CompletionMailbox) {
        let scope = work.scope();
        let cancelled = || !mailbox.owns(work.query.request, scope);
        if cancelled() {
            return;
        }
        match &work.operation {
            PreparationOperation::Documentation { original, resolved } => {
                let outcome =
                    self.documentation(original, resolved.as_deref(), work.work, cancelled);
                mailbox.publish(CompletionDelivery {
                    query: work.query.clone(),
                    scope,
                    payload: CompletionPayload::Documentation {
                        candidate: work.candidate.clone(),
                        outcome,
                    },
                });
            }
            PreparationOperation::Word {
                text,
                recorded_chars,
            } => {
                mailbox.prepared(
                    work.query.clone(),
                    scope,
                    work.candidate.clone(),
                    acceptance::word(&work.query.context, text, *recorded_chars),
                );
            }
            PreparationOperation::Language {
                source,
                original,
                resolved,
                encoding,
                resolve_required,
                recorded_chars,
            } => {
                mailbox.prepared(
                    work.query.clone(),
                    scope,
                    work.candidate.clone(),
                    acceptance::language(
                        &work.query.context,
                        source,
                        *encoding,
                        original,
                        resolved.as_deref(),
                        *resolve_required,
                        *recorded_chars,
                    ),
                );
            }
        }
    }

    fn documentation(
        &mut self,
        original: &CompletionItem,
        resolved: Option<&CompletionItem>,
        work: WorkerId,
        cancelled: impl Fn() -> bool,
    ) -> Outcome<PreparedDocumentation> {
        let documentation = resolved
            .and_then(|item| item.protocol().documentation.as_ref())
            .or(original.protocol().documentation.as_ref());
        let (source, markdown) = match documentation {
            Some(Documentation::String(text)) => (text.as_str(), false),
            Some(Documentation::MarkupContent(content)) => {
                (content.value.as_str(), content.kind == MarkupKind::Markdown)
            }
            None => (
                resolved
                    .and_then(|item| item.protocol().detail.as_deref())
                    .or(original.protocol().detail.as_deref())
                    .unwrap_or(""),
                false,
            ),
        };
        let mut end = source.len().min(MAX_DOCUMENTATION_BYTES);
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        let mut truncated = end < source.len();
        let mut spans = if markdown && end > 0 {
            let rope = Rope::from_str(&source[..end]);
            if self.markdown.is_none() {
                self.markdown = strop_syntax::Highlighter::for_path(
                    std::path::Path::new("completion.md"),
                    &rope,
                );
            }
            let Some(highlighter) = &mut self.markdown else {
                return Outcome::failed(
                    FailureKind::Unavailable,
                    "Markdown documentation grammar is unavailable",
                );
            };
            match highlighter.highlight_while(
                &rope,
                BufferRevision::new(work.get()),
                0,
                end,
                &cancelled,
            ) {
                Ok(spans) => spans,
                Err(strop_syntax::HighlightError::Cancelled) => {
                    return Outcome::Cancelled(CancelReason::Superseded)
                }
                Err(error) => {
                    return Outcome::failed(
                        FailureKind::Protocol,
                        format!("documentation highlighting failed: {error}"),
                    )
                }
            }
        } else {
            Vec::new()
        };
        if cancelled() {
            return Outcome::Cancelled(CancelReason::Superseded);
        }
        if spans.len() > MAX_DOCUMENTATION_SPANS {
            end = end.min(spans[MAX_DOCUMENTATION_SPANS - 1].end);
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            spans.truncate(MAX_DOCUMENTATION_SPANS);
            spans.retain(|span| span.end <= end);
            truncated = true;
        }
        Outcome::Success(PreparedDocumentation {
            text: Arc::from(&source[..end]),
            spans,
            truncated,
        })
    }
}

/// The caller proves that only the captured keyword prefix grew. Eligibility
/// additionally excludes explicit ranges, lazy edits and truncated universes.
pub(super) fn refilter_cached(
    query: Arc<CompletionQuery>,
    previous: &LanguageSuggestions,
    source: &Rope,
    mailbox: &CompletionMailbox,
) -> bool {
    let Some(target) = query.context.primary() else {
        return false;
    };
    if !previous.reusable() || !mailbox.owns(query.request, DeliveryScope::Query) {
        return false;
    }
    let Some(prefix) = source.get_byte_slice(target.start..target.caret) else {
        return false;
    };
    let Some(suffix) = source.get_byte_slice(target.caret..target.end) else {
        return false;
    };
    let prefix = prefix.to_string();
    if !prefix.starts_with(previous.prefix.as_ref()) || suffix != previous.suffix.as_ref() {
        return false;
    }
    let items = previous
        .items
        .items
        .iter()
        .filter(|entry| entry.item.matches_prefix(prefix.chars()))
        .cloned()
        .collect();
    let suggestions = LanguageSuggestions {
        origin: previous.origin,
        owner: previous.owner.clone(),
        items: strop_lsp::completion::CompletionList {
            items,
            server_incomplete: false,
            client_truncated: false,
            omitted_oversized: 0,
        },
        resolve_required: false,
        prefix: Arc::from(prefix),
        suffix: previous.suffix.clone(),
    };
    mailbox.publish(CompletionDelivery {
        query,
        scope: DeliveryScope::Query,
        payload: CompletionPayload::Language(Outcome::Success(suggestions)),
    })
}
