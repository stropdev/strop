---- MODULE Notify ----
(***************************************************************************)
(* Filesystem-notification lifecycle safety (0058 §2, the 2026-09-14 and  *)
(* 2026-09-16 amendments): subscription identity and lifetimes, loss and  *)
(* partial coverage, reconcile ordering, bounded flow and document        *)
(* publication, modeled over the wire family landed in                    *)
(* crates/strop-worker-protocol — request.rs: Request::Subscribe /        *)
(* Unsubscribe, Event::Notify / NotifyOverflow / ReconcileBoundary,       *)
(* NotifyKind::Ambiguous; message.rs: NotifyCoverage; id.rs:              *)
(* Subscription{id, generation}, Session{incarnation, lease}; guard.rs:   *)
(* Authority::admit / admit_subscription -> Refusal::StaleSubscription,   *)
(* WrongIncarnation.                                                      *)
(*                                                                       *)
(* OS/filesystem assumptions (named, per the amendment):                  *)
(*   - the worker beside the files owns native watching in the actual     *)
(*     selected Linux namespace (inotify-class); the native queue is      *)
(*     BOUNDED and signals overflow rather than dropping silently; watch  *)
(*     registration can fail per scope; some filesystems admit only       *)
(*     polling or on-demand coverage and some none at all;                *)
(*   - rename decisiveness is best-effort: cookie pairing can fail, so    *)
(*     an Ambiguous hint is a normal outcome, reconciled by observation   *)
(*     (parent/name coverage is folded in: replacement and rename both    *)
(*     surface as hints on the subscribed scope);                         *)
(*   - event delivery may duplicate and reorder; control messages         *)
(*     (refusals, overflow, the reconcile boundary, unsubscribe)          *)
(*     progress on their own lane under data pressure;                    *)
(*   - the editor owns document application: worker events are hints.     *)
(*                                                                       *)
(* Editor-side correspondence points: crates/strop-engine                 *)
(* editor/filesystem/reconcile.rs (the guarded-reload surface:            *)
(* admission/receipt vocabulary, dirty buffers never clobbered),          *)
(* strop-core buffer/io.rs adopt_file_binding (a clean reload adopts the  *)
(* observed binding), buffer.rs's save-refuses-external-change (the       *)
(* dirty buffer's external-change state) and editor/directory/filter.rs's *)
(* revision re-check at task completion (a stale reload is dropped        *)
(* against the expected document/revision).                               *)
(*                                                                       *)
(*   IDENTITY  a subscription's authority is (client session incarnation, *)
(*             worker incarnation, scope, generation); reinstallation     *)
(*             after loss/overflow re-enters with a bumped generation,    *)
(*             so events stamped by a superseded identity — a reused      *)
(*             descriptor/inode, an old worker incarnation — are dropped, *)
(*             never acted on (guard.rs admit_subscription).              *)
(*                                                                       *)
(*   HINTS     a delivered hint only RECORDS an invalidation; it is       *)
(*             never an authoritative edit, save receipt or write log.    *)
(*             Reconciliation is by observation (ReloadStart), and a      *)
(*             confirmed own-save receipt is never erased by a late       *)
(*             event: publication only ever installs an observation at    *)
(*             or beyond the buffer's baseline.                           *)
(*                                                                       *)
(*   BOUNDARY  subscription installation, the initial scan and later      *)
(*             events have a defined reconcile boundary: invalidations    *)
(*             recorded before the scan's snapshot are covered by it;     *)
(*             invalidations recorded after the snapshot survive scan     *)
(*             completion.                                                *)
(*                                                                       *)
(*   LOSS      overflow, registration refusal, disconnect and reported    *)
(*             excluded subtrees are explicit outcomes: the baseline is   *)
(*             invalidated, a conservative rescan obligation recorded,    *)
(*             and coverage is reestablished plus reobserved before       *)
(*             freshness is claimed again. Coalescing into the bounded    *)
(*             queue never silently drops the only staleness sign.        *)
(*                                                                       *)
(*   PUBLICATION  a clean-buffer reload installs only an observation at   *)
(*             or beyond the current baseline for the expected document;  *)
(*             a dirty buffer is preserved and receives external-change   *)
(*             state; a stale in-flight reload never clears a newer       *)
(*             baseline. An ambiguous rename never relocates a document   *)
(*             binding — only an observation-confirmed move does.         *)
(*                                                                       *)
(*   COVERAGE  coverage is reported honestly per namespace: push          *)
(*             (native/polling), on-demand, or a typed refusal for        *)
(*             unsupported; a periodic full-tree crawl is never an        *)
(*             invisible substitute.                                      *)
(*                                                                       *)
(* Model scale (the amendment's required cases): two clients, a worker    *)
(* that restarts once, reordered and duplicate observations, overflow     *)
(* during a rescan window, save versus external write, dirty-buffer       *)
(* races.                                                                 *)
(*                                                                       *)
(* THE invariants (the amendment's named safety properties):              *)
(*                                                                       *)
(*   TypeOK                    every variable stays in its declared       *)
(*                             finite domain                              *)
(*   HintsNeverAuthority       no hint was ever applied as an             *)
(*                             authoritative edit/receipt                 *)
(*   StaleIdentityNeverActs    no stale-identity event was acted on       *)
(*   AmbiguousNeverRelocates   no binding relocation without an           *)
(*                             observation-confirmed move                 *)
(*   FreshRequiresCoverage     a freshness claim implies established      *)
(*                             coverage, a valid baseline, no rescan      *)
(*                             obligation, no pending invalidation and    *)
(*                             no reload in flight                        *)
(*   CoalesceNeverLosesStaleness  a full queue recorded overflow, never   *)
(*                             a silent drop                              *)
(*   ScanNeverErasesNewer      scan completion never erased a newer       *)
(*                             invalidation                               *)
(*   DirtyNeverClobbered       no reload applied over a dirty buffer      *)
(*   StaleReloadNeverClears    no stale snapshot published over a newer   *)
(*                             baseline                                   *)
(*   CoverageHonest            no invisible full crawl substituted for    *)
(*                             native coverage                            *)
(*   BoundedQueue              the native-to-service queue stays within   *)
(*                             its bound                                  *)
(*                                                                       *)
(* Deliberately faulty variants (configs flip MUTATION):                  *)
(*   MUTATION = 1  a hint is applied as authoritative content (no         *)
(*                 observation); must die by HintsNeverAuthority          *)
(*   MUTATION = 2  a stale-identity event (superseded generation/         *)
(*                 incarnation, reused descriptor) is acted on; must die  *)
(*                 by StaleIdentityNeverActs                              *)
(*   MUTATION = 3  an ambiguous rename relocates the document binding     *)
(*                 from the guessed pair; must die by                     *)
(*                 AmbiguousNeverRelocates                                *)
(*   MUTATION = 4  freshness is claimed while coverage is lost/degraded   *)
(*                 or invalidations are pending; must die by              *)
(*                 FreshRequiresCoverage                                  *)
(*   MUTATION = 5  a full queue drops the event without recording         *)
(*                 overflow; must die by CoalesceNeverLosesStaleness      *)
(*   MUTATION = 6  scan completion erases ALL pending invalidations,      *)
(*                 including newer ones; must die by ScanNeverErasesNewer *)
(*   MUTATION = 7  a reload applies over a dirty buffer; must die by      *)
(*                 DirtyNeverClobbered                                    *)
(*   MUTATION = 8  a stale snapshot publishes over a newer baseline;      *)
(*                 must die by StaleReloadNeverClears                     *)
(*   MUTATION = 9  an unsupported namespace is served by an invisible     *)
(*                 full crawl posing as native coverage; must die by      *)
(*                 CoverageHonest                                         *)
(*                                                                       *)
(* Not modeled: path bytes and scope shape (one scope + one document per  *)
(* client represent the identity classes), listing/catalog content, the   *)
(* event sequence numbers themselves (an invalidation's only ordering     *)
(* content is whether the scan's snapshot covers it, so sequences are    *)
(* folded into covered/newer bits), the polling fallback's interval       *)
(* (polling delivers hints like native; the honesty is in the report,     *)
(* not the mechanism), byte budgets (LspWire owns queue-byte accounting), *)
(* namespace identity change (Recovery/RemoteWorkspace own session        *)
(* admission), and liveness.                                              *)
(***************************************************************************)
EXTENDS Integers, FiniteSets, TLC

CONSTANTS CLIENTS,     \* editor clients (two: identity separation)
          INC_MAX,     \* worker incarnation budget (a restart bumps it)
          GEN_MAX,     \* subscription generation budget per client scope
          REV_MAX,     \* disk content version budget per client document
          QMAX,        \* native-to-service queue bound (one coalescing slot)
          MUTATION     \* 0 = honest; faulty variants above

VARIABLES incarnation,  \* worker process incarnation (restart bumps)
          st,           \* per client: none|idle|scanning|established|degraded|lost
          cov,          \* per client: none|push|onDemand (the honest report)
          gen,          \* per client: current subscription generation
          excl,         \* per client: the subscription reported excluded subtrees
          boundaryPending, \* per client: scan snapshot taken, boundary not yet applied
          pendInv,      \* per client: recorded unreconciled invalidation
          pendNew,      \* per client: invalidation recorded AFTER the scan snapshot
          queued,       \* per client: an undelivered hint sits in the bounded slot
          staleEv,      \* per client: a stale-identity event is in flight
          rescanOwed,   \* per client: conservative rescan obligation (loss/overflow)
          rescanReload, \* per client: the in-flight reload IS the owed rescan
          fresh,        \* per client: the client currently claims freshness
          dirty,        \* per client: the buffer holds unsaved edits
          loadedRev,    \* per client: disk version the buffer content matches
          diskRev,      \* per client: actual disk content version
          extChg,       \* per client: dirty buffer carries external-change state
          renameSt,     \* per client: none|ambiguous|observed|relocated
          inFlight,     \* per client: {} or {v}: the one outstanding reload's snapshot
          authoritativeApplies, \* hints applied as authority (always 0)
          staleActs,    \* stale-identity events acted on (always 0)
          guessedRelocations, \* bindings relocated off a guessed pair (always 0)
          silentDrops,  \* queue-full drops without recorded overflow (always 0)
          boundaryErasures, \* scan completions erasing newer invalidations (always 0)
          dirtyClobbers, \* reloads applied over a dirty buffer (always 0)
          staleReloads, \* stale snapshots published over a newer baseline (always 0)
          invisibleCrawls, \* unsupported namespaces served by a hidden crawl (always 0)
          rescans,      \* rescan obligations discharged by reobservation (witness)
          staleDrops,   \* stale-identity events dropped (witness)
          dupDeliveries, \* duplicate/late redeliveries absorbed (witness)
          boundaryWithNewer, \* boundaries applied while a newer invalidation survived (witness)
          unsupportedRefusals, \* typed refusals of unsupported namespaces (witness)
          controlUnderPressure, \* unsubscribes with a full data queue (witness)
          saveRaces     \* saves committed with pre-save hints outstanding (witness)

\* Variable groups, so actions can leave whole facets untouched.
identityVars == <<st, cov, gen, excl>>
lossVars == <<pendInv, pendNew, queued, staleEv, rescanOwed, rescanReload,
              fresh>>
docVars == <<dirty, loadedRev, diskRev, extChg, renameSt, inFlight>>
faultVars == <<authoritativeApplies, staleActs, guessedRelocations,
               silentDrops, boundaryErasures, dirtyClobbers, staleReloads,
               invisibleCrawls>>
witVars == <<rescans, staleDrops, dupDeliveries, boundaryWithNewer,
             unsupportedRefusals, controlUnderPressure, saveRaces>>

vars == <<incarnation, st, cov, gen, excl, boundaryPending, pendInv,
          pendNew, queued, staleEv, rescanOwed, rescanReload, fresh,
          dirty, loadedRev, diskRev, extChg, renameSt, inFlight,
          authoritativeApplies, staleActs, guessedRelocations, silentDrops,
          boundaryErasures, dirtyClobbers, staleReloads, invisibleCrawls,
          rescans, staleDrops, dupDeliveries, boundaryWithNewer,
          unsupportedRefusals, controlUnderPressure, saveRaces>>

TypeOK ==
    /\ incarnation \in 1..INC_MAX
    /\ st \in [CLIENTS -> {"none", "idle", "scanning", "established",
                           "degraded", "lost"}]
    /\ cov \in [CLIENTS -> {"none", "push", "onDemand"}]
    /\ gen \in [CLIENTS -> 0..GEN_MAX]
    /\ excl \in [CLIENTS -> BOOLEAN]
    /\ boundaryPending \in [CLIENTS -> BOOLEAN]
    /\ pendInv \in [CLIENTS -> BOOLEAN]
    /\ pendNew \in [CLIENTS -> BOOLEAN]
    /\ queued \in [CLIENTS -> BOOLEAN]
    /\ staleEv \in [CLIENTS -> BOOLEAN]
    /\ rescanOwed \in [CLIENTS -> BOOLEAN]
    /\ rescanReload \in [CLIENTS -> BOOLEAN]
    /\ fresh \in [CLIENTS -> BOOLEAN]
    /\ dirty \in [CLIENTS -> BOOLEAN]
    /\ loadedRev \in [CLIENTS -> 0..REV_MAX]
    /\ diskRev \in [CLIENTS -> 0..REV_MAX]
    /\ extChg \in [CLIENTS -> BOOLEAN]
    /\ renameSt \in [CLIENTS -> {"none", "ambiguous", "observed", "relocated"}]
    /\ inFlight \in [CLIENTS -> SUBSET (0..REV_MAX)]
    /\ authoritativeApplies \in 0..1
    /\ staleActs \in 0..1
    /\ guessedRelocations \in 0..1
    /\ silentDrops \in 0..1
    /\ boundaryErasures \in 0..1
    /\ dirtyClobbers \in 0..1
    /\ staleReloads \in 0..1
    /\ invisibleCrawls \in 0..1
    /\ rescans \in 0..1
    /\ staleDrops \in 0..1
    /\ dupDeliveries \in 0..1
    /\ boundaryWithNewer \in 0..1
    /\ unsupportedRefusals \in 0..1
    /\ controlUnderPressure \in 0..1
    /\ saveRaces \in 0..1

Init ==
    /\ incarnation = 1
    /\ st = [c \in CLIENTS |-> "none"]
    /\ cov = [c \in CLIENTS |-> "none"]
    /\ gen = [c \in CLIENTS |-> 0]
    /\ excl = [c \in CLIENTS |-> FALSE]
    /\ boundaryPending = [c \in CLIENTS |-> FALSE]
    /\ pendInv = [c \in CLIENTS |-> FALSE]
    /\ pendNew = [c \in CLIENTS |-> FALSE]
    /\ queued = [c \in CLIENTS |-> FALSE]
    /\ staleEv = [c \in CLIENTS |-> FALSE]
    /\ rescanOwed = [c \in CLIENTS |-> FALSE]
    /\ rescanReload = [c \in CLIENTS |-> FALSE]
    /\ fresh = [c \in CLIENTS |-> FALSE]
    /\ dirty = [c \in CLIENTS |-> FALSE]
    /\ loadedRev = [c \in CLIENTS |-> 0]
    /\ diskRev = [c \in CLIENTS |-> 0]
    /\ extChg = [c \in CLIENTS |-> FALSE]
    /\ renameSt = [c \in CLIENTS |-> "none"]
    /\ inFlight = [c \in CLIENTS |-> {}]
    /\ authoritativeApplies = 0
    /\ staleActs = 0
    /\ guessedRelocations = 0
    /\ silentDrops = 0
    /\ boundaryErasures = 0
    /\ dirtyClobbers = 0
    /\ staleReloads = 0
    /\ invisibleCrawls = 0
    /\ rescans = 0
    /\ staleDrops = 0
    /\ dupDeliveries = 0
    /\ boundaryWithNewer = 0
    /\ unsupportedRefusals = 0
    /\ controlUnderPressure = 0
    /\ saveRaces = 0

\* Push coverage is live: hints flow for this subscription.
PushLive(c) == st[c] \in {"scanning", "established", "degraded"}
               /\ cov[c] = "push"

\* The client holds a subscription (any coverage).
Subscribed(c) == st[c] \in {"scanning", "established", "degraded"}

\* The client is connected (idle or subscribed).
Connected(c) == st[c] \in {"idle", "scanning", "established", "degraded"}

(***************************************************************************)
(* Connection and subscription lifecycle.                                  *)
(***************************************************************************)

\* The client handshakes: the connection binds the CURRENT worker
\* incarnation (id.rs Session{incarnation, lease}; the lease is folded
\* into the incarnation — both are fresh per worker process).
Connect(c) ==
    /\ st[c] \in {"none", "lost"}
    /\ st' = [st EXCEPT ![c] = "idle"]
    /\ UNCHANGED <<incarnation, cov, gen, excl, boundaryPending, lossVars,
                   docVars, faultVars, witVars>>

\* Subscribe over the logical scope on a push-capable namespace: the
\* worker registers the watch, bumps the generation and starts the
\* initial scan. Reused descriptors/inodes are folded in: the wire id
\* pool is finite and reused, so all discrimination rests on the
\* generation — which is exactly what events name. Excluded subtrees,
\* when present, are reported in the result.
SubscribePush(c) ==
    /\ st[c] = "idle"
    /\ gen[c] < GEN_MAX
    /\ \E ex \in BOOLEAN :
        /\ st' = [st EXCEPT ![c] = "scanning"]
        /\ cov' = [cov EXCEPT ![c] = "push"]
        /\ excl' = [excl EXCEPT ![c] = ex]
        /\ gen' = [gen EXCEPT ![c] = @ + 1]
        /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
        /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
        /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
        /\ queued' = [queued EXCEPT ![c] = FALSE]
        /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    \* A rescan obligation survives reinstallation: reestablished coverage
    \* still must reobserve before freshness. Stale events from the
    \* superseded identity stay in flight.
    /\ UNCHANGED <<incarnation, staleEv, rescanOwed, rescanReload, docVars,
                   faultVars, witVars>>

\* On-demand coverage: no push; freshness comes from explicit
\* observation. Honestly reported; established without a scan.
SubscribeOnDemand(c) ==
    /\ st[c] = "idle"
    /\ gen[c] < GEN_MAX
    /\ st' = [st EXCEPT ![c] = "established"]
    /\ cov' = [cov EXCEPT ![c] = "onDemand"]
    /\ excl' = [excl EXCEPT ![c] = FALSE]
    /\ gen' = [gen EXCEPT ![c] = @ + 1]
    /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
    /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
    /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
    /\ queued' = [queued EXCEPT ![c] = FALSE]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, staleEv, rescanOwed, rescanReload, docVars,
                   faultVars, witVars>>

\* Registration failure / unsupported namespace: a TYPED refusal is the
\* explicit outcome (message.rs NotifyCoverage::Unsupported); no watch,
\* no coverage, no freshness. MUTATION 9 instead serves the scope with an
\* invisible full-tree crawl posing as push coverage.
SubscribeRefused(c) ==
    /\ st[c] = "idle"
    /\ unsupportedRefusals = 0
    /\ unsupportedRefusals' = 1
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, pendInv,
                   pendNew, queued, staleEv, rescanOwed, rescanReload,
                   docVars, faultVars, rescans, staleDrops, dupDeliveries,
                   boundaryWithNewer, controlUnderPressure, saveRaces>>

SubscribeCrawl(c) ==
    /\ MUTATION = 9
    /\ st[c] = "idle"
    /\ gen[c] < GEN_MAX
    /\ invisibleCrawls' = 1
    /\ st' = [st EXCEPT ![c] = "scanning"]
    /\ cov' = [cov EXCEPT ![c] = "push"]
    /\ excl' = [excl EXCEPT ![c] = FALSE]
    /\ gen' = [gen EXCEPT ![c] = @ + 1]
    /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
    /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
    /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
    /\ queued' = [queued EXCEPT ![c] = FALSE]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, staleEv, rescanOwed, rescanReload, docVars,
                   authoritativeApplies, staleActs, guessedRelocations,
                   silentDrops, boundaryErasures, dirtyClobbers,
                   staleReloads, witVars>>

\* Unsubscribe retires the identity by its full stamp. Control progress
\* under data pressure: enabled regardless of whether the event slot is
\* full (the amendment's control/cancellation-progress requirement).
Unsubscribe(c) ==
    /\ Subscribed(c)
    /\ IF queued[c] /\ controlUnderPressure = 0
       THEN controlUnderPressure' = 1
       ELSE UNCHANGED controlUnderPressure
    /\ st' = [st EXCEPT ![c] = "idle"]
    /\ cov' = [cov EXCEPT ![c] = "none"]
    /\ queued' = [queued EXCEPT ![c] = FALSE]
    /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
    /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
    /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
    /\ rescanOwed' = [rescanOwed EXCEPT ![c] = FALSE]
    /\ staleEv' = [staleEv EXCEPT ![c] = FALSE]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    \* Generation history is retained: re-entry bumps the generation
    \* rather than resetting it. In-flight reloads are admitted document
    \* observations; they stay.
    /\ UNCHANGED <<incarnation, gen, excl, rescanReload, docVars,
                   faultVars, rescans, staleDrops, dupDeliveries,
                   boundaryWithNewer, unsupportedRefusals, saveRaces>>

\* The connection drops (or the worker restarted): every subscription
\* loses its baseline. An undelivered hint becomes stale-identity
\* traffic; a rescan is owed before any freshness claim. Enabled for any
\* live subscription, push or on-demand.
Disconnect(c) ==
    /\ Subscribed(c)
    /\ st' = [st EXCEPT ![c] = "lost"]
    /\ cov' = [cov EXCEPT ![c] = "none"]
    /\ staleEv' = [staleEv EXCEPT ![c] = queued[c]]
    /\ queued' = [queued EXCEPT ![c] = FALSE]
    /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
    /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
    /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
    /\ rescanOwed' = [rescanOwed EXCEPT ![c] = TRUE]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, gen, excl, rescanReload, docVars,
                   faultVars, witVars>>

\* The worker process restarts: a new incarnation. Old sessions and
\* subscription identities are dead worker-side (guard.rs
\* WrongIncarnation); clients observe the loss through Disconnect.
WorkerRestart ==
    /\ incarnation < INC_MAX
    /\ incarnation' = incarnation + 1
    /\ UNCHANGED <<st, cov, gen, excl, boundaryPending, lossVars, docVars,
                   faultVars, witVars>>

(***************************************************************************)
(* The reconcile boundary: installation, initial scan and later events.   *)
(***************************************************************************)

\* The worker finishes the initial scan's snapshot; the boundary itself
\* travels the control lane, so events may still arrive (and be
\* delivered) in between. Invalidations recorded from now on are NEWER
\* than the snapshot.
ScanSnapshot(c) ==
    /\ st[c] = "scanning"
    /\ ~boundaryPending[c]
    /\ boundaryPending' = [boundaryPending EXCEPT ![c] = TRUE]
    /\ UNCHANGED <<incarnation, identityVars, pendInv, pendNew, queued,
                   staleEv, rescanOwed, rescanReload, fresh, docVars,
                   faultVars, witVars>>

\* ReconcileBoundary applies: invalidations recorded BEFORE the snapshot
\* are covered by the scan; NEWER invalidations survive (and become the
\* unreconciled set). MUTATION 6 erases them all.
DeliverBoundary(c) ==
    /\ st[c] = "scanning"
    /\ boundaryPending[c]
    /\ IF MUTATION = 6
       THEN /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
            /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
            /\ IF pendNew[c] THEN boundaryErasures' = 1
               ELSE UNCHANGED boundaryErasures
            /\ UNCHANGED boundaryWithNewer
       ELSE /\ pendInv' = [pendInv EXCEPT ![c] = pendNew[c]]
            /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
            /\ IF pendNew[c] /\ boundaryWithNewer = 0
               THEN boundaryWithNewer' = 1
               ELSE UNCHANGED boundaryWithNewer
            /\ UNCHANGED boundaryErasures
    /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
    /\ st' = [st EXCEPT ![c] = "established"]
    /\ UNCHANGED <<incarnation, cov, gen, excl, queued, staleEv,
                   rescanOwed, rescanReload, fresh, docVars,
                   authoritativeApplies, staleActs, guessedRelocations,
                   silentDrops, dirtyClobbers, staleReloads,
                   invisibleCrawls, rescans, staleDrops, dupDeliveries,
                   unsupportedRefusals, controlUnderPressure, saveRaces>>

(***************************************************************************)
(* Filesystem churn and the bounded native queue.                          *)
(***************************************************************************)

\* Shared queue discipline for an external change with live push
\* coverage: the native layer raises a hint; the bounded slot COALESCES
\* (one outstanding hint per subscription); a full slot is an OVERFLOW —
\* the baseline is invalidated and a conservative rescan is owed,
\* atomically with the drop. The only staleness sign is never dropped
\* silently. MUTATION 5 drops it without recording anything. (Overflow
\* mid-scan voids the scan: the rescan subsumes it.)
RaiseHint(c) ==
    IF PushLive(c)
    THEN IF ~queued[c]
         THEN /\ queued' = [queued EXCEPT ![c] = TRUE]
              /\ UNCHANGED <<st, boundaryPending, pendInv, pendNew,
                             rescanOwed, silentDrops>>
         ELSE IF MUTATION = 5
              THEN /\ silentDrops' = 1
                   /\ UNCHANGED <<st, boundaryPending, pendInv, pendNew,
                                  queued, rescanOwed>>
              ELSE /\ queued' = [queued EXCEPT ![c] = FALSE]
                   /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
                   /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
                   /\ rescanOwed' = [rescanOwed EXCEPT ![c] = TRUE]
                   /\ st' = [st EXCEPT ![c] = "degraded"]
                   /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
                   /\ UNCHANGED silentDrops
    ELSE UNCHANGED <<st, boundaryPending, pendInv, pendNew, queued,
                     rescanOwed, silentDrops>>

\* An external writer changes the document on disk.
ExternalWrite(c) ==
    /\ Subscribed(c)
    /\ diskRev[c] < REV_MAX
    /\ diskRev' = [diskRev EXCEPT ![c] = @ + 1]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ RaiseHint(c)
    /\ UNCHANGED <<incarnation, cov, gen, excl, staleEv, rescanReload,
                   dirty, loadedRev, extChg, renameSt, inFlight,
                   authoritativeApplies, staleActs, guessedRelocations,
                   boundaryErasures, dirtyClobbers, staleReloads,
                   invisibleCrawls, rescans, staleDrops, dupDeliveries,
                   boundaryWithNewer, unsupportedRefusals,
                   controlUnderPressure, saveRaces>>

\* An external rename whose cookie pairing failed: the hint is AMBIGUOUS
\* (request.rs NotifyKind::Ambiguous). The document's disk identity moved;
\* only observation may confirm that. Queue handling is the honest
\* overflow path, exactly as in ExternalWrite — but MUTATION 5 is the
\* write path's fault; rename overflow is always honest.
ExternalRename(c) ==
    /\ Subscribed(c)
    /\ renameSt[c] = "none"
    /\ diskRev[c] < REV_MAX
    /\ diskRev' = [diskRev EXCEPT ![c] = @ + 1]
    /\ renameSt' = [renameSt EXCEPT ![c] = "ambiguous"]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ IF PushLive(c)
       THEN IF ~queued[c]
            THEN /\ queued' = [queued EXCEPT ![c] = TRUE]
                 /\ UNCHANGED <<st, boundaryPending, pendInv, pendNew,
                                rescanOwed>>
            ELSE /\ queued' = [queued EXCEPT ![c] = FALSE]
                 /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
                 /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
                 /\ rescanOwed' = [rescanOwed EXCEPT ![c] = TRUE]
                 /\ st' = [st EXCEPT ![c] = "degraded"]
                 /\ boundaryPending' = [boundaryPending EXCEPT ![c] = FALSE]
       ELSE UNCHANGED <<st, boundaryPending, pendInv, pendNew, queued,
                        rescanOwed>>
    /\ UNCHANGED <<incarnation, cov, gen, excl, staleEv, rescanReload,
                   dirty, loadedRev, extChg, inFlight, faultVars,
                   witVars>>

(***************************************************************************)
(* Event delivery: hints, duplicates, stale identity.                      *)
(***************************************************************************)

\* A hint event is delivered. Delivery may reorder and coalesce, so the
\* slot's hint stands for any number of native events. THE hint rule:
\* delivery only RECORDS an invalidation — never an authoritative edit,
\* save receipt or write log. An invalidation recorded while the scan's
\* boundary is pending is NEWER than the snapshot. MUTATION 1 applies the
\* hint as authority: content installed without observation.
RecordInvalidation(c) ==
    IF boundaryPending[c]
    THEN /\ pendNew' = [pendNew EXCEPT ![c] = TRUE]
         /\ UNCHANGED pendInv
    ELSE /\ pendInv' = [pendInv EXCEPT ![c] = TRUE]
         /\ UNCHANGED pendNew

DeliverHint(c) ==
    /\ queued[c]
    /\ queued' = [queued EXCEPT ![c] = FALSE]
    /\ RecordInvalidation(c)
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ IF MUTATION = 1 /\ ~dirty[c]
       THEN /\ authoritativeApplies' = 1
            /\ loadedRev' = [loadedRev EXCEPT ![c] = diskRev[c]]
       ELSE UNCHANGED <<authoritativeApplies, loadedRev>>
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, staleEv,
                   rescanOwed, rescanReload, dirty, diskRev, extChg,
                   renameSt, inFlight, staleActs, guessedRelocations,
                   silentDrops, boundaryErasures, dirtyClobbers,
                   staleReloads, invisibleCrawls, witVars>>

\* Duplicate/late redelivery of a hint (relay duplicates, reordered
\* observations). Absorbed idempotently: the same invalidation rule,
\* never authority.
DeliverDuplicate(c) ==
    /\ dupDeliveries = 0
    /\ PushLive(c)
    /\ queued[c] \/ pendInv[c] \/ pendNew[c]
    /\ dupDeliveries' = 1
    /\ RecordInvalidation(c)
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, queued,
                   staleEv, rescanOwed, rescanReload, docVars, faultVars,
                   rescans, staleDrops, boundaryWithNewer,
                   unsupportedRefusals, controlUnderPressure, saveRaces>>

\* An event stamped by a superseded identity arrives: a reused descriptor
\* after reinstallation, or traffic from the old worker incarnation. The
\* generation/incarnation check refuses it (guard.rs admit_subscription);
\* it is DROPPED. MUTATION 2 acts on it.
DeliverStale(c) ==
    /\ staleEv[c]
    /\ (MUTATION = 2 \/ staleDrops = 0)
    /\ staleEv' = [staleEv EXCEPT ![c] = FALSE]
    /\ IF MUTATION = 2
       THEN /\ staleActs' = 1
            /\ UNCHANGED staleDrops
       ELSE /\ staleDrops' = 1
            /\ UNCHANGED staleActs
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, pendInv,
                   pendNew, queued, rescanOwed, rescanReload, fresh,
                   docVars, authoritativeApplies, guessedRelocations,
                   silentDrops, boundaryErasures, dirtyClobbers,
                   staleReloads, invisibleCrawls, rescans, dupDeliveries,
                   boundaryWithNewer, unsupportedRefusals,
                   controlUnderPressure, saveRaces>>

(***************************************************************************)
(* Observation and document publication.                                   *)
(***************************************************************************)

\* Invalidation triggers observation/reconciliation: the client observes
\* the current disk state through the worker (a bounded read/stat), which
\* reconciles every recorded invalidation, resolves an ambiguous rename
\* and snapshots the observed version for the one outstanding reload. A
\* rescan obligation is discharged only by this reobservation —
\* reestablishing the baseline. Overflow during the rescan window is safe
\* by construction: a hint landing after this snapshot stays pending and
\* blocks freshness past the reload's publication.
ReloadStart(c) ==
    /\ st[c] \in {"established", "degraded"}
    /\ \/ pendInv[c]
       \/ pendNew[c]
       \/ rescanOwed[c]
       \/ renameSt[c] = "ambiguous"
       \/ /\ cov[c] = "onDemand"
          /\ loadedRev[c] < diskRev[c]
    /\ inFlight[c] = {}
    /\ inFlight' = [inFlight EXCEPT ![c] = {diskRev[c]}]
    /\ pendInv' = [pendInv EXCEPT ![c] = FALSE]
    /\ pendNew' = [pendNew EXCEPT ![c] = FALSE]
    /\ renameSt' = [renameSt EXCEPT ![c] =
                    IF @ = "ambiguous" THEN "observed" ELSE @]
    /\ IF rescanOwed[c]
       THEN /\ rescanOwed' = [rescanOwed EXCEPT ![c] = FALSE]
            /\ rescanReload' = [rescanReload EXCEPT ![c] = TRUE]
            /\ st' = [st EXCEPT ![c] = "established"]
            /\ IF rescans = 0 THEN rescans' = 1 ELSE UNCHANGED rescans
       ELSE UNCHANGED <<rescanOwed, rescanReload, st, rescans>>
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, cov, gen, excl, boundaryPending, queued,
                   staleEv, dirty, loadedRev, diskRev, extChg,
                   authoritativeApplies, staleActs, guessedRelocations,
                   silentDrops, boundaryErasures, dirtyClobbers,
                   staleReloads, invisibleCrawls, staleDrops,
                   dupDeliveries, boundaryWithNewer, unsupportedRefusals,
                   controlUnderPressure, saveRaces>>

\* A reload publishes its snapshot. Clean buffer: the snapshot installs
\* only at or beyond the current baseline (the expected document /
\* binding / observation check); a stale snapshot is dropped by the
\* revision re-check at completion (filter.rs) — newer state is never
\* cleared. Dirty buffer: preserved, external-change state set
\* (buffer.rs's save refusal is the consumer). MUTATION 7 applies over
\* the dirty buffer; MUTATION 8 publishes the stale snapshot.
ReloadPublish(c) ==
    /\ inFlight[c] # {}
    /\ LET r == CHOOSE r \in inFlight[c] : TRUE IN
        /\ IF dirty[c]
           THEN IF MUTATION = 7
                THEN /\ dirtyClobbers' = 1
                     /\ dirty' = [dirty EXCEPT ![c] = FALSE]
                     /\ loadedRev' = [loadedRev EXCEPT ![c] = r]
                     /\ extChg' = [extChg EXCEPT ![c] = FALSE]
                     /\ UNCHANGED staleReloads
                ELSE /\ extChg' = [extChg EXCEPT ![c] = TRUE]
                     /\ UNCHANGED <<dirty, loadedRev, dirtyClobbers,
                                    staleReloads>>
           ELSE IF r >= loadedRev[c]
                THEN /\ loadedRev' = [loadedRev EXCEPT ![c] = r]
                     /\ extChg' = [extChg EXCEPT ![c] = FALSE]
                     /\ UNCHANGED <<dirty, dirtyClobbers, staleReloads>>
                ELSE IF MUTATION = 8
                     THEN /\ staleReloads' = 1
                          /\ loadedRev' = [loadedRev EXCEPT ![c] = r]
                          /\ UNCHANGED <<dirty, dirtyClobbers, extChg>>
                     ELSE /\ UNCHANGED <<loadedRev, staleReloads>>
                          /\ UNCHANGED <<dirty, dirtyClobbers, extChg>>
        /\ inFlight' = [inFlight EXCEPT ![c] = {}]
        /\ rescanReload' = [rescanReload EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, pendInv,
                   pendNew, queued, staleEv, rescanOwed, fresh, diskRev,
                   renameSt, authoritativeApplies, staleActs,
                   guessedRelocations, silentDrops, boundaryErasures,
                   invisibleCrawls, witVars>>

\* The editor edits the buffer. Local document mutation is not
\* filesystem I/O; the scope's freshness claim is untouched.
Edit(c) ==
    /\ Subscribed(c)
    /\ ~dirty[c]
    /\ dirty' = [dirty EXCEPT ![c] = TRUE]
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, lossVars,
                   loadedRev, diskRev, extChg, renameSt, inFlight,
                   faultVars, witVars>>

\* The client saves through the worker's mutation family: the committed
\* receipt is AUTHORITY — it installs the baseline directly (loadedRev
\* tracks the saved version) and clears external-change state. An
\* in-flight snapshot the receipt supersedes stays in flight; it is
\* dropped by the revision re-check at ITS completion (filter.rs), never
\* eagerly. Pre-save hints left outstanding are the
\* save-versus-external-write race; a late event afterwards only records
\* an invalidation, and reobservation finds the baseline already current.
OwnSave(c) ==
    /\ dirty[c]
    /\ Connected(c)
    /\ diskRev[c] < REV_MAX
    /\ LET ns == diskRev[c] + 1 IN
        /\ diskRev' = [diskRev EXCEPT ![c] = ns]
        /\ loadedRev' = [loadedRev EXCEPT ![c] = ns]
    /\ dirty' = [dirty EXCEPT ![c] = FALSE]
    /\ extChg' = [extChg EXCEPT ![c] = FALSE]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ IF (pendInv[c] \/ pendNew[c] \/ queued[c]) /\ saveRaces = 0
       THEN saveRaces' = 1
       ELSE UNCHANGED saveRaces
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, pendInv,
                   pendNew, queued, staleEv, rescanOwed, rescanReload,
                   renameSt, inFlight, faultVars, rescans, staleDrops,
                   dupDeliveries, boundaryWithNewer, unsupportedRefusals,
                   controlUnderPressure>>

\* The client claims the scope view is fresh. Honest: established
\* coverage, a valid baseline, no rescan obligation, no pending
\* invalidation, no unsettled observation, no unresolved ambiguous
\* rename, and the document view matches disk. MUTATION 4 claims
\* regardless — freshness over lost/degraded/partial coverage.
ClaimFresh(c) ==
    /\ ~fresh[c]
    /\ IF MUTATION = 4
       THEN fresh' = [fresh EXCEPT ![c] = TRUE]
       ELSE /\ st[c] = "established"
            /\ ~rescanOwed[c]
            /\ ~pendInv[c]
            /\ ~pendNew[c]
            /\ inFlight[c] = {}
            /\ ~boundaryPending[c]
            /\ renameSt[c] # "ambiguous"
            /\ loadedRev[c] = diskRev[c]
            /\ fresh' = [fresh EXCEPT ![c] = TRUE]
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, pendInv,
                   pendNew, queued, staleEv, rescanOwed, rescanReload,
                   docVars, faultVars, witVars>>

\* The editor relocates the document binding after a rename. Honest: the
\* move was confirmed by observation. MUTATION 3 relocates straight off
\* the ambiguous hint — a silent relocation from a guessed rename pair.
RelocateBinding(c) ==
    /\ IF MUTATION = 3
       THEN /\ renameSt[c] = "ambiguous"
            /\ guessedRelocations' = 1
       ELSE /\ renameSt[c] = "observed"
            /\ UNCHANGED guessedRelocations
    /\ renameSt' = [renameSt EXCEPT ![c] = "relocated"]
    /\ fresh' = [fresh EXCEPT ![c] = FALSE]
    /\ UNCHANGED <<incarnation, identityVars, boundaryPending, pendInv,
                   pendNew, queued, staleEv, rescanOwed, rescanReload,
                   dirty, loadedRev, diskRev, extChg, inFlight,
                   authoritativeApplies, staleActs, silentDrops,
                   boundaryErasures, dirtyClobbers, staleReloads,
                   invisibleCrawls, witVars>>

Next ==
    \/ WorkerRestart
    \/ \E c \in CLIENTS : Connect(c)
    \/ \E c \in CLIENTS : SubscribePush(c)
    \/ \E c \in CLIENTS : SubscribeOnDemand(c)
    \/ \E c \in CLIENTS : SubscribeRefused(c)
    \/ \E c \in CLIENTS : SubscribeCrawl(c)
    \/ \E c \in CLIENTS : Unsubscribe(c)
    \/ \E c \in CLIENTS : Disconnect(c)
    \/ \E c \in CLIENTS : ScanSnapshot(c)
    \/ \E c \in CLIENTS : DeliverBoundary(c)
    \/ \E c \in CLIENTS : ExternalWrite(c)
    \/ \E c \in CLIENTS : ExternalRename(c)
    \/ \E c \in CLIENTS : DeliverHint(c)
    \/ \E c \in CLIENTS : DeliverDuplicate(c)
    \/ \E c \in CLIENTS : DeliverStale(c)
    \/ \E c \in CLIENTS : ReloadStart(c)
    \/ \E c \in CLIENTS : ReloadPublish(c)
    \/ \E c \in CLIENTS : Edit(c)
    \/ \E c \in CLIENTS : OwnSave(c)
    \/ \E c \in CLIENTS : ClaimFresh(c)
    \/ \E c \in CLIENTS : RelocateBinding(c)

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* The amendment's named safety properties.                                *)
(***************************************************************************)

\* Events are hints: none was ever applied as an authoritative edit,
\* save receipt or write log.
HintsNeverAuthority == authoritativeApplies = 0

\* Subscription identity: no event stamped by a superseded
\* generation/incarnation (reused descriptor, old worker) was acted on.
StaleIdentityNeverActs == staleActs = 0

\* Replacement/rename: no binding relocated from a guessed rename pair.
AmbiguousNeverRelocates == guessedRelocations = 0

\* Loss and partial coverage: freshness is claimed only with established
\* coverage, a valid baseline, no rescan obligation, no pending
\* invalidation and no unsettled reload.
FreshRequiresCoverage ==
    \A c \in CLIENTS : fresh[c] =>
        /\ st[c] = "established"
        /\ cov[c] \in {"push", "onDemand"}
        /\ ~rescanOwed[c]
        /\ ~pendInv[c]
        /\ ~pendNew[c]
        /\ inFlight[c] = {}

\* Bounded flow: coalescing never silently dropped the only staleness
\* sign — a full queue always recorded overflow.
CoalesceNeverLosesStaleness == silentDrops = 0

\* Ordering/reconciliation: scan completion never erased a newer
\* invalidation.
ScanNeverErasesNewer == boundaryErasures = 0

\* Document publication: no reload applied over a dirty buffer.
DirtyNeverClobbered == dirtyClobbers = 0

\* Document publication: no stale snapshot published over a newer
\* baseline — newer edits are never cleared by a stale reload.
StaleReloadNeverClears == staleReloads = 0

\* Unsupported filesystems are reported honestly: no invisible full
\* crawl substituted for native coverage.
CoverageHonest == invisibleCrawls = 0

\* The native-to-service queue stays within its bound.
BoundedQueue ==
    \A c \in CLIENTS : (IF queued[c] THEN 1 ELSE 0) <= QMAX

(***************************************************************************)
(* Non-vacuity witnesses: each must be REACHABLE in the honest model     *)
(* (checked as an expected-to-fail invariant over the coverage config).  *)
(***************************************************************************)

\* Overflow degraded a subscription (loss is an explicit outcome).
WitnessNoOverflow == \A c \in CLIENTS : st[c] # "degraded"

\* A rescan obligation was discharged by reobservation.
WitnessNoRescan == rescans = 0

\* The worker restarted (a second incarnation exists).
WitnessNoRestart == incarnation = 1

\* A stale-identity event arrived and was dropped.
WitnessNoStaleDrop == staleDrops = 0

\* A duplicate/late redelivery was absorbed.
WitnessNoDuplicate == dupDeliveries = 0

\* A reconcile boundary applied while a newer invalidation survived.
WitnessNoBoundaryNewer == boundaryWithNewer = 0

\* An unsupported namespace got its typed refusal.
WitnessNoUnsupportedRefusal == unsupportedRefusals = 0

\* On-demand coverage was honestly reported and used.
WitnessNoOnDemand == \A c \in CLIENTS : cov[c] # "onDemand"

\* A subscription reported excluded subtrees.
WitnessNoExclusions == \A c \in CLIENTS : ~excl[c]

\* Unsubscribe progressed with a full data queue.
WitnessNoControlPressure == controlUnderPressure = 0

\* A save committed while pre-save hints were still outstanding.
WitnessNoSaveRace == saveRaces = 0

\* A dirty buffer was preserved with external-change state.
WitnessNoDirtyExternal == \A c \in CLIENTS : ~extChg[c]

\* A binding relocated after observation confirmed the move.
WitnessNoRelocation == \A c \in CLIENTS : renameSt[c] # "relocated"

\* A hint landed inside the rescan window (overflow-during-rescan):
\* the rescan's snapshot is already stale, the invalidation survives
\* publication and freshness stays blocked until reobservation.
WitnessNoRescanChurn ==
    \A c \in CLIENTS : ~(rescanReload[c] /\ (pendInv[c] \/ pendNew[c]))

\* Both clients hold established subscriptions at once (identity
\* separation is exercised).
WitnessNoTwoEstablished ==
    \A a, b \in CLIENTS :
        a = b \/ st[a] # "established" \/ st[b] # "established"

=============================================================================
