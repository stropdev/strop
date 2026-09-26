---- MODULE WorkerDeploy ----
EXTENDS Naturals, FiniteSets, TLC

\* 0058 WK17: the deployed object is keyed by selected context and digest,
\* not by a display name or the mutable Docker default. The provider's
\* lstat/read-back/owner/exec observations are environment premises;
\* the digest is modeled as an unforgeable symbolic content identity.
\* No theorem about SHA-256 collision resistance or remote filesystem
\* atomicity follows from this finite state machine.
\*
\* Rust correspondence: worker-deploy/{deploy,cache,gc}.rs, the SFTP
\* provider and ContainerProvider capture/handshake, editor registry
\* admission. Upload stages are uniquely owned; competing installers
\* may publish the same verified content-addressed object. Only a real
\* handshake plus receipt registers a ready live lease.

CONSTANTS Clients, Contexts, Digests, MUTATION
ASSUME /\ Clients # {} /\ Contexts # {} /\ Digests # {}
       /\ MUTATION \in 0..7

None == "none"
Corrupt == "corrupt"
Targets == [context : Contexts, digest : Digests]
NoTarget == [context |-> None, digest |-> None]
Phases == {"idle", "selected", "staged", "transferred", "published",
           "verified", "receipted", "ready", "failed"}

VARIABLE state
vars == <<state>>

Init == state = [
    phase |-> [c \in Clients |-> "idle"],
    selected |-> [c \in Clients |-> None],
    default |-> [c \in Clients |-> None],
    wanted |-> [c \in Clients |-> None],
    consent |-> [c \in Clients |-> FALSE],
    stage |-> [c \in Clients |-> NoTarget],
    stageOwner |-> [c \in Clients |-> None],
    stagedVerified |-> [c \in Clients |-> FALSE],
    published |-> [t \in Targets |-> FALSE],
    authorized |-> [t \in Targets |-> FALSE],
    content |-> [t \in Targets |-> None],
    verified |-> [t \in Targets |-> FALSE],
    receipt |-> [c \in Clients |-> FALSE],
    lease |-> [c \in Clients |-> NoTarget],
    readyContext |-> [c \in Clients |-> None],
    badCleanup |-> FALSE,
    retiredWhileLeased |-> FALSE,
    uploadedWithoutConsent |-> FALSE
]

Target(c) == [context |-> state.selected[c], digest |-> state.wanted[c]]
Live(t) == \E c \in Clients : state.lease[c] = t

\* Full-state construction preserves TLC semantics while TLAPS handles one-level updates.
StateRecord(newPhase, newSelected, newDefault, newWanted, newConsent, newStage, newStageOwner, newStagedVerified, newPublished, newAuthorized, newContent, newVerified, newReceipt, newLease, newReadyContext, newBadCleanup, newRetiredWhileLeased, newUploadedWithoutConsent) ==
    [ phase |-> newPhase,
      selected |-> newSelected,
      default |-> newDefault,
      wanted |-> newWanted,
      consent |-> newConsent,
      stage |-> newStage,
      stageOwner |-> newStageOwner,
      stagedVerified |-> newStagedVerified,
      published |-> newPublished,
      authorized |-> newAuthorized,
      content |-> newContent,
      verified |-> newVerified,
      receipt |-> newReceipt,
      lease |-> newLease,
      readyContext |-> newReadyContext,
      badCleanup |-> newBadCleanup,
      retiredWhileLeased |-> newRetiredWhileLeased,
      uploadedWithoutConsent |-> newUploadedWithoutConsent ]

Select(c, context, digest) ==
    /\ state.phase[c] = "idle"
    /\ context \in Contexts /\ digest \in Digests
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "selected"],
        [state.selected EXCEPT ![c] = context],
        [state.default EXCEPT ![c] = context],
        [state.wanted EXCEPT ![c] = digest],
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

ChangeDefault(c, context) ==
    /\ state.phase[c] # "idle" /\ context \in Contexts
    /\ state' = StateRecord(
        state.phase,
        state.selected,
        [state.default EXCEPT ![c] = context],
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Authorize(c) ==
    /\ state.phase[c] = "selected"
    /\ state' = StateRecord(
        state.phase,
        state.selected,
        state.default,
        state.wanted,
        [state.consent EXCEPT ![c] = TRUE],
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Upload(c) ==
    /\ state.phase[c] = "selected"
    /\ (state.consent[c] \/ MUTATION = 1)
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "staged"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        [state.stage EXCEPT ![c] = Target(c)],
        [state.stageOwner EXCEPT ![c] = c],
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        (state.uploadedWithoutConsent \/ ~state.consent[c])
        )

