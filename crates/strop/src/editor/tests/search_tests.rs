//! Search-line contracts (0027 §1, issue 13; R5/R7): live incsearch
//! from the full saved origin, counts/registers surviving entry,
//! abort restoration of every selection and the viewport, and the
//! overlays that must NOT fire on a plain search. Real input paths; no
//! sleeps, filesystem fixtures or shell execution.

use super::*;

fn selection_pairs(e: &Editor) -> Vec<(usize, usize)> {
    std::iter::once(e.sels().primary())
        .chain(e.extra_selections().iter().copied())
        .map(|selection| (selection.anchor, selection.head))
        .collect()
}

#[test]
fn typing_backspace_and_interior_caret_edits_reresolve() {
    let mut e = Editor::new(Buffer::from_text("aa foo\nbb foobar\n"));
    e.feed_text("/fo");
    let first = e.head();
    assert_eq!(first, 3);
    e.feed_text("ob");
    assert_eq!(e.head(), 10);
    e.feed(Key::Backspace);
    assert_eq!(e.head(), first);
    e.feed_text("<esc><esc>");
    e = Editor::new(Buffer::from_text("a fx then fox\n"));
    e.feed_text("/fox<esc>hi<bs>");
    assert_eq!(e.pending.text(), "/fx");
    assert_eq!(e.head(), 2);
    e.feed_text("o");
    assert_eq!(e.pending.text(), "/fox");
    assert_eq!(e.head(), 10);
}

#[test]
fn enter_uses_the_preview_origin_for_every_counted_cursor() {
    let mut e = Editor::new(Buffer::from_text("x hit y hit z hit\n"));
    e.sels_mut().plant_extra(6);
    e.feed_text("2/hi");
    assert_eq!(e.all_cursors(), vec![8, 14]);
    e.feed(Key::Enter);
    assert_eq!(e.all_cursors(), vec![8, 14]);
    assert_eq!(e.last_search.as_ref().map(|s| s.query.source()), Some("hi"));
    assert!(!e.pending.is_active());
    e.feed(Key::CtrlO);
    assert_eq!(e.head(), 0);
}

