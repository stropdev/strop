---- MODULE RemoteWorkspace ----
(***************************************************************************)
(* The pooled remote workspace (0036 RW3 + its consumers) at the ownership  *)
(* boundaries the Rust code actually has:                                  *)
(*                                                                       *)
(*   - crates/strop-remote/src/pool.rs (landing): RemoteClient (pure,      *)
(*     cheap clone) over a weak Registry<HashMap<RemoteEndpoint,          *)
(*     Weak<EndpointHandle>>>; ConnectionLease as the strong Arc;          *)
(*     EndpointHandle {endpoint, jobs: SyncSender<Job>, stop:             *)
(*     Arc<StopSignal>, status: Arc<StatusCell>}; the actor thread        *)
(*     receives receiver/stop/status/endpoint/spawner only — never a      *)
(*     strong self-reference. Physical {epoch, runtime, child:            *)
(*     OwnedProcess, codec, ...}; a per-actor next_epoch incremented      *)
(*     BEFORE each connect attempt (failed auth included); a global       *)
(*     AtomicU64 incarnation per spawned actor; a bounded per-endpoint    *)
(*     sync_channel(QUEUE_CAPACITY = 32) queue with strictly sequential   *)
(*     dispatch                                                          *)
(*                                                                       *)
(*   - the consumer admission/publication gates in the editor:            *)
(*     Editor::remote_window_complete() (RemoteWindow::is_complete())     *)
(*     refusing full-document LSP attach and Git full-file inputs         *)
(*     (refresh_hunks / spawn_blame_file / blame_line /                   *)
(*     open_line_history) on partial/follow windows; strop_lsp            *)
(*     RequestStamp/ReplyContext/DiagnosticContext freshness via          *)
(*     Editor::lsp_reply_fresh; strop_git RepoTarget-carrying keys        *)
(*     (ContextKey/HunkKey/BlameKey/LogKey/DiveKey) validated by          *)
(*     worker::Ticket owners in editor/git_memory/jobs.rs                 *)
(*                                                                       *)
(*   1. identity is two-level: an actor INCARNATION (global monotonic,    *)
(*      one live actor per endpoint) and a per-actor physical connection  *)
(*      EPOCH (monotonic, bumped before every connect attempt). A late    *)
(*      byte stream is identified by (incarnation, epoch) and can never   *)
(*      satisfy a request dispatched on another one                      *)
(*                                                                       *)
(*   2. the registry is weak: the entry may outlive its actor (RWEAK      *)
(*      is pruned lazily on the next insert); strong leases are held by   *)
(*      documents (dlease), an explicit user connection (ulease) and      *)
(*      every admitted request (jobs hold one from admission to their     *)
(*      terminal reply, whose emission is the worker's handle-drop        *)
(*      linearization point)                                             *)
(*                                                                       *)
(*   3. the actor is sequential: at most one active job per physical      *)
(*      stream; queued jobs are an unordered bounded set (the real        *)
(*      channel is FIFO — any dispatch order is a superset, which is      *)
(*      the safe direction for every ownership property here)             *)
(*                                                                       *)
(*   4. active cancellation retires the physical connection and its       *)
(*      epoch and fails exactly that request; a queued cancellation       *)
(*      only skips its own job and never touches the connection; a        *)
(*      failed or aborted job's reply is final — only a NEW job may       *)
(*      connect (no silent retry)                                        *)
(*                                                                       *)
(*   5. the last strong lease Drop signals stop (a plain AtomicBool       *)
(*      store) and the actor itself kills and reaps the child group       *)
(*      (revoke-before-reap is owned by RemoteRead/RemoteProcessModel;    *)
(*      teardown here is one atomic step); an explicit disconnect         *)
(*      removes the registry slot and stops the actor even mid-job —      *)
(*      the active job fails Stopped, distinct from owner Cancelled       *)
(*                                                                       *)
(*   6. consumer requests freeze ownership at admission — {endpoint,      *)
(*      kind, document, revision, window-completeness} — and full-file    *)
(*      services (full-document LSP, Git hunk/blame/line-history inputs)  *)
(*      refuse partial/follow windows with a typed refusal BEFORE any     *)
(*      lease, queue or connection exists; publication re-checks the      *)
(*      frozen capture against the live workspace (lsp_reply_fresh /      *)
(*      Ticket key freshness)                                            *)
(*                                                                       *)
(*   7. the follow submodel (FOLLOW = 1) publishes a bounded window:      *)
(*      append continuity is confirmed by comparing overlapping CONTENT,  *)
(*      never by stat; size+mtime equality proves nothing (a              *)
(*      same-size same-mtime replacement is a content change); a poll     *)
(*      result of generation g may only publish while the file is still   *)
(*      generation g (reopen-and-compare; no stale publish)               *)
(*                                                                       *)
(* THE invariants — each names an executable Rust oracle boundary that    *)
(* Main owns (re-checking the same property against the real pool):       *)
(*                                                                       *)
(*   TypeOK               every variable stays in its declared finite     *)
(*                        domain                                         *)
(*   QueueCapacity        the queued set never exceeds QUEUE_CAPACITY     *)
(*   QueueAgrees          the queued set is exactly the set of jobs in    *)
(*                        JQUEUED for that endpoint                      *)
(*                        (Rust: the Job channel is the only queue;       *)
(*                        oracle: drain a saturated channel and count)    *)
(*   ActiveMatchesStream  an active job always sits on its actor's        *)
(*                        current live connection attempt — same          *)
(*                        incarnation, same epoch — and a connecting      *)
(*                        actor always has one                           *)
(*                        (oracle: assert the actor's Physical epoch      *)
(*                        equals the in-flight job's stamp at every       *)
(*                        exchange)                                      *)
(*   LeaseIncarnation     a request's strong lease targets the endpoint's *)
(*                        live incarnation; leases die with their actor   *)
(*                        (oracle: drop all handles, respawn, assert      *)
(*                        the old lease cannot keep the new actor up)     *)
(*   RegistryAgrees       a LIVE registry entry implies a live actor;     *)
(*                        a WEAK entry implies a dead one                *)
(*                        (oracle: upgrade after last-lease shutdown      *)
(*                        must fail)                                     *)
(*   NoOrphanStream       no actor exits leaving a connection, an active  *)
(*                        job or queued jobs behind                       *)
(*                        (oracle: after disconnect/last-lease shutdown,  *)
(*                        assert the child is reaped and the channel      *)
(*                        closed)                                        *)
(*   TerminalOnce         each job fires at most one terminal reply       *)
(*                        (oracle: count reply emissions per Job across   *)
(*                        racing cancel/skip/stop/finish paths)           *)
(*   ReplyOwnsDispatch    an OK reply carries the job's own dispatch      *)
(*                        identity — endpoint, incarnation, epoch         *)
(*                        (oracle: stamp every Job with its Physical      *)
(*                        epoch at dispatch and compare on reply)         *)
(*   NoRetiredStreamReply no OK reply ever came from a retired stream     *)
(*                        — late bytes from a cancelled connection are    *)
(*                        discarded, never reattached                     *)
(*                        (oracle: cancel an active exchange, let the     *)
(*                        old ssh emit, assert the bytes are dropped      *)
(*                        and the next job's reply carries the new        *)
(*                        epoch)                                         *)
(*   NoQueuedSkipTeardown a queued job's cancellation never tears down    *)
(*                        the physical connection (ghost cevents log      *)
(*                        every teardown with its reason)                 *)
(*                        (oracle: cancel ONLY a queued job; assert the   *)
(*                        connection object and epoch are unchanged and   *)
(*                        the active job still completes)                 *)
(*   LastLeaseSignalsStop a live, un-stopped actor always has at least    *)
(*                        one strong lease holder — no self-retention,    *)
(*                        no missed Drop signal                          *)
(*                        (oracle: release every ConnectionLease, assert  *)
(*                        stop is observed and the thread exits)          *)
(*   PartialRefused       a full-file service kind admitted on a partial  *)
(*                        window ends in exactly the typed refusal —      *)
(*                        never a lease, queue slot or connection         *)
(*                        (oracle: :tail a remote file, attempt LSP       *)
(*                        attach/blame, assert refusal + no wire traffic) *)
(*   ServiceOwnership     a published service result carries its job's    *)
(*                        frozen endpoint/document/revision, and          *)
(*                        full-file semantics only for full-window        *)
(*                        captures — a partial buffer never masquerades   *)
(*                        as a complete document                         *)
(*                        (oracle: edit/rebind the buffer between         *)
(*                        request and reply, assert the result is         *)
(*                        trace-rejected and nothing publishes)           *)
(*   CancelFinal          a cancelled job is terminal and is never        *)
(*                        re-executed — no silent retry                   *)
(*   FollowWindowHonest   a published follow window is bounded and        *)
(*                        either matches the file's current generation    *)
(*                        and content, or visibly belongs to an older     *)
(*                        generation (a pending repoll)                   *)
(*                        (oracle: same-size same-mtime replacement       *)
(*                        fixture must publish a reset, and the window    *)
(*                        content must equal the file's bytes)            *)
(*                                                                       *)
(* Temporal properties (FairSpec only, under explicit assumptions):       *)
(*   EventuallyReplied    an admitted (queued or active) job eventually   *)
(*                        receives its one terminal reply                *)
(*   EventuallyShutdown   once stop is signaled the actor eventually      *)
(*                        exits (kill + reap + channel close)            *)
(* Weak fairness is assumed ONLY for actor-thread actions (Dispatch,      *)
(* SkipCancelled, ConnectOk, ConnectFail, ReplyDone, TeardownActive,      *)
(* ActorStop). OS/runtime assumptions: the actor loop runs and observes   *)
(* stop within its poll cadence; SIGKILL on the child group is delivered  *)
(* and waitid observes exit (teardown/reap abstracted as one step); a     *)
(* connect attempt resolves. Submissions, cancellations, lease            *)
(* acquisitions/releases, workspace edits, publication delivery and file  *)
(* mutations are the environment — no fairness on them. The claim that    *)
(* an idle pool with no leases eventually drains follows from            *)
(* LastLeaseSignalsStop + EventuallyShutdown only under the additional    *)
(* environment assumption that holders eventually release; it is stated   *)
(* here, not checked. Qualified claims about the abstraction, not        *)
(* proofs of OS scheduling.                                              *)
(*                                                                       *)
(* Deliberately faulty variants (one knob, no copy-pasted mutant module;  *)
(* configs flip MUTATION):                                               *)
(*   MUTATION = 1  late bytes from a retired stream are attached to a     *)
(*                 queued job as its OK reply — must die by exactly       *)
(*                 NoRetiredStreamReply and ReplyOwnsDispatch            *)
(*   MUTATION = 2  a queued job's skip also tears down the physical       *)
(*                 connection (retiring the epoch under an unrelated      *)
(*                 active job) — must die by exactly                      *)
(*                 ActiveMatchesStream and NoQueuedSkipTeardown          *)
(*   MUTATION = 3  the last lease release never signals stop (the actor   *)
(*                 as-if retains its own strong lease) — must die by      *)
(*                 exactly LastLeaseSignalsStop (and, under fairness,     *)
(*                 would never satisfy EventuallyShutdown)                *)
(*   MUTATION = 4  service results publish without the ownership gate,    *)
(*                 stamping the live workspace — must die by exactly      *)
(*                 ServiceOwnership (including the partial-window         *)
(*                 masquerade: a partial capture published full because   *)
(*                 the document happens to hold a full window now)        *)
(*   MUTATION = 5  follow continuity assumed from stat alone (no content  *)
(*                 comparison; the accepted window's content is kept) —   *)
(*                 must die by exactly FollowWindowHonest                 *)
(*                                                                       *)
(* This follows rootle's provider-protocol precedent                     *)
(* (https://rootle.dev/docs/provider-protocol.html): a bounded TLA+       *)
(* model, safety invariants plus fairness-qualified temporal properties,  *)
(* deliberately faulty variants, and executable checks — no claim of      *)
(* unbounded soundness or a Rust refinement proof. Bounded model          *)
(* checking only; TLC explores the finite constants below.                *)
(*                                                                       *)
(* Not modeled: the SFTP wire itself (SftpWire) and the read-worker       *)
(* child lifecycle fine structure (RemoteRead owns register-hook,         *)
(* revoke-before-reap, pid recycling); the supervised remote process      *)
(* group (RemoteProcessModel owns it — teardown here is one atomic        *)
(* kill+reap); FIFO dispatch order (a set of queued jobs is a             *)
(* superset of the real channel's behaviors); the worker blocking on a    *)
(* full channel (modeled as the admission guard; no dropped jobs); the    *)
(* submit-racing-a-stopping-actor corner (a stopping actor admits no      *)
(* new work — a strengthening that matches the actor checking stop       *)
(* before dispatch); the post-stop window between stop signaling and      *)
(* thread exit beyond what ActorStop models; LSP wire/encoding fields     *)
(* and diagnostics versioning; the Git surface taxonomy (log/dive that    *)
(* need no full window are collapsed away — only the full-file kinds are  *)
(* kept); the status cell (Disconnected/Connecting/Connected is derivable *)
(* from cstate + actor liveness); host completion and home expansion;     *)
(* the mtime clock (a monotonic counter abstracts it — granularity is     *)
(* Rust's problem); window sliding policy inside the bounded window; UI   *)
(* and revision combinatorics beyond one workspace document per model     *)
(* document id.                                                          *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS EPS,       \* endpoint identities, e.g. {1, 2}
          JOBS,      \* request ids, e.g. {1, 2}; allocated in order, each used once
          DOCS,      \* workspace document identities, e.g. {1}
          QCAP,      \* per-endpoint queue capacity (Rust QUEUE_CAPACITY = 32, scaled)
          MAXEPOCH,  \* per-actor connection-epoch cap for finiteness
          MAXINC,    \* global actor-incarnation cap for finiteness
          MAXREV,    \* workspace revision cap for finiteness
          FOLLOW,    \* 1 enables the follow-window publication submodel
          MUTATION   \* 0 honest; 1 late-epoch attach; 2 queued-skip teardown;
                     \* 3 no stop on last release; 4 ungated publication;
                     \* 5 stat-based follow continuity

VARIABLES estate,    \* per endpoint: the actor, its registry entry, stream state
          jstate,    \* per job: frozen request ownership and lifecycle
          wstate,    \* per document: live workspace binding + document lease
          fvar,      \* the followed remote file and its published window
          ginc,      \* the global next-actor-incarnation counter
          cevents,   \* ghost: every physical teardown with its reason
          discardCount \* ghost: late bytes from retired streams, discarded (count)

vars == <<estate, jstate, wstate, fvar, ginc, cevents, discardCount>>

\* -- symbolic codes (numbers are tags only) --------------------------------
JIDLE     == 0    \* request id not yet used
JQUEUED   == 1    \* admitted: holds a lease, sits in the endpoint queue
JACTIVE   == 2    \* dispatched on the actor's current stream
JDONE     == 3    \* terminal: the one reply has been emitted
KREAD     == 0    \* a read/window request (RemoteClient::read)
KGIT      == 1    \* a full-file Git input (hunks / blame / line history)
KLSP      == 2    \* a full-document language-server request
FULLKINDS == {KGIT, KLSP}
WFULL     == 0    \* the capture is a complete window (RemoteWindow::is_complete)
WPART     == 1    \* a partial/follow window (Tail/Range)
RNONE     == 0    \* registry slot absent
RWEAK     == 1    \* weak entry present, actor dead (lazily pruned)
RLIVE     == 2    \* entry present, actor alive
SDISC     == 0    \* no physical connection
SCONNING  == 1    \* connect attempt in flight (epoch already consumed)
SCONN     == 2    \* connection established (the owned child runs)
RNONE_R   == 0    \* reply not yet emitted
ROK       == 1
RCANCELLED == 2   \* owner token fired (active: after teardown; queued: skipped)
RFAILED   == 3    \* connect/auth failure of this job's attempt
RSTOPPED  == 4    \* the actor stopped under the job (explicit disconnect)
RREFUSED  == 5    \* typed refusal: full-file service on a partial window
RACTCANCEL == 0   \* teardown reason: active job's token
RSTOP     == 1    \* teardown reason: stop signal (last lease or disconnect)
RFAIL     == 2    \* teardown reason: connect attempt failed
RSKIP     == 3    \* teardown reason: a QUEUED job was skipped (mutant only)
CIDA == 0
CIDB == 1         \* abstract file content identities for follow
JNONE  == 0       \* no active job on the actor
UHOLDER == 0      \* the explicit user connection as a lease-holder id

FIRSTEP == CHOOSE e \in EPS : TRUE
FIRSTDOC == CHOOSE d \in DOCS : TRUE

UREC     == [live: BOOLEAN, inc: 1..MAXINC]
NOU      == [live |-> FALSE, inc |-> 1]
DREC     == [live: BOOLEAN, end: EPS, inc: 1..MAXINC]
NODLEASE == [live |-> FALSE, end |-> FIRSTEP, inc |-> 1]
RETSTRM  == [inc: 1..MAXINC, epo: 1..MAXEPOCH]
CEVENTS  == [end: EPS, inc: 1..MAXINC, epo: 1..MAXEPOCH,
             reason: {RACTCANCEL, RSTOP, RFAIL, RSKIP}]
REPS     == [kind: {RNONE_R, ROK, RCANCELLED, RFAILED, RSTOPPED, RREFUSED},
             end: EPS, inc: 0..MAXINC, epoch: 0..MAXEPOCH]
NOREP    == [kind |-> RNONE_R, end |-> FIRSTEP, inc |-> 0, epoch |-> 0]
PUBS     == [has: BOOLEAN, end: EPS, doc: DOCS, rev: 0..MAXREV, full: BOOLEAN]
NOPUB    == [has |-> FALSE, end |-> FIRSTEP, doc |-> FIRSTDOC, rev |-> 0, full |-> FALSE]
WREC     == [bound: BOOLEAN, end: EPS, rev: 0..MAXREV, full: BOOLEAN, dlease: DREC]

\* Follow submodel bounds (module-fixed; the pool bounds are config knobs).
\* WIN abstracts the bounded accepted window; MAXLEN/MAXGEN/MAXMT bound the
\* abstract file (length, generation/rotation count, mtime counter); MAXEV
\* bounds ghost event counters for finiteness.
MAXGEN == 3
MAXLEN == 3
WIN    == 2
MAXMT  == 3
MAXEV  == 3
MAXDISC == 2

PENDS  == [has: BOOLEAN, gen: 0..MAXGEN, start: 0..MAXLEN, len: 0..WIN,
           size: 0..MAXLEN, cid: {CIDA, CIDB}]
NOPEND == [has |-> FALSE, gen |-> 0, start |-> 0, len |-> 0, size |-> 0, cid |-> CIDA]

TypeOK ==
    /\ \A e \in EPS :
        /\ estate[e].alive \in BOOLEAN
        /\ estate[e].reg \in {RNONE, RWEAK, RLIVE}
        /\ estate[e].stop \in BOOLEAN
        /\ estate[e].inc \in 0..MAXINC
        /\ estate[e].cepo \in 0..MAXEPOCH
        /\ estate[e].cstate \in {SDISC, SCONNING, SCONN}
        /\ estate[e].active \in {JNONE} \union JOBS
        /\ estate[e].qd \subseteq JOBS
        /\ estate[e].retired \subseteq RETSTRM
        /\ estate[e].ulease \in UREC
    /\ \A j \in JOBS :
        /\ jstate[j].phase \in {JIDLE, JQUEUED, JACTIVE, JDONE}
        /\ jstate[j].end \in EPS
        /\ jstate[j].kind \in {KREAD, KGIT, KLSP}
        /\ jstate[j].doc \in DOCS
        /\ jstate[j].rev \in 0..MAXREV
        /\ jstate[j].win \in {WFULL, WPART}
        /\ jstate[j].tok \in BOOLEAN
        /\ jstate[j].lease \in BOOLEAN
        /\ jstate[j].inc \in 0..MAXINC
        /\ jstate[j].epo \in 0..MAXEPOCH
        /\ jstate[j].rep \in REPS
        /\ jstate[j].fired \in 0..2
        /\ jstate[j].pub \in PUBS
        /\ jstate[j].replySound \in BOOLEAN
    /\ \A d \in DOCS : wstate[d] \in WREC
    /\ fvar.gen \in 0..MAXGEN
    /\ fvar.len \in 0..MAXLEN
    /\ fvar.cid \in {CIDA, CIDB}
    /\ fvar.mtime \in 0..MAXMT
    /\ fvar.pend \in PENDS
    /\ fvar.win \in PENDS
    /\ fvar.napp \in 0..MAXEV
    /\ fvar.nres \in 0..MAXEV
    /\ fvar.nss \in 0..MAXEV
    /\ ginc \in 0..MAXINC
    /\ cevents \subseteq CEVENTS
    /\ discardCount \in 0..MAXDISC

Init ==
    /\ estate = [e \in EPS |-> [alive |-> FALSE, reg |-> RNONE, stop |-> FALSE,
                                inc |-> 0, cepo |-> 0, cstate |-> SDISC,
                                active |-> JNONE, qd |-> {}, retired |-> {},
                                ulease |-> NOU]]
    /\ jstate = [j \in JOBS |-> [phase |-> JIDLE, end |-> FIRSTEP, kind |-> KREAD,
                                 doc |-> FIRSTDOC, rev |-> 0, win |-> WFULL,
                                 tok |-> FALSE, lease |-> FALSE, inc |-> 0,
                                 epo |-> 0, rep |-> NOREP, fired |-> 0,
                                 pub |-> NOPUB, replySound |-> FALSE]]
    /\ wstate = [d \in DOCS |-> [bound |-> FALSE, end |-> FIRSTEP, rev |-> 0,
                                 full |-> TRUE, dlease |-> NODLEASE]]
    /\ fvar = [gen |-> 0, len |-> 0, cid |-> CIDA, mtime |-> 0,
               pend |-> NOPEND, win |-> NOPEND,
               napp |-> 0, nres |-> 0, nss |-> 0]
    /\ ginc = 0
    /\ cevents = {}
    /\ discardCount = 0

\* -- helpers (defined before use) ------------------------------------------
\* The strong holders of endpoint e's CURRENT incarnation: documents whose
\* lease targets (e, inc), the explicit user connection when it targets the
\* live incarnation, and every not-yet-replied admitted request on e.
HoldersOfDocs(e) ==
    {d \in DOCS : /\ wstate[d].dlease.live
                  /\ wstate[d].dlease.end = e
                  /\ wstate[d].dlease.inc = estate[e].inc}
HoldersOfUser(e) ==
    IF estate[e].ulease.live /\ estate[e].ulease.inc = estate[e].inc
    THEN {UHOLDER} ELSE {}
HoldersOfJobs(e) ==
    {j \in JOBS : /\ jstate[j].lease
                  /\ jstate[j].end = e
                  /\ jstate[j].inc = estate[e].inc}
RealHolders(e) ==
    HoldersOfDocs(e) \union HoldersOfUser(e) \union HoldersOfJobs(e)

\* What the endpoint's stop flag should be after `drop` stops holding:
\* EndpointHandle Drop signals stop iff it was the last strong reference.
\* MUTATION = 3 never signals (the actor as-if retains its own lease).
StopAfter(e, drop) ==
    IF MUTATION = 3 THEN estate[e].stop
    ELSE IF (RealHolders(e) \ drop) = {} THEN TRUE
    ELSE estate[e].stop

\* Get-or-spawn: a fresh actor incarnation with a zeroed stream. The old
\* weak entry is replaced (lazy prune-on-insert); a stale ulease cannot
\* hold the new incarnation (its inc will not match).
SpawnedEndpoint(e) ==
    [estate[e] EXCEPT !.alive = TRUE, !.reg = RLIVE, !.stop = FALSE,
                    !.inc = ginc + 1, !.cepo = 0, !.cstate = SDISC,
                    !.active = JNONE, !.qd = {}, !.retired = {}]

StreamOf(e) == [inc |-> estate[e].inc, epo |-> estate[e].cepo]
StreamLive(e) == estate[e].cstate \in {SCONNING, SCONN}

\* The consumer freshness gate: the live workspace still shows exactly what
\* the job froze at admission (editor's lsp_reply_fresh / git Ticket key
\* freshness). Window toggles bump the revision, so a window change since
\* admission also breaks freshness.
Fresh(j) ==
    /\ wstate[jstate[j].doc].bound
    /\ wstate[jstate[j].doc].end = jstate[j].end
    /\ wstate[jstate[j].doc].rev = jstate[j].rev
    /\ wstate[jstate[j].doc].full = (jstate[j].win = WFULL)

Min(a, b) == IF a =< b THEN a ELSE b

\* -- environment: submissions, cancellation, leases, workspace -------------
\* Admit one request against document d's live binding. Full-file kinds on
\* a partial/follow window get the typed refusal before any lease, queue
\* slot or connection exists (Editor::remote_window_complete gate).
SubmitJob(j, k, d) ==
    /\ jstate[j].phase = JIDLE
    /\ \A s \in JOBS : s < j => jstate[s].phase /= JIDLE    \* ids monotonic
    /\ wstate[d].bound
    /\ IF k \in FULLKINDS /\ ~wstate[d].full
       THEN /\ jstate' = [jstate EXCEPT ![j] =
                [phase |-> JDONE, end |-> wstate[d].end, kind |-> k, doc |-> d,
                 rev |-> wstate[d].rev, win |-> WPART, tok |-> FALSE,
                 lease |-> FALSE, inc |-> 0, epo |-> 0,
                 rep |-> [kind |-> RREFUSED, end |-> wstate[d].end,
                          inc |-> 0, epoch |-> 0],
                 fired |-> 1, pub |-> NOPUB, replySound |-> FALSE]]
            /\ UNCHANGED <<estate, wstate, fvar, ginc, cevents, discardCount>>
       ELSE /\ Cardinality(estate[wstate[d].end].qd) < QCAP
            /\ (IF estate[wstate[d].end].alive
                THEN /\ ~estate[wstate[d].end].stop
                     /\ estate' = [estate EXCEPT ![wstate[d].end].qd = @ \union {j}]
                     /\ ginc' = ginc
                ELSE /\ ginc < MAXINC
                     /\ estate' = [estate EXCEPT ![wstate[d].end] =
                                     [SpawnedEndpoint(wstate[d].end) EXCEPT !.qd = {j}]]
                     /\ ginc' = ginc + 1)
            /\ jstate' = [jstate EXCEPT ![j] =
                [phase |-> JQUEUED, end |-> wstate[d].end, kind |-> k, doc |-> d,
                 rev |-> wstate[d].rev,
                 win |-> IF wstate[d].full THEN WFULL ELSE WPART,
                 tok |-> FALSE, lease |-> TRUE,
                 inc |-> IF estate[wstate[d].end].alive
                         THEN estate[wstate[d].end].inc ELSE ginc + 1,
                 epo |-> 0, rep |-> NOREP, fired |-> 0, pub |-> NOPUB, replySound |-> FALSE]]
            /\ UNCHANGED <<wstate, fvar, cevents, discardCount>>

\* Fire the job's CancelToken. Whether it is queued or active decides the
\* effect — the actor actions below own that distinction.
CancelJob(j) ==
    /\ jstate[j].phase \in {JQUEUED, JACTIVE}
    /\ ~jstate[j].tok
    /\ jstate' = [jstate EXCEPT ![j].tok = TRUE]
    /\ UNCHANGED <<estate, wstate, fvar, ginc, cevents, discardCount>>

\* A document acquires a strong ConnectionLease on endpoint e
\* (get-or-spawn; prunes a dead weak entry).
DocBind(d, e) ==
    /\ ~wstate[d].dlease.live
    /\ IF estate[e].alive THEN ~estate[e].stop ELSE ginc < MAXINC
    /\ estate' = IF estate[e].alive
                 THEN estate
                 ELSE [estate EXCEPT ![e] = SpawnedEndpoint(e)]
    /\ ginc' = IF estate[e].alive THEN ginc ELSE ginc + 1
    /\ wstate' = [wstate EXCEPT ![d] = [wstate[d] EXCEPT
                    !.dlease = [live |-> TRUE, end |-> e,
                                inc |-> IF estate[e].alive
                                        THEN estate[e].inc ELSE ginc + 1]]]
    /\ UNCHANGED <<jstate, fvar, cevents, discardCount>>

\* Last document lease Drop: signals stop iff it was the last holder.
DocUnbind(d) ==
    /\ wstate[d].dlease.live
    /\ wstate' = [wstate EXCEPT ![d] = [wstate[d] EXCEPT !.dlease = NODLEASE]]
    /\ estate' = [estate EXCEPT ![wstate[d].dlease.end] =
                    [estate[wstate[d].dlease.end] EXCEPT
                     !.stop = StopAfter(wstate[d].dlease.end, {d})]]
    /\ UNCHANGED <<jstate, fvar, ginc, cevents, discardCount>>

\* RemoteClient::connect: an explicit user connection (strong lease).
UserConnect(e) ==
    /\ ~estate[e].ulease.live
    /\ IF estate[e].alive THEN ~estate[e].stop ELSE ginc < MAXINC
    /\ estate' =
        IF estate[e].alive
        THEN [estate EXCEPT ![e] = [estate[e] EXCEPT
                !.ulease = [live |-> TRUE, inc |-> estate[e].inc]]]
        ELSE [estate EXCEPT ![e] = [SpawnedEndpoint(e) EXCEPT
                !.ulease = [live |-> TRUE, inc |-> ginc + 1]]]
    /\ ginc' = IF estate[e].alive THEN ginc ELSE ginc + 1
    /\ UNCHANGED <<jstate, wstate, fvar, cevents, discardCount>>

\* RemoteClient::disconnect: registry slot removed, stop signaled even with
\* leases and jobs outstanding — the actor tears down mid-job and later
\* work re-creates a fresh incarnation.
UserDisconnect(e) ==
    /\ estate[e].alive /\ ~estate[e].stop
    /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT !.ulease = NOU,
                                        !.reg = RNONE, !.stop = TRUE]]
    /\ UNCHANGED <<jstate, wstate, fvar, ginc, cevents, discardCount>>

\* The workspace document's live binding: what Fresh compares against.
WsBind(d, e) ==
    /\ ~wstate[d].bound \/ wstate[d].end /= e
    /\ (~wstate[d].bound \/ wstate[d].rev < MAXREV)
    /\ wstate' = [wstate EXCEPT ![d] = [bound |-> TRUE, end |-> e,
                                       rev |-> IF wstate[d].bound THEN wstate[d].rev + 1 ELSE 0,
                                       full |-> TRUE,
                                       dlease |-> wstate[d].dlease]]
    /\ UNCHANGED <<estate, jstate, fvar, ginc, cevents, discardCount>>

\* A local edit moves the revision (BufferRevision).
WsEdit(d) ==
    /\ wstate[d].bound
    /\ wstate[d].rev < MAXREV
    /\ wstate' = [wstate EXCEPT ![d] = [wstate[d] EXCEPT
                    !.rev = wstate[d].rev + 1]]
    /\ UNCHANGED <<estate, jstate, fvar, ginc, cevents, discardCount>>

\* Loading a partial window (or reloading the full file) replaces the
\* window — a revision change, so captures across a window change are
\* stale by construction.
WsWindow(d) ==
    /\ wstate[d].bound
    /\ wstate[d].rev < MAXREV
    /\ wstate' = [wstate EXCEPT ![d] = [wstate[d] EXCEPT
                    !.full = ~wstate[d].full, !.rev = wstate[d].rev + 1]]
    /\ UNCHANGED <<estate, jstate, fvar, ginc, cevents, discardCount>>

\* -- actor thread ------------------------------------------------------------
\* Dispatch one queued job. On a live connection it reuses the current
\* epoch (connection reuse witness); otherwise the epoch is consumed FIRST
\* (next_epoch += 1 before the attempt — failed auth included) and the
\* connect attempt begins.
Dispatch(e) ==
    /\ estate[e].alive
    /\ ~estate[e].stop
    /\ estate[e].active = JNONE
    /\ (estate[e].cstate = SDISC => estate[e].cepo < MAXEPOCH)
    /\ \E j \in estate[e].qd :
        /\ jstate[j].phase = JQUEUED
        /\ ~jstate[j].tok
        /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT
              !.active = j,
              !.qd = estate[e].qd \ {j},
              !.cepo = IF estate[e].cstate = SDISC
                       THEN estate[e].cepo + 1 ELSE estate[e].cepo,
              !.cstate = IF estate[e].cstate = SDISC
                         THEN SCONNING ELSE estate[e].cstate]]
        /\ jstate' = [jstate EXCEPT ![j] = [jstate[j] EXCEPT
              !.phase = JACTIVE,
              !.epo = IF estate[e].cstate = SDISC
                      THEN estate[e].cepo + 1 ELSE estate[e].cepo]]
    /\ UNCHANGED <<wstate, fvar, ginc, cevents, discardCount>>

\* A queued job whose token already fired is skipped with a Cancelled
\* reply. It NEVER touches the physical connection and cannot disrupt an
\* active exchange. MUTATION = 2 also tears the connection down (retiring
\* the epoch under an unrelated active job).
SkipCancelled(e) ==
    /\ estate[e].alive
    /\ ~estate[e].stop
    /\ \E j \in estate[e].qd :
        /\ jstate[j].phase = JQUEUED
        /\ jstate[j].tok
        /\ jstate' = [jstate EXCEPT ![j] = [jstate[j] EXCEPT
              !.phase = JDONE,
              !.rep = [kind |-> RCANCELLED, end |-> jstate[j].end,
                       inc |-> 0, epoch |-> 0],
              !.fired = jstate[j].fired + 1,
              !.lease = FALSE]]
        /\ estate' =
             IF MUTATION = 2 /\ StreamLive(e)
             THEN [estate EXCEPT ![e] = [estate[e] EXCEPT
                    !.qd = estate[e].qd \ {j},
                    !.cstate = SDISC,
                    !.retired = estate[e].retired \union {StreamOf(e)},
                    !.stop = StopAfter(e, {j})]]
             ELSE [estate EXCEPT ![e] = [estate[e] EXCEPT
                    !.qd = estate[e].qd \ {j},
                    !.stop = StopAfter(e, {j})]]
    /\ cevents' =
         IF MUTATION = 2 /\ StreamLive(e)
         THEN cevents \union {[end |-> e, inc |-> estate[e].inc,
                               epo |-> estate[e].cepo, reason |-> RSKIP]}
         ELSE cevents
    /\ UNCHANGED <<wstate, fvar, ginc, discardCount>>

\* The connect attempt resolved: the owned child runs, the stream is up.
ConnectOk(e) ==
    /\ estate[e].cstate = SCONNING
    /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT !.cstate = SCONN]]
    /\ UNCHANGED <<jstate, wstate, fvar, ginc, cevents, discardCount>>

\* The connect attempt failed (auth/transport). The epoch stays consumed
\* and retired; the job's reply is final — no silent retry; only a new
\* dispatched job connects again, at a fresh epoch.
ConnectFail(e) ==
    /\ estate[e].cstate = SCONNING
    /\ estate[e].active \in JOBS
    /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT
          !.cstate = SDISC,
          !.active = JNONE,
          !.retired = estate[e].retired \union {StreamOf(e)},
          !.stop = StopAfter(e, {estate[e].active})]]
    /\ cevents' = cevents \union {[end |-> e, inc |-> estate[e].inc,
                                   epo |-> estate[e].cepo, reason |-> RFAIL]}
    /\ jstate' = [jstate EXCEPT ![estate[e].active] =
          [jstate[estate[e].active] EXCEPT
           !.phase = JDONE,
           !.rep = [kind |-> RFAILED, end |-> e, inc |-> 0, epoch |-> 0],
           !.fired = jstate[estate[e].active].fired + 1,
           !.lease = FALSE]]
    /\ UNCHANGED <<wstate, fvar, ginc, discardCount>>

