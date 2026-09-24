---- MODULE LspWire ----
(***************************************************************************)
(* LSP binding/synchronization/cancellation/boundedness safety (0057       *)
(* VF05) over the AR06 bounded wire queue as implemented in                *)
(* crates/strop-lsp/src/client/{queue,sync,api}.rs.                        *)
(*                                                                       *)
(*   BINDING   didOpen binds (path -> document incarnation + revision)    *)
(*             and admits a versioned Open snapshot; didChange admits a   *)
(*             versioned Change; didClose removes the binding and frames  *)
(*             a Close only for an incarnation whose open was ADMITTED.   *)
(*             Versions allocate monotonically across reopens             *)
(*             (sync.rs next_version), so a stale versioned diagnostic    *)
(*             can never relabel itself; close/reopen is a new            *)
(*             incarnation (DocumentId + revision stamp).                 *)
(*                                                                       *)
(*   QUEUE     one FIFO worker frames admitted jobs in submission         *)
(*             order (queue.rs). Unsent jobs are bounded by count; a      *)
(*             superseded unsent Change coalesces into the newest         *)
(*             snapshot ONLY when no admitted barrier — a request, or an  *)
(*             open/close for the same document — sits between them: a    *)
(*             barrier's server-side meaning depends on the               *)
(*             intermediate version. Refusal is visible and               *)
(*             pre-admission: a refused Open records NO binding, a        *)
(*             refused Change leaves the recorded revision stale so the  *)
(*             next sync retries. A didClose whose Close frame does not   *)
(*             fit is dropped against a wedged connection — the           *)
(*             connection's terminal failure is the settlement, and no   *)
(*             safety property here depends on the close reaching the     *)
(*             wire.                                                     *)
(*                                                                       *)
(*   REQUESTS  every admitted request ends in exactly one terminal        *)
(*             event (api.rs R9): a reply applied under a fresh stamp,    *)
(*             a refusal/cancellation note, or settlement through the     *)
(*             connection's terminal failure event (ConnFail settles      *)
(*             pending/queued/dispatched requests atomically). Late or    *)
(*             cancelled results cannot publish into a new owner          *)
(*             (sync.rs owns / the editor's freshness check). Pre-init    *)
(*             requests are bounded and cancelled with an explicit note   *)
(*             when their document closes during startup.                 *)
(*                                                                       *)
(*   CONN      a connection failure stops framing; restart is a new       *)
(*             incarnation with a fresh queue and allocator; the old      *)
(*             binding is gone (the editor re-binds by reopening).        *)
(*                                                                       *)
(* THE invariants (VF05's named safety properties):                       *)
(*                                                                       *)
(*   TypeOK                  every variable stays in its declared         *)
(*                           finite domain                                *)
(*   WireOrdered             on the wire, a document's change/close/      *)
(*                           request frames appear only inside an open    *)
(*                           window for that document (didOpen precedes,  *)
(*                           no frames after didClose until a reopen)     *)
(*   VersionsIncrease        per connection and path, framed versions     *)
(*                           strictly increase — a reopen never reuses    *)
(*                           a version                                    *)
(*   CoalesceLegal           a request is framed only after the newest    *)
(*                           document version admitted before it is on    *)
(*                           the wire — a superseded snapshot was         *)
(*                           replaced only when no barrier separated      *)
(*   BindingOnlyIfAdmitted   a binding marked admitted names the exact    *)
(*                           Open admission that established it — a       *)
(*                           refusal records nothing                      *)
(*   TerminalOnce            every request settles at most once           *)
(*   RequestAccounted        a request in flight always has its queue     *)
(*                           slot / wire frame — never silently forgotten *)
(*   LateNeverApplies        no reply published against a stale stamp     *)
(*   Bounded                 the unsent queue and the pre-init pending    *)
(*                           pool stay within their bounds                *)
(*                                                                       *)
(* Deliberately faulty variants (configs flip MUTATION):                  *)
(*   MUTATION = 1  coalescing replaces a superseded snapshot ACROSS an    *)
(*                 admitted request barrier; must die by CoalesceLegal    *)
(*   MUTATION = 2  a refused Open still records the binding; must die     *)
(*                 by BindingOnlyIfAdmitted                               *)
(*   MUTATION = 3  didClose skips the pending-request cancellation and    *)
(*                 the startup flush dispatches them past the Close       *)
(*                 frame; must die by WireOrdered                         *)
(*   MUTATION = 4  a reply applies without the freshness check; must      *)
(*                 die by LateNeverApplies                                *)
(*   MUTATION = 5  the queue count bound is ignored; must die by          *)
(*                 Bounded                                                *)
(*   MUTATION = 6  a reply settles an already-terminal request a          *)
(*                 second time; must die by TerminalOnce                  *)
(*                                                                       *)
(* Not modeled: message framing/JSON (SftpWire-style byte bounds are a    *)
(* separate surface), workspace-wide symbol queries (document-free;       *)
(* their barrier role is folded into "any request"), incremental sync     *)
(* (the contract is full-text), diagnostic CONTEXTS (versioned replies    *)
(* are the reply path modeled here), and liveness.                        *)
(***************************************************************************)
EXTENDS Integers, FiniteSets, Sequences, TLC

CONSTANTS DOCS,        \* documents on the one modeled path (calibrated
                       \*   to one: no invariant quantifies over DOCS, and
                       \*   incarnation identity rides on rev/conn/ver)
          VER_MAX,     \* version-allocation budget per connection
          REQ_MAX,     \* request budget
          QMAX,        \* unsent wire-job bound (production: 256)
          PEND_MAX,    \* pre-init pending request bound (production: 64)
          MUTATION     \* 0 = honest; faulty variants above

\* One path is enough: the coalescing/barrier rules key on "this URI"
\* versus "any request", both representable on a single document path.
PATH == "a"
NONE == 0            \* no binding / no request id / no admission seq
REV_MAX == 2         \* editor-side document revision budget

VARIABLES alive,       \* connection up
          conn,        \* connection incarnation (restart bumps)
          ready,       \* handshake completed (finish_initialize)
          docOpen,     \* editor-side document open
          docRev,      \* editor-side document revision
          binding,     \* [doc, rev, ver, adm, openSeq] — the sync binding:
                       \* doc NONE = unbound; adm = an Open was admitted
                       \* to the wire queue; openSeq = that admission's
                       \* sequence number (identity, not position)
          nextVer,     \* version allocator (monotone across reopens)
          queue,       \* unsent admitted jobs (FIFO)
          wire,        \* frames the server observed, in order
          admLog,      \* append-only admission history (open/change/req)
          admSeq,      \* admission sequence counter
          reqState,    \* per request: unused|pending|queued|dispatched|terminal
          reqStamp,    \* per request: [doc, rev, conn]
          reqNotes,    \* terminal events emitted per request
          coalesced,   \* legal in-place replacements (witness)
          refusals,    \* visible refusals (witness)
          cancelNotes, \* pending requests cancelled at close (witness)
          lateRejected, \* stale replies rejected by freshness (witness)
          repliesApplied, \* fresh replies applied (witness)
          restarts,    \* connection restarts (witness)
          flushes,     \* startup flushes (witness)
          reopens,     \* close->reopen of the path (witness)
          staleApplies \* replies applied against a stale stamp (always 0)

NoBinding == [doc |-> NONE, rev |-> 0, ver |-> 0, adm |-> FALSE,
              openSeq |-> NONE]

Job(kind, ver, req, seq, cn) ==
    [kind |-> kind, path |-> PATH, ver |-> ver, req |-> req,
     seq |-> seq, conn |-> cn]

PendingCount == Cardinality({r \in 1..REQ_MAX : reqState[r] = "pending"})

\* The admission sequence number of the newest open/change admission on
\* connection cn strictly before s; NONE when there is none.
LastAdmBefore(s, cn) ==
    LET idx == {i \in DOMAIN admLog :
                  /\ admLog[i].kind \in {"open", "change"}
                  /\ admLog[i].conn = cn
                  /\ admLog[i].seq < s}
    IN IF idx = {} THEN NONE
       ELSE admLog[CHOOSE i \in idx :
                   \A j \in idx : admLog[j].seq <= admLog[i].seq].seq

\* The wire frame carrying admission sequence number a on connection cn
\* appears strictly before position k.
OnWireBefore(a, cn, k) ==
    \E j \in 1..(k-1) :
        /\ wire[j].conn = cn
        /\ wire[j].seq = a

\* Position i of the wire sits inside an open window for the path:
\* an open frame for it, no close since, same connection.
InOpenWindow(i) ==
    LET cn == wire[i].conn IN
    \E j \in 1..i :
        /\ wire[j].kind = "open"
        /\ wire[j].conn = cn
        /\ ~(\E k \in (j+1)..(i-1) :
               /\ wire[k].kind = "close"
               /\ wire[k].conn = cn)

TypeOK ==
    /\ alive \in BOOLEAN
    /\ conn \in 0..1
    /\ ready \in BOOLEAN
    /\ docOpen \in [DOCS -> BOOLEAN]
    /\ docRev \in [DOCS -> 0..REV_MAX]
    /\ binding \in [doc: {NONE} \cup DOCS, rev: 0..REV_MAX,
                    ver: 0..VER_MAX, adm: BOOLEAN, openSeq: 0..16]
    /\ nextVer \in 0..VER_MAX
    /\ queue \in Seq([kind: {"open", "change", "close", "request"},
                      path: {PATH}, ver: 0..VER_MAX, req: 0..REQ_MAX,
                      seq: 0..16, conn: 0..1])
    /\ wire \in Seq([kind: {"open", "change", "close", "request"},
                     path: {PATH}, ver: 0..VER_MAX, req: 0..REQ_MAX,
                     seq: 0..16, conn: 0..1])
    /\ admLog \in Seq([kind: {"open", "change", "request"}, path: {PATH},
                       ver: 0..VER_MAX, seq: 0..16, conn: 0..1])
    /\ admSeq \in 0..16
    /\ reqState \in [1..REQ_MAX -> {"unused", "pending", "queued",
                                    "dispatched", "terminal"}]
    /\ reqStamp \in [1..REQ_MAX -> [doc: {NONE} \cup DOCS,
                                    rev: 0..REV_MAX, conn: 0..1]]
    /\ reqNotes \in [1..REQ_MAX -> 0..2]
    /\ coalesced \in 0..2
    /\ refusals \in 0..3
    /\ cancelNotes \in 0..2
    /\ lateRejected \in 0..2
    /\ repliesApplied \in 0..2
    /\ restarts \in 0..1
    /\ flushes \in 0..1
    /\ reopens \in 0..2
    /\ staleApplies \in 0..1

Init ==
    /\ alive = TRUE
    /\ conn = 0
    /\ ready = FALSE
    /\ docOpen = [d \in DOCS |-> FALSE]
    /\ docRev = [d \in DOCS |-> 0]
    /\ binding = NoBinding
    /\ nextVer = 0
    /\ queue = <<>>
    /\ wire = <<>>
    /\ admLog = <<>>
    /\ admSeq = 0
    /\ reqState = [r \in 1..REQ_MAX |-> "unused"]
    /\ reqStamp = [r \in 1..REQ_MAX |-> [doc |-> NONE, rev |-> 0, conn |-> 0]]
    /\ reqNotes = [r \in 1..REQ_MAX |-> 0]
    /\ coalesced = 0
    /\ refusals = 0
    /\ cancelNotes = 0
    /\ lateRejected = 0
    /\ repliesApplied = 0
    /\ restarts = 0
    /\ flushes = 0
    /\ reopens = 0
    /\ staleApplies = 0

\* ---- editor-side document lifecycle ---------------------------------

EditorOpen(d) ==
    /\ alive
    /\ ~docOpen[d]
    /\ docOpen' = [docOpen EXCEPT ![d] = TRUE]
    /\ UNCHANGED <<alive, conn, ready, docRev, binding, nextVer, queue,
                   wire, admLog, admSeq, reqState, reqStamp, reqNotes,
                   coalesced, refusals, cancelNotes, lateRejected,
                   repliesApplied, restarts, flushes, reopens, staleApplies>>

EditorEdit(d) ==
    /\ alive
    /\ docOpen[d]
    /\ docRev[d] < REV_MAX
    /\ docRev' = [docRev EXCEPT ![d] = @ + 1]
    /\ UNCHANGED <<alive, conn, ready, docOpen, binding, nextVer, queue,
                   wire, admLog, admSeq, reqState, reqStamp, reqNotes,
                   coalesced, refusals, cancelNotes, lateRejected,
                   repliesApplied, restarts, flushes, reopens, staleApplies>>

\* did_open: bind the path to this document incarnation. Refusal is
\* pre-admission and visible: NO binding is recorded (MUTATION 2 records
\* it anyway — with an openSeq that names no admission).
DidOpen(d) ==
    /\ alive
    /\ docOpen[d]
    /\ binding.doc = NONE
    \* A reopen is a DidOpen after an admitted open on this connection:
    \* the allocator's monotone nextVer remembers it (binding.ver cannot —
    \* DidClose/Restart reset the binding to NoBinding before any reopen).
    /\ reopens' = IF nextVer > 0 /\ reopens < 2 THEN reopens + 1
                  ELSE reopens
    /\ IF ready
       THEN IF nextVer >= VER_MAX \/ (Len(queue) >= QMAX /\ MUTATION # 5)
            THEN \* Refused (queue full / versions exhausted): no binding.
                 /\ refusals' = IF refusals < 3 THEN refusals + 1
                                ELSE refusals
                 /\ IF MUTATION = 2
                    THEN /\ binding' = [doc |-> d, rev |-> docRev[d],
                                        ver |-> nextVer + 1, adm |-> TRUE,
                                        openSeq |-> admSeq + 1]
                         /\ nextVer' = nextVer + 1
                    ELSE UNCHANGED <<binding, nextVer>>
                 /\ UNCHANGED <<queue, admLog, admSeq>>
            ELSE /\ LET v == nextVer + 1
                        s == admSeq + 1
                    IN
                    /\ queue' = Append(queue, Job("open", v, NONE, s, conn))
                    /\ admLog' = Append(admLog, [kind |-> "open",
                                                 path |-> PATH, ver |-> v,
                                                 seq |-> s, conn |-> conn])
                    /\ admSeq' = s
                    /\ nextVer' = v
                    /\ binding' = [doc |-> d, rev |-> docRev[d], ver |-> v,
                                   adm |-> TRUE, openSeq |-> s]
                    /\ UNCHANGED refusals
       ELSE \* Pre-init: bind; the snapshot coalesces into the flush.
            /\ binding' = [doc |-> d, rev |-> docRev[d], ver |-> 0,
                           adm |-> FALSE, openSeq |-> NONE]
            /\ UNCHANGED <<nextVer, queue, admLog, admSeq, refusals>>
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, wire, reqState,
                   reqStamp, reqNotes, coalesced, cancelNotes,
                   lateRejected, repliesApplied, restarts, flushes,
                   staleApplies>>

\* did_change: the binding's revision advances; ready connections admit a
\* versioned Change. A superseded unsent Change for the path coalesces
\* in place ONLY when no barrier (any request, or an open/close for the
\* path) sits between (queue.rs admit_change). MUTATION 1 ignores
\* request barriers.
DidChange(d) ==
    /\ alive
    /\ docOpen[d]
    /\ binding.doc = d
    /\ binding.rev # docRev[d]
    /\ IF ~ready
       THEN \* Pre-init: coalesce into the pending open snapshot.
            /\ binding' = [binding EXCEPT !.rev = docRev[d]]
            /\ UNCHANGED <<queue, admLog, admSeq, nextVer, refusals,
                           coalesced>>
       ELSE IF nextVer >= VER_MAX
            THEN /\ refusals' = IF refusals < 3 THEN refusals + 1
                                ELSE refusals
                 /\ UNCHANGED <<queue, admLog, admSeq, nextVer, binding,
                               coalesced>>
            ELSE LET v == nextVer + 1
                     \* nearest unsent CHANGE for the path, newest first
                     \* (an unsent Open is a lifecycle barrier, never a
                     \* merge target)
                     mergeIdx == LET idx == {i \in DOMAIN queue :
                                     queue[i].kind = "change"}
                                 IN IF idx = {} THEN NONE
                                    ELSE CHOOSE i \in idx :
                                         \A j \in idx : j <= i
                     \* a barrier between the merge target and the tail?
                     barrierAfter(i) ==
                         \E k \in (i+1)..Len(queue) :
                             \/ queue[k].kind = "request"
                             \/ queue[k].kind \in {"open", "close"}
                     s == admSeq + 1
                 IN IF mergeIdx # NONE
                       /\ (~barrierAfter(mergeIdx) \/ MUTATION = 1)
                    THEN \* Coalesced: replace the slot in place. The slot
                         \* now REPRESENTS the newest admission: the frame
                         \* carries its version AND its admission identity
                         \* (production has no seq on the wire; it is the
                         \* model's instrumentation for CoalesceLegal, and
                         \* the superseded admission is never framed).
                         /\ queue' = [queue EXCEPT ![mergeIdx] =
                                      [queue[mergeIdx] EXCEPT !.ver = v,
                                                              !.seq = s]]
                         /\ admLog' = Append(admLog, [kind |-> "change",
                                                      path |-> PATH,
                                                      ver |-> v, seq |-> s,
                                                      conn |-> conn])
                         /\ admSeq' = s
                         /\ nextVer' = v
                         /\ binding' = [binding EXCEPT !.rev = docRev[d],
                                                     !.ver = v]
                         /\ coalesced' = IF coalesced < 2 THEN coalesced + 1
                                         ELSE coalesced
                         /\ UNCHANGED refusals
                    ELSE IF Len(queue) >= QMAX /\ MUTATION # 5
                         THEN \* Refused: the recorded revision stays
                              \* stale — the next sync retries.
                              /\ refusals' = IF refusals < 3
                                             THEN refusals + 1
                                             ELSE refusals
                              /\ UNCHANGED <<queue, admLog, admSeq, nextVer,
                                             binding, coalesced>>
                         ELSE /\ queue' = Append(queue,
                                     Job("change", v, NONE, s, conn))
                              /\ admLog' = Append(admLog,
                                     [kind |-> "change", path |-> PATH,
                                      ver |-> v, seq |-> s, conn |-> conn])
                              /\ admSeq' = s
                              /\ nextVer' = v
                              /\ binding' = [binding EXCEPT
                                             !.rev = docRev[d], !.ver = v]
                              /\ UNCHANGED <<refusals, coalesced>>
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, wire, reqState,
                   reqStamp, reqNotes, cancelNotes, lateRejected,
                   repliesApplied, restarts, flushes, reopens,
                   staleApplies>>

