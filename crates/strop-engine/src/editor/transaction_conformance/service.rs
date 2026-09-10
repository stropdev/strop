use super::*;

// ---- the service stream: Arm + Deliver through Editor::apply ---------------

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum ServiceOp {
    Open {
        text: usize,
    },
    Switch {
        slot: usize,
    },
    Split {
        slot: usize,
    },
    ClosePane,
    CloseDoc {
        slot: usize,
    },
    Arm {
        slot: usize,
    },
    Deliver {
        ticket: usize,
        anchors: Vec<(usize, usize)>,
        snippets: Vec<usize>,
        undo_open: bool,
    },
}

/// Model-only identities while generating a recipe; execution replaces these
/// with the real arena's keys before checking any production observation.
fn symbolic_id(slot: usize) -> DocumentId {
    let fields = [
        ("slot", u32::try_from(slot).expect("bounded model")),
        ("generation", 0),
    ];
    serde::Deserialize::deserialize(serde::de::value::MapDeserializer::<
        _,
        serde::de::value::Error,
    >::new(fields.into_iter()))
    .expect("symbolic model identity")
}

/// The model's whole editor: one text+revision per document slot, the
/// live ids, the panes, the armed tickets, and the ids of every closed
/// document (dead ids must stay dead forever — the incarnation oracle).
struct ServiceState {
    texts: Vec<String>,
    revisions: Vec<u64>,
    ids: Vec<Option<DocumentId>>,
    panes: Vec<usize>,
    active: usize,
    /// (slot, armed id when an editor minted it, base revision).
    tickets: Vec<Option<(usize, Option<DocumentId>, u64)>>,
    /// ids of closed documents: never resolve again, even after the
    /// arena reuses their slot index at a new generation.
    dead_ids: Vec<DocumentId>,
}

impl ServiceState {
    fn new() -> Self {
        Self {
            texts: vec![TEXTS[0].to_string()],
            revisions: vec![0],
            ids: vec![Some(symbolic_id(0))],
            panes: vec![0],
            active: 0,
            tickets: Vec::new(),
            dead_ids: Vec::new(),
        }
    }
    fn live(&self) -> Vec<usize> {
        (0..self.ids.len())
            .filter(|&slot| self.ids[slot].is_some())
            .collect()
    }
    fn slot_of(&self, id: DocumentId) -> Option<usize> {
        (0..self.ids.len()).find(|&slot| self.ids[slot] == Some(id))
    }
}