#[test]
fn counted_registered_operator_preview_and_execution_share_the_saved_origin() {
    let text = "a hit hit hit hit hit hit z\n";
    let mut e = Editor::new(Buffer::from_text(text));
    e.feed_text("\"a2d3/hit");
    assert_eq!(e.head(), 22);
    assert_eq!(
        e.preview().unwrap().unwrap().0,
        vec![strop_core::Range::charwise(0, 22)]
    );
    e.feed(Key::Enter);
    assert_eq!(e.register(Some('a')).text, &text[..22]);
    assert_eq!(e.buf().text().to_string(), "hit z\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), text);
}

#[test]
fn double_escape_restores_all_anchors_duplicates_and_both_view_axes() {
    let text = "zero foo\none foo\ntwo\nthree\nfour\nfive far\n";
    let mut e = Editor::new(Buffer::from_text(text));
    e.sels_mut().stretch_primary(1, 2);
    e.sels_mut().toggle_extra();
    e.sels_mut().stretch_primary(9, 10);
    e.sels_mut().toggle_extra();
    e.view_mut().view_top = 1;
    e.view_mut().hscroll = strop_core::id::DisplayColumn::new(3);
    let pairs = selection_pairs(&e);
    let origin = e.view().clone();
    e.feed_text("/far");
    assert_eq!(e.head(), text.find("far").unwrap());
    e.scroll_to_cursor(2);
    assert_ne!(e.view().view_top, origin.view_top);
    e.feed(Key::Esc);
    assert!(e.input_normal());
    assert_eq!(e.head(), text.find("far").unwrap());
    e.feed(Key::Esc);
    assert_eq!(selection_pairs(&e), pairs);
    assert_eq!(e.view().view_top, origin.view_top);
    assert_eq!(e.view().hscroll, origin.hscroll);
    assert!(!e.pending.is_active());
    assert_eq!(e.buf().text().to_string(), text);
}

#[test]
fn sigil_backspace_and_normal_delete_both_cancel_and_release_input() {
    for cancel in ["<bs>", "<esc>0x"] {
        let mut e = Editor::new(Buffer::from_text("foo\n"));
        e.feed_text("/");
        e.feed_text(cancel);
        assert!(!e.pending.is_active());
        e.feed_text("x");
        assert_eq!(e.buf().text().to_string(), "oo\n");
    }
}

#[test]
fn unmatched_and_invalid_queries_do_not_edit_or_leave_jumped_cursors() {
    for query in ["zz", "["] {
        let mut e = Editor::new(Buffer::from_text("aa foo\nbb\n"));
        e.feed_text("ll");
        let origin = selection_pairs(&e);
        e.feed_text("/");
        e.feed_text(query);
        assert_eq!(selection_pairs(&e), origin);
        e.feed(Key::Enter);
        assert_eq!(selection_pairs(&e), origin);
        assert_eq!(e.buf().text().to_string(), "aa foo\nbb\n");
        assert!(!e.message.is_empty());
        if query == "[" {
            assert!(e.last_search.is_none());
        }
    }
}

#[test]
fn prompt_edit_cancel_matrix_uses_identical_modal_rules_on_all_surfaces() {
    for readonly in [false, true] {
        for visual in [false, true] {
            for open in [":", "/", "?", " |"] {
                let mut e = Editor::new(Buffer::from_text("a fx then fox\nother\n"));
                e.buf_mut().readonly = readonly;
                if visual {
                    e.feed_text("v");
                }
                let origin = selection_pairs(&e);
                e.feed_text(open);
                if readonly && open == " |" {
                    assert!(!e.pending.is_active());
                    assert!(!e.message.is_empty());
                    continue;
                }
                e.feed_text("fox<esc>hi<bs>");
                assert!(e.pending.text().ends_with("fx"));
                assert!(!e.input_normal());
                e.feed_text("o<esc>");
                assert!(e.input_normal());
                e.feed(Key::Esc);
                assert!(!e.pending.is_active());
                assert_eq!(selection_pairs(&e), origin);
                assert_eq!(e.buf().text().to_string(), "a fx then fox\nother\n");
                assert_eq!(e.mode, if visual { Mode::Visual } else { Mode::Normal });
            }
        }
    }
}

#[test]
fn prompt_accept_matrix_reaches_ex_search_and_pipe_error_effects() {
    for readonly in [false, true] {
        for visual in [false, true] {
            for (keys, expected) in [(":2<cr>", 9), ("/foo<cr>", 5), ("?foo<cr>", 5)] {
                let mut e = Editor::new(Buffer::from_text("zero foo\nsecond\n"));
                e.buf_mut().readonly = readonly;
                if visual {
                    e.feed_text("v");
                }
                e.feed_text(keys);
                assert_eq!(e.head(), expected);
                assert!(!e.pending.is_active());
                assert_eq!(e.buf().text().to_string(), "zero foo\nsecond\n");
            }
        }
    }
    for visual in [false, true] {
        let mut e = Editor::new(Buffer::from_text("a\nb\n"));
        if visual {
            e.feed_text("vj");
        }
        e.feed_text(" |<cr>");
        assert!(!e.pending.is_active());
        assert_eq!(e.mode, Mode::Normal);
        assert!(!e.message.is_empty()); // pipe_run rejected the empty command
        assert_eq!(e.buf().text().to_string(), "a\nb\n");
    }
}

#[test]
fn ex_completion_updates_the_same_caret_used_by_later_typing() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(":hel<tab>p");
    assert_eq!(e.pending.text(), ":helpp");
    e.feed_text("<bs><esc><esc>");
    assert_eq!(e.buf().text().to_string(), "x\n");
    assert!(!e.pending.is_active());
}

#[test]
fn clipboard_yank_uses_counted_operator_state_and_backspace_cancels_it() {
    let mut e = Editor::new(Buffer::from_text("hello world again\n"));
    e.feed_text("2 yw");
    assert_eq!(e.register(Some('+')).text, "hello world ");
    assert!(e.osc52.is_some());
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text(" y<bs>x");
    assert_eq!(e.buf().text().to_string(), "ello world\n");
    assert!(e.register(Some('+')).text.is_empty());
}

