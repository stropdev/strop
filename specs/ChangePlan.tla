---- MODULE ChangePlan ----
(***************************************************************************)
(* Shared change plans (0043) and source-backed projection admission      *)
(* (0057 VF04): one plan over a fixed set of target MEMBERS — each       *)
(* member is one writable span of one source document, or a protected    *)
(* chrome row (title/border/gap) that rides the projection but must      *)
(* never become source text. The model checks the lifecycle the Rust     *)
(* code implements in crates/strop-engine/src/editor/changes/mod.rs and  *)
(* crates/strop-engine/src/editor/collections/editing.rs:                *)
(*                                                                       *)
(*   Prepare    Editor::build_change_plan captures every target's base   *)
(*              revision in one pure pass; no buffer mutates while the   *)
(*              plan is built. Projection admission happens here too:    *)
(*              protected rows are refused by name at admission (the     *)
(*              write-back's "edit touches generated chrome" refusal)    *)
(*              and never join a source's pending group                  *)
(*   ApplyGroup apply_change_plan's per-document step through            *)
(*              Editor::apply (editor/transact.rs), and the collection   *)
(*              write-back's per-source preflight+apply: all of one      *)
(*              source's member edits publish as ONE validated batch     *)
(*              (Buffer::prepare_replacements' sorted/non-overlapping    *)
(*              check is the overlap guard), atomically, only while the  *)
(*              source still sits at the plan's base revision; a         *)
(*              mismatch or an overlapping group refuses the WHOLE       *)
(*              group by name and earlier successes stand — there is     *)
(*              no all-files atomicity claim                             *)
(*   Finalize   the terminal ChangeReceipt (applied + refused), the      *)
(*              record undo_last_change anchors grouped undo on          *)
(*   Cancel     abandonment before any member was decided; no receipt    *)
(*                                                                       *)
(* An adversarial edit — typing, another worker, any writer outside the  *)
(* plan — advances a document's revision past the plan base. This is     *)
(* exactly what the gateway's revision check exists to catch. Revisions  *)
(* are capped by MAXREV to keep the state space finite; one plan bounds  *)
(* this model.                                                           *)
(*                                                                       *)
(* VF04 projection contract, as the named invariants below:              *)
(*   - protected headers/gaps do not become source text                  *)
(*     (ProtectedNeverApplied);                                          *)
(*   - repeated/overlapping excerpts do not duplicate a mutation: one    *)
(*     commit never carries two members whose spans overlap              *)
(*     (NoDuplicateMutation);                                            *)
(*   - coordinates cannot cross into another source or namespace: a      *)
(*     commit's revision effect lands on exactly the source its members  *)
(*     resolve to (MemberSourceBound);                                   *)
(*   - grouped edits decide per group, and a partial outcome is          *)
(*     identified per member rather than hidden behind a success count   *)
(*     (GroupUniformOutcome, HonestReceipt, PartialProgressHonest).      *)
(*                                                                       *)
(* MUTATION 1: apply without the revision check (must die by             *)
(* NoStaleApplication); 2: drop a refused group from the receipt (must   *)
(* die by HonestReceipt); 3: book a refusal as an application (must die  *)
(* by PartialProgressHonest); 4: admit a protected row into a source     *)
(* group (must die by ProtectedNeverApplied); 5: skip the overlap check  *)
(* so an overlapping excerpt pair both apply (must die by                *)
(* NoDuplicateMutation); 6: land a group's revision effect on a          *)
(* different source (must die by MemberSourceBound); 7: apply a proper  *)
(* subset of a group and refuse the rest (must die by                    *)
(* GroupUniformOutcome).                                                 *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS DOCS,       \* source documents — this instance uses {1, 2, 3}
          MAXREV,     \* revision cap per document (finite state space)
          MUTATION,   \* 0 = honest gateway; 1..7 deliberate faults
          ADVERSITY   \* FALSE quiesces adversarial edits (progress runs)

\* The VF04 fixture instance, defined here because the TLC cfg syntax
\* accepts only flat values: six projection rows over three sources.
\* Members 1 and 2 are writable spans of source 1, member 3 a writable
\* span of source 2, member 4 a protected chrome row (header/gap) on
\* source 1, and members 5 and 6 the overlapping excerpt pair — repeated
\* excerpts of one source 3 span (the duplicate-mutation probe).
MEMBERS == 1..6
SOURCEOF == [m \in MEMBERS |->
    IF m \in {1, 2, 4} THEN 1 ELSE IF m = 3 THEN 2 ELSE 3]
PROTECTED == {4}
OVERLAPPAIR == {5, 6}

VARIABLES rev,        \* doc -> current buffer revision (the gateway's clock)
          base,       \* doc -> revision the plan captured at Prepare
          pending,    \* members not yet decided by this plan
          applied,    \* members the receipt records as applied
          refused,    \* members the receipt records as refused (named)
          commits,    \* [source, base, seen, members, effectOn] of every
                      \* real gateway commit: the admitted group, the
                      \* revision evidence, and the document whose clock
                      \* actually advanced
          phase       \* idle | ready | done | cancelled

vars == <<rev, base, pending, applied, refused, commits, phase>>

Targets == MEMBERS
Writable == MEMBERS \ PROTECTED
Phases == {"idle", "ready", "done", "cancelled"}

Overlapping(a, b) == a # b /\ a \in OVERLAPPAIR /\ b \in OVERLAPPAIR

\* One source's member edits publish as one batch; the batch validation
\* (check_batch, proved in strop-core's editmap) rejects overlapping
\* spans, so a conflicted group can only be refused, never applied.
ConflictFree(group) == \A a, b \in group : a = b \/ ~Overlapping(a, b)

\* Members whose span overlaps another member's: they can never appear
\* in an honest commit, so "full success" quantifies over the rest.
Conflicted == {m \in MEMBERS : \E o \in MEMBERS : Overlapping(m, o)}

TypeOK ==
    /\ rev \in [DOCS -> 0..MAXREV]
    /\ base \in [DOCS -> 0..MAXREV]
    /\ pending \subseteq Targets
    /\ applied \subseteq Targets
    /\ refused \subseteq Targets
    /\ commits \subseteq [source : DOCS, base : 0..MAXREV, seen : 0..MAXREV,
                          members : SUBSET MEMBERS, effectOn : DOCS]
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
\* pass; nothing mutates while the plan is built. Projection admission
\* refuses protected rows by name here — chrome never joins a source's
\* pending group. MUTATION 4 admits them instead.
Prepare ==
    /\ phase = "idle"
    /\ base' = rev
    /\ pending' = IF MUTATION = 4 THEN Targets ELSE Writable
    /\ refused' = IF MUTATION = 4 THEN refused ELSE refused \cup PROTECTED
    /\ phase' = "ready"
    /\ UNCHANGED <<rev, applied, commits>>

\* apply_change_plan's per-document step, and the collection write-back's
\* per-source group: Editor::apply / apply_prepared re-check the revision
\* and publish one validated batch for all of the source's members — a
\* match advances THAT source's clock and journals the commit; a mismatch
\* or an overlapping group is a named refusal of the whole group and
\* nothing moves. MUTATION 1 skips the revision check and publishes
\* blind; 2 drops the refusal from the receipt; 3 books a refusal as an
\* application; 5 skips the overlap check; 6 lands the revision effect
\* on a different source; 7 applies a proper subset and refuses the rest.
ApplyGroup(s) ==
    /\ phase = "ready"
    /\ LET group == {m \in pending : SOURCEOF[m] = s}
       IN /\ group # {}
          /\ pending' = pending \ group
          /\ IF (ConflictFree(group) \/ MUTATION = 5)
                /\ (rev[s] = base[s] \/ MUTATION = 1)
             THEN \* admitted: one atomic commit for the whole group
                  /\ LET eff == IF MUTATION = 6
                                THEN CHOOSE x \in DOCS \ {s} : TRUE
                                ELSE s
                     IN /\ rev[eff] < MAXREV
                        /\ rev' = [rev EXCEPT ![eff] = @ + 1]
                        /\ IF MUTATION = 7 /\ Cardinality(group) > 1
                           THEN \E sub \in (SUBSET group) \ {{}, group} :
                                    /\ applied' = applied \cup sub
                                    /\ refused' = refused \cup (group \ sub)
                                    /\ commits' = commits \cup
                                       {[source |-> s, base |-> base[s],
                                         seen |-> rev[s], members |-> sub,
                                         effectOn |-> eff]}
                           ELSE /\ applied' = applied \cup group
                                /\ UNCHANGED refused
                                /\ commits' = commits \cup
                                   {[source |-> s, base |-> base[s],
                                     seen |-> rev[s], members |-> group,
                                     effectOn |-> eff]}
             ELSE \* refused: the whole group is named in the receipt
                  /\ applied' = IF MUTATION = 3 THEN applied \cup group ELSE applied
                  /\ refused' = IF MUTATION = 2 THEN refused ELSE refused \cup group
                  /\ UNCHANGED <<rev, commits>>
    /\ UNCHANGED <<base, phase>>

\* apply_change_plan's epilogue: once every member is decided the
\* receipt is terminal — applied and refused are now the record.
Finalize ==
    /\ phase = "ready"
    /\ pending = {}
    /\ phase' = "done"
    /\ UNCHANGED <<rev, base, pending, applied, refused, commits>>

\* Abandonment before any member was decided; no receipt is produced.
Cancel ==
    /\ phase = "ready"
    /\ pending = (IF MUTATION = 4 THEN Targets ELSE Writable)
    /\ phase' = "cancelled"
    /\ UNCHANGED <<rev, base, pending, applied, refused, commits>>

\* Any writer outside the plan advances a document's revision, possibly
\* between the plan's own per-source steps. The plan's base is now
\* stale for that document; the gateway must refuse it.
ExternalEdit(d) ==
    /\ ADVERSITY
    /\ rev[d] < MAXREV
    /\ rev' = [rev EXCEPT ![d] = @ + 1]
    /\ UNCHANGED <<base, pending, applied, refused, commits, phase>>

Terminal == phase \in {"done", "cancelled"} /\ UNCHANGED vars

PlanStep == Prepare \/ Finalize \/ Cancel \/ (\E s \in DOCS : ApplyGroup(s))
Next == PlanStep \/ (\E d \in DOCS : ExternalEdit(d)) \/ Terminal

\* Progress assumption: the interactive thread running the plan is
\* weakly fair — a continually enabled plan step eventually fires.
Spec == Init /\ [][Next]_vars /\ WF_vars(PlanStep)

\* A commit is recorded with the revision the gateway actually saw; the
\* record matches the plan base iff the revision check ran. A blind
\* application (MUTATION 1) records base # seen and dies here.
NoStaleApplication == \A c \in commits : c.base = c.seen

\* The terminal receipt partitions the targets exactly: every member is
\* applied or refused, none both, none neither — a partial outcome is
\* identified PER MEMBER, never hidden behind a success count. Dropping
\* a refusal (MUTATION 2) leaves a member unaccounted and dies here.
HonestReceipt ==
    phase = "done" =>
        /\ applied \cup refused = Targets
        /\ applied \cap refused = {}

\* The receipt's applied half tracks the real commits at all times:
\* nothing committed is ever unmarked (a later refusal cannot rewrite
\* earlier progress), and nothing uncommitted is ever marked applied
\* (MUTATION 3 dies here).
PartialProgressHonest == applied = UNION {c.members : c \in commits}

\* VF04: protected headers/gaps never become source text — a protected
\* row is never applied and never rides inside a commit's member set.
\* MUTATION 4 admits one and dies here.
ProtectedNeverApplied ==
    /\ applied \cap PROTECTED = {}
    /\ \A c \in commits : c.members \cap PROTECTED = {}

\* VF04: repeated/overlapping excerpts do not duplicate a mutation — no
\* single commit carries two members whose spans overlap (the batch
\* overlap check refuses such a group as a whole). MUTATION 5 dies here.
NoDuplicateMutation ==
    \A c \in commits :
        \A a, b \in c.members : a = b \/ ~Overlapping(a, b)

\* VF04: coordinates cannot cross into another source or namespace — a
\* commit's members all resolve to its source, and its revision effect
\* lands on that same source. MUTATION 6 dies here.
MemberSourceBound ==
    \A c \in commits :
        /\ c.effectOn = c.source
        /\ \A m \in c.members : SOURCEOF[m] = c.source

\* VF04: grouped edits decide per group — two writable members of one
\* source always share their outcome (both applied or both refused).
\* MUTATION 7 splits one and dies here.
GroupUniformOutcome ==
    \A a, b \in Writable :
        SOURCEOF[a] = SOURCEOF[b] =>
            /\ (a \in applied) <=> (b \in applied)
            /\ (a \in refused) <=> (b \in refused)

\* Progress, qualified: with a quiescent adversary (ADVERSITY = FALSE)
\* every plan reaches a terminal phase — applied receipt or cancelled.
PlanSettles == phase \in {"idle", "ready"} ~> phase \in {"done", "cancelled"}

\* Reachability witnesses, checked by expecting their violation.
\* Conflicted members can never appear in an honest commit, so full
\* success means every writable, conflict-free member applied.
WitnessNoFullSuccess == ~(phase = "done" /\ applied = Writable \ Conflicted)
WitnessNoPartial ==
    ~(phase = "done" /\ applied # {} /\ (Writable \ Conflicted) \cap refused # {})
WitnessNoInvalidatedAll == ~(phase = "done" /\ applied = {})
WitnessNoCancellation == phase # "cancelled"
=============================================================================
