---- MODULE WorkerSessionProofs ----
EXTENDS WorkerSession, TLAPS

\* The same transition relation and invariants checked by TLC, with
\* arbitrary client/worker/stream sets and any positive negotiated
\* bounds. No finite two-worker premise or cache of TLC states enters
\* these inductive safety obligations. Liveness and OS/peer fairness
\* remain separate from this safety theorem.
ASSUME HonestInputs == /\ MUTATION = 0
                      /\ MODE \in 0..2
                      /\ Clients # {} /\ Workers # {} /\ Streams # {}
                      /\ MAXGEN \in Nat /\ MAXGEN > 0
                      /\ WINDOW \in Nat /\ WINDOW > 0
                      /\ QCAP \in Nat /\ QCAP > 0

Inv == /\ TypeOK
       /\ BoundedOutstanding
       /\ NoStaleEffect
       /\ NoStaleCommit
       /\ ForeignStopRefused
       /\ ExitAfterOutput
       /\ LastMeansFlushed
       /\ DirtyUntilProof
       /\ RecoveryInNamespace

CountersTyped == \A s \in Streams :
    /\ state.credit[s] \in 0..WINDOW
    /\ state.outbound[s] \in 0..QCAP
    /\ state.inbound[s] \in 0..WINDOW
THEOREM InvImpliesCounters == Inv => CountersTyped
    BY DEF Inv, TypeOK, CountersTyped

StreamDomains == /\ state.active \in [Streams -> BOOLEAN]
    /\ state.credit \in [Streams -> 0..WINDOW]
    /\ state.outbound \in [Streams -> 0..QCAP]
    /\ state.inbound \in [Streams -> 0..WINDOW]
THEOREM InvImpliesStreamDomains == Inv => StreamDomains
    BY DEF Inv, TypeOK, StreamDomains

AttemptDomain == state.attempt \in [Clients -> Attempts]
THEOREM InvImpliesAttemptDomain == Inv => AttemptDomain
    BY DEF Inv, TypeOK, AttemptDomain

EmptyInactive == \A s \in Streams :
    ~state.active[s] =>
        state.credit[s] = 0 /\ state.outbound[s] = 0 /\ state.inbound[s] = 0
TransmitRoom == \A s \in Streams :
    state.active[s] /\ state.outbound[s] > 0 => state.inbound[s] < WINDOW
ConsumeRoom == \A s \in Streams :
    state.active[s] /\ state.inbound[s] > 0 => state.credit[s] < WINDOW

THEOREM InvImpliesRoom == Inv =>
    /\ EmptyInactive /\ TransmitRoom /\ ConsumeRoom
  <1>1. Inv => EmptyInactive
      BY SMTT(60), InvImpliesCounters, HonestInputs
         DEF Inv, BoundedOutstanding, CountersTyped, EmptyInactive
  <1>2. Inv => TransmitRoom
      BY SMTT(60), InvImpliesCounters, HonestInputs
         DEF Inv, BoundedOutstanding, CountersTyped, TransmitRoom
  <1>3. Inv => ConsumeRoom
      BY SMTT(60), InvImpliesCounters, HonestInputs
         DEF Inv, BoundedOutstanding, CountersTyped, ConsumeRoom
  <1>4. QED BY <1>1, <1>2, <1>3

THEOREM TransmitConserves == \A a \in Nat, b \in Nat, c \in Nat :
    b > 0 => a + (b - 1) + (c + 1) = a + b + c
    OBVIOUS
THEOREM ConsumeConserves == \A a \in Nat, b \in Nat, c \in Nat :
    c > 0 => (a + 1) + b + (c - 1) = a + b + c
    OBVIOUS

THEOREM InitImpliesInv == Init => Inv
    BY HonestInputs DEF Init, Inv, TypeOK, BoundedOutstanding,
        NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput,
        LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace, None, Owners, Attempts, Stages

