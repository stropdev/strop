//! What a review looks like: the two heading rows, the diff under them,
//! the refused targets named at its end, and the buffer that holds them.
use strop_core::id::DocumentId;
use strop_core::Buffer;

use super::render::{self, ReviewBuffer};
use super::{refused_suffix, ChangeProposal, ReviewRow, HEADING_ROWS};
use crate::editor::changes::ChangePlan;
use crate::editor::Editor;

impl Editor {
    /// Targets refused at apply join the review's list, by name.
    pub(super) fn review_name_refusals(
        &mut self,
        buffer: DocumentId,
        fresh: &[(strop_workspace::ResourceLocation, String)],
        listed: bool,
    ) {
        if fresh.is_empty() || self.docs.get(buffer).is_none() {
            return;
        }
        let mut view = ReviewBuffer::default();
        if !listed {
            view.line("", ReviewRow::Context);
            view.line("refused targets:", ReviewRow::Warning);
        }
        for (location, reason) in fresh {
            view.line(
                &format!("  {} — {reason}", location.label()),
                ReviewRow::Warning,
            );
        }
        let end = self.doc(buffer).buf.len_bytes();
        let appended = self
            .doc_mut(buffer)
            .buf
            .system_edit()
            .replace(strop_core::Range::charwise(end, end), &view.text);
        if let Err(error) = appended {
            self.message = format!("review update failed: {error}");
            return;
        }
        self.review
            .rows
            .entry(buffer)
            .or_default()
            .extend(view.rows);
    }

    /// Rewrite the two heading rows in place; the diff below stays.
    pub(super) fn review_heading(
        &mut self,
        buffer: DocumentId,
        heading: &str,
        next: &str,
        warning: bool,
    ) {
        let Some(doc) = self.docs.get(buffer) else {
            return;
        };
        let end = doc
            .buf
            .line_start(HEADING_ROWS.min(doc.buf.last_content_line() + 1));
        let text = format!("{heading}\n{next}\n");
        let published = self
            .doc_mut(buffer)
            .buf
            .system_edit()
            .replace(strop_core::Range::charwise(0, end), &text);
        if let Err(error) = published {
            self.message = format!("review heading update failed: {error}");
            return;
        }
        if let Some(rows) = self.review.rows.get_mut(&buffer) {
            if let Some(first) = rows.first_mut() {
                *first = if warning {
                    ReviewRow::Warning
                } else {
                    ReviewRow::Heading
                };
            }
        }
        let revision = self.doc(buffer).buf.revision();
        if let Some(proposal) = self.review.pending.as_mut().filter(|p| p.buffer == buffer) {
            proposal.view_revision = revision;
        }
    }

    /// Render the review buffer: per-file unified diffs computed from each
    /// target's pinned base, and every refused target named with its reason.
    pub(super) fn render_proposal(&self, plan: &ChangePlan) -> ReviewBuffer {
        let mut view = Self::review_head(plan);
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

    pub(super) fn next_review_id(&mut self) -> Option<usize> {
        match self.review.seq.checked_add(1) {
            Some(id) => Some(id),
            None => {
                self.message = "review identity exhausted".into();
                None
            }
        }
    }

    pub(super) fn review_head(plan: &ChangePlan) -> ReviewBuffer {
        let mut view = ReviewBuffer::default();
        view.line(
            &refused_suffix(
                format!(
                    "{}: {} file(s) to change",
                    plan.producer.label(),
                    plan.documents.len()
                ),
                plan.refused.len(),
            ),
            ReviewRow::Heading,
        );
        view.line(
            "Enter applies and writes the files · q cancels",
            ReviewRow::Context,
        );
        view
    }

    pub(super) fn review_refusals(view: &mut ReviewBuffer, plan: &ChangePlan) {
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

    pub(super) fn present_prepared_search_review(
        &mut self,
        plan: ChangePlan,
        body: ReviewBuffer,
        stamp: crate::editor::picker::search::SearchStamp,
        focus: bool,
    ) {
        let Some(id) = self.next_review_id() else {
            return;
        };
        let mut text = Self::review_head(&plan);
        text.append(body);
        Self::review_refusals(&mut text, &plan);
        self.publish_review(id, plan, text, Some(stamp), focus);
    }

    pub(super) fn publish_review(
        &mut self,
        id: usize,
        plan: ChangePlan,
        text: ReviewBuffer,
        search: Option<crate::editor::picker::search::SearchStamp>,
        focus: bool,
    ) {
        // Preflight (0056 AR13): refuse before the old review is retired,
        // so exhaustion never leaves a half-published one.
        if self.docs.insert_capacity() == 0 {
            self.message = "document identity space exhausted".into();
            return;
        }
        self.retire_filesystem_review("superseded by a text review");
        if let Some(old) = self.review.pending.take() {
            self.retire_review(&old, "superseded by a newer review");
        }
        self.review.seq = id;
        let producer = plan.producer.label();
        let files = plan.documents.len();
        let refused = plan.refused.len();
        let mut buf = Buffer::from_text(&text.text);
        buf.name = Some(format!("review: {producer}"));
        let view_revision = buf.revision();
        let Some(buffer) = (if focus {
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
            match self.docs.try_insert(document) {
                Ok(id) => {
                    self.mru.push(id);
                    Some(id)
                }
                Err(_) => {
                    self.message = "document identity space exhausted".into();
                    None
                }
            }
        }) else {
            return;
        };
        self.review.rows.insert(buffer, text.rows);
        self.review.pending = Some(ChangeProposal {
            plan,
            buffer,
            view_revision,
            search,
            applied: Vec::new(),
            unreverted: Vec::new(),
        });
        self.message = refused_suffix(format!("{producer}: review of {files} file(s)"), refused)
            + if focus {
                " — Enter applies and writes"
            } else {
                " — ready in buffers; focus unchanged"
            };
    }

    /// A review that will never be applied says so in its own buffer. One
    /// that was applied keeps its diff; only its `u` is gone.
    pub(super) fn retire_review(&mut self, old: &ChangeProposal, reason: &str) {
        if !old.applied.is_empty() || self.docs.get(old.buffer).is_none() {
            return;
        }
        let note = format!(
            "{}: review {reason}\nnothing applied\n",
            old.plan.producer.label()
        );
        if let Err(error) = self.replace_system(old.buffer, &note) {
            self.message = format!("could not retire the review: {error}");
        }
        self.review
            .rows
            .insert(old.buffer, vec![ReviewRow::Warning, ReviewRow::Context]);
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
        let stale = self.review.pending.take_if(|proposal| {
            proposal.applied.is_empty()
                && proposal
                    .search
                    .is_some_and(|stamp| stamp.session == session)
        });
        if let Some(proposal) = stale {
            self.retire_review(&proposal, "is stale: Search changed; prepare a new one");
        }
    }
}
