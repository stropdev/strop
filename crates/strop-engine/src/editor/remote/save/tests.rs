use super::*;
use crate::editor::document::RemoteDocument;
use crate::editor::{Document, Key};
use serde_json::json;
use strop_core::Buffer;
use strop_remote::{ReadLimit, ReadSelection, RemoteSize, RemoteWindow};

fn fixture(selection: ReadSelection) -> Editor {
    let file = RemoteFile::parse("ssh://fixture/work/file.txt").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin\n"), "/isolated".into());
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other(
            "native observation forbidden in ownership fixture",
        ))
    }));
    let document = editor.docs.insert(Document::remote(
        Buffer::from_text("before\n"),
        RemoteDocument {
            file,
            window: RemoteWindow::resolve(&selection, RemoteSize::new(7)),
            selection,
            connection: None,
            return_to: None,
            write: None,
        },
    ));
    editor.switch_to(document);
    editor
}
fn version(file: &RemoteFile, length: usize) -> RemoteVersion {
    let digest = [0u8; 32];
    serde_json::from_value(json!({"file": file, "stamp": {
        "device": 1, "inode": 1, "size": length, "mtime_ns": 1, "ctime_ns": 1,
        "mode": 416, "uid": 1, "gid": 1, "content": digest, "attributes": digest
    }}))
    .unwrap()
}
fn enable(editor: &mut Editor) {
    editor.enable_remote_edit().unwrap();
    let ticket = editor.remote.writes.pending[&editor.current()].clone();
    let version = version(&ticket.key.file, editor.buf().len_bytes());
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Enabled(version)),
    });
    assert!(!editor.buf().readonly);
}
fn receipt(editor: &Editor, ticket: &Ticket<RemoteWriteKey>) -> RemoteSaveReceipt {
    let length = editor.remote.writes.attempts[&ticket.key.document]
        .contents
        .len_bytes();
    serde_json::from_value(json!({"version": version(&ticket.key.file, length)})).unwrap()
}

#[test]
fn cancelled_admission_cannot_publish_a_queued_write_grant() {
    let mut editor = fixture(ReadSelection::Full);
    editor.enable_remote_edit().unwrap();
    let ticket = editor.remote.writes.pending[&editor.current()].clone();
    let ready = version(&ticket.key.file, editor.buf().len_bytes());
    editor.feed(Key::Esc);
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Enabled(ready)),
    });
    assert!(
        editor.buf().readonly,
        "cancelled authority cannot be revived by queued success"
    );
    editor.feed_text("iNOT-ALLOWED<esc>");
    assert_eq!(editor.buf().text(), "before\n");
}

#[test]
fn pending_edit_admission_and_follow_cannot_share_a_document() {
    let limit = ReadLimit::new(32).unwrap();
    let mut editor = fixture(ReadSelection::Tail(limit));
    let document = editor.current();
    editor.enable_remote_edit().unwrap();
    let ticket = editor.remote.writes.pending[&document].clone();
    editor.start_remote_follow(document, limit);
    assert!(
        !editor.remote_following(document),
        "pending write authority excludes automatic replacement"
    );
    let ready = version(&ticket.key.file, editor.buf().len_bytes());
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Enabled(ready)),
    });
    editor.feed_text("A draft<esc>");
    assert!(editor.buf().dirty);
    assert!(!editor.remote_following(document));
    assert_eq!(editor.buf().text(), "before draft\n");
}

#[test]
fn older_save_receipt_keeps_new_edits_and_does_not_close_the_pane() {
    let mut editor = fixture(ReadSelection::Full);
    enable(&mut editor);
    let document = editor.current();
    editor.feed_text("A first<esc>");
    editor.request_save(None, false, true);
    let ticket = editor.remote.writes.pending[&document].clone();
    let saved = receipt(&editor, &ticket);
    editor.feed_text("A newer<esc>");
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Saved(saved)),
    });
    assert_eq!(editor.current(), document);
    assert!(!editor.should_quit);
    assert!(editor.buf().dirty);
    assert_eq!(editor.buf().text(), "before first newer\n");
    assert!(
        editor.buf().path.is_none(),
        "remote saves never install a local path"
    );
}