#[test]
fn plain_search_and_nonsearch_bodies_never_leak_structural_overlays() {
    for (keys, pattern) in [
        (" |sed s/a/b/", None),
        (":w /etc", None),
        ("/const", Some("const")),
        ("?const", Some("const")),
        ("/", None),
    ] {
        let mut e = Editor::new(Buffer::from_text("alpha const void*);\nbeta\n"));
        e.feed_text(keys);
        assert_eq!(e.search_pattern(), pattern);
        assert!(e.preview().unwrap().is_none());
        assert!(e.find_candidates().is_none());
    }
    let mut e = Editor::new(Buffer::from_text("alpha const\n"));
    e.feed_text("/const<esc><esc>df");
    assert_eq!(
        e.find_candidates(),
        Some(FindPending {
            ch: 'f',
            backward: false
        })
    );
    e.feed_text("t");
    assert!(e.find_candidates().is_none());
}

#[test]
fn backward_and_wrapping_search_keep_preview_commit_and_jump_agreeing() {
    let mut e = Editor::new(Buffer::from_text("hit then end\n"));
    e.feed_text("$/hit");
    assert_eq!(e.head(), 0);
    e.feed(Key::Enter);
    assert_eq!(e.head(), 0);
    e.feed(Key::CtrlO);
    assert_eq!(e.head(), 11);
    let mut e = Editor::new(Buffer::from_text("aa yy\nzz\n"));
    e.feed_text("G$");
    let origin = e.head();
    e.feed_text("?yy");
    assert_eq!(e.head(), 3);
    e.feed_text("<bs><bs>");
    assert_eq!(e.head(), origin);
}

#[test]
fn empty_search_reuses_query_without_losing_the_new_count_or_direction() {
    let mut e = Editor::new(Buffer::from_text("a hit hit hit end\n"));
    e.feed_text("/hit<cr>");
    assert_eq!(e.head(), 2);
    e.feed_text("2/<cr>");
    assert_eq!(e.head(), 10);
    e.feed_text("?<cr>");
    assert_eq!(e.head(), 6);
}

#[test]
fn readonly_buffers_keep_live_search_and_enter_on_the_same_path() {
    let text = "zero foo\none foobar\n";
    let mut e = Editor::new(Buffer::from_text(text));
    e.buf_mut().readonly = true;
    e.feed_text("/foob");
    assert_eq!(e.head(), 13);
    e.feed(Key::Backspace);
    assert_eq!(e.head(), 5);
    e.feed(Key::Enter);
    assert_eq!(e.head(), 5);
    e.feed_text("G$?foo<cr>");
    assert_eq!(e.head(), 13);
    assert!(!e.pending.is_active());
    assert_eq!(e.buf().text().to_string(), text);
}

#[test]
fn backward_prompt_is_present_in_the_actual_headless_frame() {
    let mut e = Editor::new(Buffer::from_text("foo\nbar\n"));
    e.feed_text("?ba");
    let frame = crate::headless::frame_string(&mut e, 40, 8).unwrap();
    assert!(
        frame.contains("? ba"),
        "the backward prompt retains its typed pattern: {frame}"
    );
}

#[test]
fn bracketed_paste_belongs_to_the_prompt_not_the_document() {
    for open in [":", "/", "?", " |"] {
        let mut e = Editor::new(Buffer::from_text("a fox\n"));
        e.feed_text(open);
        e.paste_bracketed("fox");
        assert!(e.pending.text().ends_with("fox"));
        assert_eq!(e.buf().text().to_string(), "a fox\n");
        let pending = e.pending.text().to_owned();
        e.paste_bracketed("\nx");
        assert_eq!(e.pending.text(), pending);
        assert_eq!(e.buf().text().to_string(), "a fox\n");
        assert!(!e.message.is_empty());
    }
}

#[test]
fn document_switch_cancels_before_rebinding_and_never_executes_the_old_operator() {
    let mut e = Editor::new(Buffer::from_text("a foo\n"));
    let first = e.current();
    let other = e
        .docs
        .insert(Document::scratch(Buffer::from_text("different\n")));
    e.feed_text("d/foo");
    e.switch_to(other);
    assert!(!e.pending.is_active());
    assert_eq!(e.current(), other);
    e.feed(Key::Enter);
    assert_eq!(e.doc(first).buf.text().to_string(), "a foo\n");
    assert_eq!(e.buf().text().to_string(), "different\n");
}

// ---- the accepted/stale mutation boundary (Main's apply gateway) ----

