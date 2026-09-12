//! Global replace (0051 §6 R04): Enter prepares a change-plan review
//! from the accepted hits — never a bulk write from the text fields.
//! Open buffers plan against their live text (a dirty buffer wins over
//! disk) with the 0007 §4 per-hit content witnesses intact; unopened
//! targets load through owned open jobs and join the same review.
//! Apply/cancel are the existing review commands (`:apply-change` /
//! `:cancel-change`); persistence is explicit (`:save-change`), never
//! implied by apply.

use std::collections::BTreeMap;
use std::path::PathBuf;

use strop_picker::Payload;
use strop_workspace::ResourceLocation;

use super::super::changes::review::PendingReplace;
use super::super::changes::{ChangePlan, ChangeProducer, PlannedDocument};
use super::super::Editor;

/// One grep hit with its content witness: (line, col, match_len,
/// expected line text) — the tuple the payload and open intent carry.
pub(crate) type Hit = (usize, usize, usize, String);

impl Editor {
    pub(crate) fn restore_replace_context(&mut self) {
        let Some(context) = self.review.replace_context.take() else {
            return;
        };
        if self.docs.get(context.origin.document).is_some() {
            self.jump_to(context.origin);
        }
        self.set_picker(super::PickerGlue::diagnostics(context.picker));
        self.picker_query_eval(strop_picker::Kind::Replace);
    }

    /// Replace-mode Enter: prepare the review (0051 §6 R04). A query
    /// without a content expression — or with a parse diagnostic — is
    /// a named refusal, never an empty bulk operation.
    pub(crate) fn apply_replace(&mut self) {
        let Some(glue) = self.picker.as_ref() else {
            return;
        };
        let query = strop_picker::query::SearchQuery::parse(&glue.picker.input.text);
        if let Some(diagnostic) = query.diagnostics.first() {
            self.message = format!("replace: {}", diagnostic.message);
            return;
        }
        if query.content.is_none() {
            self.message = "replace needs a search expression — add text or regex:".into();
            return;
        }
        let replacement = glue.picker.replace_input.text.clone();
        let mut by_path: BTreeMap<PathBuf, Vec<Hit>> = BTreeMap::new();
        for it in glue.picker.accepted() {
            if let Payload::Grep {
                path,
                line,
                col,
                match_len,
                line_text,
            } = &it.payload
            {
                by_path.entry(path.clone()).or_default().push((
                    *line,
                    *col,
                    *match_len,
                    line_text.clone(),
                ));
            }
        }
        if by_path.is_empty() {
            self.message = "replace: no matches in the apply set".into();
            return;
        }
        let origin = self.jump_record();
        if let Some(glue) = self.picker.as_mut() {
            let picker = std::mem::replace(
                &mut glue.picker,
                strop_picker::Picker::new(strop_picker::Kind::Replace, Vec::new(), false),
            );
            self.review.replace_context =
                Some(crate::editor::changes::review::ReplaceContext { picker, origin });
        }
        self.close_picker();
        // Partition: open buffers plan now (dirty text wins), unopened
        // targets load through owned jobs and complete the same review.
        let mut documents = Vec::new();
        let mut refused = Vec::new();
        let mut unopened = Vec::new();
        for (rel, hits) in by_path {
            let full = self.cwd.join(&rel);
            match self.buffer_index_of(&full) {
                Some(bi) => self.plan_replace_target(
                    &full,
                    bi,
                    &hits,
                    &replacement,
                    &mut documents,
                    &mut refused,
                ),
                None => unopened.push((full, hits)),
            }
        }
        if unopened.is_empty() {
            self.present_replace_review(documents, refused);
            return;
        }
        let pending_opens = unopened.len();
        let review = self.begin_replace(documents, refused, pending_opens);
        self.message = format!("replace: loading {pending_opens} target(s)…");
        for (full, hits) in unopened {
            self.request_open(
                full,
                super::super::io::OpenIntent::Replace {
                    hits,
                    replacement: replacement.clone(),
                    review,
                },
            );
        }
    }

