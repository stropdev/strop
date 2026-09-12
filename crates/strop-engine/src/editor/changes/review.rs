//! Reviewable prepared changes (0049 §8): rename/code-action replies that
//! touch more than one document — or name refused targets — open a
//! real-buffer unified-diff review with explicit Apply/Cancel instead of
//! mutating immediately. Single-document, nothing-refused plans (the
//! `:format` case) keep applying directly with the existing receipt.
//!
//! A proposal is immutable once presented: identity, provenance, the
//! complete target inventory (planned documents with pinned base revisions
//! plus every refused target with its reason), and the rendered diff. Apply
//! re-checks each base revision and refuses moved targets BY NAME — it
//! never recomputes from newer text. After Apply/Cancel the review buffer
//! stays around as the receipt: outcomes per target, refusals with reasons.
//!
use strop_core::id::DocumentId;
use strop_core::Buffer;

use super::{ChangePlan, ChangeReceipt};
use crate::editor::transact::ChangeSet;
use crate::editor::Editor;
pub(crate) mod prepare;
mod render;
mod save;
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

/// The command names the review buffer advertises. Integration wires these
/// exact names to `review_apply_pub` / `review_cancel_pub`.
pub(crate) const APPLY_COMMAND: &str = ":apply-change";
pub(crate) const CANCEL_COMMAND: &str = ":cancel-change";

/// Unified-diff context lines around each changed span.
const CONTEXT: usize = 3;

/// Pending-proposal state. One proposal is reviewable at a time; a newer
/// proposal supersedes the older one's buffer with a visible note.
#[derive(Default)]
pub(crate) struct ReviewState {
    /// Monotone proposal identity; receipt headers name it.
    seq: usize,
    /// The proposal awaiting Apply/Cancel, if any.
    pending: Option<ChangeProposal>,
    pub(crate) preparing: Option<prepare::Preparation>,
    rows: std::collections::HashMap<DocumentId, Vec<ReviewRow>>,
    saves: std::collections::HashMap<DocumentId, save::PendingChangeSave>,
}

impl ReviewState {
    pub(crate) fn forget(&mut self, document: DocumentId) {
        self.rows.remove(&document);
        self.saves.retain(|_, pending| pending.report != document);
        if self
            .pending
            .as_ref()
            .is_some_and(|proposal| proposal.buffer == document)
        {
            self.pending = None;
        }
    }
}

/// An immutable prepared proposal (0049 §8): identity, provenance, the
/// complete intended target inventory with pinned base revisions, and the
/// review buffer presenting it. Nothing here recomputes at apply time.
#[derive(Debug)]
pub(crate) struct ChangeProposal {
    /// Proposal number, stable across the review buffer and its receipt.
    pub id: usize,
    /// The prepared plan: `documents` carry their base revisions, `refused`
    /// names every target that will not change, with its reason.
    pub plan: ChangePlan,
    /// The real (read-only) buffer showing the diff review; it becomes the
    /// receipt after Apply/Cancel.
    pub buffer: DocumentId,
    pub view_revision: strop_core::id::BufferRevision,
    pub search: Option<crate::editor::picker::search::SearchStamp>,
}

impl Editor {
    /// Present a prepared plan: apply single-document, nothing-refused
    /// plans directly (the existing receipt path), otherwise open the
    /// diff-review buffer and hold the proposal for Apply/Cancel.
    pub(crate) fn present_change_plan(&mut self, plan: ChangePlan) {
        if plan.documents.len() <= 1 && plan.refused.is_empty() {
            self.apply_change_plan(plan);
            return;
        }
        self.review_change_plan(plan);
    }

