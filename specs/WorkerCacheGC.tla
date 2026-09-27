---- MODULE WorkerCacheGC ----
EXTENDS Naturals, FiniteSets

\* One private cache, two independent client processes, two contexts/builds.
\* Start is an already-authorized, possibly racing installer; Publish is
\* its completed verified cache entry. Neither is a live lease. The OS
\* lock excludes Welcome's final-path check and lease publication from
\* a collector's lease snapshot through its last unlink. A crashed
\* worker's record stays pinned, rather than using a liveness guess.
\* Rust correspondence: serve/{cache_lease,cache_gc}.rs and both editor
\* admission callsites. Same-principal/cooperating processes, owner
\* checks and honest private records are environmental premises.
CONSTANTS Clients, Contexts, Digests, MUTATION
ASSUME /\ Clients # {} /\ Contexts # {} /\ Digests # {}
       /\ MUTATION \in 0..4

None == "none"
Targets == [context : Contexts, digest : Digests]
NoTarget == [context |-> None, digest |-> None]
Phases == {"idle", "exec", "checking", "ready", "locking",
            "observed", "failed"}

VARIABLE state
vars == <<state>>
Init == state = [
    phase |-> [c \in Clients |-> "idle"],
    wanted |-> [c \in Clients |-> NoTarget],
    owner |-> None,
    object |-> [t \in Targets |-> TRUE],
    receipt |-> [t \in Targets |-> TRUE],
    lease |-> [c \in Clients |-> NoTarget],
    stale |-> {},
    snapshot |-> {},
    retiredWhileLeased |-> FALSE,
    foreignRetirement |-> FALSE
]

StateRecord(phase, wanted, owner, object, receipt, lease, stale,
            snapshot, retiredWhileLeased, foreignRetirement) ==
    [ phase |-> phase, wanted |-> wanted, owner |-> owner,
      object |-> object, receipt |-> receipt, lease |-> lease,
      stale |-> stale, snapshot |-> snapshot,
      retiredWhileLeased |-> retiredWhileLeased,
      foreignRetirement |-> foreignRetirement ]

Target(c) == state.wanted[c]
Present(t) == state.object[t] /\ state.receipt[t]
Live(t) == \E c \in Clients : state.lease[c] = t
Leased == {t \in Targets : Live(t)}

\* Exec can begin while another worker collects, but cannot Welcome
\* until it acquires the cache lock and rechecks the final object.
Start(c, t) ==
    /\ c \in Clients /\ t \in Targets
    /\ state.phase[c] \in {"idle", "failed"} /\ Present(t)
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "exec"],
        [state.wanted EXCEPT ![c] = t], state.owner,
        state.object, state.receipt, state.lease, state.stale,
        state.snapshot, state.retiredWhileLeased, state.foreignRetirement)

LockWelcome(c) ==
    /\ state.phase[c] = "exec" /\ state.owner = None
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "checking"], state.wanted, c,
        state.object, state.receipt, state.lease, state.stale,
        state.snapshot, state.retiredWhileLeased, state.foreignRetirement)

\* Mutation 1 publishes a lease without first holding the shared lock;
\* mutation 4 skips the final-object/receipt check after an unlink.
Welcome(c) ==
    /\ \/ (state.phase[c] = "checking" /\ state.owner = c)
       \/ (MUTATION = 1 /\ state.phase[c] = "exec")
    /\ (Present(Target(c)) \/ MUTATION = 4)
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "ready"], state.wanted,
        IF state.owner = c THEN None ELSE state.owner,
        state.object, state.receipt,
        [state.lease EXCEPT ![c] = Target(c)], state.stale,
        state.snapshot, state.retiredWhileLeased, state.foreignRetirement)

Refuse(c) ==
    /\ state.phase[c] = "checking" /\ state.owner = c
    /\ ~Present(Target(c))
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "failed"], state.wanted, None,
        state.object, state.receipt, state.lease, state.stale,
        state.snapshot, state.retiredWhileLeased, state.foreignRetirement)

Acquire(c) ==
    /\ state.phase[c] = "ready" /\ state.owner = None
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "locking"], state.wanted, c,
        state.object, state.receipt, state.lease, state.stale,
        state.snapshot, state.retiredWhileLeased, state.foreignRetirement)

ObserveLeases(c) ==
    /\ state.phase[c] = "locking" /\ state.owner = c
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "observed"], state.wanted,
        state.owner, state.object, state.receipt, state.lease,
        state.stale, Leased \cup state.stale,
        state.retiredWhileLeased, state.foreignRetirement)

