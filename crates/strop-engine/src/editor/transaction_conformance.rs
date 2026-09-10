//! Transaction conformance (R12): the Rust half of the
//! specs/EditorProtocol.tla correspondence, checked on the real
//! Editor/Buffer APIs — `Editor::apply`, `Buffer::prepare_replacements`
//! with `apply_prepared`, `close_buffer`, the generational document arena —
//! never on internals.
//!
//! TLA+                      Rust oracle (this module)
//! -----------------------   ------------------------------------------------
//! OpenDoc/CloseDoc/gen      `docs.insert` / `close_buffer`; a closed id
//!                           fails lookup even after the arena reuses its
//!                           slot at generation+1 (Editor::apply returns
//!                           NoDocument for it — the 0023 cross-document
//!                           hunk probe)
//! PublishLocal              `prepare_replacements` + `apply_prepared`
//! Arm + Deliver             capture (DocumentId, BufferRevision), then
//!                           `Editor::apply` — fresh lands, stale/dead is
//!                           a typed Err that publishes nothing
//! NoStalePane               every pane's doc resolves after every step
//! NoWrongDocument           a ticket only ever lands on the document it
//!                           asked for, at the revision it asked for
//! RevisionTracksPublications revision delta == journal entries appended,
//!                           each entry carrying PRE-EDIT geometry
//! TicketOneShot             one ticket, one apply; every base is checked
//!                           (StaleRevision{expected, found} verbatim)
//! NoMisapply                the typed Err paths ARE the guard — nothing
//!                           publishes on a dead doc or a moved revision
//!
//! Undo grouping is treated as bookkeeping, not durability: the oracle
//! checks that open batches merge into one undo unit and that undo
//! restores the unit's start text. No crash-recovery behavior is claimed
//! or tested — the Rust contract doesn't promise it.
//!
//! Determinism and witnesses: all randomness is resolved at generation
//! time into a `Vec<Op>` driven by a seeded xorshift. A failing stream is
//! shrunk (ops greedily deleted while the failure survives) and the panic
//! prints a `strop-model-recipe-v1` JSON witness — paste it into
//! [`replay`] to reproduce exactly.

use super::transact::{ApplyError, ChangeSet};
use super::*;
use strop_core::id::{BufferRevision, ByteOffset, DocumentId};
use strop_core::{Buffer, ChangeOrigin, EditError, MotionShape, Range, Replacement};

macro_rules! must {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(format!($($arg)*));
        }
    };
}

mod batch;
mod service;
use batch::{gen_batch_stream, run_batch_stream, BatchOp};
use service::{gen_service_stream, run_service_stream, ServiceOp};

// ---- deterministic randomness ---------------------------------------------

/// Seeded xorshift64 — same generator as conformance.rs; a stream is
/// reproducible from its seed alone.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

// ---- the model expectation engine ------------------------------------------
// A plain-String mirror of the documented two-phase batch contract,
// including error precedence: ReadOnly, then StaleRevision, then whole-
// batch range validation (sorted order), then overlap, then publication.

/// Opening texts: multibyte content guarantees mid-char anchors exist.
const TEXTS: [&str; 3] = ["héllo\nwörld\n", "alpha\nbeta\ngamma\n", "αβγ\n"];

/// Insertion payloads: empty, ascii, multibyte, multiline.
const SNIPPETS: [&str; 5] = ["", "x", "é", "line\nnext", "γ"];

/// A literal charwise range — reversed/off-bounds anchors are part of
/// the contract (InvalidRange), and Range::charwise debug-asserts order.
fn raw_range(start: usize, end: usize) -> Range {
    Range {
        start: ByteOffset::new(start),
        end: ByteOffset::new(end),
        shape: MotionShape::Characterwise { inclusive: false },
    }
}

fn resolve(anchors: &[(usize, usize)], snippets: &[usize]) -> Vec<Replacement> {
    anchors
        .iter()
        .zip(snippets)
        .map(|(&(start, end), &snippet)| Replacement::new(raw_range(start, end), SNIPPETS[snippet]))
        .collect()
}

fn boundary(text: &str, at: usize) -> bool {
    at == 0 || at == text.len() || text.is_char_boundary(at)
}

fn point_of(text: &str, at: usize) -> (usize, usize) {
    let line = text[..at].bytes().filter(|b| *b == b'\n').count();
    let column = at - (text[..at].rfind('\n').map_or(0, |i| i + 1));
    (line, column)
}

fn point_extent(text: &str) -> (usize, usize) {
    let lines = text.bytes().filter(|b| *b == b'\n').count();
    let column = if lines == 0 {
        text.len()
    } else {
        text.rsplit('\n').next().map_or(0, str::len)
    };
    (lines, column)
}

