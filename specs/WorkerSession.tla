---- MODULE WorkerSession ----
EXTENDS Naturals, FiniteSets, TLC

\* 0058 WK17: the client/worker ownership boundary, not a second filesystem
\* executor. One admitted stream is keyed by (client, worker, incarnation);
\* credits account for queued and delivered-but-unconsumed chunks. Store
\* attempt evidence belongs to a captured namespace and remains dirty
\* through a lost reply. The native Store effect itself is RemoteSave.tla.
\*
\* Rust correspondence: worker-protocol/guard.rs::Authority::admit;
\* worker/serve/stream_window.rs::{take,grant,abandon};
\* worker/serve/schedule.rs::{push_chunk,flush_data};
\* worker-client/connection.rs::route_chunk and payload.rs::read;
\* engine/editor/remote/save.rs::{remote_write_done,accept_remote_receipt};
\* worker/serve/fs.rs::verify_recovered. OS/transport scheduling and digest
\* collision resistance are premises; no progress across a dead peer.

CONSTANTS Clients, Workers, Streams, MAXGEN, WINDOW, QCAP, MUTATION, MODE
ASSUME /\ Clients # {} /\ Workers # {} /\ Streams # {}
       /\ MAXGEN \in Nat /\ MAXGEN > 0
       /\ WINDOW \in Nat /\ WINDOW > 0
       /\ QCAP \in Nat /\ QCAP > 0
       /\ MUTATION \in 0..7
       /\ MODE \in 0..2

None == "none"
Stages == {"idle", "prepared", "committed"}
Owners == [client : Clients \cup {None}, worker : Workers \cup {None}, gen : 0..MAXGEN]
Attempts == [stage : Stages, worker : Workers \cup {None}, gen : 0..MAXGEN,
             namespace : Workers \cup {None}, good : BOOLEAN, dirty : BOOLEAN,
             uncertain : BOOLEAN, ack : BOOLEAN, recovered : BOOLEAN,
             verifiedNamespace : BOOLEAN, appliedFresh : BOOLEAN]

VARIABLE state
vars == <<state>>

Init == state = [
    generation |-> [w \in Workers |-> 0],
    namespace |-> [w \in Workers |-> w],
    bound |-> [c \in Clients |-> None],
    stamp |-> [c \in Clients |-> 0],
    owner |-> [s \in Streams |-> [client |-> None, worker |-> None, gen |-> 0]],
    active |-> [s \in Streams |-> FALSE],
    credit |-> [s \in Streams |-> 0],
    outbound |-> [s \in Streams |-> 0],
    inbound |-> [s \in Streams |-> 0],
    abandoned |-> [s \in Streams |-> FALSE],
    stoppedBy |-> [s \in Streams |-> None],
    stopFresh |-> [s \in Streams |-> TRUE],
    childExited |-> [s \in Streams |-> FALSE],
    drained |-> [s \in Streams |-> FALSE],
    lastDelivered |-> [s \in Streams |-> FALSE],
    exitPublished |-> [s \in Streams |-> FALSE],
    attempt |-> [c \in Clients |-> [stage |-> "idle", worker |-> None,
        gen |-> 0, namespace |-> None, good |-> TRUE, dirty |-> TRUE,
        uncertain |-> FALSE, ack |-> FALSE, recovered |-> FALSE,
        verifiedNamespace |-> TRUE, appliedFresh |-> FALSE]]
]

Bound(c) == state.bound[c] # None /\ state.stamp[c] = state.generation[state.bound[c]]
Owns(c, s) == /\ state.active[s]
              /\ state.owner[s].client = c
              /\ state.owner[s].worker = state.bound[c]
              /\ state.owner[s].gen = state.stamp[c]
              /\ Bound(c)
FreshAttempt(c) == Bound(c)
    /\ state.bound[c] = state.attempt[c].worker
    /\ state.stamp[c] = state.attempt[c].gen


\* Explicit record construction keeps the TLC state shape but gives TLAPS
\* one-level function updates; nested state EXCEPT is unsupported by TLAPS 1.5.
StateRecord(newGeneration, newNamespace, newBound, newStamp, newOwner, newActive, newCredit, newOutbound, newInbound, newAbandoned, newStoppedBy, newStopFresh, newChildExited, newDrained, newLastDelivered, newExitPublished, newAttempt) ==
    [ generation |-> newGeneration,
      namespace |-> newNamespace,
      bound |-> newBound,
      stamp |-> newStamp,
      owner |-> newOwner,
      active |-> newActive,
      credit |-> newCredit,
      outbound |-> newOutbound,
      inbound |-> newInbound,
      abandoned |-> newAbandoned,
      stoppedBy |-> newStoppedBy,
      stopFresh |-> newStopFresh,
      childExited |-> newChildExited,
      drained |-> newDrained,
      lastDelivered |-> newLastDelivered,
      exitPublished |-> newExitPublished,
      attempt |-> newAttempt ]