\* did_close: the binding goes; pre-init pending requests for this
\* incarnation are cancelled with an explicit terminal note (MUTATION 3
\* skips that); a Close is framed only for an admitted open.
DidClose(d) ==
    /\ alive
    /\ binding.doc = d
    /\ reopens' = reopens
    /\ IF MUTATION # 3
       THEN /\ reqState' = [r \in 1..REQ_MAX |->
                            IF reqState[r] = "pending" /\ reqStamp[r].doc = d
                            THEN "terminal" ELSE reqState[r]]
            /\ reqNotes' = [r \in 1..REQ_MAX |->
                            IF reqState[r] = "pending" /\ reqStamp[r].doc = d
                            THEN 1 ELSE reqNotes[r]]
            /\ cancelNotes' = IF \E r \in 1..REQ_MAX :
                                  reqState[r] = "pending" /\ reqStamp[r].doc = d
                              THEN IF cancelNotes < 2 THEN cancelNotes + 1
                                   ELSE cancelNotes
                              ELSE cancelNotes
       ELSE UNCHANGED <<reqState, reqNotes, cancelNotes>>
    /\ binding' = NoBinding
    /\ IF binding.adm /\ ready /\ Len(queue) < QMAX
       THEN /\ queue' = Append(queue, Job("close", 0, NONE, admSeq + 1, conn))
            /\ admSeq' = admSeq + 1
       ELSE UNCHANGED <<queue, admSeq>>
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, nextVer, wire,
                   admLog, reqStamp, coalesced, refusals, lateRejected,
                   repliesApplied, restarts, flushes, staleApplies>>

