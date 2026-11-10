use super::*;
use crate::editor::document::RemoteDocument;
use crate::editor::Document;
use strop_core::frontend_input::Key;
use strop_core::Buffer;
use strop_remote::{ReadLimit, ReadSelection, RemoteSize, RemoteWindow};

fn fixture(selection: ReadSelection) -> Editor {
    let file = RemoteFile::parse("ssh://fixture/work/file.txt").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin\n"), "/isolated".into());
    editor.config.completion.enabled = false; // This fixture owns save services only.
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other(
            "native observation forbidden in ownership fixture",
        ))
    }));
    let document = editor
        .docs
        .try_insert(Document::remote(
            Buffer::from_text("before\n"),
            RemoteDocument {
                file,
                window: RemoteWindow::resolve(&selection, RemoteSize::new(7)),
                selection,
                connection: None,
                return_to: None,
                write: None,
            },
        ))
        .unwrap();
    editor.switch_to(document);
    editor
}
fn version(file: &RemoteFile, contents: &Rope) -> WriteVersion {
    WriteVersion {
        file: file.clone(),
        modified: FileTime {
            seconds: 1,
            nanos: 0,
        },
        size: contents.len_bytes() as u64,
        identity: Some(ObjectId {
            device: 1,
            inode: 1,
        }),
        attributes: None,
        content: store::digest_of(contents),
    }
}
fn enable(editor: &mut Editor) {
    editor.enable_remote_edit().unwrap();
    let ticket = editor.remote.writes.pending[&editor.current()].clone();
    let version = version(&ticket.key.file, &editor.buf().snapshot());
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Enabled(version)),
    });
    assert!(!editor.buf().readonly);
}
fn receipt(editor: &Editor, ticket: &Ticket<RemoteWriteKey>) -> WriteVersion {
    version(
        &ticket.key.file,
        &editor.remote.writes.attempts[&ticket.key.document].contents,
    )
}

#[test]
fn lost_store_reply_reconciles_over_a_restarted_real_worker() {
    use strop_worker_client::{Transport, Worker};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("save.txt");
    std::fs::write(&path, b"before\n").unwrap();
    let file = RemoteFile::parse(&format!("ssh://fixture{}", path.display())).unwrap();
    let endpoint = file.endpoint().clone();
    let worker = Worker::connect_with(|| {
        let (client_read, worker_write) = std::io::pipe()?;
        let (worker_read, client_write) = std::io::pipe()?;
        std::thread::spawn(move || strop_worker::serve::run(worker_read, worker_write).unwrap());
        Ok(Transport {
            reader: Box::new(client_read),
            writer: Box::new(client_write),
            child: None,
            stderr: None,
        })
    });
    let lease = strop_remote::worker_transport::RemoteWorker::for_test(&endpoint, worker.clone());
    let (token, _owner) = strop_core::worker::CancelToken::standalone();
    let baseline =
        store::prepare_edit(&lease, &file, &Rope::from_str("before\n"), None, &token).unwrap();
    let prepared = FrozenStore::default();
    let saved = match store::save(
        &lease,
        &baseline,
        &Rope::from_str("after\n"),
        &prepared,
        &token,
    )
    .unwrap()
    {
        store::WorkerSave::Saved(saved) => saved,
        store::WorkerSave::Unconfirmed { detail, .. } => {
            panic!("store unexpectedly uncertain: {detail}")
        }
    };
    assert_eq!(std::fs::read(&path).unwrap(), b"after\n");
    assert_eq!(saved.modified, baseline.modified);
    let (namespace, operation) = prepared.lock().take().unwrap();
    let receipt = StepReceipt {
        step: 0,
        operation,
        outcome: StepOutcome::Unconfirmed {
            detail: "lost reply".into(),
            observed_destination: None,
            recovery: None,
            publication: None,
        },
    };
    worker.shutdown().unwrap();
    let verified = store::verify(
        &lease,
        &baseline,
        receipt.clone(),
        namespace.clone(),
        &token,
    )
    .unwrap();
    let store::WorkerVerification::Committed(recovered) = verified else {
        panic!("committed bytes must reconcile");
    };
    assert_eq!(
        recovered.content,
        store::digest_of(&Rope::from_str("after\n"))
    );
    std::fs::write(&path, b"foreign\n").unwrap();
    assert!(store::verify(&lease, &baseline, receipt, namespace, &token).is_err());
    worker.shutdown().unwrap();
}

