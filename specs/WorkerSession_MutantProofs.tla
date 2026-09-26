---- MODULE WorkerSession_MutantProofs ----
EXTENDS WorkerSession, TLAPS

\* Kept negative control: mutation 7 permits Commit after the bound worker
\* restarts, although FreshAttempt(c) is false.  An unproved obligation
\* here must be the NoStaleCommit induction step, never a parser failure.
ASSUME BadInputs == MUTATION = 7

THEOREM StaleCommitPreservesSafety ==
    \A c \in Clients :
        (NoStaleCommit /\ ~FreshAttempt(c) /\ Commit(c)) => NoStaleCommit'
    BY BadInputs DEF BadInputs, NoStaleCommit, FreshAttempt, Commit, Bound, StateRecord
=============================================================================