/// One journal entry the contract promises, with pre-edit geometry.
#[derive(Debug, PartialEq, Eq)]
struct ExpectedEntry {
    revision: u64,
    start_byte: usize,
    old_end_byte: usize,
    new_end_byte: usize,
    start_point: (usize, usize),
    old_end_point: (usize, usize),
    new_end_point: (usize, usize),
}

/// The model's prediction for one batch.
#[derive(Debug)]
struct BatchPrediction {
    text: String,
    delta: u64,
    entries: Vec<ExpectedEntry>,
}

fn expect_batch(
    text: &str,
    revision: u64,
    base: u64,
    edits: &[Replacement],
    readonly: bool,
) -> Result<BatchPrediction, EditError> {
    if readonly {
        return Err(EditError::ReadOnly);
    }
    if base != revision {
        return Err(EditError::StaleRevision {
            expected: BufferRevision::new(base),
            found: BufferRevision::new(revision),
        });
    }
    let mut retained: Vec<&Replacement> = edits
        .iter()
        .filter(|edit| !edit.range.is_empty() || !edit.text.is_empty())
        .collect();
    retained.sort_unstable_by_key(|edit| edit.range.start);
    for edit in &retained {
        let (start, end) = (edit.range.start.get(), edit.range.end.get());
        if start > end || !boundary(text, start) || !boundary(text, end) {
            return Err(EditError::InvalidRange);
        }
    }
    for pair in retained.windows(2) {
        if pair[0].range.end > pair[1].range.start || pair[0].range.start == pair[1].range.start {
            return Err(EditError::Overlap);
        }
    }
    if retained.is_empty() {
        return Ok(BatchPrediction {
            text: text.to_string(),
            delta: 0,
            entries: Vec::new(),
        });
    }
    let mut after = text.to_string();
    let mut entries = Vec::new();
    // applied descending: every entry's geometry is pre-edit coordinates
    for (index, edit) in retained.iter().enumerate().rev() {
        let (start, end) = (edit.range.start.get(), edit.range.end.get());
        let start_point = point_of(text, start);
        let old_end_point = point_of(text, end);
        let extent = point_extent(&edit.text);
        let new_end_point = if extent.0 == 0 {
            (start_point.0, start_point.1 + extent.1)
        } else {
            (start_point.0 + extent.0, extent.1)
        };
        after.replace_range(start..end, &edit.text);
        entries.push(ExpectedEntry {
            revision: revision + (retained.len() - index) as u64,
            start_byte: start,
            old_end_byte: end,
            new_end_byte: start + edit.text.len(),
            start_point,
            old_end_point,
            new_end_point,
        });
    }
    Ok(BatchPrediction {
        text: after,
        delta: retained.len() as u64,
        entries,
    })
}

// ---- undo grouping (bookkeeping, not durability) ---------------------------

/// One undo unit = the batches between closes. `undo()` first closes any
/// open group, so an open chain reverts as one unit.
#[derive(Default)]
struct UndoGroups {
    units: Vec<String>,
    open_start: Option<String>,
}

impl UndoGroups {
    fn batch_landed(&mut self, before: &str, delta: u64, undo_open: bool) {
        if delta == 0 {
            return; // an empty batch never opens a group
        }
        if self.open_start.is_none() {
            self.open_start = Some(before.to_string());
        }
        if !undo_open {
            self.units.push(self.open_start.take().expect("set above"));
        }
    }
    fn undo(&mut self) -> Option<String> {
        self.open_start.take().or_else(|| self.units.pop())
    }
}

// ---- shrinking + witnesses --------------------------------------------------

/// Greedy delta-debugging: delete ops while the failure survives. The
/// run function is a pure function of the op list, so this is sound.
fn shrink<T, F>(ops: &[T], run: &F) -> Vec<T>
where
    T: Clone,
    F: Fn(&[T]) -> Option<String>,
{
    let mut ops = ops.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        let mut index = 0;
        while index < ops.len() {
            let mut candidate = ops.clone();
            candidate.remove(index);
            if run(&candidate).is_some() {
                ops = candidate;
                changed = true;
            } else {
                index += 1;
            }
        }
    }
    ops
}

fn witness(harness: &str, seed: u64, ops: &impl serde::Serialize) -> String {
    serde_json::to_string(&serde_json::json!({
        "schema": "strop-model-recipe-v1",
        "harness": harness,
        "seed": seed,
        "ops": ops,
    }))
    .expect("ops serialize")
}