EditorClose(d) ==
    /\ alive
    /\ docOpen[d]
    /\ binding.doc # d
    /\ docOpen' = [docOpen EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<alive, conn, ready, docRev, binding, nextVer, queue,
                   wire, admLog, admSeq, reqState, reqStamp, reqNotes,
                   coalesced, refusals, cancelNotes, lateRejected,
                   repliesApplied, restarts, flushes, reopens, staleApplies>>

\* finish_initialize: the pending open (latest snapshot) flushes first;
\* still-owned pending requests dispatch after it (DispatchPending), and
\* unowned or refused ones end in exactly one terminal note. (The
\* single-path model folds the per-path open loop into one step; sorted
\* multi-path replay is a determinism detail, R11.)
FinishInit ==
    /\ alive
    /\ ~ready
    /\ flushes = 0
    \* The pending open must fit; production fails the flush otherwise.
    /\ (binding.doc = NONE \/ binding.adm) \/
       (nextVer < VER_MAX /\ Len(queue) < QMAX)
    /\ IF binding.doc # NONE /\ ~binding.adm
       THEN /\ LET v == nextVer + 1
                   s == admSeq + 1
               IN
               /\ queue' = Append(queue, Job("open", v, NONE, s, conn))
               /\ admLog' = Append(admLog, [kind |-> "open", path |-> PATH,
                                            ver |-> v, seq |-> s,
                                            conn |-> conn])
               /\ admSeq' = s
               /\ nextVer' = v
               /\ binding' = [binding EXCEPT !.ver = v, !.adm = TRUE,
                                           !.openSeq = s]
       ELSE UNCHANGED <<queue, admLog, admSeq, nextVer, binding>>
    /\ ready' = TRUE
    /\ flushes' = 1
    /\ UNCHANGED <<alive, conn, docOpen, docRev, wire, reqState, reqStamp,
                   reqNotes, coalesced, refusals, cancelNotes, lateRejected,
                   repliesApplied, restarts, reopens, staleApplies>>

