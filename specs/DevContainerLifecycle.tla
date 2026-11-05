---- MODULE DevContainerLifecycle ----
(***************************************************************************)
(* Bounded safety model for plans 0037 and 0081.  Discovery and review are *)
(* passive.  Attach only selects an inspected running receipt; start and   *)
(* rebuild provision only after exact workspace/config/engine trust.       *)
(* Receipts, rather than names, identify resources.  A same-name recreate  *)
(* or rebuild advances the incarnation and retires publications/leases.    *)
(* Cancellation is a request, not settlement: failed/cancelled operations  *)
(* with partial receipts must pass through reconciliation.                 *)
(*                                                                       *)
(* MUTATION = 1 omits the trust gate.                                    *)
(* MUTATION = 2 ignores operation incarnation at publication.             *)
(* MUTATION = 3 publishes before the configured waitFor hooks.            *)
(* MUTATION = 4 permits stopping an external container.                   *)
(*                                                                       *)
(* Resource contents, CLI rendering and progress/liveness are outside this *)
(* finite safety model.  No action removes a resource or deletes a volume. *)
(***************************************************************************)
EXTENDS Integers, TLC

CONSTANT MUTATION

Workspaces == {"w0", "w1"}
Digests == {"d0", "d1"}
Engines == {"e0", "e1"}
ContainerIds == {"external", "created", "rebuilt"}
Names == {"same-name"}
Owners == {"owned", "external"}
Kinds == {"container", "sidecar", "volume"}
NoPublication == [id |-> "none", incarnation |-> 0, workspace |-> "none",
                  digest |-> "none", engine |-> "none", ticket |-> 0,
                  kind |-> "none"]
Receipts ==
    [id: ContainerIds, name: Names, engine: Engines,
     workspace: Workspaces, digest: Digests, incarnation: 0..2,
     owner: Owners, kind: Kinds]
Publications ==
    [id: ContainerIds, incarnation: 0..2, workspace: Workspaces,
     digest: Digests, engine: Engines, ticket: 0..2,
     kind: {"attach", "start", "rebuild"}]
Leases == [id: ContainerIds, incarnation: 0..2, ticket: 0..2]
Effects == [kind: {"create", "hook"}, workspace: Workspaces,
            digest: Digests, engine: Engines, incarnation: 0..2,
            trusted: BOOLEAN]

VARIABLES workspace, digest, engine,
          reviewedWorkspace, reviewedDigest, reviewedEngine,
          trustWorkspace, trustDigest, trustEngine, trustEpoch,
          resources, running, stopped, activeId, activeInc,
          waitFor, hooks, published, leases, effects,
          opKind, opStage, opTicket, opDigest, opEngine, opInc, opBaseId,
          opCancelSeen, opFailed, opReconciled, nextTicket

vars == <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
          reviewedEngine, trustWorkspace, trustDigest, trustEngine, trustEpoch,
          resources, running, stopped, activeId, activeInc, waitFor, hooks, published, leases, effects,
          opKind, opStage, opTicket, opDigest, opEngine, opInc, opBaseId,
          opCancelSeen, opFailed, opReconciled, nextTicket>>

PassiveVars == <<resources, running, stopped, activeId, activeInc, hooks,
                 published, leases, effects>>

RequiredHooks ==
    IF waitFor = "updateContent"
    THEN {"onCreate", "updateContent"}
    ELSE {"onCreate", "updateContent", "postCreate"}

ActiveReceipts ==
    {r \in resources : r.id = activeId /\ r.incarnation = activeInc}

CurrentReceiptMatches(r) ==
    /\ r.id = activeId
    /\ r.incarnation = activeInc
    /\ r.workspace = workspace
    /\ r.digest = digest
    /\ r.engine = engine

TrustedHere ==
    /\ trustWorkspace = workspace
    /\ trustDigest = digest
    /\ trustEngine = engine
    /\ reviewedWorkspace = workspace
    /\ reviewedDigest = digest
    /\ reviewedEngine = engine

