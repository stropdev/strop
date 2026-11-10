//! The change review (0094 §3.2): a plan touching several files — a project
//! replace, a rename, a code action — opens one real buffer with the diff.
//! `Enter` applies every target and writes it; `u` reverts the buffers and
//! the files; closing a review that was never applied cancels it.
//!
//! A review is exact: identity, provenance, the complete target inventory
//! (planned documents with pinned base revisions plus every refused target
//! with its reason) and the rendered diff. Apply re-checks each base and
//! refuses moved targets by name; it never recomputes from newer text. The
//! same buffer keeps the diff after the apply, under a heading that says
//! what happened.
use strop_core::id::DocumentId;

use super::ChangePlan;
use crate::editor::transact::ChangeSet;
use crate::editor::Editor;
pub(crate) mod prepare;
mod present;
pub(crate) mod render;
use render::ReviewBuffer;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum ReviewRow {
    Heading,
    File,
    Hunk,
    Context,
    Removed,
    Added,
    Warning,
}

/// Unified-diff context lines around each changed span.
const CONTEXT: usize = 3;

/// The heading is two rows; everything below it is the diff.
const HEADING_ROWS: usize = 2;

/// Review state. One review is live at a time; a newer one supersedes the
/// older one's buffer with a visible note.
#[derive(Default)]
pub(crate) struct ReviewState {
    /// Monotone review identity.
    seq: usize,
    /// The live review, awaiting `Enter` or already applied.
    pending: Option<ChangeProposal>,
    pub(crate) preparing: Option<prepare::Preparation>,
    rows: std::collections::HashMap<DocumentId, Vec<ReviewRow>>,
}

impl ReviewState {
    pub(crate) fn forget(&mut self, document: DocumentId) -> Option<ChangeProposal> {
        self.rows.remove(&document);
        self.pending.take_if(|proposal| proposal.buffer == document)
    }

    /// `document` is the live review's buffer.
    pub(crate) fn live(&self, document: DocumentId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|proposal| proposal.buffer == document)
    }
}

/// One target the apply changed: `u` reverts it while its history still
/// sits where the apply left it.
#[derive(Debug)]
struct Applied {
    document: DocumentId,
    /// The source's history position right after the apply.
    position: usize,
}

/// A prepared review: the plan with pinned base revisions, the buffer
/// presenting it, and what the last `Enter` changed.
#[derive(Debug)]
pub(crate) struct ChangeProposal {
    /// `documents` carry their base revisions, `refused` names every target
    /// that will not change, with its reason.
    pub plan: ChangePlan,
    /// The real (read-only) buffer showing the diff.
    pub buffer: DocumentId,
    pub view_revision: strop_core::id::BufferRevision,
    pub search: Option<crate::editor::picker::search::SearchStamp>,
    /// Empty until `Enter`; emptied again by `u`.
    applied: Vec<Applied>,
    /// What the last `u` could not revert, by name.
    unreverted: Vec<String>,
}

impl Editor {
    /// Present a prepared plan: a plan for one buffer with nothing refused
    /// applies directly; anything wider is reviewed first.
    pub(crate) fn present_change_plan(&mut self, plan: ChangePlan) {
        if plan.documents.len() <= 1 && plan.refused.is_empty() {
            self.apply_change_plan(plan);
            return;
        }
        let Some(id) = self.next_review_id() else {
            return;
        };
        let text = self.render_proposal(&plan);
        self.publish_review(id, plan, text, None, true);
    }