    /// Apply the reviewed proposal. Each target's base revision is checked
    /// first: a source edited since the proposal (or closed) is refused by
    /// name — never recomputed from newer text. Applied targets go through
    /// the revision-checked gateway exactly as prepared. The review buffer
    /// becomes the receipt and stays open; the receipt is recorded for
    /// grouped undo (`:undo-change`).
    pub(crate) fn review_apply_pub(&mut self) {
        let Some(proposal) = self.review.pending.take() else {
            self.message = "no change proposal awaiting review".into();
            return;
        };
        if self
            .docs
            .get(proposal.buffer)
            .is_none_or(|doc| doc.buf.revision() != proposal.view_revision)
        {
            self.review.pending = Some(proposal);
            self.message = "review changed; prepare a new proposal before applying".into();
            return;
        }
        if proposal
            .search
            .is_some_and(|stamp| self.search_stamp(stamp.session) != Some(stamp))
        {
            self.review.pending = Some(proposal);
            self.message = "Search changed; prepare a new review".into();
            return;
        }
        let producer = proposal.plan.producer.label().to_string();
        let mut receipt = ChangeReceipt {
            producer: producer.clone(),
            applied: Vec::new(),
            applied_positions: Vec::new(),
            refused: proposal.plan.refused.clone(),
            redo_positions: None,
        };
        let mut lines: Vec<String> = proposal
            .plan
            .refused
            .iter()
            .map(|(location, reason)| format!("refused: {} — {reason}", location.label()))
            .collect();
        for target in proposal.plan.documents {
            let label = target.location.label();
            let current = self.docs.get(target.document).map(|doc| doc.buf.revision());
            match current {
                None => {
                    let reason = "document closed since the proposal".to_string();
                    receipt.refused.push((target.location, reason.clone()));
                    lines.push(format!("refused: {label} — {reason}"));
                }
                Some(_)
                    if proposal.search.is_some()
                        && !self.doc(target.document).matches_target(
                            &crate::files::FileTarget::Local(target.location.path.clone()),
                        ) =>
                {
                    let reason = "source binding changed since the proposal".to_string();
                    receipt.refused.push((target.location, reason.clone()));
                    lines.push(format!("refused: {label} — {reason}"));
                }
                Some(revision) if revision != target.base => {
                    let reason = format!(
                        "edited since the proposal (base revision {}, now {revision}) — re-run the {producer}",
                        target.base
                    );
                    receipt.refused.push((target.location, reason.clone()));
                    lines.push(format!("refused: {label} — {reason}"));
                }
                Some(_) => {
                    let changes = ChangeSet {
                        edits: target.edits,
                        undo_open: false,
                    };
                    match self.apply(target.document, target.base, changes) {
                        Ok(committed) => {
                            receipt.applied.push((
                                target.document,
                                target.base,
                                committed.revision,
                            ));
                            if let Some(position) = self
                                .docs
                                .get(target.document)
                                .and_then(|doc| doc.buf.history().committed_position())
                            {
                                receipt.applied_positions.push(position);
                            }
                            lines.push(format!(
                                "applied: {label} (revision {} -> {})",
                                target.base, committed.revision
                            ));
                        }
                        Err(error) => {
                            let reason = format!("changed since plan: {error}");
                            receipt.refused.push((target.location, reason.clone()));
                            lines.push(format!("refused: {label} — {reason}"));
                        }
                    }
                }
            }
        }
        let status = if receipt.refused.is_empty() {
            "APPLIED"
        } else if receipt.applied.is_empty() {
            "REFUSED"
        } else {
            "PARTIAL"
        };
        let mut text = format!(
            "strop change proposal {}: {producer} — {status}\n{} buffer(s) applied, {} target(s) refused\n:undo-change reverts the applied group; :save-change saves changed files\n\n",
            proposal.id,
            receipt.applied.len(),
            receipt.refused.len()
        );
        for line in &lines {
            text.push_str(line);
            text.push('\n');
        }
        let publish = self.replace_system(proposal.buffer, &text);
        self.review.rows.insert(
            proposal.buffer,
            text.lines()
                .enumerate()
                .map(|(line, _)| {
                    if line == 0 && !receipt.refused.is_empty() {
                        ReviewRow::Warning
                    } else if line == 0 {
                        ReviewRow::Heading
                    } else {
                        ReviewRow::Context
                    }
                })
                .collect(),
        );
        if self.current() == proposal.buffer {
            self.set_head(0);
            self.view_mut().view_top = 0;
        }
        self.message = match (receipt.applied.len(), receipt.refused.len()) {
            (applied, 0) => format!("{producer}: applied to {applied} buffer(s)"),
            (applied, refused) => {
                format!("{producer}: {applied} buffer(s) applied, {refused} target(s) refused")
            }
        };
        if let Err(error) = publish {
            self.message = format!("change receipt publish failed: {error}");
        }
        self.changes.record(receipt);
    }

