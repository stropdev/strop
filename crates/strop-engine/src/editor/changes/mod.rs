//! Shared change plans (0043, 0094): every multi-edit producer — LSP format,
//! rename, code actions, project replace — funnels through one plan with
//! revision-checked application per document. No producer mutates buffers
//! directly. A plan for one buffer applies at once; a plan for several is
//! reviewed (`review.rs`), applied and written with one key, and reverted
//! with `u`.
//!
//! Targets are documents open on the requesting server (the LSP binding
//! table is the inverse map). A target the editor has not open is a named
//! refusal, never a silent drop and never an implicit local-path guess.

use strop_core::id::{BufferRevision, DocumentId};
use strop_lsp::{PositionEncoding, ServerEdit};
use strop_workspace::ResourceLocation;

use super::Editor;

/// Who produced the plan — provenance for the receipt and the message.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ChangeProducer {
    Format,
    Rename,
    /// `workspace/willRenameFiles` preparation edits (0054 §12).
    FileRename,
    CodeAction,
    /// A global replace accepted from the picker review (0051 §6 R04).
    Replace,
}

impl ChangeProducer {
    fn label(&self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::Rename => "rename",
            Self::FileRename => "file rename",
            Self::CodeAction => "code action",
            Self::Replace => "replace",
        }
    }
}
/// One target document's prepared edits: byte ranges resolved against the
/// base revision, ready for the transaction gateway.
#[derive(Debug)]
pub(crate) struct PlannedDocument {
    pub location: ResourceLocation,
    pub document: DocumentId,
    pub base: BufferRevision,
    pub edits: Vec<strop_core::Replacement>,
}

/// A validated plan ready to apply. Building resolves server positions to
/// byte ranges; nothing here touches a buffer mutably.
#[derive(Debug)]
pub(crate) struct ChangePlan {
    pub producer: ChangeProducer,
    pub documents: Vec<PlannedDocument>,
    /// Targets that cannot be applied, each named with a reason.
    pub refused: Vec<(ResourceLocation, String)>,
}

pub(crate) struct ChangeState {
    /// Code actions offered by the open picker (0043); acceptance applies
    /// the chosen action's edits as a change plan.
    pub(crate) pending_actions: Vec<strop_lsp::ProtoAction>,
    /// The requesting server's negotiated encoding for pending actions.
    pub(crate) pending_encoding: strop_lsp::PositionEncoding,
    /// Groups of files being written together (`write.rs`).
    pub(crate) writes: Vec<write::BatchWrite>,
    pub(crate) write_seq: u64,
    /// The quickfix list: the newest collection (0094 §3.1).
    pub(crate) quickfix: Option<DocumentId>,
}

impl Default for ChangeState {
    /// The spec default encoding until a server negotiates otherwise.
    fn default() -> Self {
        Self {
            pending_actions: Vec::new(),
            pending_encoding: strop_lsp::PositionEncoding::Utf16,
            writes: Vec::new(),
            write_seq: 0,
            quickfix: None,
        }
    }
}

pub(crate) mod review;
pub(crate) mod write;

#[cfg(test)]
mod tests;

impl Editor {
    /// A chosen picker action applies through the same plan path as
    /// rename/format; command-only or file-operation actions are named
    /// inapplicable, never silently skipped.
    pub(crate) fn accept_code_action(&mut self, index: usize) {
        let Some(action) = self.changes.pending_actions.get(index) else {
            self.message = "stale code action — re-request".into();
            return;
        };
        let Some(edits) = action.edits.clone() else {
            self.message = if action.has_external_command {
                "action runs an external command — not applicable in strop".into()
            } else {
                "action needs file operations strop does not apply yet".into()
            };
            return;
        };
        if edits.is_empty() {
            self.message = "action has no edits".into();
            return;
        }
        let encoding = self.changes.pending_encoding;
        let plan = self.build_change_plan(ChangeProducer::CodeAction, edits, encoding);
        self.present_change_plan(plan);
    }
}

impl Editor {
    /// The document open on a server for one resource — the LSP binding
    /// table is the exact inverse map, for local and remote alike.
    fn bound_document(&self, location: &ResourceLocation) -> Option<DocumentId> {
        self.lsp_state
            .bindings
            .iter()
            .find(|(_, binding)| {
                binding.path == location.path && binding.target == location.filesystem
            })
            .map(|(document, _)| *document)
    }

