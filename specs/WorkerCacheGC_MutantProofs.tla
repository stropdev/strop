---- MODULE WorkerCacheGC_MutantProofs ----
EXTENDS WorkerCacheGC, TLAPS

\* Matched negative control: mutation 3 bypasses the worker's
\* admitted-context check at the native retirement boundary.
\* The exact positive scoped-retirement theorem must fail here.
ASSUME BadInputs == MUTATION = 3

THEOREM RetirePreservesScope ==
    \A actor \in Clients, t \in Targets :
        (ScopedRetirement /\ Retire(actor, t)) => ScopedRetirement'
    BY BadInputs
       DEF BadInputs, ScopedRetirement, Retire, StateRecord,
           Target, NoTarget, Targets
=============================================================================