\* One still-owned pending request dispatches behind every open frame;
\* an unowned one (its document closed or changed during startup) ends in
\* exactly one terminal note instead.
DispatchPending(r) ==
    /\ alive
    /\ ready
    /\ reqState[r] = "pending"
    \* MUTATION 3's other half: a pending request whose cancellation the
    \* faulty didClose skipped is dispatched anyway, past the Close frame.
    /\ IF (reqStamp[r].doc = binding.doc /\ reqStamp[r].rev = binding.rev)
          \/ MUTATION = 3
       THEN IF Len(queue) >= QMAX /\ MUTATION # 5
            THEN /\ reqState' = [reqState EXCEPT ![r] = "terminal"]
                 /\ reqNotes' = [reqNotes EXCEPT ![r] = 1]
                 /\ refusals' = IF refusals < 3 THEN refusals + 1
                                ELSE refusals
                 /\ UNCHANGED <<queue, admLog, admSeq>>
            ELSE /\ LET s == admSeq + 1 IN
                    /\ queue' = Append(queue, Job("request", 0, r, s, conn))
                    /\ admLog' = Append(admLog, [kind |-> "request",
                                                 path |-> PATH, ver |-> 0,
                                                 seq |-> s, conn |-> conn])
                    /\ admSeq' = s
                    /\ reqState' = [reqState EXCEPT ![r] = "queued"]
                    /\ UNCHANGED <<reqNotes, refusals>>
       ELSE \* Unowned: cancelled during startup, one terminal note.
            /\ reqState' = [reqState EXCEPT ![r] = "terminal"]
            /\ reqNotes' = [reqNotes EXCEPT ![r] = 1]
            /\ UNCHANGED <<queue, admLog, admSeq, refusals>>
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, binding, nextVer,
                   wire, reqStamp, coalesced, cancelNotes, lateRejected,
                   repliesApplied, restarts, flushes, reopens, staleApplies>>

