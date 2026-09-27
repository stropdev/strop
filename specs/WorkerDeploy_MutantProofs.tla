---- MODULE WorkerDeploy_MutantProofs ----
EXTENDS WorkerDeploy, TLAPS

\* Kept negative control: mutation 7 lets an actor retire another
\* context's object (the cross-context cache-cleanup mutant). The
\* OwnedCleanup preservation step must fail here on its own obligation,
\* never on tooling.
ASSUME BadInputs == MUTATION = 7

THEOREM CollectPreservesOwnedCleanup ==
    \A actor \in Clients, t \in Targets :
        (OwnedCleanup /\ Collect(actor, t)) => OwnedCleanup'
    BY BadInputs
       DEF BadInputs, OwnedCleanup, Collect, StateRecord,
           NoTarget, None, Targets
=============================================================================
