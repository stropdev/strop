use super::*;

// ---- the batch stream: PublishLocal on a real Buffer -----------------------

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum Base {
    Current,
    Behind,
    Ahead,
}

impl Base {
    pub(super) fn concrete(self, revision: u64) -> u64 {
        match self {
            Base::Current => revision,
            Base::Behind => revision.saturating_sub(1),
            Base::Ahead => revision + 1,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum BatchOp {
    Batch {
        base: Base,
        anchors: Vec<(usize, usize)>,
        snippets: Vec<usize>,
        undo_open: bool,
    },
    Undo,
}

/// One step of the batch protocol against the model — and, when a Buffer
/// is supplied, the same step on the real API with every contract clause
/// asserted. The generator runs this with `buffer: None`.
fn step_batch(
    op: &BatchOp,
    text: &mut String,
    revision: &mut u64,
    groups: &mut UndoGroups,
    buffer: Option<&mut Buffer>,
) -> Result<(), String> {
    let BatchOp::Batch {
        base,
        anchors,
        snippets,
        undo_open,
    } = op
    else {
        // Undo: closes any open group and reverts one unit. The revision
        // afterwards counts the history move's publications — the model
        // resynchronizes from the buffer (monotonic, never back).
        let expected = groups.undo();
        if let Some(buffer) = buffer {
            buffer
                .undo()
                .map_err(|error| format!("undo errored: {error}"))?;
            let undone = buffer.text().to_string();
            let want = expected.clone().unwrap_or_else(|| undone.clone());
            must!(
                undone == want,
                "undo restored {undone:?}, expected {want:?}"
            );
            let now = buffer.revision().get();
            must!(
                now >= *revision,
                "revision went back on undo ({revision} -> {now})"
            );
            *revision = now;
            *text = undone;
        } else {
            // generator approximation: undo publishes and reverts a unit
            if let Some(start) = expected {
                *text = start;
            }
            *revision += 1;
        }
        return Ok(());
    };
    let edits = resolve(anchors, snippets);
    let before = text.clone();
    let before_revision = *revision;
    let expected = expect_batch(
        &before,
        before_revision,
        base.concrete(before_revision),
        &edits,
        false,
    );
    if let Some(buffer) = buffer {
        let journal_before = buffer.changes().len();
        let depth_before = buffer.history().depth();
        // the two-phase gateway: validate everything (prepare), then
        // publish (apply_prepared). Nothing in between can half-land.
        let prepared = buffer.prepare_replacements(
            BufferRevision::new(base.concrete(before_revision)),
            edits.clone(),
        );
        match (&expected, prepared) {
            (Ok(prediction), Ok(prepared)) => {
                let got = buffer.apply_prepared(prepared, *undo_open);
                must!(
                    got == Ok(BufferRevision::new(before_revision + prediction.delta)),
                    "applied to revision {got:?}, expected {}",
                    before_revision + prediction.delta
                );
                must!(
                    *buffer.text() == prediction.text,
                    "text {:?} after batch, expected {:?}",
                    buffer.text().to_string(),
                    prediction.text
                );
                // RevisionTracksPublications: one journal entry per
                // published revision, pre-edit geometry, consecutive.
                let appended = &buffer.changes()[journal_before..];
                must!(
                    appended.len() as u64 == prediction.delta,
                    "journal appended {} entries for {} revisions",
                    appended.len(),
                    prediction.delta
                );
                for (entry, want) in appended.iter().zip(&prediction.entries) {
                    must!(
                        entry.origin == ChangeOrigin::User,
                        "journal origin is not User"
                    );
                    must!(
                        entry.revision.get() == want.revision,
                        "journal revision {} at {:?}, expected {}",
                        entry.revision.get(),
                        (want.start_byte, want.old_end_byte),
                        want.revision
                    );
                    let edit = entry.edit;
                    must!(
                        (edit.start_byte, edit.old_end_byte, edit.new_end_byte)
                            == (want.start_byte, want.old_end_byte, want.new_end_byte),
                        "journal bytes {:?} for revision {}, expected {:?} (pre-edit geometry)",
                        (edit.start_byte, edit.old_end_byte, edit.new_end_byte),
                        entry.revision.get(),
                        (want.start_byte, want.old_end_byte, want.new_end_byte)
                    );
                    must!(
                        (edit.start_point, edit.old_end_point, edit.new_end_point)
                            == (want.start_point, want.old_end_point, want.new_end_point),
                        "journal points {:?} for revision {}, expected {:?}",
                        (edit.start_point, edit.old_end_point, edit.new_end_point),
                        entry.revision.get(),
                        (want.start_point, want.old_end_point, want.new_end_point)
                    );
                }
                buffer.clear_changes(); // the lease consumes the journal
            }
            (Err(want), Err(got)) => {
                must!(got == *want, "rejected with {got:?}, expected {want:?}");
                // a rejected batch changes NOTHING: text, revision,
                // journal, history depth all identical.
                must!(*buffer.text() == before, "rejected batch changed the text");
                must!(
                    buffer.revision().get() == before_revision,
                    "rejected batch moved the revision"
                );
                must!(
                    buffer.changes().len() == journal_before,
                    "rejected batch appended journal entries"
                );
                must!(
                    buffer.history().depth() == depth_before,
                    "rejected batch changed the history depth"
                );
            }
            (expected, prepared) => {
                return Err(match (expected.is_ok(), prepared.is_ok()) {
                    (true, false) => "prepare rejected a batch the model accepts".into(),
                    _ => "prepare accepted a batch the model rejects".into(),
                });
            }
        }
    }
    // Rejected batches leave the model unchanged.
    if let Ok(prediction) = expected {
        groups.batch_landed(&before, prediction.delta, *undo_open);
        *text = prediction.text;
        *revision += prediction.delta;
    }
    Ok(())
}

pub(super) fn gen_batch_stream(seed: u64, steps: usize) -> Vec<BatchOp> {
    let mut rng = Rng(seed);
    let mut text = TEXTS[(seed % TEXTS.len() as u64) as usize].to_string();
    let mut revision = 0u64;
    let mut groups = UndoGroups::default();
    let mut ops = Vec::new();
    while ops.len() < steps {
        let count = rng.below(3) + 1;
        let overlap = rng.below(100) < 20;
        let mut anchors: Vec<(usize, usize)> = Vec::new();
        let mut snippets = Vec::new();
        for index in 0..count {
            let (start, end) = if overlap && index > 0 {
                let start = anchors[0].0;
                (start, (start + 1 + rng.below(2)).min(text.len() + 1))
            } else {
                (rng.below(text.len() + 1), rng.below(text.len() + 1))
            };
            anchors.push((start, end));
            snippets.push(rng.below(SNIPPETS.len()));
        }
        if rng.below(100) < 10 {
            // an off-bounds / mid-char anchor for the InvalidRange path
            let index = rng.below(anchors.len());
            let start = text.len() + 1 + rng.below(2);
            anchors[index] = (start, start + rng.below(2));
        }
        let base = if rng.below(100) < 85 {
            Base::Current
        } else if revision > 0 {
            Base::Behind
        } else {
            Base::Ahead
        };
        let undo_open = rng.below(4) == 0;
        let op = BatchOp::Batch {
            base,
            anchors,
            snippets,
            undo_open,
        };
        step_batch(&op, &mut text, &mut revision, &mut groups, None)
            .expect("generator model diverged from itself");
        ops.push(op);
        if rng.below(100) < 12 {
            step_batch(&BatchOp::Undo, &mut text, &mut revision, &mut groups, None)
                .expect("generator model diverged from itself");
            ops.push(BatchOp::Undo);
        }
    }
    ops
}

pub(super) fn run_batch_stream(ops: &[BatchOp]) -> Option<String> {
    let start = TEXTS[0];
    let mut text = start.to_string();
    let mut revision = 0u64;
    let mut groups = UndoGroups::default();
    let mut buffer = Buffer::from_text(start);
    for (index, op) in ops.iter().enumerate() {
        step_batch(op, &mut text, &mut revision, &mut groups, Some(&mut buffer))
            .err()
            .map(|message| format!("op {index} {op:?}: {message}"))
            .map(Some)
            .unwrap_or(None)?;
    }
    None
}