TypeOK ==
    /\ workspace \in Workspaces
    /\ digest \in Digests
    /\ engine \in Engines
    /\ reviewedWorkspace \in Workspaces \cup {"none"}
    /\ reviewedDigest \in Digests \cup {"none"}
    /\ reviewedEngine \in Engines \cup {"none"}
    /\ trustWorkspace \in Workspaces \cup {"none"}
    /\ trustDigest \in Digests \cup {"none"}
    /\ trustEngine \in Engines \cup {"none"}
    /\ trustEpoch \in 0..2
    /\ \A r \in resources : r \in Receipts
    /\ running \subseteq resources
    /\ stopped \subseteq resources
    /\ activeId \in ContainerIds
    /\ activeInc \in 0..2
    /\ waitFor \in {"updateContent", "postCreate"}
    /\ hooks \in SUBSET {"onCreate", "updateContent", "postCreate", "postStart"}
    /\ published \in Publications \cup {NoPublication}
    /\ \A l \in leases : l \in Leases
    /\ \A e \in effects : e \in Effects
    /\ opKind \in {"none", "attach", "start", "rebuild"}
    /\ opStage \in {"none", "requested", "ready", "cancelRequested",
                    "failed", "reconciled", "settled"}
    /\ opTicket \in 0..2
    /\ opDigest \in Digests \cup {"none"}
    /\ opEngine \in Engines \cup {"none"}
    /\ opInc \in 0..2
    /\ opBaseId \in ContainerIds \cup {"none"}
    /\ opCancelSeen \in BOOLEAN
    /\ opFailed \in BOOLEAN
    /\ opReconciled \in BOOLEAN
    /\ nextTicket \in 0..2

NoUntrustedEffects ==
    \A e \in effects : e.trusted

NoWrongContainer ==
    published = NoPublication \/
    /\ published.workspace = workspace
    /\ published.digest = digest
    /\ published.engine = engine
    /\ \E r \in running :
        /\ r.id = published.id
        /\ r.incarnation = published.incarnation
        /\ r.workspace = published.workspace
        /\ r.digest = published.digest
        /\ r.engine = published.engine

NoStaleWorkspacePublication ==
    published = NoPublication \/
    /\ published.id = activeId
    /\ published.incarnation = activeInc

OnlyOwnedResourcesStopped ==
    \A r \in stopped : r.owner = "owned" /\ r.kind \in {"container", "sidecar"}

NoReadyBeforeRequiredHooks ==
    /\ opStage = "ready" /\ opKind \in {"start", "rebuild"}
        => RequiredHooks \subseteq hooks
    /\ published # NoPublication /\ published.kind # "attach"
        => RequiredHooks \subseteq hooks

LeaseScopedToIncarnation ==
    \A l \in leases : l.id = activeId /\ l.incarnation = activeInc

OperationStateCoherence ==
    /\ opStage = "none" <=> opKind = "none"
    /\ opStage \in {"requested", "ready", "cancelRequested", "failed",
                    "reconciled", "settled"} => opTicket > 0
    /\ opStage = "settled" /\ (opCancelSeen \/ opFailed) => opReconciled
    /\ opStage = "cancelRequested" => opCancelSeen
    /\ opStage = "failed" => opFailed
    /\ opStage = "ready" => ~opCancelSeen /\ ~opFailed
    /\ published # NoPublication => published.ticket > 0
    /\ opKind = "rebuild" /\ opStage # "none" => opBaseId # "none"

Init ==
    LET external == [id |-> "external", name |-> "same-name", engine |-> "e0",
                     workspace |-> "w0", digest |-> "d0", incarnation |-> 0,
                     owner |-> "external", kind |-> "container"]
        volume == [id |-> "external", name |-> "same-name", engine |-> "e0",
                   workspace |-> "w0", digest |-> "d0", incarnation |-> 0,
                   owner |-> "external", kind |-> "volume"]
    IN
    /\ workspace = "w0" /\ digest = "d0" /\ engine = "e0"
    /\ reviewedWorkspace = "none" /\ reviewedDigest = "none" /\ reviewedEngine = "none"
    /\ trustWorkspace = "none" /\ trustDigest = "none" /\ trustEngine = "none" /\ trustEpoch = 0
    /\ resources = {external, volume}
    /\ running = {external}
    /\ stopped = {}
    /\ activeId = "external" /\ activeInc = 0
    /\ waitFor \in {"updateContent", "postCreate"}
    /\ hooks = {} /\ published = NoPublication /\ leases = {} /\ effects = {}
    /\ opKind = "none" /\ opStage = "none" /\ opTicket = 0
    /\ opDigest = "none" /\ opEngine = "none" /\ opInc = 0 /\ opBaseId = "none"
    /\ opCancelSeen = FALSE /\ opFailed = FALSE /\ opReconciled = FALSE
    /\ nextTicket = 0