Handshake(c, w) ==
    state' = StateRecord(
        state.generation,
        state.namespace,
        [state.bound EXCEPT ![c] = w],
        [state.stamp EXCEPT ![c] = state.generation[w]],
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
Restart(w, n) ==
    /\ state.generation[w] < MAXGEN
    /\ n \in Workers
    /\ state' = StateRecord(
        [state.generation EXCEPT ![w] = state.generation[w] + 1],
        [state.namespace EXCEPT ![w] = n],
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
Open(c, s) ==
    /\ Bound(c) /\ ~state.active[s]
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        [state.owner EXCEPT ![s] = [client |-> c, worker |-> state.bound[c], gen |-> state.stamp[c]]],
        [state.active EXCEPT ![s] = TRUE],
        [state.credit EXCEPT ![s] = WINDOW],
        [state.outbound EXCEPT ![s] = 0],
        [state.inbound EXCEPT ![s] = 0],
        [state.abandoned EXCEPT ![s] = FALSE],
        [state.stoppedBy EXCEPT ![s] = None],
        [state.stopFresh EXCEPT ![s] = TRUE],
        [state.childExited EXCEPT ![s] = FALSE],
        [state.drained EXCEPT ![s] = FALSE],
        [state.lastDelivered EXCEPT ![s] = FALSE],
        [state.exitPublished EXCEPT ![s] = FALSE],
        state.attempt
        )
Produce(s) ==
    /\ state.active[s] /\ ~state.abandoned[s] /\ ~state.drained[s]
    /\ state.owner[s].gen = state.generation[state.owner[s].worker]
    /\ state.credit[s] > 0 /\ state.outbound[s] < QCAP
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        [state.credit EXCEPT ![s] = state.credit[s] - 1],
        [state.outbound EXCEPT ![s] = state.outbound[s] + 1],
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
Transmit(s) ==
    /\ state.active[s] /\ state.outbound[s] > 0
    /\ state.owner[s].gen = state.generation[state.owner[s].worker]
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        [state.outbound EXCEPT ![s] = state.outbound[s] - 1],
        [state.inbound EXCEPT ![s] = state.inbound[s] + 1],
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
Consume(s) ==
    /\ state.active[s] /\ state.inbound[s] > 0
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        [state.credit EXCEPT ![s] = state.credit[s] + 1],
        state.outbound,
        [state.inbound EXCEPT ![s] = state.inbound[s] - 1],
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
Abandon(c, s) ==
    /\ Owns(c, s)
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        [state.abandoned EXCEPT ![s] = TRUE],
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
Stop(c, s) ==
    /\ state.active[s] /\ state.stoppedBy[s] = None
    /\ (Owns(c, s) \/ MUTATION = 5)
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        [state.stoppedBy EXCEPT ![s] = c],
        [state.stopFresh EXCEPT ![s] = Owns(c, s)],
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
RecordExit(s) ==
    /\ state.active[s] /\ ~state.childExited[s]
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        [state.childExited EXCEPT ![s] = TRUE],
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
FinishPump(s) ==
    /\ state.childExited[s] /\ ~state.drained[s]
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        [state.drained EXCEPT ![s] = TRUE],
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )
EndOutput(s) ==
    /\ state.drained[s] /\ state.outbound[s] = 0
    /\ ~state.lastDelivered[s]
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        [state.lastDelivered EXCEPT ![s] = TRUE],
        state.exitPublished,
        state.attempt
        )
PublishExit(s) ==
    /\ state.childExited[s] /\ ~state.exitPublished[s]
    /\ (state.lastDelivered[s] \/ MUTATION = 4)
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        [state.exitPublished EXCEPT ![s] = TRUE],
        state.attempt
        )
Submit(c) ==
    /\ state.attempt[c].stage = "idle"
    /\ (Bound(c) \/ MUTATION = 1)
    /\ state.bound[c] # None
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        [state.attempt EXCEPT ![c] = [ stage |-> "prepared",
              worker |-> state.bound[c],
              gen |-> state.stamp[c],
              namespace |-> state.namespace[state.bound[c]],
              good |-> Bound(c),
              dirty |-> TRUE,
              uncertain |-> FALSE,
              ack |-> FALSE,
              recovered |-> FALSE,
              verifiedNamespace |-> TRUE,
              appliedFresh |-> FALSE ]]
        )