fn insertion(at: usize, text: &str) -> crate::editor::transact::ChangeSet {
    crate::editor::transact::ChangeSet {
        edits: vec![strop_core::Replacement::new(
            strop_core::Range::charwise(at, at),
            text.to_owned(),
        )],
        undo_open: false,
    }
}

#[test]
fn accepted_origin_edit_restores_before_remapping_and_consumes_pending_operator() {
    let mut e = Editor::new(Buffer::from_text("a hit\n"));
    let doc = e.current();
    e.feed_text("d/hit");
    assert_eq!(e.head(), 2);
    e.apply(doc, e.buf().revision(), insertion(0, "X")).unwrap();
    assert!(!e.pending.is_active());
    assert_eq!(
        e.head(),
        1,
        "map the origin at zero, not the incsearch hit at two"
    );
    e.feed(Key::Enter);
    assert_eq!(e.buf().text().to_string(), "Xa hit\n");
}

#[test]
fn stale_and_overlapping_batches_do_not_cancel_or_move_the_prompt() {
    let mut e = Editor::new(Buffer::from_text("a hit\n"));
    e.feed_text("d/hit");
    let doc = e.current();
    let revision = e.buf().revision();
    let wrong = strop_core::id::BufferRevision::new(revision.get() + 1);
    let origin = selection_pairs(&e);
    assert!(e.apply(doc, wrong, insertion(0, "X")).is_err());
    assert!(e.pending.is_active());
    assert_eq!(selection_pairs(&e), origin);
    let changes = crate::editor::transact::ChangeSet {
        edits: vec![
            strop_core::Replacement::new(strop_core::Range::charwise(0, 3), "left".to_owned()),
            strop_core::Replacement::new(strop_core::Range::charwise(2, 4), "right".to_owned()),
        ],
        undo_open: false,
    };
    assert!(e.apply(doc, revision, changes).is_err());
    assert!(e.pending.is_active());
    assert_eq!(selection_pairs(&e), origin);
    assert_eq!(e.buf().text().to_string(), "a hit\n");
    e.feed(Key::Enter);
    assert_eq!(e.buf().text().to_string(), "hit\n");
}

#[test]
fn unrelated_document_commit_preserves_the_live_prompt_and_focus() {
    let mut e = Editor::new(Buffer::from_text("a hit\n"));
    let doc = e.current();
    let other = e
        .docs
        .insert(Document::scratch(Buffer::from_text("other\n")));
    e.feed_text("d/hit");
    let shown = selection_pairs(&e);
    e.apply(other, e.doc(other).buf.revision(), insertion(0, "X"))
        .unwrap();
    assert_eq!(e.current(), doc);
    assert!(e.pending.is_active());
    assert_eq!(selection_pairs(&e), shown);
    assert_eq!(e.doc(other).buf.text().to_string(), "Xother\n");
    e.feed(Key::Enter);
    assert_eq!(e.buf().text().to_string(), "hit\n");
}

#[test]
fn equal_revision_and_reused_slot_cannot_accept_another_documents_prompt() {
    for reuse_slot in [false, true] {
        let mut e = Editor::new(Buffer::from_text("a hit\n"));
        let original = e.current();
        e.feed_text("d/hit");
        if reuse_slot {
            e.docs.remove(original);
        }
        let other = e
            .docs
            .insert(Document::scratch(Buffer::from_text("other\n")));
        // Simulate delivery that bypassed the normal switch hook; validity is
        // still checked at acceptance using generation-bearing DocumentId.
        e.view_mut().doc = other;
        e.feed(Key::Enter);
        assert!(!e.pending.is_active());
        assert_eq!(e.current(), other);
        assert_eq!(e.buf().text().to_string(), "other\n");
        if !reuse_slot {
            assert_eq!(e.doc(original).buf.text().to_string(), "a hit\n");
        }
    }
}

#[test]
fn pipe_prompt_owns_a_visible_input_card_on_normal_and_visual_surfaces() {
    for visual in [false, true] {
        let mut e = Editor::new(Buffer::from_text("unchanged\n"));
        if visual {
            e.feed_text("v");
        }
        e.feed_text(" |tr a-z A-Z");
        let frame = crate::headless::frame_string(&mut e, 60, 10).unwrap();
        assert!(frame.contains("pipe"), "{frame}");
        assert!(frame.contains("tr a-z A-Z"), "{frame}");
        assert_eq!(e.buf().text().to_string(), "unchanged\n");
    }
}