Discover ==
    /\ opStage = "none" /\ published = NoPublication /\ leases = {}
    /\ workspace' \in Workspaces
    /\ digest' \in Digests
    /\ engine' \in Engines
    /\ reviewedWorkspace' = "none" /\ reviewedDigest' = "none" /\ reviewedEngine' = "none"
    /\ trustWorkspace' = "none" /\ trustDigest' = "none" /\ trustEngine' = "none"
    /\ UNCHANGED <<trustEpoch, waitFor, opKind, opStage, opTicket,
                   opDigest, opEngine, opInc, opBaseId, opCancelSeen, opFailed,
                   opReconciled, nextTicket, PassiveVars>>

Review ==
    /\ opStage = "none"
    /\ reviewedWorkspace' = workspace
    /\ reviewedDigest' = digest
    /\ reviewedEngine' = engine
    /\ UNCHANGED <<workspace, digest, engine, trustWorkspace, trustDigest,
                   trustEngine, trustEpoch, waitFor, opKind, opStage, opTicket,
                   opDigest, opEngine, opInc, opBaseId, opCancelSeen, opFailed,
                   opReconciled, nextTicket, PassiveVars>>

ApproveTrust ==
    /\ opStage = "none"
    /\ reviewedWorkspace = workspace
    /\ reviewedDigest = digest /\ reviewedEngine = engine
    /\ trustEpoch < 2
    /\ trustWorkspace' = workspace
    /\ trustDigest' = digest /\ trustEngine' = engine
    /\ trustEpoch' = trustEpoch + 1
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine, waitFor, opKind, opStage, opTicket, opDigest,
                   opEngine, opInc, opBaseId, opCancelSeen, opFailed, opReconciled,
                   nextTicket, PassiveVars>>

BeginAttach ==
    /\ opStage = "none" /\ nextTicket < 2
    /\ reviewedWorkspace = workspace
    /\ reviewedDigest = digest /\ reviewedEngine = engine
    /\ \E r \in running : CurrentReceiptMatches(r)
    /\ opKind' = "attach" /\ opStage' = "ready"
    /\ nextTicket' = nextTicket + 1 /\ opTicket' = nextTicket'
    /\ opDigest' = digest /\ opEngine' = engine /\ opInc' = activeInc
    /\ opBaseId' = activeId
    /\ opCancelSeen' = FALSE /\ opFailed' = FALSE /\ opReconciled' = FALSE
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, waitFor, PassiveVars>>

BeginStart ==
    /\ opStage = "none" /\ nextTicket < 2 /\ activeInc < 2
    /\ (TrustedHere \/ MUTATION = 1)
    /\ opKind' = "start" /\ opStage' = "requested"
    /\ nextTicket' = nextTicket + 1 /\ opTicket' = nextTicket'
    /\ opDigest' = digest /\ opEngine' = engine /\ opInc' = activeInc + 1
    /\ opBaseId' = activeId
    /\ opCancelSeen' = FALSE /\ opFailed' = FALSE /\ opReconciled' = FALSE
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, waitFor, PassiveVars>>

BeginRebuild ==
    /\ opStage = "none" /\ nextTicket < 2 /\ activeInc < 2
    /\ TrustedHere
    /\ \E r \in ActiveReceipts :
        /\ CurrentReceiptMatches(r)
        /\ r.owner = "owned" /\ r.kind = "container" /\ r \in running
    /\ opKind' = "rebuild" /\ opStage' = "requested"
    /\ nextTicket' = nextTicket + 1 /\ opTicket' = nextTicket'
    /\ opDigest' = digest /\ opEngine' = engine /\ opInc' = activeInc + 1
    /\ opBaseId' = activeId
    /\ opCancelSeen' = FALSE /\ opFailed' = FALSE /\ opReconciled' = FALSE
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, waitFor, PassiveVars>>