    /// Verify one target's hits against CURRENT text and produce either
    /// a planned document (pinned base revision, exact edit spans) or a
    /// named refusal. The per-hit content witnesses are the 0007 §4
    /// stale-hit protection — unchanged by the move to change plans.
    fn plan_replace_target(
        &self,
        full: &std::path::Path,
        bi: strop_core::id::DocumentId,
        hits: &[Hit],
        replacement: &str,
        documents: &mut Vec<PlannedDocument>,
        refused: &mut Vec<(ResourceLocation, String)>,
    ) {
        let location = || ResourceLocation::local(full.to_path_buf());
        let Some(doc) = self.docs.get(bi) else {
            refused.push((location(), "document closed before the review".into()));
            return;
        };
        if doc.buf.readonly {
            refused.push((location(), "buffer is read-only".into()));
            return;
        }
        let (edits, stale) = verified_edits(&doc.buf, hits, replacement);
        if edits.is_empty() {
            refused.push((
                location(),
                format!("all {} match(es) stale — re-run the search", hits.len()),
            ));
            return;
        }
        if stale > 0 {
            refused.push((
                location(),
                format!(
                    "{stale} of {} match(es) stale at review — skipped",
                    hits.len()
                ),
            ));
        }
        documents.push(PlannedDocument {
            location: location(),
            document: bi,
            base: doc.buf.revision(),
            edits,
        });
    }

    /// The assembled review: every prepared target and every named
    /// refusal, always through the review buffer — never a direct
    /// apply, however small the plan.
    fn present_replace_review(
        &mut self,
        documents: Vec<PlannedDocument>,
        refused: Vec<(ResourceLocation, String)>,
    ) {
        if documents.is_empty() {
            self.message = match refused.first() {
                None => "replace: nothing to review".into(),
                Some((location, reason)) if refused.len() == 1 => {
                    format!("replace refused: {} — {reason}", location.label())
                }
                Some(_) => format!(
                    "replace: all {} target(s) refused — nothing to apply",
                    refused.len()
                ),
            };
            self.restore_replace_context();
            return;
        }
        self.review_change_plan(ChangePlan {
            producer: ChangeProducer::Replace,
            documents,
            refused,
        });
    }

    /// An owned open for the pending replace review landed: verify the
    /// hits against the loaded text and merge the outcome; the review
    /// presents when the last target resolves. A generation mismatch
    /// means a newer replace superseded this one — the buffer simply
    /// stays open.
    pub(crate) fn complete_replace_open(
        &mut self,
        document: strop_core::id::DocumentId,
        hits: &[Hit],
        replacement: &str,
        review: usize,
    ) {
        if self.review.replace.as_ref().is_none_or(|p| p.id != review) {
            return;
        }
        let full = self
            .docs
            .get(document)
            .and_then(|doc| doc.buf.path.clone())
            .unwrap_or_default();
        let mut documents = Vec::new();
        let mut refused = Vec::new();
        self.plan_replace_target(
            &full,
            document,
            hits,
            replacement,
            &mut documents,
            &mut refused,
        );
        let pending: &mut PendingReplace = self.review.replace.as_mut().unwrap();
        pending.documents.extend(documents);
        pending.refused.extend(refused);
        pending.pending_opens = pending.pending_opens.saturating_sub(1);
        if pending.pending_opens > 0 {
            self.message = format!("replace: loading {} more target(s)…", pending.pending_opens);
            return;
        }
        let pending = self.review.replace.take().unwrap();
        self.present_replace_review(pending.documents, pending.refused);
    }

    /// A replace target's open failed: a named refusal in the same
    /// review, never a silent drop (0051 §6 R04).
    pub(crate) fn refuse_replace_open(
        &mut self,
        path: &std::path::Path,
        reason: String,
        review: usize,
    ) {
        let Some(pending) = self.review.replace.as_mut().filter(|p| p.id == review) else {
            return;
        };
        pending
            .refused
            .push((ResourceLocation::local(path.to_path_buf()), reason));
        pending.pending_opens = pending.pending_opens.saturating_sub(1);
        if pending.pending_opens > 0 {
            self.message = format!("replace: loading {} more target(s)…", pending.pending_opens);
            return;
        }
        let pending = self.review.replace.take().unwrap();
        self.present_replace_review(pending.documents, pending.refused);
    }

