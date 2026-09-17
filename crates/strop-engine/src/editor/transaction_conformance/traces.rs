//! EditorProtocol.tla freshness correspondence (0057 VF03): named model
//! traces replayed step by step through the ACTUAL admission handlers —
//! `Editor::apply` (the worker-ticket gateway), `Buffer::
//! prepare_replacements` + `apply_prepared` (PublishLocal), and the
//! generational document arena (`docs.try_insert` / removal) — asserting
//! the invariants the model claims, on the real editor state, after every
//! step. Where the generated-stream oracles in `service.rs` quantify over
//! random schedules, each test here pins ONE documented trace so a
//! regression names the exact broken transition.
//!
//! Trace steps are labelled with the TLA+ action they replay
//! (specs/EditorProtocol.tla):
//!
//!   OpenDoc/CloseDoc     arena insert/remove; reopen = new generation
//!   PublishLocal         the two-phase buffer lease publishing a batch
//!   Arm                  capture (DocumentId, BufferRevision) at ask time
//!   Deliver              Editor::apply — Fresh(r) lands, else typed Err
//!
//! Invariants under correspondence:
//!
//!   NoMisapply        no delivery lands on a dead doc, a reincarnation,
//!                     or a moved revision — the typed Err paths ARE this
//!                     guard, and nothing publishes on the Err path
//!   NoWrongDocument   an applied ticket lands on exactly what it asked,
//!                     in the same incarnation: text/revision/journal of
//!                     the target move by exactly the batch, and no other
//!                     document moves at all
//!   TicketOneShot     a consumed ticket never lands again: replaying the
//!                     same (doc, base) is stale, not a second apply

use super::*;

/// One typed insertion batch at byte 0 — the payload every trace carries.
fn cs(text: &str) -> ChangeSet {
    ChangeSet {
        edits: vec![Replacement::new(raw_range(0, 0), text)],
        undo_open: false,
    }
}

/// The full observable mutation state of one document: text, revision,
/// unconsumed journal length and history depth. NoMisapply's "nothing
/// moves" is checked against all four.
fn observation(e: &Editor, doc: DocumentId) -> (String, BufferRevision, usize, usize) {
    let buf = &e.doc(doc).buf;
    (
        buf.text().to_string(),
        buf.revision(),
        buf.changes().len(),
        buf.history().depth(),
    )
}

/// Trace: OpenDoc, Arm(r), ExternalEdit (PublishLocal by another writer
/// moves the revision), Deliver(r) — the ticket's captured base is now
/// stale, so Deliver is the DROPPED phase: StaleRevision{expected, found}
/// verbatim and zero publication.
#[test]
fn deliver_on_moved_revision_drops_without_publication() {
    let mut e = Editor::new(Buffer::from_text("héllo\n"));
    let doc = e.current();
    // Arm(r): the ticket captures (doc, generation, rev = 0).
    let base = e.buf().revision();
    // ExternalEdit: any writer outside the ticket advances the revision.
    e.apply(doc, base, cs("hi ")).expect("fresh publish lands");
    let moved = observation(&e, doc);
    assert_eq!(moved.0, "hi héllo\n");
    // Deliver(r): Fresh(r) fails on rev mismatch -> DROPPED, no publish.
    let got = e.apply(doc, base, cs("x"));
    assert!(
        got == Err(ApplyError::Edit(EditError::StaleRevision {
            expected: base,
            found: moved.1,
        })),
        "the stale ticket must be refused with its exact base, got {got:?}"
    );
    // NoMisapply: nothing landed on the moved revision.
    assert_eq!(observation(&e, doc), moved, "a dropped ticket published");
}

/// Trace: OpenDoc, Arm(r), CloseDoc, Deliver(r) — the document is dead;
/// Deliver is NoDocument and no other document's state moves.
#[test]
fn deliver_on_dead_document_is_no_document() {
    let mut e = Editor::new(Buffer::from_text("one\n"));
    let survivor = e.current();
    let dead = e
        .docs
        .try_insert(Document::new(Buffer::from_text("two\n")))
        .unwrap();
    // Arm(r) on the doomed document.
    let base = e.doc(dead).buf.revision();
    // CloseDoc.
    e.docs.remove(dead);
    let survivor_before = observation(&e, survivor);
    // Deliver(r): the guard's dead-doc arm.
    let got = e.apply(dead, base, cs("x"));
    assert!(
        got == Err(ApplyError::NoDocument),
        "a dead id must be NoDocument, got {got:?}"
    );
    assert_eq!(
        observation(&e, survivor),
        survivor_before,
        "NoMisapply: a dead delivery touched a survivor"
    );
}