Commit(c) ==
    /\ state.attempt[c].stage = "prepared"
    /\ state.attempt[c].good
    /\ (FreshAttempt(c) \/ MUTATION = 7)
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        [state.attempt EXCEPT ![c] = [ stage |-> "committed",
              worker |-> state.attempt[c].worker,
              gen |-> state.attempt[c].gen,
              namespace |-> state.attempt[c].namespace,
              good |-> state.attempt[c].good,
              dirty |-> state.attempt[c].dirty,
              uncertain |-> state.attempt[c].uncertain,
              ack |-> state.attempt[c].ack,
              recovered |-> state.attempt[c].recovered,
              verifiedNamespace |-> state.attempt[c].verifiedNamespace,
              appliedFresh |-> FreshAttempt(c) ]]
        )
LoseReply(c) ==
    /\ state.attempt[c].stage # "idle" /\ ~state.attempt[c].ack
    /\ ~state.attempt[c].uncertain
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        [state.attempt EXCEPT ![c] = [ stage |-> state.attempt[c].stage,
              worker |-> state.attempt[c].worker,
              gen |-> state.attempt[c].gen,
              namespace |-> state.attempt[c].namespace,
              good |-> state.attempt[c].good,
              dirty |-> IF MUTATION = 2 THEN FALSE ELSE TRUE,
              uncertain |-> TRUE,
              ack |-> state.attempt[c].ack,
              recovered |-> state.attempt[c].recovered,
              verifiedNamespace |-> state.attempt[c].verifiedNamespace,
              appliedFresh |-> state.attempt[c].appliedFresh ]]
        )
Receipt(c) ==
    /\ state.attempt[c].stage = "committed" /\ ~state.attempt[c].uncertain
    /\ state.bound[c] = state.attempt[c].worker
    /\ state.stamp[c] = state.attempt[c].gen
    /\ state.generation[state.bound[c]] = state.stamp[c]
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        [state.attempt EXCEPT ![c] = [ stage |-> state.attempt[c].stage,
              worker |-> state.attempt[c].worker,
              gen |-> state.attempt[c].gen,
              namespace |-> state.attempt[c].namespace,
              good |-> state.attempt[c].good,
              dirty |-> FALSE,
              uncertain |-> state.attempt[c].uncertain,
              ack |-> TRUE,
              recovered |-> state.attempt[c].recovered,
              verifiedNamespace |-> state.attempt[c].verifiedNamespace,
              appliedFresh |-> state.attempt[c].appliedFresh ]]
        )
Verify(c) ==
    /\ state.attempt[c].uncertain /\ ~state.attempt[c].ack
    /\ state.bound[c] = state.attempt[c].worker
    /\ Bound(c)
    /\ (state.namespace[state.bound[c]] = state.attempt[c].namespace
         \/ MUTATION = 6)
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        [state.attempt EXCEPT ![c] = [ stage |-> state.attempt[c].stage,
              worker |-> state.attempt[c].worker,
              gen |-> state.attempt[c].gen,
              namespace |-> state.attempt[c].namespace,
              good |-> state.attempt[c].good,
              dirty |-> (state.attempt[c].stage # "committed"),
              uncertain |-> FALSE,
              ack |-> (state.attempt[c].stage = "committed"),
              recovered |-> (state.attempt[c].stage = "committed"),
              verifiedNamespace |-> (state.namespace[state.bound[c]] = state.attempt[c].namespace),
              appliedFresh |-> state.attempt[c].appliedFresh ]]
        )
RetireAttempt(c) ==
    /\ state.attempt[c].stage # "idle"
    /\ (state.attempt[c].ack \/
        (state.attempt[c].stage = "prepared" /\ ~state.attempt[c].uncertain))
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        state.credit,
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        [state.attempt EXCEPT ![c] = [ stage |-> "idle",
              worker |-> state.attempt[c].worker,
              gen |-> state.attempt[c].gen,
              namespace |-> state.attempt[c].namespace,
              good |-> state.attempt[c].good,
              dirty |-> state.attempt[c].dirty,
              uncertain |-> state.attempt[c].uncertain,
              ack |-> state.attempt[c].ack,
              recovered |-> state.attempt[c].recovered,
              verifiedNamespace |-> state.attempt[c].verifiedNamespace,
              appliedFresh |-> state.attempt[c].appliedFresh ]]
        )
CreditWithoutConsume(s) ==
    /\ MUTATION = 3 /\ state.active[s] /\ state.credit[s] < WINDOW
    /\ state' = StateRecord(
        state.generation,
        state.namespace,
        state.bound,
        state.stamp,
        state.owner,
        state.active,
        [state.credit EXCEPT ![s] = state.credit[s] + 1],
        state.outbound,
        state.inbound,
        state.abandoned,
        state.stoppedBy,
        state.stopFresh,
        state.childExited,
        state.drained,
        state.lastDelivered,
        state.exitPublished,
        state.attempt
        )\* MODE=0 checks the joint service/save product in a single-client
