//! 0058 S7 acceptance: hint application over the real watcher thread
//! (inotify watches, 25ms pump), plus deterministic synthetic-record
//! drives for identity,
//! overflow and lifecycle edges. Invariants: dirty
//! buffers are never clobbered, stale generations never act, overflow
//! is a conservative rescan, close retires the subscription.
use super::*;
use crate::editor::Editor;
use std::time::{Duration, Instant};
use strop_core::worker::Outcome;
use strop_core::Buffer;
use strop_fs::notify::{Event, NotifyHint, NotifyKind, Subscription};

const TIMEOUT: Duration = Duration::from_secs(30);

/// An editor with the scope subscription established by the real
/// watcher thread.
fn subscribed_editor(dir: &std::path::Path) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.to_path_buf());
    editor.start_notifications();
    let deadline = Instant::now() + TIMEOUT;
    while editor.notify.subscription.is_none() {
        assert!(Instant::now() < deadline, "the subscribe job settles");
        editor.drain_notify();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        editor.notify.push_coverage(),
        "test namespaces watch natively"
    );
    editor
}

fn hint(editor: &Editor, relative: &str, kind: NotifyKind) -> Event {
    Event::Notify {
        subscription: editor.notify.subscription.unwrap(),
        sequence: 0,
        hints: vec![NotifyHint {
            path: relative.as_bytes().to_vec(),
            kind,
        }],
    }
}

/// Pump real worker events and drive the resulting reload jobs until
/// `condition` holds.
fn pump_until(editor: &mut Editor, mut condition: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition(editor) {
        assert!(Instant::now() < deadline, "the condition settles");
        editor.drain_notify();
        let _ = editor.wait_io();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A hint reloads a clean buffer: the on-disk change appears, the
/// buffer stays clean, and the observed binding advances (a later save
/// does not trip the external-change refusal).
#[test]
fn a_hint_reloads_a_clean_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "one\n").unwrap();
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(&path).unwrap();
    assert_eq!(editor.buf().text(), "one\n");
    std::fs::write(&path, "two\n").unwrap();
    pump_until(&mut editor, |e| e.buf().text() == "two\n");
    assert!(!editor.buf().is_dirty());
    assert!(!editor.cur().external_change);
}

/// A dirty buffer is never clobbered: the hint sets external-change
/// state, the edits survive, and the on-disk content stays out.
#[test]
fn a_hint_preserves_a_dirty_buffer_with_external_change_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "one\n").unwrap();
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(&path).unwrap();
    editor.feed_text("imine <esc>");
    assert!(editor.buf().is_dirty());
    std::fs::write(&path, "theirs\n").unwrap();
    pump_until(&mut editor, |e| e.cur().external_change);
    assert_eq!(editor.buf().text(), "mine one\n");
    assert!(editor.buf().is_dirty(), "edits are preserved");
}

/// Overflow is an explicit rescan obligation: with no usable hint
/// paths, the clean document still reobserves and reloads, and the
/// dirty document gains external-change state.
#[test]
fn overflow_rescans_conservatively() {
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("clean.txt");
    let dirty = dir.path().join("dirty.txt");
    std::fs::write(&clean, "clean-one\n").unwrap();
    std::fs::write(&dirty, "dirty-one\n").unwrap();
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(&clean).unwrap();
    let clean_doc = editor.current();
    editor.open_fixture(&dirty).unwrap();
    let dirty_doc = editor.current();
    editor.feed_text("imine <esc>");
    assert!(editor.buf().is_dirty());

    std::fs::write(&clean, "clean-two\n").unwrap();
    std::fs::write(&dirty, "dirty-two\n").unwrap();
    let subscription = editor.notify.subscription.unwrap();
    editor.notify.queue.push_event(Event::NotifyOverflow {
        subscription,
        sequence: 1,
    });
    editor.handle_notify();
    pump_until(&mut editor, |e| {
        !e.notify.pending()
            && e.doc(clean_doc).buf.text() == "clean-two\n"
            && e.doc(dirty_doc).external_change
    });
    assert_eq!(
        editor.doc(dirty_doc).buf.text(),
        "mine dirty-one\n",
        "the dirty buffer kept its edits through the rescan"
    );
}

