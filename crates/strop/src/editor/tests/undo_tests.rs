use super::*;

#[test]
fn insert_session_undoes_as_one_unit() {
    let mut e = Editor::new(Buffer::from_text("hello\n"));
    e.feed_text("A world"); // append " world" at EOL
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().text().to_string(), "hello world\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "hello\n");
    e.feed(crate::editor::Key::CtrlR);
    assert_eq!(e.buf().text().to_string(), "hello world\n");
}

#[test]
fn change_op_holds_one_undo_unit() {
    let mut e = Editor::new(Buffer::from_text("say [old] now\n"));
    e.feed_text("w"); // onto [old]
    e.feed_text("ci["); // change inside brackets
    e.feed_text("new");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().text().to_string(), "say [new] now\n");
    e.feed_text("u"); // ONE undo restores the whole change
    assert_eq!(e.buf().text().to_string(), "say [old] now\n");
}

#[test]
fn n_repeats_search_and_wraps() {
    let mut e = Editor::new(Buffer::from_text("foo bar\nfoo baz\n"));
    e.feed_text("/foo\r"); // lands on the *next* match (vim)
    assert_eq!(e.head(), 8);
    e.feed_text("n"); // wraps to the first
    assert_eq!(e.head(), 0);
    e.feed_text("N"); // backward, wraps from top
    assert_eq!(e.head(), 8);
}

#[test]
fn edit_after_undo_forks_and_ctrlr_redoes_last_branch() {
    let mut e = Editor::new(Buffer::from_text("ab\n"));
    e.feed_text("rx"); // replace a with x
    e.feed_text("u");
    e.feed_text("ry"); // fork: replace a with y
    e.feed_text("u"); // back to ab
    e.feed(crate::editor::Key::CtrlR); // redo the last-visited branch
    assert_eq!(e.buf().text().to_string(), "yb\n");
    // redo once more: nothing (the fork tip is current)
    e.feed(crate::editor::Key::CtrlR);
    assert_eq!(e.buf().text().to_string(), "yb\n");
}

#[test]
fn readonly_buffers_refuse_undo() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.buf_mut().readonly = true;
    e.feed_text("u");
    assert!(e.message.contains("readonly"));
    assert_eq!(e.buf().text().to_string(), "x\n");
}
