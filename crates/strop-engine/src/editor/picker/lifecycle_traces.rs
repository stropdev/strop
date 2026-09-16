//! 0063 §6.6 production correspondence: SearchLifecycle.tla's transition
//! vocabulary replayed through the actual admission/publication
//! handlers — `handle_picker_event`, `handle_lsp_event`'s
//! workspace-symbols merge and the warm-queue drain — asserting the
//! model's named invariants (RowsCurrent, CompletionHonest,
//! WarmBounded) at each step, including the negative traces the model
//! rejects (a retired generation's late publish, a truncated
//! completion). This is correspondence, not a re-test of the e2e: the
//! seams are driven directly, streams hand-armed like the established
//! worker-lifecycle tests (deterministic — no rg, no sleeps).
//!
//! The publication decisions under test are the verified kernel's
//! (`strop_core::searchguard`): each guard site below calls it, so a
//! trace that passes here exercises the same function Verus proves.

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::editor::lsp::attach::AttachKey;
    use std::path::PathBuf;
    use strop_core::worker::{Outcome, Ticket, WorkerId};
    use strop_core::Buffer;
    use strop_picker::{Item, Payload, PickerMsg};
    use strop_workspace::Filesystem;

    fn item(text: &str) -> Item {
        Item {
            badge: None,
            text: text.into(),
            payload: Payload::File(PathBuf::from(text)),
        }
    }

    /// Model `TypeQuery`: a fresh generation's request registers before
    /// launch and takes ownership of the stream (the query path's row
    /// purge is its own action, covered by the query tests; the seam
    /// under replay here is the publication guard).
    fn arm_stream(e: &mut Editor) -> Ticket<PickerKey> {
        let picker = e.picker.as_ref().map(|glue| glue.id).unwrap();
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

    /// Settle the ranking actor only: hand-armed streams hold no
    /// retained worker channel, so `wait_picker`'s stream loop does
    /// not apply (the established worker-lifecycle tests settle the
    /// same way once their injected terminal lands).
    fn settle_ranking(e: &mut Editor) {
        while e
            .picker
            .as_ref()
            .is_some_and(|glue| glue.rank_pending.is_some())
        {
            let event = e
                .picker_ranking
                .rx
                .as_ref()
                .expect("local ranking channel")
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("ranking settled");
            e.handle_picker_ranking(event);
        }
    }

    fn row_texts(e: &Editor) -> Vec<String> {
        e.picker
            .as_ref()
            .unwrap()
            .picker
            .rows
            .iter()
            .map(|row| {
                e.picker.as_ref().unwrap().picker.items[row.item]
                    .text
                    .clone()
            })
            .collect()
    }

    /// The merge seam's publication set: every item the picker holds,
    /// before ranking narrows the presentation (the ranked `rows` view
    /// would hide a wrongly-merged row that fails the query's text
    /// match — the mutant survives there).
    fn item_texts(e: &Editor) -> Vec<String> {
        e.picker
            .as_ref()
            .unwrap()
            .picker
            .items
            .iter()
            .map(|item| item.text.clone())
            .collect()
    }

    /// Trace: OpenPicker, TypeQuery (g1), ProviderPartial(g1),
    /// TypeQuery (g2 retires g1), late ProviderPartial/ProviderDone(g1)
    /// — refused — ProviderPartial(g2), ProviderDone(g2).
    /// Invariant: RowsCurrent (no retired-query publication).
    #[test]
    fn retired_stream_generation_never_publishes() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Search);
        let g1 = arm_stream(&mut e);
        // ProviderPartial(g1): the live generation publishes.
        e.handle_picker_event(PickerEvent {
            ticket: g1.clone(),
            msg: PickerMsg::Items(vec![item("gen1.rs")].into()),
        });
        settle_ranking(&mut e);
        assert_eq!(
            row_texts(&e),
            vec!["gen1.rs".to_string()],
            "RowsCurrent: the live generation's rows publish"
        );
        // TypeQuery: generation 2 retires generation 1.
        let g2 = arm_stream(&mut e);
        // Late arrivals from the retired generation, in every shape the
        // stream can take: the model rejects each transition and so
        // must the handler.
        for msg in [
            PickerMsg::Items(vec![item("retired.rs")].into()),
            PickerMsg::Warning("retired truncation".into()),
            PickerMsg::Finished(Outcome::Success(())),
        ] {
            e.handle_picker_event(PickerEvent {
                ticket: g1.clone(),
                msg,
            });
        }
        settle_ranking(&mut e);
        let glue = e.picker.as_ref().unwrap();
        assert_eq!(
            row_texts(&e),
            vec!["gen1.rs".to_string()],
            "RowsCurrent: a retired generation never appends"
        );
        assert!(
            glue.picker.warning.is_none(),
            "a retired generation's truncation never surfaces"
        );
        assert!(
            glue.picker.streaming,
            "ProviderDone(g1) cannot publish generation 2's completion"
        );
        // ProviderPartial(g2) + ProviderDone(g2): publish, then an
        // honest completion (no truncation on record).
        e.handle_picker_event(PickerEvent {
            ticket: g2.clone(),
            msg: PickerMsg::Items(vec![item("gen2.rs")].into()),
        });
        e.handle_picker_event(PickerEvent {
            ticket: g2.clone(),
            msg: PickerMsg::Finished(Outcome::Success(())),
        });
        settle_ranking(&mut e);
        let glue = e.picker.as_ref().unwrap();
        assert_eq!(
            row_texts(&e),
            vec!["gen1.rs".to_string(), "gen2.rs".to_string()],
            "the owning generation publishes"
        );
        assert!(!glue.picker.streaming, "the live terminal settles");
        assert!(
            glue.picker.error.is_none() && glue.picker.warning.is_none(),
            "CompletionHonest: the untruncated live terminal completes clean"
        );
    }

    /// Trace: OpenPicker (workspace symbols), ProviderPartial(g),
    /// TypeQuery (g+1 retires g), late ProviderPartial(g) — refused —
    /// ProviderPartial(g+1). Same RowsCurrent invariant, second guard
    /// site: `merge_workspace_symbols` via `handle_lsp_event`.
    #[test]
    fn retired_wsymbols_generation_never_merges() {
        use strop_lsp::protocol::{ProtoSymbol, ServerColumn, ServerLocation, ServerPosition};
        use strop_lsp::{LspEvent, ServerId};

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn wrap() {}\n").unwrap();
        let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
        e.open_picker(Kind::WorkspaceSymbols);
        e.wait_picker();
        let g = e.picker.as_ref().unwrap().wsymbols_generation;
        let symbol = |name: &str| ProtoSymbol {
            name: name.into(),
            container: String::new(),
            kind: "Function".into(),
            location: ServerLocation {
                doc: strop_workspace::ResourceLocation::local(dir.path().join("lib.rs")),
                position: ServerPosition {
                    line: strop_core::id::LineIndex::new(0),
                    column: ServerColumn::new(3),
                },
            },
        };
        // ProviderPartial(g): the live generation merges.
        e.handle_lsp_event(LspEvent::WorkspaceSymbols {
            server: ServerId::new(9),
            generation: g,
            symbols: vec![symbol("semantic_hit")],
        });
        e.wait_picker();
        assert!(
            item_texts(&e)
                .iter()
                .any(|text| text.starts_with("semantic_hit  ")),
            "RowsCurrent: the live generation merges"
        );
        // TypeQuery: typing retires generation g.
        e.paste_bracketed("wrap");
        e.wait_picker();
        let retired = g;
        let g = e.picker.as_ref().unwrap().wsymbols_generation;
        assert_eq!(g, retired + 1, "the query change bumped the generation");
        // ProviderPartial(retired) landing late: the model rejects it.
        e.handle_lsp_event(LspEvent::WorkspaceSymbols {
            server: ServerId::new(9),
            generation: retired,
            symbols: vec![symbol("retired_hit")],
        });
        e.wait_picker();
        assert!(
            !item_texts(&e)
                .iter()
                .any(|text| text.starts_with("retired_hit")),
            "RowsCurrent: a retired generation's reply never merges"
        );
        // ProviderPartial(g) for the new live generation: merges.
        e.handle_lsp_event(LspEvent::WorkspaceSymbols {
            server: ServerId::new(9),
            generation: g,
            symbols: vec![symbol("wrap_semantic")],
        });
        e.wait_picker();
        assert!(
            item_texts(&e)
                .iter()
                .any(|text| text.starts_with("wrap_semantic  ")),
            "the new live generation merges"
        );
    }

    /// Trace: TypeQuery (g), TruncateIndex, ProviderDone(g).
    /// Invariant: CompletionHonest — the model keeps completeF false on
    /// a truncated terminal; the handler's correspondence is that the
    /// bound hit stays visibly on the settled picker (never a silent
    /// empty success), which the kernel's decision mirrors.
    #[test]
    fn truncated_completion_stays_visibly_incomplete() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Search);
        let g1 = arm_stream(&mut e);
        // TruncateIndex: the source reports its bounds were hit.
        e.handle_picker_event(PickerEvent {
            ticket: g1.clone(),
            msg: PickerMsg::Warning(
                "workspace symbols: syntax tier truncated at its bounds; narrow the scope for full coverage".into(),
            ),
        });
        // ProviderDone(g): the stream settles, the claim does not.
        e.handle_picker_event(PickerEvent {
            ticket: g1.clone(),
            msg: PickerMsg::Finished(Outcome::Success(())),
        });
        let glue = e.picker.as_ref().unwrap();
        assert!(!glue.picker.streaming, "the terminal settles the stream");
        assert!(
            glue.picker.warning.is_some(),
            "CompletionHonest: a truncated completion stays visibly incomplete"
        );
        assert!(glue.picker.error.is_none(), "a bound hit is not a failure");
        // The kernel decision agrees with the model's completeF clause:
        // honest iff the live generation's terminal AND no truncation.
        let gen = g1.request.get();
        assert!(!strop_core::searchguard::completion_is_honest(
            gen, gen, true
        ));
        assert!(strop_core::searchguard::completion_is_honest(
            gen, gen, false
        ));
        assert!(!strop_core::searchguard::completion_is_honest(
            gen - 1,
            gen,
            false
        ));
    }

    /// Trace: WarmStart ×3 (pre-filled), WarmEnqueue ×3 — exactly one
    /// further WarmStart, the rest wait — WarmDone, one refill
    /// WarmStart, ClosePopup (the queue dies with the surface).
    /// Invariant: WarmBounded (warm-up concurrency within the bound,
    /// started only while the symbols surface is open).
    #[test]
    fn warm_drain_holds_the_bound_and_dies_with_the_surface() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
        e.lsp_state.attach.enabled = true;
        e.open_picker(Kind::WorkspaceSymbols);
        e.wait_picker();
        // WarmStart ×3 already in flight.
        for n in 0..3u64 {
            e.lsp_state.attach.pending.insert(
                AttachKey {
                    target: Filesystem::Local,
                    language: "rust".into(),
                    path: dir.path().join(format!("inflight{n}")),
                },
                WorkerId::new(90 + n),
            );
        }
        // WarmEnqueue ×3 with one slot free: exactly one WarmStart.
        e.lsp_warm_scope_projects(
            (0..3)
                .map(|n| (dir.path().join(format!("p{n}")), "Cargo.toml".to_string()))
                .collect(),
        );
        assert_eq!(
            e.lsp_state.attach.pending.len(),
            4,
            "WarmBounded: the one free slot fills, no more"
        );
        assert_eq!(
            e.lsp_state.attach.warm_queue.len(),
            2,
            "the rest of the queue waits"
        );
        // WarmDone frees a slot: the drain starts exactly one more.
        let freed = e.lsp_state.attach.pending.keys().next().unwrap().clone();
        e.lsp_state.attach.pending.remove(&freed);
        e.lsp_warm_scope_projects(Vec::new());
        assert_eq!(
            e.lsp_state.attach.pending.len(),
            4,
            "WarmBounded: the refill holds the bound"
        );
        assert_eq!(e.lsp_state.attach.warm_queue.len(), 1);
        // ClosePopup: warm-up is on demand — the queue dies with the
        // symbols surface.
        e.open_picker(Kind::Files);
        assert!(
            e.lsp_state.attach.warm_queue.is_empty(),
            "installing another surface ends warm-up"
        );
    }
}