/// Events stamped by a superseded generation are refused: no reload,
/// no external-change state, no picker invalidation.
#[test]
fn a_dead_generation_never_acts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "one\n").unwrap();
    // A fixed identity and no watcher thread: the only event is the
    // stale one pushed below.
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_fixture(&path).unwrap();
    let current = Subscription {
        id: 1,
        generation: 1,
    };
    editor.notify.subscription = Some(current);
    let stale = Subscription {
        id: current.id,
        generation: current.generation + 1,
    };
    std::fs::write(&path, "two\n").unwrap();
    editor.notify.queue.push_event(Event::Notify {
        subscription: stale,
        sequence: 99,
        hints: vec![NotifyHint {
            path: b"note.txt".to_vec(),
            kind: NotifyKind::Modified,
        }],
    });
    editor.handle_notify();
    assert!(
        !editor.notify.pending(),
        "no reload was spawned for the stale generation"
    );
    assert_eq!(editor.buf().text(), "one\n", "stale events never act");
    assert!(!editor.cur().external_change);
}

/// A hint against an open Directory buffer reobserves the listing
/// through the worker (the revision re-check at completion is the
/// directory/filter.rs correspondence).
#[test]
fn a_hint_reobserves_an_open_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("before.txt"), "x\n").unwrap();
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(dir.path()).unwrap();
    assert!(editor.directory().is_some(), "a directory buffer opened");
    std::fs::write(dir.path().join("after.txt"), "y\n").unwrap();
    let event = hint(&editor, "after.txt", NotifyKind::Created);
    editor.notify.queue.push_event(event);
    editor.handle_notify();
    pump_until(&mut editor, |e| {
        e.directory().is_some() && e.buf().text().to_string().contains("after.txt")
    });
}

/// A hinted rename/vanish of the open clean file never relocates or
/// clobbers: observation reports it missing, the buffer is preserved
/// with external-change state (AmbiguousNeverRelocates).
#[test]
fn a_removed_file_preserves_the_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "one\n").unwrap();
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let event = hint(&editor, "note.txt", NotifyKind::Removed);
    editor.notify.queue.push_event(event);
    editor.handle_notify();
    pump_until(&mut editor, |e| e.cur().external_change);
    assert_eq!(editor.buf().text(), "one\n", "content preserved");
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(path.as_path()),
        "the binding never relocated"
    );
}

/// Close retires the subscription: after finish, queued late events
/// are refused (no identity), and the editor's notify state is clean.
#[test]
fn close_retires_the_subscription() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "one\n").unwrap();
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(&path).unwrap();
    editor.finish_background_work();
    assert!(
        editor.notify.subscription.is_none(),
        "unsubscribed at close"
    );
    std::fs::write(&path, "two\n").unwrap();
    editor.drain_notify();
    let _ = editor.wait_io();
    assert_eq!(editor.buf().text(), "one\n", "late events never apply");
}

#[test]
fn trace_owned_hints_do_not_wake_the_editor_but_other_files_still_do() {
    let directory = tempfile::tempdir().unwrap();
    let trace = strop_trace::start(
        &directory.path().join("capture.jsonl"),
        strop_trace::TraceOptions::default(),
    )
    .unwrap();
    let subscription = Subscription {
        id: 1,
        generation: 1,
    };
    let queue = NotifyQueue::new();
    queue.set_owned_trace(Some((subscription, b"capture.jsonl".to_vec())));
    let hint = |path: &[u8]| NotifyHint {
        path: path.to_vec(),
        kind: NotifyKind::Modified,
    };
    assert!(!queue.push_event(Event::Notify {
        subscription,
        sequence: 1,
        hints: vec![hint(b"capture.jsonl")],
    }));
    assert!(queue.drain().0.is_empty(), "no self-triggered wake");
    assert!(queue.push_event(Event::Notify {
        subscription,
        sequence: 2,
        hints: vec![hint(b"capture.jsonl"), hint(b"user.txt")],
    }));
    let (records, rescan) = queue.drain();
    assert!(!rescan);
    assert!(matches!(
        records.as_slice(),
        [Record::Hints { hints, .. }] if hints.len() == 1 && hints[0].path == b"user.txt"
    ));
    trace.finish().unwrap();
}