\* Cancellation and stop are checked before committing a reply. The job's
\* lease release signals stop only when no document, pin or other job remains.
ReplyDone(e) ==
    /\ estate[e].cstate = SCONN
    /\ estate[e].active \in JOBS
    /\ jstate[estate[e].active].phase = JACTIVE
    /\ ~jstate[estate[e].active].tok /\ ~estate[e].stop
    /\ jstate' = [jstate EXCEPT ![estate[e].active] =
          [jstate[estate[e].active] EXCEPT
           !.phase = JDONE,
           !.rep = [kind |-> ROK, end |-> e,
                    inc |-> estate[e].inc, epoch |-> estate[e].cepo],
           !.replySound = StreamOf(e) \notin estate[e].retired,
           !.fired = jstate[estate[e].active].fired + 1,
           !.lease = FALSE]]
    /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT
          !.active = JNONE,
          !.stop = StopAfter(e, {estate[e].active})]]
    /\ UNCHANGED <<wstate, fvar, ginc, cevents, discardCount>>

\* Active cancellation or stop observed mid-exchange: retire the physical
\* connection and its epoch (the interrupted stream is discarded; its late
\* bytes can satisfy nothing), then fail exactly this job — token first:
\* owner Cancelled, otherwise Stopped (explicit disconnect).
TeardownActive(e) ==
    /\ estate[e].alive
    /\ estate[e].active \in JOBS
    /\ jstate[estate[e].active].phase = JACTIVE
    /\ jstate[estate[e].active].tok \/ estate[e].stop
    /\ jstate' = [jstate EXCEPT ![estate[e].active] =
          [jstate[estate[e].active] EXCEPT
           !.phase = JDONE,
           !.rep = IF jstate[estate[e].active].tok
                   THEN [kind |-> RCANCELLED, end |-> jstate[estate[e].active].end,
                        inc |-> 0, epoch |-> 0]
                   ELSE [kind |-> RSTOPPED, end |-> jstate[estate[e].active].end,
                        inc |-> 0, epoch |-> 0],
           !.fired = jstate[estate[e].active].fired + 1,
           !.lease = FALSE]]
    /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT
          !.active = JNONE,
          !.cstate = SDISC,
          !.retired = IF StreamLive(e)
                      THEN estate[e].retired \union {StreamOf(e)}
                      ELSE estate[e].retired,
          !.stop = StopAfter(e, {estate[e].active})]]
    /\ cevents' = IF StreamLive(e)
       THEN cevents \union {[end |-> e, inc |-> estate[e].inc,
                             epo |-> estate[e].cepo,
                             reason |-> IF jstate[estate[e].active].tok
                                         THEN RACTCANCEL ELSE RSTOP]}
       ELSE cevents
    /\ UNCHANGED <<wstate, fvar, ginc, discardCount>>

