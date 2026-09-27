---- MODULE WorkerDeployProofs ----
EXTENDS WorkerDeploy, TLAPS

\* Induct on the same deployment/cache action relation TLC exhausts.
\* Arbitrary nonempty Clients, Contexts and Digests; mutation disabled.
\* The authenticated provider, filesystem durability and host liveness
\* remain explicit external premises, not consequences of this theorem.
ASSUME HonestInputs == MUTATION = 0 /\ Clients # {}
    /\ Contexts # {} /\ Digests # {}

Inv == /\ TypeOK
       /\ VerifiedActivation
       /\ ChosenContext
       /\ OwnedCleanup
       /\ NoRetireLive
       /\ ConsentBeforeUpload
       /\ SelectedTargetValid
       /\ VerifiedObjectTrusted

THEOREM InitImpliesInv == Init => Inv
  <1>1. Init => state.phase \in [Clients -> Phases]
      BY DEF Init, Phases
  <1>2. Init => state.selected \in [Clients -> Contexts \cup {None}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>3. Init => state.default \in [Clients -> Contexts \cup {None}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>4. Init => state.wanted \in [Clients -> Digests \cup {None}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>5. Init => state.consent \in [Clients -> BOOLEAN]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>6. Init => state.stage \in [Clients -> Targets \cup {NoTarget}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>7. Init => state.stageOwner \in [Clients -> Clients \cup {None}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>8. Init => state.stagedVerified \in [Clients -> BOOLEAN]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>9. Init => state.published \in [Targets -> BOOLEAN]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>10. Init => state.content \in [Targets -> Digests \cup {None, Corrupt}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>11. Init => state.verified \in [Targets -> BOOLEAN]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>12. Init => state.authorized \in [Targets -> BOOLEAN]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>13. Init => state.receipt \in [Clients -> BOOLEAN]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>14. Init => state.lease \in [Clients -> Targets \cup {NoTarget}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>15. Init => state.readyContext \in [Clients -> Contexts \cup {None}]
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>16. Init => state.badCleanup \in BOOLEAN
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>17. Init => state.retiredWhileLeased \in BOOLEAN
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>18. Init => state.uploadedWithoutConsent \in BOOLEAN
      BY DEF Init, Phases, Targets, NoTarget, None, Corrupt
  <1>19. Init => TypeOK
      BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18 DEF TypeOK
  <1>20. Init => VerifiedActivation
      BY DEF Init, VerifiedActivation, Target, Live, NoTarget, None
  <1>21. Init => ChosenContext
      BY DEF Init, ChosenContext, Target, Live, NoTarget, None
  <1>22. Init => OwnedCleanup
      BY DEF Init, OwnedCleanup, Target, Live, NoTarget, None
  <1>23. Init => NoRetireLive
      BY DEF Init, NoRetireLive, Target, Live, NoTarget, None
  <1>24. Init => ConsentBeforeUpload
      BY DEF Init, ConsentBeforeUpload, Target, Live, NoTarget, None
  <1>25. Init => SelectedTargetValid
      BY DEF Init, SelectedTargetValid, Targets, None
  <1>26. Init => VerifiedObjectTrusted
      BY DEF Init, VerifiedObjectTrusted, Targets
  <1>27. QED
      BY <1>19, <1>20, <1>21, <1>22, <1>23, <1>24,
         <1>25, <1>26 DEF Inv

THEOREM InductiveStep == Inv /\ [Next]_vars => Inv'
  <1> SUFFICES ASSUME Inv, [Next]_vars PROVE Inv'
      OBVIOUS
  <1>1. CASE \E c \in Clients, context \in Contexts, digest \in Digests : Select(c, context, digest)
      <2>1. PICK c \in Clients, context \in Contexts, digest \in Digests : Select(c, context, digest)
          BY <1>1
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Select, StateRecord, Phases
      <2>3. state'.selected \in [Clients -> Contexts \cup {None}]
          BY <2>1 DEF Inv, TypeOK, Select, StateRecord, None
      <2>4. state'.default \in [Clients -> Contexts \cup {None}]
          BY <2>1 DEF Inv, TypeOK, Select, StateRecord, None
      <2>5. state'.wanted \in [Clients -> Digests \cup {None}]
          BY <2>1 DEF Inv, TypeOK, Select, StateRecord, None
      <2>6. TypeOK'
          BY <2>1, <2>2, <2>3, <2>4, <2>5
             DEF Inv, TypeOK, Select, StateRecord
      <2>7. SelectedTargetValid'
          <3>1. SUFFICES \A t \in Clients :
              state'.phase[t] # "idle" =>
                  [context |-> state'.selected[t],
                   digest |-> state'.wanted[t]] \in Targets
              BY DEF SelectedTargetValid
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              <4>1. [context |-> context, digest |-> digest] \in Targets
                  BY IsaT(120), <2>1 DEF Targets
              <4>2. QED
                  BY SMTT(120), <2>1, <3>2, <3>3, <4>1
                     DEF Inv, TypeOK, Select, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, SelectedTargetValid, Select, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>8. ChosenContext'
          <3>1. SUFFICES \A t \in Clients :
              state'.phase[t] = "ready" =>
                  state'.readyContext[t] = state'.selected[t]
              BY DEF ChosenContext
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              BY <2>1, <3>2, <3>3 DEF Select, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, ChosenContext, Select, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>9. QED
          BY <2>1, <2>6, <2>7, <2>8
             DEF Inv, TypeOK, VerifiedObjectTrusted, VerifiedActivation,
                 OwnedCleanup, NoRetireLive, ConsentBeforeUpload,
                 Select, StateRecord
  <1>2. CASE \E c \in Clients, context \in Contexts : ChangeDefault(c, context)
      <2>1. PICK c \in Clients, context \in Contexts : ChangeDefault(c, context)
          BY <1>2
      <2>2. state'.default \in [Clients -> Contexts \cup {None}]
          BY <2>1 DEF Inv, TypeOK, ChangeDefault, StateRecord, None
      <2>3. TypeOK'
          BY <2>1, <2>2 DEF Inv, TypeOK, ChangeDefault, StateRecord
      <2>4. QED
          BY <2>1, <2>3
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, ChangeDefault,
                 StateRecord, Target, Live
  <1>3. CASE \E c \in Clients : Authorize(c)
      <2>1. PICK c \in Clients : Authorize(c)
          BY <1>3
      <2>2. state'.consent \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Authorize, StateRecord
      <2>3. TypeOK'
          BY <2>1, <2>2
             DEF Inv, TypeOK, Authorize, StateRecord
      <2>4. QED
          BY <2>1, <2>3, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, Authorize,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>4. CASE \E c \in Clients : Upload(c)
      <2>1. PICK c \in Clients : Upload(c)
          BY <1>4
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Upload, StateRecord, Phases
      <2>3. Target(c) \in Targets
          BY <2>1 DEF Inv, SelectedTargetValid, Target, Upload
      <2>4. state'.stage \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1, <2>3 DEF Inv, TypeOK, Upload, StateRecord, NoTarget
      <2>5. state'.stageOwner \in [Clients -> Clients \cup {None}]
          BY <2>1 DEF Inv, TypeOK, Upload, StateRecord, None
      <2>6. state'.uploadedWithoutConsent \in BOOLEAN
          BY <2>1, HonestInputs
             DEF Inv, TypeOK, Upload, StateRecord, HonestInputs
      <2>7. TypeOK'
          BY <2>1, <2>2, <2>4, <2>5, <2>6
             DEF Inv, TypeOK, Upload, StateRecord
      <2>8. state.consent[c]
          BY <2>1, HonestInputs DEF HonestInputs, Upload
      <2>9. ConsentBeforeUpload'
          BY <2>1, <2>8 DEF Inv, ConsentBeforeUpload, Upload, StateRecord
      <2>10. QED
          BY <2>1, <2>3, <2>7, <2>9
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, Upload, StateRecord, Target, Live, Targets
  <1>5. CASE \E c \in Clients : VerifyTransfer(c)
      <2>1. PICK c \in Clients : VerifyTransfer(c)
          BY <1>5
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, VerifyTransfer, StateRecord, Phases
      <2>3. state'.stagedVerified \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, VerifyTransfer, StateRecord
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, VerifyTransfer, StateRecord
      <2>5. QED
          BY <2>1, <2>4, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, VerifyTransfer,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>6. CASE \E c \in Clients : Publish(c)
      <2>1. PICK c \in Clients : Publish(c)
          BY <1>6
      <2>2. Target(c) \in Targets
          BY <2>1 DEF Inv, SelectedTargetValid, Target, Publish
      <2>3. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Publish, StateRecord, Phases
      <2>4. state'.published \in [Targets -> BOOLEAN]
          BY <2>1, <2>2 DEF Inv, TypeOK, Publish, StateRecord
      <2>5. state'.content \in [Targets -> Digests \cup {None, Corrupt}]
          BY <2>1, <2>2 DEF Inv, TypeOK, Publish, StateRecord, None, Corrupt
      <2>6. state'.stage \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1 DEF Inv, TypeOK, Publish, StateRecord, NoTarget
      <2>7. state'.stageOwner \in [Clients -> Clients \cup {None}]
          BY <2>1 DEF Inv, TypeOK, Publish, StateRecord, None
      <2>8. TypeOK'
          BY <2>1, <2>3, <2>4, <2>5, <2>6, <2>7
             DEF Inv, TypeOK, Publish, StateRecord
      <2>9. QED
          BY <2>1, <2>2, <2>8, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, Publish,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>7. CASE \E c \in Clients : VerifyObject(c)
      <2>1. PICK c \in Clients : VerifyObject(c)
          BY <1>7
      <2>2. Target(c) \in Targets
          BY <2>1 DEF Inv, SelectedTargetValid, Target, VerifyObject
      <2>3. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, VerifyObject, StateRecord, Phases
      <2>4. state'.verified \in [Targets -> BOOLEAN]
          BY <2>1, <2>2 DEF Inv, TypeOK, VerifyObject, StateRecord
      <2>5. TypeOK'
          BY <2>1, <2>3, <2>4
             DEF Inv, TypeOK, VerifyObject, StateRecord
      <2>6. QED
          BY <2>1, <2>2, <2>5, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, VerifyObject,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>8. CASE \E c \in Clients : WriteReceipt(c)
      <2>1. PICK c \in Clients : WriteReceipt(c)
          BY <1>8
      <2>2. Target(c) \in Targets
          BY <2>1 DEF Inv, SelectedTargetValid, Target, WriteReceipt
      <2>3. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, WriteReceipt, StateRecord, Phases
      <2>4. state'.receipt \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, WriteReceipt, StateRecord
      <2>5. state'.authorized \in [Targets -> BOOLEAN]
          BY <2>1, <2>2 DEF Inv, TypeOK, WriteReceipt, StateRecord
      <2>6. TypeOK'
          BY <2>1, <2>3, <2>4, <2>5
             DEF Inv, TypeOK, WriteReceipt, StateRecord
      <2>7. QED
          BY <2>1, <2>2, <2>6, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, WriteReceipt,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>9. CASE \E c \in Clients : Activate(c)
      <2>1. PICK c \in Clients : Activate(c)
          BY <1>9
      <2>2. Target(c) \in Targets
          BY <2>1 DEF Inv, SelectedTargetValid, Target, Activate
      <2>3. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Activate, StateRecord, Phases
      <2>4. state'.lease \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1, <2>2 DEF Inv, TypeOK, Activate, StateRecord, NoTarget, None
      <2>5. state'.readyContext \in [Clients -> Contexts \cup {None}]
          BY <2>1 DEF Inv, TypeOK, Activate, StateRecord, HonestInputs, None
      <2>6. TypeOK'
          BY <2>1, <2>3, <2>4, <2>5
             DEF Inv, TypeOK, Activate, StateRecord
      <2>7. SelectedTargetValid'
          <3>1. SUFFICES \A t \in Clients :
              state'.phase[t] # "idle" =>
                  [context |-> state'.selected[t],
                   digest |-> state'.wanted[t]] \in Targets
              BY DEF SelectedTargetValid
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              BY <2>1, <2>2, <3>2, <3>3
                 DEF Inv, TypeOK, Target, Activate, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, SelectedTargetValid, Activate, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>8. VerifiedActivation'
          <3>1. SUFFICES \A t \in Clients :
              state'.lease[t] # NoTarget =>
                  state'.published[state'.lease[t]]
                  /\ state'.verified[state'.lease[t]]
                  /\ state'.content[state'.lease[t]]
                     = state'.lease[t].digest
                  /\ state'.receipt[t]
              BY DEF VerifiedActivation
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              <4>1. (state'.lease)[c] = Target(c)
                  BY <2>1, <3>3
                     DEF Inv, TypeOK, Activate, StateRecord
              <4>2. (state.verified)[Target(c)]
                     /\ (state'.verified)[Target(c)]
                     /\ (state'.receipt)[c]
                  BY <2>1, <3>3, HonestInputs
                     DEF HonestInputs, Activate, StateRecord
              <4>3. QED
                  BY <2>1, <2>2, <3>3, <4>1, <4>2, HonestInputs
                     DEF Inv, VerifiedObjectTrusted,
                         Activate, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, VerifiedActivation, Activate, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>9. ChosenContext'
          <3>1. SUFFICES \A t \in Clients :
              state'.phase[t] = "ready" =>
                  state'.readyContext[t] = state'.selected[t]
              BY DEF ChosenContext
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              <4>1. (state'.readyContext)[c] = (state.selected)[c]
                  BY <2>1, <3>3, HonestInputs
                     DEF HonestInputs, Inv, TypeOK,
                         Activate, StateRecord
              <4>2. (state'.selected)[c] = (state.selected)[c]
                  BY <2>1 DEF Activate, StateRecord
              <4>3. QED
                  BY <2>1, <3>3, <4>1, <4>2
                     DEF Inv, TypeOK, ChosenContext,
                         Activate, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, ChosenContext, Activate, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>10. QED
          BY <2>1, <2>6, <2>7, <2>8, <2>9
             DEF Inv, TypeOK, VerifiedObjectTrusted, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, Activate, StateRecord
  <1>10. CASE \E c \in Clients : HandshakeFailure(c)
      <2>1. PICK c \in Clients : HandshakeFailure(c)
          BY <1>10
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, HandshakeFailure, StateRecord, Phases
      <2>3. TypeOK'
          BY <2>1, <2>2
             DEF Inv, TypeOK, HandshakeFailure, StateRecord
      <2>4. QED
          BY <2>1, <2>3, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, HandshakeFailure,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>11. CASE \E c \in Clients : Retire(c)
      <2>1. PICK c \in Clients : Retire(c)
          BY <1>11
      <2>2. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Retire, StateRecord, Phases
      <2>3. state'.lease \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1 DEF Inv, TypeOK, Retire, StateRecord, NoTarget
      <2>4. state'.receipt \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Retire, StateRecord
      <2>5. state'.consent \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Retire, StateRecord
      <2>6. state'.stagedVerified \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Retire, StateRecord
      <2>7. TypeOK'
          BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6
             DEF Inv, TypeOK, Retire, StateRecord
      <2>8. SelectedTargetValid'
          <3>1. SUFFICES \A t \in Clients :
              state'.phase[t] # "idle" =>
                  [context |-> state'.selected[t],
                   digest |-> state'.wanted[t]] \in Targets
              BY DEF SelectedTargetValid
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              BY <2>1, <3>2, <3>3
                 DEF Inv, TypeOK, Retire, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, SelectedTargetValid, Retire, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>9. VerifiedActivation'
          <3>1. SUFFICES \A t \in Clients :
              state'.lease[t] # NoTarget =>
                  state'.published[state'.lease[t]]
                  /\ state'.verified[state'.lease[t]]
                  /\ state'.content[state'.lease[t]]
                     = state'.lease[t].digest
                  /\ state'.receipt[t]
              BY DEF VerifiedActivation
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              BY <2>1, <3>2, <3>3
                 DEF Inv, TypeOK, Retire, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, VerifiedActivation, Retire, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>10. ChosenContext'
          <3>1. SUFFICES \A t \in Clients :
              state'.phase[t] = "ready" =>
                  state'.readyContext[t] = state'.selected[t]
              BY DEF ChosenContext
          <3>2. TAKE t \in Clients
          <3>3. CASE t = c
              BY <2>1, <3>2, <3>3
                 DEF Inv, TypeOK, Retire, StateRecord
          <3>4. CASE t # c
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, ChosenContext, Retire, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>11. QED
          BY <2>1, <2>7, <2>8, <2>9, <2>10
             DEF Inv, TypeOK, VerifiedObjectTrusted, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, Retire, StateRecord
  <1>12. CASE \E c \in Clients : Reuse(c)
      <2>1. PICK c \in Clients : Reuse(c)
          BY <1>12
      <2>2. Target(c) \in Targets
          BY <2>1 DEF Inv, SelectedTargetValid, Target, Reuse
      <2>3. state'.phase \in [Clients -> Phases]
          BY <2>1 DEF Inv, TypeOK, Reuse, StateRecord, Phases
      <2>4. state'.receipt \in [Clients -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Reuse, StateRecord
      <2>5. TypeOK'
          BY <2>1, <2>3, <2>4
             DEF Inv, TypeOK, Reuse, StateRecord
      <2>6. QED
          BY <2>1, <2>2, <2>5, HonestInputs
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, OwnedCleanup,
                 NoRetireLive, ConsentBeforeUpload, HonestInputs, Reuse,
                 StateRecord, Target, Live, Targets, NoTarget, None
  <1>13. CASE \E t \in Targets : CorruptObject(t)
      <2>1. PICK t \in Targets : CorruptObject(t)
          BY <1>13
      <2>2. state'.content \in [Targets -> Digests \cup {None, Corrupt}]
          BY <2>1 DEF Inv, TypeOK, CorruptObject, StateRecord, Corrupt
      <2>3. state'.verified \in [Targets -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, CorruptObject, StateRecord
      <2>4. TypeOK'
          BY <2>1, <2>2, <2>3
             DEF Inv, TypeOK, CorruptObject, StateRecord
      <2>5. VerifiedObjectTrusted'
          <3>1. SUFFICES \A u \in Targets :
              state'.verified[u] =>
                  state'.published[u] /\ state'.content[u] = u.digest
              BY DEF VerifiedObjectTrusted
          <3>2. TAKE u \in Targets
          <3>3. CASE u = t
              BY <2>1, <3>2, <3>3
                 DEF Inv, TypeOK, CorruptObject, StateRecord
          <3>4. CASE u # t
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, VerifiedObjectTrusted,
                     CorruptObject, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>6. VerifiedActivation'
          <3>1. SUFFICES \A c \in Clients :
              state'.lease[c] # NoTarget =>
                  state'.published[state'.lease[c]]
                  /\ state'.verified[state'.lease[c]]
                  /\ state'.content[state'.lease[c]]
                     = state'.lease[c].digest
                  /\ state'.receipt[c]
              BY DEF VerifiedActivation
          <3>2. TAKE c \in Clients
          <3>3. state.lease[c] # t
              BY <2>1, <3>2 DEF CorruptObject, Live
          <3>4. QED
          BY <2>1, <3>2, <3>3
             DEF Inv, TypeOK, VerifiedActivation, CorruptObject, StateRecord
      <2>7. QED
          BY <2>1, <2>4, <2>5, <2>6
             DEF Inv, TypeOK, SelectedTargetValid, ChosenContext,
                 OwnedCleanup, NoRetireLive, ConsentBeforeUpload,
                 CorruptObject, StateRecord
  <1>14. CASE \E actor \in Clients, owner \in Clients : CleanStage(actor, owner)
      <2>1. PICK actor \in Clients, owner \in Clients : CleanStage(actor, owner)
          BY <1>14
      <2>2. state'.stage \in [Clients -> Targets \cup {NoTarget}]
          BY <2>1 DEF Inv, TypeOK, CleanStage, StateRecord, NoTarget
      <2>3. state'.stageOwner \in [Clients -> Clients \cup {None}]
          BY <2>1 DEF Inv, TypeOK, CleanStage, StateRecord, None
      <2>4. state'.badCleanup \in BOOLEAN
          BY <2>1 DEF Inv, TypeOK, CleanStage, StateRecord, HonestInputs
      <2>5. TypeOK'
          BY <2>1, <2>2, <2>3, <2>4
             DEF Inv, TypeOK, CleanStage, StateRecord
      <2>6. actor = owner
          BY <2>1, HonestInputs DEF HonestInputs, CleanStage
      <2>7. OwnedCleanup'
          BY <2>1, <2>6 DEF Inv, OwnedCleanup, CleanStage, StateRecord
      <2>8. QED
          BY <2>1, <2>5, <2>7
             DEF Inv, SelectedTargetValid, VerifiedObjectTrusted,
                 VerifiedActivation, ChosenContext, NoRetireLive,
                 ConsentBeforeUpload, CleanStage, StateRecord, Target, Live
  <1>15. CASE \E actor \in Clients, t \in Targets : Collect(actor, t)
      <2>1. PICK actor \in Clients, t \in Targets : Collect(actor, t)
          BY <1>15
      <2>2. state'.published \in [Targets -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Collect, StateRecord
      <2>3. state'.verified \in [Targets -> BOOLEAN]
          BY <2>1 DEF Inv, TypeOK, Collect, StateRecord
      <2>4. state'.content \in [Targets -> Digests \cup {None, Corrupt}]
          BY <2>1 DEF Inv, TypeOK, Collect, StateRecord, None
      <2>5. state'.retiredWhileLeased \in BOOLEAN
          BY <2>1 DEF Inv, TypeOK, Collect, StateRecord, HonestInputs
      <2>6. state'.badCleanup \in BOOLEAN
          BY <2>1 DEF Inv, TypeOK, Collect, StateRecord, HonestInputs
      <2>7. TypeOK'
          BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6
             DEF Inv, TypeOK, Collect, StateRecord
      <2>8. state.selected[actor] = t.context /\ ~Live(t)
          BY <2>1, HonestInputs DEF HonestInputs, Collect
      <2>9. (state'.badCleanup) = ((state.badCleanup)
                 \/ ((state.selected)[actor] # t.context))
          BY <2>1 DEF Collect, StateRecord
      <2>10. OwnedCleanup'
          BY <2>8, <2>9 DEF Inv, OwnedCleanup
      <2>11. (state'.retiredWhileLeased) = ((state.retiredWhileLeased)
                 \/ Live(t))
          BY <2>1 DEF Collect, StateRecord
      <2>12. NoRetireLive'
          BY <2>8, <2>11 DEF Inv, NoRetireLive
      <2>13. VerifiedObjectTrusted'
          <3>1. SUFFICES \A u \in Targets :
              state'.verified[u] =>
                  state'.published[u] /\ state'.content[u] = u.digest
              BY DEF VerifiedObjectTrusted
          <3>2. TAKE u \in Targets
          <3>3. CASE u = t
              BY <2>1, <3>2, <3>3
                 DEF Inv, TypeOK, Collect, StateRecord
          <3>4. CASE u # t
              BY <2>1, <3>2, <3>4
                 DEF Inv, TypeOK, VerifiedObjectTrusted, Collect, StateRecord
          <3>5. QED BY <3>3, <3>4
      <2>14. VerifiedActivation'
          <3>1. SUFFICES \A c \in Clients :
              state'.lease[c] # NoTarget =>
                  state'.published[state'.lease[c]]
                  /\ state'.verified[state'.lease[c]]
                  /\ state'.content[state'.lease[c]]
                     = state'.lease[c].digest
                  /\ state'.receipt[c]
              BY DEF VerifiedActivation
          <3>2. TAKE c \in Clients
          <3>3. state.lease[c] # t
              BY <2>8, <3>2 DEF Live
          <3>4. QED
          BY <2>1, <3>2, <3>3
             DEF Inv, TypeOK, VerifiedActivation, Collect, StateRecord
      <2>15. QED
          BY <2>1, <2>7, <2>10, <2>12, <2>13, <2>14
             DEF Inv, TypeOK, SelectedTargetValid, ChosenContext,
                 ConsentBeforeUpload, Collect, StateRecord
  <1>16. CASE UNCHANGED vars
      <2>1. state' = state BY <1>16 DEF vars
      <2>2. QED BY <2>1 DEF Inv, TypeOK, SelectedTargetValid,
          VerifiedObjectTrusted, VerifiedActivation, ChosenContext,
          OwnedCleanup, NoRetireLive, ConsentBeforeUpload,
          Target, Live
  <1>17. QED
      BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16
         DEF Next, vars

THEOREM InvImpliesSafety == Inv => /\ VerifiedActivation
    /\ ChosenContext /\ OwnedCleanup /\ NoRetireLive
    /\ ConsentBeforeUpload
    BY DEF Inv
=============================================================================