Provision ==
    /\ opStage = "requested" /\ opKind \in {"start", "rebuild"}
    /\ opDigest = digest /\ opEngine = engine /\ opInc = activeInc + 1
    /\ LET id == IF opKind = "start" THEN "created" ELSE "rebuilt"
        receipt == [id |-> id, name |-> "same-name", engine |-> engine,
                    workspace |-> workspace, digest |-> digest,
                    incarnation |-> activeInc + 1, owner |-> "owned",
                    kind |-> "container"]
        effect == [kind |-> "create", workspace |-> workspace,
                   digest |-> digest, engine |-> engine,
                   incarnation |-> activeInc + 1, trusted |-> TrustedHere]
        IN
        /\ resources' = resources \cup {receipt}
        /\ running' = running \cup {receipt}
        /\ activeId' = id /\ activeInc' = activeInc + 1
        /\ hooks' = {} /\ published' = NoPublication /\ leases' = {}
        /\ effects' = effects \cup {effect}
        /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                       reviewedEngine,
                       trustWorkspace, trustDigest, trustEngine, trustEpoch, stopped, waitFor,
                       opKind, opStage, opTicket, opDigest, opEngine, opInc,
                       opBaseId, opCancelSeen, opFailed, opReconciled, nextTicket>>

ReachReady ==
    /\ opStage = "requested" /\ opKind \in {"start", "rebuild"}
    /\ (RequiredHooks \subseteq hooks \/ MUTATION = 3)
    /\ opStage' = "ready"
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine, trustWorkspace, trustDigest, trustEngine,
                   trustEpoch, resources, running, stopped, activeId, activeInc,
                   waitFor, hooks, published, leases, effects, opKind, opTicket,
                   opDigest, opEngine, opInc, opBaseId, opCancelSeen, opFailed,
                   opReconciled, nextTicket>>

RunHook(h) ==
    /\ h \in {"onCreate", "updateContent", "postCreate", "postStart"}
    /\ opKind \in {"start", "rebuild"}
    /\ opStage \in {"requested", "ready", "cancelRequested"}
    /\ \E r \in ActiveReceipts : CurrentReceiptMatches(r) /\ r.owner = "owned"
    /\ hooks' = hooks \cup {h}
    /\ effects' = effects \cup
        {[kind |-> "hook", workspace |-> workspace, digest |-> digest,
          engine |-> engine, incarnation |-> activeInc, trusted |-> TrustedHere]}
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, published, leases,
                   opKind, opStage, opTicket, opDigest, opEngine, opInc,
                   opBaseId, opCancelSeen, opFailed, opReconciled, nextTicket>>

FailOperation ==
    /\ opStage \in {"requested", "ready"} /\ opKind \in {"start", "rebuild"}
    /\ opStage' = "failed" /\ opFailed' = TRUE
    /\ published' = NoPublication /\ leases' = {}
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, effects,
                   opKind, opTicket, opDigest, opEngine, opInc, opBaseId,
                   opCancelSeen, opReconciled, nextTicket>>

RequestCancel ==
    /\ opStage \in {"requested", "ready"}
    /\ opStage' = "cancelRequested" /\ opCancelSeen' = TRUE
    /\ published' = NoPublication /\ leases' = {}
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, effects,
                   opKind, opTicket, opDigest, opEngine, opInc, opBaseId,
                   opFailed, opReconciled, nextTicket>>

Reconcile ==
    /\ opStage \in {"cancelRequested", "failed"}
    /\ opStage' = "reconciled" /\ opReconciled' = TRUE
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, published,
                   leases, effects, opKind, opTicket, opDigest, opEngine,
                   opInc, opBaseId, opCancelSeen, opFailed, nextTicket>>

Settle ==
    /\ opStage \in {"ready", "reconciled"}
    /\ opCancelSeen \/ opFailed => opReconciled
    /\ opStage' = "settled"
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, published,
                   leases, effects, opKind, opTicket, opDigest, opEngine,
                   opInc, opBaseId, opCancelSeen, opFailed, opReconciled,
                   nextTicket>>

ClearOperation ==
    /\ opStage = "settled"
    /\ opKind' = "none" /\ opStage' = "none" /\ opTicket' = 0
    /\ opDigest' = "none" /\ opEngine' = "none" /\ opBaseId' = "none"
    /\ opCancelSeen' = FALSE /\ opFailed' = FALSE /\ opReconciled' = FALSE
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, published,
                   leases, effects, opInc, nextTicket>>