    /// Cancel the reviewed proposal: nothing is applied, ever. The review
    /// buffer becomes a cancelled receipt and stays open.
    pub(crate) fn review_cancel_pub(&mut self) {
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
        let Some(proposal) = self.review.pending.take() else {
            self.message = "no change proposal awaiting review".into();
            return;
        };
        let search = proposal.search;
        let producer = proposal.plan.producer.label();
        let mut text = format!(
            "strop change proposal {}: {producer} — CANCELLED\nnothing applied; {} file(s) had been proposed\n",
            proposal.id,
            proposal.plan.documents.len()
        );
        if !proposal.plan.refused.is_empty() {
            text.push_str("\nrefused targets (would have been skipped):\n");
            for (location, reason) in &proposal.plan.refused {
                text.push_str(&format!("  {} — {reason}\n", location.label()));
            }
        }
        let publish = self.replace_system(proposal.buffer, &text);
        self.review.rows.insert(
            proposal.buffer,
            text.lines()
                .enumerate()
                .map(|(line, _)| {
                    if line == 0 {
                        ReviewRow::Heading
                    } else {
                        ReviewRow::Context
                    }
                })
                .collect(),
        );
        if self.current() == proposal.buffer {
            self.set_head(0);
            self.view_mut().view_top = 0;
        }
        self.message = format!(
            "{producer}: proposal {} cancelled — nothing applied",
            proposal.id
        );
        if let Err(error) = publish {
            self.message = format!("change receipt publish failed: {error}");
        }
        if let Some(stamp) = search {
            self.resume_search_after_review(stamp);
        }
    }

    /// Render the review buffer: header, per-file unified diffs computed
    /// from each target's pinned base, and every refused target named with
    /// its reason.
    fn render_proposal(&self, id: usize, plan: &ChangePlan) -> ReviewBuffer {
        let mut view = Self::review_heading(id, plan);
        for target in &plan.documents {
            view.line("", ReviewRow::Context);
            let label = match target.location.filesystem {
                strop_workspace::Filesystem::Local => target
                    .location
                    .path
                    .strip_prefix(&self.cwd)
                    .unwrap_or(&target.location.path)
                    .display()
                    .to_string(),
                _ => target.location.label(),
            };
            match self.docs.get(target.document) {
                Some(document) => {
                    match render::file_diff(&label, &document.buf, target.base, &target.edits) {
                        Ok(diff) => view.append(diff),
                        Err(error) => {
                            view.line(&format!("refused: {label} — {error}"), ReviewRow::Warning)
                        }
                    }
                }
                None => view.line(
                    &format!("refused: {label} — document closed"),
                    ReviewRow::Warning,
                ),
            }
        }
        Self::review_refusals(&mut view, plan);
        view
    }
}
impl Editor {
    /// Present a prepared plan as a review ALWAYS (0051 §6 R04): a
    /// global replace never applies straight from the picker's text
    /// fields, even when the plan is a single file. Same review shape
    /// as `present_change_plan` — kept separate so that method's
    /// direct-apply fast path stays untouched for LSP producers.
    pub(crate) fn review_change_plan(&mut self, plan: ChangePlan) {
        let Some(id) = self.next_review_id() else {
            return;
        };
        let text = self.render_proposal(id, &plan);
        self.publish_review(id, plan, text, None, true);
    }

    fn next_review_id(&mut self) -> Option<usize> {
        match self.review.seq.checked_add(1) {
            Some(id) => Some(id),
            None => {
                self.message = "review identity exhausted".into();
                None
            }
        }
    }

    fn review_heading(id: usize, plan: &ChangePlan) -> ReviewBuffer {
        let mut view = ReviewBuffer::default();
        view.line(
            &format!("strop change proposal {id}: {}", plan.producer.label()),
            ReviewRow::Heading,
        );
        view.line(
            &format!(
                "{} file(s) to change, {} target(s) refused",
                plan.documents.len(),
                plan.refused.len()
            ),
            ReviewRow::Context,
        );
        view.line(
            &format!("{APPLY_COMMAND} applies exactly what is shown; {CANCEL_COMMAND} discards it"),
            ReviewRow::Context,
        );
        view.line(
            "bases are pinned — editing a source invalidates that file at apply",
            ReviewRow::Context,
        );
        view
    }