#[test]
fn a_full_notify_queue_cannot_drop_subscription_settlement() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), directory.path().to_path_buf());
    editor.notify.subscribing = true;
    // Distinct identities prevent tail coalescing: these records occupy the
    // complete bounded landing zone before the subscribe job settles.
    for id in 0..64 {
        assert!(editor.notify.queue.push_hints(
            Subscription { id, generation: 0 },
            vec![NotifyHint {
                path: b"queued.txt".to_vec(),
                kind: NotifyKind::Modified,
            }],
        ));
    }
    let settled = Subscription {
        id: 65,
        generation: 1,
    };
    editor
        .notify
        .queue
        .push_record(Record::Settled(Outcome::Success(SubscribedScope {
            subscription: settled,
            root: directory.path().to_path_buf(),
            owned_trace: None,
        })));

    editor.drain_notify();
    assert!(
        !editor.notify.subscribing,
        "the settled record must release the in-flight subscription"
    );
    assert_eq!(
        editor.notify.subscription,
        Some(settled),
        "queue overflow cannot strand subscription establishment"
    );
}

/// An editor whose subscription settled through a synthetic record, so
/// hints are driven exactly; `root` is the watched root resolved.
fn settled_editor(cwd: &std::path::Path) -> (Editor, Subscription) {
    let mut editor = Editor::new_in(Buffer::from_text(""), cwd.to_path_buf());
    editor.notify.subscribing = true;
    let subscription = Subscription {
        id: 1,
        generation: 1,
    };
    editor
        .notify
        .queue
        .push_record(Record::Settled(Outcome::Success(SubscribedScope {
            subscription,
            root: std::fs::canonicalize(cwd).unwrap(),
            owned_trace: None,
        })));
    editor.drain_notify();
    (editor, subscription)
}

fn modified(path: &str) -> NotifyHint {
    NotifyHint {
        path: path.as_bytes().to_vec(),
        kind: NotifyKind::Modified,
    }
}

/// 0108 H7: a hint names a path under the watched root, matched against
/// the documents' own names and the root's resolved twin — the handler
/// resolves nothing itself. Through a symlinked root the canonical
/// identity still matches; a hint about a symlink inside the tree is
/// about the link, not the file it points at.
#[test]
fn hints_match_through_the_resolved_root_without_resolving_each_hint() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, dir.path().join("link")).unwrap();
    std::fs::write(real.join("note.txt"), "one\n").unwrap();
    std::os::unix::fs::symlink("note.txt", real.join("alias")).unwrap();
    let (mut editor, subscription) = settled_editor(&dir.path().join("link"));
    editor
        .open_fixture(&std::fs::canonicalize(real.join("note.txt")).unwrap())
        .unwrap();
    editor.feed_text("ix\x1b");
    assert!(editor.buf().is_dirty());
    editor
        .notify
        .queue
        .push_hints(subscription, vec![modified("alias")]);
    editor.drain_notify();
    assert!(
        !editor.cur().external_change,
        "the link changed, not the file"
    );
    editor
        .notify
        .queue
        .push_hints(subscription, vec![modified("note.txt")]);
    editor.drain_notify();
    assert!(editor.cur().external_change);
}

/// A checkout's worth of hints against many open documents is one index
/// lookup per hint.
#[test]
fn a_flood_of_hints_finds_the_one_open_document_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for index in 0..50 {
        std::fs::write(root.join(format!("open{index}.txt")), "x\n").unwrap();
    }
    let (mut editor, subscription) = settled_editor(&root);
    for index in 0..50 {
        editor
            .open_fixture(&root.join(format!("open{index}.txt")))
            .unwrap();
        editor.feed_text("iy\x1b");
    }
    for batch in 0..64 {
        let hints = (0..500)
            .map(|index| modified(&format!("build/{batch}/{index}.o")))
            .chain((batch == 63).then(|| modified("open7.txt")))
            .collect();
        editor.notify.queue.push_hints(subscription, hints);
    }
    editor.drain_notify();
    let marked: Vec<_> = editor
        .docs
        .iter()
        .filter(|(_, document)| document.external_change)
        .filter_map(|(_, document)| document.buf.path.clone())
        .collect();
    assert_eq!(marked, vec![root.join("open7.txt")]);
}
