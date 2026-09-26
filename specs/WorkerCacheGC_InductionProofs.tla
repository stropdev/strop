---- MODULE WorkerCacheGC_InductionProofs ----
EXTENDS WorkerCacheGC, TLAPS

\* The sentinels are not real client IDs, endpoint context strings or
\* content digests in the Rust cache. They must be separate abstract
\* sorts; otherwise a client literally named "none" can own a lock
\* while the model also reads that value as an unlocked cache.
ASSUME HonestInputs == MUTATION = 0 /\ Clients # {}
    /\ Contexts # {} /\ Digests # {}
    /\ None \notin Clients /\ None \notin Contexts /\ None \notin Digests

ActivePhases == {"ready", "locking", "observed"}
RunningPhases == ActivePhases \cup {"exec", "checking"}
ActiveTargetValid == \A c \in Clients :
    state.phase[c] \in RunningPhases => Target(c) \in Targets
ConnectedLeases == \A c \in Clients :
    state.lease[c] # NoTarget => state.phase[c] \in ActivePhases
HasActiveLease == \A c \in Clients :
    state.phase[c] \in ActivePhases =>
        state.lease[c] = Target(c) /\ state.lease[c] # NoTarget
OwnerWellFormed == \A c \in Clients :
    state.phase[c] \in {"checking", "locking", "observed"} =>
        state.owner = c

Inv == /\ TypeOK
       /\ ActiveTargetValid
       /\ ConnectedLeases
       /\ HasActiveLease
       /\ OwnerWellFormed
       /\ SnapshotComplete
       /\ LiveObject
       /\ ScopedRetirement
       /\ NoRetireLive
       /\ StalePinned

THEOREM InitImpliesInv == Init => Inv
    BY SMTT(120) DEF Init, Inv, TypeOK, ActivePhases,
       RunningPhases, ActiveTargetValid, ConnectedLeases,
       HasActiveLease, OwnerWellFormed, SnapshotComplete,
       LiveObject, ScopedRetirement, NoRetireLive, StalePinned,
       Leased, Live, Present, Targets, NoTarget, Phases, None, Target

