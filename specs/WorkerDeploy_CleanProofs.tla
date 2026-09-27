---- MODULE WorkerDeploy_CleanProofs ----
EXTENDS WorkerDeploy, TLAPS

\* Matched positive control: without mutation 7, cache collection is
\* scoped to the actor's own selected context, so owned cleanup holds.
\* The same theorem fails in WorkerDeploy_MutantProofs.
ASSUME CleanInputs == MUTATION = 0

THEOREM CollectPreservesOwnedCleanup ==
    \A actor \in Clients, t \in Targets :
        (OwnedCleanup /\ Collect(actor, t)) => OwnedCleanup'
  <1>1. SUFFICES ASSUME NEW CONSTANT actor \in Clients,
                       NEW CONSTANT t \in Targets,
                       OwnedCleanup, Collect(actor, t)
             PROVE OwnedCleanup'
      OBVIOUS
  <1>2. (state'.badCleanup) = ((state.badCleanup)
             \/ ((state.selected)[actor] # t.context))
      BY <1>1 DEF Collect, StateRecord
  <1>3. (state.selected)[actor] = t.context
      BY <1>1, CleanInputs DEF CleanInputs, Collect
  <1>4. QED
      BY <1>1, <1>2, <1>3 DEF OwnedCleanup
=============================================================================