\* connection; MODE=1 and MODE=2 exhaust two-client stream and Store
\* ownership respectively. Every public action runs in a clean
\* campaign, with the common handshake/restart actions in all three.
SessionActions == \/ \E c \in Clients, w \in Workers : Handshake(c, w)
                  \/ \E w \in Workers, n \in Workers : Restart(w, n)
StreamActions == \/ \E c \in Clients, s \in Streams :
                      Open(c, s) \/ Abandon(c, s) \/ Stop(c, s)
                 \/ \E s \in Streams :
                      Produce(s) \/ Transmit(s) \/ Consume(s)
                      \/ RecordExit(s) \/ FinishPump(s)
                      \/ EndOutput(s) \/ PublishExit(s)
                      \/ CreditWithoutConsume(s)
StoreActions == \E c \in Clients :
    Submit(c) \/ Commit(c) \/ LoseReply(c)
    \/ Receipt(c) \/ Verify(c) \/ RetireAttempt(c)
Next == SessionActions
        \/ (MODE \in {0, 1} /\ StreamActions)
        \/ (MODE \in {0, 2} /\ StoreActions)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ state.generation \in [Workers -> 0..MAXGEN]
    /\ state.namespace \in [Workers -> Workers]
    /\ state.bound \in [Clients -> Workers \cup {None}]
    /\ state.stamp \in [Clients -> 0..MAXGEN]
    /\ state.owner \in [Streams -> Owners]
    /\ state.active \in [Streams -> BOOLEAN]
    /\ state.credit \in [Streams -> 0..WINDOW]
    /\ state.outbound \in [Streams -> 0..QCAP]
    /\ state.inbound \in [Streams -> 0..WINDOW]
    /\ state.abandoned \in [Streams -> BOOLEAN]
    /\ state.stoppedBy \in [Streams -> Clients \cup {None}]
    /\ state.stopFresh \in [Streams -> BOOLEAN]
    /\ state.childExited \in [Streams -> BOOLEAN]
    /\ state.drained \in [Streams -> BOOLEAN]
    /\ state.lastDelivered \in [Streams -> BOOLEAN]
    /\ state.exitPublished \in [Streams -> BOOLEAN]
    /\ state.attempt \in [Clients -> Attempts]

BoundedOutstanding == \A s \in Streams :
    IF state.active[s] THEN
        state.credit[s] + state.outbound[s] + state.inbound[s] = WINDOW
    ELSE state.credit[s] + state.outbound[s] + state.inbound[s] = 0

NoStaleEffect == \A c \in Clients :
    state.attempt[c].stage # "idle" => state.attempt[c].good
NoStaleCommit == \A c \in Clients :
    state.attempt[c].stage = "committed" => state.attempt[c].appliedFresh


ForeignStopRefused == \A s \in Streams :
    state.stoppedBy[s] # None =>
        state.stoppedBy[s] = state.owner[s].client /\ state.stopFresh[s]

ExitAfterOutput == \A s \in Streams :
    state.exitPublished[s] =>
        state.drained[s] /\ state.outbound[s] = 0 /\ state.lastDelivered[s]

LastMeansFlushed == \A s \in Streams :
    state.lastDelivered[s] => state.drained[s] /\ state.outbound[s] = 0

DirtyUntilProof == \A c \in Clients :
    state.attempt[c].uncertain /\ ~state.attempt[c].ack => state.attempt[c].dirty

RecoveryInNamespace == \A c \in Clients :
    state.attempt[c].recovered => state.attempt[c].verifiedNamespace

\* Reachability checks run these NEGATIONS as invariants: each expected
\* counterexample is a live path, never a clean run on a dead branch.
WitnessSuccess == ~(\E c \in Clients : state.attempt[c].ack)
WitnessUncertain == ~(\E c \in Clients : state.attempt[c].uncertain)
WitnessRecovery == ~(\E c \in Clients :
    IF state.attempt[c].worker = None THEN FALSE ELSE
        state.attempt[c].recovered
        /\ state.generation[state.attempt[c].worker] > state.attempt[c].gen)
WitnessAbandoned == ~(\E s \in Streams : state.abandoned[s])
WitnessChangedNamespace == ~(\E c \in Clients :
    IF state.attempt[c].worker = None THEN FALSE ELSE
        state.attempt[c].uncertain
        /\ state.namespace[state.attempt[c].worker] # state.attempt[c].namespace)
WitnessBothClients == ~(\A c \in Clients : state.attempt[c].stage # "idle")
WitnessBackpressured == ~(\E s \in Streams :
    state.active[s] /\ state.credit[s] = 0 /\ state.outbound[s] = QCAP)
WitnessStalePrepared == ~(\E c \in Clients :
    IF state.attempt[c].worker = None THEN FALSE ELSE
        state.attempt[c].stage = "prepared"
        /\ state.generation[state.attempt[c].worker] > state.attempt[c].gen)
=============================================================================