/// Re-run a recorded witness — the JSON a failure's REPLAY line prints.
/// Paste it into a throwaway test to reproduce a divergence exactly:
/// `replay(r#"{"schema":"strop-model-recipe-v1",...}"#);`
pub fn replay(recipe: &str) {
    let value: serde_json::Value = serde_json::from_str(recipe).expect("witness is JSON");
    assert_eq!(
        value["schema"], "strop-model-recipe-v1",
        "unknown witness schema: {}",
        value["schema"]
    );
    let seed = value["seed"].as_u64().expect("witness seed");
    match value["harness"].as_str().expect("witness harness") {
        "batch" => {
            let ops: Vec<BatchOp> =
                serde_json::from_value(value["ops"].clone()).expect("witness batch ops");
            if let Some(message) = run_batch_stream(&ops) {
                panic!("replay seed {seed}: {message}");
            }
        }
        "service" => {
            let ops: Vec<ServiceOp> =
                serde_json::from_value(value["ops"].clone()).expect("witness service ops");
            if let Some(message) = run_service_stream(&ops) {
                panic!("replay seed {seed}: {message}");
            }
        }
        other => panic!("unknown witness harness: {other}"),
    }
}

// ---- the tests ---------------------------------------------------------------

/// The full two-phase gateway in one call: prepare, then publish.
fn land(
    buffer: &mut Buffer,
    base: BufferRevision,
    edits: Vec<Replacement>,
    undo_open: bool,
) -> Result<BufferRevision, EditError> {
    let prepared = buffer.prepare_replacements(base, edits)?;
    buffer.apply_prepared(prepared, undo_open)
}

#[test]
fn batch_oracle_generated_streams_match_the_contract() {
    for seed in [1u64, 7, 42, 1337, 99991] {
        let ops = gen_batch_stream(seed, 120);
        if let Some(message) = run_batch_stream(&ops) {
            let minimal = shrink(&ops, &run_batch_stream);
            panic!(
                "batch oracle diverged (seed {seed}): {message}\n\
                 minimal ops: {minimal:?}\n\
                 REPLAY: {}",
                witness("batch", seed, &minimal)
            );
        }
    }
}

#[test]
fn service_oracle_generated_streams_match_the_contract() {
    for seed in [1u64, 7, 42, 1337, 99991] {
        let ops = gen_service_stream(seed, 120);
        if let Some(message) = run_service_stream(&ops) {
            let minimal = shrink(&ops, &run_service_stream);
            panic!(
                "service oracle diverged (seed {seed}): {message}\n\
                 minimal ops: {minimal:?}\n\
                 REPLAY: {}",
                witness("service", seed, &minimal)
            );
        }
    }
}

#[test]
fn replay_witnesses_roundtrip() {
    // a printed witness must deserialize and re-execute clean — the
    // shrink/replay escape hatch is part of the contract
    let ops = gen_batch_stream(42, 24);
    replay(&witness("batch", 42, &ops));
    let ops = gen_service_stream(42, 24);
    replay(&witness("service", 42, &ops));
}

