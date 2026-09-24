---- MODULE Recovery ----
(***************************************************************************)
(* Draft checkpoint/recovery safety (0057 VF13) over the AR04 contract as  *)
(* implemented in crates/strop-engine/src/editor/recovery/{mod,store,      *)
(* surface}.rs. The model owns the cohort checkpoint/publish/restore/      *)
(* save-retire boundary:                                                  *)
(*                                                                       *)
(*   CLOCKS   applied revision (the live draft), captured checkpoint      *)
(*            (the immutable cohort snapshot handed to the worker) and    *)
(*            durable checkpoint (the atomically published store) are     *)
(*            three separate clocks. Checkpointing never marks text       *)
(*            clean and never advances the saved baseline; the recovery   *)
(*            guarantee is the last COMPLETED durable checkpoint, not     *)
(*            unflushed bytes after a crash (AR04 §5).                    *)
(*                                                                       *)
(*   CAPTURE  capture is SYNCHRONOUS with the triggering event:           *)
(*            recovery_after_event runs inside the edit/save/consent      *)
(*            handler, so a stale queued cohort never survives the        *)
(*            event that made it stale — the newest capture replaces      *)
(*            the unwritten queued one (mod.rs recovery_publish: one      *)
(*            in flight, one queued). The only non-synchronous staleness  *)
(*            is the publication-failure rewind, recaptured on the next   *)
(*            event (the standalone Capture action).                      *)
(*                                                                       *)
(*   COHORT   a multi-document change captures one coherent cohort; the   *)
(*            store publishes it all-or-nothing through staged,           *)
(*            permission-private, atomic publication (store.rs            *)
(*            publish_cohort). A failed or interrupted publication        *)
(*            leaves the last complete checkpoint untouched, and the      *)
(*            captured clock rewinds to the durable one so the next       *)
(*            event retries (mod.rs finish_publication). Snapshot slots   *)
(*            hold content only while their cohort is in flight or        *)
(*            queued — publication/failure/crash clears them.             *)
(*                                                                       *)
(*   EPOCH    records key on the document's binding incarnation (AR04's   *)
(*            resource binding epoch; DocumentId in the code). A          *)
(*            crash/reopen is a new incarnation: a revision coincidence   *)
(*            across it can never alias the captured/durable clocks.      *)
(*                                                                       *)
(*   SAVE     a confirmed save retires only the checkpoint it             *)
(*            supersedes: the next cohort simply excludes the now-clean   *)
(*            draft; edits made after the saved snapshot are dirty        *)
(*            again and captured by a later cohort (recovery_note_saved). *)
(*                                                                       *)
(*   RESTORE  restore lands in a checked pathless dirty draft: it never   *)
(*            overwrites changed disk content, never recreates a          *)
(*            deleted target and never re-grants authority (surface.rs    *)
(*            recovery_restore). The current/conflict/missing comparison  *)
(*            reads the checkpoint's source observation, taken            *)
(*            worker-side at publish time (store.rs observe).             *)
(*                                                                       *)
(*   POLICY   memory-only mode persists nothing and says so; remote       *)
(*            drafts persist only with explicit session consent           *)
(*            (privacy.rs persistence_admitted; consent is per-session    *)
(*            and resets on restart).                                     *)
(*                                                                       *)
(* THE invariants (VF13's named safety properties):                       *)
(*                                                                       *)
(*   TypeOK                    every variable stays in its declared       *)
(*                             finite domain                              *)
(*   CohortCoherent            every durable record belongs to the same   *)
(*                             cohort — recovery never mixes half of a    *)
(*                             newer group with half of an older          *)
(*                             checkpoint                                 *)
(*   DurableMatchesLastComplete                                           *)
(*                             the durable store is exactly the image of  *)
(*                             the last COMPLETED publication — a failed  *)
(*                             or interrupted one changes nothing         *)
(*                             (last-complete-checkpoint retention)       *)
(*   SaveRetiresOnlySuperseded when the pipeline has settled, every       *)
(*                             eligible dirty draft is durable at its     *)
(*                             current applied revision and incarnation   *)
(*                             — a save retired only the checkpoint it    *)
(*                             superseded                                 *)
(*   CheckpointNeverCleans     no checkpoint/publish step cleared a       *)
(*                             draft's dirty flag                         *)
(*   RemoteConsentGated        a durable remote draft exists only under   *)
(*                             a cohort captured with explicit consent    *)
(*   RestoreNeverWritesDisk    no restore wrote to (or recreated on)      *)
(*                             the filesystem                             *)
(*                                                                       *)
(* Deliberately faulty variants (configs flip MUTATION):                  *)
(*   MUTATION = 1  publication MERGES the new cohort into the old store   *)
(*                 instead of replacing it — half-cohort mixing;          *)
(*                 must die by CohortCoherent and                         *)
(*                 DurableMatchesLastComplete                             *)
(*   MUTATION = 2  a failed publication clears the durable store —        *)
(*                 premature cleanup; must die by                         *)
(*                 DurableMatchesLastComplete                             *)
(*   MUTATION = 3  capture snapshots the DISK baseline instead of the     *)
(*                 applied draft — checkpoint-as-save; must die by        *)
(*                 SaveRetiresOnlySuperseded                              *)
(*   MUTATION = 4  restore writes the checkpoint bytes back to the        *)
(*                 source path, clobbering/recreating disk content;       *)
(*                 must die by RestoreNeverWritesDisk                     *)
(*   MUTATION = 5  a successful publication marks the published drafts    *)
(*                 clean; must die by CheckpointNeverCleans               *)
(*   MUTATION = 6  eligibility ignores remote consent; must die by        *)
(*                 RemoteConsentGated                                     *)
(*                                                                       *)
(* Not modeled: record byte budgets (16 MiB demotion is a per-record      *)
(* report, not a safety boundary), the read/load path (the durable store  *)
(* is the source of truth; loads are plain reads), framing/serialization, *)
(* and liveness (a wedged worker is a progress gap, never a safety        *)
(* violation). The store's atomic rename is the assumed linearization     *)
(* point of PublishOk; a crash mid-publish IS PublishFail. Witness        *)
(* counters are sticky booleans and environment budgets are single-shot   *)
(* by design: one occurrence per behavior suffices for coverage, and      *)
(* counters would multiply the reachable state space without changing     *)
(* the protocol.                                                          *)
(***************************************************************************)
EXTENDS Integers, FiniteSets, TLC

CONSTANTS DOCS,        \* open documents
          REMOTE,      \* \subseteq DOCS: drafts behind a remote namespace
          REV_MAX,     \* revision budget per clock
          COHORT_MAX,  \* cohort budget
          MUTATION     \* 0 = honest; faulty variants above

VARIABLES persistent,  \* policy+mode+state-dir allow persistence (fixed)
          applied,     \* the live draft revision per document
          inc,         \* binding incarnation per document (crash/reopen
                       \*   changes it — the AR04 binding epoch; same
                       \*   revision in two incarnations is not identity)
          dirty,       \* unsaved-draft flag per document
          diskRev,     \* what the source on disk contains
          diskExists,  \* whether the source still exists
          consent,     \* explicit session consent for remote drafts
          cohort,      \* capture counter (cohort identity)
          capturedSet, \* documents the captured checkpoint covers
          capturedRev, \* their revisions at capture time
          capturedInc, \* their incarnations at capture time
          snapSet,     \* cohort snapshots, live only while in flight/queued
          snapRev,
          snapInc,
          snapConsent, \* consent in force when the cohort was captured
          inFlight,    \* cohort being published (0 = none)
          queued,      \* next cohort awaiting the worker (0 = none)
          durableSet,  \* the durable store: records present
          durableRev,  \*   their captured revisions
          durableIncOf, \*  their binding incarnations
          durableCohortOf, \* the cohort each durable record belongs to
          durableCohort, \* the store's cohort id
          durableObs,  \* source observation taken at publish time
          lastPubSet,  \* the image of the last COMPLETED publication
          lastPubRev,
          crashed,     \* one crash/restart is modeled
          restoreDone, \* one restore is modeled
          extChanged,  \* one external source edit is modeled
          extDeleted,  \* one external source deletion is modeled
          everFail,    \* sticky witnesses: a publication failure recovered
          everConflict,   \* a restore against a changed/missing source
          everCrashRestore, \* a restore after a crash
          everConsentRevoke, \* a grant-then-revoke cycle
          everQueueReplace, \* a queued publication superseded unwritten
          everRetired,      \* a durable record retired by republication
          everDiscard,      \* a deliberate discard
          restoreWroteDisk, \* restore-to-disk events (always FALSE)
          checkpointCleaned   \* checkpoint-cleared-dirty events (always FALSE)

EligibleWith(dirtyF, consentF, d) ==
    dirtyF[d] /\ (d \notin REMOTE \/ consentF \/ MUTATION = 6)

Eligible(d) == EligibleWith(dirty, consent, d)

\* The captured clock matches the live eligible set exactly.
CapturedMatchesCurrent ==
    /\ \A d \in DOCS : (d \in capturedSet) = Eligible(d)
    /\ \A d \in capturedSet : capturedRev[d] = applied[d]
                              /\ capturedInc[d] = inc[d]

\* Nothing is publishable and nothing is stale.
Settled ==
    /\ persistent
    /\ inFlight = 0
    /\ queued = 0
    /\ CapturedMatchesCurrent


\* Clear a finished cohort's snapshot slot: slot content lives only
\* while its cohort is in flight or queued.
ClearSnap(c) ==
    [snapSet EXCEPT ![c] = {}]

ClearSnapRev(c) ==
    [snapRev EXCEPT ![c] = [d \in DOCS |-> 0]]

ClearSnapInc(c) ==
    [snapInc EXCEPT ![c] = [d \in DOCS |-> 0]]

ClearSnapConsent(c) ==
    [snapConsent EXCEPT ![c] = FALSE]

\* The synchronous capture tail of an edit/save/consent event
\* (recovery_after_event inside the handler): if the post-event state is
\* stale against the captured clock, a new cohort captures the post-event
\* image and joins the bounded queue (in flight, else queued — the newest
\* replaces the unwritten queued one). `postDirty`, `postApplied` and
\* `postConsent` are the event's own next-state values.
CaptureTail(postDirty, postApplied, postConsent) ==
    LET postSet == {d \in DOCS :
                    EligibleWith(postDirty, postConsent, d)}
        postStale ==
            \/ \E d \in DOCS : (d \in capturedSet) # (d \in postSet)
            \/ \E d \in capturedSet : capturedRev[d] # postApplied[d]
                                      \/ capturedInc[d] # inc[d]
        c == cohort + 1
    IN IF postStale /\ persistent /\ cohort < COHORT_MAX
       THEN /\ cohort' = c
            /\ capturedSet' = postSet
            \* The captured clock always records the applied revision
            \* (staleness stays honest); MUTATION 3 snapshots the DISK
            \* baseline into the cohort instead of the applied draft —
            \* checkpoint-as-save.
            /\ capturedRev' = postApplied
            /\ capturedInc' = inc
            /\ snapSet' = [snapSet EXCEPT ![c] = postSet]
            /\ snapRev' = [snapRev EXCEPT ![c] =
                           IF MUTATION = 3 THEN diskRev ELSE postApplied]
            /\ snapInc' = [snapInc EXCEPT ![c] = inc]
            /\ snapConsent' = [snapConsent EXCEPT ![c] = postConsent]
            /\ IF inFlight = 0
               THEN /\ inFlight' = c
                    /\ queued' = queued
                    /\ everQueueReplace' = everQueueReplace
               ELSE /\ inFlight' = inFlight
                    /\ queued' = c
                    /\ everQueueReplace' = IF queued = 0
                                           THEN everQueueReplace
                                           ELSE TRUE
       ELSE UNCHANGED <<cohort, capturedSet, capturedRev, capturedInc,
                        snapSet, snapRev, snapInc, snapConsent, inFlight,
                        queued, everQueueReplace>>

TypeOK ==
    /\ persistent \in BOOLEAN
    /\ applied \in [DOCS -> 0..REV_MAX]
    /\ inc \in [DOCS -> 0..1]
    /\ dirty \in [DOCS -> BOOLEAN]
    /\ diskRev \in [DOCS -> 0..REV_MAX]
    /\ diskExists \in [DOCS -> BOOLEAN]
    /\ consent \in BOOLEAN
    /\ cohort \in 0..COHORT_MAX
    /\ capturedSet \in SUBSET DOCS
    /\ capturedRev \in [DOCS -> 0..REV_MAX]
    /\ capturedInc \in [DOCS -> 0..1]
    /\ snapSet \in [1..COHORT_MAX -> SUBSET DOCS]
    /\ snapRev \in [1..COHORT_MAX -> [DOCS -> 0..REV_MAX]]
    /\ snapInc \in [1..COHORT_MAX -> [DOCS -> 0..1]]
    /\ snapConsent \in [1..COHORT_MAX -> BOOLEAN]
    /\ inFlight \in 0..COHORT_MAX
    /\ queued \in 0..COHORT_MAX
    /\ durableSet \in SUBSET DOCS
    /\ durableRev \in [DOCS -> 0..REV_MAX]
    /\ durableIncOf \in [DOCS -> 0..1]
    /\ durableCohortOf \in [DOCS -> 0..COHORT_MAX]
    /\ durableCohort \in 0..COHORT_MAX
    /\ durableObs \in [DOCS -> [rev: 0..REV_MAX, exists: BOOLEAN,
                                consent: BOOLEAN]]
    /\ lastPubSet \in SUBSET DOCS
    /\ lastPubRev \in [DOCS -> 0..REV_MAX]
    /\ crashed \in BOOLEAN
    /\ restoreDone \in BOOLEAN
    /\ extChanged \in BOOLEAN
    /\ extDeleted \in BOOLEAN
    /\ everFail \in BOOLEAN
    /\ everConflict \in BOOLEAN
    /\ everCrashRestore \in BOOLEAN
    /\ everConsentRevoke \in BOOLEAN
    /\ everQueueReplace \in BOOLEAN
    /\ everRetired \in BOOLEAN
    /\ everDiscard \in BOOLEAN
    /\ restoreWroteDisk \in BOOLEAN
    /\ checkpointCleaned \in BOOLEAN

Init ==
    \* Policy is a session-fixed fact; TLC explores both products.
    /\ persistent \in BOOLEAN
    /\ applied = [d \in DOCS |-> 0]
    /\ inc = [d \in DOCS |-> 0]
    /\ dirty = [d \in DOCS |-> FALSE]
    /\ diskRev = [d \in DOCS |-> 0]
    /\ diskExists = [d \in DOCS |-> TRUE]
    /\ consent = FALSE
    /\ cohort = 0
    /\ capturedSet = {}
    /\ capturedRev = [d \in DOCS |-> 0]
    /\ capturedInc = [d \in DOCS |-> 0]
    /\ snapSet = [c \in 1..COHORT_MAX |-> {}]
    /\ snapRev = [c \in 1..COHORT_MAX |-> [d \in DOCS |-> 0]]
    /\ snapInc = [c \in 1..COHORT_MAX |-> [d \in DOCS |-> 0]]
    /\ snapConsent = [c \in 1..COHORT_MAX |-> FALSE]
    /\ inFlight = 0
    /\ queued = 0
    /\ durableSet = {}
    /\ durableRev = [d \in DOCS |-> 0]
    /\ durableIncOf = [d \in DOCS |-> 0]
    /\ durableCohortOf = [d \in DOCS |-> 0]
    /\ durableCohort = 0
    /\ durableObs = [d \in DOCS |-> [rev |-> 0, exists |-> TRUE,
                                     consent |-> FALSE]]
    /\ lastPubSet = {}
    /\ lastPubRev = [d \in DOCS |-> 0]
    /\ crashed = FALSE
    /\ restoreDone = FALSE
    /\ extChanged = FALSE
    /\ extDeleted = FALSE
    /\ everFail = FALSE
    /\ everConflict = FALSE
    /\ everCrashRestore = FALSE
    /\ everConsentRevoke = FALSE
    /\ everQueueReplace = FALSE
    /\ everRetired = FALSE
    /\ everDiscard = FALSE
    /\ restoreWroteDisk = FALSE
    /\ checkpointCleaned = FALSE

\* A draft edit: the applied clock moves; the other two do not. The
\* handler's synchronous capture tail runs with the post-edit image.
Edit(d) ==
    /\ applied[d] < REV_MAX
    /\ applied' = [applied EXCEPT ![d] = @ + 1]
    /\ dirty' = [dirty EXCEPT ![d] = TRUE]
    /\ CaptureTail(dirty', applied', consent)
    /\ UNCHANGED <<persistent, inc, diskRev, diskExists, consent,
                   durableSet, durableRev, durableIncOf, durableCohortOf,
                   durableCohort, durableObs, lastPubSet, lastPubRev, crashed,
                   restoreDone, extChanged, extDeleted, everFail,
                   everConflict, everCrashRestore, everConsentRevoke,
                   everRetired, everDiscard, restoreWroteDisk,
                   checkpointCleaned>>

\* The environment edits the source behind the editor's back (not an
\* editor event: no capture tail). Single-shot by design.
ExternalEdit(d) ==
    /\ diskExists[d]
    /\ ~extChanged
    /\ diskRev[d] < REV_MAX
    /\ diskRev' = [diskRev EXCEPT ![d] = @ + 1]
    /\ extChanged' = TRUE
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskExists, consent,
                   cohort, capturedSet, capturedRev, capturedInc, snapSet,
                   snapRev, snapInc, snapConsent, inFlight, queued,
                   durableSet, durableRev, durableIncOf, durableCohortOf,
                   durableCohort, durableObs, lastPubSet, lastPubRev, crashed,
                   restoreDone, extDeleted, everFail, everConflict,
                   everCrashRestore, everConsentRevoke, everQueueReplace,
                   everRetired, everDiscard, restoreWroteDisk,
                   checkpointCleaned>>

ExternalDelete(d) ==
    /\ diskExists[d]
    /\ ~extDeleted
    /\ diskExists' = [diskExists EXCEPT ![d] = FALSE]
    /\ extDeleted' = TRUE
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskRev, consent, cohort,
                   capturedSet, capturedRev, capturedInc, snapSet, snapRev,
                   snapInc, snapConsent, inFlight, queued, durableSet,
                   durableRev, durableIncOf, durableCohortOf, durableCohort,
                   durableObs, lastPubSet, lastPubRev, crashed, restoreDone,
                   extChanged, everFail, everConflict, everCrashRestore,
                   everConsentRevoke, everQueueReplace, everRetired,
                   everDiscard, restoreWroteDisk, checkpointCleaned>>

\* Granting consent makes remote drafts eligible; the handler recaptures
\* synchronously (recovery_set_remote_consent -> recovery_after_event).
ConsentGrant ==
    /\ ~consent
    /\ consent' = TRUE
    /\ CaptureTail(dirty, applied, TRUE)
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskRev, diskExists,
                   durableSet, durableRev, durableIncOf, durableCohortOf,
                   durableCohort, durableObs, lastPubSet, lastPubRev, crashed,
                   restoreDone, extChanged, extDeleted, everFail,
                   everConflict, everCrashRestore, everConsentRevoke,
                   everRetired, everDiscard, restoreWroteDisk,
                   checkpointCleaned>>

\* Revoking consent retires remote drafts from the NEXT cohort.
ConsentRevoke ==
    /\ consent
    /\ consent' = FALSE
    /\ everConsentRevoke' = TRUE
    /\ CaptureTail(dirty, applied, FALSE)
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskRev, diskExists,
                   durableSet, durableRev, durableIncOf, durableCohortOf,
                   durableCohort, durableObs, lastPubSet, lastPubRev, crashed,
                   restoreDone, extChanged, extDeleted, everFail,
                   everConflict, everCrashRestore, everRetired, everDiscard,
                   restoreWroteDisk, checkpointCleaned>>

\* A confirmed save: the disk baseline advances to the applied revision
\* and the draft turns clean. NOT a checkpoint concern: the synchronous
\* capture tail republishes the cohort without it — the save retires
\* only the checkpoint it supersedes.
Save(d) ==
    /\ dirty[d]
    /\ diskRev' = [diskRev EXCEPT ![d] = applied[d]]
    /\ diskExists' = [diskExists EXCEPT ![d] = TRUE]
    /\ dirty' = [dirty EXCEPT ![d] = FALSE]
    /\ CaptureTail(dirty', applied, consent)
    /\ UNCHANGED <<persistent, applied, inc, consent, durableSet, durableRev,
                   durableIncOf, durableCohortOf, durableCohort, durableObs,
                   lastPubSet, lastPubRev, crashed, restoreDone, extChanged,
                   extDeleted, everFail, everConflict, everCrashRestore,
                   everConsentRevoke, everRetired, everDiscard,
                   restoreWroteDisk, checkpointCleaned>>

\* The only asynchronous capture: the publication-failure rewind may
\* leave the captured clock stale; the next event recaptures.
Capture ==
    /\ persistent
    /\ ~CapturedMatchesCurrent
    /\ cohort < COHORT_MAX
    /\ CaptureTail(dirty, applied, consent)
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskRev, diskExists,
                   consent, durableSet, durableRev, durableIncOf,
                   durableCohortOf, durableCohort, durableObs, lastPubSet,
                   lastPubRev, crashed, restoreDone, extChanged, extDeleted,
                   everFail, everConflict, everCrashRestore,
                   everConsentRevoke, everRetired, everDiscard,
                   restoreWroteDisk, checkpointCleaned>>

\* Atomic all-or-nothing publication (store.rs publish_cohort): the
\* staged rename is the linearization point. The worker-side source
\* observation is taken here. The finished cohort's slot clears; the
\* queued publication starts on completion, whatever the outcome.
PublishOk ==
    /\ inFlight # 0
    /\ LET c == inFlight IN
       \* MUTATION 1: the new cohort MERGES into the old store instead of
       \* replacing it — records the cohort no longer covers keep their
       \* old cohort identity (half-cohort mixing). The last-complete
       \* bookkeeping still records the honest image, so the store stops
       \* matching it.
       /\ durableSet' = IF MUTATION = 1 THEN durableSet \cup snapSet[c]
                        ELSE snapSet[c]
       /\ durableRev' = [d \in DOCS |->
                         IF MUTATION = 1 /\ d \notin snapSet[c]
                         THEN durableRev[d] ELSE snapRev[c][d]]
       /\ durableIncOf' = [d \in DOCS |->
                           IF MUTATION = 1 /\ d \notin snapSet[c]
                           THEN durableIncOf[d] ELSE snapInc[c][d]]
       /\ durableCohortOf' = [d \in DOCS |->
                              IF MUTATION = 1 /\ d \notin snapSet[c]
                              THEN durableCohortOf[d] ELSE c]
       /\ durableCohort' = c
       /\ durableObs' = [d \in DOCS |-> [rev |-> diskRev[d],
                                         exists |-> diskExists[d],
                                         consent |-> snapConsent[c]]]
       /\ lastPubSet' = snapSet[c]
       /\ lastPubRev' = snapRev[c]
       /\ everRetired' = (everRetired \/ (\E d \in durableSet :
                                          d \notin snapSet[c]))
       /\ snapSet' = ClearSnap(c)
       /\ snapRev' = ClearSnapRev(c)
       /\ snapInc' = ClearSnapInc(c)
       /\ snapConsent' = ClearSnapConsent(c)
       /\ inFlight' = queued
       /\ queued' = 0
    \* MUTATION 5: the checkpoint marks published drafts clean.
    /\ dirty' = IF MUTATION = 5
                THEN [d \in DOCS |-> IF d \in snapSet[inFlight]
                                     THEN FALSE ELSE dirty[d]]
                ELSE dirty
    /\ checkpointCleaned' = IF MUTATION = 5 /\ snapSet[inFlight] # {}
                            THEN TRUE ELSE checkpointCleaned
    /\ UNCHANGED <<persistent, applied, inc, diskRev, diskExists, consent,
                   cohort, capturedSet, capturedRev, capturedInc, crashed,
                   restoreDone, extChanged, extDeleted, everFail,
                   everConflict, everCrashRestore, everConsentRevoke,
                   everQueueReplace, everDiscard, restoreWroteDisk>>