\* The actor observed stop (last-lease Drop or explicit disconnect): it
\* kills and reaps the child group, closes the channel — every queued job
\* fails Stopped (its reply receiver is gone) — and exits. The dead weak
\* registry entry is left for lazy pruning (an explicit disconnect already
\* removed the slot).
ActorStop(e) ==
    /\ estate[e].alive
    /\ estate[e].stop
    /\ estate[e].active = JNONE
    /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT
          !.alive = FALSE,
          !.reg = IF estate[e].reg = RLIVE THEN RWEAK ELSE estate[e].reg,
          !.cstate = SDISC,
          !.retired = IF StreamLive(e)
                      THEN estate[e].retired \union {StreamOf(e)}
                      ELSE estate[e].retired,
          !.qd = {}]]
    /\ cevents' = IF StreamLive(e)
       THEN cevents \union {[end |-> e, inc |-> estate[e].inc,
                             epo |-> estate[e].cepo, reason |-> RSTOP]}
       ELSE cevents
    /\ jstate' = [j \in JOBS |->
          IF j \in estate[e].qd
          THEN [jstate[j] EXCEPT
                !.phase = JDONE,
                !.rep = [kind |-> RSTOPPED, end |-> jstate[j].end,
                         inc |-> 0, epoch |-> 0],
                !.fired = jstate[j].fired + 1,
                !.lease = FALSE]
          ELSE jstate[j]]
    /\ UNCHANGED <<wstate, fvar, ginc, discardCount>>

