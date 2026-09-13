//! Source witnesses shared by Search opening and owned replacement preparation.

#[cfg(test)]
use super::super::Editor;

/// Exact source-line witness carried across loading and review preparation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplacementHit {
    pub line: usize,
    pub col: usize,
    pub match_len: usize,
    pub text: std::sync::Arc<str>,
}

#[cfg(test)]
impl Editor {
    /// Verified, single-transaction replacement in an open buffer —
    /// the test surface for the witness checks review planning shares
    /// (`verified_edits`); production replace goes through the review.
    #[cfg(test)]
    pub(crate) fn replace_in_buffer_pub(
        &mut self,
        bi: strop_core::id::DocumentId,
        hits: &[ReplacementHit],
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
#[cfg(test)]
fn verified_edits(
    buf: &strop_core::Buffer,
    hits: &[ReplacementHit],
    replacement: &str,
) -> (Vec<strop_core::Replacement>, usize) {
    let mut edits = Vec::new();
    let mut stale = 0;
    for hit in hits {
        if let Some(range) = checked_hit_range(buf.text(), hit) {
            edits.push(strop_core::Replacement::new(range, replacement));
        } else {
            stale += 1;
        }
    }
    edits.sort_by_key(|edit| edit.range.start.get());
    (edits, stale)
}

/// Verify an exact Search witness against authoritative source text.
pub fn checked_hit_range(buffer: &ropey::Rope, hit: &ReplacementHit) -> Option<strop_core::Range> {
    if hit.line == 0 || hit.line > buffer.len_lines() {
        return None;
    }
    let start = hit.col.checked_sub(1)?;
    let end = start.checked_add(hit.match_len)?;
    let (checked_start, checked_end) =
        strop_picker::replace_span(&hit.text, hit.col, hit.match_len);
    if (start, end) != (checked_start, checked_end) {
        return None;
    }
    let line_start = buffer.line_to_byte(hit.line - 1);
    let mut line_end = if hit.line < buffer.len_lines() {
        buffer.line_to_byte(hit.line)
    } else {
        buffer.len_bytes()
    };
    if line_end > line_start && buffer.byte(line_end - 1) == b'\n' {
        line_end -= 1;
    }
    let absolute_start = line_start.checked_add(start)?;
    let absolute_end = line_start.checked_add(end)?;
    // Equality to the char-boundary-checked witness establishes source boundaries.
    (absolute_end <= line_end && buffer.byte_slice(line_start..line_end) == hit.text.as_ref())
        .then(|| strop_core::Range::charwise(absolute_start, absolute_end))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use strop_core::Buffer;
    use strop_picker::{Item, Payload, Picker};

    /// One exact source witness.
    fn hit(line: usize, col: usize, len: usize, text: &str) -> ReplacementHit {
        ReplacementHit {
            line,
            col,
            match_len: len,
            text: text.into(),
        }
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
        e.open_search(true);
        e.feed_text("language:rust");
        e.prepare_search_review();
        assert!(!e.message.is_empty());
        assert!(e.picker.is_some(), "the picker stays open to fix the query");
        assert_eq!(e.buf().text(), "x\n");
    }

    #[test]
    fn enter_without_matches_mutates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new_in(Buffer::from_text("x\n"), dir.path().to_path_buf());
        e.open_search(true);
        e.feed_text("foo");
        e.wait_picker();
        e.prepare_search_review();
        assert!(e.picker.is_some());
        assert_eq!(e.buf().text(), "x\n");
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
        e.open_search(true);
        e.wait_picker();
        e.feed_text("bar");
        e.accept_current_picker();
        e.wait_io().unwrap();
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
        e.prepare_search_review();
        e.wait_io().unwrap();
        let review = e.buf().text().to_string();
        assert!(review.contains("a.txt"), "{review}");
        assert!(review.contains("read-only"), "{review}");
        e.review_apply_pub();
        assert_eq!(text_of(&e, a_doc), "alpha foo\n");
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
        e.review_apply_pub();
        assert_eq!(text_of(&e, a_doc), "alpha bar\n");
        let receipt = text_of(&e, review);
        assert!(receipt.contains("refused: "), "{receipt}");
        assert!(receipt.contains("b.txt"), "{receipt}");
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
        e.wait_io().unwrap();
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
        e.wait_io().unwrap();
        assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
        e.review_cancel_pub();
        assert_eq!(text_of(&e, a_doc), "alpha foo\n");
        assert!(!e.docs.get(a_doc).unwrap().buf.dirty);
        e.wait_picker();
        assert!(!e.io_pending(), "cancel schedules no saves");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "alpha foo\n");
    }

    #[test]
    fn excluded_rows_stay_out_of_the_apply_set() {
        let mut p = Picker::search(
            vec![
                Item {
                    badge: None,
                    text: "a".into(),
                    payload: Payload::Grep {
                        location: strop_workspace::ResourceLocation::local(PathBuf::from("/a")),
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
                        location: strop_workspace::ResourceLocation::local(PathBuf::from("/b")),
                        line: 1,
                        col: 1,
                        match_len: 1,
                        line_text: "y".into(),
                    },
                },
            ],
            false,
            true,
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
    fn replace_filters_narrow_the_apply_set() {
        // user ask: extension limiting + file exclusion in Space R —
        // the qualifier language scopes the hit set the review prepares
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "foo one\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "foo two\n").unwrap();
        std::fs::write(dir.path().join("c.py"), "foo three\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_search(true);
        // 0051: the qualifier language, not rg passthrough flags
        e.feed_text("foo -glob:\"*.py\"");
        e.wait_picker();
        let p = &e.picker.as_ref().unwrap().picker;
        assert_eq!(p.items.len(), 2, "py excluded via -glob:");
        assert!(p.items.iter().all(|i| !format!("{i:?}").contains("c.py")));
    }
}