\* A failed/interrupted publication: the last complete checkpoint is
\* untouched, and the captured clock rewinds to the durable one so the
\* next event retries instead of believing unsaved bytes are safe.
PublishFail ==
    /\ inFlight # 0
    /\ everFail' = TRUE
    /\ capturedSet' = durableSet
    /\ capturedRev' = durableRev
    /\ capturedInc' = durableIncOf
    /\ snapSet' = ClearSnap(inFlight)
    /\ snapRev' = ClearSnapRev(inFlight)
    /\ snapInc' = ClearSnapInc(inFlight)
    /\ snapConsent' = ClearSnapConsent(inFlight)
    /\ inFlight' = queued
    /\ queued' = 0
    \* MUTATION 2: the failure takes the last complete checkpoint with it.
    /\ IF MUTATION = 2
       THEN /\ durableSet' = {}
            /\ durableCohort' = 0
       ELSE UNCHANGED <<durableSet, durableCohort>>
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskRev, diskExists,
                   consent, cohort, durableRev, durableIncOf, durableCohortOf,
                   durableObs, lastPubSet, lastPubRev, crashed, restoreDone,
                   extChanged, extDeleted, everConflict, everCrashRestore,
                   everConsentRevoke, everQueueReplace, everRetired,
                   everDiscard, restoreWroteDisk, checkpointCleaned>>