\* A retired stream still emits late bytes (the killed child's pipes).
\* Honest: they are discarded — counted, never consumed. MUTATION = 1
\* attaches them to a queued job as its OK reply: an old-epoch answer to a
\* request that never dispatched on that stream.
StaleBytes(e) ==
    /\ estate[e].retired /= {}
    /\ discardCount < MAXDISC
    /\ IF MUTATION = 1
       THEN \E r \in estate[e].retired :
            \E j \in estate[e].qd :
                /\ jstate[j].phase = JQUEUED
                /\ jstate' = [jstate EXCEPT ![j] = [jstate[j] EXCEPT
                      !.phase = JDONE,
                      !.rep = [kind |-> ROK, end |-> e,
                               inc |-> r.inc, epoch |-> r.epo],
                      !.replySound = r \notin estate[e].retired,
                      !.fired = jstate[j].fired + 1,
                      !.lease = FALSE]]
                /\ estate' = [estate EXCEPT ![e] = [estate[e] EXCEPT
                      !.qd = estate[e].qd \ {j},
                      !.stop = StopAfter(e, {j})]]
                /\ discardCount' = discardCount + 1
       ELSE /\ estate' = estate
            /\ jstate' = jstate
            /\ discardCount' = discardCount + 1
    /\ UNCHANGED <<wstate, fvar, ginc, cevents>>

