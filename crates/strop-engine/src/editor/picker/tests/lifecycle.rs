//! Worker lifecycle: injected terminal events (deterministic — no real
//! rg, no sleeps, no HOME assumptions; the supervisor's own decisions
//! are unit-tested in strop-picker).

use super::super::preview::read_preview;
use super::super::*;
use std::path::PathBuf;
use strop_core::worker::{CancelReason, Completion, FailureKind, Load, Outcome, Ticket};
use strop_core::Buffer;
use strop_picker::{Item, Payload, PickerMsg};

fn editor() -> Editor {
    Editor::new(Buffer::from_text("x\n"))
}

fn item(text: &str) -> Item {
    Item {
        badge: None,
        text: text.into(),
        payload: Payload::File(PathBuf::from(text)),
    }
}

fn rank_fixture(e: &mut Editor) {
    let picker = &mut e.picker.as_mut().unwrap().picker;
    let ranking = strop_picker::rank::rank(&picker.filter_request(), || false)
        .unwrap()
        .unwrap();
    assert!(picker.install_ranking(ranking));
}

/// Register a grep-stream request without launching rg: the glue
/// only needs the ticket; events are injected by hand.
fn register_stream(e: &mut Editor) -> Ticket<PickerKey> {
    e.open_picker(Kind::Search);
    let picker = e.picker.as_ref().map(|g| g.id).unwrap();
    let request = e.worker_ids.allocate().unwrap();
    let ticket = Ticket {
        request,
        key: PickerKey {
            picker,
            cwd: e.cwd.clone(),
        },
    };
    let glue = e.picker.as_mut().unwrap();
    glue.active = Some(ticket.clone());
    glue.picker.streaming = true;
    ticket
}

/// Register a preview request without launching a read thread.
fn register_preview(
    e: &mut Editor,
    rel: &str,
) -> (Ticket<PreviewKey>, strop_workspace::ResourceLocation) {
    let path = strop_workspace::ResourceLocation::local(e.cwd.join(rel));
    let picker = e.picker.as_ref().map(|g| g.id).unwrap();
    let request = e.worker_ids.allocate().unwrap();
    let ticket = Ticket {
        request,
        key: PreviewKey {
            picker,
            path: path.clone(),
        },
    };
    e.preview_loads
        .insert(path.clone(), Load::Running(ticket.clone()));
    (ticket, path)
}

#[test]
fn files_request_registers_before_launch_and_cancels_on_close() {
    let mut e = editor();
    e.open_picker(Kind::Files);
    let glue = e.picker.as_ref().unwrap();
    assert!(glue.picker.streaming, "the walk streams");
    assert!(
        glue.active.is_some(),
        "the request owns the stream before launch"
    );
    let ticket = glue.active.clone().unwrap();
    e.close_picker();
    assert!(!e.picker_open(), "close settles the picker");
    // a terminal event from the cancelled walk cannot resurrect it
    e.handle_picker_event(PickerEvent {
        ticket,
        msg: PickerMsg::Finished(Outcome::Success(())),
    });
    assert!(!e.picker_open());
}

#[test]
fn streamed_items_survive_a_terminal_failure_and_error_is_sticky() {
    let mut e = editor();
    let ticket = register_stream(&mut e);
    e.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Items(vec![item("a.rs"), item("b.rs")].into()),
    });
    e.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Finished(Outcome::failed(FailureKind::Spawn, "rg: not found")),
    });
    e.wait_picker();
    let glue = e.picker.as_ref().unwrap();
    assert!(!glue.picker.streaming, "terminal failure settles streaming");
    assert_eq!(glue.picker.rows.len(), 2, "useful partial results survive");
    assert_eq!(
        glue.picker.error.as_deref(),
        Some("rg: not found"),
        "error is sticky"
    );
    assert!(glue.active.is_none(), "the request is released");
    // duplicate terminals and post-terminal items are rejected
    e.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Finished(Outcome::Success(())),
    });
    e.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Items(vec![item("late.rs")].into()),
    });
    let glue = e.picker.as_ref().unwrap();
    assert_eq!(glue.picker.rows.len(), 2, "settled state is immutable");
    assert!(!glue.picker.streaming);
}

#[test]
fn empty_success_settles_the_stream_without_error() {
    // exit-1/no-matches and empty queries land as Success: an
    // empty picker, not a failure and not perpetual loading
    let mut e = editor();
    let ticket = register_stream(&mut e);
    e.handle_picker_event(PickerEvent {
        ticket,
        msg: PickerMsg::Finished(Outcome::Success(())),
    });
    let glue = e.picker.as_ref().unwrap();
    assert!(!glue.picker.streaming);
    assert!(glue.picker.error.is_none(), "no fabricated failure");
    assert!(glue.picker.rows.is_empty());
}

