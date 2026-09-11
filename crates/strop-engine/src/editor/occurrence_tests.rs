//! Occurrence selection tests (0049 §7): seed-by-word, add-next with
//! wrap-once, select-all, skip, pop, literal visual seeds, real
//! selections through the operator cascade, Esc clearing.

#[cfg(test)]
mod tests {
    use crate::editor::{Editor, Key, Mode};
    use strop_core::Buffer;

    fn text(e: &Editor) -> String {
        e.buf().text().to_string()
    }

    /// (anchor, head) of the primary, then every extra in order.
    fn spans(e: &Editor) -> Vec<(usize, usize)> {
        std::iter::once(e.sels().primary())
            .chain(e.extra_selections().iter().copied())
            .map(|s| (s.anchor, s.head))
            .collect()
    }

    #[test]
    fn seed_selects_the_word_under_the_caret() {
        let mut e = Editor::new(Buffer::from_text("alpha beta alpha\n"));
        e.set_head(7); // on "beta" = [6, 10)
        e.occurrence_next_pub();
        assert_eq!(e.mode, Mode::Visual);
        assert_eq!((e.anchor(), e.head()), (6, 9));
        assert_eq!(e.message, "1 occurrence of \"beta\"");
    }

    #[test]
    fn caret_on_whitespace_seeds_nothing() {
        let mut e = Editor::new(Buffer::from_text("foo  bar\n"));
        e.set_head(4); // between the words
        e.occurrence_next_pub();
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(e.message, "no word under cursor");
        assert!(e.occurrence.is_none());
        assert_eq!(e.sels().count(), 1);
    }

    #[test]
    fn add_next_walks_document_order_and_wraps_once() {
        // foo at [0,3), [8,11), [16,19)
        let mut e = Editor::new(Buffer::from_text("foo bar foo baz foo\n"));
        e.set_head(9);
        e.occurrence_next_pub();
        assert_eq!(spans(&e), vec![(8, 10)]);
        e.occurrence_next_pub(); // forward first
        assert_eq!(spans(&e), vec![(8, 10), (16, 18)]);
        assert_eq!(e.message, "2 occurrences of \"foo\"");
        e.occurrence_next_pub(); // wraps to the top, once
        assert_eq!(spans(&e), vec![(8, 10), (0, 2), (16, 18)]);
        assert_eq!(e.message, "3 occurrences of \"foo\"");
        e.occurrence_next_pub(); // exhausted: no duplicates cycled in
        assert_eq!(spans(&e), vec![(8, 10), (0, 2), (16, 18)]);
        assert_eq!(e.message, "no more occurrences of \"foo\"");
    }

    #[test]
    fn select_all_covers_every_match() {
        let mut e = Editor::new(Buffer::from_text("foo bar foo baz foo\n"));
        e.set_head(1);
        e.occurrence_all_pub();
        assert_eq!(e.mode, Mode::Visual);
        assert_eq!(spans(&e), vec![(0, 2), (8, 10), (16, 18)]);
        assert_eq!(e.message, "3 occurrences of \"foo\"");
        // everything is selected: the next add reports exhaustion
        e.occurrence_next_pub();
        assert_eq!(e.message, "no more occurrences of \"foo\"");
    }