\* -- consumer publication -----------------------------------------------------
\* The worker's OK reply is delivered into the editor. The frozen capture
\* {endpoint, document, revision} must still be what the workspace shows
\* (lsp_reply_fresh / Ticket freshness); the record is stamped from the
\* live workspace, so an ungated delivery stamps a foreign identity.
\* MUTATION = 4 skips the gate.
PublishService(j) ==
    /\ jstate[j].phase = JDONE
    /\ jstate[j].rep.kind = ROK
    /\ ~jstate[j].pub.has
    /\ Fresh(j) \/ MUTATION = 4
    /\ jstate' = [jstate EXCEPT ![j] = [jstate[j] EXCEPT
          !.pub = [has |-> TRUE,
                   end |-> wstate[jstate[j].doc].end,
                   doc |-> jstate[j].doc,
                   rev |-> wstate[jstate[j].doc].rev,
                   full |-> wstate[jstate[j].doc].full]]]
    /\ UNCHANGED <<estate, wstate, fvar, ginc, cevents, discardCount>>

\* -- follow submodel (FOLLOW = 1) ---------------------------------------------
FOLLOWING == FOLLOW = 1

\* File events. Append keeps the generation; everything else — rotation,
\* replacement, truncation — starts a new generation. SameStat is the
\* identity trap: same size, same mtime, different content.
FFileAppend ==
    /\ FOLLOWING
    /\ fvar.len < MAXLEN
    /\ fvar.mtime < MAXMT
    /\ fvar' = [fvar EXCEPT !.len = fvar.len + 1, !.mtime = fvar.mtime + 1]
    /\ UNCHANGED <<estate, jstate, wstate, ginc, cevents, discardCount>>

