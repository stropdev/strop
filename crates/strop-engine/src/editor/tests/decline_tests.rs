//! Leader verbs that decline say why (0088 U17): nothing opening is never
//! silent — the echo row carries the reason.
use super::*;
use std::path::PathBuf;

#[test]
fn undo_tree_on_an_empty_history_says_so() {
    let mut e = Editor::new(Buffer::from_text("alpha\n"));
    e.feed_text(" u");
    assert_eq!(e.message(), "undo tree: undo history is empty");
    assert!(e.undo_browser.is_none());
}

#[test]
fn each_x_is_its_own_undo_step_and_feeds_the_undo_tree() {
    // regression: `x` left its undo group open, so `xxu` restored both
    // characters and `Space u` reported an empty history after edits.
    let mut e = Editor::new(Buffer::from_text("abcdef\n"));
    e.feed_text("xx");
    assert_eq!(e.buf().text(), "cdef\n");
    e.feed_text("u");
    assert_eq!(e.buf().text(), "bcdef\n", "vim: one x per undo");
    e.feed_text(" u");
    assert!(e.undo_browser.is_some(), "{}", e.message());
    assert_eq!(e.buf().name.as_deref(), Some("undo tree"));
}

#[test]
fn symbols_and_hover_without_a_language_server_name_the_verb_and_the_cause() {
    for (keys, verb) in [(" s", "document symbols"), (" k", "hover")] {
        let mut e = Editor::new_in(Buffer::from_text("int main() {}\n"), PathBuf::from("/proj"));
        e.buf_mut().path = Some(PathBuf::from("/proj/lonely.cpp"));
        e.feed_text(keys);
        assert!(e.picker.is_none());
        assert!(
            e.message().starts_with(&format!("{verb}: "))
                && e.message().contains("no language server"),
            "{keys:?}: {}",
            e.message()
        );
    }
}

#[test]
fn symbols_in_a_buffer_without_a_language_says_so() {
    let mut e = Editor::new(Buffer::from_text("plain\n"));
    e.feed_text(" s");
    assert!(e.picker.is_none());
    assert!(
        e.message().starts_with("document symbols: "),
        "{}",
        e.message()
    );
}

#[test]
fn git_log_on_a_dirty_buffer_opens_the_log() {
    let (_dir, mut e) = crate::editor::test_support::git::fixture();
    e.feed_text("x");
    assert!(e.buf().dirty);
    e.feed_text(" gl");
    assert!(matches!(e.surface(), Some(Surface::CommitLog { .. })));
}

#[test]
fn git_log_outside_a_repository_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new_in(Buffer::from_text("plain\n"), dir.path().to_path_buf());
    e.feed_text(" gl");
    assert!(e.surface().is_none());
    assert_eq!(e.message(), "not a git repo");
}
