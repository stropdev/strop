use super::*;

#[test]
fn counts_multiply_non_operator_commands() {
    // 2x deletes two chars
    let mut e = Editor::new(Buffer::from_text("abcde\n"));
    e.feed_text("2x");
    assert_eq!(e.buf().rope.to_string(), "cde\n");
    // 2u undoes twice
    let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
    e.feed_text("dd");
    e.feed_text("dd");
    assert_eq!(e.buf().rope.to_string(), "c\n");
    e.feed_text("2u");
    assert_eq!(e.buf().rope.to_string(), "a\nb\nc\n");
    // 3rx replaces three
    let mut e = Editor::new(Buffer::from_text("abcde\n"));
    e.feed_text("3rx");
    assert_eq!(e.buf().rope.to_string(), "xxxde\n"); // 3 chars, not 4
                                                     // 2p pastes twice
    let mut e = Editor::new(Buffer::from_text("ab\n"));
    e.feed_text("yiw");
    e.feed_text("2p");
    assert_eq!(e.buf().rope.to_string(), "aababb\n"); // register twice at one spot
}

#[test]
fn counted_insert_repeats_text() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text("3iZ");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().rope.to_string(), "ZZZx\n"); // vim: i inserts before
    e.feed_text("u"); // one undo unit for the whole counted insert
    assert_eq!(e.buf().rope.to_string(), "x\n");
}

#[test]
fn cc_keeps_the_line_and_indent() {
    let mut e = Editor::new(Buffer::from_text("one\n  two\nthree\n"));
    e.feed_text("jccX");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().rope.to_string(), "one\n  X\nthree\n");
    e.feed_text("u");
    assert_eq!(e.buf().rope.to_string(), "one\n  two\nthree\n");
    // the empty case: line empties, never merges
    let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
    e.feed_text("jcc");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().rope.to_string(), "one\n\nthree\n");
}

#[test]
fn c_enters_insert_at_deletion_start() {
    // hello world: w → 6, C deletes "world", cursor stays at 6,
    // typed text lands after the space
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text("wCthere");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().rope.to_string(), "hello there\n");
    let mut e = Editor::new(Buffer::from_text("abcdef\n"));
    e.feed_text("3lCX");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().rope.to_string(), "abcX\n");
}

#[test]
fn i_caret_tilde_s_work() {
    let mut e = Editor::new(Buffer::from_text("    let x = 1;\n"));
    e.feed_text("0^"); // ^ → first non-blank (4)
    assert_eq!(e.head(), 4);
    let mut e = Editor::new(Buffer::from_text("    let x = 1;\n"));
    e.feed_text("I"); // insert at first non-blank
    assert_eq!(e.mode, crate::editor::Mode::Insert);
    assert_eq!(e.head(), 4);
    e.feed(crate::editor::Key::Esc);
    // ~ toggles case and advances
    let mut e = Editor::new(Buffer::from_text("abc\n"));
    e.feed_text("~~");
    assert_eq!(e.buf().rope.to_string(), "ABc\n");
    // S = cc
    let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
    e.feed_text("SX");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().rope.to_string(), "X\ntwo\n");
}

#[test]
fn unknown_bare_keys_say_so() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text("="); // not implemented
    assert!(e.message.contains("not an editor command"));
}