FFileReplace ==
    /\ FOLLOWING
    /\ fvar.gen < MAXGEN
    /\ fvar.mtime < MAXMT
    /\ \E len \in 0..MAXLEN, c \in {CIDA, CIDB} :
        /\ c /= fvar.cid
        /\ fvar' = [fvar EXCEPT !.gen = fvar.gen + 1, !.len = len,
                    !.cid = c, !.mtime = fvar.mtime + 1]
    /\ UNCHANGED <<estate, jstate, wstate, ginc, cevents, discardCount>>

FFileSameStat ==
    /\ FOLLOWING
    /\ fvar.gen < MAXGEN
    /\ fvar.nss < MAXEV
    /\ fvar' = [fvar EXCEPT !.gen = fvar.gen + 1,
                !.cid = IF fvar.cid = CIDA THEN CIDB ELSE CIDA,
                !.nss = fvar.nss + 1]
    /\ UNCHANGED <<estate, jstate, wstate, ginc, cevents, discardCount>>

FFileShrink ==
    /\ FOLLOWING
    /\ fvar.gen < MAXGEN
    /\ fvar.mtime < MAXMT
    /\ fvar.len > 0
    /\ \E len \in 0..(fvar.len - 1) :
        fvar' = [fvar EXCEPT !.gen = fvar.gen + 1, !.len = len,
                 !.mtime = fvar.mtime + 1]
    /\ UNCHANGED <<estate, jstate, wstate, ginc, cevents, discardCount>>