#[test]
fn write_and_quit_does_not_discard_an_explicit_destination() {
    for command in [":wq /tmp/export<cr>", ":wq! ssh://other/work/file<cr>"] {
        let mut editor = fixture(ReadSelection::Full);
        enable(&mut editor);
        let document = editor.current();
        editor.feed_text("A draft<esc>");
        editor.feed_text(command);
        // A broken dispatcher can issue a write to the original file. Settle
        // that effect if issued, then assert the consumer-visible refusal.
        if let Some(ticket) = editor.remote.writes.pending.get(&document).cloned() {
            let saved = receipt(&editor, &ticket);
            editor.remote_write_done(Completion {
                ticket,
                outcome: Outcome::Success(RemoteWriteResult::Saved(saved)),
            });
        }
        assert!(editor.docs.get(document).is_some_and(|doc| doc.buf.dirty));
        assert_eq!(editor.current(), document);
        assert!(!editor.should_quit);
    }
}

#[test]
fn cancellation_does_not_discard_a_save_receipt_already_queued() {
    let mut editor = fixture(ReadSelection::Full);
    enable(&mut editor);
    editor.feed_text("A draft<esc>");
    let document = editor.current();
    editor.request_save(None, false, false);
    let ticket = editor.remote.writes.pending[&document].clone();
    let saved = receipt(&editor, &ticket);
    editor.cancel_remote_write(document);
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Saved(saved)),
    });
    assert!(
        !editor.buf().dirty,
        "a confirmed commit outranks an ineffective late cancellation"
    );
    assert_eq!(editor.buf().text(), "before draft\n");
}

fn publish_rank(editor: &mut Editor) {
    let glue = editor.picker.as_ref().unwrap();
    let ticket = glue.rank_pending.clone().unwrap();
    let ranking = strop_picker::rank::rank(&glue.picker.filter_request(), || false)
        .unwrap()
        .unwrap();
    editor.handle_picker_ranking(crate::editor::picker::ranking::Event {
        picker: glue.id,
        update: strop_picker::RankingEvent::Completed(Completion {
            ticket,
            outcome: Outcome::Success(ranking),
        }),
    });
}

fn remote_collection(editor: &mut Editor) -> DocumentId {
    editor.set_picker(crate::editor::picker::PickerGlue::diagnostics(
        strop_picker::Picker::new(strop_picker::Kind::Locations, Vec::new(), false),
    ));
    publish_rank(editor);
    editor
        .picker
        .as_mut()
        .unwrap()
        .picker
        .append(vec![strop_picker::Item {
            badge: None,
            text: "remote source".into(),
            payload: strop_picker::Payload::Remote {
                endpoint: strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
                path: "/work/file.txt".into(),
                line: 1,
                col: 1,
            },
        }]);
    // The fixture tape suppresses native actors. Publish the real pure
    // ranker's result through its registered owner, rather than waiting
    // for an intentionally unlaunched worker.
    editor.request_picker_ranking();
    publish_rank(editor);
    editor.feed(Key::CtrlO);
    editor.current()
}

#[test]
fn collection_write_and_quit_waits_for_the_remote_receipt() {
    let mut editor = fixture(ReadSelection::Full);
    enable(&mut editor);
    let source = editor.current();
    editor.feed_text("A draft<esc>");
    let collection = remote_collection(&mut editor);
    editor.feed_text(":wq<cr>");
    assert_eq!(editor.current(), collection);
    let ticket = editor.remote.writes.pending[&source].clone();
    let saved = receipt(&editor, &ticket);
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Saved(saved)),
    });
    assert!(
        editor.docs.get(collection).is_none(),
        "confirmed source write closes the view"
    );
    assert!(editor.docs.get(source).is_some_and(|doc| !doc.buf.dirty));
}

#[test]
fn refused_collection_save_cannot_close_on_an_unrelated_old_receipt() {
    let mut editor = fixture(ReadSelection::Full);
    enable(&mut editor);
    let source = editor.current();
    editor.feed_text("A draft<esc>");
    editor.request_save(None, false, false);
    let ticket = editor.remote.writes.pending[&source].clone();
    let saved = receipt(&editor, &ticket);
    let collection = remote_collection(&mut editor);
    editor.feed_text(":wq<cr>");
    assert!(editor.message.contains("pending"), "{}", editor.message);
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Saved(saved)),
    });
    assert_eq!(editor.current(), collection);
    assert!(editor.docs.get(collection).is_some());
}
