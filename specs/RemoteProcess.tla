---- MODULE RemoteProcess ----
(***************************************************************************)
(* The supervised remote-execution lifecycle of 0036 as designed in        *)
(* crates/strop-remote/src/exec/* + ssh.rs (RemoteCommand, run(), the      *)
(* embedded Python 3 supervisor) and strop-core's process.rs               *)
(* (OwnedProcess: hook-before-spawn, kill-at-publication,                  *)
(* revoke-before-reap). One supervised request end to end:                 *)
(*                                                                       *)
(*   LOCAL   the caller's CancelToken; OwnedProcess owns the local ssh     *)
(*           process group: the cancel resource is registered BEFORE      *)
(*           spawn, a token already cancelled refuses the spawn, a        *)
(*           cancel that fires before the pid is published kills the      *)
(*           child at publication, wait() revokes the signalling          *)
(*           capability before reaping, and run() returns only after      *)
(*           the ssh child is reaped (synchronous Drop backstop).         *)
(*                                                                       *)
(*   REMOTE  login shell execs the fixed embedded supervisor. Supervisor  *)
(*           forks the ANCHOR; anchor setsid()s (PGID = own PID),         *)
(*           ignores HUP/TERM/INT, posts a READY byte; supervisor answers *)
(*           GO on the launch-gate pipe; only then does the anchor fork   *)
(*           the WORKER (dispositions+mask reset, chdir, execvpe). The    *)
(*           worker exists only after READY->GO. Anchor waitpid(worker),  *)
(*           posts the 6-byte status record, then EXITS AND STAYS AN      *)
(*           UNREAPED ZOMBIE: the zombie is the PID/PGID reservation.     *)
(*           Supervisor reads the record, killpg(KILL) for stragglers     *)
(*           (inherited in-group descendants that outlived the worker),   *)
(*           THEN reaps the anchor, then writes the nonce-marked          *)
(*           STROP-SUP-v1 exit|signal|cancel|error line, then exits with  *)
(*           the worker's code. Line and exit status reach run() only     *)
(*           after cleanup, riding ssh's exit-status channel.             *)
(*                                                                       *)
(*   CANCEL  SSH stdin EOF is CANCEL by contract - never "deliver the     *)
(*           rest and wait". A 300ms last-chance drain lets an            *)
(*           already-in-flight worker-exit record win (recorded exit      *)
(*           beats cancel); if the window elapses first, the group gets   *)
(*           TERM -> 2s grace -> KILL (abstracted here as the single      *)
(*           FinalKill: the worker is arbitrary code, so TERM is never    *)
(*           relied on; KILL is).                                        *)
(*                                                                       *)
(*   RELAY   application stdin is relayed through a 4 MiB buffer with     *)
(*           O_NONBLOCK; poll() uses an events=0 mask so POLLHUP is       *)
(*           always reported: cancellation is observed even when the      *)
(*           data path is backpressured. EPIPE stops the relay but the    *)
(*           worker keeps running.                                       *)
(*                                                                       *)
(* HONEST LIMITS - there is NO heartbeat. Killing the local ssh does not  *)
(* prove the remote group is gone; it only causes sshd to close the       *)
(* supervisor's stdin. That EOF delivery is the environment: EofObserve   *)
(* is a nondeterministic action with NO fairness in Spec. FairSpec adds   *)
(* WF(EofObserve) strictly as the documented no-partition assumption      *)
(* (sshd delivers the channel close once TCP is up), together with        *)
(* WF(SshExits) (the exit-status channel holds) and FinalKill's           *)
(*           lethality (SIGKILL delivered to the process group). Across a *)
(* partition the model truthfully keeps the remote group alive after the  *)
(* local result is already terminal - see WitnessNoPartitionWindow.       *)
(* Excluded outright: a worker child that setsid()s out of the group      *)
(* (escape), OS refusal to deliver SIGKILL, and any progress claim about  *)
(* arbitrary worker code (WorkerExits/WorkerCrashes/WorkerReads carry no  *)
(* fairness). The policy guarantees inherited process-group descendants   *)
(* only - the straggler here is always an in-group descendant.            *)
(*                                                                       *)
(* Resource domains are independent: LPIDS is the local ssh pid space,    *)
(* RPIDS the remote anchor pid/PGID space; every signal event is tagged   *)
(* with its domain, so a wrong-domain or recycled-pid kill is visible.    *)
(*                                                                       *)
(* THE invariants - each names an executable Rust oracle boundary that    *)
(* Main owns (re-checking the same property against the real API):       *)
(*                                                                       *)
(*   TypeOK              every variable stays in its declared finite      *)
(*                       domain                                          *)
(*   TerminalOnce        each request emits at most one terminal outcome  *)
(*                       (Rust: run()'s single return; oracle: count      *)
(*                       results per request across racing cancel/finish) *)
(*   LaunchGated         the worker was forked only after the supervisor  *)
(*                       answered GO (Rust: the launch-gate pipe;         *)
(*                       oracle: cancel during the READY->GO handshake,   *)
(*                       assert no remote worker pid ever existed)        *)
(*   SignalOwnedTarget   every signal targeted a pid/PGID whose live      *)
(*                       holder is the signaller - never a recycled or    *)
(*                       foreign one, in either domain                    *)
(*                       (Rust: zombie anchor reserves the PGID until     *)
(*                       reap; killpg precedes waitpid; oracle: strace    *)
(*                       the killpg/waitpid order and pid reuse)          *)
(*   ReapAfterFinalKill  the anchor was reaped only after the final       *)
(*                       group signal (Rust: record -> killpg ->          *)
(*                       waitpid ordering; oracle: same strace)           *)
(*   NoPostEofDelivery   no stdin byte was delivered after the            *)
(*                       supervisor observed EOF - EOF commits to cancel  *)
(*                       (Rust: relay stops at POLLHUP, the 300ms window  *)
(*                       only waits for the exit record; oracle: fill     *)
(*                       >4 MiB stdin, close it, assert the worker's      *)
(*                       read() sees no further byte)                     *)
(*   OutcomeClean        an Exited/Signaled outcome was emitted only in a *)
(*                       fully cleaned-up state: record read, final kill  *)
(*                       done, anchor reaped, no live straggler, line     *)
(*                       written, ssh reaped (Rust: line + exit status    *)
(*                       ride channels that open only after cleanup;      *)
(*                       oracle: assert no observable window between       *)
(*                       worker exit and a clean result)                  *)
(*   NoUnownedSsh        once run() returned, the local ssh child is      *)
(*                       reaped or never existed                          *)
(*                       (Rust: wait()/Drop before return; oracle: assert *)
(*                       the pid is gone on every exit path)              *)
(*                                                                       *)
(* Temporal properties (FairSpec only, under the assumptions above):      *)
(*   EventuallyTerminal      once committed (cancel, EOF, worker record   *)
(*                           or closed gate) run() eventually returns     *)
(*   EventuallyRemoteSettled once committed remotely, the group is fully  *)
(*                           settled: anchor reaped or absent, no live    *)
(*                           worker, no live straggler - no orphan group  *)
(*                           after observed cancellation                  *)
(*   EventuallyEofObserved   a pending EOF (writer closed or ssh killed)  *)
(*                           is eventually observed by the supervisor -   *)
(*                           THIS is the no-partition assumption itself;  *)
(*                           it is not a claim about partitioned runs     *)
(*                                                                       *)
(* Deliberately faulty variants (one knob; configs flip MUTATION):        *)
(*   MUTATION = 1  the anchor forks the worker without waiting for GO -   *)
(*                 must die by exactly LaunchGated                       *)
(*   MUTATION = 2  the anchor is reaped before the final killpg and the   *)
(*                 capability is not revoked, so the late killpg fires on *)
(*                 a recycled/foreign PGID - must die by exactly          *)
(*                 ReapAfterFinalKill and SignalOwnedTarget (and, under   *)
(*                 fairness, by EventuallyRemoteSettled: the straggler    *)
(*                 group is orphaned when the signal hits the wrong pid)  *)
(*   MUTATION = 3  the HUP mask is dropped under backpressure: EOF is not *)
(*                 observed while the relay buffer is full, and pending   *)
(*                 input is delivered after EOF ("deliver rest and       *)
(*                 wait") - must die by exactly NoPostEofDelivery and,    *)
(*                 under fairness, by EventuallyEofObserved               *)
(*   MUTATION = 4  run() returns success on the bare exit record before   *)
(*                 cleanup finished - must die by exactly OutcomeClean    *)
(*                 and NoUnownedSsh                                      *)
(*                                                                       *)
(* Not modeled: multiple concurrent requests (RemotePool owns the pool),  *)
(* stdout/stderr capture and dropped-byte accounting, the nonce and       *)
(* stderr framing, the 20ms poll cadence (abstracted as LocalPollCancel), *)
(* the 2s TERM grace and 300ms timer values (the WindowElapse/record      *)
(* race keeps contention), worker argv/cwd validation, SupervisionKey     *)
(* bookkeeping, SupervisorError from supervisor-internal faults (the      *)
(* model's error path is the worker exec failure), local pid recycling    *)
(* (single local child), ssh transport faults (SftpWire's domain).        *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS REQS,      \* supervised request ids, e.g. {1}; one at a time here
          LPIDS,     \* local ssh pid space, e.g. {101} (its own domain)
          RPIDS,     \* remote anchor pid/PGID space, e.g. {1, 2}; the OS
                     \* may recycle reaped ones (including to "other")
          MAXBUF,    \* relay buffer capacity in chunks (the 4 MiB cap)
          MAXPEND,   \* stdin chunks the writer may have pending
          MUTATION   \* 0 honest; 1 no launch gate; 2 early reap;
                     \* 3 lost cancel under backpressure; 4 premature
                     \* success

VARIABLES phase,     \* per request: IDLE | RUNNING (admitted once)
          work,      \* per request: WORKING | RETURNED (run()'s thread)
          cancelled, \* per request: the CancelToken flag
          hook,      \* per request: cancel-resource lifecycle state
          ssh,       \* per request: the local ssh child's stage
          lpid,      \* per request: the ssh child's local pid
          lgpid,     \* per request: the local group capability target
          sup,       \* per request: the remote supervisor's stage
          gate,      \* per request: the launch gate WAIT | GO | CLOSED
          anchor,    \* per request: the anchor's stage (zombie = reserved)
          rpid,      \* per request: the anchor's remote pid (= PGID)
          rgpid,     \* per request: the remote group capability target
          worker,    \* per request: the worker's stage
          wkind,     \* per request: how the worker ended (record kind)
          ckind,     \* per request: the committed cleanup reason
          recPosted, \* per request: the anchor posted the status record
          eofR,      \* per request: the supervisor observed ssh-stdin EOF
          eofLocal,  \* per request: the local writer closed run()'s stdin
          buf,       \* per request: relay buffer fill (0..MAXBUF)
          pend,      \* per request: chunks the writer has pending
          relayBroken, \* per request: EPIPE - relay stopped, worker runs
          drainDelivered, \* per request: ghost count of post-EOF deliveries
          straggler, \* per request: an inherited in-group descendant
          finalKill, \* per request: the final killpg was issued
          line,      \* per request: the STROP-SUP-v1 line kind, or none
          resultReady, \* per request: line parsed + ssh exit status seen
          outcome,   \* per request: the single terminal outcome
          fired,     \* per request: count of terminal emissions
          emitClean, \* per request: ghost - the state was clean at emit
          signalLog, \* ghost: every signal event [dom, pid, by, owner]
          otherLive, \* ghost: a foreign remote process holds otherPid
          otherPid   \* ghost: its pid (OS reuse across RPIDS)

vars == <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate, anchor,
          rpid, rgpid, worker, wkind, ckind, recPosted, eofR, eofLocal, buf,
          pend, relayBroken, drainDelivered, straggler, finalKill, line,
          resultReady, outcome, fired, emitClean, signalLog, otherLive,
          otherPid>>

\* -- symbolic codes (numbers are tags only) --------------------------------
IDLE      == 0
RUNNING   == 1
WORKING   == 0
RETURNED  == 1
HNONE     == 0    \* cancel resource not yet installed
HLIVE     == 1    \* registered (register_cancel_resource)
HINFLIGHT == 2    \* dequeued by cancel; the hook body is executing
HCLEARED  == 3    \* consumed (fired) or detached (clear_cancel_resource)
SSHNONE   == 0
SSHSPAWNING == 1  \* native spawn started; cancel may finish first
SSHLIVE   == 2
SSHDEAD   == 3    \* exited or killed, still unreaped (pid reserved)
SSHREAPED == 4
SNONE     == 0    \* supervisor stages
SBOOT     == 1    \* python booting, anchor forked
SGATE     == 2    \* READY received; launch-gate decision pending
SRUN      == 3    \* GO sent; worker may run
SDRAIN    == 4    \* EOF observed; 300ms last-chance window
SCLEAN    == 5    \* committed; killpg/reap/line in progress
SLINE     == 6    \* cleanup done; writing the result line
SDONE     == 7    \* supervisor exited (worker's code rides ssh)
ANONE     == 0
AFORKING  == 1
AREADY    == 2    \* setsid done (PGID reserved from here)
ALIVE     == 3    \* post-GO, forking/waiting the worker
AZOMBIE   == 4    \* exited, unreaped: the PID/PGID reservation
AREAPED   == 5
GWAIT     == 0
GGO       == 1
GCLOSED   == 2
WNONE     == 0
WFORKING  == 1
WRUNNING  == 2
WDEAD     == 3
KNONE     == 0    \* worker end kinds (the status record's kind byte)
KEXIT     == 2
KSIGNAL   == 3
KERROR    == 4
KCANCEL   == 1    \* cleanup reason only - never a worker record kind
LNONE     == 0    \* STROP-SUP-v1 line kinds
LEXIT     == 1
LSIGNAL   == 2
LCANCEL   == 3
LERROR    == 4
ONONE     == 0    \* SupervisionOutcome family (SupervisorError folded in)
OEXIT     == 1    \* Exited
OSIGNAL   == 2    \* Signaled
OCANCEL   == 3    \* Cancelled
OLAUNCHFAIL == 4  \* LaunchFailure (spawn refused / gate closed / exec fail)
XNONE     == 0    \* straggler (inherited in-group descendant)
XLIVE     == 1
XKILLED   == 2
NOID      == 0
DOML      == 0    \* the local ssh domain
DOMR      == 1    \* the remote anchor-group domain
NOOWNER   == 0
FOREIGN   == 9    \* a foreign remote process (OS pid reuse)

SigEvents == [dom: {DOML, DOMR}, pid: LPIDS \union RPIDS,
              by: REQS, owner: {NOOWNER} \union REQS \union {FOREIGN}]

TypeOK ==
    /\ phase \in [REQS -> {IDLE, RUNNING}]
    /\ work \in [REQS -> {WORKING, RETURNED}]
    /\ cancelled \in [REQS -> BOOLEAN]
    /\ hook \in [REQS -> {HNONE, HLIVE, HINFLIGHT, HCLEARED}]
    /\ ssh \in [REQS -> {SSHNONE, SSHSPAWNING, SSHLIVE, SSHDEAD, SSHREAPED}]
    /\ lpid \in [REQS -> {NOID} \union LPIDS]
    /\ lgpid \in [REQS -> {NOID} \union LPIDS]
    /\ sup \in [REQS -> {SNONE, SBOOT, SGATE, SRUN, SDRAIN, SCLEAN, SLINE, SDONE}]
    /\ gate \in [REQS -> {GWAIT, GGO, GCLOSED}]
    /\ anchor \in [REQS -> {ANONE, AFORKING, AREADY, ALIVE, AZOMBIE, AREAPED}]
    /\ rpid \in [REQS -> {NOID} \union RPIDS]
    /\ rgpid \in [REQS -> {NOID} \union RPIDS]
    /\ worker \in [REQS -> {WNONE, WFORKING, WRUNNING, WDEAD}]
    /\ wkind \in [REQS -> {KNONE, KEXIT, KSIGNAL, KERROR}]
    /\ ckind \in [REQS -> {KNONE, KCANCEL, KEXIT, KSIGNAL, KERROR}]
    /\ recPosted \in [REQS -> BOOLEAN]
    /\ eofR \in [REQS -> BOOLEAN]
    /\ eofLocal \in [REQS -> BOOLEAN]
    /\ buf \in [REQS -> 0..MAXBUF]
    /\ pend \in [REQS -> 0..MAXPEND]
    /\ relayBroken \in [REQS -> BOOLEAN]
    /\ drainDelivered \in [REQS -> 0..1]
    /\ straggler \in [REQS -> {XNONE, XLIVE, XKILLED}]
    /\ finalKill \in [REQS -> BOOLEAN]
    /\ line \in [REQS -> {LNONE, LEXIT, LSIGNAL, LCANCEL, LERROR}]
    /\ resultReady \in [REQS -> BOOLEAN]
    /\ outcome \in [REQS -> {ONONE, OEXIT, OSIGNAL, OCANCEL, OLAUNCHFAIL}]
    /\ fired \in [REQS -> 0..1]
    /\ emitClean \in [REQS -> BOOLEAN]
    /\ signalLog \subseteq SigEvents
    /\ otherLive \in BOOLEAN
    /\ otherPid \in {NOID} \union RPIDS

Init ==
    /\ phase = [r \in REQS |-> IDLE]
    /\ work = [r \in REQS |-> RETURNED]
    /\ cancelled = [r \in REQS |-> FALSE]
    /\ hook = [r \in REQS |-> HNONE]
    /\ ssh = [r \in REQS |-> SSHNONE]
    /\ lpid = [r \in REQS |-> NOID]
    /\ lgpid = [r \in REQS |-> NOID]
    /\ sup = [r \in REQS |-> SNONE]
    /\ gate = [r \in REQS |-> GWAIT]
    /\ anchor = [r \in REQS |-> ANONE]
    /\ rpid = [r \in REQS |-> NOID]
    /\ rgpid = [r \in REQS |-> NOID]
    /\ worker = [r \in REQS |-> WNONE]
    /\ wkind = [r \in REQS |-> KNONE]
    /\ ckind = [r \in REQS |-> KNONE]
    /\ recPosted = [r \in REQS |-> FALSE]
    /\ eofR = [r \in REQS |-> FALSE]
    /\ eofLocal = [r \in REQS |-> FALSE]
    /\ buf = [r \in REQS |-> 0]
    /\ pend = [r \in REQS |-> 0]
    /\ relayBroken = [r \in REQS |-> FALSE]
    /\ drainDelivered = [r \in REQS |-> 0]
    /\ straggler = [r \in REQS |-> XNONE]
    /\ finalKill = [r \in REQS |-> FALSE]
    /\ line = [r \in REQS |-> LNONE]
    /\ resultReady = [r \in REQS |-> FALSE]
    /\ outcome = [r \in REQS |-> ONONE]
    /\ fired = [r \in REQS |-> 0]
    /\ emitClean = [r \in REQS |-> TRUE]
    /\ signalLog = {}
    /\ otherLive = FALSE
    /\ otherPid = NOID

\* -- helpers (defined before use) -------------------------------------------
\* clear_cancel_resource drops only a REGISTERED hook; one already dequeued
\* by cancel keeps executing.
ClearIfRegistered(r) ==
    IF hook[r] = HLIVE THEN [hook EXCEPT ![r] = HCLEARED] ELSE hook

\* The anchor reserves its pid/PGID from setsid until it is reaped; a live
\* foreign process reserves its own. SpawnAnchor/OtherSpawn pick from the
\* unreserved pids only - so recycling is real but never aliased.
ReservedRP ==
    {rpid[t] : t \in {s \in REQS : anchor[s] \in {AREADY, ALIVE, AZOMBIE}}}
        \union (IF otherLive THEN {otherPid} ELSE {})

FreeRP == RPIDS \ ReservedRP

AnchorHolders(p) ==
    {t \in REQS : /\ anchor[t] \in {AREADY, ALIVE, AZOMBIE}
                  /\ rpid[t] = p}

OwnerOfR(p) ==
    IF AnchorHolders(p) = {}
       THEN IF otherLive /\ otherPid = p THEN FOREIGN ELSE NOOWNER
       ELSE CHOOSE t \in AnchorHolders(p) : TRUE
           \* a singleton: anchors hold distinct reserved pids

\* The supervisor's result line is a pure image of the cleanup reason.
MapC(k) ==
    IF k = KCANCEL THEN LCANCEL
    ELSE IF k = KEXIT THEN LEXIT
    ELSE IF k = KSIGNAL THEN LSIGNAL
    ELSE LERROR

\* -- environment: the caller and the worker's arbitrary code (NO fairness) --

\* Admission: the caller invokes run() with a fresh token.
Start(r) ==
    /\ phase[r] = IDLE
    /\ work[r] = RETURNED
    /\ phase' = [phase EXCEPT ![r] = RUNNING]
    /\ work' = [work EXCEPT ![r] = WORKING]
    /\ cancelled' = [cancelled EXCEPT ![r] = FALSE]
    /\ hook' = [hook EXCEPT ![r] = HNONE]
    /\ ssh' = [ssh EXCEPT ![r] = SSHNONE]
    /\ lpid' = [lpid EXCEPT ![r] = NOID]
    /\ lgpid' = [lgpid EXCEPT ![r] = NOID]
    /\ sup' = [sup EXCEPT ![r] = SNONE]
    /\ gate' = [gate EXCEPT ![r] = GWAIT]
    /\ anchor' = [anchor EXCEPT ![r] = ANONE]
    /\ rpid' = [rpid EXCEPT ![r] = NOID]
    /\ rgpid' = [rgpid EXCEPT ![r] = NOID]
    /\ worker' = [worker EXCEPT ![r] = WNONE]
    /\ wkind' = [wkind EXCEPT ![r] = KNONE]
    /\ ckind' = [ckind EXCEPT ![r] = KNONE]
    /\ recPosted' = [recPosted EXCEPT ![r] = FALSE]
    /\ eofR' = [eofR EXCEPT ![r] = FALSE]
    /\ eofLocal' = [eofLocal EXCEPT ![r] = FALSE]
    /\ buf' = [buf EXCEPT ![r] = 0]
    /\ pend' = [pend EXCEPT ![r] = 0]
    /\ relayBroken' = [relayBroken EXCEPT ![r] = FALSE]
    /\ drainDelivered' = [drainDelivered EXCEPT ![r] = 0]
    /\ straggler' = [straggler EXCEPT ![r] = XNONE]
    /\ finalKill' = [finalKill EXCEPT ![r] = FALSE]
    /\ line' = [line EXCEPT ![r] = LNONE]
    /\ resultReady' = [resultReady EXCEPT ![r] = FALSE]
    /\ outcome' = [outcome EXCEPT ![r] = ONONE]
    /\ fired' = [fired EXCEPT ![r] = 0]
    /\ emitClean' = [emitClean EXCEPT ![r] = TRUE]
    /\ UNCHANGED <<signalLog, otherLive, otherPid>>

\* The token is cancelled (user close, supersede, ...). A registered hook
\* is dequeued and goes in flight.
CancelToken(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ ~cancelled[r]
    /\ cancelled' = [cancelled EXCEPT ![r] = TRUE]
    /\ hook' = IF hook[r] = HLIVE
               THEN [hook EXCEPT ![r] = HINFLIGHT]
               ELSE hook
    /\ UNCHANGED <<phase, work, ssh, lpid, lgpid, sup, gate, anchor, rpid,
                   rgpid, worker, wkind, ckind, recPosted, eofR, eofLocal,
                   buf, pend, relayBroken, drainDelivered, straggler,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* The writer feeds application stdin (bounded, finite).
WriterSubmits(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ ~eofLocal[r]
    /\ pend[r] < MAXPEND
    /\ sup[r] \in {SBOOT, SGATE, SRUN}
    /\ pend' = [pend EXCEPT ![r] = @ + 1]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* Finite stdin ends: the writer closes run()'s stdin. Over the wire this
\* becomes the supervisor's EOF - CANCEL by contract, subject to the
\* last-chance drain.
WriterEof(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ ~eofLocal[r]
    /\ sup[r] \in {SBOOT, SGATE, SRUN}
    /\ eofLocal' = [eofLocal EXCEPT ![r] = TRUE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, buf, pend, relayBroken, drainDelivered, straggler,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* The worker reads a buffered chunk (arbitrary code; may never read).
WorkerReads(r) ==
    /\ worker[r] = WRUNNING
    /\ buf[r] > 0
    /\ buf' = [buf EXCEPT ![r] = @ - 1]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* The worker closed its stdin: EPIPE stops the relay; the worker runs on
\* (shutdown responses must survive).
WorkerClosesStdin(r) ==
    /\ worker[r] = WRUNNING
    /\ ~relayBroken[r]
    /\ relayBroken' = [relayBroken EXCEPT ![r] = TRUE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, drainDelivered, straggler,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* The worker exits normally (arbitrary code; no fairness assumed).
WorkerExits(r) ==
    /\ worker[r] = WRUNNING
    /\ worker' = [worker EXCEPT ![r] = WDEAD]
    /\ wkind' = [wkind EXCEPT ![r] = KEXIT]
    /\ recPosted' = [recPosted EXCEPT ![r] = TRUE]
    /\ anchor' = [anchor EXCEPT ![r] = AZOMBIE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, rgpid, ckind, eofR, eofLocal, buf, pend,
                   relayBroken, drainDelivered, straggler, finalKill, line,
                   resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* The worker dies by its own signal (crash) - a record with kind=signal
\* needs no cancellation at all.
WorkerCrashes(r) ==
    /\ worker[r] = WRUNNING
    /\ worker' = [worker EXCEPT ![r] = WDEAD]
    /\ wkind' = [wkind EXCEPT ![r] = KSIGNAL]
    /\ recPosted' = [recPosted EXCEPT ![r] = TRUE]
    /\ anchor' = [anchor EXCEPT ![r] = AZOMBIE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, rgpid, ckind, eofR, eofLocal, buf, pend,
                   relayBroken, drainDelivered, straggler, finalKill, line,
                   resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* The worker forked an in-group descendant before exiting: the straggler
\* that outlives a normal worker exit and is swept only by the final
\* killpg. (A child that setsid()s OUT of the group is excluded by policy.)
StragglerAppears(r) ==
    /\ worker[r] = WRUNNING
    /\ straggler[r] = XNONE
    /\ straggler' = [straggler EXCEPT ![r] = XLIVE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* OS pid reuse on the remote host: a foreign process may take a pid the
\* reaped anchor released. Independent of the protocol; no fairness.
OtherSpawn ==
    /\ ~otherLive
    /\ FreeRP /= {}
    /\ \E p \in FreeRP :
        /\ otherLive' = TRUE
        /\ otherPid' = p
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog>>

OtherExit ==
    /\ otherLive
    /\ otherLive' = FALSE
    /\ otherPid' = NOID
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog>>

\* -- local side: OwnedProcess over the ssh child ----------------------------

\* Register the cancellation resource BEFORE spawning. Registering after
\* the token was cancelled refuses the spawn outright (LaunchFailure).
InstallHook(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ hook[r] = HNONE
    /\ hook' = IF cancelled[r]
               THEN [hook EXCEPT ![r] = HCLEARED]
               ELSE [hook EXCEPT ![r] = HLIVE]
    /\ UNCHANGED <<phase, work, cancelled, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

BeginSpawnSsh(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ hook[r] = HLIVE
    /\ ~cancelled[r]
    /\ ssh[r] = SSHNONE
    /\ ssh' = [ssh EXCEPT ![r] = SSHSPAWNING]
    /\ UNCHANGED <<phase, work, cancelled, hook, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* The ssh child appears and its pid is published under the group lock.
\* The remote supervisor boots and forks the anchor in the same breath.
\* If the hook already fired, the eventually-published child is killed at
\* publication - that is the cancel-during-creation race.
SpawnSsh(r) ==
    /\ ssh[r] = SSHSPAWNING
    /\ hook[r] \in {HLIVE, HINFLIGHT, HCLEARED}
    /\ \E p \in LPIDS :
        /\ lpid' = [lpid EXCEPT ![r] = p]
        /\ lgpid' = [lgpid EXCEPT ![r] = p]
        /\ ssh' = [ssh EXCEPT ![r] = IF cancelled[r] THEN SSHDEAD ELSE SSHLIVE]
        /\ sup' = [sup EXCEPT ![r] = SBOOT]
        /\ anchor' = [anchor EXCEPT ![r] = AFORKING]
    /\ UNCHANGED <<phase, work, cancelled, hook, gate, rpid, rgpid, worker,
                   wkind, ckind, recPosted, eofR, eofLocal, buf, pend,
                   relayBroken, drainDelivered, straggler, finalKill, line,
                   resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* The token was cancelled before the spawn: OwnedProcess::spawn refuses;
\* run() returns LaunchFailure. No child ever existed.
SpawnRefused(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ cancelled[r]
    /\ hook[r] = HCLEARED
    /\ ssh[r] = SSHNONE
    /\ outcome[r] = ONONE
    /\ outcome' = [outcome EXCEPT ![r] = OLAUNCHFAIL]
    /\ fired' = [fired EXCEPT ![r] = @ + 1]
    /\ work' = [work EXCEPT ![r] = RETURNED]
    /\ UNCHANGED <<phase, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, emitClean,
                   signalLog, otherLive, otherPid>>

\* The cancel hook body: SIGKILL the local ssh process group. It fires at
\* whatever the capability currently targets; before publication there is
\* nothing to signal yet (SpawnSsh then publishes dead).
HookFires(r) ==
    /\ hook[r] = HINFLIGHT
    /\ hook' = [hook EXCEPT ![r] = HCLEARED]
    /\ signalLog' = IF lgpid[r] = NOID
                    THEN signalLog
                    ELSE signalLog \union {[dom |-> DOML, pid |-> lgpid[r],
                                            by |-> r, owner |-> r]}
    /\ ssh' = IF lgpid[r] = NOID THEN ssh
              ELSE IF ssh[r] = SSHLIVE THEN [ssh EXCEPT ![r] = SSHDEAD]
              ELSE ssh
    /\ UNCHANGED <<phase, work, cancelled, lpid, lgpid, sup, gate, anchor,
                   rpid, rgpid, worker, wkind, ckind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, otherLive, otherPid>>

\* run()'s 20ms is_cancelled poll, fused with the synchronous Drop
\* backstop: the still-live ssh is killed AND reaped before Cancelled is
\* returned. A parsed exit status beats the poll (EmitExec wins that race).
LocalPollCancel(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ cancelled[r]
    /\ outcome[r] = ONONE
    /\ ~resultReady[r]
    /\ ssh[r] \in {SSHLIVE, SSHDEAD, SSHREAPED}
    /\ outcome' = [outcome EXCEPT ![r] = OCANCEL]
    /\ fired' = [fired EXCEPT ![r] = @ + 1]
    /\ work' = [work EXCEPT ![r] = RETURNED]
    /\ hook' = ClearIfRegistered(r)
    /\ ssh' = [ssh EXCEPT ![r] = SSHREAPED]
    /\ lgpid' = [lgpid EXCEPT ![r] = NOID]
    /\ signalLog' = IF ssh[r] = SSHLIVE /\ lgpid[r] /= NOID
                    THEN signalLog \union {[dom |-> DOML, pid |-> lgpid[r],
                                            by |-> r, owner |-> r]}
                    ELSE signalLog
    /\ UNCHANGED <<phase, cancelled, lpid, sup, gate, anchor, rpid, rgpid,
                   worker, wkind, ckind, recPosted, eofR, eofLocal, buf,
                   pend, relayBroken, drainDelivered, straggler, finalKill,
                   line, resultReady, emitClean, otherLive, otherPid>>

\* -- remote side: supervisor / anchor / worker ------------------------------

\* The anchor setsids (PGID = own pid, reserved from here) and posts the
\* READY byte; the supervisor reaches the gate decision.
AnchorReady(r) ==
    /\ sup[r] = SBOOT
    /\ anchor[r] = AFORKING
    /\ \E p \in FreeRP :
        /\ rpid' = [rpid EXCEPT ![r] = p]
        /\ rgpid' = [rgpid EXCEPT ![r] = p]
        /\ anchor' = [anchor EXCEPT ![r] = AREADY]
        /\ sup' = [sup EXCEPT ![r] = SGATE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, gate,
                   worker, wkind, ckind, recPosted, eofR, eofLocal, buf,
                   pend, relayBroken, drainDelivered, straggler, finalKill,
                   line, resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* THE LAUNCH GATE: READY seen, no cancellation latched - answer GO.
SupGo(r) ==
    /\ sup[r] = SGATE
    /\ gate[r] = GWAIT
    /\ ~eofR[r]
    /\ anchor[r] = AREADY
    /\ gate' = [gate EXCEPT ![r] = GGO]
    /\ sup' = [sup EXCEPT ![r] = SRUN]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, anchor,
                   rpid, rgpid, worker, wkind, ckind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* Cancellation arrived before GO: the gate closes without GO and the
\* cleanup commit starts - the anchor will exit without forking a worker.
GateClose(r) ==
    /\ sup[r] = SGATE
    /\ gate[r] = GWAIT
    /\ eofR[r]
    /\ gate' = [gate EXCEPT ![r] = GCLOSED]
    /\ sup' = [sup EXCEPT ![r] = SCLEAN]
    /\ ckind' = [ckind EXCEPT ![r] = KCANCEL]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, anchor,
                   rpid, rgpid, worker, wkind, recPosted, eofR, eofLocal, buf,
                   pend, relayBroken, drainDelivered, straggler, finalKill,
                   line, resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* Only a GO'd anchor forks the worker. MUTATION = 1 removes the gate: the
\* anchor forks without waiting for GO (and even after the gate closed).
ForkWorker(r) ==
    /\ anchor[r] = AREADY
    /\ worker[r] = WNONE
    /\ (MUTATION = 1 \/ gate[r] = GGO)
    /\ anchor' = [anchor EXCEPT ![r] = ALIVE]
    /\ worker' = [worker EXCEPT ![r] = WFORKING]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, rgpid, wkind, ckind, recPosted, eofR, eofLocal,
                   buf, pend, relayBroken, drainDelivered, straggler,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* execvpe succeeded: the worker runs (arbitrary native argv/cwd).
WorkerRuns(r) ==
    /\ worker[r] = WFORKING
    /\ worker' = [worker EXCEPT ![r] = WRUNNING]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, wkind, ckind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* execvpe failed: the anchor posts the error record (kind=error).
WorkerExecFails(r) ==
    /\ worker[r] = WFORKING
    /\ worker' = [worker EXCEPT ![r] = WDEAD]
    /\ wkind' = [wkind EXCEPT ![r] = KERROR]
    /\ recPosted' = [recPosted EXCEPT ![r] = TRUE]
    /\ anchor' = [anchor EXCEPT ![r] = AZOMBIE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, rgpid, ckind, eofR, eofLocal, buf, pend,
                   relayBroken, drainDelivered, straggler, finalKill, line,
                   resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* The supervisor reads the record outside the drain window: normal exit
\* path, cleanup commits with the record's kind.
ReadRecord(r) ==
    /\ sup[r] = SRUN
    /\ recPosted[r]
    /\ ckind' = [ckind EXCEPT ![r] = wkind[r]]
    /\ sup' = [sup EXCEPT ![r] = SCLEAN]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, gate,
                   anchor, rpid, rgpid, worker, wkind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* The 300ms last-chance window: an already-posted record arriving inside
\* it wins - the recorded exit beats cancel.
ReadRecordDrain(r) ==
    /\ sup[r] = SDRAIN
    /\ recPosted[r]
    /\ ckind' = [ckind EXCEPT ![r] = wkind[r]]
    /\ sup' = [sup EXCEPT ![r] = SCLEAN]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, gate,
                   anchor, rpid, rgpid, worker, wkind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* The window elapsed first: EOF commits to cancel; the group is killed.
\* The nondeterministic ReadRecordDrain/WindowElapse race IS the timer.
WindowElapse(r) ==
    /\ sup[r] = SDRAIN
    /\ ckind' = [ckind EXCEPT ![r] = KCANCEL]
    /\ sup' = [sup EXCEPT ![r] = SCLEAN]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, gate,
                   anchor, rpid, rgpid, worker, wkind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* Gate closed pre-GO: the anchor (blocked on the gate pipe) reads EOF and
\* exits without a worker - straight to zombie, straight to reservation.
AnchorExitNoGo(r) ==
    /\ gate[r] = GCLOSED
    /\ anchor[r] = AREADY
    /\ anchor' = [anchor EXCEPT ![r] = AZOMBIE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, rgpid, worker, wkind, ckind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* THE OBSERVED EOF. This is the environment assumption, not a mechanism:
\* sshd delivers the channel close caused by the killed local ssh or the
\* closed writer. No heartbeat exists to enforce it across a partition.
\* Honest poll() uses an events=0 mask, so POLLHUP is reported regardless
\* of the buffer; MUTATION = 3 is the faulty mask that needs drain room.
EofObserve(r) ==
    /\ phase[r] = RUNNING
    /\ ~eofR[r]
    /\ sup[r] \in {SBOOT, SGATE, SRUN}
    /\ (eofLocal[r] \/ ssh[r] \in {SSHDEAD, SSHREAPED})
    /\ (MUTATION /= 3 \/ buf[r] < MAXBUF \/ relayBroken[r])
    /\ eofR' = [eofR EXCEPT ![r] = TRUE]
    /\ IF sup[r] = SGATE
       THEN /\ gate' = [gate EXCEPT ![r] = GCLOSED]
            /\ sup' = [sup EXCEPT ![r] = SCLEAN]
            /\ ckind' = [ckind EXCEPT ![r] = KCANCEL]
       ELSE IF sup[r] = SRUN
            THEN /\ sup' = [sup EXCEPT ![r] = SDRAIN]
                 /\ UNCHANGED <<gate, ckind>>
            ELSE /\ UNCHANGED <<gate, ckind, sup>>
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, anchor,
                   rpid, rgpid, worker, wkind, recPosted, eofLocal, buf,
                   pend, relayBroken, drainDelivered, straggler, finalKill,
                   line, resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

\* THE FINAL GROUP SIGNAL: TERM -> 2s grace -> KILL, abstracted to the
\* KILL that always lands (OS assumption). Kills every live in-group
\* member: straggler, worker, anchor. The signal targets the capability,
\* and the capability is only honest while the anchor is unreaped.
\* MUTATION = 2 reaps early without revoking, so this fires on a recycled
\* or foreign PGID - and then the group's own members are NOT the ones
\* signalled: they survive, orphaned.
FinalKill(r) ==
    /\ sup[r] = SCLEAN
    /\ ~finalKill[r]
    /\ rgpid[r] /= NOID
    /\ (MUTATION = 2 \/ anchor[r] \in {AREADY, ALIVE, AZOMBIE})
    /\ finalKill' = [finalKill EXCEPT ![r] = TRUE]
    /\ signalLog' = signalLog \union {[dom |-> DOMR, pid |-> rgpid[r],
                                      by |-> r, owner |-> OwnerOfR(rgpid[r])]}
    /\ IF OwnerOfR(rgpid[r]) = r
       THEN /\ anchor' = [anchor EXCEPT ![r] =
                             IF anchor[r] \in {AREADY, ALIVE} THEN AZOMBIE ELSE @]
            /\ worker' = [worker EXCEPT ![r] =
                             IF worker[r] \in {WFORKING, WRUNNING} THEN WDEAD ELSE @]
            /\ wkind' = [wkind EXCEPT ![r] =
                             IF worker[r] \in {WFORKING, WRUNNING} THEN KSIGNAL ELSE @]
            /\ straggler' = [straggler EXCEPT ![r] =
                             IF straggler[r] = XLIVE THEN XKILLED ELSE @]
       ELSE /\ UNCHANGED <<anchor, worker, wkind, straggler>>
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, rgpid, ckind, recPosted, eofR, eofLocal, buf, pend,
                   relayBroken, drainDelivered, line, resultReady, outcome,
                   fired, emitClean, otherLive, otherPid>>

\* Reap the anchor - only AFTER the final kill in the honest spec, and the
\* zombie's PID/PGID reservation is released here. MUTATION = 2 drops the
\* ordering and the revoke.
ReapAnchor(r) ==
    /\ sup[r] = SCLEAN
    /\ anchor[r] = AZOMBIE
    /\ (MUTATION = 2 \/ finalKill[r])
    /\ anchor' = [anchor EXCEPT ![r] = AREAPED]
    /\ rgpid' = IF MUTATION = 2 THEN rgpid
                ELSE [rgpid EXCEPT ![r] = NOID]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   rpid, worker, wkind, ckind, recPosted, eofR, eofLocal,
                   buf, pend, relayBroken, drainDelivered, straggler,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* Cleanup complete: the nonce-marked result line is written, only now.
WriteLine(r) ==
    /\ sup[r] = SCLEAN
    /\ finalKill[r]
    /\ anchor[r] = AREAPED
    /\ sup' = [sup EXCEPT ![r] = SLINE]
    /\ line' = [line EXCEPT ![r] = MapC(ckind[r])]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, gate,
                   rpid, rgpid, worker, wkind, ckind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, anchor, finalKill, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* The supervisor exits with the worker's code.
SupExit(r) ==
    /\ sup[r] = SLINE
    /\ sup' = [sup EXCEPT ![r] = SDONE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* The exit-status channel: ssh exits once the supervisor did. A fair
\* assumption only while the connection holds (no heartbeat enforces it).
SshExits(r) ==
    /\ ssh[r] = SSHLIVE
    /\ sup[r] = SDONE
    /\ ssh' = [ssh EXCEPT ![r] = SSHDEAD]
    /\ UNCHANGED <<phase, work, cancelled, hook, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, outcome, fired,
                   emitClean, signalLog, otherLive, otherPid>>

\* run() parses the captured line; combined with the observed exit status
\* this is the emit gate that lets a recorded exit beat a cancel.
ResultReadyAct(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ outcome[r] = ONONE
    /\ ssh[r] \in {SSHDEAD, SSHREAPED}
    /\ line[r] /= LNONE
    /\ resultReady' = [resultReady EXCEPT ![r] = TRUE]
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* OwnedProcess::wait on the ssh child: revoke the capability, then reap.
\* run() cannot return a result before its child is settled.
ReapSsh(r) ==
    /\ ssh[r] = SSHDEAD
    /\ work[r] = WORKING
    /\ ssh' = [ssh EXCEPT ![r] = SSHREAPED]
    /\ lgpid' = [lgpid EXCEPT ![r] = NOID]
    /\ hook' = ClearIfRegistered(r)
    /\ UNCHANGED <<phase, work, cancelled, lpid, sup, gate, anchor, rpid,
                   rgpid, worker, wkind, ckind, recPosted, eofR, eofLocal,
                   buf, pend, relayBroken, drainDelivered, straggler,
                   finalKill, line, resultReady, outcome, fired, emitClean,
                   signalLog, otherLive, otherPid>>

\* THE TERMINAL EMIT. Honest: only after the exit status was observed AND
\* the ssh child was reaped AND the remote is fully settled - the line and
\* status could not exist otherwise. The 20ms poll loses to this race.
\* MUTATION = 4 emits on the bare record, before cleanup finished.
EmitExec(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ outcome[r] = ONONE
    /\ IF MUTATION = 4
       THEN ckind[r] \in {KEXIT, KSIGNAL, KERROR}
       ELSE /\ resultReady[r]
            /\ ssh[r] = SSHREAPED
            /\ sup[r] = SDONE
            /\ anchor[r] = AREAPED
            /\ finalKill[r]
            /\ straggler[r] /= XLIVE
            /\ worker[r] \notin {WFORKING, WRUNNING}
    /\ outcome' = [outcome EXCEPT ![r] =
                       IF ckind[r] = KEXIT THEN OEXIT
                       ELSE IF ckind[r] = KSIGNAL THEN OSIGNAL
                       ELSE IF ckind[r] = KCANCEL THEN OCANCEL
                       ELSE OLAUNCHFAIL]
    /\ fired' = [fired EXCEPT ![r] = @ + 1]
    /\ work' = [work EXCEPT ![r] = RETURNED]
    /\ hook' = ClearIfRegistered(r)
    /\ emitClean' = [emitClean EXCEPT ![r] =
                        /\ anchor[r] = AREAPED
                        /\ finalKill[r]
                        /\ straggler[r] /= XLIVE
                        /\ worker[r] \notin {WFORKING, WRUNNING}
                        /\ sup[r] = SDONE
                        /\ ssh[r] = SSHREAPED
                        /\ resultReady[r]]
    /\ UNCHANGED <<phase, cancelled, ssh, lpid, lgpid, sup, gate, anchor,
                   rpid, rgpid, worker, wkind, ckind, recPosted, eofR,
                   eofLocal, buf, pend, relayBroken, drainDelivered,
                   straggler, finalKill, line, resultReady, signalLog,
                   otherLive, otherPid>>

\* -- relay ------------------------------------------------------------------
\* Nonblocking relay into the bounded buffer. Honest: stops the moment EOF
\* is observed - EOF commits to cancel, the window never delivers stdin.
\* MUTATION = 3 keeps delivering after EOF ("deliver the rest and wait").
RelayChunk(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ pend[r] > 0
    /\ buf[r] < MAXBUF
    /\ ~relayBroken[r]
    /\ sup[r] \in {SBOOT, SGATE, SRUN}
    /\ (MUTATION = 3 \/ ~eofR[r])
    /\ buf' = [buf EXCEPT ![r] = @ + 1]
    /\ pend' = [pend EXCEPT ![r] = @ - 1]
    /\ drainDelivered' = IF eofR[r]
                         THEN [drainDelivered EXCEPT ![r] = 1]
                         ELSE drainDelivered
    /\ UNCHANGED <<phase, work, cancelled, hook, ssh, lpid, lgpid, sup, gate,
                   anchor, rpid, rgpid, worker, wkind, ckind, recPosted,
                   eofR, eofLocal, relayBroken, straggler, finalKill, line,
                   resultReady, outcome, fired, emitClean, signalLog,
                   otherLive, otherPid>>

Next ==
    \/ \E r \in REQS : Start(r)
    \/ \E r \in REQS : CancelToken(r)
    \/ \E r \in REQS : WriterSubmits(r)
    \/ \E r \in REQS : WriterEof(r)
    \/ \E r \in REQS : WorkerReads(r)
    \/ \E r \in REQS : WorkerClosesStdin(r)
    \/ \E r \in REQS : WorkerExits(r)
    \/ \E r \in REQS : WorkerCrashes(r)
    \/ \E r \in REQS : StragglerAppears(r)
    \/ OtherSpawn
    \/ OtherExit
    \/ \E r \in REQS : InstallHook(r)
    \/ \E r \in REQS : BeginSpawnSsh(r)
    \/ \E r \in REQS : SpawnSsh(r)
    \/ \E r \in REQS : SpawnRefused(r)
    \/ \E r \in REQS : HookFires(r)
    \/ \E r \in REQS : LocalPollCancel(r)
    \/ \E r \in REQS : AnchorReady(r)
    \/ \E r \in REQS : SupGo(r)
    \/ \E r \in REQS : GateClose(r)
    \/ \E r \in REQS : ForkWorker(r)
    \/ \E r \in REQS : WorkerRuns(r)
    \/ \E r \in REQS : WorkerExecFails(r)
    \/ \E r \in REQS : ReadRecord(r)
    \/ \E r \in REQS : ReadRecordDrain(r)
    \/ \E r \in REQS : WindowElapse(r)
    \/ \E r \in REQS : AnchorExitNoGo(r)
    \/ \E r \in REQS : EofObserve(r)
    \/ \E r \in REQS : FinalKill(r)
    \/ \E r \in REQS : ReapAnchor(r)
    \/ \E r \in REQS : WriteLine(r)
    \/ \E r \in REQS : SupExit(r)
    \/ \E r \in REQS : SshExits(r)
    \/ \E r \in REQS : ResultReadyAct(r)
    \/ \E r \in REQS : ReapSsh(r)
    \/ \E r \in REQS : EmitExec(r)
    \/ \E r \in REQS : RelayChunk(r)

Spec == Init /\ [][Next]_vars

\* -- the invariants -----------------------------------------------------------

TerminalOnce ==
    \A r \in REQS : fired[r] =< 1

\* The worker exists only behind the READY->GO gate.
LaunchGated ==
    \A r \in REQS :
        worker[r] \in {WFORKING, WRUNNING, WDEAD} => gate[r] = GGO

\* Every signal - local ssh group or remote anchor group - targeted a
\* pid/PGID whose live holder was the signaller itself.
SignalOwnedTarget ==
    \A e \in signalLog : e.owner = e.by

\* The anchor zombie holds the PGID reservation until the final group
\* signal has been issued; only then is it reaped.
ReapAfterFinalKill ==
    \A r \in REQS : anchor[r] = AREAPED => finalKill[r]

\* EOF commits to cancel: not one stdin chunk is delivered afterwards.
NoPostEofDelivery ==
    \A r \in REQS : drainDelivered[r] = 0

\* An Exited/Signaled result implies the state was fully cleaned up at
\* emit time. (Cancelled/LaunchFailure make no such claim: the remote
\* cleanup may still be in flight - or partitioned away.)
OutcomeClean ==
    \A r \in REQS :
        outcome[r] \in {OEXIT, OSIGNAL} => emitClean[r]

\* run() never returns while it still owns a live or unreaped ssh child.
NoUnownedSsh ==
    \A r \in REQS :
        work[r] = RETURNED => ssh[r] \in {SSHNONE, SSHREAPED}

\* -- temporal properties (FairSpec only, under the header's assumptions) ------

\* Committed = the request's fate is sealed locally: the token fired, the
\* writer closed stdin, the supervisor observed EOF, the worker's record
\* was posted, or the gate closed without GO.
Committed(r) ==
    cancelled[r] \/ eofR[r] \/ eofLocal[r] \/ recPosted[r] \/ gate[r] = GCLOSED

EventuallyTerminal ==
    \A r \in REQS : Committed(r) ~> (work[r] = RETURNED)

SettledRemote(r) ==
    /\ anchor[r] \in {ANONE, AREAPED}
    /\ worker[r] \notin {WFORKING, WRUNNING}
    /\ straggler[r] /= XLIVE

\* No orphan group after observed cancellation or a worker record - the
\* "no orphan" half of the exec contract, valid only where the EOF
\* delivery and SIGKILL assumptions hold.
EventuallyRemoteSettled ==
    \A r \in REQS :
        (recPosted[r] \/ eofR[r] \/ gate[r] = GCLOSED \/ cancelled[r])
            ~> SettledRemote(r)

\* A pending EOF is eventually observed by the supervisor. This property
\* IS the no-partition assumption (WF on EofObserve); in plain Spec it is
\* not claimed at all, and MUTATION = 3 breaks it under backpressure.
EofPending(r) ==
    /\ phase[r] = RUNNING
    /\ work[r] = WORKING
    /\ ~eofR[r]
    /\ sup[r] \in {SBOOT, SGATE, SRUN}
    /\ (eofLocal[r] \/ ssh[r] = SSHDEAD)

EventuallyEofObserved ==
    \A r \in REQS : EofPending(r) ~> (eofR[r] \/ SettledRemote(r))

\* -- fairness ------------------------------------------------------------------
\* Weak fairness for the committed protocol code only: OwnedProcess and
\* run() (local actions), the supervisor/anchor state machine, and the
\* relay thread. TWO of these are explicitly ASSUMPTIONS, not mechanisms:
\*   EofObserve  sshd delivers the channel close (no heartbeat enforces
\*               it; across a partition the remote group honestly
\*               outlives the local result)
\*   SshExits    the exit-status channel holds (ssh terminates once the
\*               remote command did)
\* FinalKill additionally assumes SIGKILL delivery to the process group.
\* NOT fair: the caller (Start/CancelToken/WriterSubmits/WriterEof), the
\* worker's arbitrary code (WorkerReads/WorkerClosesStdin/WorkerExits/
\* WorkerCrashes/StragglerAppears) and the OS reuse scheduler
\* (OtherSpawn/OtherExit). These are qualified claims about the
\* abstraction, not proofs of OS or network scheduling.
Fairness ==
    \A r \in REQS :
        /\ WF_vars(InstallHook(r))
        /\ WF_vars(BeginSpawnSsh(r))
        /\ WF_vars(SpawnSsh(r))
        /\ WF_vars(SpawnRefused(r))
        /\ WF_vars(HookFires(r))
        /\ WF_vars(LocalPollCancel(r))
        /\ WF_vars(AnchorReady(r))
        /\ WF_vars(SupGo(r))
        /\ WF_vars(GateClose(r))
        /\ WF_vars(ForkWorker(r))
        /\ WF_vars(WorkerRuns(r))
        /\ WF_vars(WorkerExecFails(r))
        /\ WF_vars(ReadRecord(r))
        /\ WF_vars(ReadRecordDrain(r))
        /\ WF_vars(WindowElapse(r))
        /\ WF_vars(AnchorExitNoGo(r))
        /\ WF_vars(EofObserve(r))
        /\ WF_vars(FinalKill(r))
        /\ WF_vars(ReapAnchor(r))
        /\ WF_vars(WriteLine(r))
        /\ WF_vars(SupExit(r))
        /\ WF_vars(SshExits(r))
        /\ WF_vars(ResultReadyAct(r))
        /\ WF_vars(ReapSsh(r))
        /\ WF_vars(EmitExec(r))
        /\ WF_vars(RelayChunk(r))

FairSpec == Spec /\ Fairness

\* -- coverage witnesses (checked only by the coverage config; each must    *)
\* be VIOLATED so the model cannot pass vacuously)                          *)

WitnessNoAdmission ==
    ~(\E r \in REQS : phase[r] = RUNNING)

WitnessNoLaunch ==
    ~(\E r \in REQS : gate[r] = GGO)

WitnessNoWorkerRun ==
    ~(\E r \in REQS : worker[r] = WRUNNING)

WitnessNoExitOutcome ==
    ~(\E r \in REQS : outcome[r] = OEXIT)

\* The 300ms window: cancellation observed while the worker still runs.
WitnessNoCancelDuringRun ==
    ~(\E r \in REQS : eofR[r] /\ worker[r] = WRUNNING)

\* The events=0 mask: EOF observed with the relay buffer completely full.
WitnessNoBackpressuredEof ==
    ~(\E r \in REQS : eofR[r] /\ buf[r] = MAXBUF)

\* An inherited in-group descendant swept by the final killpg.
WitnessNoStragglerCleanup ==
    ~(\E r \in REQS : straggler[r] = XKILLED)

\* THE HONESTY WINDOW: the local ssh is already dead, the remote has not
\* observed EOF yet. Local death alone proves nothing about the remote.
WitnessNoPartitionWindow ==
    ~(\E r \in REQS : cancelled[r] /\ ssh[r] = SSHDEAD /\ ~eofR[r])

\* MUTATION = 0 configs check safety; FairSpec configs separately check
\* progress under the assumptions above. No universal implementation
\* theorem is asserted.
=============================================================================