\* The editor admits a request stamped with the CURRENT binding
\* incarnation. Refusal before admission is a terminal note; pre-init
\* the bounded pending pool holds it.
Request(d) ==
    /\ alive
    /\ docOpen[d]
    /\ binding.doc = d
    /\ \E r \in 1..REQ_MAX :
        /\ reqState[r] = "unused"
        /\ reqStamp' = [reqStamp EXCEPT ![r] = [doc |-> d,
                                                rev |-> docRev[d],
                                                conn |-> conn]]
        /\ IF ready
           THEN IF Len(queue) >= QMAX /\ MUTATION # 5
                THEN /\ reqState' = [reqState EXCEPT ![r] = "terminal"]
                     /\ reqNotes' = [reqNotes EXCEPT ![r] = 1]
                     /\ refusals' = IF refusals < 3 THEN refusals + 1
                                    ELSE refusals
                     /\ UNCHANGED <<queue, admLog, admSeq>>
                ELSE /\ LET s == admSeq + 1 IN
                        /\ queue' = Append(queue, Job("request", 0, r, s,
                                                      conn))
                        /\ admLog' = Append(admLog, [kind |-> "request",
                                                     path |-> PATH, ver |-> 0,
                                                     seq |-> s, conn |-> conn])
                        /\ admSeq' = s
                        /\ reqState' = [reqState EXCEPT ![r] = "queued"]
                        /\ UNCHANGED <<reqNotes, refusals>>
           ELSE IF PendingCount >= PEND_MAX
                THEN /\ reqState' = [reqState EXCEPT ![r] = "terminal"]
                     /\ reqNotes' = [reqNotes EXCEPT ![r] = 1]
                     /\ refusals' = IF refusals < 3 THEN refusals + 1
                                    ELSE refusals
                ELSE /\ reqState' = [reqState EXCEPT ![r] = "pending"]
                     /\ UNCHANGED <<reqNotes, refusals>>
                /\ UNCHANGED <<queue, admLog, admSeq>>
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, binding, nextVer,
                   wire, coalesced, cancelNotes, lateRejected,
                   repliesApplied, restarts, flushes, reopens, staleApplies>>