\* SIGKILL/power loss and restart: volatile state dies with the session;
\* the durable store persists. Documents reopen from DISK as a NEW
\* binding incarnation; unsaved bytes beyond the last durable checkpoint
\* are honestly lost. Consent is a per-session grant and resets.
CrashRestart ==
    /\ ~crashed
    /\ applied' = [d \in DOCS |-> diskRev[d]]
    /\ inc' = [d \in DOCS |-> inc[d] + 1]
    /\ dirty' = [d \in DOCS |-> FALSE]
    /\ capturedSet' = {}
    /\ capturedRev' = [d \in DOCS |-> 0]
    /\ capturedInc' = [d \in DOCS |-> 0]
    /\ cohort' = 0
    /\ inFlight' = 0
    /\ queued' = 0
    /\ consent' = FALSE
    /\ crashed' = TRUE
    /\ snapSet' = [c \in 1..COHORT_MAX |-> {}]
    /\ snapRev' = [c \in 1..COHORT_MAX |-> [d \in DOCS |-> 0]]
    /\ snapInc' = [c \in 1..COHORT_MAX |-> [d \in DOCS |-> 0]]
    /\ snapConsent' = [c \in 1..COHORT_MAX |-> FALSE]
    /\ UNCHANGED <<persistent, diskRev, diskExists, durableSet, durableRev,
                   durableIncOf, durableCohortOf, durableCohort, durableObs,
                   lastPubSet, lastPubRev, restoreDone, extChanged, extDeleted,
                   everFail, everConflict, everCrashRestore,
                   everConsentRevoke, everQueueReplace, everRetired,
                   everDiscard, restoreWroteDisk, checkpointCleaned>>

