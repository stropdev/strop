---- MODULE RemoteRead ----
(***************************************************************************)
(* The remote-read lifecycle as shipped (0034): admission, worker, child   *)
(* process, cancellation and publication — the seam between               *)
(* crates/strop/src/editor/io.rs + io/remote.rs (editor/owner side),       *)
(* crates/strop-remote/src/transport.rs (worker-side orchestration) and    *)
(* strop-core's worker.rs / process.rs (spawn, cancel, reap). The wire     *)
(* exchange itself is SftpWire; this module owns everything around it.     *)
(*                                                                       *)
(*   1. admission is a capture: request_target builds OpenKey             *)
(*      {origin document, buffer revision, focus_epoch, intent} and       *)
(*      installs it in io.open, with the navigation slot pointing at      *)
(*      it. A superseding navigation, Escape and closing the owner        *)
(*      cancel first (cancel_open: entry removed, then handle.cancel)     *)
(*                                                                       *)
(*   2. the worker is single-terminal: worker::spawn's emitter is         *)
(*      taken exactly once — by the worker's finish() or by the           *)
(*      cancel closure ("reservation is the terminal linearization        *)
(*      point"). Whatever races, exactly one outcome is ever emitted      *)
(*      and a queued completion is delivered at most once                 *)
(*                                                                       *)
(*   3. the child is owned: OwnedProcess registers its cancellation       *)
(*      hook BEFORE spawning; the kill capability targets the private     *)
(*      process group; publication of the pid re-checks the cancel        *)
(*      flag under the group lock, so a cancel that fired before the      *)
(*      pid existed still kills the eventually-published child;           *)
(*      wait() revokes the capability (group.pid = None) BEFORE           *)
(*      reaping, and only its owner reaps                                *)
(*                                                                       *)
(*   4. staleness loses: handle_io re-checks the entry, then              *)
(*      open_fresh (docs non-empty, same current document, same           *)
(*      focus_epoch, same revision) before anything is published; a       *)
(*      cancelled request's entry is gone, so even an already-queued      *)
(*      success cannot publish. Success publishes a read-only buffer      *)
(*      (Buffer::readonly = true; request_save refuses remote             *)
(*      documents). A refresh publication replaces the document           *)
(*      incarnation and bumps focus_epoch (finish_remote_refresh)         *)
(*                                                                       *)
(* THE invariants — each names an executable Rust oracle boundary that    *)
(* Main owns (re-checking the same property against the real API):        *)
(*                                                                       *)
(*   TypeOK              every variable stays in its declared finite      *)
(*                       domain (supersets where mutants need headroom)   *)
(*   FreshPublication    a published result carried the admission's       *)
(*                       document, focus and revision — publication       *)
(*                       happened while the capture was still fresh       *)
(*                       (Rust: handle_io's open_fresh gate; oracle:      *)
(*                       move focus / edit / close between worker         *)
(*                       success and delivery, assert nothing lands)      *)
(*   TerminalOnce        each request fires at most one terminal          *)
(*                       outcome (Rust: the single-shot emitter;          *)
(*                       oracle: count Outcome emissions per              *)
(*                       WorkerId across racing cancel/finish paths)      *)
(*   CancellationWins    once a request is cancelled its result can       *)
(*                       never publish, and a still-live child has        *)
(*                       the kill already in flight                       *)
(*                       (Rust: cancel_open removes the entry before      *)
(*                       handle.cancel; the hook signals the group;       *)
(*                       oracle: cancel then finish, assert no buffer     *)
(*                       appears and the child is dead before reap)       *)
(*   SignalOwnedPid      every signal targeted the pid of an unreaped     *)
(*                       child of the signaller — never a recycled        *)
(*                       pid owned by someone else                        *)
(*                       (Rust: revoke-before-reap in OwnedProcess::wait; *)
(*                       oracle: kill a child, reap, spawn again, fire    *)
(*                       the late detached callback, assert only the      *)
(*                       first child was signalled)                       *)
(*   NoUnownedChild      once the worker has returned, its child is       *)
(*                       reaped or never existed — no child outlives      *)
(*                       its owner (Rust: read_failure / reap path /      *)
(*                       OwnedProcess Drop; oracle: assert the pid is     *)
(*                       gone on every exit path)                         *)
(*   ReadonlyPublication every published remote snapshot is read-only     *)
(*                       (Rust: buffer.readonly = true before the         *)
(*                       buffer escapes; oracle: assert readonly on       *)
(*                       every DocumentSource::Remote publication and     *)
(*                       that :w!/:set noro refuse)                       *)
(*                                                                       *)
(* Temporal properties (FairSpec only, under explicit assumptions):       *)
(*   EventuallyTerminal  an admitted worker eventually returns            *)
(*   EventuallyReaped    a spawned child is eventually reaped            *)
(* Weak fairness is assumed ONLY for worker-local and OS-progress         *)
(* actions (install, spawn, hook fire, transfer end, reap, emit).         *)
(* The OS assumptions: SIGKILL on the private group is delivered; a       *)
(* session whose pipes closed (or that was killed) exits; waitid          *)
(* observes exit; the session deadline bounds any lingering transfer      *)
(* (abstracted as the nondeterministic TransferDone / TransferFails).     *)
(* User/editor actions and event delivery are the environment — no       *)
(* fairness on them. These are qualified claims about the abstraction,    *)
(* not proofs of OS scheduling.                                          *)
(*                                                                       *)
(* Deliberately faulty variants (one knob, no copy-pasted mutant          *)
(* module; configs flip MUTATION):                                       *)
(*   MUTATION = 1  deliver-publish without the freshness gate — must      *)
(*                 die by exactly FreshPublication                       *)
(*   MUTATION = 2  emit a second terminal outcome after success — must    *)
(*                 die by exactly TerminalOnce                           *)
(*   MUTATION = 3  reap without revoking the pid capability first —       *)
(*                 must die by exactly SignalOwnedPid (a late            *)
(*                 detached callback then kills a recycled pid)          *)
(*   MUTATION = 4  return without cleanup (no reap, no RAII backstop)     *)
(*                 — must die by exactly NoUnownedChild and, under       *)
(*                 fairness, by EventuallyReaped                          *)
(*                                                                       *)
(* This follows rootle's provider-protocol precedent                     *)
(* (https://rootle.dev/docs/provider-protocol.html): a bounded TLA+       *)
(* model, safety invariants plus fairness-qualified temporal             *)
(* properties, deliberately faulty variants, and executable checks —     *)
(* no claim of unbounded soundness or a Rust refinement proof.           *)
(*                                                                       *)
(* Not modeled: wire-level faults (SftpWire owns them), the deadline      *)
(* clock (its effect is the TransferDone/TransferFails nondeterminism),  *)
(* stderr draining, session persistence, Editor's `finishing`            *)
(* reentrancy flag, LspLocation freshness, finish_remote_refresh's       *)
(* internal cancel_pending (the model lets other requests keep racing    *)
(* — strictly more behaviors), the post-publication switch to the new    *)
(* document and canonical-target dedup (exact native-path equality in    *)
(* files.rs/FileTarget — cosmetic after the freshness gate), WorkerId    *)
(* exhaustion, thread-start failure.                                     *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS REQS,      \* request (worker) ids, e.g. {1, 2}; allocated in order
          PIDS,      \* OS pid values, e.g. {1, 2}; the OS may recycle reaped ones
          MAXFOCUS,  \* focus_epoch cap for finiteness
          MAXREV,    \* buffer revision cap for finiteness
          MAXHEAD,   \* document incarnation cap for finiteness
          MUTATION   \* 0 honest; 1 stale publish; 2 double terminal;
                     \* 3 early pid reuse; 4 skip cleanup

VARIABLES focus,     \* editor focus_epoch (panes, document switches, refresh)
          rev,       \* current buffer revision (local edits)
          alive,     \* a current document exists (docs non-empty)
          head,      \* identity of the current document incarnation
          remoteDoc, \* the current document incarnation is a remote snapshot
          nav,       \* io.navigation: the request owning navigation, or NONE
          phase,     \* per request: IDLE | RUNNING (admitted once; ids monotonic)
          intent,    \* per request: IOPEN | IREFRESH
          work,      \* per request: WORKING | RETURNED (the worker thread)
          outcome,   \* per request: the single emitted outcome, or NONE
          fired,     \* per request: count of terminal emissions, including duplicates
          hook,      \* per request: cancel-resource lifecycle state
          child,     \* per request: the owned child's stage
          pid,       \* per request: the child's OS pid (NOPID before spawn)
          gpid,      \* per request: the group's live pid capability target
          cancelled, \* per request: the CancelToken flag
          entry,     \* per request: the io.open admission entry is present
          key,       \* per request: [head, focus, rev] captured at admission
          pub,       \* per request: the publication record, or NOPUB
          pending,   \* per request: a completion is queued for delivery
          dropped,   \* per request: a completion was processed without publishing
          signalLog  \* ghost: every signal event [pid, by, owner]

vars == <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome, fired,
          hook, child, pid, gpid, cancelled, entry, key, pub, pending, dropped,
          signalLog>>

\* -- symbolic codes (numbers are tags only) --------------------------------
NONE      == 0
IDLE      == 0
RUNNING   == 1
IOPEN     == 0    \* a navigation that opens/switches to the snapshot
IREFRESH  == 1    \* OpenIntent::Refresh: :e! on a remote snapshot
WORKING   == 0
RETURNED  == 1
OK        == 1    \* Outcome::Success
CANCELLED == 2    \* Outcome::Cancelled
FAILED    == 3    \* Outcome::Failed
HNONE     == 0    \* cancel resource not yet installed
HLIVE     == 1    \* registered (register_cancel_resource)
HINFLIGHT == 2    \* dequeued by cancel; the hook body is executing
HCLEARED  == 3    \* consumed (fired) or detached (clear_cancel_resource)
CNONE     == 0    \* no child was spawned
CLIVE     == 1    \* spawned, unreaped
CDEAD     == 2    \* exited or killed, still unreaped (pid reserved)
CREAPED   == 3    \* waited on; the pid may be recycled by the OS
CSPAWNING == 4    \* native spawn started; cancellation may finish before pid publication
NOPID     == 0
NOPUB     == [present |-> FALSE, head |-> 0, focus |-> 0, rev |-> 0, readonly |-> TRUE]

PUBS == [present: BOOLEAN, head: 0..MAXHEAD, focus: 0..MAXFOCUS, rev: 0..MAXREV, readonly: BOOLEAN]
Events == [pid: PIDS, by: REQS, owner: {NONE} \union REQS]

TypeOK ==
    /\ focus \in 0..MAXFOCUS
    /\ rev \in 0..MAXREV
    /\ head \in 0..MAXHEAD
    /\ alive \in BOOLEAN
    /\ remoteDoc \in BOOLEAN
    /\ nav \in {NONE} \union REQS
    /\ phase \in [REQS -> {IDLE, RUNNING}]
    /\ intent \in [REQS -> {IOPEN, IREFRESH}]
    /\ work \in [REQS -> {WORKING, RETURNED}]
    /\ outcome \in [REQS -> {NONE, OK, CANCELLED, FAILED}]
    /\ fired \in [REQS -> 0..2]
    /\ hook \in [REQS -> {HNONE, HLIVE, HINFLIGHT, HCLEARED}]
    /\ child \in [REQS -> {CNONE, CSPAWNING, CLIVE, CDEAD, CREAPED}]
    /\ pid \in [REQS -> {NOPID} \union PIDS]
    /\ gpid \in [REQS -> {NOPID} \union PIDS]
    /\ cancelled \in [REQS -> BOOLEAN]
    /\ entry \in [REQS -> BOOLEAN]
    /\ key \in [REQS -> [head: 0..MAXHEAD, focus: 0..MAXFOCUS, rev: 0..MAXREV]]
    /\ pub \in [REQS -> PUBS]
    /\ pending \in [REQS -> BOOLEAN]
    /\ dropped \in [REQS -> BOOLEAN]
    /\ signalLog \subseteq Events

Init ==
    /\ focus = 0 /\ rev = 0 /\ alive = TRUE /\ head = 0 /\ nav = NONE
    /\ remoteDoc = FALSE
    /\ phase = [r \in REQS |-> IDLE]
    /\ intent = [r \in REQS |-> IOPEN]
    /\ work = [r \in REQS |-> RETURNED]
    /\ outcome = [r \in REQS |-> NONE]
    /\ fired = [r \in REQS |-> 0]
    /\ hook = [r \in REQS |-> HNONE]
    /\ child = [r \in REQS |-> CNONE]
    /\ pid = [r \in REQS |-> NOPID]
    /\ gpid = [r \in REQS |-> NOPID]
    /\ cancelled = [r \in REQS |-> FALSE]
    /\ entry = [r \in REQS |-> FALSE]
    /\ key = [r \in REQS |-> [head |-> 0, focus |-> 0, rev |-> 0]]
    /\ pub = [r \in REQS |-> NOPUB]
    /\ pending = [r \in REQS |-> FALSE]
    /\ dropped = [r \in REQS |-> FALSE]
    /\ signalLog = {}

\* -- helpers ----------------------------------------------------------------
\* open_fresh: the capture is still exactly what the editor looks at.
Fresh(r) ==
    /\ alive
    /\ head = key[r].head
    /\ focus = key[r].focus
    /\ rev = key[r].rev

\* The worker may return once its child is settled (reaped, or none).
\* MUTATION = 4 lets it abandon a dead, unreaped child instead.
Settled(r) ==
    \/ child[r] \in {CNONE, CREAPED}
    \/ (MUTATION = 4 /\ child[r] = CDEAD)

\* Unreaped children reserve their pid; the OS may hand a reaped pid out
\* again. SpawnChild only ever picks from the unreserved pids.
Reserved == {pid[s] : s \in {t \in REQS : child[t] \in {CLIVE, CDEAD}}}
FreePids == PIDS \ Reserved

Holders(p) == {s \in REQS : /\ pid[s] = p /\ child[s] \in {CLIVE, CDEAD}}
OwnerOf(p) == IF Holders(p) = {} THEN NONE ELSE CHOOSE s \in Holders(p) : TRUE
    \* a singleton by pid exclusivity: only SpawnChild assigns pids, and it
    \* excludes reserved ones

\* clear_cancel_resource drops only a REGISTERED hook; one already dequeued
\* by cancel keeps executing. Both Rust paths land on HCLEARED-or-inflight.
ClearIfRegistered(r) ==
    IF hook[r] = HLIVE THEN [hook EXCEPT ![r] = HCLEARED] ELSE hook

\* -- editor side ------------------------------------------------------------
\* A new navigation. request_target first cancel_open's the previous
\* navigation (Superseded): its entry is removed and, if the worker has not
\* finished yet, its outcome is reserved CANCELLED and its hook goes in
\* flight. The new admission captures origin/revision/focus atomically.
RequestNew(r, it) ==
    /\ phase[r] = IDLE
    /\ \A s \in REQS : s < r => phase[s] /= IDLE     \* WorkerIds are monotonic
    /\ alive
    /\ IF it = IREFRESH
       THEN /\ head < MAXHEAD
            /\ focus < MAXFOCUS
            /\ remoteDoc
       ELSE TRUE
    /\ phase' = [phase EXCEPT ![r] = RUNNING]
    /\ work' = [work EXCEPT ![r] = WORKING]
    /\ child' = [child EXCEPT ![r] = CNONE]
    /\ pid' = [pid EXCEPT ![r] = NOPID]
    /\ gpid' = [gpid EXCEPT ![r] = NOPID]
    /\ key' = [key EXCEPT ![r] = [head |-> head, focus |-> focus, rev |-> rev]]
    /\ intent' = [intent EXCEPT ![r] = it]
    /\ pub' = [pub EXCEPT ![r] = NOPUB]
    /\ pending' = IF nav \in REQS /\ entry[nav] /\ outcome[nav] = NONE
                  THEN [pending EXCEPT ![nav] = TRUE, ![r] = FALSE]
                  ELSE [pending EXCEPT ![r] = FALSE]
    /\ dropped' = [dropped EXCEPT ![r] = FALSE]
    /\ IF nav \in REQS /\ entry[nav]
       THEN /\ nav' = r
            /\ entry' = [entry EXCEPT ![nav] = FALSE, ![r] = TRUE]
            /\ IF outcome[nav] = NONE
               THEN /\ outcome' = [outcome EXCEPT ![nav] = CANCELLED, ![r] = NONE]
                    /\ fired' = [fired EXCEPT ![nav] = @ + 1, ![r] = 0]
                    /\ cancelled' = [cancelled EXCEPT ![nav] = TRUE, ![r] = FALSE]
                    /\ IF hook[nav] = HLIVE
                       THEN hook' = [hook EXCEPT ![nav] = HINFLIGHT, ![r] = HNONE]
                       ELSE hook' = [hook EXCEPT ![r] = HNONE]
               ELSE /\ outcome' = [outcome EXCEPT ![r] = NONE]
                    /\ fired' = [fired EXCEPT ![r] = 0]
                    /\ cancelled' = [cancelled EXCEPT ![r] = FALSE]
                    /\ hook' = [hook EXCEPT ![r] = HNONE]
       ELSE /\ nav' = r
            /\ entry' = [entry EXCEPT ![r] = TRUE]
            /\ outcome' = [outcome EXCEPT ![r] = NONE]
            /\ fired' = [fired EXCEPT ![r] = 0]
            /\ cancelled' = [cancelled EXCEPT ![r] = FALSE]
            /\ hook' = [hook EXCEPT ![r] = HNONE]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, signalLog>>

\* Escape: cancel_open — the entry is removed first; the handle is only
\* cancelled if the worker has not already claimed the single outcome.
UserCancel(r) ==
    /\ nav = r
    /\ entry[r]
    /\ phase[r] = RUNNING
    /\ nav' = NONE
    /\ entry' = [entry EXCEPT ![r] = FALSE]
    /\ IF outcome[r] = NONE
       THEN /\ outcome' = [outcome EXCEPT ![r] = CANCELLED]
            /\ fired' = [fired EXCEPT ![r] = @ + 1]
            /\ cancelled' = [cancelled EXCEPT ![r] = TRUE]
            /\ pending' = [pending EXCEPT ![r] = TRUE]
            /\ hook' = IF hook[r] = HLIVE
                       THEN [hook EXCEPT ![r] = HINFLIGHT]
                       ELSE hook
       ELSE /\ UNCHANGED <<outcome, fired, cancelled, hook, pending>>
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, phase, intent, work, child, pid,
                   gpid, key, pub, dropped, signalLog>>

\* Closing the owner: the pending navigation is cancelled (OwnerClosed)
\* and no current document remains.
CloseOwner ==
    /\ alive
    /\ alive' = FALSE
    /\ IF nav \in REQS /\ entry[nav] /\ phase[nav] = RUNNING
       THEN /\ nav' = NONE
            /\ entry' = [entry EXCEPT ![nav] = FALSE]
            /\ IF outcome[nav] = NONE
               THEN /\ outcome' = [outcome EXCEPT ![nav] = CANCELLED]
                    /\ fired' = [fired EXCEPT ![nav] = @ + 1]
                    /\ cancelled' = [cancelled EXCEPT ![nav] = TRUE]
                    /\ pending' = [pending EXCEPT ![nav] = TRUE]
                    /\ IF hook[nav] = HLIVE
                       THEN hook' = [hook EXCEPT ![nav] = HINFLIGHT]
                       ELSE hook' = hook
               ELSE /\ UNCHANGED <<outcome, fired, cancelled, hook, pending>>
       ELSE /\ UNCHANGED <<nav, entry, outcome, fired, cancelled, hook, pending>>
    /\ UNCHANGED <<focus, rev, head, remoteDoc, phase, intent, work, child, pid, gpid,
                   key, pub, dropped, signalLog>>

\* A closed arena slot may be reopened with a new document incarnation.
NewOwner ==
    /\ ~alive /\ head < MAXHEAD /\ focus < MAXFOCUS
    /\ alive' = TRUE /\ head' = head + 1 /\ focus' = focus + 1
    /\ rev' = 0 /\ remoteDoc' = FALSE
    /\ UNCHANGED <<nav, phase, intent, work, outcome, fired, hook, child, pid, gpid,
                   cancelled, entry, key, pub, pending, dropped, signalLog>>

\* A focus change that does NOT cancel (e.g. a split). Pane and document
\* switches cancel as well — that variant is UserCancel/RequestNew; keeping
\* the non-cancelling move is the worst case for the delivery race.
Move ==
    /\ focus < MAXFOCUS
    /\ focus' = focus + 1
    /\ UNCHANGED <<rev, alive, head, remoteDoc, nav, phase, intent, work, outcome, fired,
                   hook, child, pid, gpid, cancelled, entry, key, pub, pending,
                   dropped, signalLog>>

\* A local edit on the current document moves the revision.
Edit ==
    /\ alive
    /\ rev < MAXREV
    /\ rev' = rev + 1
    /\ UNCHANGED <<focus, alive, head, remoteDoc, nav, phase, intent, work, outcome, fired,
                   hook, child, pid, gpid, cancelled, entry, key, pub, pending,
                   dropped, signalLog>>

\* -- worker side -------------------------------------------------------------
\* OwnedProcess::spawn registers the cancellation resource BEFORE spawning.
\* Registering after the token was cancelled invokes it immediately and
\* fails the spawn (no child at all).
InstallHook(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ hook[r] = HNONE
    /\ hook' = IF cancelled[r]
               THEN [hook EXCEPT ![r] = HCLEARED]
               ELSE [hook EXCEPT ![r] = HLIVE]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome,
                   fired, child, pid, gpid, cancelled, entry, key, pub, pending,
                   dropped, signalLog>>

\* Split acquisition from publication so a completed cancellation hook may race
\* the native spawn, not merely a hook still waiting for the group mutex.
BeginSpawn(r) ==
    /\ phase[r] = RUNNING /\ work[r] = WORKING
    /\ hook[r] = HLIVE /\ ~cancelled[r] /\ child[r] = CNONE
    /\ child' = [child EXCEPT ![r] = CSPAWNING]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work,
                   outcome, fired, hook, pid, gpid, cancelled, entry, key, pub,
                   pending, dropped, signalLog>>

\* The child appears and its pid is published under the group lock. If the
\* hook already fired (Group.cancelled), the eventually-published child is
\* killed at publication — that is the before/during-publication cancel.
SpawnChild(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ child[r] = CSPAWNING
    /\ \E p \in FreePids :
       /\ pid' = [pid EXCEPT ![r] = p]
       /\ gpid' = [gpid EXCEPT ![r] = p]
       /\ child' = [child EXCEPT ![r] = IF cancelled[r] THEN CDEAD ELSE CLIVE]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome,
                   fired, hook, cancelled, entry, key, pub, pending, dropped,
                   signalLog>>

\* The cancelled hook body, serialized against the group lock by TLA's
\* atomicity: it fires at whatever the capability currently targets. In the
\* honest spec revoke-before-reap has emptied that; under MUTATION = 3 a
\* reaped-and-recycled pid gets killed and the event records a foreign
\* owner.
HookFires(r) ==
    /\ hook[r] = HINFLIGHT
    /\ hook' = [hook EXCEPT ![r] = HCLEARED]
    /\ IF gpid[r] = NOPID
       THEN /\ UNCHANGED <<child, signalLog>>
       ELSE /\ signalLog' = signalLog \union {[pid |-> gpid[r], by |-> r,
                                              owner |-> OwnerOf(gpid[r])]}
            /\ child' = [s \in REQS |-> IF s \in Holders(gpid[r]) THEN CDEAD ELSE child[s]]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome,
                   fired, pid, gpid, cancelled, entry, key, pub, pending, dropped>>

\* The session finished inside the deadline (SftpWire's happy path): ssh
\* exits once its pipes closed. OS-progress assumption.
TransferDone(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ child[r] = CLIVE
    /\ child' = [child EXCEPT ![r] = CDEAD]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome,
                   fired, hook, pid, gpid, cancelled, entry, key, pub, pending,
                   dropped, signalLog>>

\* The session failed or the deadline elapsed: the owner terminates the
\* group (read_failure / reap_after_shutdown's kill path).
TransferFails(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ child[r] = CLIVE
    /\ gpid[r] /= NOPID
    /\ signalLog' = signalLog \union {[pid |-> gpid[r], by |-> r, owner |-> r]}
    /\ child' = [child EXCEPT ![r] = CDEAD]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome,
                   fired, hook, pid, gpid, cancelled, entry, key, pub, pending,
                   dropped>>

\* OwnedProcess::wait: revoke the signalling capability, reap, detach any
\* still-registered hook. The Drop backstop is the same action — reaping a
\* dead child is always available to the owner. MUTATION = 3 skips the
\* revoke; MUTATION = 4 removes cleanup entirely.
ReapChild(r) ==
    /\ child[r] = CDEAD
    /\ MUTATION /= 4
    /\ child' = [child EXCEPT ![r] = CREAPED]
    /\ IF MUTATION = 3 THEN gpid' = gpid ELSE gpid' = [gpid EXCEPT ![r] = NOPID]
    /\ hook' = ClearIfRegistered(r)
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, outcome,
                   fired, pid, cancelled, entry, key, pub, pending, dropped,
                   signalLog>>

\* The worker claims the single outcome and queues the completion. This is
\* the emitter take in worker::spawn's finish(); the final token check in
\* transport's read() means a cancelled worker never reaches here.
WorkSucceeds(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ Settled(r)
    /\ child[r] /= CNONE      \* a remote success ran the ssh child (leaked under MUTATION = 4)
    /\ outcome[r] = NONE
    /\ outcome' = [outcome EXCEPT ![r] = OK]
    /\ fired' = [fired EXCEPT ![r] = @ + 1]
    /\ pending' = [pending EXCEPT ![r] = TRUE]
    /\ work' = [work EXCEPT ![r] = RETURNED]
    /\ hook' = ClearIfRegistered(r)
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, child, pid,
                   gpid, cancelled, entry, key, pub, dropped, signalLog>>

\* Any failure exit: typed Fault, child already settled by read_failure.
WorkFails(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ Settled(r)
    /\ outcome[r] = NONE
    /\ outcome' = [outcome EXCEPT ![r] = FAILED]
    /\ fired' = [fired EXCEPT ![r] = @ + 1]
    /\ pending' = [pending EXCEPT ![r] = TRUE]
    /\ work' = [work EXCEPT ![r] = RETURNED]
    /\ hook' = ClearIfRegistered(r)
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, child, pid,
                   gpid, cancelled, entry, key, pub, dropped, signalLog>>

\* The worker returns after its outcome was already reserved by a cancel:
\* finish() finds the emitter gone and the result is discarded.
DiscardResult(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ Settled(r)
    /\ outcome[r] /= NONE
    /\ work' = [work EXCEPT ![r] = RETURNED]
    /\ hook' = ClearIfRegistered(r)
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, outcome, fired,
                   child, pid, gpid, cancelled, entry, key, pub, pending,
                   dropped, signalLog>>

\* THE DUPLICATE-TERMINAL MUTATION (MUTATION = 2): a second emit path
\* fires after success. The single-shot emitter makes this impossible in
\* the shipped code.
EmitAgain(r) ==
    /\ MUTATION = 2
    /\ work[r] = RETURNED
    /\ outcome[r] = OK
    /\ ~cancelled[r]
    /\ fired[r] = 1
    /\ UNCHANGED outcome
    /\ fired' = [fired EXCEPT ![r] = @ + 1]
    /\ pending' = [pending EXCEPT ![r] = TRUE]
    /\ UNCHANGED <<focus, rev, alive, head, remoteDoc, nav, phase, intent, work, hook,
                   child, pid, gpid, cancelled, entry, key, pub, dropped,
                   signalLog>>

\* -- delivery ----------------------------------------------------------------
\* handle_io processes the queued completion: entry re-check, entry
\* removal, navigation release, then the freshness gate, then publication
\* (or the silent drop). MUTATION = 1 publishes without the gate.
DeliverCompletion(r) ==
    /\ pending[r]
    /\ pending' = [pending EXCEPT ![r] = FALSE]
    /\ IF nav = r THEN nav' = NONE ELSE nav' = nav
    /\ IF entry[r] THEN entry' = [entry EXCEPT ![r] = FALSE] ELSE entry' = entry
    /\ IF entry[r] /\ outcome[r] = OK /\ (Fresh(r) \/ MUTATION = 1)
       THEN /\ pub' = [pub EXCEPT ![r] =
                           [present |-> TRUE, head |-> head, focus |-> focus, rev |-> rev,
                            readonly |-> TRUE]]
            /\ remoteDoc' = TRUE
            /\ dropped' = [dropped EXCEPT ![r] = FALSE]
            /\ IF intent[r] = IREFRESH /\ head < MAXHEAD /\ focus < MAXFOCUS
               THEN /\ head' = head + 1
                    /\ focus' = focus + 1
               ELSE /\ UNCHANGED <<head, focus>>
       ELSE /\ dropped' = [dropped EXCEPT ![r] = TRUE]
            /\ UNCHANGED <<pub, head, focus, remoteDoc>>
    /\ UNCHANGED <<rev, alive, phase, intent, work, outcome, fired, hook,
                   child, pid, gpid, cancelled, key, signalLog>>

Next ==
    \/ \E r \in REQS, it \in {IOPEN, IREFRESH} : RequestNew(r, it)
    \/ Move
    \/ Edit
    \/ \E r \in REQS : UserCancel(r)
    \/ CloseOwner
    \/ NewOwner
    \/ \E r \in REQS : InstallHook(r)
    \/ \E r \in REQS : BeginSpawn(r)
    \/ \E r \in REQS : SpawnChild(r)
    \/ \E r \in REQS : HookFires(r)
    \/ \E r \in REQS : TransferDone(r)
    \/ \E r \in REQS : TransferFails(r)
    \/ \E r \in REQS : ReapChild(r)
    \/ \E r \in REQS : WorkSucceeds(r)
    \/ \E r \in REQS : WorkFails(r)
    \/ \E r \in REQS : DiscardResult(r)
    \/ \E r \in REQS : EmitAgain(r)
    \/ \E r \in REQS : DeliverCompletion(r)

Spec == Init /\ [][Next]_vars

\* -- the invariants -----------------------------------------------------------

FreshPublication ==
    \A r \in REQS :
        pub[r].present =>
            /\ pub[r].head = key[r].head
            /\ pub[r].focus = key[r].focus
            /\ pub[r].rev = key[r].rev

TerminalOnce ==
    \A r \in REQS : fired[r] =< 1

CancellationWins ==
    \A r \in REQS :
        cancelled[r] =>
            /\ ~pub[r].present
            /\ outcome[r] /= OK
            /\ (child[r] = CLIVE => hook[r] = HINFLIGHT)

SignalOwnedPid ==
    \A e \in signalLog : e.owner = e.by

NoUnownedChild ==
    \A r \in REQS : work[r] = RETURNED => child[r] \in {CNONE, CREAPED}

ReadonlyPublication ==
    \A r \in REQS : pub[r].present => pub[r].readonly = TRUE

\* -- temporal properties (FairSpec only) ---------------------------------------
\* Qualified by the fairness and OS assumptions in the module header.

EventuallyTerminal ==
    \A r \in REQS : (phase[r] = RUNNING) ~> (work[r] = RETURNED)

EventuallyReaped ==
    \A r \in REQS : (child[r] \in {CLIVE, CDEAD}) ~> (child[r] = CREAPED)

\* Weak fairness on worker-local and OS-progress actions only. The editor
\* (RequestNew/Move/Edit/UserCancel/CloseOwner) and the event loop
\* (DeliverCompletion) are the environment: no fairness is assumed for them.
A_InstallHook   == \E r \in REQS : InstallHook(r)
A_SpawnChild    == \E r \in REQS : SpawnChild(r)
A_HookFires     == \E r \in REQS : HookFires(r)
A_TransferDone  == \E r \in REQS : TransferDone(r)
A_TransferFails == \E r \in REQS : TransferFails(r)
A_ReapChild     == \E r \in REQS : ReapChild(r)
A_WorkSucceeds  == \E r \in REQS : WorkSucceeds(r)
A_WorkFails     == \E r \in REQS : WorkFails(r)
A_DiscardResult == \E r \in REQS : DiscardResult(r)

Fairness ==
    \A r \in REQS :
        /\ WF_vars(InstallHook(r))
        /\ WF_vars(BeginSpawn(r))
        /\ WF_vars(SpawnChild(r))
        /\ WF_vars(HookFires(r))
        /\ WF_vars(TransferDone(r))
        /\ WF_vars(TransferFails(r))
        /\ WF_vars(ReapChild(r))
        /\ WF_vars(WorkSucceeds(r))
        /\ WF_vars(WorkFails(r))
        /\ WF_vars(DiscardResult(r))

FairSpec == Spec /\ Fairness

\* -- coverage witnesses (checked only by the coverage config; each must    *)
(* be VIOLATED so the model cannot pass vacuously)                          *)

WitnessNoAdmission ==
    ~(\E r \in REQS : phase[r] = RUNNING)

WitnessNoPublication ==
    ~(\E r \in REQS : pub[r].present)

WitnessNoCancelledChild ==
    ~(\E r \in REQS : cancelled[r] /\ child[r] \in {CDEAD, CREAPED})

WitnessNoDroppedSuccess ==
    ~(\E r \in REQS : dropped[r] /\ outcome[r] = OK)

\* MUTATION = 0 configs check safety; FairSpec configs separately check progress
\* under the assumptions above. No universal implementation theorem is asserted.
=============================================================================