    fn review_refusals(view: &mut ReviewBuffer, plan: &ChangePlan) {
        if !plan.refused.is_empty() {
            view.line("", ReviewRow::Context);
            view.line("refused targets:", ReviewRow::Warning);
            for (location, reason) in &plan.refused {
                view.line(
                    &format!("  {} — {reason}", location.label()),
                    ReviewRow::Warning,
                );
            }
        }
    }

    fn present_prepared_search_review(
        &mut self,
        plan: ChangePlan,
        body: ReviewBuffer,
        stamp: crate::editor::picker::search::SearchStamp,
        focus: bool,
    ) {
        let Some(id) = self.next_review_id() else {
            return;
        };
        let mut text = Self::review_heading(id, &plan);
        text.append(body);
        Self::review_refusals(&mut text, &plan);
        self.publish_review(id, plan, text, Some(stamp), focus);
    }

    fn publish_review(
        &mut self,
        id: usize,
        plan: ChangePlan,
        text: ReviewBuffer,
        search: Option<crate::editor::picker::search::SearchStamp>,
        focus: bool,
    ) {
        if let Some(old) = self.review.pending.take() {
            let note = format!(
                "strop change proposal {}: {} — SUPERSEDED by a newer proposal\n",
                old.id,
                old.plan.producer.label()
            );
            if let Err(error) = self.replace_system(old.buffer, &note) {
                self.message = format!("could not retire proposal {}: {error}", old.id);
            }
            self.review
                .rows
                .insert(old.buffer, vec![ReviewRow::Heading]);
        }
        self.review.seq = id;
        let producer = plan.producer.label().to_owned();
        let files = plan.documents.len();
        let refused = plan.refused.len();
        let mut buf = Buffer::from_text(&text.text);
        buf.name = Some(format!("change proposal {id}"));
        let view_revision = buf.revision();
        let buffer = if focus {
            self.open_temporary_output(buf)
        } else {
            let mut document = crate::editor::Document::output(buf);
            if let Some(origin) = self
                .retained_search
                .as_ref()
                .and_then(|glue| glue.search.as_ref())
                .map(|context| context.origin.clone())
            {
                document.set_return_point(origin);
            }
            let id = self.docs.insert(document);
            self.mru.push(id);
            id
        };
        self.review.rows.insert(buffer, text.rows);
        self.review.pending = Some(ChangeProposal {
            id,
            plan,
            buffer,
            view_revision,
            search,
        });
        self.message = format!(
            "{producer}: proposal {id} reviews {files} file(s), {refused} refused — {}",
            if focus {
                ":apply-change or :cancel-change"
            } else {
                "ready in buffers; focus unchanged"
            }
        );
    }

    pub(crate) fn invalidate_search_review(&mut self, session: strop_core::worker::WorkerId) {
        if self
            .review
            .preparing
            .as_ref()
            .is_some_and(|pending| pending.ticket.key.stamp.session == session)
        {
            self.cancel_review_preparation();
        }
        if !self.review.pending.as_ref().is_some_and(|proposal| {
            proposal
                .search
                .is_some_and(|stamp| stamp.session == session)
        }) {
            return;
        }
        if let Some(proposal) = self.review.pending.take() {
            let note = format!("strop change proposal {} — STALE\nSearch changed; prepare a new review. Nothing applied.\n", proposal.id);
            if let Err(error) = self.replace_system(proposal.buffer, &note) {
                self.message = format!("could not retire stale review: {error}");
            }
            self.review.rows.insert(
                proposal.buffer,
                vec![ReviewRow::Warning, ReviewRow::Context],
            );
        }
    }
}

#[cfg(test)]
mod tests;

impl Editor {
    pub fn review_row(&self, document: DocumentId, row: usize) -> Option<ReviewRow> {
        self.review.rows.get(&document)?.get(row).copied()
    }
}
