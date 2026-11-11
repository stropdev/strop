//! Plan 0095's keys, where the differential harness cannot see: register
//! contents by name, modes mid-command, messages, prompts, marks that
//! are gone.

use super::*;
use strop_core::frontend_input::Key;

fn editor_with(text: &str) -> Editor {
    Editor::new(Buffer::from_text(text))
}

fn text(e: &Editor) -> String {
    e.buf().text().to_string()
}

#[test]
fn visual_p_swaps_the_selection_with_the_register() {
    let mut e = editor_with("one two three\n");
    e.feed_text("yiwwviwp");
    assert_eq!(text(&e), "one one three\n");
    // the replaced text is now the unnamed register; the yank survives in "0
    assert_eq!(e.register(None).text, "two");
    assert_eq!(e.register(Some('0')).text, "one");
    assert_eq!(e.register(Some('-')).text, "two");
    // so a second `p` puts back what the first one replaced
    e.feed_text("wviwp");
    assert_eq!(text(&e), "one one two\n");
    assert_eq!(e.register(None).text, "three");
}

#[test]
fn visual_capital_p_leaves_every_register_alone() {
    let mut e = editor_with("one two three\n");
    e.feed_text("yiwwviwP");
    assert_eq!(text(&e), "one one three\n");
    assert_eq!(e.register(None).text, "one");
    assert_eq!(e.register(Some('-')).text, "");
    e.feed_text("wviwP");
    assert_eq!(text(&e), "one one one\n");
    assert_eq!(e.mode, Mode::Normal);
}

#[test]
fn visual_p_from_a_named_register_still_fills_the_unnamed_one() {
    let mut e = editor_with("one two\n");
    e.feed_text("\"ayiwwviw\"ap");
    assert_eq!(text(&e), "one one\n");
    assert_eq!(e.register(Some('a')).text, "one");
    assert_eq!(e.register(None).text, "two");
}

#[test]
fn a_replaced_line_goes_to_the_numbered_registers() {
    let mut e = editor_with("one\ntwo\n");
    e.feed_text("yiwjVp");
    assert_eq!(text(&e), "one\none\n");
    assert_eq!(e.register(Some('1')).text, "two\n");
    assert_eq!(e.register(None).text, "two\n");
}

#[test]
fn replace_mode_backspace_gives_back_what_was_typed_over() {
    let mut e = editor_with("abcdef\n");
    e.feed_text("lRXYZ");
    assert_eq!(text(&e), "aXYZef\n");
    assert_eq!(e.mode, Mode::Insert);
    e.feed(Key::Backspace);
    e.feed(Key::Backspace);
    assert_eq!(text(&e), "aXcdef\n");
    e.feed(Key::Backspace);
    e.feed(Key::Backspace);
    assert_eq!(text(&e), "abcdef\n", "past the typed text it only moves");
    e.feed(Key::Esc);
    assert_eq!(e.mode, Mode::Normal);
    // an ordinary Insert afterwards inserts again
    e.feed_text("iQ");
    e.feed(Key::Esc);
    assert_eq!(text(&e), "Qabcdef\n");
}

#[test]
fn ctrl_o_runs_one_command_and_returns_to_insert() {
    let mut e = editor_with("alpha beta\n");
    e.feed_text("A");
    e.feed(Key::CtrlO);
    assert_eq!(e.mode, Mode::Normal);
    e.feed_text("d");
    assert_eq!(e.mode, Mode::Normal, "the command is not complete yet");
    e.feed_text("b");
    assert_eq!(e.mode, Mode::Insert);
    assert_eq!(text(&e), "alpha a\n");
    e.feed_text("X");
    e.feed(Key::Esc);
    assert_eq!(text(&e), "alpha aX\n");
}

#[test]
fn a_mark_on_a_deleted_line_is_not_set_any_more() {
    let mut e = editor_with("one\ntwo\nthree\n");
    e.feed_text("jmadd");
    e.feed_text("'a");
    assert_eq!(e.message, "mark a not set");
    e.feed_text("d'a");
    assert_eq!(text(&e), "one\nthree\n");
    assert_eq!(e.message, "mark a not set");
}

