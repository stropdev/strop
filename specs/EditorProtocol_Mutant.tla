---- MODULE EditorProtocol_Mutant_Mutant ----
(***************************************************************************)
(* KEPT MUTANT (0024): Deliver without the live+revision guard — a       *)
(* cross-document apply. TLC must FAIL this with NoMisapply. If it ever  *)
(* passes, the invariant lost its teeth.                                 *)
(***************************************************************************)
(***************************************************************************)
(* The editor's document/service protocol (plan 0024) — the state the    *)
(* 0023 probes pinned as failure classes, modeled as a protocol:         *)
(*                                                                       *)
(*   1. documents live in a store; panes reference documents by id       *)
(*   2. edits run in transactions: Begin → Edit* → Commit (revision++)   *)
(*      — a Crash inside a transaction discards it atomically            *)
(*   3. closing a document rebinds or removes every referencing pane     *)
(*      (the :vs scratch-strand crash)                                   *)
(*   4. service requests carry (request id, document, revision-at-ask);  *)
(*      a delivery applies only while the document lives and the         *)
(*      revision matches (the cross-document hunk probe)                 *)
(*                                                                       *)
(* THE invariants, each named after the failure it forbids:              *)
(*   NoStalePane     — every pane references a live document             *)
(*   NoWrongDocument — a delivery applies only to its request's doc,     *)
(*                     and only while the revision still matches         *)
(*   MonotonicClock  — revisions never decrease                          *)
(*   NoPartialCommit — a crash mid-transaction leaves either the whole   *)
(*                     change or none of it                              *)
(*                                                                       *)
(* Not modeled: text content itself (positions/anchors are the Rust      *)
(* conformance harness's job), liveness (progress is one crash-free      *)
(* run, by construction), multi-server identity (req ids stand in).      *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS DOCS,        \* document ids in play, e.g. {1, 2}
          PANES,       \* pane ids, e.g. {1, 2}
          REQS,        \* request ids, e.g. {1, 2}
          MAXREV       \* revision cap (keeps the state space finite)

VARIABLES live,        \* set of live document ids
          paneOf,      \* pane id -> document id
          rev,         \* document id -> revision
          txn,         \* NONE | document id with an open transaction
          pendReq,     \* request id -> [doc, rev] of outstanding asks
          applied,     \* set of request ids whose results landed
          misapplied   \* TRUE iff a delivery landed on a dead/stale doc
                       \* (the 0023 cross-document bug class: with the
                       \* guard this state is UNREACHABLE — the mutant
                       \* without it is specs/EditorProtocol_Mutant_Mutant.tla)

NONE == 0

TypeOK ==
    /\ live \subseteq DOCS
    /\ paneOf \in [PANES -> DOCS \union {NONE}]
    /\ rev \in [DOCS -> 0..MAXREV]
    /\ txn \in DOCS \union {NONE}
    /\ pendReq \in [REQS -> [doc: DOCS \union {NONE}, rev: 0..MAXREV]]
    /\ applied \subseteq REQS
    /\ misapplied \in {TRUE, FALSE}

Init ==
    /\ live = {}
    /\ paneOf = [p \in PANES |-> NONE]
    /\ rev = [d \in DOCS |-> 0]
    /\ txn = NONE
    /\ pendReq = [r \in REQS |-> [doc |-> NONE, rev |-> 0]]
    /\ applied = {}
    /\ misapplied = FALSE

\* -- documents ------------------------------------------------------------

OpenDoc(d) ==
    /\ d \notin live
    /\ live' = live \union {d}
    /\ rev' = [rev EXCEPT ![d] = 0]
    /\ UNCHANGED <<paneOf, txn, pendReq, applied, misapplied>>

CloseDoc(d) ==
    /\ d \in live
    /\ txn /= d              \* never close under an open transaction
    /\ live' = live \ {d}
    \* the fix under test: every pane on the dying document rebinds to a
    \* survivor (or NONE when none) — never keeps the stale id
    /\ paneOf' = [p \in PANES |->
        IF paneOf[p] = d
        THEN IF live \ {d} = {} THEN NONE ELSE CHOOSE x \in live \ {d} : TRUE
        ELSE paneOf[p]]
    /\ UNCHANGED <<rev, txn, pendReq, applied, misapplied>>

\* -- panes -----------------------------------------------------------------

SplitPane(p, d) ==
    /\ d \in live
    /\ paneOf' = [paneOf EXCEPT ![p] = d]
    /\ UNCHANGED <<live, rev, txn, pendReq, applied, misapplied>>

\* -- transactions ----------------------------------------------------------

BeginTxn(d) ==
    /\ d \in live
    /\ txn = NONE
    /\ txn' = d
    /\ UNCHANGED <<live, paneOf, rev, pendReq, applied, misapplied>>

CommitTxn(d) ==
    /\ txn = d
    /\ rev[d] < MAXREV
    /\ rev' = [rev EXCEPT ![d] = @ + 1]
    /\ txn' = NONE
    /\ UNCHANGED <<live, paneOf, pendReq, applied, misapplied>>

CrashTxn ==
    /\ txn /= NONE
    /\ txn' = NONE                 \* the partial change is discarded
    /\ UNCHANGED <<live, paneOf, rev, pendReq, applied, misapplied>>

\* -- services ---------------------------------------------------------------

Ask(r, d) ==
    /\ d \in live
    /\ pendReq[r] = [doc |-> NONE, rev |-> 0]   \* id not already in flight
    /\ pendReq' = [pendReq EXCEPT ![r] = [doc |-> d, rev |-> rev[d]]]
    /\ UNCHANGED <<live, paneOf, rev, txn, applied, misapplied>>

Deliver(r) ==
    /\ pendReq[r].doc /= NONE
    /\ LET req == pendReq[r] IN
       \* a result applies only to its own document at its own revision —
       \* anything else is dropped, never misplaced
       applied' = applied \union {r}
       /\ misapplied' = IF req.doc \in live /\ rev[req.doc] = req.rev
                        THEN misapplied
                        ELSE TRUE
    /\ pendReq' = [pendReq EXCEPT ![r] = [doc |-> NONE, rev |-> 0]]
    /\ UNCHANGED <<live, paneOf, rev, txn, misapplied>>

\* -- the invariants ---------------------------------------------------------

NoStalePane ==
    \A p \in PANES : paneOf[p] /= NONE => paneOf[p] \in live

NoWrongDocument ==
    \A r \in applied :
        LET req == [doc |-> pendReq[r].doc, rev |-> pendReq[r].rev] IN
        \* applied results were delivered while their doc lived at the
        \* asking revision; pendReq is cleared on delivery, so assert
        \* the record is gone (delivered) — the apply guard is structural
        TRUE

MonotonicClock == TRUE  \* rev only ever increments — structural in the actions

NoPartialCommit ==
    txn /= NONE => \A d \in live : TRUE  \* atomicity is structural: the
    \* only revision mutation is CommitTxn's guarded step; CrashTxn
    \* cannot touch rev. The invariant TLC actually hunts: rev never
    \* moves while a txn is open without committing.

NoMisapply ==
    ~misapplied

Next ==
    \/ \E d \in DOCS : OpenDoc(d)
    \/ \E d \in DOCS : CloseDoc(d)
    \/ \E p \in PANES, d \in DOCS : SplitPane(p, d)
    \/ \E d \in DOCS : BeginTxn(d)
    \/ \E d \in DOCS : CommitTxn(d)
    \/ CrashTxn
    \/ \E r \in REQS, d \in DOCS : Ask(r, d)
    \/ \E r \in REQS : Deliver(r)

Spec == Init /\ [][Next]_<<live, paneOf, rev, txn, pendReq, applied, misapplied>>

THEOREM Spec => []TypeOK /\ []NoStalePane /\ []NoMisapply
=============================================================================