\* One follow poll: reopen and read the CURRENT generation, then decide
\* append vs reset by comparing overlapping CONTENT (never stat).
\* MUTATION = 5 assumes continuity without the content comparison and
\* keeps the accepted window's (possibly wrong) content identity.
FollowPoll ==
    /\ FOLLOWING
    /\ ~fvar.pend.has
    /\ fvar.napp + fvar.nres < MAXEV
    /\ ~fvar.win.has \/ fvar.win.gen /= fvar.gen
         \/ fvar.win.size /= fvar.len \/ fvar.win.cid /= fvar.cid
    /\ LET Start == IF fvar.len =< WIN THEN 0 ELSE fvar.len - WIN
           AppendOK ==
               /\ fvar.win.has
               /\ fvar.len > fvar.win.size
               /\ Start < fvar.win.start + fvar.win.len
               /\ fvar.win.cid = fvar.cid
           NewWin ==
               [has |-> TRUE, gen |-> fvar.gen, start |-> Start,
                len |-> Min(fvar.len, WIN), size |-> fvar.len,
                cid |-> IF MUTATION = 5 /\ fvar.win.has /\ fvar.len >= fvar.win.size
                        THEN fvar.win.cid ELSE fvar.cid]
       IN fvar' = [fvar EXCEPT !.pend = NewWin,
                   !.napp = IF AppendOK THEN fvar.napp + 1 ELSE fvar.napp,
                   !.nres = IF AppendOK THEN fvar.nres ELSE fvar.nres + 1]
    /\ UNCHANGED <<estate, jstate, wstate, ginc, cevents, discardCount>>

\* Publish the captured snapshot. It need not still equal the remote file:
\* mutation after reading is outside the editor's knowledge. Local owner and
\* revision freshness are checked separately by the service publication model.
FollowDeliver ==
    /\ FOLLOWING
    /\ fvar.pend.has
    /\ fvar' = [fvar EXCEPT !.win = fvar.pend, !.pend = NOPEND]
    /\ UNCHANGED <<estate, jstate, wstate, ginc, cevents, discardCount>>

Next ==
    \/ \E j \in JOBS, k \in {KREAD, KGIT, KLSP}, d \in DOCS : SubmitJob(j, k, d)
    \/ \E j \in JOBS : CancelJob(j)
    \/ \E d \in DOCS, e \in EPS : DocBind(d, e)
    \/ \E d \in DOCS : DocUnbind(d)
    \/ \E e \in EPS : UserConnect(e)
    \/ \E e \in EPS : UserDisconnect(e)
    \/ \E d \in DOCS, e \in EPS : WsBind(d, e)
    \/ \E d \in DOCS : WsEdit(d)
    \/ \E d \in DOCS : WsWindow(d)
    \/ \E j \in JOBS : PublishService(j)
    \/ \E e \in EPS : Dispatch(e)
    \/ \E e \in EPS : SkipCancelled(e)
    \/ \E e \in EPS : ConnectOk(e)
    \/ \E e \in EPS : ConnectFail(e)
    \/ \E e \in EPS : ReplyDone(e)
    \/ \E e \in EPS : TeardownActive(e)
    \/ \E e \in EPS : ActorStop(e)
    \/ \E e \in EPS : StaleBytes(e)
    \/ FFileAppend \/ FFileReplace \/ FFileSameStat \/ FFileShrink
    \/ FollowPoll \/ FollowDeliver

Spec == Init /\ [][Next]_vars

\* Orthogonal content/window projection; ownership is checked by Spec/FairSpec.
FollowNext == FFileAppend \/ FFileReplace \/ FFileSameStat \/ FFileShrink
              \/ FollowPoll \/ FollowDeliver
FollowSpec == Init /\ [][FollowNext]_vars

\* -- the invariants -----------------------------------------------------------

QueueCapacity ==
    \A e \in EPS : Cardinality(estate[e].qd) =< QCAP

QueueAgrees ==
    \A e \in EPS, j \in JOBS :
        j \in estate[e].qd <=> (jstate[j].phase = JQUEUED /\ jstate[j].end = e)

ActiveMatchesStream ==
    \A e \in EPS :
        /\ (estate[e].cstate = SCONNING => estate[e].active \in JOBS)
        /\ (estate[e].active \in JOBS =>
            /\ estate[e].alive
            /\ estate[e].cstate \in {SCONNING, SCONN}
            /\ jstate[estate[e].active].phase = JACTIVE
            /\ jstate[estate[e].active].end = e
            /\ jstate[estate[e].active].inc = estate[e].inc
            /\ jstate[estate[e].active].epo = estate[e].cepo)

LeaseIncarnation ==
    \A j \in JOBS :
        jstate[j].lease /\ estate[jstate[j].end].alive =>
            jstate[j].inc = estate[jstate[j].end].inc

RegistryAgrees ==
    \A e \in EPS :
        /\ (estate[e].reg = RLIVE => estate[e].alive)
        /\ (estate[e].reg = RWEAK => ~estate[e].alive)
        /\ (estate[e].alive => estate[e].reg /= RWEAK)

NoOrphanStream ==
    \A e \in EPS :
        ~estate[e].alive =>
            /\ estate[e].cstate = SDISC
            /\ estate[e].active = JNONE
            /\ estate[e].qd = {}

TerminalOnce ==
    \A j \in JOBS : jstate[j].fired =< 1

ReplyOwnsDispatch ==
    \A j \in JOBS :
        jstate[j].rep.kind = ROK =>
            /\ jstate[j].rep.end = jstate[j].end
            /\ jstate[j].rep.inc = jstate[j].inc
            /\ jstate[j].rep.epoch = jstate[j].epo
            /\ jstate[j].phase = JDONE