    /// `Enter` in a review: apply every target and write it. Each base is
    /// checked first; a source edited since the review (or closed) is
    /// refused by name and left alone.
    pub(crate) fn review_apply_pub(&mut self) {
        if self.apply_filesystem_review() {
            return;
        }
        if self.is_filesystem_report(self.current()) {
            self.message = "this filesystem report has no pending proposal to apply".into();
            return;
        }
        let Some(mut proposal) = self.review.pending.take() else {
            self.message = "no review to apply".into();
            return;
        };
        let refusal = if !proposal.applied.is_empty() {
            Some("already applied — u reverts the buffers and the files")
        } else if proposal.plan.documents.is_empty() {
            Some("nothing left to apply in this review")
        } else if self
            .docs
            .get(proposal.buffer)
            .is_none_or(|doc| doc.buf.revision() != proposal.view_revision)
        {
            Some("review changed; prepare a new one before applying")
        } else if proposal
            .search
            .is_some_and(|stamp| self.search_stamp(stamp.session) != Some(stamp))
        {
            Some("Search changed; prepare a new review")
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.review.pending = Some(proposal);
            self.message = refusal.into();
            return;
        }
        let producer = proposal.plan.producer.label();
        let known = proposal.plan.refused.len();
        let mut kept = Vec::new();
        for target in std::mem::take(&mut proposal.plan.documents) {
            let reason = match self.docs.get(target.document) {
                None => Some("closed since the review".to_owned()),
                Some(doc)
                    if proposal.search.is_some()
                        && !doc.matches_target(&crate::files::FileTarget::Local(
                            target.location.path.clone(),
                        )) =>
                {
                    Some("source binding changed since the review".to_owned())
                }
                Some(doc) if doc.buf.revision() != target.base => {
                    Some(format!("edited since the review; run the {producer} again"))
                }
                Some(_) => {
                    let changes = ChangeSet {
                        edits: target.edits.clone(),
                        undo_open: false,
                    };
                    self.apply(target.document, target.base, changes)
                        .err()
                        .map(|error| error.to_string())
                }
            };
            match reason {
                Some(reason) => proposal.plan.refused.push((target.location, reason)),
                None => {
                    if let Some(position) = self
                        .docs
                        .get(target.document)
                        .and_then(|doc| doc.buf.history().committed_position())
                    {
                        proposal.applied.push(Applied {
                            document: target.document,
                            position,
                        });
                    }
                    kept.push(target);
                }
            }
        }
        proposal.plan.documents = kept;
        let buffer = proposal.buffer;
        let changed: Vec<DocumentId> = proposal
            .applied
            .iter()
            .map(|applied| applied.document)
            .collect();
        let count = changed.len();
        let refused = proposal.plan.refused.len();
        let heading = refused_suffix(format!("{producer}: changed {count} file(s)"), refused);
        let next = if count == 0 {
            "nothing was applied · q closes"
        } else {
            "writing the files…"
        };
        let fresh = proposal.plan.refused[known..].to_vec();
        self.review.pending = Some(proposal);
        self.review_name_refusals(buffer, &fresh, known > 0);
        self.review_heading(buffer, &heading, next, refused > 0);
        if count == 0 {
            self.message = heading;
            return;
        }
        self.write_documents(producer, changed, Some(buffer));
        if self.review_write_pending(buffer) {
            self.message = heading;
        }
    }

    /// The files of a review reached disk (or did not).
    pub(crate) fn review_written(&mut self, buffer: DocumentId, wrote: usize, failed: &[String]) {
        let Some(proposal) = self.review.pending.as_ref().filter(|p| p.buffer == buffer) else {
            return;
        };
        let producer = proposal.plan.producer.label();
        let refused = proposal.plan.refused.len();
        let reverted = proposal.applied.is_empty();
        let verb = if reverted { "reverted" } else { "changed" };
        let heading = refused_suffix(
            format!("{producer}: {verb} and wrote {wrote} file(s)"),
            refused,
        );
        let next = match (failed, reverted) {
            ([], true) if !proposal.unreverted.is_empty() => {
                format!("could not revert {}", proposal.unreverted.join(", "))
            }
            ([], true) => "Enter applies again · q closes".to_owned(),
            ([], false) => "u reverts the buffers and the files · q closes".to_owned(),
            (failed, _) => format!("could not write {}", failed.join(", ")),
        };
        let warning = refused > 0 || !failed.is_empty() || !proposal.unreverted.is_empty();
        self.review_heading(buffer, &heading, &next, warning);
    }

    /// `u` in a review: revert each buffer whose history still sits where
    /// the apply left it, write it again, and name every one it could not.
    pub(crate) fn review_revert(&mut self) {
        let Some(mut proposal) = self.review.pending.take() else {
            self.message = "no review here".into();
            return;
        };
        if proposal.applied.is_empty() {
            self.review.pending = Some(proposal);
            self.message = "nothing applied to revert — Enter applies and writes".into();
            return;
        }
        if self.review_write_pending(proposal.buffer) {
            self.review.pending = Some(proposal);
            self.message = "the files are still being written; u again when they land".into();
            return;
        }
        let producer = proposal.plan.producer.label();
        let mut reverted = Vec::new();
        let mut rewrite = Vec::new();
        let mut kept = Vec::new();
        for applied in std::mem::take(&mut proposal.applied) {
            let state = self.docs.get(applied.document).map(|doc| {
                (
                    doc.label(&self.cwd),
                    doc.buf.readonly,
                    doc.buf.history().committed_position() == Some(applied.position),
                )
            });
            let reason = match state {
                None => Some("a closed buffer".to_owned()),
                Some((label, true, _)) => Some(format!("{label} (read-only)")),
                Some((label, _, false)) => Some(format!("{label} (edited since)")),
                Some((label, false, true)) => {
                    let undone = self.doc_mut(applied.document).buf.undo();
                    match undone {
                        Ok(Some(_)) => None,
                        Ok(None) => Some(format!("{label} (nothing to undo)")),
                        Err(error) => Some(format!("{label} ({error})")),
                    }
                }
            };
            match reason {
                Some(reason) => kept.push(reason),
                None => {
                    reverted.push(applied.document);
                    if self.doc(applied.document).buf.is_dirty() {
                        rewrite.push(applied.document);
                    }
                }
            }
        }
        // The reverted text is the reviewed text: the same edits apply
        // again from the new revisions. Targets that stayed changed leave
        // the plan.
        proposal
            .plan
            .documents
            .retain(|target| reverted.contains(&target.document));
        for target in &mut proposal.plan.documents {
            target.base = self.doc(target.document).buf.revision();
        }
        let buffer = proposal.buffer;
        let count = reverted.len();
        let refused = proposal.plan.refused.len();
        proposal.unreverted = kept.clone();
        self.review.pending = Some(proposal);
        let heading = refused_suffix(format!("{producer}: reverted {count} file(s)"), refused);
        let next = if kept.is_empty() {
            "Enter applies again · q closes".to_owned()
        } else {
            format!("could not revert {}", kept.join(", "))
        };
        self.review_heading(buffer, &heading, &next, !kept.is_empty());
        let message = if kept.is_empty() {
            heading
        } else {
            format!("{heading}; {next}")
        };
        let wrote = !rewrite.is_empty();
        if wrote {
            self.write_documents(producer, rewrite, Some(buffer));
        }
        // The summary stands unless the files already landed with nothing
        // left to name.
        if !kept.is_empty() || !wrote || self.review_write_pending(buffer) {
            self.message = message;
        }
    }