\* Restore lands in a checked pathless dirty draft. The disk is never
\* written: no overwrite of changed content, no recreation of a deleted
\* target, no re-granted authority. The sticky flags record the
\* interesting journeys (conflict/missing source, post-crash) as
\* witnesses.
Restore(d) ==
    /\ ~restoreDone
    /\ d \in durableSet
    /\ restoreDone' = TRUE
    /\ everConflict' = IF ~diskExists[d] \/ diskRev[d] # durableObs[d].rev
                       THEN TRUE ELSE everConflict
    /\ everCrashRestore' = IF crashed THEN TRUE ELSE everCrashRestore
    \* MUTATION 4: restore writes the checkpoint bytes back to disk.
    /\ IF MUTATION = 4
       THEN /\ diskRev' = [diskRev EXCEPT ![d] = durableRev[d]]
            /\ diskExists' = [diskExists EXCEPT ![d] = TRUE]
            /\ restoreWroteDisk' = TRUE
       ELSE UNCHANGED <<diskRev, diskExists, restoreWroteDisk>>
    /\ UNCHANGED <<persistent, applied, inc, dirty, consent, cohort,
                   capturedSet, capturedRev, capturedInc, snapSet, snapRev,
                   snapInc, snapConsent, inFlight, queued, durableSet,
                   durableRev, durableIncOf, durableCohortOf, durableCohort,
                   durableObs, lastPubSet, lastPubRev, crashed, extChanged,
                   extDeleted, everFail, everConsentRevoke, everQueueReplace,
                   everRetired, everDiscard, checkpointCleaned>>