    #[test]
    fn skip_passes_a_candidate_without_selecting_it() {
        // foo at [0,3), [4,7), [8,11)
        let mut e = Editor::new(Buffer::from_text("foo foo foo\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_skip_pub();
        assert_eq!(spans(&e), vec![(0, 2)], "skipped match stays unselected");
        assert_eq!(e.message, "occurrence skipped (1 selected)");
        e.occurrence_next_pub(); // the one after the skipped candidate
        assert_eq!(spans(&e), vec![(0, 2), (8, 10)]);
        e.occurrence_next_pub(); // the skipped one is never re-offered
        assert_eq!(e.message, "no more occurrences of \"foo\"");
        assert_eq!(spans(&e), vec![(0, 2), (8, 10)]);
    }

    #[test]
    fn skip_then_select_all_keeps_the_skip() {
        let mut e = Editor::new(Buffer::from_text("foo foo foo\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_skip_pub();
        e.occurrence_all_pub();
        assert_eq!(spans(&e), vec![(0, 2), (8, 10)]);
        assert_eq!(e.message, "2 occurrences of \"foo\"");
    }

    #[test]
    fn pop_removes_last_added_then_ends_the_session() {
        let mut e = Editor::new(Buffer::from_text("foo foo foo\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        assert_eq!(spans(&e), vec![(0, 2), (4, 6), (8, 10)]);
        e.occurrence_pop_pub();
        assert_eq!(spans(&e), vec![(0, 2), (4, 6)]);
        assert_eq!(e.message, "2 occurrences of \"foo\"");
        e.occurrence_pop_pub();
        assert_eq!(spans(&e), vec![(0, 2)]);
        e.occurrence_pop_pub(); // popping the seed ends the session
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(spans(&e), vec![(0, 0)]);
        assert_eq!(e.message, "occurrence selection cleared");
        assert!(e.occurrence.is_none());
        e.occurrence_pop_pub();
        assert_eq!(e.message, "no occurrence selection");
    }

    #[test]
    fn visual_selection_seeds_literal_text_not_a_pattern() {
        // `.` in the needle must NOT match "axb"
        let mut e = Editor::new(Buffer::from_text("a.b axb a.b\n"));
        e.set_head(0);
        e.feed_text("vll"); // select "a.b"
        e.occurrence_next_pub();
        assert_eq!(e.message, "1 occurrence of \"a.b\"");
        e.occurrence_next_pub();
        // only the other literal "a.b" — "axb" at [4,7) is no match
        assert_eq!(spans(&e), vec![(0, 2), (8, 10)]);
        e.occurrence_next_pub();
        assert_eq!(e.message, "no more occurrences of \"a.b\"");
    }

    #[test]
    fn unicode_seeds_stay_on_char_boundaries() {
        // é = [0,2), second é = [7,9)
        let mut e = Editor::new(Buffer::from_text("é foo é\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        assert_eq!(spans(&e), vec![(0, 0)]); // head clamps off the continuation byte
        e.occurrence_next_pub();
        assert_eq!(spans(&e), vec![(0, 0), (7, 7)]);
        assert_eq!(e.message, "2 occurrences of \"é\"");
    }

    #[test]
    fn motions_move_heads_and_keep_anchors() {
        // foo at [0,3) and [8,11); head 10 has room to walk right
        let mut e = Editor::new(Buffer::from_text("foo bar foo x\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        assert_eq!(spans(&e), vec![(0, 2), (8, 10)]);
        e.feed_text("l"); // visual motion: heads walk, anchors sit
        assert_eq!(spans(&e), vec![(0, 3), (8, 11)]);
    }

    #[test]
    fn change_cascades_and_undoes_as_one_group() {
        let mut e = Editor::new(Buffer::from_text("foo a\nfoo b\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        assert_eq!(spans(&e), vec![(0, 2), (6, 8)]);
        e.feed_text("c");
        assert_eq!(e.mode, Mode::Insert);
        e.feed_text("bar");
        e.feed(Key::Esc);
        assert_eq!(text(&e), "bar a\nbar b\n");
        e.feed_text("u"); // ONE undo for delete + mirrored insert
        assert_eq!(text(&e), "foo a\nfoo b\n");
    }

    #[test]
    fn delete_cascades_across_occurrences() {
        let mut e = Editor::new(Buffer::from_text("foo a\nfoo b\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        e.feed_text("d");
        assert_eq!(text(&e), " a\n b\n");
        assert_eq!(e.mode, Mode::Normal);
        e.feed_text("u");
        assert_eq!(text(&e), "foo a\nfoo b\n");
    }

    #[test]
    fn yank_cascades_and_keeps_the_selections() {
        let mut e = Editor::new(Buffer::from_text("foo a\nfoo b\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        e.feed_text("y");
        assert_eq!(text(&e), "foo a\nfoo b\n");
        assert_eq!(e.registers[&'"'].text, "foo\nfoo"); // helix join rule
        assert_eq!(e.mode, Mode::Visual, "yank keeps the selections");
        assert_eq!(spans(&e), vec![(0, 2), (6, 8)]);
    }

    #[test]
    fn esc_finishes_visual_then_normal_esc_clears_the_session() {
        let mut e = Editor::new(Buffer::from_text("foo a\nfoo b\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.occurrence_next_pub();
        e.feed(Key::Esc); // finish Visual first — selections survive
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(e.sels().count(), 2);
        e.feed(Key::Esc); // Normal Esc clears extras + occurrence state
        assert_eq!(e.sels().count(), 1);
        assert_eq!(spans(&e), vec![(2, 2)]); // the caret stays where it was
        assert_eq!(e.message, "1 cursor");
    }

    #[test]
    fn edits_make_the_session_reseed_instead_of_acting_on_ghosts() {
        let mut e = Editor::new(Buffer::from_text("foo foo\n"));
        e.set_head(0);
        e.occurrence_next_pub();
        e.feed_text("x"); // visual delete consumes the selection
        assert_eq!(text(&e), " foo\n");
        assert_eq!(e.mode, Mode::Normal);
        // the session's ranges are gone: gb seeds fresh from the caret
        // (the delete landed it on the space before the second "foo")
        e.set_head(1);
        e.occurrence_next_pub();
        assert_eq!(e.message, "1 occurrence of \"foo\"");
        assert_eq!(spans(&e), vec![(1, 3)]);
    }
}
