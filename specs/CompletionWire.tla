---- MODULE CompletionWire ----
(***************************************************************************
0059 physical completion requests and notification flush ordering.

LspWire.tla continues to cover binding/open/close/coalescing. This extension
models the new boundary its historical dispatched/terminal states do not:
a logically cancelled SENT completion still owns its physical request slot
until reply/connection close. Query and resolve share this bounded registry.
An outer-queue notification dequeue only STARTS a flush. A request cannot be
polled/sent before the preceding full-document notification physically flushes.

Transport framing is atomic in this model. Header/body/node bounds, actual
RPC-ID correspondence and cancellation between decoder quanta are checked by
the production Rust regressions, not proved by this finite abstraction.

Mutants: 1 free a sent slot on logical cancellation; 2 cross an unfinished
notification flush; 3 issue a second terminal outcome; 4 ignore physical slot
admission; 5 lose admitted requests at connection close.
***************************************************************************)
EXTENDS Integers, Sequences, FiniteSets, TLC
CONSTANTS REQUESTS, VERSIONS, OUTSTANDING, QCAP, MUTATION
VARIABLE s
vars == <<s>>
Ids == 1..REQUESTS
Job(kind, q, v) == [kind |-> kind, request |-> q, version |-> v]
Init == s = [next |-> 0, revision |-> 0, serverRevision |-> 0,
             queue |-> <<>>, flushing |-> -1, closed |-> FALSE,
             slots |-> {}, sent |-> {}, phase |-> [q \in Ids |-> "unused"],
             terminal |-> [q \in Ids |-> 0], captured |-> [q \in Ids |-> 0],
             cancelled |-> {}, badOrder |-> FALSE, replied |-> FALSE,
             cancelledSent |-> FALSE, flushed |-> FALSE]
Edit == /\ ~s.closed /\ s.revision < VERSIONS
        /\ s' = [s EXCEPT !.revision = @ + 1]
Admit == /\ ~s.closed /\ s.next < REQUESTS /\ Len(s.queue) + 2 <= QCAP
         /\ (Cardinality(s.slots) < OUTSTANDING \/ MUTATION = 4)
         /\ LET q == s.next + 1 IN
            s' = [s EXCEPT !.next = q, !.slots = @ \cup {q},
                 !.phase[q] = "queued", !.captured[q] = s.revision,
                 !.queue = Append(Append(@, Job("sync", 0, s.revision)), Job("request", q, s.revision))]
StartFlush == /\ ~s.closed /\ s.flushing = -1 /\ Len(s.queue) > 0
              /\ Head(s.queue).kind = "sync"
              /\ s' = [s EXCEPT !.flushing = Head(s.queue).version, !.queue = Tail(@)]
FinishFlush == /\ ~s.closed /\ s.flushing # -1
               /\ s' = [s EXCEPT !.serverRevision = s.flushing, !.flushing = -1, !.flushed = TRUE]
Send == /\ ~s.closed /\ Len(s.queue) > 0 /\ Head(s.queue).kind = "request"
        /\ (s.flushing = -1 \/ MUTATION = 2)
        /\ LET q == Head(s.queue).request IN
           /\ s.phase[q] = "queued"
           /\ s' = [s EXCEPT !.queue = Tail(@), !.phase[q] = "sent", !.sent = @ \cup {q},
                            !.badOrder = @ \/ s.serverRevision # s.captured[q] \/ s.flushing # -1]
DropCancelledBarrier == /\ ~s.closed /\ Len(s.queue) > 0 /\ Head(s.queue).kind = "request"
                        /\ s.phase[Head(s.queue).request] = "done"
                        /\ s' = [s EXCEPT !.queue = Tail(@)]
Cancel(q) == /\ q \in Ids /\ ~s.closed /\ s.phase[q] \in {"queued", "sent"} /\ s.terminal[q] = 0
             /\ s' = [s EXCEPT !.terminal[q] = 1, !.cancelled = @ \cup {q},
                  !.slots = IF s.phase[q] = "queued" \/ MUTATION = 1 THEN @ \ {q} ELSE @,
                  !.phase[q] = IF @ = "queued" THEN "done" ELSE @,
                  !.cancelledSent = @ \/ s.phase[q] = "sent"]
Reply(q) == /\ q \in s.sent /\ ~s.closed
            /\ s' = [s EXCEPT !.sent = @ \ {q}, !.slots = @ \ {q}, !.phase[q] = "done",
                 !.terminal[q] = IF MUTATION = 3 THEN @ + 1 ELSE 1, !.replied = TRUE]
Close == /\ ~s.closed
         /\ s' = [s EXCEPT !.closed = TRUE, !.queue = <<>>, !.flushing = -1,
              !.sent = {}, !.slots = {},
              !.phase = [q \in Ids |-> IF @ [q] = "unused" THEN "unused" ELSE "done"],
              !.terminal = [q \in Ids |-> IF MUTATION = 5 \/ s.phase[q] = "unused" THEN @ [q] ELSE 1]]
Next == Edit \/ Admit \/ StartFlush \/ FinishFlush \/ Send \/ DropCancelledBarrier \/ Close
        \/ \E q \in Ids : Cancel(q) \/ Reply(q)
Spec == Init /\ [][Next]_vars
TypeOK == /\ s.next \in 0..REQUESTS /\ s.revision \in 0..VERSIONS
          /\ s.serverRevision \in 0..VERSIONS /\ s.flushing \in -1..VERSIONS
          /\ s.queue \in Seq([kind : {"sync", "request"}, request : 0..REQUESTS, version : 0..VERSIONS])
          /\ s.slots \subseteq Ids /\ s.sent \subseteq Ids /\ s.cancelled \subseteq Ids
          /\ s.phase \in [Ids -> {"unused", "queued", "sent", "done"}]
          /\ s.terminal \in [Ids -> 0..2] /\ s.captured \in [Ids -> 0..VERSIONS]
          /\ \A name \in {"closed", "badOrder", "replied", "cancelledSent", "flushed"} : s[name] \in BOOLEAN
PhysicalSlotsRetained == s.sent \subseteq s.slots
PhysicalOutstandingBounded == Cardinality(s.slots) <= OUTSTANDING /\ Cardinality(s.sent) <= OUTSTANDING
WireSnapshotOrdered == ~s.badOrder
TerminalOnce == \A q \in Ids : s.terminal[q] <= 1
ClosedSettlesAll == s.closed => (\A q \in 1..s.next : s.terminal[q] = 1)
OuterQueueBounded == Len(s.queue) <= QCAP
WitnessNoReply == ~s.replied
WitnessNoCancelledSent == ~s.cancelledSent
WitnessNoPhysicalFlush == ~s.flushed
=============================================================================
