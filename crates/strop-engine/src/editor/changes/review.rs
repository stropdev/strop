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
//! Integration wiring (reported to 0049's integrator, none done here):
//! `changes/mod.rs` declares `pub(crate) mod review;`, `editor/mod.rs`
use strop_core::id::DocumentId;
use strop_core::{Buffer, Replacement};

use super::{ChangePlan, ChangeReceipt};
use crate::editor::transact::ChangeSet;
use crate::editor::{Document, Editor};

/// The command names the review buffer advertises. Integration wires these
/// exact names to `review_apply_pub` / `review_cancel_pub`.
pub(crate) const APPLY_COMMAND: &str = ":apply-change";
pub(crate) const CANCEL_COMMAND: &str = ":cancel-change";

/// Unified-diff context lines around each changed span.
const CONTEXT: usize = 3;

/// Pending-proposal state. One proposal is reviewable at a time; a newer
/// proposal supersedes the older one's buffer with a visible note.
#[derive(Debug, Default)]
pub(crate) struct ReviewState {
    /// Monotone proposal identity; receipt headers name it.
    seq: usize,
    /// The proposal awaiting Apply/Cancel, if any.
    pending: Option<ChangeProposal>,
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
        if let Some(old) = self.review.pending.take() {
            let note = format!(
                "strop change proposal {}: {} — SUPERSEDED by a newer proposal\n",
                old.id,
                old.plan.producer.label()
            );
            if let Err(error) = self.replace_system(old.buffer, &note) {
                self.message = format!("could not retire proposal {}: {error}", old.id);
            }
        }
        self.review.seq += 1;
        let id = self.review.seq;
        let text = self.render_proposal(id, &plan);
        let producer = plan.producer.label().to_string();
        let files = plan.documents.len();
        let refused = plan.refused.len();
        let mut buf = Buffer::from_text(&text);
        buf.name = Some(format!("change proposal {id}"));
        let buffer = self.docs.insert(Document::output(buf));
        self.drop_stale_scratch(buffer);
        self.switch_to(buffer);
        self.set_head(0);
        self.view_mut().view_top = 0;
        self.review.pending = Some(ChangeProposal { id, plan, buffer });
        self.message = match refused {
            0 => format!(
                "{producer}: proposal {id} reviews {files} file(s) — {APPLY_COMMAND} or {CANCEL_COMMAND}"
            ),
            _ => format!(
                "{producer}: proposal {id} reviews {files} file(s), {refused} refused — {APPLY_COMMAND} or {CANCEL_COMMAND}"
            ),
        };
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
        let producer = proposal.plan.producer.label().to_string();
        let mut receipt = ChangeReceipt {
            producer: producer.clone(),
            applied: Vec::new(),
            refused: proposal.plan.refused.clone(),
            redo_depths: None,
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
                        undo_open: true,
                    };
                    match self.apply(target.document, target.base, changes) {
                        Ok(committed) => {
                            receipt.applied.push((
                                target.document,
                                target.base,
                                committed.revision,
                            ));
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
        let mut text = format!(
            "strop change proposal {}: {producer} — APPLIED\n{} buffer(s) applied, {} target(s) refused\n:undo-change reverts the applied group\n\n",
            proposal.id,
            receipt.applied.len(),
            receipt.refused.len()
        );
        for line in &lines {
            text.push_str(line);
            text.push('\n');
        }
        let publish = self.replace_system(proposal.buffer, &text);
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
        let Some(proposal) = self.review.pending.take() else {
            self.message = "no change proposal awaiting review".into();
            return;
        };
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
    }

    /// Render the review buffer: header, per-file unified diffs computed
    /// from each target's pinned base, and every refused target named with
    /// its reason.
    fn render_proposal(&self, id: usize, plan: &ChangePlan) -> String {
        let producer = plan.producer.label();
        let mut text = format!(
            "strop change proposal {id}: {producer}\n{} file(s) to change, {} target(s) refused\n{APPLY_COMMAND} applies exactly what is shown; {CANCEL_COMMAND} discards it\nbases are pinned — editing a source invalidates that file at apply\n",
            plan.documents.len(),
            plan.refused.len()
        );
        for target in &plan.documents {
            text.push('\n');
            let label = target.location.label();
            match self.docs.get(target.document) {
                Some(document) => {
                    let base = document.buf.text().to_string();
                    text.push_str(&render_file_diff(&label, &base, &target.edits));
                }
                None => {
                    text.push_str(&format!("--- {label}: document closed since the plan\n"));
                }
            }
        }
        if !plan.refused.is_empty() {
            text.push_str("\nrefused targets:\n");
            for (location, reason) in &plan.refused {
                text.push_str(&format!("  {} — {reason}\n", location.label()));
            }
        }
        text
    }
}

/// A changed line span in the base text plus its replacement lines.
struct Span {
    /// First affected base line (0-based).
    first: usize,
    /// Last affected base line, inclusive.
    last: usize,
    /// The lines replacing that span (content, no terminators).
    added: Vec<String>,
}

/// Line starts of `text`; an empty text is one empty line.
fn line_layout(text: &str) -> (Vec<usize>, Vec<&str>) {
    let lines: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.split_inclusive('\n').collect()
    };
    let mut starts = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        starts.push(offset);
        offset += line.len();
    }
    (starts, lines)
}