#[test]
fn a_mark_follows_its_text() {
    let mut e = editor_with("one\ntwo\n");
    e.feed_text("jlmaggOzero");
    e.feed(Key::Esc);
    e.feed_text("`a");
    assert_eq!(e.buf().line_of(e.head()), 2);
    assert_eq!(e.buf().col_of(e.head()), 1);
}

#[test]
fn the_special_marks_are_read_by_operators() {
    let mut e = editor_with("one\ntwo\nthree\nfour\n");
    e.feed_text("jVj");
    e.feed(Key::Esc);
    e.feed_text("Gd'<");
    assert_eq!(
        text(&e),
        "one\n",
        "from the selection's first line to the cursor"
    );
}

#[test]
fn the_filter_operator_opens_the_command_line_with_its_range() {
    let mut e = editor_with("b\na\nc\n");
    e.feed_text("!j");
    assert_eq!(e.pending.text(), ":.,.+1!");
    e.feed(Key::Esc);
    e.feed(Key::Esc);
    e.feed_text("!!");
    assert_eq!(e.pending.text(), ":.!");
    e.feed(Key::Esc);
    e.feed(Key::Esc);
    e.feed_text("Vj!");
    assert_eq!(e.pending.text(), ":'<,'>!");
    assert_eq!(e.mode, Mode::Normal);
}

#[test]
fn an_operator_strop_lacks_takes_its_motion_and_says_so() {
    for keys in ["=ip", "gqip", "gwip", "zfip", "=="] {
        let mut e = editor_with("  one\n  two\n");
        e.feed_text(keys);
        assert_eq!(text(&e), "  one\n  two\n", "{keys}");
        assert_eq!(
            e.mode,
            Mode::Normal,
            "{keys}: `ip` must not be typed as i then p"
        );
        assert!(
            e.message.contains("is not supported"),
            "{keys}: {}",
            e.message
        );
        assert!(e.walker.is_ground());
    }
}

#[test]
fn the_alternate_file_register() {
    let mut e = editor_with("one\n");
    assert!(matches!(
        e.read_register(Some('#')),
        Ok(crate::editor::registers::RegisterRead::Text(register)) if register.text.is_empty()
    ));
    e.feed_text("\"#yy");
    assert!(
        e.message.contains('#'),
        "the register is read-only: {}",
        e.message
    );
}

#[test]
fn a_prompt_field_writes_registers() {
    let mut e = editor_with("one\n");
    e.feed_text("/alpha beta");
    e.feed(Key::Esc);
    e.feed_text("0wyiw");
    assert_eq!(e.register(None).text, "alpha");
    assert_eq!(e.register(Some('0')).text, "alpha");
    e.feed_text("wdiw");
    assert_eq!(e.register(None).text, "beta");
    assert_eq!(e.register(Some('-')).text, "beta");
    assert_eq!(
        e.register(Some('0')).text,
        "alpha",
        "a delete leaves the yank"
    );
}

#[test]
fn ctrl_e_and_ctrl_y_scroll_without_moving_the_cursor_off_its_line() {
    let lines: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    let mut e = editor_with(&lines);
    e.scroll_to_cursor(10);
    e.feed_text("5G");
    let top = e.view_top();
    e.feed(Key::CtrlE);
    assert_eq!(e.view_top(), top + 1);
    assert_eq!(e.buf().line_of(e.head()), 4);
    e.feed(Key::CtrlY);
    assert_eq!(e.view_top(), top);
    assert_eq!(text(&e), lines, "they are not `e` and `y`");
}

#[test]
fn q_records_only_into_a_register() {
    let mut e = editor_with("one\n");
    e.feed_text("q:");
    assert!(e.recording.is_none());
    assert!(e.message.contains("not a register"));
}
