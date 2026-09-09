---- MODULE RemoteSave ----
EXTENDS Naturals, Integers, FiniteSets, TLC

(* One attempt per participant bounds this model. It checks the write protocol,
   not SSH cryptography, kernel correctness or nonparticipant exclusion.
   Mutation 1: stat-only comparison; 2: write original during staging;
   3: lose metadata; 4: follow symlink; 5: expose staged draft;
   6: replace lock namespace; 7: ignore receipt ownership; 8: acknowledge before sync.
   COOPERATING = FALSE is a limitation witness, not a product guarantee. *)
CONSTANTS Actors, MUTATION, COOPERATING, ADVERSITY
VARIABLES file, durable, linkTarget, lockEpoch, held, phase, client,
          baseline, revision, permit, capturedRevision, capturedPermit,
          dirty, cancelled, stage, stageMode, protected, ack,
          commits, externalDone, verified, crashed
vars == <<file, durable, linkTarget, lockEpoch, held, phase, client,
          baseline, revision, permit, capturedRevision, capturedPermit,
          dirty, cancelled, stage, stageMode, protected, ack,
          commits, externalDone, verified, crashed>>

InitialFile == [kind |-> 0, bytes |-> 0, mode |-> 1, time |-> 0, inode |-> 0]
Phases == {"idle", "acquire", "partial", "whole", "metadata", "synced",
           "ready", "renamed", "durable", "done", "crashed"}
Clients == {"idle", "pending", "confirmed", "refused", "unconfirmed"}
NoLocks == \A a \in Actors: held[a] = 0
Compatible(a) == IF MUTATION = 1
                 THEN file.mode = baseline[a].mode /\ file.time = baseline[a].time
                 ELSE file = baseline[a]
Owns(a) == revision[a] = capturedRevision[a] /\ permit[a] = capturedPermit[a]
PreCommit(a) == phase[a] \in {"acquire", "partial", "whole", "metadata", "synced", "ready"}

Init ==
    /\ file = InitialFile /\ durable = TRUE /\ linkTarget = 0
    /\ lockEpoch = 1 /\ held = [a \in Actors |-> 0]
    /\ phase = [a \in Actors |-> "idle"] /\ client = [a \in Actors |-> "idle"]
    /\ baseline = [a \in Actors |-> InitialFile]
    /\ revision = [a \in Actors |-> 0] /\ permit = [a \in Actors |-> 0]
    /\ capturedRevision = revision /\ capturedPermit = permit
    /\ dirty = [a \in Actors |-> TRUE] /\ cancelled = [a \in Actors |-> FALSE]
    /\ stage = [a \in Actors |-> 0] /\ stageMode = [a \in Actors |-> 0]
    /\ protected = [a \in Actors |-> TRUE]
    /\ ack = [a \in Actors |-> [revision |-> 0, permit |-> 0, synced |-> FALSE]]
    /\ commits = {} /\ externalDone = FALSE
    /\ verified = [a \in Actors |-> FALSE] /\ crashed = [a \in Actors |-> FALSE]

Submit(a) ==
    /\ phase[a] = "idle"
    /\ phase' = [phase EXCEPT ![a] = "acquire"]
    /\ client' = [client EXCEPT ![a] = "pending"]
    /\ baseline' = [baseline EXCEPT ![a] = file]
    /\ capturedRevision' = [capturedRevision EXCEPT ![a] = revision[a]]
    /\ capturedPermit' = [capturedPermit EXCEPT ![a] = permit[a]]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, held, revision, permit,
                   dirty, cancelled, stage, stageMode, protected, ack, commits,
                   externalDone, verified, crashed>>

Acquire(a) ==
    /\ phase[a] = "acquire"
    /\ ~\E b \in Actors: held[b] = lockEpoch
    /\ held' = [held EXCEPT ![a] = lockEpoch]
    /\ phase' = [phase EXCEPT ![a] = "partial"]
    /\ stage' = [stage EXCEPT ![a] = 1]
    /\ protected' = [protected EXCEPT ![a] = (MUTATION # 5)]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, client, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stageMode, ack, commits, externalDone, verified, crashed>>

