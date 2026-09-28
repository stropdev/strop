---- MODULE CompletionOwnership_MutantProofs ----
EXTENDS CompletionOwnership, TLAPS

ASSUME Unguarded == MUTATION = 7
\* Matched negative control for the independent apply-time ownership guard.
THEOREM StaleAcceptancePreservesSafety ==
    \A i \in Slots :
        (~s.staleApply /\ ~Current(s.data[i]) /\ Apply(i)) => ~s'.staleApply
    BY Unguarded DEF Unguarded, Current, NativeOwns, Apply, StateRecord
=============================================================================