\* A deliberate discard: republish the cohort without the record
\* (store.rs discard). The updated durable image IS the new last
\* complete publication.
Discard(d) ==
    /\ persistent
    /\ ~everDiscard
    /\ d \in durableSet
    /\ everDiscard' = TRUE
    /\ LET kept == durableSet \ {d} IN
       /\ durableSet' = kept
       /\ lastPubSet' = kept
    /\ capturedSet' = capturedSet \ {d}
    /\ UNCHANGED <<persistent, applied, inc, dirty, diskRev, diskExists,
                   consent, cohort, capturedRev, capturedInc, snapSet,
                   snapRev, snapInc, snapConsent, inFlight, queued,
                   durableRev, durableIncOf, durableCohortOf, durableCohort,
                   durableObs, lastPubRev, crashed, restoreDone, extChanged,
                   extDeleted, everFail, everConflict, everCrashRestore,
                   everConsentRevoke, everQueueReplace, everRetired,
                   restoreWroteDisk, checkpointCleaned>>

Next ==
    \/ \E d \in DOCS : Edit(d)
    \/ \E d \in DOCS : ExternalEdit(d)
    \/ \E d \in DOCS : ExternalDelete(d)
    \/ ConsentGrant
    \/ ConsentRevoke
    \/ \E d \in DOCS : Save(d)
    \/ Capture
    \/ PublishOk
    \/ PublishFail
    \/ CrashRestart
    \/ \E d \in DOCS : Restore(d)
    \/ \E d \in DOCS : Discard(d)