/// Trace: OpenDoc, Arm(r), CloseDoc, OpenDoc (the arena reuses the slot
/// at generation+1), Deliver(r) — the old incarnation's ticket must not
/// resolve to the new tenant (the 0023 cross-document probe).
#[test]
fn reincarnation_never_accepts_the_old_ticket() {
    let mut e = Editor::new(Buffer::from_text("one\n"));
    let dead = e
        .docs
        .try_insert(Document::new(Buffer::from_text("two\n")))
        .unwrap();
    let base = e.doc(dead).buf.revision();
    e.docs.remove(dead);
    let fresh = e
        .docs
        .try_insert(Document::new(Buffer::from_text("again\n")))
        .unwrap();
    if fresh.index() == dead.index() {
        assert_ne!(
            fresh.generation(),
            dead.generation(),
            "slot reuse must bump the generation"
        );
        let before = observation(&e, fresh);
        let got = e.apply(dead, base, cs("x"));
        assert!(
            got == Err(ApplyError::NoDocument),
            "the old incarnation's ticket landed on the new tenant: {got:?}"
        );
        assert_eq!(
            observation(&e, fresh),
            before,
            "NoWrongDocument: the reincarnation was mutated by a stale ticket"
        );
    }
}

/// Trace: OpenDoc d1, OpenDoc d2, Arm(r, d1), PublishLocal(d2),
/// Deliver(r) — the delivery lands on exactly what the ticket asked:
/// d1's text/revision/history move by exactly the batch; d2 and focus
/// are untouched by the delivery.
#[test]
fn fresh_deliver_lands_on_exactly_the_asked_document() {
    let mut e = Editor::new(Buffer::from_text("d1\n"));
    let d1 = e.current();
    let d2 = e
        .docs
        .try_insert(Document::new(Buffer::from_text("d2\n")))
        .unwrap();
    // Arm(r, d1).
    let base = e.doc(d1).buf.revision();
    // PublishLocal(d2): an unrelated publication between Arm and Deliver.
    e.apply(d2, e.doc(d2).buf.revision(), cs("w"))
        .expect("the interleaved publication is fresh");
    let d2_before = observation(&e, d2);
    let focus = (e.current(), e.active_pane);
    // Deliver(r): Fresh(r) holds — the ticket lands.
    let committed = e.apply(d1, base, cs("v")).expect("fresh ticket lands");
    // NoWrongDocument: landed == ticket — d1 moved by exactly the batch.
    assert_eq!(committed.revision, e.doc(d1).buf.revision());
    assert_eq!(e.doc(d1).buf.text().to_string(), "vd1\n");
    assert!(
        e.doc(d1).buf.changes().is_empty(),
        "the mutation lease consumes its journal"
    );
    assert!(
        e.doc(d1).buf.history().depth() > 0,
        "the landed batch is journaled for undo"
    );
    // ...and nothing else moved: the delivery cannot cross documents.
    assert_eq!(observation(&e, d2), d2_before, "Deliver touched d2");
    assert_eq!((e.current(), e.active_pane), focus, "Deliver moved focus");
}

/// Trace: Arm(r), Deliver(r) (APPLIED), Deliver(r) again — the ticket is
/// consumed; the replay is stale, not a second landing. TicketOneShot.
#[test]
fn consumed_ticket_never_lands_twice() {
    let mut e = Editor::new(Buffer::from_text("base\n"));
    let doc = e.current();
    let base = e.buf().revision();
    e.apply(doc, base, cs("1")).expect("first delivery lands");
    let landed = observation(&e, doc);
    let got = e.apply(doc, base, cs("2"));
    assert!(
        got == Err(ApplyError::Edit(EditError::StaleRevision {
            expected: base,
            found: landed.1,
        })),
        "a consumed ticket replays as stale, got {got:?}"
    );
    assert_eq!(
        observation(&e, doc),
        landed,
        "TicketOneShot: a consumed ticket published again"
    );
}

/// Trace: the readonly presentation contract at the mutation gateway —
/// a delivery against a readonly document is refused BEFORE the
/// freshness check (the documented precedence: ReadOnly, then
/// StaleRevision), so a readonly view can never acquire write authority
/// through a stale OR a fresh ticket.
#[test]
fn readonly_delivery_refused_before_freshness() {
    let mut e = Editor::new(Buffer::from_text("locked\n"));
    let doc = e.current();
    e.buf_mut()
        .set_readonly(strop_core::ReadonlyReason::Command);
    let before = observation(&e, doc);
    // Fresh ticket: refused as ReadOnly, not applied.
    let fresh_base = e.buf().revision();
    let got = e.apply(doc, fresh_base, cs("x"));
    assert!(
        got == Err(ApplyError::Edit(EditError::ReadOnly)),
        "readonly refusal precedes a fresh base, got {got:?}"
    );
    // Stale ticket: still ReadOnly — precedence holds on the Err path.
    let got = e.apply(doc, BufferRevision::new(99), cs("y"));
    assert!(
        got == Err(ApplyError::Edit(EditError::ReadOnly)),
        "readonly refusal precedes the freshness check, got {got:?}"
    );
    assert_eq!(
        observation(&e, doc),
        before,
        "a readonly document moved under a delivery"
    );
}