CompleteStage(a) ==
    /\ phase[a] = "partial"
    /\ phase' = [phase EXCEPT ![a] = "whole"]
    /\ stage' = [stage EXCEPT ![a] = 2]
    /\ file' = IF MUTATION = 2 THEN [file EXCEPT !.bytes = -1] ELSE file
    /\ UNCHANGED <<durable, linkTarget, lockEpoch, held, client, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stageMode, protected, ack, commits, externalDone, verified, crashed>>

RestoreMetadata(a) ==
    /\ phase[a] = "whole"
    /\ phase' = [phase EXCEPT ![a] = "metadata"]
    /\ stageMode' = [stageMode EXCEPT ![a] = IF MUTATION = 3 THEN 1 - baseline[a].mode ELSE baseline[a].mode]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, held, client, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stage, protected, ack, commits, externalDone, verified, crashed>>

SyncStage(a) ==
    /\ phase[a] = "metadata"
    /\ phase' = [phase EXCEPT ![a] = "synced"]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, held, client, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stage, stageMode, protected, ack, commits, externalDone, verified, crashed>>

Validate(a) ==
    /\ phase[a] = "synced"
    /\ LET accepted == Compatible(a) /\ (file.kind = 0 \/ MUTATION = 4) IN
       /\ phase' = [phase EXCEPT ![a] = IF accepted THEN "ready" ELSE "done"]
       /\ client' = [client EXCEPT ![a] = IF accepted THEN @ ELSE "refused"]
       /\ held' = [held EXCEPT ![a] = IF accepted THEN @ ELSE 0]
       /\ stage' = [stage EXCEPT ![a] = IF accepted THEN @ ELSE 0]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stageMode, protected, ack, commits, externalDone, verified, crashed>>

Commit(a) ==
    /\ phase[a] = "ready"
    /\ phase' = [phase EXCEPT ![a] = "renamed"]
    /\ commits' = commits \cup {[base |-> baseline[a], before |-> file,
                                 metadata |-> (stageMode[a] = baseline[a].mode)]}
    /\ file' = IF file.kind = 1 THEN file ELSE
               [kind |-> 0, bytes |-> a, mode |-> stageMode[a], time |-> baseline[a].time, inode |-> a]
    /\ linkTarget' = IF file.kind = 1 THEN a ELSE linkTarget
    /\ durable' = FALSE /\ stage' = [stage EXCEPT ![a] = 0]
    /\ UNCHANGED <<lockEpoch, held, client, baseline, revision, permit,
                   capturedRevision, capturedPermit, dirty, cancelled, stageMode,
                   protected, ack, externalDone, verified, crashed>>

SyncCommit(a) ==
    /\ phase[a] = "renamed"
    /\ phase' = [phase EXCEPT ![a] = "durable"] /\ durable' = TRUE
    /\ UNCHANGED <<file, linkTarget, lockEpoch, held, client, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stage, stageMode, protected, ack, commits, externalDone, verified, crashed>>

Deliver(a) ==
    /\ phase[a] = "durable" \/ (MUTATION = 8 /\ phase[a] = "renamed")
    /\ phase' = [phase EXCEPT ![a] = "done"] /\ held' = [held EXCEPT ![a] = 0]
    /\ LET accepted == client[a] = "pending" /\ (Owns(a) \/ MUTATION = 7) IN
       /\ client' = [client EXCEPT ![a] = IF accepted THEN "confirmed" ELSE IF @ = "pending" THEN "refused" ELSE @]
       /\ dirty' = [dirty EXCEPT ![a] = IF accepted THEN FALSE ELSE @]
       /\ ack' = [ack EXCEPT ![a] = IF accepted THEN
            [revision |-> capturedRevision[a], permit |-> capturedPermit[a], synced |-> durable] ELSE @]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, baseline, revision, permit,
                   capturedRevision, capturedPermit, cancelled, stage, stageMode,
                   protected, commits, externalDone, verified, crashed>>