vars == <<persistent, applied, inc, dirty, diskRev, diskExists, consent,
          cohort, capturedSet, capturedRev, capturedInc, snapSet, snapRev,
          snapInc, snapConsent, inFlight, queued, durableSet, durableRev,
          durableIncOf, durableCohortOf, durableCohort, durableObs,
          lastPubSet, lastPubRev, crashed, restoreDone, extChanged,
          extDeleted, everFail, everConflict, everCrashRestore,
          everConsentRevoke, everQueueReplace, everRetired, everDiscard,
          restoreWroteDisk, checkpointCleaned>>

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* VF13 named safety properties.                                          *)
(***************************************************************************)

\* No half-cohort mixing: every durable record belongs to one cohort.
CohortCoherent ==
    \A d1, d2 \in durableSet : durableCohortOf[d1] = durableCohortOf[d2]

\* Last-complete-checkpoint retention: the durable store is exactly the
\* image of the last COMPLETED publication — a failed or interrupted
\* publication (or a crash midway) changes nothing.
DurableMatchesLastComplete ==
    /\ durableSet = lastPubSet
    /\ \A d \in durableSet : durableRev[d] = lastPubRev[d]

\* A confirmed save retires only the checkpoint it supersedes: once the
\* pipeline settles, every eligible dirty draft is durable at its
\* CURRENT applied revision and incarnation — edits after the saved
\* snapshot are never stranded by the save's retirement.
SaveRetiresOnlySuperseded ==
    Settled =>
        \A d \in DOCS : Eligible(d) =>
            /\ d \in durableSet
            /\ durableRev[d] = applied[d]
            /\ durableIncOf[d] = inc[d]

