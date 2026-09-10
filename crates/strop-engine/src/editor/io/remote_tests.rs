//! Remote content is a real buffer; stale service deliveries cannot take a view.
use super::*;
use crate::editor::Key;

use crate::editor::test_support::remote::{deliver, io_editor as editor, open, ticket};
use strop_workspace::RemoteFile;

#[test]
fn escaped_and_superseded_opens_cannot_replace_the_current_buffer() {
    let mut editor = editor("origin\n");
    editor.feed_text(":e ssh://fixture/first.log<cr>");
    let escaped = ticket(&editor);
    editor.feed(Key::Esc);
    deliver(&mut editor, escaped, "stale\n");
    assert_eq!(editor.buf().text(), "origin\n");
    editor.feed_text(":e ssh://fixture/first.log<cr>");
    let superseded = ticket(&editor);
    editor.feed_text(":e ssh://fixture/second.log<cr>");
    let newest = ticket(&editor);
    deliver(&mut editor, superseded, "stale again\n");
    assert_eq!(editor.buf().text(), "origin\n");
    deliver(&mut editor, newest, "current\n");
    assert_eq!(editor.buf().text(), "current\n");
    assert_eq!(
        editor.remote_file(),
        Some(&RemoteFile::parse("ssh://fixture/second.log").unwrap())
    );
}

#[test]
fn moving_focus_cancels_remote_navigation() {
    let mut editor = editor("origin\n");
    editor.feed_text(":e ssh://fixture/slow.log<cr>");
    let stale = ticket(&editor);
    editor.split_document(true, editor.current());
    deliver(&mut editor, stale, "wrong pane\n");
    assert_eq!(editor.panes.len(), 2);
    assert_eq!(editor.buf().text(), "origin\n");
    assert!(!editor.io_pending());
}

#[test]
fn refresh_preserves_split_positions_and_failure_keeps_old_text() {
    let mut editor = editor("");
    open(&mut editor, "first\nneedle\nlast\n");
    editor.feed_text("j");
    editor.split_document(true, editor.current());
    editor.feed_text("j:e!<cr>");
    let failed = ticket(&editor);
    editor.handle_io(IoEvent::Open(Box::new(Completion {
        ticket: failed,
        outcome: Outcome::failed(FailureKind::Io, "read denied"),
    })));
    assert_eq!(editor.buf().text(), "first\nneedle\nlast\n");
    editor.feed_text(":e!<cr>");
    let refresh = ticket(&editor);
    deliver(
        &mut editor,
        refresh,
        "new first\nneedle changed\nnew last\nmore\n",
    );
    assert_eq!(editor.buf().line_of(editor.head()), 2);
    assert_eq!(editor.buf().line_of(editor.panes[0].sels.primary().head), 1);
    assert!(editor.panes.iter().all(|pane| pane.doc == editor.current()));
}

#[test]
fn closing_the_only_remote_snapshot_quits_without_a_dead_current_document() {
    let mut editor = editor("");
    open(&mut editor, "log\n");
    assert_eq!(editor.docs.len(), 1);
    editor.feed_text("q");
    assert!(editor.should_quit);
    assert!(editor.docs.is_empty());
}
