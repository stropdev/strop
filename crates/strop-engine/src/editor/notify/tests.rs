//! 0058 S7 acceptance: hint application over the REAL in-process worker
//! (subscribe handshake, inotify watches, 25ms pump, codec — the WK04
//! seam), plus deterministic synthetic-record drives for identity,
//! overflow and lifecycle edges. Notify.tla correspondence: dirty
//! buffers are never clobbered, stale generations never act, overflow
//! is a conservative rescan, close retires the subscription.
use super::*;
use crate::editor::Editor;
use std::time::{Duration, Instant};
use strop_core::Buffer;
use strop_worker_protocol::{LeaseId, NotifyHint, NotifyKind};

const TIMEOUT: Duration = Duration::from_secs(30);

/// An editor with the scope subscription established through the real
/// in-process worker lease.
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
    assert!(!editor.buf().dirty);
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
    assert!(editor.buf().dirty);
    std::fs::write(&path, "theirs\n").unwrap();
    pump_until(&mut editor, |e| e.cur().external_change);
    assert_eq!(editor.buf().text(), "mine one\n");
    assert!(editor.buf().dirty, "edits are preserved");
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
    assert!(editor.buf().dirty);

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
    let mut editor = subscribed_editor(dir.path());
    editor.open_fixture(&path).unwrap();
    let current = editor.notify.subscription.unwrap();
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

/// A worker restart kills the subscription with its session: the lease
/// observation invalidates the baseline, surfaces the loss, and
/// reestablishes coverage with a fresh generation.
#[test]
fn worker_loss_reestablishes_with_a_fresh_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = subscribed_editor(dir.path());
    let before = editor.notify.subscription.unwrap();
    // Simulate the lease moving past the recorded session (the in-process
    // connection is alive, so the recorded incarnation must diverge).
    editor.notify.session = Some(Session {
        incarnation: u64::MAX,
        lease: LeaseId(u64::MAX),
    });
    editor.notify_observe_lease();
    assert!(
        editor.notify.subscription.is_none(),
        "the dead session's subscription is dropped"
    );
    assert!(
        editor.notify.subscribing || editor.notify.subscription.is_some(),
        "coverage is reestablished"
    );
    let deadline = Instant::now() + TIMEOUT;
    while editor.notify.subscription.is_none() {
        assert!(Instant::now() < deadline, "the resubscribe settles");
        editor.drain_notify();
        std::thread::sleep(Duration::from_millis(5));
    }
    let after = editor.notify.subscription.unwrap();
    assert_ne!(
        before, after,
        "reinstallation enters with a bumped identity"
    );
}

/// WK07: hints on an admitted remote scope apply conservatively — a
/// dirty remote document gains external-change state and is never
/// clobbered, and a clean one is left to its follow polling.
#[test]
fn remote_hints_mark_dirty_remote_documents_and_never_clobber() {
    let mut editor = crate::editor::test_support::remote::snapshot_editor("remote bytes\n");
    let document = editor.current();
    editor.doc_mut(document).buf.dirty = true;
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap();
    let root = ResourceLocation::remote(endpoint.clone(), "/repo".into());
    let identity = Subscription {
        id: 41,
        generation: 7,
    };
    editor.notify.remote.insert(
        root,
        RemoteScope {
            endpoint,
            subscription: Some(identity),
            session: None,
            subscribing: false,
            subscribe_handle: None,
        },
    );
    editor.notify.queue.push_hints(
        identity,
        vec![NotifyHint {
            path: b"app.log".to_vec(),
            kind: NotifyKind::Modified,
        }],
    );
    editor.drain_notify();
    assert!(
        editor.cur().external_change,
        "the dirty remote document gains external-change state"
    );
    assert_eq!(editor.buf().text(), "remote bytes\n", "never clobbered");
    assert!(editor.buf().dirty, "edits are preserved");
}

/// WK07: events stamped by an unknown or superseded remote identity
/// never act.
#[test]
fn a_dead_remote_generation_never_acts() {
    let mut editor = crate::editor::test_support::remote::snapshot_editor("remote bytes\n");
    let document = editor.current();
    editor.doc_mut(document).buf.dirty = true;
    editor.notify.queue.push_hints(
        Subscription {
            id: 999,
            generation: 1,
        },
        vec![NotifyHint {
            path: b"app.log".to_vec(),
            kind: NotifyKind::Modified,
        }],
    );
    editor.drain_notify();
    assert!(!editor.cur().external_change, "no identity, no effect");
}

/// WK07: a remote scope adopts its identity only from the settle record
/// — hints arriving under the settled identity then apply in order.
#[test]
fn remote_subscription_identity_comes_from_the_settle_record() {
    let mut editor = crate::editor::test_support::remote::snapshot_editor("remote bytes\n");
    let document = editor.current();
    editor.doc_mut(document).buf.dirty = true;
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap();
    let root = ResourceLocation::remote(endpoint.clone(), "/repo".into());
    editor.notify.remote.insert(
        root.clone(),
        RemoteScope {
            endpoint,
            subscription: None,
            session: None,
            subscribing: true,
            subscribe_handle: None,
        },
    );
    // Before the settle: hints with any identity are inert.
    editor.notify.queue.push_hints(
        Subscription {
            id: 1,
            generation: 1,
        },
        vec![NotifyHint {
            path: b"app.log".to_vec(),
            kind: NotifyKind::Modified,
        }],
    );
    editor.drain_notify();
    assert!(!editor.cur().external_change);
    // The settle lands on the same queue and adopts the identity.
    let identity = Subscription {
        id: 5,
        generation: 1,
    };
    editor.notify.queue.push_record(Record::RemoteSettled {
        root,
        outcome: Outcome::Success(SubscribedScope {
            subscription: identity,
            coverage: strop_worker_protocol::NotifyCoverage::Native,
            owned_trace: None,
        }),
    });
    editor.drain_notify();
    editor.notify.queue.push_hints(
        identity,
        vec![NotifyHint {
            path: b"app.log".to_vec(),
            kind: NotifyKind::Modified,
        }],
    );
    editor.drain_notify();
    assert!(editor.cur().external_change, "the settled identity acts");
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
    let queue = NotifyQueue {
        state: Mutex::new(QueueState::default()),
    };
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