\* Checkpointing is not Save: no capture/publish step cleared a draft.
CheckpointNeverCleans == ~checkpointCleaned

\* Remote drafts persist only under explicit session consent — the grant
\* in force at capture is stamped into the durable record (surviving
\* restarts, unlike the volatile per-cohort snapshot metadata).
RemoteConsentGated ==
    \A d \in durableSet \cap REMOTE : durableObs[d].consent

\* Restore never clobbers or recreates disk content.
RestoreNeverWritesDisk == ~restoreWroteDisk

(***************************************************************************)
(* Non-vacuity witnesses: each must be REACHABLE in the honest model      *)
(* (checked as an expected-to-fail invariant over the coverage config).   *)
(***************************************************************************)

\* A durable cohort is published at all.
WitnessNoDurable == durableSet = {}

\* A cohort covering more than one draft publishes (multi-document
\* coherence is exercised, not vacuous).
WitnessNoMultiDocCohort == Cardinality(durableSet) < 2

\* A publication fails and the pipeline continues afterwards.
WitnessNoFailedPublish == ~everFail

\* A restore happens against a changed or missing source.
WitnessNoConflictRestore == ~everConflict

\* A crash/restart is followed by a restore of the durable checkpoint.
WitnessNoCrashRestore == ~everCrashRestore

\* Consent is granted and revoked again.
WitnessNoConsentCycle == ~everConsentRevoke

\* The memory-only product is exercised: a session with persistence off
\* carries a dirty draft (the property holds in the initial state of the
\* persistent product only — an init-state violation cannot count as
\* reached, so the witness requires a real memory-only edit).
WitnessNoMemoryOnly == persistent \/ ~(\E d \in DOCS : dirty[d])

\* A queued publication is replaced unwritten by a newer capture.
WitnessNoQueueReplace == ~everQueueReplace

\* A republication retires a durable record (save superseding it, or
\* consent revocation excluding it).
WitnessNoRetirement == ~everRetired

\* A deliberate discard republishes without a record.
WitnessNoDiscard == ~everDiscard


\* State constraint (used by every config): the cohort/revision budgets
\* are model artifacts, not protocol states. Production's u64 cohort
\* counter always recaptures a stale pipeline; here a spent budget would
\* freeze staleness forever, so those truncated states are excluded —
\* the checks above cover every behavior UP TO the budget. A mutant may
\* not rely on a pruned state to survive: the gate requires each mutant
\* to actually die.
CohortBudgetHonest == cohort < COHORT_MAX \/ CapturedMatchesCurrent

=============================================================================