#[test]
fn superseded_requests_cannot_touch_the_new_owner() {
    let mut e = editor();
    let old = register_stream(&mut e);
    // the next query's registration (no rg: emulated by hand)
    let fresh = {
        let picker = e.picker.as_ref().unwrap().id;
        let request = e.worker_ids.allocate().unwrap();
        let ticket = Ticket {
            request,
            key: PickerKey {
                picker,
                cwd: e.cwd.clone(),
            },
        };
        e.picker.as_mut().unwrap().active = Some(ticket.clone());
        ticket
    };
    // A's late messages in every shape are rejected wholesale
    for msg in [
        PickerMsg::Items(vec![item("stale.rs")].into()),
        PickerMsg::Warning("stale".into()),
        PickerMsg::Finished(Outcome::Success(())),
        PickerMsg::Finished(Outcome::failed(FailureKind::Io, "stale pipe")),
        PickerMsg::Finished(Outcome::Cancelled(CancelReason::Superseded)),
    ] {
        e.handle_picker_event(PickerEvent {
            ticket: old.clone(),
            msg,
        });
    }
    let glue = e.picker.as_ref().unwrap();
    assert!(
        glue.picker.rows.is_empty(),
        "stale streams never touch the model"
    );
    assert!(glue.picker.error.is_none(), "stale failures never surface");
    assert!(glue.picker.streaming, "B keeps streaming");
    assert_eq!(glue.active.as_ref(), Some(&fresh), "B keeps ownership");
    // B then settles normally
    e.handle_picker_event(PickerEvent {
        ticket: fresh,
        msg: PickerMsg::Finished(Outcome::Success(())),
    });
    assert!(!e.picker.as_ref().unwrap().picker.streaming);
}

#[test]
fn preview_cache_evicts_insertion_order_but_keeps_the_selection() {
    let mut e = editor();
    e.open_picker(Kind::Search);
    e.picker
        .as_mut()
        .unwrap()
        .picker
        .append(vec![item("selected.txt")]);
    rank_fixture(&mut e);
    for index in 0..32 {
        let name = if index == 0 {
            "selected.txt".to_string()
        } else {
            format!("cached-{index}.txt")
        };
        let (ticket, path) = register_preview(&mut e, &name);
        e.handle_preview(Completion {
            ticket,
            outcome: Outcome::Success(String::from("body").into()),
        });
        assert!(e.previews.contains_key(&path));
    }
    let selected = strop_workspace::ResourceLocation::local(e.cwd.join("selected.txt"));
    let oldest = strop_workspace::ResourceLocation::local(e.cwd.join("cached-1.txt"));
    let (ticket, _) = register_preview(&mut e, "new.txt");
    e.handle_preview(Completion {
        ticket,
        outcome: Outcome::Success(String::from("new").into()),
    });
    assert_eq!(e.previews.len(), 32);
    assert!(e.previews.contains_key(&selected));
    assert!(!e.previews.contains_key(&oldest));
    e.close_picker();
}

#[test]
fn preview_failure_is_visible_and_retryable_after_reopen() {
    let mut e = editor();
    e.open_picker(Kind::Search);
    e.picker
        .as_mut()
        .unwrap()
        .picker
        .append(vec![item("a.txt")]);
    rank_fixture(&mut e);
    let (ticket, path) = register_preview(&mut e, "a.txt");
    e.handle_preview(Completion {
        ticket: ticket.clone(),
        outcome: Outcome::failed(FailureKind::Io, "no such file"),
    });
    assert!(
        matches!(e.preview_loads.get(&path), Some(Load::Failed { .. })),
        "the failure is owned, not a forever-empty success"
    );
    assert!(
        !e.previews.contains_key(&path),
        "a failure is not an empty file"
    );
    assert!(matches!(
        e.picker_preview().unwrap().2,
        PreviewSource::Failed(_)
    ));
    // Later frames preserve the terminal error instead of retrying.
    assert!(
        matches!(e.preview_loads.get(&path), Some(Load::Failed { .. })),
        "no silent per-frame retry"
    );
    // closing forgets the negative cache…
    e.close_picker();
    assert!(!e.previews.contains_key(&path), "negative cache forgotten");
    assert!(!e.preview_loads.contains_key(&path));
    // …so an explicit reopen can retry with a fresh request
    e.open_picker(Kind::Search);
    e.picker
        .as_mut()
        .unwrap()
        .picker
        .append(vec![item("a.txt")]);
    rank_fixture(&mut e);
    let (fresh, path) = register_preview(&mut e, "a.txt");
    assert_ne!(
        fresh.request, ticket.request,
        "a reopen allocates a new request"
    );
    // the old ticket's late success cannot touch the fresh request
    e.handle_preview(Completion {
        ticket: ticket.clone(),
        outcome: Outcome::Success(String::from("late").into()),
    });
    assert!(
        matches!(e.preview_loads.get(&path), Some(Load::Running(t)) if *t == fresh),
        "stale preview results are rejected"
    );
    // the fresh one succeeds and its cache survives the next close
    e.handle_preview(Completion {
        ticket: fresh,
        outcome: Outcome::Success(String::from("body").into()),
    });
    let entry = e.previews.get(&path).unwrap();
    assert_eq!(entry.rope.to_string(), "body");
    // (highlighter presence is strop-syntax's language call, not
    // this lifecycle contract)
    e.close_picker();
    assert!(
        e.previews.contains_key(&path),
        "positive caches survive reopen"
    );
    assert!(matches!(e.preview_loads.get(&path), Some(Load::Ready(_))));
}

