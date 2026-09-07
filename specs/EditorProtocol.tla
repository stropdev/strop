---- MODULE EditorProtocol ----
(***************************************************************************)
(* The editor's document/service protocol (0024, re-grounded R12) —      *)
(* modeled at the publication boundary the Rust code actually has.      *)
(*                                                                       *)
(*   1. documents live in a generational arena: an id is (slot,          *)
(*      generation). CloseDoc ends an incarnation; the next OpenDoc      *)
(*      starts generation+1; a ticket armed for an old incarnation      *)
(*      never lands on the new one (strop-core's Arena IS this gen —    *)
(*      Editor::apply returns NoDocument for a dead or reincarnated     *)
(*      id: the 0023 cross-document hunk probe)                          *)
(*   2. publication is ONE atomic step: validate the whole batch, then  *)
(*      publish it and consume the next revision. A rejected batch      *)
(*      publishes nothing at all. Undo grouping is bookkeeping, NOT     *)
(*      durability: no crash-recovery action exists here because the    *)
(*      Rust contract (Buffer::apply_replacements / Editor::apply)      *)
(*      promises validation/publication atomicity only — the model      *)
(*      must not overclaim (R12)                                         *)
(*   3. closing a document rebinds every referencing pane (the :vs      *)
(*      scratch-strand crash)                                            *)
(*   4. worker tickets are single-shot: Arm captures (doc, generation,  *)
(*      revision-at-ask); Deliver consumes the ticket exactly once —    *)
(*      applied if still fresh, dropped (a typed Err, nothing           *)
(*      published) otherwise                                             *)
(*                                                                       *)
(* THE invariants — predicates over observable state, no tautologies.   *)
(* The Rust-to-model correspondence: each maps to an oracle in          *)
(* crates/strop/src/editor/transaction_conformance.rs that re-checks    *)
(* the same property on the real Editor/Buffer APIs:                     *)
(*                                                                       *)
(*   NoStalePane               every pane references a live document    *)
(*                              (Rust: panes resolve after every step)  *)
(*   NoWrongDocument           every applied ticket landed on exactly   *)
(*                              what it asked, in the same incarnation  *)
(*                              (Rust: apply only ever mutates its own  *)
(*                              DocumentId; dead ids are NoDocument)    *)
(*   RevisionTracksPublications the visible revision equals the number  *)
(*                              of published revisions of the current   *)
(*                              incarnation, gapless from zero (Rust:   *)
(*                              revision delta == journal entries       *)
(*                              appended, each with pre-edit geometry)  *)
(*   TicketOneShot             a ticket is idle, in flight, or          *)
(*                              consumed; a landing exists iff the      *)
(*                              ticket was consumed as applied —        *)
(*                              deliver-without-consume and re-arm      *)
(*                              mutants break it                        *)
(*   NoMisapply                no delivery ever landed on a dead doc,   *)
(*                              a reincarnation, or a moved revision    *)
(*                              (Rust: the typed Err paths ARE this     *)
(*                              guard; the kept mutant in              *)
(*                              specs/EditorProtocol_Mutant.tla must    *)
(*                              die by exactly this invariant)          *)
(*                                                                       *)
(* Not modeled: text content (the Rust oracles own it — positions,      *)
(* anchors, overlap), liveness, undo grouping semantics (per-buffer     *)
(* bookkeeping, not protocol), crash recovery (explicitly NOT           *)
(* promised by the Rust contract).                                      *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS DOCS,        \* document ids in play, e.g. {1, 2}
          PANES,       \* pane ids, e.g. {1, 2}
          REQS,        \* request (ticket) ids, e.g. {1, 2}
          MAXREV,      \* revision cap per incarnation (finite state space)
          MAXGEN       \* incarnation cap per document (finite state space)

VARIABLES live,        \* set of live document ids
          gen,         \* doc id -> incarnation counter (arena generation;
                       \* bumped by OpenDoc, never reset: reopen = new doc)
          rev,         \* doc id -> visible revision of the current
                       \* incarnation (strop-core's epoch)
          claims,      \* doc id -> revisions published by the current
                       \* incarnation (the journal: one entry per
                       \* published revision, reset on incarnation change)
          paneOf,      \* pane id -> document id | NONE
          phase,       \* request id -> IDLE | ARMED | APPLIED | DROPPED
                       \* (a worker ticket's exact state machine)
          ticket,      \* request id -> [doc, g, r] it asked for (the
                       \* stub survives consumption: landings are checked
                       \* against what was asked)
          landed,      \* request id -> [doc, g, r] it landed on (NONE
                       \* until applied, write-once)
          misapplied   \* TRUE iff a delivery landed somewhere stale —
                       \* unreachable here (the guard drops instead); the
                       \* mutant without the guard sets it and must die
                       \* by exactly NoMisapply

NONE == 0
IDLE == 0
ARMED == 1
APPLIED == 2
DROPPED == 3

TypeOK ==
    /\ live \subseteq DOCS
    /\ gen \in [DOCS -> 0..MAXGEN]
    /\ rev \in [DOCS -> 0..MAXREV]
    /\ claims \in [DOCS -> SUBSET (0..MAXREV)]
    /\ paneOf \in [PANES -> DOCS \union {NONE}]
    /\ phase \in [REQS -> {IDLE, ARMED, APPLIED, DROPPED}]
    /\ ticket \in [REQS -> [doc: DOCS \union {NONE}, g: 0..MAXGEN, r: 0..MAXREV]]
    /\ landed \in [REQS -> [doc: DOCS \union {NONE}, g: 0..MAXGEN, r: 0..MAXREV]]
    /\ misapplied \in {FALSE, TRUE}
Init ==
    /\ live = {}
    /\ gen = [d \in DOCS |-> 0]
    /\ rev = [d \in DOCS |-> 0]
    /\ claims = [d \in DOCS |-> {}]
    /\ paneOf = [p \in PANES |-> NONE]
    /\ phase = [r \in REQS |-> IDLE]
    /\ ticket = [r \in REQS |-> [doc |-> NONE, g |-> 0, r |-> 0]]
    /\ landed = [r \in REQS |-> [doc |-> NONE, g |-> 0, r |-> 0]]
    /\ misapplied = FALSE

\* -- documents ------------------------------------------------------------
\* OpenDoc is the arena insert: a closed slot reopens at generation+1.
\* Everything armed for an older generation is stale by construction.

OpenDoc(d) ==
    /\ d \notin live
    /\ gen[d] < MAXGEN
    /\ live' = live \union {d}
    /\ gen' = [gen EXCEPT ![d] = @ + 1]
    /\ rev' = [rev EXCEPT ![d] = 0]
    /\ claims' = [claims EXCEPT ![d] = {}]
    /\ UNCHANGED <<paneOf, phase, ticket, landed, misapplied>>

CloseDoc(d) ==
    /\ d \in live
    /\ live' = live \ {d}
    \* every pane on the dying document rebinds to a survivor (or NONE
    \* when none) — never keeps the stale id (the :vs scratch-strand fix)
    /\ paneOf' = [p \in PANES |->
        IF paneOf[p] = d
        THEN IF live \ {d} = {} THEN NONE ELSE CHOOSE x \in live \ {d} : TRUE
        ELSE paneOf[p]]
    /\ UNCHANGED <<gen, rev, claims, phase, ticket, landed, misapplied>>

\* -- panes -----------------------------------------------------------------

SplitPane(p, d) ==
    /\ d \in live
    /\ paneOf' = [paneOf EXCEPT ![p] = d]
    /\ UNCHANGED <<live, gen, rev, claims, phase, ticket, landed, misapplied>>

\* -- local publication -----------------------------------------------------
\* A user/extension batch published on the editor's own lease (typing, a
\* macro, a worker with a fresh lease). Publication = validate, then
\* consume the next revision of THIS incarnation and journal it — one
\* atomic step. There is deliberately no crash action and no open
\* transaction state: validation/publication atomicity is the whole
\* claim, undo grouping is not durability, and recovery is not promised.

PublishLocal(d) ==
    /\ d \in live
    /\ rev[d] < MAXREV
    /\ rev' = [rev EXCEPT ![d] = @ + 1]
    /\ claims' = [claims EXCEPT ![d] = @ \union {rev[d]}]
    /\ UNCHANGED <<live, gen, paneOf, phase, ticket, landed, misapplied>>

\* -- worker tickets --------------------------------------------------------
\* Arm == Ask: capture (doc, generation, revision) at ask time. The
\* capture is atomic with the ask — there is no armed-without-a-snapshot
\* state, exactly like capturing DocumentId + BufferRevision in Rust.

Arm(r, d) ==
    /\ d \in live
    /\ phase[r] = IDLE
    /\ phase' = [phase EXCEPT ![r] = ARMED]
    /\ ticket' = [ticket EXCEPT ![r] = [doc |-> d, g |-> gen[d], r |-> rev[d]]]
    /\ UNCHANGED <<live, gen, rev, claims, paneOf, landed, misapplied>>

\* Fresh == the guard Editor::apply actually runs: the document lives,
\* is the same incarnation (arena generation match), the revision has
\* not moved, and there is a revision left to consume.
Fresh(r) ==
    /\ ticket[r].doc \in live
    /\ ticket[r].g = gen[ticket[r].doc]
    /\ ticket[r].r = rev[ticket[r].doc]
    /\ rev[ticket[r].doc] < MAXREV

\* Deliver == Editor::apply(doc, base, ChangeSet): consumes the ticket
\* exactly once. Fresh => the batch publishes (revision consumed and
\* journaled atomically). Stale/dead => the typed Err path: nothing is
\* published, nothing moves — only the ticket is consumed.
Deliver(r) ==
    /\ phase[r] = ARMED
    /\ IF Fresh(r)
       THEN /\ phase' = [phase EXCEPT ![r] = APPLIED]
            /\ landed' = [landed EXCEPT ![r] = ticket[r]]
            /\ rev' = [rev EXCEPT ![ticket[r].doc] = @ + 1]
            /\ claims' = [claims EXCEPT ![ticket[r].doc] = @ \union {rev[ticket[r].doc]}]
            /\ UNCHANGED <<live, gen, paneOf, ticket, misapplied>>
       ELSE /\ phase' = [phase EXCEPT ![r] = DROPPED]
            /\ UNCHANGED <<live, gen, rev, claims, paneOf, ticket, landed, misapplied>>

\* -- the invariants ---------------------------------------------------------

NoStalePane ==
    \A p \in PANES : paneOf[p] /= NONE => paneOf[p] \in live

NoWrongDocument ==
    \A r \in REQS :
        (phase[r] = APPLIED) =>
            /\ landed[r] = ticket[r]
            /\ \/ landed[r].doc \notin live
               \/ /\ landed[r].doc \in live
                  /\ \/ /\ landed[r].g = gen[landed[r].doc]
                        /\ landed[r].r < rev[landed[r].doc]
                     \/ landed[r].g < gen[landed[r].doc]

\* The visible revision IS the journal: one claim per published
\* revision of the current incarnation, gapless from zero. A partial
\* publication (revision bumped, entry missing), a silent rollback, or
\* a double-claim of one revision breaks it.

RevisionTracksPublications ==
    \A d \in DOCS :
        /\ rev[d] = Cardinality(claims[d])
        /\ \A c \in claims[d] : c < rev[d]

\* The ticket state machine, observed: in flight iff armed with a
\* snapshot; a landing exists iff consumed as applied (a deliver that
\* lands without consuming — the double-delivery bug — breaks it).

TicketOneShot ==
    \A r \in REQS :
        /\ (phase[r] = IDLE) <=> (ticket[r].doc = NONE)
        /\ (phase[r] = ARMED) => ticket[r].doc /= NONE
        /\ (phase[r] = APPLIED) <=> (landed[r].doc /= NONE)

NoMisapply == ~misapplied

Next ==
    \/ \E d \in DOCS : OpenDoc(d)
    \/ \E d \in DOCS : CloseDoc(d)
    \/ \E p \in PANES, d \in DOCS : SplitPane(p, d)
    \/ \E d \in DOCS : PublishLocal(d)
    \/ \E r \in REQS, d \in DOCS : Arm(r, d)
    \/ \E r \in REQS : Deliver(r)

Spec == Init /\ [][Next]_<<live, gen, rev, claims, paneOf, phase, ticket, landed, misapplied>>

THEOREM Spec => []TypeOK /\ []NoStalePane /\ []NoWrongDocument
             /\ []RevisionTracksPublications /\ []TicketOneShot /\ []NoMisapply
=============================================================================
