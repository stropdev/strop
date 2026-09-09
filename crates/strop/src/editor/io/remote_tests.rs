//! Remote content is a real buffer; stale service deliveries cannot take a view.
use super::*;
use crate::editor::{document::DocumentSource, Key};
use std::rc::Rc;
use strop_remote::RemoteFile;
use strop_trace::replay::Tape;

fn editor(text: &str) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(text), "/isolated".into());
    editor.tape = Rc::new(Tape::fixture(|operation, _| match operation {
        "analysis.start" => Ok(serde_json::json!({"Ok": null})),
        _ => Err(std::io::Error::other("native work forbidden")),
    }));
    editor
}
fn ticket(editor: &Editor) -> Ticket<OpenKey> {
    let (&request, key) = editor.io.open.iter().next().unwrap();
    Ticket {
        request,
        key: key.clone(),
    }
}
fn deliver(editor: &mut Editor, ticket: Ticket<OpenKey>, text: &str) {
    let FileTarget::Remote(file) = &ticket.key.path else {
        panic!("remote ticket")
    };
    let result = Opened {
        document: Document::remote(
            Buffer::from_text(text),
            crate::editor::document::RemoteDocument {
                file: file.absolute_file().unwrap().clone(),
                window: strop_remote::RemoteWindow::resolve(
                    &ticket.key.selection,
                    strop_remote::RemoteSize::new(text.len() as u64),
                ),
                selection: ticket.key.selection,
                connection: None,
                return_to: None,
            },
        ),
        canonical: ticket.key.path.clone(),
    };
    editor.handle_io(IoEvent::Open(Box::new(Completion {
        ticket,
        outcome: Outcome::Success(result),
    })));
}
fn open(editor: &mut Editor, text: &str) {
    editor.feed_text(":e ssh://fixture/var/log/app.log<cr>");
    let ticket = ticket(editor);
    deliver(editor, ticket, text);
}

#[test]
fn snapshot_search_yank_and_readonly_commands_use_the_real_buffer() {
    let mut editor = editor("origin\n");
    open(&mut editor, "first\nneedle detail\nlast\n");
    editor.feed_text("/needle<cr>yy");
    assert_eq!(editor.buf().line_of(editor.head()), 1);
    assert_eq!(editor.register(None).text, "needle detail\n");
    editor.feed_text(":set noro<cr>dd");
    assert_eq!(editor.buf().text(), "first\nneedle detail\nlast\n");
    assert!(editor.buf().readonly);
    editor.feed_text(":w! /would-write-locally<cr>");
    assert!(
        !editor.io_pending(),
        "write is refused, never pending on a worker"
    );
    assert!(editor.message.contains("remote"));
    assert!(editor.buf().path.is_none());
    assert!(matches!(editor.cur().source, DocumentSource::Remote(_)));
    editor.feed(Key::Esc); // dismiss the long write error before inspecting identity
    let frame = crate::headless::frame_string(&mut editor, 100, 8).unwrap();
    assert!(frame.contains("ssh:fixture"));
    assert!(frame.contains("app.log"));
    assert!(frame.contains("[RO]"));
}

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
