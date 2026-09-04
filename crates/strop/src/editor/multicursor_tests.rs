//! Multicursor cascade tests (0013): Q toggles at point, Space c stacks
//! a cursor on the next line, edits cascade, Esc collapses, undo takes
//! the whole cascade back as one unit. Motions move every cursor
//! (nvim 0.13 semantics).

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

    fn text(e: &Editor) -> String {
        e.buf().rope.to_string()
    }

    #[test]
    fn q_toggles_at_point_and_motions_move_all() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("Q");
        assert_eq!(e.sels.heads(), vec![0, 0]);
        e.feed_text("j"); // every cursor moves (nvim 0.13)
        assert_eq!(e.head(), 4);
        assert_eq!(e.sels.heads(), vec![4, 4]);
        e.feed_text("Q"); // the extra sits on the primary → toggles off
        assert!(e
            .extra_selections()
            .iter()
            .map(|s| s.head)
            .collect::<Vec<_>>()
            .is_empty());
    }

    #[test]
    fn space_c_stacks_down_a_column() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text(" c"); // cursor below joins
        assert_eq!(e.sels.heads(), vec![0, 4]);
        e.feed_text(" c");
        assert_eq!(
            e.extra_selections()
                .iter()
                .map(|s| s.head)
                .collect::<Vec<_>>(),
            vec![4, 8]
        ); // "three" starts at 8
        e.feed_text(" c"); // no fourth line
        assert_eq!(
            e.extra_selections()
                .iter()
                .map(|s| s.head)
                .collect::<Vec<_>>(),
            vec![4, 8]
        );
    }

    #[test]
    fn delete_cascades_and_undoes_as_one_unit() {
        let mut e = Editor::new(Buffer::from_text("foo a\nfoo b\n"));
        e.feed_text(" c"); // cursors on both "foo"s
        e.feed_text("dw");
        assert_eq!(text(&e), "a\nb\n");
        e.feed_text("u"); // ONE undo for the whole cascade
        assert_eq!(text(&e), "foo a\nfoo b\n");
    }

    #[test]
    fn change_flows_into_mirrored_insert() {
        let mut e = Editor::new(Buffer::from_text("x = 1\ny = 2\n"));
        e.feed_text(" c");
        e.feed_text("cw"); // change both first words
        e.feed_text("val");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(text(&e), "val = 1\nval = 2\n");
        e.feed_text("u"); // change + insert = one unit
        assert_eq!(text(&e), "x = 1\ny = 2\n");
    }

    #[test]
    fn esc_collapses_to_primary() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
        e.feed_text(" c");
        e.feed_text(" c");
        assert_eq!((e.sels.count() - 1), 2);
        e.feed(crate::editor::Key::Esc);
        assert!(e
            .extra_selections()
            .iter()
            .map(|s| s.head)
            .collect::<Vec<_>>()
            .is_empty());
    }

    #[test]
    fn stacked_cursors_edit_once() {
        let mut e = Editor::new(Buffer::from_text("word here\n"));
        e.feed_text("Q"); // extra stacked on the primary at 0
        e.feed_text("dw"); // one delete, not two
        assert_eq!(text(&e), "here\n");
        e.feed_text("u");
        e.feed_text("i"); // insert mirrors once, not twice
        e.feed_text("Z");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(text(&e), "Zword here\n");
    }

    #[test]
    fn paste_cascades_to_every_cursor() {
        let mut e = Editor::new(Buffer::from_text("aa\nbb\n"));
        e.feed_text("yiw"); // yank "aa"
        e.feed_text(" c"); // cursor joins on line 2
        e.feed_text("e"); // word end on both
        e.feed_text("p"); // paste "aa" after each cursor (vim: no space magic)
        assert_eq!(text(&e), "aaaa\nbbaa\n");
    }

    #[test]
    fn n_cascades_to_every_cursor() {
        let mut e = Editor::new(Buffer::from_text("1 foo\n2 foo\n3 foo\n"));
        e.feed_text(" c"); // extra on line 2
        e.feed_text("/foo\r");
        assert_eq!(e.head(), 2);
        assert_eq!(
            e.extra_selections()
                .iter()
                .map(|s| s.head)
                .collect::<Vec<_>>(),
            vec![8]
        ); // foo on line 2 starts at 8
    }

    #[test]
    fn extra_cursors_render_without_panicking() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text(" c");
        let frame = crate::headless::frame_string(&mut e, 40, 10);
        assert!(frame.contains("one") && frame.contains("two"));
    }

    #[test]
    fn key_soup_never_panics() {
        // seeded LCG drives thousands of keystrokes across buffer shapes:
        // cursors, pickers, surfaces, undo — every path must stay total
        let keys = "hjklwbe0$GwWbBeEdyc><iIoOaAvVspPxXuQq ?%fFtT/\",.:;[]{}()m'rcnNZSL=+-_!@#^&*|~123456789 ";
        let shapes = ["", "x\n", "fn main() {\n    let x = 1;\n}\n", "a\nb\nc\n"];
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for (round, shape) in shapes.iter().enumerate() {
            let mut e = Editor::new(Buffer::from_text(shape));
            for _ in 0..3000 {
                let c = keys.as_bytes()[(next() as usize) % keys.len()] as char;
                e.feed(crate::editor::Key::Char(c));
                if round % 3 == 0 && next() % 7 == 0 {
                    e.feed(crate::editor::Key::Esc);
                }
                if next() % 11 == 0 {
                    e.drain_picker();
                    e.drain_git_jobs();
                    e.drain_lsp();
                    e.drain_clipboard();
                }
            }
            // a frame render must never panic either (cursor invariants)
            let _ = crate::headless::frame_string(&mut e, 80, 24);
        }
    }
}
