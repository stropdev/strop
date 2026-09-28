---- MODULE CompletionOwnershipProofs ----
EXTENDS CompletionOwnership, TLAPS

\* Generalized preservation over the SAME transition relation checked by TLC.
\* Explicit FALSE values supply the Boolean typing needed by TLA's untyped
\* record projections. Cardinality/retention and OS fairness are separate.
ASSUME Honest == MUTATION = 0
Freshness == /\ s.stalePublish = FALSE /\ s.staleApply = FALSE
             /\ s.staleView = FALSE /\ s.restarted = FALSE
THEOREM InitialFreshness == Init => Freshness
    BY DEF Init, Freshness
THEOREM FreshnessImpliesSafety == Freshness =>
    /\ NativePublicationOwned /\ NoStalePresentationOrApply /\ IndexNotRestartedByQuery
    BY DEF Freshness, NativePublicationOwned, NoStalePresentationOrApply, IndexNotRestartedByQuery

THEOREM FreshnessPreserved == Freshness /\ [Next]_vars => Freshness'
  <1> SUFFICES ASSUME Freshness, [Next]_vars PROVE Freshness'
      OBVIOUS
  <1>1. CASE NewQuery
      <2>1. MUTATION # 2 BY SMT, Honest DEF Honest
      <2>2. (s.restarted \/ (MUTATION = 2 /\ s.indexStep > 0)) = FALSE
          BY <2>1 DEF Freshness
      <2>3. QED BY <1>1, <2>2 DEF Freshness, NewQuery, StateRecord
  <1>2. CASE IndexStep
      BY <1>2 DEF Freshness, IndexStep, StateRecord
  <1>3. CASE BeginQuery
      BY <1>3 DEF Freshness, BeginQuery, StateRecord
  <1>4. CASE DropObsoleteWork
      BY <1>4 DEF Freshness, DropObsoleteWork, StateRecord
  <1>5. CASE Choose
      BY <1>5 DEF Freshness, Choose, StateRecord
  <1>6. CASE Prepare
      BY <1>6 DEF Freshness, Prepare, StateRecord
  <1>7. CASE Cancel
      BY <1>7 DEF Freshness, Cancel, StateRecord
  <1>8. CASE ChangeContext
      BY <1>8 DEF Freshness, ChangeContext, StateRecord
  <1>9. CASE Disable
      BY <1>9 DEF Freshness, Disable, StateRecord
  <1>10. CASE \E i \in Slots : PublishRows(i)
      <2>1. PICK i \in Slots : PublishRows(i) BY <1>10
      <2>2. NativeOwns(Payload(s.working, "rows"))
          BY SMT, <2>1, Honest DEF Honest, PublishRows
      <2>3. (s.stalePublish \/ ~NativeOwns(Payload(s.working, "rows"))) = FALSE
          BY <2>2 DEF Freshness
      <2>4. QED BY <2>1, <2>3 DEF Freshness, PublishRows, StateRecord
  <1>11. CASE \E i \in Slots : PublishPlan(i)
      BY <1>11 DEF Freshness, PublishPlan, StateRecord
  <1>12. CASE \E i \in Slots : Acquire(i)
      BY <1>12 DEF Freshness, Acquire, StateRecord
  <1>13. CASE \E i \in Slots : Show(i)
      <2>1. PICK i \in Slots : Show(i) BY <1>13
      <2>2. Current(s.data[i]) BY SMT, <2>1, Honest DEF Honest, Show
      <2>3. (s.staleView \/ ~Current(s.data[i])) = FALSE BY <2>2 DEF Freshness
      <2>4. QED BY <2>1, <2>3 DEF Freshness, Show, StateRecord
  <1>14. CASE \E i \in Slots : Apply(i)
      <2>1. PICK i \in Slots : Apply(i) BY <1>14
      <2>2. Current(s.data[i]) BY SMT, <2>1, Honest DEF Honest, Apply
      <2>3. (s.staleApply \/ ~Current(s.data[i])) = FALSE BY <2>2 DEF Freshness
      <2>4. QED BY <2>1, <2>3 DEF Freshness, Apply, StateRecord
  <1>15. CASE \E i \in Slots : ReleaseUI(i)
      BY <1>15 DEF Freshness, ReleaseUI, StateRecord
  <1>16. CASE \E i \in Slots : Retire(i)
      BY <1>16 DEF Freshness, Retire, StateRecord
  <1>17. CASE UNCHANGED vars
      BY <1>17 DEF Freshness, vars
  <1>18. QED
      BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9,
         <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17 DEF Next, vars

THEOREM AlwaysFresh == Spec => []Freshness
    BY InitialFreshness, FreshnessPreserved, PTL DEF Spec, vars
=============================================================================