\* The FIFO worker frames the head job; an admitted request becomes
\* dispatched (its reply may now arrive).
Drain ==
    /\ alive
    /\ queue # <<>>
    /\ LET job == Head(queue) IN
       /\ queue' = Tail(queue)
       /\ wire' = Append(wire, job)
       /\ IF job.kind = "request"
          THEN reqState' = [reqState EXCEPT ![job.req] = "dispatched"]
          ELSE reqState' = reqState
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, binding, nextVer,
                   admLog, admSeq, reqStamp, reqNotes, coalesced, refusals,
                   cancelNotes, lateRejected, repliesApplied, restarts,
                   flushes, reopens, staleApplies>>

\* The server answers a dispatched request. THE GUARD (sync.rs owns +
\* the editor's freshness check): the stamp must name the LIVE binding
\* incarnation on the SAME connection. MUTATION 4 applies regardless;
\* MUTATION 6 settles an already-terminal request again.
Reply(r) ==
    /\ reqState[r] = "dispatched" \/ (MUTATION = 6 /\ reqState[r] = "terminal")
    /\ LET stamp == reqStamp[r]
           fresh == /\ stamp.doc = binding.doc
                    /\ stamp.rev = binding.rev
                    /\ stamp.conn = conn
                    /\ alive
       IN
       /\ IF fresh
          THEN /\ repliesApplied' = IF repliesApplied < 2
                                    THEN repliesApplied + 1
                                    ELSE repliesApplied
               /\ UNCHANGED <<lateRejected, staleApplies>>
          ELSE IF MUTATION = 4
               THEN /\ staleApplies' = 1
                    /\ UNCHANGED <<lateRejected, repliesApplied>>
               ELSE /\ lateRejected' = IF lateRejected < 2
                                       THEN lateRejected + 1
                                       ELSE lateRejected
                    /\ UNCHANGED <<repliesApplied, staleApplies>>
       /\ reqState' = [reqState EXCEPT ![r] = "terminal"]
       /\ reqNotes' = [reqNotes EXCEPT ![r] = @ + 1]
    /\ UNCHANGED <<alive, conn, ready, docOpen, docRev, binding, nextVer,
                   queue, wire, admLog, admSeq, reqStamp, coalesced,
                   refusals, cancelNotes, restarts, flushes, reopens>>

\* The connection dies: framing stops, the retained queue drops with the
\* worker, and every in-flight request settles through the terminal
\* failure event — atomically, exactly once each.
ConnFail ==
    /\ alive
    /\ alive' = FALSE
    /\ ready' = FALSE
    /\ queue' = <<>>
    /\ reqState' = [r \in 1..REQ_MAX |->
                    IF reqState[r] \in {"pending", "queued", "dispatched"}
                    THEN "terminal" ELSE reqState[r]]
    /\ reqNotes' = [r \in 1..REQ_MAX |->
                    IF reqState[r] \in {"pending", "queued", "dispatched"}
                    THEN reqNotes[r] + 1 ELSE reqNotes[r]]
    /\ UNCHANGED <<conn, docOpen, docRev, binding, nextVer, wire, admLog,
                   admSeq, reqStamp, coalesced, refusals, cancelNotes,
                   lateRejected, repliesApplied, restarts, flushes,
                   reopens, staleApplies>>