THEOREM InductiveStep == Inv /\ [Next]_vars => Inv'
  <1> SUFFICES ASSUME Inv, [Next]_vars PROVE Inv'
      OBVIOUS
  <1>1. CASE \E c \in Clients, w \in Workers : Handshake(c, w)
      BY <1>1, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Handshake, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>2. CASE \E w \in Workers, n \in Workers : Restart(w, n)
      BY <1>2, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Restart, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>3. CASE MODE \in {0, 1} /\ \E c \in Clients, s \in Streams : Open(c, s)
      <2>1. PICK c \in Clients, s \in Streams : Open(c, s)
          BY <1>3
      <2>2. TypeOK'
          BY <2>1, HonestInputs DEF Inv, TypeOK, Open, StateRecord,
              Bound, Owners, Attempts, Stages, None, vars
      <2>3. BoundedOutstanding'
          <3>1. SUFFICES \A t \in Streams :
              IF state'.active[t] THEN
                  state'.credit[t] + state'.outbound[t] + state'.inbound[t] = WINDOW
              ELSE state'.credit[t] + state'.outbound[t] + state'.inbound[t] = 0
              BY DEF BoundedOutstanding
          <3>2. TAKE t \in Streams
          <3>3. CASE t = s
              BY <2>1, <3>2, <3>3, InvImpliesRoom, HonestInputs
                 DEF Inv, TypeOK, BoundedOutstanding, Open, StateRecord,
                     EmptyInactive, Bound, Owners, Attempts, Stages, None, vars
          <3>4. CASE t # s
              BY <2>1, <3>2, <3>4 DEF Inv, BoundedOutstanding,
                 Open, StateRecord, vars
          <3>5. QED
              BY <3>3, <3>4
      <2>4. LastMeansFlushed'
          BY <2>1, HonestInputs DEF Inv, TypeOK, LastMeansFlushed,
              Open, StateRecord, Owners, Attempts, Stages, None, vars
      <2>5. ForeignStopRefused'
          BY <2>1, HonestInputs
             DEF Inv, TypeOK, ForeignStopRefused, Open, StateRecord,
                 None, Owners
      <2>6. ExitAfterOutput'
          BY <2>1, HonestInputs
             DEF Inv, TypeOK, ExitAfterOutput, Open, StateRecord, None
      <2>7. QED
          BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, HonestInputs
             DEF Inv, NoStaleEffect, NoStaleCommit,
                 DirtyUntilProof, RecoveryInNamespace, Open,
                 StateRecord, vars
  <1>4. CASE MODE \in {0, 1} /\ \E c \in Clients, s \in Streams : Abandon(c, s)
      BY <1>4, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Abandon, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>5. CASE MODE \in {0, 1} /\ \E c \in Clients, s \in Streams : Stop(c, s)
      BY <1>5, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Stop, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>6. CASE MODE \in {0, 1} /\ \E s \in Streams : Produce(s)
      <2>1. PICK s \in Streams : Produce(s)
          BY <1>6
      <2>2. TypeOK'
          BY <2>1, HonestInputs DEF Inv, TypeOK, Produce, StateRecord,
              Owners, Attempts, Stages, None, vars
      <2>3. BoundedOutstanding'
          <3>1. SUFFICES \A t \in Streams :
              IF state'.active[t] THEN
                  state'.credit[t] + state'.outbound[t] + state'.inbound[t] = WINDOW
              ELSE state'.credit[t] + state'.outbound[t] + state'.inbound[t] = 0
              BY DEF BoundedOutstanding
          <3>2. TAKE t \in Streams
          <3>3. CASE t = s
              BY <2>1, <3>2, <3>3, HonestInputs
                 DEF Inv, TypeOK, BoundedOutstanding, Produce, StateRecord,
                     Owners, Attempts, Stages, None, vars
          <3>4. CASE t # s
              BY <2>1, <3>2, <3>4 DEF Inv, BoundedOutstanding,
                 Produce, StateRecord, vars
          <3>5. QED
              BY <3>3, <3>4
      <2>4. LastMeansFlushed'
          BY <2>1, HonestInputs DEF Inv, TypeOK, LastMeansFlushed,
              Produce, StateRecord, Owners, Attempts, Stages, None, vars
      <2>5. QED
          BY <2>1, <2>2, <2>3, <2>4, HonestInputs
             DEF Inv, NoStaleEffect, NoStaleCommit,
                 ForeignStopRefused, ExitAfterOutput,
                 DirtyUntilProof, RecoveryInNamespace, Produce,
                 StateRecord, Bound, Owns, FreshAttempt, vars
  <1>7. CASE MODE \in {0, 1} /\ \E s \in Streams : Transmit(s)
      <2>1. PICK s \in Streams : Transmit(s)
          BY <1>7
      <2>2. TypeOK'
          BY <2>1, InvImpliesRoom, HonestInputs
             DEF Inv, TypeOK, Transmit, StateRecord, TransmitRoom,
                 Bound, Owners, Attempts, Stages, None, vars
      <2>3. BoundedOutstanding'
          <3>1. SUFFICES \A t \in Streams :
              IF state'.active[t] THEN
                  state'.credit[t] + state'.outbound[t] + state'.inbound[t] = WINDOW
              ELSE state'.credit[t] + state'.outbound[t] + state'.inbound[t] = 0
              BY DEF BoundedOutstanding
          <3>2. TAKE t \in Streams
          <3>3. CASE t = s
              <4>1. /\ state.credit[s] \in Nat
                     /\ state.outbound[s] \in Nat
                     /\ state.inbound[s] \in Nat
                  BY InvImpliesCounters DEF Inv, CountersTyped
              <4>2. state.credit[s] + (state.outbound[s] - 1)
                     + (state.inbound[s] + 1)
                     = state.credit[s] + state.outbound[s] + state.inbound[s]
                  BY <4>1, <2>1, TransmitConserves DEF Transmit
              <4>3. /\ state'.active[t]
                     /\ state'.credit[t] = state.credit[s]
                     /\ state'.outbound[t] = state.outbound[s] - 1
                     /\ state'.inbound[t] = state.inbound[s] + 1
                  BY <2>1, <3>3, InvImpliesStreamDomains
                     DEF Inv, StreamDomains, Transmit, StateRecord
              <4>4. state.credit[s] + state.outbound[s]
                     + state.inbound[s] = WINDOW
                  BY <2>1 DEF Inv, BoundedOutstanding, Transmit
              <4>5. QED BY <4>2, <4>3, <4>4
          <3>4. CASE t # s
              BY <2>1, <3>2, <3>4 DEF Inv, BoundedOutstanding,
                 Transmit, StateRecord, vars
          <3>5. QED
              BY <3>3, <3>4
      <2>4. LastMeansFlushed'
          BY <2>1, HonestInputs DEF Inv, TypeOK, LastMeansFlushed,
              Transmit, StateRecord, Owners, Attempts, Stages, None, vars
      <2>5. QED
          BY <2>1, <2>2, <2>3, <2>4, HonestInputs
             DEF Inv, NoStaleEffect, NoStaleCommit,
                 ForeignStopRefused, ExitAfterOutput,
                 DirtyUntilProof, RecoveryInNamespace, Transmit,
                 StateRecord, Bound, Owns, FreshAttempt, vars
  <1>8. CASE MODE \in {0, 1} /\ \E s \in Streams : Consume(s)
      <2>1. PICK s \in Streams : Consume(s)
          BY <1>8
      <2>2. TypeOK'
          BY <2>1, InvImpliesRoom, HonestInputs
             DEF Inv, TypeOK, Consume, StateRecord, ConsumeRoom,
                 Bound, Owners, Attempts, Stages, None, vars
      <2>3. BoundedOutstanding'
          <3>1. SUFFICES \A t \in Streams :
              IF state'.active[t] THEN
                  state'.credit[t] + state'.outbound[t] + state'.inbound[t] = WINDOW
              ELSE state'.credit[t] + state'.outbound[t] + state'.inbound[t] = 0
              BY DEF BoundedOutstanding
          <3>2. TAKE t \in Streams
          <3>3. CASE t = s
              <4>1. /\ state.credit[s] \in Nat
                     /\ state.outbound[s] \in Nat
                     /\ state.inbound[s] \in Nat
                  BY InvImpliesCounters DEF Inv, CountersTyped
              <4>2. (state.credit[s] + 1) + state.outbound[s]
                     + (state.inbound[s] - 1)
                     = state.credit[s] + state.outbound[s] + state.inbound[s]
                  BY <4>1, <2>1, ConsumeConserves DEF Consume
              <4>3. /\ state'.active[t]
                     /\ state'.credit[t] = state.credit[s] + 1
                     /\ state'.outbound[t] = state.outbound[s]
                     /\ state'.inbound[t] = state.inbound[s] - 1
                  BY <2>1, <3>3, InvImpliesStreamDomains
                     DEF Inv, StreamDomains, Consume, StateRecord
              <4>4. state.credit[s] + state.outbound[s]
                     + state.inbound[s] = WINDOW
                  BY <2>1 DEF Inv, BoundedOutstanding, Consume
              <4>5. QED BY <4>2, <4>3, <4>4
          <3>4. CASE t # s
              BY <2>1, <3>2, <3>4 DEF Inv, BoundedOutstanding,
                 Consume, StateRecord, vars
          <3>5. QED
              BY <3>3, <3>4
      <2>4. LastMeansFlushed'
          BY <2>1, HonestInputs DEF Inv, TypeOK, LastMeansFlushed,
              Consume, StateRecord, Owners, Attempts, Stages, None, vars
      <2>5. QED
          BY <2>1, <2>2, <2>3, <2>4, HonestInputs
             DEF Inv, NoStaleEffect, NoStaleCommit,
                 ForeignStopRefused, ExitAfterOutput,
                 DirtyUntilProof, RecoveryInNamespace, Consume,
                 StateRecord, Bound, Owns, FreshAttempt, vars
  <1>9. CASE MODE \in {0, 1} /\ \E s \in Streams : RecordExit(s)
      BY <1>9, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          RecordExit, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>10. CASE MODE \in {0, 1} /\ \E s \in Streams : FinishPump(s)
      BY <1>10, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          FinishPump, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>11. CASE MODE \in {0, 1} /\ \E s \in Streams : EndOutput(s)
      BY <1>11, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          EndOutput, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>12. CASE MODE \in {0, 1} /\ \E s \in Streams : PublishExit(s)
      BY <1>12, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          PublishExit, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>13. CASE MODE \in {0, 1} /\ \E s \in Streams : CreditWithoutConsume(s)
      BY <1>13, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          CreditWithoutConsume, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>14. CASE MODE \in {0, 2} /\ \E c \in Clients : Submit(c)
      <2>1. PICK c \in Clients : Submit(c)
          BY <1>14
      <2>2. TypeOK'
          BY <2>1, HonestInputs DEF Inv, TypeOK, Submit,
              Bound, StateRecord, Owners, Attempts, Stages, None, vars
      <2>3. NoStaleEffect'
          <3>1. SUFFICES \A t \in Clients :
              state'.attempt[t].stage # "idle" => state'.attempt[t].good
              BY DEF NoStaleEffect
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              <4>1. Bound(c)
                  BY <2>1, HonestInputs DEF Submit, HonestInputs
              <4>2. /\ state'.attempt[t].stage = "prepared"
                     /\ state'.attempt[t].good = Bound(c)
                  BY <2>1, <3>3, InvImpliesAttemptDomain
                     DEF Inv, AttemptDomain, Submit, StateRecord
              <4>3. QED BY <4>1, <4>2
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, NoStaleEffect, Submit, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>4. DirtyUntilProof'
          BY <2>1
             DEF Inv, DirtyUntilProof, Submit, StateRecord
      <2>5. QED
          BY <2>1, <2>2, <2>3, <2>4, HonestInputs
             DEF Inv, BoundedOutstanding, NoStaleCommit,
                 ForeignStopRefused, ExitAfterOutput, LastMeansFlushed,
                 RecoveryInNamespace, Submit, StateRecord, vars
  <1>15. CASE MODE \in {0, 2} /\ \E c \in Clients : Commit(c)
      BY <1>15, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Commit, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>16. CASE MODE \in {0, 2} /\ \E c \in Clients : LoseReply(c)
      BY <1>16, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          LoseReply, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>17. CASE MODE \in {0, 2} /\ \E c \in Clients : Receipt(c)
      BY <1>17, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Receipt, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>18. CASE MODE \in {0, 2} /\ \E c \in Clients : Verify(c)
      BY <1>18, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          Verify, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>19. CASE MODE \in {0, 2} /\ \E c \in Clients : RetireAttempt(c)
      BY <1>19, HonestInputs DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect, NoStaleCommit, ForeignStopRefused, ExitAfterOutput, LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace,
          RetireAttempt, Bound, Owns, FreshAttempt, StateRecord, Owners, Attempts, Stages, None, vars
  <1>20. CASE UNCHANGED vars
      <2>1. state' = state
          BY <1>20 DEF vars
      <2>2. QED
          BY <2>1 DEF Inv, TypeOK, BoundedOutstanding, NoStaleEffect,
              NoStaleCommit, ForeignStopRefused, ExitAfterOutput,
              LastMeansFlushed, DirtyUntilProof, RecoveryInNamespace
  <1>21. QED
      BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9,
         <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17,
         <1>18, <1>19, <1>20
         DEF Next, SessionActions, StreamActions, StoreActions, vars

THEOREM InvImpliesSafety == Inv =>
    /\ NoStaleEffect /\ NoStaleCommit /\ ForeignStopRefused
    /\ BoundedOutstanding /\ ExitAfterOutput /\ LastMeansFlushed
    /\ DirtyUntilProof /\ RecoveryInNamespace
    BY DEF Inv
=============================================================================