VerifyTransfer(c) ==
    /\ state.phase[c] = "staged"
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "transferred"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        [state.stagedVerified EXCEPT ![c] = TRUE],
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Publish(c) ==
    /\ state.phase[c] = "transferred" /\ state.stagedVerified[c]
    /\ ( ~state.published[Target(c)]
         \/ state.content[Target(c)] = state.wanted[c] )
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "published"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        [state.stage EXCEPT ![c] = NoTarget],
        [state.stageOwner EXCEPT ![c] = None],
        state.stagedVerified,
        [state.published EXCEPT ![Target(c)] = TRUE],
        state.authorized,
        [state.content EXCEPT ![Target(c)] = state.wanted[c]],
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

\* A disk fault/corrupt transfer may change a published object's bytes;
\* no readiness follows until the final-path verifier observes it.
CorruptObject(t) ==
    /\ t \in Targets /\ state.published[t] /\ ~Live(t)
    /\ state' = StateRecord(
        state.phase,
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        [state.content EXCEPT ![t] = Corrupt],
        [state.verified EXCEPT ![t] = FALSE],
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

VerifyObject(c) ==
    /\ state.phase[c] = "published"
    /\ state.published[Target(c)]
    /\ (state.content[Target(c)] = state.wanted[c] \/ MUTATION = 2)
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "verified"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        [state.verified EXCEPT ![Target(c)] = TRUE],
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

WriteReceipt(c) ==
    /\ state.phase[c] = "verified" /\ state.verified[Target(c)]
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "receipted"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        [state.authorized EXCEPT ![Target(c)] = TRUE],
        state.content,
        state.verified,
        [state.receipt EXCEPT ![c] = TRUE],
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Activate(c) ==
    /\ state.phase[c] = "receipted" /\ state.receipt[c]
    /\ (state.verified[Target(c)] \/ MUTATION = 6)
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "ready"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        [state.lease EXCEPT ![c] = Target(c)],
        [state.readyContext EXCEPT ![c] = IF MUTATION = 3 THEN state.default[c]
                             ELSE state.selected[c]],
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

HandshakeFailure(c) ==
    /\ state.phase[c] = "receipted"
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "failed"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Retire(c) ==
    /\ state.phase[c] \in {"ready", "failed"}
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "idle"],
        state.selected,
        state.default,
        state.wanted,
        [state.consent EXCEPT ![c] = FALSE],
        state.stage,
        state.stageOwner,
        [state.stagedVerified EXCEPT ![c] = FALSE],
        state.published,
        state.authorized,
        state.content,
        state.verified,
        [state.receipt EXCEPT ![c] = FALSE],
        [state.lease EXCEPT ![c] = NoTarget],
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

CleanStage(actor, owner) ==
    /\ actor \in Clients /\ owner \in Clients
    /\ state.stage[owner] # NoTarget
    /\ (actor = owner \/ MUTATION = 4)
    /\ state' = StateRecord(
        state.phase,
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        [state.stage EXCEPT ![owner] = NoTarget],
        [state.stageOwner EXCEPT ![owner] = None],
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        state.receipt,
        state.lease,
        state.readyContext,
        (state.badCleanup \/ actor # owner),
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Collect(actor, t) ==
    /\ actor \in Clients /\ t \in Targets
    /\ state.published[t]
    /\ (state.selected[actor] = t.context \/ MUTATION = 7)
    /\ (~Live(t) \/ MUTATION = 5)
    /\ state' = StateRecord(
        state.phase,
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        [state.published EXCEPT ![t] = FALSE],
        state.authorized,
        [state.content EXCEPT ![t] = None],
        [state.verified EXCEPT ![t] = FALSE],
        state.receipt,
        state.lease,
        state.readyContext,
        (state.badCleanup \/ state.selected[actor] # t.context),
        (state.retiredWhileLeased \/ Live(t)),
        state.uploadedWithoutConsent
        )

\* A corrupted object cannot be treated as a verified cache hit. The
\* mutant bypasses this final-content premise and registers a wrong
\* executable object under a real-looking receipt.
Reuse(c) ==
    /\ state.phase[c] = "selected" /\ state.published[Target(c)]
    /\ state.authorized[Target(c)]
    /\ ((state.verified[Target(c)]
         /\ state.content[Target(c)] = state.wanted[c]) \/ MUTATION = 6)
    /\ state' = StateRecord(
        [state.phase EXCEPT ![c] = "receipted"],
        state.selected,
        state.default,
        state.wanted,
        state.consent,
        state.stage,
        state.stageOwner,
        state.stagedVerified,
        state.published,
        state.authorized,
        state.content,
        state.verified,
        [state.receipt EXCEPT ![c] = TRUE],
        state.lease,
        state.readyContext,
        state.badCleanup,
        state.retiredWhileLeased,
        state.uploadedWithoutConsent
        )