Cancel(a) ==
    /\ ADVERSITY /\ client[a] = "pending" /\ ~cancelled[a]
    /\ client' = [client EXCEPT ![a] = "unconfirmed"]
    /\ cancelled' = [cancelled EXCEPT ![a] = TRUE]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, held, phase, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   stage, stageMode, protected, ack, commits, externalDone, verified, crashed>>

ObserveCancel(a) ==
    /\ cancelled[a] /\ PreCommit(a)
    /\ phase' = [phase EXCEPT ![a] = "done"] /\ held' = [held EXCEPT ![a] = 0]
    /\ stage' = [stage EXCEPT ![a] = 0]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, client, baseline,
                   revision, permit, capturedRevision, capturedPermit, dirty,
                   cancelled, stageMode, protected, ack, commits, externalDone, verified, crashed>>

Crash(a) ==
    /\ ADVERSITY /\ phase[a] \notin {"idle", "done", "crashed"}
    /\ phase' = [phase EXCEPT ![a] = "crashed"] /\ held' = [held EXCEPT ![a] = 0]
    /\ client' = [client EXCEPT ![a] = IF @ = "pending" THEN "unconfirmed" ELSE @]
    /\ crashed' = [crashed EXCEPT ![a] = TRUE]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, baseline, revision,
                   permit, capturedRevision, capturedPermit, dirty, cancelled,
                   stage, stageMode, protected, ack, commits, externalDone, verified>>

LocalChange(a) ==
    /\ ADVERSITY /\ client[a] # "idle" /\ revision[a] = 0
    /\ revision' = [revision EXCEPT ![a] = 1] /\ dirty' = [dirty EXCEPT ![a] = TRUE]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, held, phase, client, baseline,
                   permit, capturedRevision, capturedPermit, cancelled, stage,
                   stageMode, protected, ack, commits, externalDone, verified, crashed>>

Revoke(a) ==
    /\ ADVERSITY /\ client[a] # "idle" /\ permit[a] = 0
    /\ permit' = [permit EXCEPT ![a] = 1] /\ dirty' = [dirty EXCEPT ![a] = TRUE]
    /\ UNCHANGED <<file, durable, linkTarget, lockEpoch, held, phase, client, baseline,
                   revision, capturedRevision, capturedPermit, cancelled, stage,
                   stageMode, protected, ack, commits, externalDone, verified, crashed>>