\* Freeze validity when the reply commits; later disconnect cannot invalidate
\* an already completed read. A delayed retired-stream response is still a bug.
NoRetiredStreamReply ==
    \A j \in JOBS : jstate[j].rep.kind = ROK => jstate[j].replySound

NoQueuedSkipTeardown ==
    \A ev \in cevents : ev.reason /= RSKIP

LastLeaseSignalsStop ==
    \A e \in EPS :
        estate[e].alive /\ ~estate[e].stop => RealHolders(e) /= {}

PartialRefused ==
    \A j \in JOBS :
        jstate[j].kind \in FULLKINDS /\ jstate[j].win = WPART
            /\ jstate[j].phase /= JIDLE =>
                /\ jstate[j].rep.kind = RREFUSED
                /\ jstate[j].phase = JDONE
                /\ ~jstate[j].lease

ServiceOwnership ==
    \A j \in JOBS :
        jstate[j].pub.has =>
            /\ jstate[j].pub.end = jstate[j].end
            /\ jstate[j].pub.doc = jstate[j].doc
            /\ jstate[j].pub.rev = jstate[j].rev
            /\ (jstate[j].pub.full => jstate[j].win = WFULL)

CancelFinal ==
    \A j \in JOBS :
        jstate[j].rep.kind = RCANCELLED => jstate[j].phase = JDONE

FollowWindowHonest ==
    fvar.win.has =>
        /\ fvar.win.len =< WIN
        /\ fvar.win.start + fvar.win.len = fvar.win.size
        /\ \/ /\ fvar.win.gen = fvar.gen
              /\ fvar.win.cid = fvar.cid
              /\ fvar.win.start + fvar.win.len =< fvar.len
           \/ fvar.win.gen < fvar.gen

\* -- temporal properties (FairSpec only) ---------------------------------------
\* Qualified by the fairness and OS assumptions in the module header.

EventuallyReplied ==
    \A j \in JOBS :
        (jstate[j].phase \in {JQUEUED, JACTIVE}) ~> (jstate[j].phase = JDONE)

EventuallyShutdown ==
    \A e \in EPS :
        (estate[e].stop /\ estate[e].alive) ~> (~estate[e].alive)

\* Weak fairness on actor-thread actions only. Submissions, cancellation,
\* leases, workspace movement, publication delivery and file mutations are
\* the environment: no fairness is assumed for them.
Fairness ==
    \A e \in EPS :
        /\ WF_vars(Dispatch(e))
        /\ WF_vars(SkipCancelled(e))
        /\ WF_vars(ConnectOk(e))
        /\ WF_vars(ConnectFail(e))
        /\ WF_vars(ReplyDone(e))
        /\ WF_vars(TeardownActive(e))
        /\ WF_vars(ActorStop(e))

FairSpec == Spec /\ Fairness

\* -- coverage witnesses (checked only by the coverage configs; each must    *)
(* be VIOLATED so the model cannot pass vacuously)                          *)

\* connection reuse: two requests dispatched on one physical stream
WitnessNoReuse ==
    ~(\E j1, j2 \in JOBS :
        j1 /= j2
        /\ jstate[j1].inc = jstate[j2].inc
        /\ jstate[j1].inc /= 0
        /\ jstate[j1].end = jstate[j2].end
        /\ jstate[j1].epo = jstate[j2].epo
        /\ jstate[j1].epo /= 0)

\* reconnect: a second epoch inside one incarnation (after retirement)
WitnessNoReconnect ==
    ~(\E j1, j2 \in JOBS :
        jstate[j1].inc = jstate[j2].inc
        /\ jstate[j1].inc /= 0
        /\ jstate[j1].end = jstate[j2].end
        /\ jstate[j1].epo >= 1
        /\ jstate[j2].epo > jstate[j1].epo)

\* active cancellation: a dispatched job failed by its own token
WitnessNoActiveCancel ==
    ~(\E j \in JOBS :
        jstate[j].rep.kind = RCANCELLED /\ jstate[j].epo /= 0)

\* queued cancellation: a job skipped before any dispatch
WitnessNoQueuedCancel ==
    ~(\E j \in JOBS :
        jstate[j].rep.kind = RCANCELLED /\ jstate[j].epo = 0)

\* last-lease shutdown left the dead weak entry behind
WitnessNoLastLeaseShutdown ==
    ~(\E e \in EPS : estate[e].reg = RWEAK)

\* the typed partial-window refusal fired
WitnessNoRefusal ==
    ~(\E j \in JOBS : jstate[j].rep.kind = RREFUSED)

\* a service result published into the workspace
WitnessNoServicePublication ==
    ~(\E j \in JOBS : jstate[j].pub.has)

\* a job failed Stopped by an explicit disconnect under it
WitnessNoStoppedJob ==
    ~(\E j \in JOBS : jstate[j].rep.kind = RSTOPPED)

\* late bytes from a retired stream arrived and were discarded
WitnessNoStaleBytesDiscarded ==
    discardCount = 0

\* an endpoint ran a second actor incarnation
WitnessNoReincarnation ==
    ~(\E e \in EPS : estate[e].inc >= 2)

\* follow: a window was published at all
WitnessNoFollowPublish == ~fvar.win.has

\* follow: overlap-confirmed append published
WitnessNoFollowAppend == fvar.napp = 0

\* follow: a reset (replacement/shrink/first window) published
WitnessNoFollowReset == fvar.nres = 0

\* follow: a same-size same-mtime replacement occurred
WitnessNoSameStatReplacement == fvar.nss = 0

\* Configs (specs/cfg/remote-workspace*.cfg) and their expected outcomes:
\*   remote-workspace.cfg             MUTATION = 0, FOLLOW = 0: all pool
\*                                    invariants hold (clean run)
\*   remote-workspace-liveness.cfg    FairSpec: clean + EventuallyReplied,
\*                                    EventuallyShutdown (projection: one
\*                                    endpoint, MAXREV = 0; contention of
\*                                    two jobs, cancellation, disconnect
\*                                    and reconnect retained)
\*   remote-workspace-epoch.cfg       MUTATION = 1: dies by EXACTLY
\*                                    NoRetiredStreamReply, ReplyOwnsDispatch
\*   remote-workspace-queued-cancel.cfg MUTATION = 2: dies by EXACTLY
\*                                    ActiveMatchesStream, NoQueuedSkipTeardown
\*   remote-workspace-lease-release.cfg MUTATION = 3: dies by EXACTLY
\*                                    LastLeaseSignalsStop
\*   remote-workspace-stale-service.cfg MUTATION = 4: dies by EXACTLY
\*                                    ServiceOwnership
\*   remote-workspace-coverage.cfg    each WitnessNo* violated (checked one
\*                                    at a time by the gate loop)
\*   remote-workspace-follow.cfg      FOLLOW = 1, MUTATION = 0: clean incl.
\*                                    FollowWindowHonest
\*   remote-workspace-follow-stat-identity.cfg MUTATION = 5: dies by EXACTLY
\*                                    FollowWindowHonest
\*   remote-workspace-follow-coverage.cfg follow witnesses, one at a time.
\* MUTATION = 0 configs check safety; the FairSpec config separately checks
\* progress under the assumptions above. No universal implementation
(* theorem is asserted, and no run of TLC is claimed by this file alone.     *)
=============================================================================