Next == \/ \E c \in Clients, context \in Contexts, digest \in Digests :
                Select(c, context, digest)
        \/ \E c \in Clients, context \in Contexts : ChangeDefault(c, context)
        \/ \E c \in Clients : Authorize(c) \/ Upload(c) \/ VerifyTransfer(c)
                             \/ Publish(c) \/ VerifyObject(c) \/ WriteReceipt(c)
                             \/ Activate(c) \/ HandshakeFailure(c) \/ Retire(c)
                             \/ Reuse(c)
        \/ \E t \in Targets : CorruptObject(t)
        \/ \E actor \in Clients, owner \in Clients : CleanStage(actor, owner)
        \/ \E actor \in Clients, t \in Targets : Collect(actor, t)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ state.phase \in [Clients -> Phases]
    /\ state.selected \in [Clients -> Contexts \cup {None}]
    /\ state.default \in [Clients -> Contexts \cup {None}]
    /\ state.wanted \in [Clients -> Digests \cup {None}]
    /\ state.consent \in [Clients -> BOOLEAN]
    /\ state.stage \in [Clients -> Targets \cup {NoTarget}]
    /\ state.stageOwner \in [Clients -> Clients \cup {None}]
    /\ state.stagedVerified \in [Clients -> BOOLEAN]
    /\ state.published \in [Targets -> BOOLEAN]
    /\ state.content \in [Targets -> Digests \cup {None, Corrupt}]
    /\ state.verified \in [Targets -> BOOLEAN]
    /\ state.authorized \in [Targets -> BOOLEAN]
    /\ state.receipt \in [Clients -> BOOLEAN]
    /\ state.lease \in [Clients -> Targets \cup {NoTarget}]
    /\ state.readyContext \in [Clients -> Contexts \cup {None}]
    /\ state.badCleanup \in BOOLEAN
    /\ state.retiredWhileLeased \in BOOLEAN
    /\ state.uploadedWithoutConsent \in BOOLEAN

\* Inductive bridge facts: selecting a real target precedes every
\* non-idle action, and final-path verification cannot certify wrong bytes.
SelectedTargetValid == \A c \in Clients :
    state.phase[c] # "idle" =>
        [context |-> state.selected[c],
         digest |-> state.wanted[c]] \in Targets
VerifiedObjectTrusted == \A t \in Targets :
    state.verified[t] => state.published[t] /\ state.content[t] = t.digest

VerifiedActivation == \A c \in Clients :
    state.lease[c] # NoTarget =>
        state.published[state.lease[c]]
        /\ state.verified[state.lease[c]]
        /\ state.content[state.lease[c]] = state.lease[c].digest
        /\ state.receipt[c]

ChosenContext == \A c \in Clients :
    state.phase[c] = "ready" => state.readyContext[c] = state.selected[c]

OwnedCleanup == ~state.badCleanup
NoRetireLive == ~state.retiredWhileLeased
ConsentBeforeUpload == ~state.uploadedWithoutConsent

\* Each negated witness is run alone as an invariant. Its named
\* counterexample proves a scenario is reachable, not dead code.
WitnessReady == ~(\E c \in Clients : state.phase[c] = "ready")
WitnessPublishedNotReady == ~(\E c \in Clients : state.phase[c] = "failed")
WitnessBothInstallers == ~(\A c \in Clients : state.phase[c] # "idle")
WitnessLiveLease == ~(\E c \in Clients : state.lease[c] # NoTarget)
WitnessConcurrentInstallers == ~(\E a, b \in Clients :
    a # b /\ state.stage[a] # NoTarget /\ state.stage[a] = state.stage[b])
WitnessCorruptUnready == ~(\E c \in Clients :
    IF state.selected[c] = None THEN FALSE ELSE
        state.phase[c] = "published"
        /\ state.content[Target(c)] = Corrupt
        /\ ~state.verified[Target(c)])
WitnessDifferentContexts == ~(\E a, b \in Clients :
    a # b /\ state.selected[a] # None /\ state.selected[b] # None
    /\ state.selected[a] # state.selected[b])
=============================================================================