/// One step of the service protocol against the model — and, when an
/// Editor is supplied, the same step on the real API with the outcome
/// asserted. The generator runs this with `editor: None`.
fn step_service(
    op: &ServiceOp,
    model: &mut ServiceState,
    mut editor: Option<&mut Editor>,
) -> Result<(), String> {
    match op {
        ServiceOp::Open { text } => {
            let slot = model.texts.len();
            model.texts.push(TEXTS[*text].to_string());
            model.revisions.push(0);
            model.ids.push(Some(symbolic_id(slot)));
            if let Some(editor) = editor.as_deref_mut() {
                let id = editor
                    .docs
                    .insert(Document::new(Buffer::from_text(TEXTS[*text])));
                model.ids[slot] = Some(id);
                let document = editor.docs.get(id).expect("freshly inserted");
                must!(
                    document.buf.text() == TEXTS[*text],
                    "opened document text diverged"
                );
                must!(
                    document.buf.revision().get() == 0,
                    "opened at revision != 0"
                );
            }
        }
        ServiceOp::Switch { slot } => {
            let Some(id) = model.ids.get(*slot).copied().flatten() else {
                return Ok(()); // dead slot: never point a pane at a corpse
            };
            model.panes[model.active] = *slot;
            if let Some(editor) = editor.as_deref_mut() {
                editor.switch_to(id);
                must!(
                    editor.current() == id,
                    "switch_to did not focus the document"
                );
            }
        }
        ServiceOp::Split { slot } => {
            let Some(id) = model.ids.get(*slot).copied().flatten() else {
                return Ok(());
            };
            model.panes.push(*slot);
            model.active = model.panes.len() - 1;
            if let Some(editor) = editor.as_deref_mut() {
                editor.split_document(true, id);
                must!(
                    editor.panes.len() == model.panes.len(),
                    "split did not add a pane"
                );
                must!(
                    editor.active_pane == model.active,
                    "split did not focus the new pane"
                );
                must!(
                    editor.panes[model.active].doc == id,
                    "new pane shows the wrong doc"
                );
            }
        }
        ServiceOp::ClosePane => {
            if model.panes.len() > 1 {
                model.panes.remove(model.active);
                model.active = model.active.min(model.panes.len() - 1);
                if let Some(editor) = editor.as_deref_mut() {
                    editor.close_pane_or_buffer(true);
                    must!(
                        editor.panes.len() == model.panes.len(),
                        "pane count diverged"
                    );
                    must!(editor.active_pane == model.active, "active pane diverged");
                }
            } else if model.live().len() > 1 {
                // the last pane's close is document close; which document
                // takes over is editor policy (mru) — observe it, then
                // check the protocol property: the next focus is live.
                let closed = model.panes[model.active];
                let closed_id = model.ids[closed].expect("live slot");
                model.ids[closed] = None;
                model.dead_ids.push(closed_id);
                if let Some(editor) = editor.as_deref_mut() {
                    editor.close_pane_or_buffer(true);
                    must!(
                        editor.docs.get(closed_id).is_none(),
                        "closed id still resolves"
                    );
                    let next = editor.current();
                    must!(editor.docs.get(next).is_some(), "close left a dead focus");
                    model.panes[model.active] = model.slot_of(next).expect("live focus in model");
                } else {
                    model.panes[model.active] = model.live()[0];
                }
            }
            // else: closing the only pane of the only document quits the
            // session — outside the protocol; the op is a no-op.
        }
        ServiceOp::CloseDoc { slot } => {
            let Some(id) = model.ids.get(*slot).copied().flatten() else {
                return Ok(()); // already dead
            };
            if model.live().len() <= 1 {
                return Ok(()); // would quit the session
            }
            let next_slot = if let Some(editor) = editor.as_deref_mut() {
                editor.switch_to(id);
                editor.close_buffer(true);
                must!(editor.docs.get(id).is_none(), "closed id still resolves");
                let next = editor.current();
                must!(editor.docs.get(next).is_some(), "close left a dead focus");
                model.slot_of(next).expect("live focus in model")
            } else {
                model
                    .live()
                    .into_iter()
                    .find(|candidate| candidate != slot)
                    .expect("survivor")
            };
            model.panes[model.active] = next_slot;
            for pane in &mut model.panes {
                if *pane == *slot {
                    *pane = next_slot;
                }
            }
            model.ids[*slot] = None;
            model.dead_ids.push(id);
        }
        ServiceOp::Arm { slot } => {
            let Some(id) = model.ids.get(*slot).copied().flatten() else {
                return Ok(());
            };
            let base = model.revisions[*slot];
            if let Some(editor) = editor.as_deref_mut() {
                // arm == capture: the snapshot is exactly the document's
                // revision at ask time (Ask's atomic capture in the model)
                let document = editor.docs.get(id).expect("live slot");
                must!(
                    document.buf.revision().get() == base,
                    "arm captured revision {}, model says {base}",
                    document.buf.revision().get()
                );
            }
            model.tickets.push(Some((*slot, Some(id), base)));
        }
        ServiceOp::Deliver {
            ticket,
            anchors,
            snippets,
            undo_open,
        } => {
            let Some((slot, armed_id, base)) = model.tickets.get(*ticket).copied().flatten() else {
                return Ok(()); // consumed or unarmed: exactly once, ever
            };
            // The armed id decides liveness — a reincarnated slot has a
            // new id; the old one must not resolve to it (the arena's
            // generation IS the model's incarnation guard).
            let target_live = match armed_id {
                Some(id) => model.slot_of(id).is_some(),
                None => model.ids.get(slot).copied().flatten().is_some(), // generator mode
            };
            let target_id = armed_id
                .or(model.ids.get(slot).copied().flatten())
                .expect("armed on a live slot");
            let edits = resolve(anchors, snippets);
            let revision = model.revisions[slot];
            let expected: Result<BatchPrediction, ApplyError> = if !target_live {
                Err(ApplyError::NoDocument)
            } else {
                expect_batch(&model.texts[slot], revision, base, &edits, false)
                    .map_err(ApplyError::Edit)
            };
            if let Some(editor) = editor.as_deref_mut() {
                let focus_before = (editor.current(), editor.active_pane);
                let got = editor.apply(
                    target_id,
                    BufferRevision::new(base),
                    ChangeSet {
                        edits,
                        undo_open: *undo_open,
                    },
                );
                match (&expected, got) {
                    (Ok(prediction), Ok(committed)) => {
                        must!(
                            committed.revision.get() == revision + prediction.delta,
                            "committed revision {}, expected {}",
                            committed.revision.get(),
                            revision + prediction.delta
                        );
                        let document = editor.docs.get(target_id).expect("live target");
                        must!(
                            *document.buf.text() == prediction.text,
                            "applied text diverged from the model"
                        );
                        // the mutation lease consumes the journal: after
                        // Editor::apply returns, nothing is pending
                        must!(
                            document.buf.changes().is_empty(),
                            "Editor::apply left journal entries unconsumed"
                        );
                    }
                    (Err(want), Err(got)) => {
                        must!(
                            got == *want,
                            "apply rejected with {got:?}, expected {want:?}"
                        );
                        if let Some(document) = editor.docs.get(target_id) {
                            must!(
                                *document.buf.text() == model.texts[slot],
                                "rejected apply changed the text"
                            );
                            must!(
                                document.buf.revision().get() == revision,
                                "rejected apply moved the revision"
                            );
                        }
                    }
                    (expected, got) => {
                        return Err(format!(
                            "apply outcome {got:?}, model expected {expected:?}"
                        ));
                    }
                }
                // apply preserves focus: it edits, it never navigates
                must!(
                    editor.current() == focus_before.0 && editor.active_pane == focus_before.1,
                    "Editor::apply moved the focus"
                );
            }
            model.tickets[*ticket] = None; // consumed, exactly once
            if let Ok(prediction) = expected {
                model.texts[slot] = prediction.text;
                model.revisions[slot] += prediction.delta;
            }
        }
    }
    if let Some(editor) = editor {
        sweep(editor, model)?;
    }
    Ok(())
}