\* A new connection: fresh incarnation, fresh queue and allocator; the
\* old binding is gone (the editor re-binds by reopening).
Restart ==
    /\ ~alive
    /\ conn < 1
    /\ alive' = TRUE
    /\ conn' = conn + 1
    /\ nextVer' = 0
    /\ binding' = NoBinding
    /\ restarts' = 1
    /\ UNCHANGED <<ready, docOpen, docRev, queue, wire, admLog, admSeq,
                   reqState, reqStamp, reqNotes, coalesced, refusals,
                   cancelNotes, lateRejected, repliesApplied, flushes,
                   reopens, staleApplies>>

Next ==
    \/ \E d \in DOCS : EditorOpen(d)
    \/ \E d \in DOCS : EditorEdit(d)
    \/ \E d \in DOCS : DidOpen(d)
    \/ \E d \in DOCS : DidChange(d)
    \/ \E d \in DOCS : DidClose(d)
    \/ \E d \in DOCS : EditorClose(d)
    \/ FinishInit
    \/ \E r \in 1..REQ_MAX : DispatchPending(r)
    \/ \E d \in DOCS : Request(d)
    \/ Drain
    \/ \E r \in 1..REQ_MAX : Reply(r)
    \/ ConnFail
    \/ Restart

vars == <<alive, conn, ready, docOpen, docRev, binding, nextVer, queue,
          wire, admLog, admSeq, reqState, reqStamp, reqNotes, coalesced,
          refusals, cancelNotes, lateRejected, repliesApplied, restarts,
          flushes, reopens, staleApplies>>

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* VF05 named safety properties.                                          *)
(***************************************************************************)

