//! Picker modal-field and navigation behavior (the input box is a
//! real modal field: Esc/i, jk on results, arrows move the caret).

#[cfg(test)]
mod picker_tests {
    use super::super::*;
    use strop_core::Buffer;

    #[test]
    fn picker_field_is_modal() {
        // rootle's input boxes: Esc enters normal mode on the field,
        // keys edit the query, i returns to insert, Esc closes
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Files);
        e.feed_text("main");
        e.feed(crate::editor::Key::Esc);
        assert!(e.picker_open(), "esc once: picker stays open");
        e.feed_text("0x"); // to 0, delete 'm'
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "ain");
        e.feed(crate::editor::Key::Esc);
        assert!(!e.picker_open(), "esc twice closes");
    }

    #[test]
    fn picker_normal_mode_jk_walk_results() {
        // Esc into the field's normal mode; j/k move the selection,
        // not the text caret (user report: only Tab/arrows navigated)
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        for f in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            std::fs::write(dir.path().join(f), "x\n").unwrap();
            e.open_buffer(&dir.path().join(f)).unwrap();
        }
        e.open_picker(Kind::Buffers);
        let sel = |e: &Editor| e.picker.as_ref().unwrap().picker.selected;
        assert_eq!(sel(&e), 0);
        e.feed(crate::editor::Key::Esc); // normal mode on the field
        e.feed_text("jj");
        assert_eq!(sel(&e), 2, "j moved the selection down twice");
        e.feed_text("k");
        assert_eq!(sel(&e), 1);
        e.feed_text("i"); // back to insert
        e.feed_text("j"); // types into the query instead
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.text,
            "j",
            "insert mode: j filters"
        );
    }

    #[test]
    fn picker_arrows_navigate_and_move_caret() {
        // user report: physical arrows did nothing in pickers — the
        // translation layer dropped KeyCode::Up/Down entirely
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        for f in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.path().join(f), "x\n").unwrap();
            e.open_buffer(&dir.path().join(f)).unwrap();
        }
        e.open_picker(Kind::Buffers);
        e.feed_text("a");
        e.feed(crate::editor::Key::Down);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.selected,
            1,
            "Down walks results"
        );
        e.feed(crate::editor::Key::Up);
        assert_eq!(e.picker.as_ref().unwrap().picker.selected, 0);
        e.feed(crate::editor::Key::Left);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.cursor,
            0,
            "Left moves the caret"
        );
        e.feed(crate::editor::Key::Right);
        assert_eq!(e.picker.as_ref().unwrap().picker.input.cursor, 1);
    }
}