    /// Resolve one server edit to a byte range against the buffer's
    /// CURRENT text (which must still equal the base the server saw —
    /// the gateway re-checks the revision at apply).
    fn resolve_edit(
        &self,
        document: DocumentId,
        edit: &ServerEdit,
        encoding: PositionEncoding,
    ) -> Option<strop_core::Replacement> {
        let buf = &self.docs.get(document)?.buf;
        let start = strop_lsp::checked_byte_offset(buf.text(), edit.start, encoding).ok()?;
        let end = strop_lsp::checked_byte_offset(buf.text(), edit.end, encoding).ok()?;
        if start > end {
            return None;
        }
        Some(strop_core::Replacement::new(
            strop_core::Range::charwise(start, end),
            edit.new_text.clone(),
        ))
    }

    /// Build a plan from server workspace edits. Pure preparation: no
    /// buffer is mutated; unbound or unresolvable targets are named.
    pub(crate) fn build_change_plan(
        &self,
        producer: ChangeProducer,
        edits: Vec<(ResourceLocation, Vec<ServerEdit>)>,
        encoding: PositionEncoding,
    ) -> ChangePlan {
        let mut documents = Vec::new();
        let mut refused = Vec::new();
        for (location, server_edits) in edits {
            let total = server_edits.len();
            let Some(document) = self.bound_document(&location) else {
                refused.push((
                    location,
                    format!("{total} edit(s) target a document not open on the server"),
                ));
                continue;
            };
            let Some(doc) = self.docs.get(document) else {
                refused.push((location, "the bound document is closed".into()));
                continue;
            };
            let base = doc.buf.revision();
            let mut resolved = Vec::with_capacity(total);
            let mut invalid = 0;
            for edit in &server_edits {
                match self.resolve_edit(document, edit, encoding) {
                    Some(replacement) => resolved.push(replacement),
                    None => invalid += 1,
                }
            }
            if invalid > 0 {
                refused.push((
                    location,
                    format!("{invalid} of {total} edit(s) resolve outside the document"),
                ));
                continue;
            }
            if resolved.is_empty() {
                continue; // a no-op target needs no plan entry
            }
            resolved.sort_by_key(|replacement| replacement.range.start.get());
            documents.push(PlannedDocument {
                location,
                document,
                base,
                edits: resolved,
            });
        }
        ChangePlan {
            producer,
            documents,
            refused,
        }
    }

    /// Apply a plan document by document through the revision-checked
    /// gateway. A document edited since the plan's base is refused by
    /// name; earlier successes stand and are reported. A rename flashes the
    /// span it changed in the buffer in view (a format on every save would
    /// only flicker).
    pub(crate) fn apply_change_plan(&mut self, plan: ChangePlan) {
        let producer = plan.producer.label();
        let flashes = matches!(plan.producer, ChangeProducer::Rename);
        let mut refused = plan.refused.len();
        let mut applied = 0;
        for target in plan.documents {
            let span = target
                .edits
                .first()
                .zip(target.edits.last())
                .map(|(first, last)| (first.range.start.get(), last.range.end.get()));
            let delta = target.edits.iter().fold(0isize, |delta, edit| {
                delta + edit.text.len() as isize - edit.range.len() as isize
            });
            let changes = super::transact::ChangeSet {
                edits: target.edits,
                undo_open: false,
            };
            match self.apply(target.document, target.base, changes) {
                Ok(_) => {
                    applied += 1;
                    if let Some((start, end)) =
                        span.filter(|_| flashes && target.document == self.current())
                    {
                        let end = end.saturating_add_signed(delta).min(self.buf().len_bytes());
                        self.flash(strop_core::Range::charwise(start.min(end), end));
                    }
                }
                Err(_) => refused += 1,
            }
        }
        self.message = match refused {
            0 => format!("{producer}: applied to {applied} buffer(s)"),
            refused => {
                format!("{producer}: {applied} buffer(s) applied, {refused} target(s) refused")
            }
        };
    }
}
