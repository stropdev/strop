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
    e.feed(crate::editor::Key::Esc);
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("scratch-test.rs");
    std::fs::write(&f, "fn a() {}\n").unwrap();
    e.open_fixture(&f).unwrap();
    assert_eq!(e.docs.len(), 2, "edited scratch is real work");
}