/// The full invariant sweep after every service step — the Rust image of
/// the TLA+ state invariants over observable editor state.
fn sweep(editor: &Editor, model: &ServiceState) -> Result<(), String> {
    // NoStalePane + the pane model
    for (index, pane) in editor.panes.iter().enumerate() {
        must!(
            editor.docs.get(pane.doc).is_some(),
            "pane {index} holds a stale doc id"
        );
        let slot = model.panes[index];
        let Some(expected) = model.ids.get(slot).copied().flatten() else {
            return Err(format!("pane {index} references dead slot {slot}"));
        };
        must!(
            pane.doc == expected,
            "pane {index} shows slot {slot}'s old incarnation"
        );
    }
    must!(
        editor.active_pane == model.active,
        "active pane diverged from the model"
    );
    // text/revision equality per live document
    for slot in model.live() {
        let id = model.ids[slot].expect("live slot");
        let Some(document) = editor.docs.get(id) else {
            return Err(format!("slot {slot}: model id does not resolve"));
        };
        must!(
            *document.buf.text() == model.texts[slot],
            "slot {slot}: text diverged from the model"
        );
        must!(
            document.buf.revision().get() == model.revisions[slot],
            "slot {slot}: revision {} != model {}",
            document.buf.revision().get(),
            model.revisions[slot]
        );
    }
    // generational incarnation: a dead id never resolves again, even
    // after the arena reuses its slot index at a new generation
    for dead in &model.dead_ids {
        must!(
            editor.docs.get(*dead).is_none(),
            "dead id {dead:?} resolved after close (slot reincarnation leak)"
        );
    }
    Ok(())
}

pub(super) fn gen_service_stream(seed: u64, steps: usize) -> Vec<ServiceOp> {
    let mut rng = Rng(seed);
    let mut model = ServiceState::new();
    let mut ops = Vec::new();
    while ops.len() < steps {
        let live = model.live();
        let pick = |rng: &mut Rng, live: &[usize]| live[rng.below(live.len())];
        let op = match rng.below(100) {
            n if n < 15 => ServiceOp::Open {
                text: rng.below(TEXTS.len()),
            },
            n if n < 25 => {
                let candidates = &live;
                if candidates.len() < 2 {
                    ServiceOp::Open {
                        text: rng.below(TEXTS.len()),
                    }
                } else {
                    ServiceOp::CloseDoc {
                        slot: candidates[rng.below(candidates.len())],
                    }
                }
            }
            n if n < 35 => ServiceOp::Switch {
                slot: pick(&mut rng, &live),
            },
            n if n < 45 => ServiceOp::Split {
                slot: pick(&mut rng, &live),
            },
            n if n < 55 => ServiceOp::ClosePane,
            n if n < 75 => ServiceOp::Arm {
                slot: pick(&mut rng, &live),
            },
            _ => {
                let armed: Vec<usize> = model
                    .tickets
                    .iter()
                    .enumerate()
                    .filter_map(|(index, ticket)| ticket.is_some().then_some(index))
                    .collect();
                if armed.is_empty() {
                    ServiceOp::Arm {
                        slot: pick(&mut rng, &live),
                    }
                } else {
                    // anchors drawn against the armed slot's model text
                    let slot = model.tickets[armed[rng.below(armed.len())]]
                        .as_ref()
                        .expect("armed")
                        .0;
                    let text = model.texts[slot].clone();
                    let count = rng.below(2) + 1;
                    let overlap = rng.below(100) < 20;
                    let mut anchors: Vec<(usize, usize)> = Vec::new();
                    let mut snippets = Vec::new();
                    for index in 0..count {
                        let (start, end) = if overlap && index > 0 {
                            let start = anchors[0].0;
                            (start, (start + 1).min(text.len() + 1))
                        } else {
                            (rng.below(text.len() + 1), rng.below(text.len() + 1))
                        };
                        anchors.push((start, end));
                        snippets.push(rng.below(SNIPPETS.len()));
                    }
                    ServiceOp::Deliver {
                        ticket: armed[rng.below(armed.len())],
                        anchors,
                        snippets,
                        undo_open: rng.below(4) == 0,
                    }
                }
            }
        };
        step_service(&op, &mut model, None).expect("generator model diverged from itself");
        ops.push(op);
    }
    ops
}

pub(super) fn run_service_stream(ops: &[ServiceOp]) -> Option<String> {
    let mut model = ServiceState::new();
    let mut editor = Editor::new(Buffer::from_text(TEXTS[0]));
    model.ids[0] = Some(editor.current());
    for (index, op) in ops.iter().enumerate() {
        step_service(op, &mut model, Some(&mut editor))
            .err()
            .map(|message| format!("op {index} {op:?}: {message}"))
            .map(Some)
            .unwrap_or(None)?;
    }
    None
}
