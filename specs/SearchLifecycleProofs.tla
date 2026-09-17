---- MODULE SearchLifecycleProofs ----
(***************************************************************************)
(* TLAPS inductive-safety proofs for SearchLifecycle (0063 §6.6's        *)
(* deferred TLAPS obligation, discharged against 0057 VF17).               *)
(*                                                                         *)
(* THEOREMS (VF17's required shape):                                       *)
(*   InitImpliesInv      Init => Inv                                       *)
(*   InductiveStep       Inv /\ [Next]_vars => Inv'                        *)
(*   InvImpliesSafety    Inv => each named §6.5 safety property            *)
(*                                                                         *)
(* GENERALITY: the proof is over the exact module TLC checks, with the    *)
(* constants left symbolic — arbitrary GENS, REV_MAX, WARM_MAX \in Nat    *)
(* and an arbitrary (finite or not) PATHS set; no constant is ever        *)
(* instantiated. What stays at the TLC model's shape: the binary scope    *)
(* domain {"s1","s2"}, the warm-queue capacity 3 and the stale-accept     *)
(* counter cap 5 of TypeOK. No proof step depends on their cardinalities. *)
(*                                                                         *)
(* The kept mutant's obligations (SearchLifecycle_MutantProofs.tla) do    *)
(* NOT prove — the negative control.                                       *)
(*                                                                         *)
(* PROOF-ENGINEERING NOTE: with the TLAPS 1.5.0 backends, a goal that     *)
(* mixes `x \in BOOLEAN` with any other conjunct in one backend call is   *)
(* not discharged even when each conjunct proves alone, so TypeOK is      *)
(* proved as TypeOKCore plus each BOOLEAN membership in isolation.        *)
(***************************************************************************)
EXTENDS SearchLifecycle

\* Symbolic-constant typing assumptions. Nothing else about the constants
\* is ever used: the theorems hold for every generation/revision/warm
\* budget and every candidate-path set.
ASSUME ConstA == /\ GENS \in Nat
                 /\ REV_MAX \in Nat
                 /\ WARM_MAX \in Nat

vars == <<phase, qgen, scope, inFlight, rows, accepted, staleAccepts,
          truncated, completeF, bufferRev, warm, warmQueue>>

\* TypeOK minus its two BOOLEAN memberships (see the note above).
TypeOKCore ==
    /\ phase \in {"closed", "open"}
    /\ qgen \in 0..GENS
    /\ scope \in {"s1", "s2"}
    /\ \A j \in inFlight : j.gen \in 0..GENS /\ j.scope \in {"s1", "s2"}
    /\ \A r \in rows : r.gen \in 0..GENS /\ r.scope \in {"s1", "s2"}
                       /\ r.path \in PATHS /\ r.rev \in 0..REV_MAX
    /\ \A a \in accepted : a \in [gen: 0..GENS, scope: {"s1","s2"},
                                  path: PATHS, rev: 0..REV_MAX]
    /\ staleAccepts \in 0..5
    /\ bufferRev \in 0..REV_MAX
    /\ warm \in 0..WARM_MAX
    /\ warmQueue \in 0..3

(***************************************************************************)
(* The named invariants alone are not inductive. CompletionHonest's        *)
(* preservation under ProviderDone needs that no two in-flight jobs share  *)
(* a generation (each TypeQuery both bumps qgen and is the only job        *)
(* source), which itself needs that job generations never lead qgen; and   *)
(* Accept's TypeOK obligation needs published rows to be exact 4-field     *)
(* records of the row type (TypeOK types only the field accesses).         *)
(***************************************************************************)
JobsBounded == \A j \in inFlight : j.gen <= qgen
JobsUnique  == \A j, k \in inFlight : j.gen = k.gen => j = k

RowSet == [gen: 0..GENS, scope: {"s1","s2"}, path: PATHS, rev: 0..REV_MAX]
RowsAreRecords == \A r \in rows : r \in RowSet

Inv == /\ TypeOK
       /\ RowsCurrent
       /\ RowsInScope
       /\ StaleAcceptsNever
       /\ CompletionHonest
       /\ WarmBounded
       /\ JobsBounded
       /\ JobsUnique
       /\ RowsAreRecords

THEOREM InitImpliesInv == Init => Inv
  <1>1. Init => TypeOKCore
      BY ConstA DEF Init, TypeOKCore
  <1>2. Init => truncated \in BOOLEAN
      BY DEF Init
  <1>3. Init => completeF \in BOOLEAN
      BY DEF Init
  <1>4. Init => TypeOK
      BY <1>1, <1>2, <1>3 DEF TypeOK, TypeOKCore
  <1>5. Init => RowsCurrent
      BY DEF Init, RowsCurrent
  <1>6. Init => RowsInScope
      BY DEF Init, RowsInScope
  <1>7. Init => StaleAcceptsNever
      BY DEF Init, StaleAcceptsNever
  <1>8. Init => CompletionHonest
      BY DEF Init, CompletionHonest
  <1>9. Init => WarmBounded
      BY ConstA DEF Init, WarmBounded
  <1>10. Init => JobsBounded
      BY DEF Init, JobsBounded
  <1>11. Init => JobsUnique
      BY DEF Init, JobsUnique
  <1>12. Init => RowsAreRecords
      BY DEF Init, RowsAreRecords
  <1>13. QED
      BY <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF Inv

THEOREM InductiveStep == Inv /\ [Next]_vars => Inv'
  <1> SUFFICES ASSUME Inv, [Next]_vars PROVE Inv'
      OBVIOUS
  <1>1. CASE OpenPicker
      BY <1>1, ConstA DEF OpenPicker, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>2. CASE ClosePopup
      <2>1. TypeOKCore'
          BY <1>2, ConstA DEF ClosePopup, Inv, TypeOK, TypeOKCore
      <2>2. truncated' \in BOOLEAN
          BY <1>2 DEF ClosePopup, Inv, TypeOK
      <2>3. completeF' \in BOOLEAN
          BY <1>2 DEF ClosePopup, Inv, TypeOK
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3 DEF TypeOK, TypeOKCore
      <2>5. RowsCurrent'
          BY <1>2 DEF ClosePopup, Inv, RowsCurrent
      <2>6. RowsInScope'
          BY <1>2 DEF ClosePopup, Inv, RowsInScope
      <2>7. StaleAcceptsNever'
          BY <1>2 DEF ClosePopup, Inv, StaleAcceptsNever
      <2>8. CompletionHonest'
          BY <1>2 DEF ClosePopup, Inv, CompletionHonest
      <2>9. WarmBounded'
          BY <1>2 DEF ClosePopup, Inv, WarmBounded
      <2>10. JobsBounded'
          BY <1>2 DEF ClosePopup, Inv, JobsBounded
      <2>11. JobsUnique'
          BY <1>2 DEF ClosePopup, Inv, JobsUnique
      <2>12. RowsAreRecords'
          BY <1>2 DEF ClosePopup, Inv, RowsAreRecords
      <2>13. QED
          BY <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, <2>11, <2>12 DEF Inv
  <1>3. CASE TypeQuery
      <2>1. TypeOKCore'
          BY <1>3, ConstA DEF TypeQuery, Inv, TypeOK, TypeOKCore
      <2>2. truncated' \in BOOLEAN
          BY <1>3 DEF TypeQuery
      <2>3. completeF' \in BOOLEAN
          BY <1>3 DEF TypeQuery
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3 DEF TypeOK, TypeOKCore
      <2>5. RowsCurrent'
          BY <1>3 DEF TypeQuery, Inv, RowsCurrent
      <2>6. RowsInScope'
          BY <1>3 DEF TypeQuery, Inv, RowsInScope
      <2>7. StaleAcceptsNever'
          BY <1>3 DEF TypeQuery, Inv, StaleAcceptsNever
      <2>8. CompletionHonest'
          BY <1>3 DEF TypeQuery, Inv, CompletionHonest
      <2>9. WarmBounded'
          BY <1>3 DEF TypeQuery, Inv, WarmBounded
      <2>10. JobsBounded'
          BY <1>3 DEF TypeQuery, Inv, TypeOK, JobsBounded
      <2>11. JobsUnique'
          BY <1>3 DEF TypeQuery, Inv, TypeOK, JobsBounded, JobsUnique
      <2>12. RowsAreRecords'
          BY <1>3 DEF TypeQuery, RowsAreRecords
      <2>13. QED
          BY <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, <2>11, <2>12 DEF Inv
  <1>4. CASE \E j \in inFlight : ProviderPartial(j)
      BY <1>4, ConstA DEF ProviderPartial, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords, RowSet
  <1>5. CASE \E j \in inFlight : ProviderDone(j)
      BY <1>5, ConstA DEF ProviderDone, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>6. CASE TruncateIndex
      BY <1>6, ConstA DEF TruncateIndex, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>7. CASE EditBuffer
      BY <1>7, ConstA DEF EditBuffer, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>8. CASE \E r \in rows : Accept(r)
      <2>1. PICK r \in rows : Accept(r)
          BY <1>8
      <2>2. TypeOKCore'
          <3>1. r \in RowSet
              BY <2>1 DEF Accept, Inv, RowsAreRecords
          <3>2. \A a \in accepted : a \in RowSet
              BY DEF Inv, TypeOK, RowSet
          <3>3. QED
              BY <2>1, <3>1, <3>2, ConstA DEF Accept, Inv, TypeOK,
                 TypeOKCore, RowSet
      <2>3. truncated' \in BOOLEAN
          BY <2>1 DEF Accept, Inv, TypeOK
      <2>4. completeF' \in BOOLEAN
          BY <2>1 DEF Accept, Inv, TypeOK
      <2>5. TypeOK'
          BY <2>2, <2>3, <2>4 DEF TypeOK, TypeOKCore
      <2>6. RowsCurrent'
          BY <2>1 DEF Accept, Inv, RowsCurrent
      <2>7. RowsInScope'
          BY <2>1 DEF Accept, Inv, RowsInScope
      <2>8. StaleAcceptsNever'
          BY <2>1 DEF Accept, Inv, StaleAcceptsNever
      <2>9. CompletionHonest'
          BY <2>1 DEF Accept, Inv, CompletionHonest
      <2>10. WarmBounded'
          BY <2>1 DEF Accept, Inv, WarmBounded
      <2>11. JobsBounded'
          BY <2>1 DEF Accept, Inv, JobsBounded
      <2>12. JobsUnique'
          BY <2>1 DEF Accept, Inv, JobsUnique
      <2>13. RowsAreRecords'
          BY <2>1 DEF Accept, Inv, RowsAreRecords
      <2>14. QED
          BY <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, <2>11, <2>12, <2>13
             DEF Inv
  <1>9. CASE WarmStart
      BY <1>9, ConstA DEF WarmStart, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>10. CASE WarmDone
      BY <1>10, ConstA DEF WarmDone, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>11. CASE WarmEnqueue
      BY <1>11, ConstA DEF WarmEnqueue, Inv, TypeOK, RowsCurrent,
         RowsInScope, StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>12. CASE Restart
      <2>1. TypeOKCore'
          BY <1>12, ConstA DEF Restart, Inv, TypeOK, TypeOKCore
      <2>2. truncated' \in BOOLEAN
          BY <1>12 DEF Restart
      <2>3. completeF' \in BOOLEAN
          BY <1>12 DEF Restart
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3 DEF TypeOK, TypeOKCore
      <2>5. RowsCurrent'
          BY <1>12 DEF Restart, Inv, RowsCurrent
      <2>6. RowsInScope'
          BY <1>12 DEF Restart, Inv, RowsInScope
      <2>7. StaleAcceptsNever'
          BY <1>12 DEF Restart, Inv, StaleAcceptsNever
      <2>8. CompletionHonest'
          BY <1>12 DEF Restart, Inv, CompletionHonest
      <2>9. WarmBounded'
          BY <1>12, ConstA DEF Restart, Inv, WarmBounded
      <2>10. JobsBounded'
          BY <1>12 DEF Restart, Inv, JobsBounded
      <2>11. JobsUnique'
          BY <1>12 DEF Restart, Inv, JobsUnique
      <2>12. RowsAreRecords'
          BY <1>12 DEF Restart, RowsAreRecords
      <2>13. QED
          BY <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, <2>11, <2>12 DEF Inv
  <1>13. CASE UNCHANGED vars
      BY <1>13 DEF vars, Inv, TypeOK, RowsCurrent, RowsInScope,
         StaleAcceptsNever, CompletionHonest, WarmBounded,
         JobsBounded, JobsUnique, RowsAreRecords
  <1>14. QED
      BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10,
         <1>11, <1>12, <1>13 DEF Next, vars

\* VF17: Inv => each named safety property (they are conjuncts of Inv).
THEOREM InvImpliesSafety ==
    Inv => /\ RowsCurrent
           /\ RowsInScope
           /\ StaleAcceptsNever
           /\ CompletionHonest
           /\ WarmBounded
  BY DEF Inv

=============================================================================
