---- MODULE WorkerCacheGCProofs ----
EXTENDS WorkerCacheGC, TLAPS

\* The bounded checker establishes SnapshotComplete for two clients;
\* these arbitrary-set theorems prove that a retired object cannot be
\* live, stale-pinned or foreign once that snapshot premise holds.
\* This is NOT an induction for SnapshotComplete or a Rust/OS proof.
ASSUME HonestInputs == MUTATION = 0 /\ Clients # {}
    /\ Contexts # {} /\ Digests # {}

THEOREM RetirePreservesScope ==
    \A actor \in Clients, t \in Targets :
        (ScopedRetirement /\ Retire(actor, t)) => ScopedRetirement'
  <1>1. SUFFICES ASSUME NEW CONSTANT actor \in Clients,
                         NEW CONSTANT t \in Targets,
                         ScopedRetirement, Retire(actor, t)
               PROVE ScopedRetirement'
      OBVIOUS
  <1>2. (state'.foreignRetirement) =
        ((state.foreignRetirement) \/ (t.context # Target(actor).context))
      BY <1>1 DEF Retire, StateRecord
  <1>3. Target(actor).context = t.context
      BY <1>1, HonestInputs DEF HonestInputs, Retire
  <1>4. QED BY <1>1, <1>2, <1>3 DEF ScopedRetirement

THEOREM RetirePreservesLive ==
    \A actor \in Clients, t \in Targets :
        (SnapshotComplete /\ NoRetireLive /\ Retire(actor, t)) =>
            NoRetireLive'
  <1>1. SUFFICES ASSUME NEW CONSTANT actor \in Clients,
                         NEW CONSTANT t \in Targets,
                         SnapshotComplete, NoRetireLive,
                         Retire(actor, t)
               PROVE NoRetireLive'
      OBVIOUS
  <1>2. state.phase[actor] = "observed" /\ state.owner = actor
         /\ t \notin state.snapshot
      BY <1>1, HonestInputs DEF HonestInputs, Retire
  <1>3. Leased \subseteq state.snapshot
      BY <1>1, <1>2 DEF SnapshotComplete
  <1>4. ~Live(t)
      BY <1>2, <1>3 DEF Leased, Live
  <1>5. (state'.retiredWhileLeased) =
        ((state.retiredWhileLeased) \/ Live(t))
      BY <1>1 DEF Retire, StateRecord
  <1>6. QED BY <1>1, <1>4, <1>5 DEF NoRetireLive

THEOREM RetirePreservesStale ==
    \A actor \in Clients, t \in Targets :
        (SnapshotComplete /\ StalePinned /\ Retire(actor, t)) =>
            StalePinned'
  <1> SUFFICES ASSUME NEW CONSTANT actor \in Clients,
                       NEW CONSTANT t \in Targets,
                       SnapshotComplete, StalePinned,
                       Retire(actor, t)
             PROVE StalePinned'
      OBVIOUS
  <1>1. state.stale \subseteq state.snapshot
      BY DEF SnapshotComplete, Retire
  <1>2. t \notin state.stale
      BY <1>1, HonestInputs DEF HonestInputs, Retire
  <1>3. QED BY SMTT(120), <1>2
      DEF StalePinned, Retire, StateRecord, Targets
=============================================================================
