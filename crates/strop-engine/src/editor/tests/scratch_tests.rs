use super::*;

#[test]
fn first_open_replaces_the_scratch_buffer() {
    // regression: opening over the initial scratch left it behind —
    // :q closed the file, the welcome card showed, you kept quitting
    // tempdir per test: parallel tests sharing one fixture race
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("scratch-test.rs");
    std::fs::write(&f, "fn a() {}\n").unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.open_fixture(&f).unwrap();
    assert_eq!(e.docs.len(), 1, "scratch replaced, not stacked");
    assert_eq!(e.buf().path.as_deref(), Some(f.as_path()));
    e.feed_text(":q\r");

    assert!(e.should_quit, "one :q quits");
}

#[test]
fn view_marks_readonly_and_edits_refuse() {
    let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
    e.feed_text(":view\r");
    assert!(e.buf().readonly);
    e.feed_text("x");
    assert_eq!(e.buf().text().to_string(), "one\ntwo\n", "no edit landed");
    assert!(e.message.contains("readonly"));
}

#[test]
fn edited_scratch_survives() {
    let mut e = Editor::new(Buffer::from_text(""));
    e.feed_text("ix"); // scratch has content now
    e.feed(strop_core::frontend_input::Key::Esc);
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("scratch-test.rs");
    std::fs::write(&f, "fn a() {}\n").unwrap();
    e.open_fixture(&f).unwrap();
    assert_eq!(e.docs.len(), 2, "edited scratch is real work");
}

/// `tail -f log | strop -`: stdin streams into a read-only buffer that
/// never dirties or records undo; the cursor on the last line follows;
/// EOF makes it an ordinary editable scratch buffer.
#[test]
fn stdin_streams_into_a_following_buffer_until_eof() {
    use crate::editor::events::AppEvent;
    let mut e = Editor::new(Buffer::from_text(""));
    e.adopt_stdin();
    assert!(e.stdin_streaming());
    assert_eq!(e.buf().name.as_deref(), Some("stdin"));
    e.handle_app_event(AppEvent::Stdin(Some("one\ntw".into())));
    e.handle_app_event(AppEvent::Stdin(Some("o\nthree\n".into())));
    assert_eq!(e.buf().text().to_string(), "one\ntwo\nthree\n");
    assert_eq!(e.buf().line_of(e.head()), 2, "the cursor follows the tail");
    assert!(!e.buf().is_dirty());
    assert!(
        !e.buf().history().can_undo(),
        "stdin is not an edit to undo"
    );
    e.feed_text("x");
    assert_eq!(
        e.buf().text().to_string(),
        "one\ntwo\nthree\n",
        "read-only while streaming"
    );
    e.feed_text("gg");
    e.handle_app_event(AppEvent::Stdin(Some("four\n".into())));
    assert_eq!(
        e.buf().line_of(e.head()),
        0,
        "a cursor that left the tail stays put"
    );
    e.handle_app_event(AppEvent::Stdin(None));
    assert!(!e.stdin_streaming());
    assert!(!e.buf().readonly);
    assert!(e.message().contains("4 lines"), "{}", e.message());
    e.feed_text("x");
    assert_eq!(e.buf().text().to_string(), "ne\ntwo\nthree\nfour\n");
}