#[test]
fn close_with_a_full_semantic_lane_still_publishes_terminal() {
    let (tx, rx) = crate::editor::events::channel();
    let mut e = editor();
    e.app_tx = Some(tx.clone());
    e.open_picker(Kind::Files);
    for _ in 0..crate::editor::events::MAX_SEMANTIC_EVENTS {
        tx.send(crate::editor::events::AppEvent::EditorKey(
            strop_core::frontend_input::Key::Char('x'),
        ))
        .unwrap();
    }
    e.close_picker();
    let terminal = std::iter::from_fn(|| rx.try_recv().ok()).any(|event| {
        matches!(
            event,
            crate::editor::events::AppEvent::Picker(PickerEvent {
                msg: PickerMsg::Finished(_),
                ..
            })
        )
    });
    assert!(terminal, "the cancelled request publishes Finished");
}

#[test]
fn preview_empty_success_is_ready_and_cancel_does_not_resubmit() {
    let mut e = editor();
    e.open_picker(Kind::Search);
    e.picker
        .as_mut()
        .unwrap()
        .picker
        .append(vec![item("empty.txt")]);
    rank_fixture(&mut e);
    let (ticket, path) = register_preview(&mut e, "empty.txt");
    // an empty file is a real success: cached Ready
    e.handle_preview(Completion {
        ticket,
        outcome: Outcome::Success(String::new().into()),
    });
    assert!(matches!(e.preview_loads.get(&path), Some(Load::Ready(_))));
    assert!(e.previews[&path].rope.to_string().is_empty());

    // an explicit cancellation settles the load and frames do not
    // silently retry it
    let (ticket, path) = register_preview(&mut e, "cancelled.txt");
    e.handle_preview(Completion {
        ticket,
        outcome: Outcome::Cancelled(CancelReason::Dismissed),
    });
    assert!(
        matches!(e.preview_loads.get(&path), Some(Load::Cancelled { .. })),
        "cancelled settles terminal"
    );
    e.picker
        .as_mut()
        .unwrap()
        .picker
        .append(vec![item("cancelled.txt")]);
    rank_fixture(&mut e);
    e.picker.as_mut().unwrap().picker.move_by(1);
    let preview = e.picker_preview().unwrap();
    assert!(
        matches!(preview.2, PreviewSource::Cancelled(CancelReason::Dismissed)),
        "cancelled previews are terminal, not perpetually loading"
    );
    assert!(
        matches!(e.preview_loads.get(&path), Some(Load::Cancelled { .. })),
        "no new request was enqueued"
    );
}

#[test]
fn read_preview_types_every_miss() {
    let (token, _cancel) = strop_core::worker::CancelToken::standalone();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ok.txt"), "body\n").unwrap();
    match read_preview(&dir.path().join("ok.txt"), &token) {
        Outcome::Success(text) => assert_eq!(text.rope, "body\n"),
        outcome => panic!("readable file must succeed: {outcome:?}"),
    }
    // empty file: a real success, not a failure
    std::fs::write(dir.path().join("empty.txt"), "").unwrap();
    let empty = read_preview(&dir.path().join("empty.txt"), &token);
    assert!(matches!(&empty, Outcome::Success(t) if t.rope.len_bytes() == 0));
    // over the 512 KiB cap
    std::fs::write(dir.path().join("big.txt"), vec![b'x'; 512 * 1024 + 1]).unwrap();
    let big = read_preview(&dir.path().join("big.txt"), &token);
    assert!(
        matches!(&big, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable)
    );
    // a directory is not a preview target
    let dir_result = read_preview(dir.path(), &token);
    assert!(
        matches!(&dir_result, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable)
    );
    // missing path
    let missing = read_preview(&dir.path().join("missing.txt"), &token);
    assert!(matches!(&missing, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Io));
    // invalid UTF-8
    std::fs::write(dir.path().join("bin.bin"), [0xff, 0xfe, 0xfd]).unwrap();
    let bin = read_preview(&dir.path().join("bin.bin"), &token);
    assert!(matches!(&bin, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Io));

    let (cancelled, cancel) = strop_core::worker::CancelToken::standalone();
    cancel.cancel(CancelReason::Superseded);
    assert!(matches!(
        read_preview(&dir.path().join("ok.txt"), &cancelled),
        Outcome::Cancelled(CancelReason::Superseded)
    ));
}
