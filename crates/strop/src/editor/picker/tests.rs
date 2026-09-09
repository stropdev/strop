//! Picker modal-field and navigation behavior (the input box is a
//! real modal field: Esc/i, jk on results, arrows move the caret).

#[cfg(test)]
mod picker_tests {
    use super::super::*;
    use strop_core::Buffer;

    #[test]
    fn picker_field_is_modal() {
        // rootle's input boxes: Esc enters normal mode on the field,
        // keys edit the query, i returns to insert, Esc closes
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Files);
        e.feed_text("main");
        e.feed(crate::editor::Key::Esc);
        assert!(e.picker_open(), "esc once: picker stays open");
        e.feed_text("0x"); // to 0, delete 'm'
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "ain");
        e.feed(crate::editor::Key::Esc);
        assert!(!e.picker_open(), "esc twice closes");
    }

    #[test]
    fn picker_normal_mode_jk_walk_results() {
        // Esc into the field's normal mode; j/k move the selection,
        // not the text caret (user report: only Tab/arrows navigated)
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        for f in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            std::fs::write(dir.path().join(f), "x\n").unwrap();
            e.open_fixture(&dir.path().join(f)).unwrap();
        }
        e.open_picker(Kind::Buffers);
        e.wait_picker();
        let sel = |e: &Editor| e.picker.as_ref().unwrap().picker.selected;
        assert_eq!(sel(&e), 0);
        e.feed(crate::editor::Key::Esc); // normal mode on the field
        e.feed_text("jj");
        assert_eq!(sel(&e), 2, "j moved the selection down twice");
        e.feed_text("k");
        assert_eq!(sel(&e), 1);
        e.feed_text("i"); // back to insert
        e.feed_text("j"); // types into the query instead
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.text,
            "j",
            "insert mode: j filters"
        );
    }

    #[test]
    fn picker_arrows_navigate_and_move_caret() {
        // user report: physical arrows did nothing in pickers — the
        // translation layer dropped KeyCode::Up/Down entirely
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        for f in ["aa.txt", "ab.txt", "ac.txt"] {
            std::fs::write(dir.path().join(f), "x\n").unwrap();
            e.open_fixture(&dir.path().join(f)).unwrap();
        }
        e.open_picker(Kind::Buffers);
        e.feed_text("a");
        e.wait_picker();
        e.feed(crate::editor::Key::Down);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.selected,
            1,
            "Down walks results"
        );
        e.feed(crate::editor::Key::Up);
        assert_eq!(e.picker.as_ref().unwrap().picker.selected, 0);
        e.feed(crate::editor::Key::Left);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.cursor,
            0,
            "Left moves the caret"
        );
        e.feed(crate::editor::Key::Right);
        assert_eq!(e.picker.as_ref().unwrap().picker.input.cursor, 1);
    }
}

// Worker lifecycle: injected terminal events (deterministic — no real
// rg, no sleeps, no HOME assumptions; the supervisor's own decisions
// are unit-tested in strop-picker).
#[cfg(test)]
mod worker_lifecycle_tests {
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
            text: text.into(),
            payload: Payload::File(PathBuf::from(text)),
        }
    }

    fn rank_fixture(e: &mut Editor) {
        let picker = &mut e.picker.as_mut().unwrap().picker;
        let ranking = strop_picker::rank::rank(&picker.filter_request(), || false).unwrap();
        assert!(picker.install_ranking(ranking));
    }

    /// Register a grep-stream request without launching rg: the glue
    /// only needs the ticket; events are injected by hand.
    fn register_stream(e: &mut Editor) -> Ticket<PickerKey> {
        e.open_picker(Kind::Grep);
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
    fn register_preview(e: &mut Editor, rel: &str) -> (Ticket<PreviewKey>, PathBuf) {
        let path = e.cwd.join(rel);
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
        assert!(matches!(glue.worker, Some(PickerWorker::Files(_))));
        e.close_picker();
        assert!(!e.picker_open(), "close settles the picker");
        // a terminal event from the cancelled walk cannot resurrect it
        e.handle_picker_event(PickerEvent {
            ticket: Ticket {
                request: strop_core::worker::WorkerId::new(1),
                key: PickerKey {
                    picker: PickerId(strop_core::worker::WorkerId::new(2)),
                    cwd: e.cwd.clone(),
                },
            },
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
    fn preview_failure_is_visible_and_retryable_after_reopen() {
        let mut e = editor();
        e.open_picker(Kind::Grep);
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
        e.open_picker(Kind::Grep);
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
    fn preview_empty_success_is_ready_and_cancel_does_not_resubmit() {
        let mut e = editor();
        e.open_picker(Kind::Grep);
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
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.txt"), "body\n").unwrap();
        match read_preview(&dir.path().join("ok.txt")) {
            Outcome::Success(text) => assert_eq!(text.rope, "body\n"),
            outcome => panic!("readable file must succeed: {outcome:?}"),
        }
        // empty file: a real success, not a failure
        std::fs::write(dir.path().join("empty.txt"), "").unwrap();
        let empty = read_preview(&dir.path().join("empty.txt"));
        assert!(matches!(&empty, Outcome::Success(t) if t.rope.len_bytes() == 0));
        // over the 512 KiB cap
        std::fs::write(dir.path().join("big.txt"), vec![b'x'; 512 * 1024 + 1]).unwrap();
        let big = read_preview(&dir.path().join("big.txt"));
        assert!(
            matches!(&big, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable)
        );
        // a directory is not a preview target
        let dir_result = read_preview(dir.path());
        assert!(
            matches!(&dir_result, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable)
        );
        // missing path
        let missing = read_preview(&dir.path().join("missing.txt"));
        assert!(
            matches!(&missing, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Io)
        );
        // invalid UTF-8
        std::fs::write(dir.path().join("bin.bin"), [0xff, 0xfe, 0xfd]).unwrap();
        let bin = read_preview(&dir.path().join("bin.bin"));
        assert!(matches!(&bin, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Io));
    }
}