Publish ==
    /\ opStage = "ready" /\ published = NoPublication
    /\ opDigest = digest /\ opEngine = engine
    /\ (opInc = activeInc \/ MUTATION = 2)
    /\ (opKind = "attach" \/ RequiredHooks \subseteq hooks \/ MUTATION = 3)
    /\ \E r \in resources :
        /\ r.id = activeId
        /\ (r.incarnation = opInc \/ MUTATION = 2)
        /\ r.workspace = workspace
        /\ r.digest = digest
        /\ r.engine = engine
        /\ (opKind = "attach" => r \in running)
        /\ published' = [id |-> r.id, incarnation |-> r.incarnation,
                         workspace |-> workspace, digest |-> digest,
                         engine |-> engine, ticket |-> opTicket, kind |-> opKind]
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, leases,
                   effects, opKind, opStage, opTicket, opDigest, opEngine,
                   opInc, opBaseId, opCancelSeen, opFailed, opReconciled,
                   nextTicket>>

IssueLease ==
    /\ opStage = "ready" /\ published # NoPublication
    /\ published.id = activeId /\ published.incarnation = activeInc
    /\ leases' = leases \cup
        {[id |-> activeId, incarnation |-> activeInc, ticket |-> opTicket]}
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, running,
                   stopped, activeId, activeInc, waitFor, hooks, published,
                   effects, opKind, opStage, opTicket, opDigest, opEngine,
                   opInc, opBaseId, opCancelSeen, opFailed, opReconciled,
                   nextTicket>>

ExternalSameNameRecreate ==
    /\ activeInc < 2
    /\ LET receipt == [id |-> "external", name |-> "same-name", engine |-> engine,
                    workspace |-> workspace, digest |-> digest,
                    incarnation |-> activeInc + 1, owner |-> "external",
                    kind |-> "container"]
        IN
        /\ resources' = resources \cup {receipt}
        /\ running' = running \cup {receipt}
        /\ activeId' = "external" /\ activeInc' = activeInc + 1
        /\ opStage' = IF opKind = "none" THEN "none" ELSE "failed"
        /\ opFailed' = (opKind # "none")
        /\ hooks' = {} /\ published' = NoPublication /\ leases' = {}
        /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                       reviewedEngine,
                       trustWorkspace, trustDigest, trustEngine, trustEpoch, stopped, waitFor,
                       effects, opKind, opTicket, opDigest, opEngine,
                       opInc, opBaseId, opCancelSeen, opReconciled,
                       nextTicket>>

StopResource(r) ==
    /\ r \in resources /\ r \in running
    /\ r.kind \in {"container", "sidecar"}
    /\ (r.id = activeId /\ r.incarnation = activeInc) => opStage = "none"
    /\ (r.owner = "owned" \/ MUTATION = 4)
    /\ stopped' = stopped \cup {r}
    /\ running' = running \ {r}
    /\ IF r.id = activeId /\ r.incarnation = activeInc
       THEN published' = NoPublication /\ leases' = {}
       ELSE UNCHANGED <<published, leases>>
    /\ UNCHANGED <<workspace, digest, engine, reviewedWorkspace, reviewedDigest,
                   reviewedEngine,
                   trustWorkspace, trustDigest, trustEngine, trustEpoch, resources, activeId,
                   activeInc, waitFor, hooks, effects, opKind, opStage,
                   opTicket, opDigest, opEngine, opInc, opBaseId,
                   opCancelSeen, opFailed, opReconciled, nextTicket>>

Next ==
    \/ Discover \/ Review \/ ApproveTrust
    \/ BeginAttach \/ BeginStart \/ BeginRebuild \/ Provision \/ ReachReady
    \/ \E h \in {"onCreate", "updateContent", "postCreate", "postStart"} :
        RunHook(h)
    \/ FailOperation \/ RequestCancel \/ Reconcile \/ Settle \/ ClearOperation
    \/ Publish \/ IssueLease \/ ExternalSameNameRecreate
    \/ \E r \in resources : StopResource(r)

Spec == Init /\ [][Next]_vars

Safety ==
    /\ TypeOK
    /\ NoUntrustedEffects
    /\ NoWrongContainer
    /\ NoStaleWorkspacePublication
    /\ OnlyOwnedResourcesStopped
    /\ NoReadyBeforeRequiredHooks
    /\ LeaseScopedToIncarnation
    /\ OperationStateCoherence
=============================================================================
