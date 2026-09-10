---- MODULE ChangePlan ----
(***************************************************************************)
(* Shared change plans (0043): one plan over a fixed set of target        *)
(* documents, one revision-checked gateway application per document, one  *)
(* honest receipt. The model checks the lifecycle the Rust code           *)
(* implements in crates/strop/src/editor/changes/mod.rs:                  *)
(*                                                                       *)
(*   Prepare    Editor::build_change_plan captures every target's base    *)
(*              revision in one pure pass; no buffer mutates while the    *)
(*              plan is built                                             *)
(*   Apply(d)   Editor::apply_change_plan's per-document step through     *)
(*              Editor::apply (crates/strop/src/editor/transact.rs): the  *)
(*              gateway publishes only when the document still sits at    *)
(*              the plan's base revision; a mismatch refuses THAT         *)
(*              document by name and earlier successes stand — there is   *)
(*              no all-files atomicity claim                              *)
(*   Finalize   the terminal ChangeReceipt (applied + refused), the       *)
(*              record undo_last_change anchors grouped undo on           *)
(*   Cancel     abandonment before any target was decided; no receipt     *)
(*                                                                       *)
(* An adversarial edit — typing, another worker, any writer outside the   *)
(* plan — advances a document's revision past the plan base. This is     *)
(* exactly what the gateway's revision check exists to catch. Revisions  *)
(* are capped by MAXREV to keep the state space finite; one plan bounds  *)
(* this model.                                                           *)
(*                                                                       *)
(* MUTATION 1: apply without the revision check (must die by             *)
(* NoStaleApplication); 2: drop a refused target from the receipt (must *)
(* die by HonestReceipt); 3: book a refusal as an application (must die *)
(* by PartialProgressHonest).                                            *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS DOCS,       \* target documents of the one plan, e.g. {1, 2}
          MAXREV,     \* revision cap per document (finite state space)
          MUTATION,   \* 0 = honest gateway; 1..3 deliberate faults
          ADVERSITY   \* FALSE quiesces adversarial edits (progress runs)

VARIABLES rev,        \* doc -> current buffer revision (the gateway's clock)
          base,       \* doc -> revision the plan captured at Prepare
          pending,    \* targets not yet decided by this plan
          applied,    \* targets the receipt records as applied
          refused,    \* targets the receipt records as refused (named)
          commits,    \* [doc, base, seen] of every real gateway commit
          phase       \* idle | ready | done | cancelled

vars == <<rev, base, pending, applied, refused, commits, phase>>

Targets == DOCS
Phases == {"idle", "ready", "done", "cancelled"}

TypeOK ==
    /\ rev \in [DOCS -> 0..MAXREV]
    /\ base \in [DOCS -> 0..MAXREV]
    /\ pending \subseteq Targets
    /\ applied \subseteq Targets
    /\ refused \subseteq Targets
    /\ commits \subseteq [doc : DOCS, base : 0..MAXREV, seen : 0..MAXREV]
    /\ phase \in Phases

Init ==
    /\ rev = [d \in DOCS |-> 0]
    /\ base = [d \in DOCS |-> 0]
    /\ pending = {}
    /\ applied = {}
    /\ refused = {}
    /\ commits = {}
    /\ phase = "idle"

\* build_change_plan: capture every target's base revision in one pure
\* pass; nothing mutates while the plan is built.
Prepare ==
    /\ phase = "idle"
    /\ base' = rev
    /\ pending' = Targets
    /\ phase' = "ready"
    /\ UNCHANGED <<rev, applied, refused, commits>>

\* apply_change_plan's per-document step. Editor::apply(document, base,
\* changes) re-checks the revision: a match publishes and advances the
\* clock; a mismatch is a named refusal and nothing moves. MUTATION 1
\* skips the check and publishes blind; MUTATION 2 drops the refusal
\* from the receipt; MUTATION 3 books a refusal as an application.
Apply(d) ==
    /\ phase = "ready"
    /\ d \in pending
    /\ pending' = pending \ {d}
    /\ IF rev[d] = base[d] \/ MUTATION = 1
       THEN /\ rev[d] < MAXREV
            /\ rev' = [rev EXCEPT ![d] = @ + 1]
            /\ commits' = commits \cup
                          {[doc |-> d, base |-> base[d], seen |-> rev[d]]}
            /\ applied' = applied \cup {d}
            /\ UNCHANGED refused
       ELSE /\ applied' = IF MUTATION = 3 THEN applied \cup {d} ELSE applied
            /\ refused' = IF MUTATION = 2 THEN refused ELSE refused \cup {d}
            /\ UNCHANGED <<rev, commits>>
    /\ UNCHANGED <<base, phase>>

\* apply_change_plan's epilogue: once every target is decided the
\* receipt is terminal — applied and refused are now the record.
Finalize ==
    /\ phase = "ready"
    /\ pending = {}
    /\ phase' = "done"
    /\ UNCHANGED <<rev, base, pending, applied, refused, commits>>

\* Abandonment before any target was decided; no receipt is produced.
Cancel ==
    /\ phase = "ready"
    /\ pending = Targets
    /\ phase' = "cancelled"
    /\ UNCHANGED <<rev, base, pending, applied, refused, commits>>

\* Any writer outside the plan advances a document's revision, possibly
\* between the plan's own per-document steps. The plan's base is now
\* stale for that document; the gateway must refuse it.
ExternalEdit(d) ==
    /\ ADVERSITY
    /\ rev[d] < MAXREV
    /\ rev' = [rev EXCEPT ![d] = @ + 1]
    /\ UNCHANGED <<base, pending, applied, refused, commits, phase>>

Terminal == phase \in {"done", "cancelled"} /\ UNCHANGED vars

PlanStep == Prepare \/ Finalize \/ Cancel \/ (\E d \in DOCS : Apply(d))
Next == PlanStep \/ (\E d \in DOCS : ExternalEdit(d)) \/ Terminal

\* Progress assumption: the interactive thread running the plan is
\* weakly fair — a continually enabled plan step eventually fires.
Spec == Init /\ [][Next]_vars /\ WF_vars(PlanStep)

\* A commit is recorded with the revision the gateway actually saw; the
\* record matches the plan base iff the revision check ran. A blind
\* application (MUTATION 1) records base # seen and dies here.
NoStaleApplication == \A c \in commits : c.base = c.seen

\* The terminal receipt partitions the targets exactly: every target is
\* applied or refused, none both, none neither. Dropping a refusal
\* (MUTATION 2) leaves a target unaccounted and dies here.
HonestReceipt ==
    phase = "done" =>
        /\ applied \cup refused = Targets
        /\ applied \cap refused = {}

\* The receipt's applied half tracks the real commits at all times:
\* nothing committed is ever unmarked (a later refusal cannot rewrite
\* earlier progress), and nothing uncommitted is ever marked applied
\* (MUTATION 3 dies here).
PartialProgressHonest == applied = {c.doc : c \in commits}

\* Progress, qualified: with a quiescent adversary (ADVERSITY = FALSE)
\* every plan reaches a terminal phase — applied receipt or cancelled.
PlanSettles == phase \in {"idle", "ready"} ~> phase \in {"done", "cancelled"}

\* Reachability witnesses, checked by expecting their violation.
WitnessNoFullSuccess == ~(phase = "done" /\ applied = Targets)
WitnessNoPartial ==
    ~(phase = "done" /\ Cardinality(applied) = 1 /\ Cardinality(refused) = 1)
WitnessNoInvalidatedAll == ~(phase = "done" /\ refused = Targets)
WitnessNoCancellation == phase # "cancelled"
=============================================================================
