---- MODULE CompletionOwnership_CleanProofs ----
EXTENDS CompletionOwnership, TLAPS

ASSUME Guarded == MUTATION = 0
\* The identical theorem is rejected with mutation 7: a prepared operation
\* no longer matching the live epoch/selection must not be applied.
THEOREM StaleAcceptancePreservesSafety ==
    \A i \in Slots :
        (~s.staleApply /\ ~Current(s.data[i]) /\ Apply(i)) => ~s'.staleApply
    BY Guarded DEF Guarded, Current, NativeOwns, Apply, StateRecord
=============================================================================