#[test]
fn rejected_batches_change_nothing() {
    let cases: Vec<(&str, u64, Vec<Replacement>, Option<EditError>)> = vec![
        (
            "stale base (ahead)",
            1,
            vec![Replacement::new(raw_range(0, 0), "x")],
            Some(EditError::StaleRevision {
                expected: BufferRevision::new(1),
                found: BufferRevision::new(0),
            }),
        ),
        (
            "overlap: duplicate start",
            0,
            vec![
                Replacement::new(raw_range(0, 0), "a"),
                Replacement::new(raw_range(0, 3), "b"),
            ],
            Some(EditError::Overlap),
        ),
        (
            "overlap: straddle",
            0,
            vec![
                Replacement::new(raw_range(0, 4), "x"),
                Replacement::new(raw_range(3, 6), "y"),
            ],
            Some(EditError::Overlap),
        ),
        (
            "anchor mid-char",
            0,
            vec![Replacement::new(raw_range(2, 3), "x")],
            Some(EditError::InvalidRange),
        ),
        (
            "anchor reversed",
            0,
            vec![Replacement::new(raw_range(5, 2), "x")],
            Some(EditError::InvalidRange),
        ),
        (
            "anchor past the end",
            0,
            vec![Replacement::new(raw_range(99, 99), "x")],
            Some(EditError::InvalidRange),
        ),
        ("empty batch publishes nothing", 0, vec![], None),
        (
            "all-trivial batch publishes nothing",
            0,
            vec![Replacement::new(raw_range(3, 3), "")],
            None,
        ),
    ];
    for (name, base, edits, want) in cases {
        let mut buffer = Buffer::from_text("héllo\nwörld\n");
        let revision = buffer.revision().get();
        let (text, changes, depth) = (
            buffer.text().to_string(),
            buffer.changes().len(),
            buffer.history().depth(),
        );
        let got = land(&mut buffer, BufferRevision::new(base), edits, false);
        match want {
            Some(expected) => {
                assert!(
                    got.as_ref().err() == Some(&expected),
                    "{name}: got {got:?}, want {expected:?}"
                )
            }
            None => assert!(
                got == Ok(BufferRevision::new(revision)),
                "{name}: empty batch must be a revision-preserving Ok"
            ),
        }
        assert_eq!(
            buffer.text().to_string(),
            text,
            "{name}: rejected batch changed the text"
        );
        assert_eq!(
            buffer.revision().get(),
            revision,
            "{name}: rejected batch moved the revision"
        );
        assert_eq!(
            buffer.changes().len(),
            changes,
            "{name}: rejected batch appended journal entries"
        );
        assert_eq!(
            buffer.history().depth(),
            depth,
            "{name}: rejected batch changed the history depth"
        );
    }
    // a stale base BEHIND the frontier after a real publication
    let mut buffer = Buffer::from_text("hello\n");
    let old = buffer.revision().get();
    land(
        &mut buffer,
        BufferRevision::new(old),
        vec![Replacement::new(raw_range(0, 0), "hi ")],
        false,
    )
    .expect("first batch lands");
    let snapshot = (buffer.text().to_string(), buffer.revision());
    let got = land(
        &mut buffer,
        BufferRevision::new(old),
        vec![Replacement::new(raw_range(0, 0), "x")],
        false,
    );
    assert!(
        got == Err(EditError::StaleRevision {
            expected: BufferRevision::new(old),
            found: snapshot.1
        }),
        "stale-behind must carry the exact expected/found payload, got {got:?}"
    );
    assert_eq!((buffer.text().to_string(), buffer.revision()), snapshot);
    // readonly: the first refusal, before any base check
    buffer.readonly = true;
    let revision = buffer.revision();
    let got = land(
        &mut buffer,
        revision,
        vec![Replacement::new(raw_range(0, 0), "x")],
        false,
    );
    assert!(
        got == Err(EditError::ReadOnly),
        "readonly must refuse first, got {got:?}"
    );
}

#[test]
fn editor_apply_refuses_dead_and_stale_without_side_effects() {
    let mut e = Editor::new(Buffer::from_text("hello\n"));
    let doc = e.current();
    let cs = |text: &str| ChangeSet {
        edits: vec![Replacement::new(raw_range(0, 0), text)],
        undo_open: false,
    };
    // a fresh ticket lands
    let committed = e
        .apply(doc, e.buf().revision(), cs("hi "))
        .expect("fresh base lands");
    assert_eq!(committed.revision, e.buf().revision());
    assert_eq!(e.buf().text().to_string(), "hi hello\n");
    // the mutation lease consumed the journal
    assert!(e.doc(doc).buf.changes().is_empty());
    // stale: exact typed payload, nothing changed
    let snapshot = (e.buf().text().to_string(), e.buf().revision());
    let got = e.apply(doc, BufferRevision::new(0), cs("x"));
    match got {
        Err(ApplyError::Edit(EditError::StaleRevision { expected, found })) => {
            assert_eq!(
                expected,
                BufferRevision::new(0),
                "expected = the claimed base"
            );
            assert_eq!(found, snapshot.1, "found = the document's real revision");
        }
        other => panic!("stale apply must be StaleRevision{{expected, found}}, got {other:?}"),
    }
    assert_eq!((e.buf().text().to_string(), e.buf().revision()), snapshot);
    // a second document; applying to it preserves focus everywhere
    let other = e.docs.insert(Document::new(Buffer::from_text("other\n")));
    let focus = (e.current(), e.active_pane);
    e.apply(other, e.doc(other).buf.revision(), cs("Z"))
        .expect("applies to a background doc");
    assert_eq!((e.current(), e.active_pane), focus, "apply moved the focus");
    assert_eq!(
        e.buf().text().to_string(),
        snapshot.0,
        "apply touched the focused document"
    );
    assert_eq!(e.doc(other).buf.text().to_string(), "Zother\n");
    // dead id: NoDocument — and still NoDocument when the arena reuses
    // the slot index at a new generation (the 0023 cross-document probe)
    let dead = other;
    e.docs.remove(dead);
    let got = e.apply(dead, BufferRevision::new(0), cs("x"));
    assert!(
        matches!(got, Err(ApplyError::NoDocument)),
        "dead id must be NoDocument, got {got:?}"
    );
    let fresh = e.docs.insert(Document::new(Buffer::from_text("again\n")));
    if fresh.index() == dead.index() {
        assert_ne!(
            fresh.generation(),
            dead.generation(),
            "slot reuse must bump the generation"
        );
        // the old id still refuses — it must NOT resolve to the new tenant
        let got = e.apply(dead, e.doc(fresh).buf.revision(), cs("x"));
        assert!(
            matches!(got, Err(ApplyError::NoDocument)),
            "a reincarnated slot must not accept the old incarnation's id, got {got:?}"
        );
        assert_eq!(e.doc(fresh).buf.text().to_string(), "again\n");
    }
}

