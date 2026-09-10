use super::*;

#[test]
fn undo_after_visual_delete() {
    let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
    e.feed_text("ved"); // vim: deletes "say", the space stays
    assert_eq!(e.buf().text().to_string(), " \"hi\" now\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "say \"hi\" now\n");
}

#[test]
fn quote_object_scans_forward_on_the_line() {
    // vim i" special case: cursor before the string uses the next pair
    let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
    e.feed_text("vi\"");
    let r = e.visual_range().expect("selection");
    assert_eq!(e.buf().slice_string(r), "hi");
}

#[test]
fn quit_leaves_editor_drain_safe() {
    // regression: the last :q! emptied the buffer list and the
    // post-feed drain tick panicked indexing buffers[current]
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(":q!\r");
    assert!(e.should_quit);
    assert!(e.docs.is_empty());
    e.drain_picker();
    e.drain_git_jobs();
    e.drain_lsp();
    e.lsp_sync_changed();
}

#[test]
fn undo_lands_cursor_at_change_start() {
    // regression: undo took the first replayed op (the tail of the
    // change) — vim lands at the start of the undone region
    let mut e = Editor::new(Buffer::from_text("hello\n"));
    e.feed_text("A world");
    e.feed(crate::editor::Key::Esc);
    e.feed_text("0"); // move away from the change
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "hello\n");
    // the change started at byte 5 (" world"); normal-mode clamp
    // pulls 5 onto the last char of the line
    assert_eq!(e.head(), 4);
}