Verify(a) ==
    /\ client[a] = "unconfirmed" /\ NoLocks /\ ~verified[a]
    /\ verified' = [verified EXCEPT ![a] = TRUE]
    /\ LET intended == file.kind = 0 /\ file.bytes = a /\ file.mode = baseline[a].mode
                       /\ file.time = baseline[a].time IN
       /\ durable' = IF intended /\ MUTATION # 8 THEN TRUE ELSE durable
       /\ client' = [client EXCEPT ![a] = IF intended THEN "confirmed" ELSE "refused"]
       /\ dirty' = [dirty EXCEPT ![a] = IF intended /\ Owns(a) THEN FALSE ELSE @]
       /\ ack' = [ack EXCEPT ![a] = IF intended /\ Owns(a) THEN
            [revision |-> capturedRevision[a], permit |-> capturedPermit[a],
             synced |-> (durable \/ MUTATION # 8)] ELSE @]
    /\ UNCHANGED <<file, linkTarget, lockEpoch, held, phase, baseline, revision,
                   permit, capturedRevision, capturedPermit, cancelled, stage,
                   stageMode, protected, commits, externalDone, crashed>>

ExternalChange ==
    /\ ~externalDone /\ (NoLocks \/ ~COOPERATING)
    /\ file' \in {[kind |-> kind, bytes |-> 3, mode |-> 1, time |-> 0, inode |-> 3] : kind \in {0, 1}}
    /\ externalDone' = TRUE /\ durable' = TRUE
    /\ UNCHANGED <<linkTarget, lockEpoch, held, phase, client, baseline, revision,
                   permit, capturedRevision, capturedPermit, dirty, cancelled,
                   stage, stageMode, protected, ack, commits, verified, crashed>>

ReplaceLock ==
    /\ MUTATION = 6 /\ lockEpoch = 1 /\ ~NoLocks
    /\ lockEpoch' = 2
    /\ UNCHANGED <<file, durable, linkTarget, held, phase, client, baseline, revision,
                   permit, capturedRevision, capturedPermit, dirty, cancelled,
                   stage, stageMode, protected, ack, commits, externalDone, verified, crashed>>

Step(a) == Acquire(a) \/ CompleteStage(a) \/ RestoreMetadata(a) \/ SyncStage(a)
           \/ Validate(a) \/ Commit(a) \/ SyncCommit(a) \/ Deliver(a) \/ ObserveCancel(a)
Terminal == (\A a \in Actors: phase[a] \in {"done", "crashed"}) /\ UNCHANGED vars
Next == (\E a \in Actors: Submit(a) \/ Step(a) \/ Cancel(a) \/ Crash(a) \/ LocalChange(a) \/ Revoke(a) \/ Verify(a))
        \/ ExternalChange \/ ReplaceLock \/ Terminal
Spec == Init /\ [][Next]_vars /\ \A a \in Actors: WF_vars(Step(a))

FileDomain == [kind : 0..1, bytes : (-1)..3, mode : 0..1, time : {0}, inode : 0..3]
AckDomain == [revision : 0..1, permit : 0..1, synced : BOOLEAN]
TypeOK == /\ Actors \subseteq {1, 2} /\ phase \in [Actors -> Phases] /\ client \in [Actors -> Clients]
          /\ held \in [Actors -> 0..2] /\ lockEpoch \in 1..2
          /\ revision \in [Actors -> 0..1] /\ permit \in [Actors -> 0..1]
          /\ stage \in [Actors -> 0..2] /\ stageMode \in [Actors -> 0..1]
          /\ dirty \in [Actors -> BOOLEAN] /\ cancelled \in [Actors -> BOOLEAN]
          /\ protected \in [Actors -> BOOLEAN] /\ durable \in BOOLEAN
          /\ file \in FileDomain /\ linkTarget \in 0..2 /\ baseline \in [Actors -> FileDomain]
          /\ capturedRevision \in [Actors -> 0..1] /\ capturedPermit \in [Actors -> 0..1]
          /\ ack \in [Actors -> AckDomain] /\ externalDone \in BOOLEAN
          /\ verified \in [Actors -> BOOLEAN] /\ crashed \in [Actors -> BOOLEAN]
          /\ commits \subseteq [base : FileDomain, before : FileDomain, metadata : BOOLEAN]
NoPartialOriginal == file.bytes \in 0..3
NoBlindOverwrite == \A entry \in commits: entry.base = entry.before
MetadataPreserved == \A entry \in commits: entry.metadata
NoLinkWrites == linkTarget = 0
PrivateDraft == \A a \in Actors: stage[a] > 0 => protected[a] \/ stageMode[a] = 0
DurableAcknowledgment == \A a \in Actors: ~dirty[a] => ack[a].synced
OwnedAcknowledgment == \A a \in Actors: ~dirty[a] => ack[a].revision = revision[a] /\ ack[a].permit = permit[a]
SingleLockDomain == \A a, b \in Actors: (a # b /\ held[a] > 0 /\ held[b] > 0) => FALSE
RequestsSettle == \A a \in Actors: client[a] = "pending" ~> client[a] # "pending"
WitnessNoCommit == commits = {}
WitnessNoConfirmed == \A a \in Actors: client[a] # "confirmed"
WitnessNoCancelledCommit == \A a \in Actors: cancelled[a] => file.bytes # a
WitnessNoPrivateOrphan == \A a \in Actors: ~(crashed[a] /\ stage[a] > 0 /\ protected[a])
WitnessNoVerification == \A a \in Actors: ~verified[a]
=============================================================================