    /// Open-buffer index for an absolute path, if loaded.
    fn buffer_index_of(&self, abs: &std::path::Path) -> Option<strop_core::id::DocumentId> {
        self.docs.iter().find_map(|(id, document)| {
            (document.buf.path.as_deref() == Some(abs) || document.buf.file_identity() == Some(abs))
                .then_some(id)
        })
    }
    /// Verified, single-transaction replacement in an open buffer —
    /// the test surface for the witness checks review planning shares
    /// (`verified_edits`); production replace goes through the review.
    #[cfg(test)]
    pub(crate) fn replace_in_buffer_pub(
        &mut self,
        bi: strop_core::id::DocumentId,
        hits: &[Hit],
        replacement: &str,
    ) -> (usize, usize, usize) {
        if self.docs.get(bi).is_none() || self.doc(bi).buf.readonly {
            return (0, 0, hits.len());
        }
        let (edits, stale) = verified_edits(&self.doc(bi).buf, hits, replacement);
        let applied = edits.len();
        if edits.is_empty() {
            return (0, 0, stale);
        }
        let base = self.doc(bi).buf.revision();
        match self.apply(
            bi,
            base,
            crate::editor::transact::ChangeSet {
                edits,
                undo_open: false,
            },
        ) {
            Ok(_) => (1, applied, stale),
            Err(_) => (0, 0, applied + stale), // raced — report all stale
        }
    }
}