#[test]
fn undo_groups_batches_but_promises_no_durability() {
    let mut buffer = Buffer::from_text("ab\n");
    let base = buffer.revision();
    land(
        &mut buffer,
        base,
        vec![Replacement::new(raw_range(0, 0), "X")],
        true,
    )
    .expect("open batch lands");
    let base = buffer.revision();
    land(
        &mut buffer,
        base,
        vec![Replacement::new(raw_range(3, 3), "Y")],
        false,
    )
    .expect("closing batch lands");
    assert_eq!(buffer.text().to_string(), "XabY\n");
    buffer.undo().expect("undo is not an error");
    assert_eq!(
        buffer.text().to_string(),
        "ab\n",
        "one undo reverts the whole open chain"
    );
    // a lone closed batch is its own unit
    let base = buffer.revision();
    land(
        &mut buffer,
        base,
        vec![Replacement::new(raw_range(0, 0), "Z")],
        false,
    )
    .expect("closed batch lands");
    buffer.undo().expect("undo is not an error");
    assert_eq!(buffer.text().to_string(), "ab\n");
    // undoing at the root changes nothing
    buffer.undo().expect("undo at the root is Ok");
    assert_eq!(buffer.text().to_string(), "ab\n");
}

#[test]
fn prepared_batches_are_bound_to_their_buffer_and_revision() {
    // bound to the buffer: identical text is still a different buffer
    // (the binding is the trace id, not the content)
    let mut a = Buffer::from_text("shared\n");
    let mut b = Buffer::from_text("shared\n");
    let prepared = a
        .prepare_replacements(a.revision(), vec![Replacement::new(raw_range(0, 0), "x")])
        .expect("prepares");
    let got = b.apply_prepared(prepared, false);
    assert!(
        got == Err(EditError::WrongBuffer),
        "cross-buffer apply must be WrongBuffer, got {got:?}"
    );
    assert_eq!(b.text().to_string(), "shared\n");
    assert_eq!(b.revision().get(), 0);

    // bound to the revision: publish something else between prepare and
    // apply — the prepared batch must be refused, nothing half-landed
    let prepared = a
        .prepare_replacements(a.revision(), vec![Replacement::new(raw_range(0, 0), "y")])
        .expect("prepares");
    let base = a.revision();
    land(
        &mut a,
        base,
        vec![Replacement::new(raw_range(6, 6), "!")],
        false,
    )
    .expect("intervening batch lands");
    let now = a.revision();
    let got = a.apply_prepared(prepared, false);
    assert!(
        got == Err(EditError::StaleRevision {
            expected: base,
            found: now
        }),
        "a prepared batch outliving its revision must be stale, got {got:?}"
    );
    assert_eq!(a.text().to_string(), "shared!\n");

    // an empty prepared batch preserves everything — the path the editor
    // uses to keep prompt state untouched on empty ChangeSets
    let before = (
        a.text().to_string(),
        a.revision(),
        a.changes().len(),
        a.history().depth(),
    );
    let prepared = a
        .prepare_replacements(a.revision(), vec![Replacement::new(raw_range(0, 0), "")])
        .expect("prepares");
    assert!(prepared.is_empty());
    let got = a.apply_prepared(prepared, false);
    assert_eq!(got, Ok(before.1));
    assert_eq!(
        (
            a.text().to_string(),
            a.revision(),
            a.changes().len(),
            a.history().depth()
        ),
        before
    );

    // readonly between prepare and apply: refused with nothing changed
    let prepared = a
        .prepare_replacements(a.revision(), vec![Replacement::new(raw_range(0, 0), "z")])
        .expect("prepares");
    a.readonly = true;
    let got = a.apply_prepared(prepared, false);
    assert!(
        got == Err(EditError::ReadOnly),
        "readonly must refuse the apply, got {got:?}"
    );
    a.readonly = false;
    assert_eq!(a.text().to_string(), before.0);
    assert_eq!(a.revision(), before.1);
}
