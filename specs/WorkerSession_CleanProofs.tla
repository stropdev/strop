---- MODULE WorkerSession_CleanProofs ----
EXTENDS WorkerSession, TLAPS

\* Matched positive control: without mutation 7, Commit requires the
\* captured worker incarnation to match the prepared attempt. The same
\* theorem fails in WorkerSession_MutantProofs when that guard is removed.
ASSUME CleanInputs == MUTATION = 0

THEOREM StaleCommitPreservesSafety ==
    \A c \in Clients :
        (NoStaleCommit /\ ~FreshAttempt(c) /\ Commit(c)) => NoStaleCommit'
    BY CleanInputs DEF CleanInputs, NoStaleCommit, FreshAttempt, Commit, Bound, StateRecord
=============================================================================