    /// Cancel the review in view: nothing is applied. A review that was
    /// applied is not cancelled by closing it; `u` reverts it.
    pub(crate) fn review_cancel_pub(&mut self) {
        if self.cancel_filesystem_review() {
            return;
        }
        if self.is_filesystem_report(self.current()) {
            self.message = "this filesystem report has no pending proposal to cancel".into();
            return;
        }
        if let Some(stamp) = self
            .review
            .preparing
            .as_ref()
            .map(|pending| pending.ticket.key.stamp)
        {
            self.cancel_review_preparation();
            self.resume_search_after_review(stamp);
            self.message = "replacement review cancelled; nothing applied".into();
            return;
        }
        let Some(proposal) = self
            .review
            .pending
            .take_if(|proposal| proposal.applied.is_empty())
        else {
            self.message = "no review to cancel".into();
            return;
        };
        if proposal.buffer == self.current() {
            // Closing the buffer is the cancel (`review_closed`).
            self.review.pending = Some(proposal);
            self.close_buffer(true);
            return;
        }
        self.retire_review(&proposal, "cancelled");
        self.review_closed(proposal);
    }

    /// The review's buffer closed. One that was never applied is cancelled:
    /// Search comes back where it was.
    pub(crate) fn review_closed(&mut self, proposal: ChangeProposal) {
        if !proposal.applied.is_empty() || proposal.plan.documents.is_empty() {
            return;
        }
        self.message = format!(
            "{}: review cancelled — nothing applied",
            proposal.plan.producer.label()
        );
        if let Some(stamp) = proposal.search {
            self.resume_search_after_review(stamp);
        }
    }

    /// What `Space a` offers on the review in view, first row first.
    pub(crate) fn review_actions(&self) -> Vec<crate::editor::actions::ReviewAction> {
        use crate::editor::actions::ReviewAction;
        let current = self.current();
        if self.filesystem.reviews(current) {
            return vec![ReviewAction::Apply, ReviewAction::Cancel];
        }
        match self.review.pending.as_ref().filter(|p| p.buffer == current) {
            Some(proposal) if !proposal.applied.is_empty() => vec![ReviewAction::Revert],
            Some(proposal) if !proposal.plan.documents.is_empty() => {
                vec![ReviewAction::Apply, ReviewAction::Cancel]
            }
            _ => Vec::new(),
        }
    }

    /// Keys a review buffer owns: `Enter` applies, `u` reverts.
    pub(crate) fn review_key(&mut self, key: strop_core::frontend_input::Key) -> bool {
        use strop_core::frontend_input::Key;
        let current = self.current();
        let filesystem = self.filesystem.reviews(current);
        match key {
            Key::Enter if filesystem || self.review.live(current) => self.review_apply_pub(),
            Key::Char('u') if self.review.live(current) => self.review_revert(),
            _ => return false,
        }
        true
    }

    pub(crate) fn set_filesystem_review_rows(
        &mut self,
        document: DocumentId,
        rows: Vec<ReviewRow>,
    ) {
        self.filesystem_report_opened(document);
        self.review.rows.insert(document, rows);
    }

    pub(crate) fn retire_text_review_for_filesystem(&mut self) {
        if let Some(proposal) = self.review.pending.take() {
            self.retire_review(&proposal, "superseded by a filesystem review");
        }
    }

    pub(crate) fn invalidate_filesystem_text_review(&mut self, documents: &[DocumentId]) {
        self.cancel_review_preparation();
        if self.review.pending.as_ref().is_some_and(|proposal| {
            proposal
                .plan
                .documents
                .iter()
                .any(|target| documents.contains(&target.document))
        }) {
            self.retire_text_review_for_filesystem();
        }
    }
    pub fn review_row(&self, document: DocumentId, row: usize) -> Option<ReviewRow> {
        self.review.rows.get(&document)?.get(row).copied()
    }
}

/// `heading`, with the refused count when there is one.
fn refused_suffix(heading: String, refused: usize) -> String {
    match refused {
        0 => heading,
        refused => format!("{heading}, {refused} refused"),
    }
}

#[cfg(test)]
mod tests;
