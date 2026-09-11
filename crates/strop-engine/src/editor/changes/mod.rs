//! Shared change plans (0043): every multi-edit producer — LSP format,
//! rename, code actions — funnels through one plan with revision-checked
//! application per document and an explicit receipt. No producer mutates
//! buffers directly; the receipt anchors grouped undo (`:undo-change`).
//!
//! v1 scope: targets are documents open on the requesting server (the
//! LSP binding table is the inverse map). A target the editor has not
//! open is a named refusal in the receipt, never a silent drop and never
//! an implicit local-path guess.

use std::collections::VecDeque;

use strop_core::id::{BufferRevision, DocumentId};
use strop_lsp::{PositionEncoding, ServerEdit, ServerPosition};
use strop_workspace::ResourceLocation;

use super::Editor;

/// Who produced the plan — provenance for the receipt and the message.
/// Who produced the plan — provenance for the receipt and the message.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ChangeProducer {
    Format,
    Rename,
    CodeAction,
    /// An edit written back from an editable code collection (0044).
    CollectionEdit,
}

impl ChangeProducer {
    fn label(&self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::Rename => "rename",
            Self::CodeAction => "code action",
            Self::CollectionEdit => "collection edit",
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

/// What actually happened — per-document outcomes, before/after revisions,
/// and named refusals. This, not a success flag, is the apply result.
#[derive(Debug)]
pub(crate) struct ChangeReceipt {
    pub producer: String,
    /// (document, revision before, revision after) per applied target.
    pub applied: Vec<(DocumentId, BufferRevision, BufferRevision)>,
    /// (target, reason) for everything NOT applied.
    pub refused: Vec<(ResourceLocation, String)>,
    /// Per-member history depth after this group was undone (0049 §5):
    /// revisions are monotonic and cannot name an undone position; the
    /// depth can. Set by the undoer, preflighted by the redoer.
    pub redo_depths: Option<Vec<usize>>,
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
    /// Newest-last; grouped undo pops from the back.
    receipts: VecDeque<ChangeReceipt>,
    /// Groups undone since the last apply — `collection redo` replays
    /// them newest-first. A new receipt clears the redo branch, like
    /// any history fork.
    undone: VecDeque<ChangeReceipt>,
    /// Code actions offered by the open picker (0043); acceptance applies
    /// the chosen action's edits as a change plan.
    pub(crate) pending_actions: Vec<strop_lsp::ProtoAction>,
    /// The requesting server's negotiated encoding for pending actions.
    pub(crate) pending_encoding: strop_lsp::PositionEncoding,
}

impl Default for ChangeState {
    /// The spec default encoding until a server negotiates otherwise.
    fn default() -> Self {
        Self {
            receipts: VecDeque::new(),
            undone: VecDeque::new(),
            pending_actions: Vec::new(),
            pending_encoding: strop_lsp::PositionEncoding::Utf16,
        }
    }
}

impl ChangeState {
    const RETAINED: usize = 16;

    fn record(&mut self, receipt: ChangeReceipt) {
        self.undone.clear(); // a new change forks history — redo dies
        self.receipts.push_back(receipt);
        while self.receipts.len() > Self::RETAINED {
            self.receipts.pop_front();
        }
    }

    /// Take the newest receipt matching `pred` (0049 §5: a collection's
    /// undo scopes to its own edit groups, never the global tail).
    pub(crate) fn take_newest_matching(
        &mut self,
        pred: impl Fn(&ChangeReceipt) -> bool,
    ) -> Option<(usize, ChangeReceipt)> {
        let index = self.receipts.iter().rposition(pred)?;
        self.receipts.remove(index).map(|receipt| (index, receipt))
    }

    /// Return a refused receipt to its exact place (preflight failure
    /// must not consume it, 0049 §5).
    pub(crate) fn restore(&mut self, index: usize, receipt: ChangeReceipt) {
        self.receipts
            .insert(index.min(self.receipts.len()), receipt);
    }

    /// Redo stack access for the scoped redo.
    pub(crate) fn take_undone_matching(
        &mut self,
        pred: impl Fn(&ChangeReceipt) -> bool,
    ) -> Option<ChangeReceipt> {
        let index = self.undone.iter().rposition(pred)?;
        self.undone.remove(index)
    }
    pub(crate) fn push_undone(&mut self, receipt: ChangeReceipt) {
        self.undone.push_back(receipt);
    }
    pub(crate) fn push_receipt_back(&mut self, receipt: ChangeReceipt) {
        self.receipts.push_back(receipt);
    }
}

pub(crate) mod review;

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
        let offset = |position: &ServerPosition| {
            let last = buf.len_lines().saturating_sub(1);
            let line = position.line.get().min(last);
            let line_start = buf.line_start(line);
            let text = buf.line_text(strop_core::id::LineIndex::new(line));
            let col = strop_lsp::to_byte_col(&text, position.column, encoding);
            line_start + col.get()
        };
        let (start, end) = (offset(&edit.start), offset(&edit.end));
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
    /// name; earlier successes stand and are reported. Each apply is one
    /// bounded unit through the gateway — never an unbounded loop.
    pub(crate) fn apply_change_plan(&mut self, plan: ChangePlan) {
        let producer = plan.producer.label().to_string();
        let mut receipt = ChangeReceipt {
            producer: producer.clone(),
            applied: Vec::new(),
            refused: plan.refused,
            redo_depths: None,
        };
        for target in plan.documents {
            let changes = super::transact::ChangeSet {
                edits: target.edits,
                undo_open: true,
            };
            match self.apply(target.document, target.base, changes) {
                Ok(committed) => {
                    receipt
                        .applied
                        .push((target.document, target.base, committed.revision))
                }
                Err(error) => receipt
                    .refused
                    .push((target.location, format!("changed since plan: {error}"))),
            }
        }
        self.message = match (receipt.applied.len(), receipt.refused.len()) {
            (applied, 0) => format!("{producer}: applied to {applied} buffer(s)"),
            (applied, refused) => {
                format!("{producer}: {applied} buffer(s) applied, {refused} target(s) refused")
            }
        };
        self.changes.record(receipt);
    }

    /// `:undo-change` — grouped undo of the most recent receipt. Each
    /// buffer must still sit at the receipt's after-revision: intervening
    /// edits refuse that buffer by name, never a blind history walk.
    pub(crate) fn undo_last_change(&mut self) {
        let Some(receipt) = self.changes.receipts.pop_back() else {
            self.message = "no change to undo".into();
            return;
        };
        let mut undone = 0;
        let mut skipped = 0;
        for (document, _before, after) in &receipt.applied {
            let Some(doc) = self.docs.get(*document) else {
                skipped += 1;
                continue;
            };
            if doc.buf.revision() != *after {
                skipped += 1;
                continue;
            }
            match self.doc_mut(*document).buf.undo() {
                Ok(Some(_)) => undone += 1,
                _ => skipped += 1,
            }
        }
        self.message = match skipped {
            0 => format!("undid {} across {undone} buffer(s)", receipt.producer),
            _ => format!(
                "undid {} in {undone} buffer(s); {skipped} skipped (edited or closed since)",
                receipt.producer
            ),
        };
    }
}