THEOREM InductiveStep == Inv /\ [Next]_vars => Inv'
  <1> SUFFICES ASSUME Inv, [Next]_vars PROVE Inv'
      OBVIOUS
  <1>1. CASE \E c \in Clients, t \in Targets : Start(c, t)
      <2>1. PICK c \in Clients, t \in Targets : Start(c, t) BY <1>1
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Start, StateRecord, Phases
      <2>3. state'.wanted \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1 DEF Inv, TypeOK, Start, StateRecord, Targets, NoTarget
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, Start, StateRecord
      <2>5. ActiveTargetValid'
          <3>1. SUFFICES \A d \in Clients :
              state'.phase[d] \in RunningPhases => state'.wanted[d] \in Targets
              BY DEF ActiveTargetValid, Target
          <3>2. TAKE d \in Clients
          <3>3. CASE d = c
              <4>1. state'.wanted = [state.wanted EXCEPT ![c] = t]
                  BY <2>1 DEF Start, StateRecord
              <4>2. state'.wanted[d] = t
                  BY IsaT(120), <2>1, <3>2, <3>3, <4>1
                     DEF Inv, TypeOK
              <4>3. QED
                  BY <2>1, <3>2, <3>3, <4>2
                     DEF Start, StateRecord, RunningPhases, ActivePhases
          <3>4. CASE d # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, ActiveTargetValid, Start, StateRecord, Target
          <3>5. QED BY <3>3, <3>4
      <2>6. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, ConnectedLeases, ActivePhases,
                 Start, StateRecord, NoTarget
      <2>7. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, HasActiveLease, ActivePhases,
                 Start, StateRecord, Target
      <2>8. OwnerWellFormed'
          BY SMTT(120), <2>1
             DEF Inv, OwnerWellFormed, Start, StateRecord
      <2>9. SnapshotComplete'
          BY SMTT(120), <2>1
             DEF Inv, SnapshotComplete, Leased, Live,
                 Start, StateRecord
      <2>10. QED BY <2>1, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9
          DEF Inv, LiveObject, ScopedRetirement, NoRetireLive,
              StalePinned, Start, StateRecord, Present
  <1>2. CASE \E c \in Clients : LockWelcome(c)
      <2>1. PICK c \in Clients : LockWelcome(c) BY <1>2
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, LockWelcome, StateRecord, Phases
      <2>3. state'.owner \in Clients \cup {None}
          BY <2>1 DEF Inv, TypeOK, LockWelcome, StateRecord, None
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, LockWelcome, StateRecord
      <2>5. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, ActiveTargetValid, RunningPhases,
                 ActivePhases, LockWelcome, StateRecord, Target
      <2>6. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, ConnectedLeases, ActivePhases,
                 LockWelcome, StateRecord, NoTarget
      <2>7. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, HasActiveLease, ActivePhases,
                 LockWelcome, StateRecord, Target
      <2>8. OwnerWellFormed'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, OwnerWellFormed, LockWelcome,
                 StateRecord, HonestInputs
      <2>9. SnapshotComplete'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, SnapshotComplete, Leased, Live,
                 OwnerWellFormed, LockWelcome,
                 StateRecord, HonestInputs
      <2>10. QED BY <2>1, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9
          DEF Inv, LiveObject, ScopedRetirement, NoRetireLive,
              StalePinned, LockWelcome, StateRecord, Present
  <1>3. CASE \E c \in Clients : Welcome(c)
      <2>1. PICK c \in Clients : Welcome(c) BY <1>3
      <2>2. state.phase[c] = "checking" /\ state.owner = c
             /\ Present(Target(c))
          BY <2>1, HonestInputs DEF Welcome, HonestInputs
      <2>3. Target(c) \in Targets
          BY <2>2 DEF Inv, ActiveTargetValid,
             RunningPhases, ActivePhases
      <2>4. NoTarget \notin Targets
          BY HonestInputs DEF HonestInputs, Targets, NoTarget, None
      <2>5. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Welcome, StateRecord, Phases
      <2>6. state'.owner \in Clients \cup {None}
          BY <2>1, <2>2 DEF Inv, TypeOK,
             Welcome, StateRecord, None
      <2>7. state'.lease \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1, <2>3 DEF Inv, TypeOK,
             Welcome, StateRecord, NoTarget
      <2>8. TypeOK'
          BY <2>1, <2>5, <2>6, <2>7
             DEF Inv, TypeOK, Welcome, StateRecord
      <2>9. ActiveTargetValid'
          BY SMTT(120), <2>1, <2>3
             DEF Inv, ActiveTargetValid, RunningPhases,
                 ActivePhases, Welcome, StateRecord, Target
      <2>10. ConnectedLeases'
          BY SMTT(120), <2>1, <2>2
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 Welcome, StateRecord, NoTarget
      <2>11. HasActiveLease'
          BY SMTT(120), <2>1, <2>2, <2>3, <2>4
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 Welcome, StateRecord, Target, NoTarget, Targets
      <2>12. OwnerWellFormed'
          BY SMTT(120), <2>1, <2>2, HonestInputs
             DEF Inv, TypeOK, OwnerWellFormed, Welcome,
                 StateRecord, HonestInputs
      <2>13. SnapshotComplete'
          BY SMTT(120), <2>1, <2>2, HonestInputs
             DEF Inv, SnapshotComplete, Leased, Live,
                 OwnerWellFormed, Welcome,
                 StateRecord, HonestInputs
      <2>14. LiveObject'
          BY SMTT(120), <2>1, <2>2, <2>3
             DEF Inv, LiveObject, Welcome,
                 StateRecord, Present, Target, NoTarget
      <2>15. QED
          BY <2>1, <2>8, <2>9, <2>10, <2>11,
             <2>12, <2>13, <2>14
             DEF Inv, ScopedRetirement, NoRetireLive,
                 StalePinned, Welcome, StateRecord, Targets
  <1>4. CASE \E c \in Clients : Refuse(c)
      <2>1. PICK c \in Clients : Refuse(c) BY <1>4
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Refuse, StateRecord, Phases
      <2>3. TypeOK'
          BY <2>1, <2>2 DEF Inv, TypeOK, Refuse, StateRecord, None
      <2>4. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ActiveTargetValid,
                 RunningPhases, ActivePhases, Refuse, StateRecord, Target
      <2>5. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 Refuse, StateRecord, NoTarget
      <2>6. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 Refuse, StateRecord, Target
      <2>7. OwnerWellFormed'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, TypeOK, OwnerWellFormed, Refuse,
                 StateRecord, HonestInputs
      <2>8. SnapshotComplete'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, SnapshotComplete, Leased, Live,
                 OwnerWellFormed, Refuse, StateRecord, HonestInputs
      <2>9. QED
          BY <2>1, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8
             DEF Inv, LiveObject, ScopedRetirement, NoRetireLive,
                 StalePinned, Refuse, StateRecord, Present
  <1>5. CASE \E c \in Clients : Acquire(c)
      <2>1. PICK c \in Clients : Acquire(c) BY <1>5
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Acquire, StateRecord, Phases
      <2>3. state'.owner \in Clients \cup {None}
          BY <2>1 DEF Inv, TypeOK, Acquire, StateRecord, None
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, Acquire, StateRecord
      <2>5. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ActiveTargetValid, RunningPhases,
                 ActivePhases, Acquire, StateRecord, Target
      <2>6. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 Acquire, StateRecord, NoTarget
      <2>7. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 Acquire, StateRecord, Target
      <2>8. OwnerWellFormed'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, TypeOK, OwnerWellFormed, Acquire,
                 StateRecord, HonestInputs
      <2>9. SnapshotComplete'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, SnapshotComplete, Leased, Live,
                 OwnerWellFormed, Acquire, StateRecord, HonestInputs
      <2>10. QED BY <2>1, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9
          DEF Inv, LiveObject, ScopedRetirement, NoRetireLive,
              StalePinned, Acquire, StateRecord, Present
  <1>6. CASE \E c \in Clients : ObserveLeases(c)
      <2>1. PICK c \in Clients : ObserveLeases(c) BY <1>6
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, ObserveLeases, StateRecord, Phases
      <2>3. state'.snapshot \in SUBSET Targets
          BY <2>1 DEF Inv, TypeOK, ObserveLeases,
             StateRecord, Leased, Live
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, ObserveLeases, StateRecord
      <2>5. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ActiveTargetValid, RunningPhases,
                 ActivePhases, ObserveLeases, StateRecord, Target
      <2>6. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 ObserveLeases, StateRecord, NoTarget
      <2>7. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 ObserveLeases, StateRecord, Target
      <2>8. OwnerWellFormed'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, TypeOK, OwnerWellFormed, ObserveLeases,
                 StateRecord, HonestInputs
      <2>9. SnapshotComplete'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, TypeOK, SnapshotComplete, Leased, Live,
                 OwnerWellFormed, ObserveLeases,
                 StateRecord, HonestInputs
      <2>10. QED BY <2>1, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9
          DEF Inv, LiveObject, ScopedRetirement, NoRetireLive,
              StalePinned, ObserveLeases, StateRecord, Present
  <1>7. CASE \E c \in Clients, t \in Targets : Retire(c, t)
      <2>1. PICK c \in Clients, t \in Targets : Retire(c, t) BY <1>7
      <2>2. t \notin state.snapshot
             /\ t.context = Target(c).context
          BY <2>1, HonestInputs DEF Retire, HonestInputs
      <2>3. ~Live(t)
          BY <2>1, <2>2
             DEF Inv, SnapshotComplete, Leased, Live, Retire
      <2>4. t \notin state.stale
          BY <2>1, <2>2 DEF Inv, SnapshotComplete, Retire
      <2>5. state'.object \in [Targets -> BOOLEAN]
             /\ state'.receipt \in [Targets -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Retire, StateRecord
      <2>6. TypeOK'
          BY <2>1, <2>5
             DEF Inv, TypeOK, Retire, StateRecord
      <2>7. LiveObject'
          BY SMTT(120), <2>1, <2>3
             DEF Inv, TypeOK, LiveObject, Retire,
                 StateRecord, Present, Live, NoTarget
      <2>8. ScopedRetirement'
          BY <2>1, <2>2
             DEF Inv, ScopedRetirement, Retire, StateRecord
      <2>9. NoRetireLive'
          BY <2>1, <2>3
             DEF Inv, NoRetireLive, Retire, StateRecord
      <2>10. StalePinned'
          BY SMTT(120), <2>1, <2>4
             DEF Inv, TypeOK, StalePinned, Retire,
                 StateRecord, Targets
      <2>11. QED
          BY <2>1, <2>6, <2>7, <2>8, <2>9, <2>10
             DEF Inv, ActiveTargetValid, ConnectedLeases,
                 HasActiveLease, OwnerWellFormed, SnapshotComplete,
                 Retire, StateRecord, Leased, Live, Target,
                 ActivePhases, RunningPhases
  <1>8. CASE \E c \in Clients : Release(c)
      <2>1. PICK c \in Clients : Release(c) BY <1>8
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Release, StateRecord, Phases
      <2>3. TypeOK'
          BY <2>1, <2>2
             DEF Inv, TypeOK, Release, StateRecord, None
      <2>4. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ActiveTargetValid, RunningPhases,
                 ActivePhases, Release, StateRecord, Target
      <2>5. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 Release, StateRecord, NoTarget
      <2>6. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 Release, StateRecord, Target
      <2>7. OwnerWellFormed'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, TypeOK, OwnerWellFormed, Release,
                 StateRecord, HonestInputs
      <2>8. SnapshotComplete'
          BY SMTT(120), <2>1, HonestInputs
             DEF Inv, TypeOK, SnapshotComplete, Leased, Live,
                 OwnerWellFormed, Release, StateRecord, HonestInputs
      <2>9. QED BY <2>1, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8
          DEF Inv, LiveObject, ScopedRetirement, NoRetireLive,
              StalePinned, Release, StateRecord, Present
  <1>9. CASE \E c \in Clients : Stop(c)
      <2>1. PICK c \in Clients : Stop(c) BY <1>9
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Stop, StateRecord, Phases
      <2>3. state'.lease \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1 DEF Inv, TypeOK, Stop, StateRecord, NoTarget
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, Stop, StateRecord
      <2>5. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ActiveTargetValid, RunningPhases,
                 ActivePhases, Stop, StateRecord, Target
      <2>6. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 Stop, StateRecord, NoTarget
      <2>7. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 Stop, StateRecord, Target
      <2>8. OwnerWellFormed'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, OwnerWellFormed,
                 Stop, StateRecord
      <2>9. SnapshotComplete'
          <3>1. Leased' \subseteq Leased
              BY SMTT(120), <2>1, HonestInputs
                 DEF Inv, TypeOK, Stop, StateRecord,
                     Leased, Live, NoTarget, Targets, HonestInputs
          <3>2. QED
              BY SMTT(120), <2>1, <3>1
                 DEF Inv, TypeOK, SnapshotComplete, Leased, Live,
                     Stop, StateRecord
      <2>10. LiveObject'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, LiveObject, Stop,
                 StateRecord, Present, NoTarget
      <2>11. QED BY <2>1, <2>4, <2>5, <2>6, <2>7,
             <2>8, <2>9, <2>10
          DEF Inv, ScopedRetirement, NoRetireLive,
              StalePinned, Stop, StateRecord
  <1>10. CASE \E c \in Clients : Crash(c)
      <2>1. PICK c \in Clients : Crash(c) BY <1>10
      <2>2. Target(c) \in Targets
             /\ state.lease[c] = Target(c)
             /\ state.lease[c] # NoTarget
          BY <2>1
             DEF Inv, TypeOK, ActiveTargetValid, HasActiveLease,
                 RunningPhases, ActivePhases, Crash, Target
      <2>3. state.object[Target(c)] /\ state.receipt[Target(c)]
          BY <2>2 DEF Inv, LiveObject, Present
      <2>4. state'.phase \in [Clients -> Phases]
             /\ state'.lease \in [Clients -> Targets \cup {NoTarget}]
             /\ state'.stale \in SUBSET Targets
          BY <2>1, <2>2
             DEF Inv, TypeOK, Crash, StateRecord, Phases, NoTarget
      <2>5. TypeOK'
          BY <2>1, <2>4
             DEF Inv, TypeOK, Crash, StateRecord
      <2>6. ActiveTargetValid'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ActiveTargetValid, RunningPhases,
                 ActivePhases, Crash, StateRecord, Target
      <2>7. ConnectedLeases'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, ConnectedLeases, ActivePhases,
                 Crash, StateRecord, NoTarget
      <2>8. HasActiveLease'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, HasActiveLease, ActivePhases,
                 Crash, StateRecord, Target
      <2>9. OwnerWellFormed'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, OwnerWellFormed,
                 Crash, StateRecord
      <2>10. SnapshotComplete'
          BY SMTT(120), <2>1, <2>2, HonestInputs
             DEF Inv, TypeOK, SnapshotComplete, Leased, Live,
                 Crash, StateRecord, Target, NoTarget,
                 Targets, HonestInputs
      <2>11. LiveObject'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, LiveObject, Crash,
                 StateRecord, Present, NoTarget
      <2>12. StalePinned'
          BY SMTT(120), <2>1, <2>2, <2>3
             DEF Inv, TypeOK, StalePinned, Crash,
                 StateRecord, Target, Targets
      <2>13. QED BY <2>1, <2>5, <2>6, <2>7, <2>8,
             <2>9, <2>10, <2>11, <2>12
          DEF Inv, ScopedRetirement, NoRetireLive,
              Crash, StateRecord
  <1>11. CASE \E t \in Targets : Publish(t)
      <2>1. PICK t \in Targets : Publish(t) BY <1>11
      <2>2. state'.object \in [Targets -> BOOLEAN]
             /\ state'.receipt \in [Targets -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Publish, StateRecord
      <2>3. TypeOK'
          BY <2>1, <2>2
             DEF Inv, TypeOK, Publish, StateRecord
      <2>4. LiveObject'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, LiveObject, Publish,
                 StateRecord, Present
      <2>5. StalePinned'
          BY SMTT(120), <2>1
             DEF Inv, TypeOK, StalePinned, Publish,
                 StateRecord, Targets
      <2>6. QED BY <2>1, <2>3, <2>4, <2>5
          DEF Inv, ActiveTargetValid, ConnectedLeases,
              HasActiveLease, OwnerWellFormed, SnapshotComplete,
              ScopedRetirement, NoRetireLive, Publish, StateRecord,
              Leased, Live, Target, ActivePhases, RunningPhases
  <1>12. CASE UNCHANGED vars
      <2>1. state' = state BY <1>12 DEF vars
      <2>2. QED BY <2>1
          DEF Inv, TypeOK, ActiveTargetValid, ConnectedLeases,
              HasActiveLease, OwnerWellFormed, SnapshotComplete,
              LiveObject, ScopedRetirement, NoRetireLive, StalePinned,
              Target, Leased, Live, Present
  <1>13. QED
      BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6,
         <1>7, <1>8, <1>9, <1>10, <1>11, <1>12
         DEF Next, vars

THEOREM InvImpliesSafety == Inv =>
    SnapshotComplete /\ LiveObject /\ ScopedRetirement
    /\ NoRetireLive /\ StalePinned
    BY DEF Inv
=============================================================================