#[test]
fn cancelled_admission_cannot_publish_a_queued_write_grant() {
    let mut editor = fixture(ReadSelection::Full);
    editor.enable_remote_edit().unwrap();
    let ticket = editor.remote.writes.pending[&editor.current()].clone();
    let ready = version(&ticket.key.file, &editor.buf().snapshot());
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
    let ready = version(&ticket.key.file, &editor.buf().snapshot());
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Enabled(ready)),
    });
    editor.feed_text("A draft<esc>");
    assert!(editor.buf().is_dirty());
    assert!(!editor.remote_following(document));
    assert_eq!(editor.buf().text(), "before draft\n");
}

#[test]
fn lost_worker_reply_keeps_dirty_attempt_until_explicit_verification() {
    let mut editor = fixture(ReadSelection::Full);
    enable(&mut editor);
    editor.feed_text("A draft<esc>");
    let document = editor.current();
    editor.request_save(None, false, false);
    let ticket = editor.remote.writes.pending[&document].clone();
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::failed(
            strop_core::worker::FailureKind::Io,
            "worker connection lost after store submission",
        ),
    });
    assert!(editor.buf().is_dirty());
    assert!(editor.remote.writes.attempts[&document].unconfirmed);
    assert!(editor.message.contains("unconfirmed"));
    editor.feed_text(":w<cr>");
    assert!(editor.message.contains(":remote verify"));
    assert!(!editor.remote.writes.pending.contains_key(&document));
    editor.verify_remote_save().unwrap();
    let ticket = editor.remote.writes.pending[&document].clone();
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Verified(VerifiedWrite::Unchanged)),
    });
    assert!(editor.buf().is_dirty());
    assert!(!editor.remote.writes.attempts.contains_key(&document));
    assert!(editor.message.contains("local edits remain unsaved"));
}

#[test]
fn remote_unconfirmed_feedback_offers_verify_not_blind_retry() {
    let mut editor = fixture(ReadSelection::Full);
    enable(&mut editor);
    editor.feed_text("A draft<esc>");
    let document = editor.current();
    editor.request_save(None, false, false);
    let ticket = editor.remote.writes.pending[&document].clone();
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::failed(
            strop_core::worker::FailureKind::Io,
            "worker connection lost after store submission",
        ),
    });
    assert!(editor.feedback()[0]
        .detail()
        .contains("worker connection lost"));
    assert!(editor
        .filesystem_shutdown_error()
        .is_some_and(|error| error.contains("remote document save")));

    editor.feed_text("gg/unrelated<esc>");
    editor.context_actions();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker
        .items
        .iter()
        .any(|item| item.text.contains("Verify the unconfirmed remote save")));
    assert!(!picker
        .items
        .iter()
        .any(|item| item.text.contains("Retry the failed save")));
    editor.accept_action(0);
    let ticket = editor.remote.writes.pending[&document].clone();
    assert_eq!(ticket.key.action, WriteAction::Verify);
    editor.remote_write_done(Completion {
        ticket,
        outcome: Outcome::Success(RemoteWriteResult::Verified(VerifiedWrite::Unchanged)),
    });
    assert!(editor.buf().is_dirty());
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        crate::editor::feedback::FeedbackLifecycle::Resolved
    );
    assert!(editor.filesystem_shutdown_error().is_none());
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
    assert!(editor.buf().is_dirty());
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
        assert!(editor
            .docs
            .get(document)
            .is_some_and(|doc| doc.buf.is_dirty()));
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
        !editor.buf().is_dirty(),
        "a confirmed commit outranks an ineffective late cancellation"
    );
    assert_eq!(editor.buf().text(), "before draft\n");
}

/// `:unfollow` keeps the snapshot and stops the follow clock; with
/// nothing followed it says so.
#[test]
fn unfollow_stops_following_and_keeps_the_snapshot() {
    let limit = ReadLimit::new(32).unwrap();
    let mut editor = fixture(ReadSelection::Tail(limit));
    let document = editor.current();
    editor.start_remote_follow(document, limit);
    assert!(editor.remote_following(document));
    editor.feed_text(":unfollow\r");
    assert_eq!(editor.message(), "follow stopped");
    assert!(!editor.remote_following(document));
    assert_eq!(editor.buf().text(), "before\n", "the snapshot stays");
    editor.feed_text(":unfollow\r");
    assert_eq!(editor.message(), "not following");
}
