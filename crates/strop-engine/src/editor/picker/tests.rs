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

    #[test]
    fn paste_into_picker_edits_the_field_not_the_document() {
        // user report: bracketed paste in a picker or the connect-to-
        // remote field was silently dropped (or worse, could land in
        // the buffer behind the card)
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Files);
        e.paste_bracketed("main");
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "main");
        assert_eq!(e.buf().text(), "x\n", "the document is untouched");
        // caret placement is honored
        e.feed(crate::editor::Key::Left);
        e.feed(crate::editor::Key::Left);
        e.paste_bracketed("__");
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "ma__in");
        // paste works in the field's normal mode too (the ex line's
        // pending reducer pastes the same way)
        e.feed(crate::editor::Key::Esc);
        e.feed_text("0");
        e.paste_bracketed("#");
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "#ma__in");
    }

    #[test]
    fn paste_newline_into_picker_is_rejected_with_a_message() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Files);
        e.paste_bracketed("a\nb");
        assert_eq!(
            e.message, "picker input cannot contain a newline",
            "a refused paste says so — silence reads as a broken terminal"
        );
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "");
    }

    #[test]
    fn jumps_picker_walks_history_and_ctrl_o_returns() {
        // 0047 §2: the jumplist as a menu — past newest-first, the
        // current position marked, accepting an entry lands there and
        // records the spot so ctrl-o returns.
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\nfour\n"));
        e.feed_text("G"); // jump to line 4
        e.feed_text("gg"); // jump back to line 1
        e.open_picker(Kind::Jumps);
        e.wait_picker();
        let glue = e.picker.as_ref().unwrap();
        let texts: Vec<&str> = glue.picker.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts.len(), 3, "two jumps + the current row: {texts:?}");
        assert!(
            texts[0].contains(":4") && texts[0].contains("four"),
            "newest past entry first: {texts:?}"
        );
        assert!(
            texts[2].starts_with("> "),
            "the current row is marked: {texts:?}"
        );
        e.feed(crate::editor::Key::Enter);
        assert!(!e.picker_open(), "accept closes the picker");
        assert_eq!(e.buf().line_of(e.head()), 3, "landed on the accepted entry");
        e.feed(crate::editor::Key::CtrlO);
        assert_eq!(
            e.buf().line_of(e.head()),
            0,
            "ctrl-o returns to where the picker was opened"
        );
    }

    #[test]
    fn jumps_picker_filters_dead_documents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.txt");
        std::fs::write(&path, "x\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
        e.open_fixture(&path).unwrap();
        e.feed_text("G"); // a jump inside other.txt
        let dead = e.current();
        e.close_buffer(true);
        assert!(e.docs.get(dead).is_none(), "the document is gone");
        e.open_picker(Kind::Jumps);
        e.wait_picker();
        let glue = e.picker.as_ref().unwrap();
        assert!(
            glue.picker
                .items
                .iter()
                .all(|i| !i.text.contains("other.txt")),
            "dead-document entries are absent: {:?}",
            glue.picker
                .items
                .iter()
                .map(|i| i.text.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn remote_hosts_enter_carries_unmatched_text_into_the_address_box() {
        // 0.21.0 field report: a typed hostname that matched no listed
        // destination made Enter a no-op. Now the pinned "Add a host…"
        // row survives filtering and Enter carries the text into the
        // address box as a draft — never a blind connect to a host
        // literally named like the filter text.
        let mut e = Editor::new(Buffer::from_text("x\n"));
        let mut picker = Picker::new(
            Kind::RemoteHosts,
            vec![Item {
                badge: None,
                text: "Add a host\u{2026}".into(),
                payload: Payload::RemoteConnect,
            }],
            false,
        );
        picker.pinned_tail = 1;
        e.set_picker(PickerGlue::diagnostics(picker));
        e.feed_text("ewosd-tt-925");
        e.wait_picker();
        assert_eq!(
            e.picker
                .as_ref()
                .unwrap()
                .picker
                .current()
                .map(|i| i.text.as_str()),
            Some("Add a host\u{2026}"),
            "the pinned row survives a dead query"
        );
        e.accept_current_picker();
        let glue = e.picker.as_ref().expect("the address box opens");
        assert_eq!(glue.picker.kind, Kind::RemoteAddress);
        assert_eq!(
            glue.picker.input.text, "ewosd-tt-925",
            "the typed destination arrives as the address draft"
        );
    }

    #[test]
    fn remote_hosts_empty_enter_opens_the_address_box() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        let mut picker = Picker::new(
            Kind::RemoteHosts,
            vec![Item {
                badge: None,
                text: "Add a host\u{2026}".into(),
                payload: Payload::RemoteConnect,
            }],
            false,
        );
        picker.pinned_tail = 1;
        e.set_picker(PickerGlue::diagnostics(picker));
        e.wait_picker();
        e.accept_current_picker();
        assert_eq!(
            e.picker.as_ref().map(|glue| glue.picker.kind),
            Some(Kind::RemoteAddress),
            "empty input on the pinned row still opens the address box"
        );
    }

    #[test]
    fn paste_into_remote_address_field_is_accepted_verbatim() {
        // the connect-to-remote case: an ssh://user@host:port/path URL
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::RemoteAddress);
        e.paste_bracketed("ssh://user@example.com:2222/tmp/dir");
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.text,
            "ssh://user@example.com:2222/tmp/dir"
        );
    }

    /// 0051 §3: the qualifier language end to end — language filter +
    /// literal content, rows install, accept opens the hit.
    #[test]
    fn grep_query_language_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn send() {}\nlet x = send;\n").unwrap();
        std::fs::write(dir.path().join("b.py"), "send = 1\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_picker(Kind::Search);
        e.feed_text("language:rust send");
        e.wait_picker();
        let p = &e.picker.as_ref().unwrap().picker;
        assert_eq!(p.items.len(), 2, "rust-only hits: {:?}", p.items.len());
        assert_eq!(p.rows.len(), 2, "rows install for grep hits");
        assert!(
            p.items.iter().all(|i| !format!("{i:?}").contains("b.py")),
            "no python hit leaks through"
        );
        e.feed(crate::editor::Key::Enter);
        assert!(!e.picker_open(), "Enter accepts the first hit");
    }

    /// An invalid query never broadens the search (0051 §4): no worker
    /// launches, the diagnostic shows, and previous state stays.
    #[test]
    fn invalid_query_never_launches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn send() {}\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_picker(Kind::Search);
        e.feed_text("langauge:rust send");
        let p = &e.picker.as_ref().unwrap().picker;
        assert!(
            p.error
                .as_ref()
                .is_some_and(|e| e.contains("unknown qualifier")),
            "the correction shows: {:?}",
            p.error
        );
        assert_eq!(p.items.len(), 0, "nothing searched");
    }

    /// 0051 R02: ctrl-space offers manual suggestions from the parse
    /// position; Enter replaces the exact token span.
    #[test]
    fn ctrl_space_suggests_and_enter_completes() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Search);
        e.feed_text("lang");
        e.feed(crate::editor::Key::CtrlSpace);
        let glue = e.picker.as_ref().unwrap();
        let list = glue.suggestions.as_ref().expect("suggestions open");
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].insert, "language:");
        e.feed(crate::editor::Key::Enter);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.text,
            "language:",
            "the span replaced"
        );
        // Esc in the suggestion list dismisses IT, not the picker
        e.feed(crate::editor::Key::CtrlSpace);
        e.feed(crate::editor::Key::Esc);
        assert!(e.picker_open(), "esc dismissed only the list");
        assert!(e.picker.as_ref().unwrap().suggestions.is_none());
    }

    /// The replacement With field and non-query fields keep literal
    /// semantics — no suggestions there.
    #[test]
    fn suggestions_stay_out_of_literal_fields() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_search(true);
        e.feed_text("foo");
        e.feed(crate::editor::Key::Tab); // With field
        e.feed_text("lang");
        e.feed(crate::editor::Key::CtrlSpace);
        assert!(
            e.picker.as_ref().unwrap().suggestions.is_none(),
            "no suggestions in the With field"
        );
    }

    /// 0051 R03: the search-options selector shows live values and toggles
    /// the session default.
    #[test]
    fn search_options_toggle_the_session_default() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_search_options();
        e.wait_picker();
        let p = &e.picker.as_ref().unwrap().picker;
        assert!(p.items[0].text.contains("hidden (dotfiles): include"));
        e.feed(crate::editor::Key::Enter);
        assert!(!e.config.search_show_hidden, "the toggle flipped");
        let p = &e.picker.as_ref().unwrap().picker;
        assert!(p.items[0].text.contains("hidden (dotfiles): exclude"));
        // and the qualifier still overrides the session default per query
        e.config.search_show_hidden = true;
        let policy = strop_picker::SelectionPolicy {
            hidden: e.config.search_show_hidden,
            respect_ignore: e.config.search_respect_ignore,
        };
        let query = strop_picker::query::SearchQuery::parse("hidden:exclude");
        let plan = strop_picker::query::FileSelectionPlan::compile(&query).unwrap();
        assert!(!policy.effective(&plan).hidden, "the query qualifier wins");
    }
}
// Worker lifecycle: injected terminal events (deterministic — no real
// rg, no sleeps, no HOME assumptions; the supervisor's own decisions
// are unit-tested in strop-picker).
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
