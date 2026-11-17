//! History: what goes in, how `Up`/`Down` and `Ctrl-P`/`Ctrl-N` walk it.

use super::super::history::{History, CAP};
use super::editor;

fn lines(history: &History) -> Vec<&str> {
    history.iter().collect()
}

#[test]
fn an_equal_line_moves_to_the_newest_place_and_the_list_is_capped() {
    let mut history = History::default();
    for line in ["a", "b", "a", ""] {
        history.push(line);
    }
    assert_eq!(lines(&history), ["b", "a"]);
    for n in 0..CAP + 5 {
        history.push(&n.to_string());
    }
    assert_eq!(history.len(), CAP);
    assert_eq!(history.get(0), Some("5"));
}

#[test]
fn up_recalls_lines_that_start_with_what_was_typed_and_down_restores_it() {
    let mut e = editor("one\ntwo\nthree\nfour\n");
    e.feed_text(":1d<cr>:2y<cr>:1y<cr>");
    e.feed_text(":1<up>");
    assert_eq!(e.cmdline.text(), ":1y");
    e.feed_text("<up>");
    assert_eq!(e.cmdline.text(), ":1d");
    e.feed_text("<up>");
    assert_eq!(e.cmdline.text(), ":1d", "no older line: it stays");
    e.feed_text("<down><down>");
    assert_eq!(e.cmdline.text(), ":1", "past the newest: what was typed");
    // the filter is the text before the cursor when the walk began
    e.feed_text("<esc>:1y<left><up>");
    assert_eq!(e.cmdline.text(), ":1y");
    e.feed_text("<up>");
    assert_eq!(e.cmdline.text(), ":1d");
}

#[test]
fn ctrl_p_and_ctrl_n_walk_every_line() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text(":1y<cr>:2y<cr>:3<c-p>");
    assert_eq!(e.cmdline.text(), ":2y");
    e.feed_text("<c-p>");
    assert_eq!(e.cmdline.text(), ":1y");
    e.feed_text("<c-n><c-n>");
    assert_eq!(e.cmdline.text(), ":3");
}

#[test]
fn typing_ends_a_walk_and_the_next_one_filters_by_the_new_text() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text(":1y<cr>:2y<cr>:2d<cr>:<up>");
    assert_eq!(e.cmdline.text(), ":2d");
    e.feed_text("<bs>y<up>");
    assert_eq!(e.cmdline.text(), ":2y");
}

#[test]
fn failed_lines_are_kept_and_search_has_its_own_list() {
    let mut e = editor("one\ntwo\n");
    e.feed_text(":frob<cr>/two<cr>");
    assert_eq!(lines(&e.cmdline.history.ex), ["frob"]);
    assert_eq!(lines(&e.cmdline.history.search), ["two"]);
    e.feed_text("/<up>");
    assert_eq!(e.cmdline.text(), "/two");
    e.feed_text("<esc>:<up>");
    assert_eq!(e.cmdline.text(), ":frob");
}

#[test]
fn lines_run_by_a_macro_or_by_global_and_normal_are_not_kept() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text("qa:1y<cr>q");
    assert_eq!(lines(&e.cmdline.history.ex), ["1y"]);
    e.feed_text(":2y<cr>@a");
    assert_eq!(lines(&e.cmdline.history.ex), ["1y", "2y"], "replayed lines");
    e.feed_text(":g/o/y<cr>");
    assert_eq!(lines(&e.cmdline.history.ex), ["1y", "2y", "g/o/y"]);
}

#[test]
fn the_colon_register_is_the_newest_line_that_is_not_a_repeat() {
    let mut e = editor("one\ntwo\nthree\nfour\n");
    e.feed_text(":1d<cr>:@:<cr>");
    assert_eq!(e.buf().text().to_string(), "three\nfour\n");
    assert_eq!(e.register(Some(':')).text, "1d");
    e.feed_text("@:");
    assert_eq!(e.buf().text().to_string(), "four\n");
}

#[test]
fn a_pipe_line_has_no_history() {
    let mut e = editor("one\n");
    e.feed_text(":1y<cr>");
    e.feed_text(" |<up>");
    assert_eq!(e.cmdline.text(), "|");
}