\* didOpen precedes; nothing for the path after didClose until a reopen.
WireOrdered ==
    \A i \in DOMAIN wire :
        wire[i].kind \in {"change", "close", "request"} => InOpenWindow(i)

\* Per connection, framed versions strictly increase — a reopen never
\* reuses a version, so a stale versioned reply cannot relabel itself.
VersionsIncrease ==
    \A i, j \in DOMAIN wire :
        (i < j /\ wire[i].conn = wire[j].conn /\ wire[i].ver > 0
         /\ wire[j].ver > 0) => wire[i].ver < wire[j].ver

\* A request is framed only after the newest document version admitted
\* before it is on the wire: a superseded snapshot was replaced ONLY
\* when no barrier separated them.
CoalesceLegal ==
    \A k \in DOMAIN wire :
        (wire[k].kind = "request") =>
            LET L == LastAdmBefore(wire[k].seq, wire[k].conn) IN
            L = NONE \/ OnWireBefore(L, wire[k].conn, k)

\* A binding marked admitted names the exact Open admission that
\* established it — a refusal records nothing.
BindingOnlyIfAdmitted ==
    (binding.doc # NONE /\ binding.adm) =>
        \E i \in DOMAIN admLog :
            /\ admLog[i].kind = "open"
            /\ admLog[i].seq = binding.openSeq
            /\ admLog[i].conn = conn

\* Every request settles at most once.
TerminalOnce == \A r \in 1..REQ_MAX : reqNotes[r] <= 1

\* A request in flight always retains its slot: queued (a job exists) or
\* dispatched (a wire frame exists on this connection) — never silently
\* forgotten.
RequestAccounted ==
    \A r \in 1..REQ_MAX :
        /\ (reqState[r] = "queued") =>
            \E i \in DOMAIN queue : queue[i].kind = "request"
                                    /\ queue[i].req = r
        /\ (reqState[r] = "dispatched") =>
            \E i \in DOMAIN wire : wire[i].kind = "request"
                                   /\ wire[i].req = r
                                   /\ wire[i].conn = conn

\* No reply published against a stale stamp.
LateNeverApplies == staleApplies = 0

\* The unsent queue and the pre-init pending pool stay bounded.
Bounded == Len(queue) <= QMAX /\ PendingCount <= PEND_MAX

(***************************************************************************)
(* Non-vacuity witnesses: each must be REACHABLE in the honest model      *)
(* (checked as an expected-to-fail invariant over the coverage config).   *)
(***************************************************************************)

\* A superseded unsent snapshot coalesced in place.
WitnessNoCoalesce == coalesced = 0

\* A full queue refused an admission, visibly.
WitnessNoRefusal == refusals = 0

\* The path closed and reopened under a new document incarnation.
WitnessNoReopen == reopens = 0

\* A pending request was cancelled with a note when its document closed
\* during startup.
WitnessNoCancelNote == cancelNotes = 0

\* The startup flush ran (pre-init opens/requests reached the wire).
WitnessNoFlush == flushes = 0

\* A late/stale reply was rejected by the freshness check.
WitnessNoLateRejected == lateRejected = 0

\* A fresh reply was applied.
WitnessNoApplied == repliesApplied = 0

\* The connection failed, in-flight work settled, and a restart
\* re-incarnated the wire.
WitnessNoRestart == restarts = 0

=============================================================================