\* The actor's own executable always stays; foreign context receipts
\* never authorize cleanup. Mutations 2/3 violate these two premises.
Retire(c, t) ==
    /\ state.phase[c] = "observed" /\ state.owner = c
    /\ t \in Targets /\ Present(t) /\ t # Target(c)
    /\ (t \notin state.snapshot \/ MUTATION = 2)
    /\ (t.context = Target(c).context \/ MUTATION = 3)
    /\ state' = StateRecord(
        state.phase, state.wanted, state.owner,
        [state.object EXCEPT ![t] = FALSE],
        [state.receipt EXCEPT ![t] = FALSE], state.lease,
        state.stale, state.snapshot,
        state.retiredWhileLeased \/ Live(t),
        state.foreignRetirement \/ t.context # Target(c).context)

Release(c) ==
    /\ state.phase[c] = "observed" /\ state.owner = c
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "ready"], state.wanted, None,
        state.object, state.receipt, state.lease, state.stale, {},
        state.retiredWhileLeased, state.foreignRetirement)

Stop(c) ==
    /\ state.phase[c] = "ready"
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "idle"], state.wanted,
        state.owner, state.object, state.receipt,
        [state.lease EXCEPT ![c] = NoTarget], state.stale,
        state.snapshot, state.retiredWhileLeased, state.foreignRetirement)

Crash(c) ==
    /\ state.phase[c] = "ready"
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "failed"], state.wanted,
        state.owner, state.object, state.receipt,
        [state.lease EXCEPT ![c] = NoTarget],
        state.stale \cup {Target(c)}, state.snapshot,
        state.retiredWhileLeased, state.foreignRetirement)

\* A completed new install may race collection. It is not allowed to
\* claim readiness until a separate locked Welcome validates its path.
Publish(t) ==
    /\ t \in Targets /\ ~state.object[t]
    /\ state' = StateRecord(
        state.phase, state.wanted, state.owner,
        [state.object EXCEPT ![t] = TRUE],
        [state.receipt EXCEPT ![t] = TRUE], state.lease,
        state.stale, state.snapshot,
        state.retiredWhileLeased, state.foreignRetirement)

Next == \/ \E c \in Clients, t \in Targets : Start(c, t)
        \/ \E c \in Clients : LockWelcome(c) \/ Welcome(c) \/ Refuse(c)
        \/ \E c \in Clients : Acquire(c) \/ ObserveLeases(c)
                              \/ Release(c) \/ Stop(c) \/ Crash(c)
        \/ \E c \in Clients, t \in Targets : Retire(c, t)
        \/ \E t \in Targets : Publish(t)
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ state.phase \in [Clients -> Phases]
    /\ state.wanted \in [Clients -> Targets \cup {NoTarget}]
    /\ state.owner \in Clients \cup {None}
    /\ state.object \in [Targets -> BOOLEAN]
    /\ state.receipt \in [Targets -> BOOLEAN]
    /\ state.lease \in [Clients -> Targets \cup {NoTarget}]
    /\ state.stale \in SUBSET Targets
    /\ state.snapshot \in SUBSET Targets
    /\ state.retiredWhileLeased \in BOOLEAN
    /\ state.foreignRetirement \in BOOLEAN

\* Once observed under the exclusive lock, departing clients can only
\* shrink the live set; crashes add records already present in it.
SnapshotComplete == \A c \in Clients :
    state.phase[c] = "observed" /\ state.owner = c =>
        Leased \cup state.stale \subseteq state.snapshot

LiveObject == \A c \in Clients :
    state.lease[c] # NoTarget => Present(state.lease[c])
ScopedRetirement == ~state.foreignRetirement
NoRetireLive == ~state.retiredWhileLeased
StalePinned == state.stale \subseteq {t \in Targets : state.object[t]}

\* Independent reachability checks prevent a vacuous serial model.
WitnessConcurrentReady == ~(\E a, b \in Clients :
    a # b /\ state.lease[a] # NoTarget /\ state.lease[b] # NoTarget)
WitnessBlockedWelcome == ~(\E a, b \in Clients :
    a # b /\ state.phase[a] = "observed" /\ state.phase[b] = "exec")
WitnessRetired == ~(\E t \in Targets : ~state.object[t])
WitnessCrashPinned == ~(\E t \in Targets : t \in state.stale
    /\ state.object[t] /\ \E c \in Clients : state.phase[c] = "observed")
WitnessDifferentBuilds == ~(\E a, b \in Clients :
    a # b /\ state.lease[a] # NoTarget /\ state.lease[b] # NoTarget
    /\ state.lease[a].digest # state.lease[b].digest)
=============================================================================