/// Each hit retains its complete source-line witness. This protects regex
/// context and zero-width matches as well as the replaced bytes. All edits
/// are prepared against the same unchanged buffer before any are applied.
fn verified_edits(
    buf: &strop_core::Buffer,
    hits: &[Hit],
    replacement: &str,
) -> (Vec<strop_core::Replacement>, usize) {
    let mut edits = Vec::new();
    let mut stale = 0;
    for (line, col, match_len, expected) in hits {
        if *line == 0 || *line > buf.len_lines() {
            stale += 1;
            continue;
        }
        let (s, e) = strop_picker::replace_span(expected, *col, *match_len);
        let (ls, len) = (buf.line_start(line - 1), buf.len_bytes());
        let abs_s = ls + s;
        let abs_e = (ls + e).min(len);
        // A matching substring alone cannot prove an anchored/boundary regex
        // still matches, and an empty substring is no witness at all.
        let aligned = buf.is_boundary(abs_s) && buf.is_boundary(abs_e);
        let matches = aligned
            && abs_s <= abs_e
            && buf.text().byte_slice(ls..buf.line_end(line - 1)) == expected.as_str()
            && buf.text().byte_slice(abs_s..abs_e) == expected[s..e];
        if !matches {
            stale += 1;
            continue;
        }
        edits.push(strop_core::Replacement::new(
            strop_core::Range::charwise(abs_s, abs_e),
            replacement,
        ));
    }
    edits.sort_by_key(|edit| edit.range.start.get());
    (edits, stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;
    use strop_picker::{Item, Kind, Picker};

    /// One hit tuple: (line, col, match_len, expected line text).
    fn hit(line: usize, col: usize, len: usize, text: &str) -> Hit {
        (line, col, len, text.to_string())
    }

    #[test]
    fn verified_edits_skips_stale_witnesses() {
        let buf = Buffer::from_text("foo bar foo\n");
        let hits = vec![
            hit(1, 9, 3, "foo bar foo"),
            hit(1, 1, 3, "foo bar foo"),
            hit(1, 5, 3, "WRONG — stale line"),
        ];
        let (edits, stale) = verified_edits(&buf, &hits, "baz");
        assert_eq!((edits.len(), stale), (2, 1));
        assert!(edits[0].range.start.get() < edits[1].range.start.get());
    }

    #[test]
    fn changed_regex_context_and_empty_matches_refuse_old_hits() {
        let mut editor = Editor::new(Buffer::from_text("foobar\n"));
        let source = editor.current();
        for witness in [hit(1, 1, 3, "foo"), hit(1, 4, 0, "foo")] {
            assert_eq!(
                editor.replace_in_buffer_pub(source, &[witness], "bar"),
                (0, 0, 1)
            );
            assert_eq!(editor.buf().text(), "foobar\n");
        }
    }

    #[test]
    fn enter_without_content_expression_is_a_named_refusal() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Replace);
        e.feed_text("language:rust");
        e.apply_replace();
        assert!(
            e.message.contains("search expression"),
            "named state, not an empty bulk op: {}",
            e.message
        );
        assert!(e.picker.is_some(), "the picker stays open to fix the query");
        assert!(e.review.replace.is_none());
    }

    #[test]
    fn enter_without_matches_mutates_nothing() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Replace);
        e.feed_text("foo");
        e.apply_replace();
        assert_eq!(e.message, "replace: no matches in the apply set");
        assert!(e.picker.is_some());
        assert!(e.review.replace.is_none());
    }

    fn text_of(e: &Editor, document: strop_core::id::DocumentId) -> String {
        e.docs.get(document).unwrap().buf.text().to_string()
    }

    /// Space R against a temp tree: one open buffer, one unopened
    /// file. Enter must present one review covering both files without
    /// mutating anything; apply writes dirty edits through the gateway
    /// with a receipt and no saves; `:save-change` persists exactly
    /// those two documents, identically for open and unopened targets.
    #[test]
    fn replace_review_apply_and_save_change_flow() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "alpha foo\nbeta foo\ngamma\n").unwrap();
        std::fs::write(&b, "foo one\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        let a_doc = e.open_fixture(&a).unwrap();

        e.feed_text(" Rfoo");
        e.wait_picker();
        assert_eq!(e.picker.as_ref().unwrap().picker.items.len(), 3);
        e.feed(crate::editor::Key::Tab);
        e.feed_text("bar");
        e.accept_current_picker();
        // the unopened target loads through its owned job, then the
        // review presents — nothing has mutated yet
        e.wait_io().unwrap();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        let review = e.current();
        let proposal = e.buf().text().to_string();
        assert!(
            proposal.contains("strop change proposal 1: replace"),
            "{proposal}"
        );
        assert!(proposal.contains("-alpha foo"), "{proposal}");
        assert!(proposal.contains("+alpha bar"), "{proposal}");
        assert!(proposal.contains("-beta foo"), "{proposal}");
        assert!(proposal.contains("-foo one"), "{proposal}");
        assert!(proposal.contains("+bar one"), "{proposal}");
        assert!(proposal.contains(":apply-change"), "{proposal}");
        assert_eq!(text_of(&e, a_doc), "alpha foo\nbeta foo\ngamma\n");
        let b_doc = e
            .docs
            .iter()
            .find_map(|(id, doc)| (doc.buf.path.as_deref() == Some(b.as_path())).then_some(id))
            .expect("the unopened target loaded as a real buffer");
        assert_eq!(text_of(&e, b_doc), "foo one\n");
        assert!(!e.io_pending(), "the review schedules no saves");

        e.review_apply_pub();
        assert_eq!(text_of(&e, a_doc), "alpha bar\nbeta bar\ngamma\n");
        assert_eq!(text_of(&e, b_doc), "bar one\n");
        assert!(e.docs.get(a_doc).unwrap().buf.dirty);
        assert!(e.docs.get(b_doc).unwrap().buf.dirty);
        // apply ≠ save: both buffers dirty, both files untouched on disk
        assert!(!e.io_pending(), "apply schedules no saves");
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "alpha foo\nbeta foo\ngamma\n"
        );
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "foo one\n");
        let receipt = text_of(&e, review);
        assert!(receipt.contains("— APPLIED"), "{receipt}");
        assert!(receipt.contains("a.txt"), "{receipt}");
        assert!(receipt.contains("b.txt"), "{receipt}");

        // explicit persistence: open and unopened targets save
        // identically through the per-document contract — and ONLY
        // those targets: an unrelated dirty file is left alone
        let c = dir.path().join("c.txt");
        std::fs::write(&c, "unrelated\n").unwrap();
        let c_doc = e.open_fixture(&c).unwrap();
        e.feed_text("Ihand edited \x1b");
        assert!(e.docs.get(c_doc).unwrap().buf.dirty);
        e.save_changed_files_pub();
        e.wait_io().unwrap();
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "alpha bar\nbeta bar\ngamma\n"
        );
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "bar one\n");
        assert!(!e.docs.get(a_doc).unwrap().buf.dirty);
        assert!(!e.docs.get(b_doc).unwrap().buf.dirty);
        assert_eq!(
            std::fs::read_to_string(&c).unwrap(),
            "unrelated\n",
            ":save-change saves exactly the change-plan documents"
        );
        assert!(e.docs.get(c_doc).unwrap().buf.dirty);

        // the receipt anchors grouped undo of the applied group
        e.feed_text(":undo-change\r");
        assert_eq!(text_of(&e, a_doc), "alpha foo\nbeta foo\ngamma\n");
        assert_eq!(text_of(&e, b_doc), "foo one\n");
    }

    /// Dirty open buffers win over disk (0051 §6 R04): a hit whose
    /// witness fails against the live buffer text is named stale at
    /// the review — the plan follows the buffer, not the disk file.
    #[test]
    fn dirty_open_buffer_wins_over_disk() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "alpha foo\nbeta foo\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        let a_doc = e.open_fixture(&a).unwrap();
        e.feed_text(" Rfoo");
        e.wait_picker();
        assert_eq!(e.picker.as_ref().unwrap().picker.items.len(), 2);
        // the buffer drifts from the searched disk text (unsaved) —
        // the picker must be closed for keys to reach the buffer
        e.close_picker();
        e.switch_to(a_doc);
        e.feed_text("ichanged <esc>");
        // same search again (disk is unchanged), then Enter
        e.feed_text(" Rfoo");
        e.wait_picker();
        e.feed(crate::editor::Key::Tab);
        e.feed_text("bar");
        e.accept_current_picker();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        let proposal = e.buf().text().to_string();
        assert!(proposal.contains("+beta bar"), "{proposal}");
        e.review_apply_pub();
        assert_eq!(text_of(&e, a_doc), "changed alpha bar\nbeta bar\n");
    }
    #[test]
    fn readonly_target_is_named_not_bulk_written() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "alpha foo\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        let a_doc = e.open_fixture(&a).unwrap();
        e.doc_mut(a_doc).buf.readonly = true;
        e.feed_text(" Rfoo");
        e.wait_picker();
        assert_eq!(e.picker.as_ref().unwrap().picker.items.len(), 1);
        e.feed(crate::editor::Key::Tab);
        e.feed_text("bar");
        // direct prepare: the Enter/accept path is covered end-to-end
        // by replace_review_apply_and_save_change_flow; this test pins
        // the refusal naming, not accept timing
        e.apply_replace();
        assert!(e.message.contains("a.txt"), "named: {}", e.message);
        assert!(e.message.contains("read-only"), "{}", e.message);
        assert_eq!(text_of(&e, a_doc), "alpha foo\n");
        assert_ne!(
            e.buf().name.as_deref(),
            Some("change proposal 1"),
            "no empty review for a fully refused operation"
        );
    }

    #[test]
    fn failed_open_is_named_in_review_and_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "alpha foo\n").unwrap();
        std::fs::write(&b, "foo one\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        let a_doc = e.open_fixture(&a).unwrap();
        e.feed_text(" Rfoo");
        e.wait_picker();
        e.feed(crate::editor::Key::Tab);
        e.feed_text("bar");
        // the unopened target becomes unreadable before its owned open
        // lands (a directory never reads as a file)
        std::fs::remove_file(&b).unwrap();
        std::fs::create_dir(&b).unwrap();
        e.accept_current_picker();
        e.wait_io().unwrap();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        let review = e.current();
        let proposal = e.buf().text().to_string();
        assert!(
            proposal.contains("b.txt"),
            "failed target named: {proposal}"
        );
        assert!(proposal.contains("open failed"), "{proposal}");
        e.review_apply_pub();
        assert_eq!(text_of(&e, a_doc), "alpha bar\n");
        let receipt = text_of(&e, review);
        assert!(receipt.contains("refused: "), "{receipt}");
        assert!(receipt.contains("b.txt"), "{receipt}");
        assert!(receipt.contains("open failed"), "{receipt}");
    }

    #[test]
    fn apply_refuses_a_target_edited_since_the_review_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "alpha foo\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        let a_doc = e.open_fixture(&a).unwrap();
        e.feed_text(" Rfoo");
        e.wait_picker();
        e.feed(crate::editor::Key::Tab);
        e.feed_text("bar");
        e.accept_current_picker();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        // the source moves after the review was prepared
        e.switch_to(a_doc);
        e.feed_text("Ichanged \x1b");
        e.review_apply_pub();
        assert_eq!(
            text_of(&e, a_doc),
            "changed alpha foo\n",
            "a moved target is refused, never recomputed"
        );
        let receipt = e
            .docs
            .iter()
            .find_map(|(id, doc)| {
                (doc.buf.name.as_deref() == Some("change proposal 1")).then_some(id)
            })
            .unwrap();
        let receipt = text_of(&e, receipt);
        assert!(receipt.contains("refused: "), "{receipt}");
        assert!(
            receipt.contains("a.txt"),
            "the stale target is named: {receipt}"
        );
        assert!(receipt.contains("edited since the proposal"), "{receipt}");
    }

    #[test]
    fn cancel_mutates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "alpha foo\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
        let a_doc = e.open_fixture(&a).unwrap();
        e.feed_text(" Rfoo");
        e.wait_picker();
        e.feed(crate::editor::Key::Tab);
        e.feed_text("bar");
        e.accept_current_picker();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        e.review_cancel_pub();
        assert_eq!(text_of(&e, a_doc), "alpha foo\n");
        assert!(!e.docs.get(a_doc).unwrap().buf.dirty);
        assert!(!e.io_pending(), "cancel schedules no saves");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "alpha foo\n");
    }

    #[test]
    fn excluded_rows_stay_out_of_the_apply_set() {
        let mut p = Picker::new(
            Kind::Replace,
            vec![
                Item {
                    badge: None,
                    text: "a".into(),
                    payload: Payload::Grep {
                        path: PathBuf::from("a"),
                        line: 1,
                        col: 1,
                        match_len: 1,
                        line_text: "x".into(),
                    },
                },
                Item {
                    badge: None,
                    text: "b".into(),
                    payload: Payload::Grep {
                        path: PathBuf::from("b"),
                        line: 1,
                        col: 1,
                        match_len: 1,
                        line_text: "y".into(),
                    },
                },
            ],
            false,
        );
        p.install_ranking(
            strop_picker::rank::rank(&p.filter_request(), || false)
                .unwrap()
                .unwrap(),
        );
        p.toggle_excluded(); // excludes row 0
        assert_eq!(p.accepted().count(), 1);
        assert_eq!(p.accepted().next().unwrap().text, "b");
        p.toggle_excluded(); // toggles back
        assert_eq!(p.accepted().count(), 2);
    }

    #[test]
    fn space_r_replace_field_flow() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" Rfoo");
        assert_eq!(e.picker.as_ref().unwrap().picker.kind, Kind::Replace);

        e.feed(crate::editor::Key::Tab);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.field,
            strop_picker::Field::Replace
        );
        e.feed_text("bar");
        assert_eq!(e.picker.as_ref().unwrap().picker.replace_input.text, "bar");
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "foo");
    }

    #[test]
    fn replace_filters_narrow_the_apply_set() {
        // user ask: extension limiting + file exclusion in Space R —
        // the qualifier language scopes the hit set the review prepares
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "foo one\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "foo two\n").unwrap();
        std::fs::write(dir.path().join("c.py"), "foo three\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_picker(Kind::Replace);
        // 0051: the qualifier language, not rg passthrough flags
        e.feed_text("foo -glob:\"*.py\"");
        e.wait_picker();
        let p = &e.picker.as_ref().unwrap().picker;
        assert_eq!(p.items.len(), 2, "py excluded via -glob:");
        assert!(p.items.iter().all(|i| !format!("{i:?}").contains("c.py")));
    }
}