/// The 0-based line containing byte offset `byte`.
fn line_of(starts: &[usize], byte: usize) -> usize {
    starts
        .partition_point(|start| *start <= byte)
        .saturating_sub(1)
}

/// Clamp `byte` down to a char boundary (prepared ranges are boundaries
/// already; this keeps rendering total for any server oddity).
fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// A line's content without its terminator.
fn strip_newline(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

/// Per-file unified diff of the prepared edits against the pinned base
/// text. Hunks come from the known edit spans (not a text re-diff), so the
/// review always shows exactly what apply will do.
fn render_file_diff(label: &str, base: &str, edits: &[Replacement]) -> String {
    let mut out = format!("--- a/{label}\n+++ b/{label}\n");
    let (starts, lines) = line_layout(base);
    let last_line = lines.len() - 1;
    let mut spans: Vec<Span> = Vec::with_capacity(edits.len());
    for edit in edits {
        let start = floor_boundary(base, edit.range.start.get());
        let end = floor_boundary(base, edit.range.end.get());
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let first = line_of(&starts, start).min(last_line);
        let last = if end > start {
            line_of(&starts, end - 1).min(last_line)
        } else {
            first
        };
        // The replacement plus the unchanged fragments of the boundary
        // lines gives the span's added lines exactly.
        let line_end = starts.get(last + 1).copied().unwrap_or(base.len());
        let joined = format!(
            "{}{}{}",
            &base[starts[first]..start],
            edit.text,
            &base[end..line_end]
        );
        let mut added: Vec<String> = joined.split('\n').map(str::to_string).collect();
        if joined.ends_with('\n') {
            added.pop();
        }
        spans.push(Span { first, last, added });
    }
    // Merge context windows that touch; spans are start-sorted.
    let mut windows: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (index, span) in spans.iter().enumerate() {
        let window_first = span.first.saturating_sub(CONTEXT);
        let window_last = (span.last + CONTEXT).min(last_line);
        match windows.last_mut() {
            Some(window) if window_first <= window.1 + 1 => {
                window.1 = window.1.max(window_last);
                window.3 = index + 1;
            }
            _ => windows.push((window_first, window_last, index, index + 1)),
        }
    }
    let mut delta: isize = 0;
    for (window_first, window_last, span_lo, span_hi) in windows {
        let window_spans = &spans[span_lo..span_hi];
        let removed: usize = window_spans
            .iter()
            .map(|span| span.last + 1 - span.first)
            .sum();
        let added: usize = window_spans.iter().map(|span| span.added.len()).sum();
        let base_count = window_last + 1 - window_first;
        let new_count = base_count - removed + added;
        let new_first = (window_first as isize + delta + 1).max(1);
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            window_first + 1,
            base_count,
            new_first,
            new_count
        ));
        let mut cursor = window_first;
        for span in window_spans {
            for line in &lines[cursor..span.first.max(cursor)] {
                out.push(' ');
                out.push_str(strip_newline(line));
                out.push('\n');
            }
            for line in &lines[span.first..=span.last] {
                out.push('-');
                out.push_str(strip_newline(line));
                out.push('\n');
            }
            for line in &span.added {
                out.push('+');
                out.push_str(line);
                out.push('\n');
            }
            cursor = cursor.max(span.last + 1);
        }
        for line in &lines[cursor..=window_last] {
            out.push(' ');
            out.push_str(strip_newline(line));
            out.push('\n');
        }
        delta += added as isize - removed as isize;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::ChangeProducer;
    use super::*;
    use std::path::PathBuf;
    use strop_core::id::LineIndex;
    use strop_lsp::{PositionEncoding, ServerColumn, ServerEdit, ServerId, ServerPosition};
    use strop_workspace::ResourceLocation;

    fn at(line: usize, col: usize) -> ServerPosition {
        ServerPosition {
            line: LineIndex::new(line),
            column: ServerColumn::new(col),
        }
    }

    fn edit(sl: usize, sc: usize, el: usize, ec: usize, text: &str) -> ServerEdit {
        ServerEdit {
            start: at(sl, sc),
            end: at(el, ec),
            new_text: text.into(),
        }
    }

    fn file_editor(dir: &tempfile::TempDir, name: &str, text: &str) -> (Editor, DocumentId) {
        std::fs::write(dir.path().join(name), text).unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        e.open_fixture(&dir.path().join(name)).unwrap();
        let document = e.current();
        (e, document)
    }

    fn open_second(e: &mut Editor, dir: &tempfile::TempDir, name: &str, text: &str) -> DocumentId {
        std::fs::write(dir.path().join(name), text).unwrap();
        e.open_fixture(&dir.path().join(name)).unwrap();
        e.current()
    }

    /// Bind the document to a server so `build_change_plan` resolves its
    /// location — the inverse map the live LSP path maintains.
    fn bind(e: &mut Editor, document: DocumentId) {
        let (revision, path) = {
            let doc = e.docs.get(document).unwrap();
            (doc.buf.revision(), doc.buf.path.clone().unwrap())
        };
        e.lsp_state.bindings.insert(
            document,
            crate::editor::lsp::state::Binding {
                server: ServerId::new(7),
                revision,
                path,
                root: PathBuf::from("/workspace"),
                language: "rust".into(),
                target: strop_workspace::Filesystem::Local,
            },
        );
    }

    fn text_of(e: &Editor, document: DocumentId) -> String {
        e.docs.get(document).unwrap().buf.text().to_string()
    }

    /// A two-document rename proposal with one unopened (refused) target.
    fn two_file_editor() -> (Editor, tempfile::TempDir, DocumentId, DocumentId) {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, first) = file_editor(&dir, "a.txt", "alpha here\n");
        let second = open_second(&mut e, &dir, "b.txt", "alpha there\n");
        bind(&mut e, first);
        bind(&mut e, second);
        let plan = e.build_change_plan(
            ChangeProducer::Rename,
            vec![
                (
                    ResourceLocation::local(dir.path().join("a.txt")),
                    vec![edit(0, 0, 0, 5, "omega")],
                ),
                (
                    ResourceLocation::local(dir.path().join("b.txt")),
                    vec![edit(0, 0, 0, 5, "omega")],
                ),
                (
                    ResourceLocation::local(dir.path().join("ghost.txt")),
                    vec![edit(0, 0, 0, 3, "x")],
                ),
            ],
            PositionEncoding::Utf8,
        );
        e.present_change_plan(plan);
        (e, dir, first, second)
    }

    #[test]
    fn multi_target_plan_opens_a_diff_review_without_mutating() {
        let (e, _dir, first, _second) = two_file_editor();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        let review = e.current();
        assert_ne!(review, first, "the review buffer takes focus");
        assert!(e.buf().readonly, "the review is a real read-only buffer");
        let text = e.buf().text().to_string();
        assert!(text.contains("strop change proposal 1: rename"), "{text}");
        assert!(text.contains(APPLY_COMMAND), "{text}");
        assert!(text.contains(CANCEL_COMMAND), "{text}");
        // Per-file unified diffs against the pinned bases.
        assert!(text.contains("--- a/"), "{text}");
        assert!(text.contains("+++ b/"), "{text}");
        assert!(text.contains("@@ -1,1 +1,1 @@"), "{text}");
        assert!(text.contains("-alpha here"), "{text}");
        assert!(text.contains("+omega here"), "{text}");
        assert!(text.contains("-alpha there"), "{text}");
        assert!(text.contains("+omega there"), "{text}");
        // The unopened target is named with its reason, never dropped.
        assert!(text.contains("ghost.txt"), "{text}");
        assert!(text.contains("not open on the server"), "{text}");
        // Nothing has mutated yet.
        assert_eq!(text_of(&e, first), "alpha here\n");
        assert!(e.message.contains("proposal 1"), "{}", e.message);
    }

    #[test]
    fn apply_mutates_exactly_the_planned_sources_and_leaves_a_receipt() {
        let (mut e, _dir, first, second) = two_file_editor();
        let review = e.current();
        e.review_apply_pub();
        assert_eq!(text_of(&e, first), "omega here\n");
        assert_eq!(text_of(&e, second), "omega there\n");
        assert_eq!(
            e.message,
            "rename: 2 buffer(s) applied, 1 target(s) refused"
        );
        // The review buffer stays around as the receipt.
        let receipt = text_of(&e, review);
        assert!(receipt.contains("— APPLIED"), "{receipt}");
        assert!(receipt.contains("applied: "), "{receipt}");
        assert!(receipt.contains("a.txt"), "{receipt}");
        assert!(receipt.contains("b.txt"), "{receipt}");
        assert!(receipt.contains("refused: "), "{receipt}");
        assert!(receipt.contains("ghost.txt"), "{receipt}");
        assert!(receipt.contains("not open on the server"), "{receipt}");
        // The recorded receipt anchors grouped undo across both buffers.
        e.feed_text(":undo-change\r");
        assert_eq!(text_of(&e, first), "alpha here\n");
        assert_eq!(text_of(&e, second), "alpha there\n");
    }

    #[test]
    fn cancel_mutates_nothing_and_leaves_a_cancelled_receipt() {
        let (mut e, _dir, first, second) = two_file_editor();
        let review = e.current();
        e.review_cancel_pub();
        assert_eq!(text_of(&e, first), "alpha here\n");
        assert_eq!(text_of(&e, second), "alpha there\n");
        let receipt = text_of(&e, review);
        assert!(receipt.contains("— CANCELLED"), "{receipt}");
        assert!(receipt.contains("nothing applied"), "{receipt}");
        assert!(receipt.contains("ghost.txt"), "{receipt}");
        assert!(e.message.contains("cancelled"), "{}", e.message);
        // No receipt was recorded: there is nothing to undo.
        e.feed_text(":undo-change\r");
        assert_eq!(e.message, "no change to undo");
        // A second cancel is a named no-op, not a panic or a stale apply.
        e.review_cancel_pub();
        assert_eq!(e.message, "no change proposal awaiting review");
    }

    #[test]
    fn a_source_edited_since_the_proposal_is_refused_by_name() {
        let (mut e, _dir, first, second) = two_file_editor();
        let review = e.current();
        e.switch_to(first);
        e.feed_text("0rx"); // revision moves past the proposal's base
        e.review_apply_pub();
        // The user's edit stands; the proposal never recomputes onto it.
        assert_eq!(text_of(&e, first), "xlpha here\n");
        assert_eq!(text_of(&e, second), "omega there\n");
        assert_eq!(
            e.message,
            "rename: 1 buffer(s) applied, 2 target(s) refused"
        );
        let receipt = text_of(&e, review);
        assert!(receipt.contains("a.txt"), "{receipt}");
        assert!(receipt.contains("edited since the proposal"), "{receipt}");
        assert!(receipt.contains("re-run the rename"), "{receipt}");
    }

    #[test]
    fn a_source_closed_since_the_proposal_is_refused_by_name() {
        let (mut e, _dir, first, second) = two_file_editor();
        let review = e.current();
        e.switch_to(second);
        e.close_buffer(true); // close b.txt after the proposal
        assert_eq!(text_of(&e, first), "alpha here\n");
        e.review_apply_pub();
        assert_eq!(text_of(&e, first), "omega here\n");
        let receipt = text_of(&e, review);
        assert!(receipt.contains("closed since the proposal"), "{receipt}");
        assert_eq!(
            e.message,
            "rename: 1 buffer(s) applied, 2 target(s) refused"
        );
    }

    #[test]
    fn a_single_clean_document_applies_directly_with_the_existing_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, document) = file_editor(&dir, "a.txt", "alpha here\n");
        bind(&mut e, document);
        let plan = e.build_change_plan(
            ChangeProducer::Rename,
            vec![(
                ResourceLocation::local(dir.path().join("a.txt")),
                vec![edit(0, 0, 0, 5, "omega")],
            )],
            PositionEncoding::Utf8,
        );
        e.present_change_plan(plan);
        assert_eq!(e.current(), document, "focus stays on the edited file");
        assert_eq!(text_of(&e, document), "omega here\n");
        assert_eq!(e.message, "rename: applied to 1 buffer(s)");
    }

    #[test]
    fn a_newer_proposal_supersedes_the_older_buffer_visibly() {
        let (mut e, dir, first, _second) = two_file_editor();
        let stale = e.current();
        let plan = e.build_change_plan(
            ChangeProducer::CodeAction,
            vec![
                (
                    ResourceLocation::local(dir.path().join("a.txt")),
                    vec![edit(0, 0, 0, 5, "sigma")],
                ),
                (
                    ResourceLocation::local(dir.path().join("b.txt")),
                    vec![edit(0, 0, 0, 5, "sigma")],
                ),
            ],
            PositionEncoding::Utf8,
        );
        e.present_change_plan(plan);
        assert_ne!(e.current(), stale);
        let retired = text_of(&e, stale);
        assert!(retired.contains("SUPERSEDED"), "{retired}");
        assert!(e.buf().text().to_string().contains("change proposal 2"));
        e.review_apply_pub();
        assert_eq!(text_of(&e, first), "sigma here\n");
    }

    #[test]
    fn nearby_edits_share_one_hunk_with_context_on_both_sides() {
        // Rendering is exercised through the public review buffer, so the
        // hunk layout a user sees is what is asserted.
        let dir = tempfile::tempdir().unwrap();
        let (mut e, document) = file_editor(
            &dir,
            "a.txt",
            "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
        );
        bind(&mut e, document);
        let plan = e.build_change_plan(
            ChangeProducer::CodeAction,
            vec![
                (
                    ResourceLocation::local(dir.path().join("a.txt")),
                    vec![edit(1, 0, 1, 3, "TWO"), edit(2, 0, 2, 5, "THREE")],
                ),
                (
                    ResourceLocation::local(dir.path().join("ghost.txt")),
                    vec![edit(0, 0, 0, 1, "x")],
                ),
            ],
            PositionEncoding::Utf8,
        );
        e.present_change_plan(plan);
        let text = e.buf().text().to_string();
        // Adjacent edits merge into one hunk with context on both sides.
        assert!(text.contains("@@ -1,6 +1,6 @@"), "{text}");
        assert!(
            text.contains(" one\n-two\n+TWO\n-three\n+THREE\n four\n five\n six\n"),
            "{text}"
        );
        assert_eq!(
            text.lines().filter(|line| line.starts_with("@@")).count(),
            1,
            "{text}"
        );
        e.review_apply_pub();
        assert_eq!(
            text_of(&e, document),
            "one\nTWO\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
        );
    }
}
