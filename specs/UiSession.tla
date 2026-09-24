---- MODULE UiSession ----
(***************************************************************************)
(* UI-stdio session safety (0057 VF14) over the landed 0056 AR09/AR10      *)
(* protocol. Correspondence points:                                        *)
(*   crates/strop-ui-protocol/src/message.rs  — envelopes: hello/welcome,  *)
(*     act/ack, snapshot, delta, effect/effect-result, resync,             *)
(*     shutdown/bye; BaseStamp{incarnation, generation}; the typed         *)
(*     refusals StaleGeneration/FutureGeneration/WrongIncarnation.         *)
(*   crates/strop-ui-protocol/src/client.rs   — the poison rules: a        *)
(*     snapshot is valid against any prior state and clears poisoning;     *)
(*     a delta is valid only against its exact base; a mismatched or       *)
(*     foreign delta poisons until a snapshot; a poisoned client never     *)
(*     produces a base stamp.                                              *)
(*   crates/strop-ui-protocol/src/driver.rs   — the message barriers: an   *)
(*     act waits for its acknowledgement AND the publication reaching      *)
(*     the acknowledged generation before the next act computes a base.    *)
(*   crates/strop/src/ui_stdio/serve.rs       — admit() (incarnation,      *)
(*     then future-ceiling, then client_known floor), publish() (only      *)
(*     when the view moved; an unchanged observation sends nothing; a      *)
(*     snapshot raises client_known, a delta never does), emit_effects()   *)
(*     (one monotone-id effect per staged payload), and the shutdown       *)
(*     shape (final ack, finish, bye; nothing publishes after bye).        *)
(*                                                                       *)
(*   ORDERING/STALE AUTHORITY  an action is admitted only on a base of     *)
(*     THIS incarnation at or above the floor the client provably knows    *)
(*     (the newest acked/snapshotted generation) and never beyond the      *)
(*     newest publication. Asynchronous publications never raise the       *)
(*     floor: ordinary input is not racy, exactly as a TUI user's keys     *)
(*     are not refused when output lands mid-keystroke. A client that      *)
(*     dropped a delta is poisoned by the next mismatched delta and        *)
(*     recovers only through a complete current snapshot (resync).         *)
(*                                                                       *)
(*   PUBLICATION HONESTY  every publication carries new information:       *)
(*     consecutive publications never repeat the same (generation,         *)
(*     content) stamp — there is no empty success. A published             *)
(*     generation never exceeds what the engine prepared, and the          *)
(*     client cache never claims a view the backend never published.       *)
(*                                                                       *)
(*   EFFECT AUTHORITY  an admitted action may stage one clipboard          *)
(*     payload; the server emits exactly one host effect per staged        *)
(*     payload (monotone ids) and never emits one unstaged. Shutdown       *)
(*     drains staged effects before the bye.                               *)
(*                                                                       *)
(*   SHUTDOWN  the authorized shutdown acks the final request, then        *)
(*     says bye; no publication follows the bye.                           *)
(*                                                                       *)
(* THE invariants (VF14's named safety properties):                        *)
(*                                                                       *)
(*   TypeOK                 every variable stays in its declared finite    *)
(*                          domain                                         *)
(*   StaleNeverActs         no action was applied on a base below the      *)
(*                          client-known floor                             *)
(*   FutureNeverActs        no action was applied on a base beyond the     *)
(*                          newest publication                             *)
(*   ForeignNeverActs       no action was applied on a foreign             *)
(*                          incarnation's base                             *)
(*   PoisonedNeverActs      the client never sent an action while          *)
(*                          poisoned                                       *)
(*   PoisonedUntilSnapshot  poisoning cleared only by applying a           *)
(*                          snapshot (resync), never by a delta            *)
(*   NoEmptyPublication     no publication repeated the previous           *)
(*                          publication's stamp (no empty success)         *)
(*   EffectExactlyOnce      emitted effects never exceed staged            *)
(*                          payloads                                       *)
(*   ByeAfterFinalAck       the bye follows the final acknowledgement      *)
(*   NoPostByePublication   nothing publishes after the bye                *)
(*   CliNeverAhead          the client cache never claims a generation     *)
(*                          the backend never published                    *)
(*   PublishedNeverAhead    publications never claim a generation the      *)
(*                          engine never prepared                          *)
(*   ByeNeverStrandsEffects the bye never precedes a staged effect         *)
(*                                                                       *)
(* Deliberately faulty variants (configs flip MUTATION):                   *)
(*   MUTATION = 1  admit() drops the floor check — a stale base is         *)
(*                 applied; must die by StaleNeverActs                     *)
(*   MUTATION = 2  admit() drops the ceiling check — a future base is      *)
(*                 applied; must die by FutureNeverActs                    *)
(*   MUTATION = 3  admit() drops the incarnation check — a foreign base    *)
(*                 is applied; must die by ForeignNeverActs                *)
(*   MUTATION = 4  a poisoned client sends an action anyway; must die by   *)
(*                 PoisonedNeverActs                                       *)
(*   MUTATION = 5  a poisoned client accepts a delta as if it cleared      *)
(*                 poisoning; must die by PoisonedUntilSnapshot            *)
(*   MUTATION = 6  the server publishes an unchanged view (an empty        *)
(*                 success); must die by NoEmptyPublication                *)
(*   MUTATION = 7  emit_effects re-emits a staged payload; must die by     *)
(*                 EffectExactlyOnce                                       *)
(*   MUTATION = 8  the shutdown says bye without the final ack; must die   *)
(*                 by ByeAfterFinalAck                                     *)
(*   MUTATION = 9  the server publishes after the bye; must die by         *)
(*                 NoPostByePublication                                    *)
(*                                                                       *)
(* Not modeled: request sequence numbers (the server handles one request   *)
(* per loop iteration, so act/ack correlation is atomic here), frame       *)
(* decode/version errors and EOF/disconnect byes (typed-error coverage is  *)
(* native, tests/ui_stdio.rs), viewport bound refusal, effect-result       *)
(* payloads (terminal fire-and-forget records, AR08), and backend          *)
(* restart: the incarnation is per-process, so one session never sees an   *)
(* incarnation change — the WrongIncarnation discipline is exercised by    *)
(* rogue bases naming a foreign incarnation. Liveness (a publication that  *)
(* never comes) is a progress matter.                                      *)
(***************************************************************************)
EXTENDS Integers, Sequences

CONSTANTS GEN_MAX,     \* engine generation budget (hello prepares gen 1)
          CREV_MAX,    \* content-revision budget (same-generation moves)
          QMAX,        \* bounded publication wire (AR06 backpressure)
          ACT_MAX,     \* applied-action budget (honest + rogue)
          EFF_MAX,     \* staged clipboard payloads
          DROP_MAX,    \* client-side publication drops
          RESYNC_MAX,  \* explicit resyncs
          ROGUE_MAX,   \* rogue (bad-base) actions
          MUTATION     \* 0 = honest; faulty variants above

\* The serving backend's incarnation (message.rs BackendInfo.incarnation);
\* FOREIGN names any other backend process's incarnation.
INC == 1
FOREIGN == 2

\* One publication on the wire (message.rs ServerMessage::{Snapshot, Delta}).
Pub == [kind: {"snap", "delta"}, inc: {INC, FOREIGN},
        gen: 1..GEN_MAX, base: 0..GEN_MAX, crev: 0..CREV_MAX]

VARIABLES srvGen,      \* the engine's prepared generation (monotone)
          srvCrev,     \* content revision: semantic moves at one generation
          pubGen,      \* last published generation (0 = nothing published)
          pubCrev,     \* last published content revision
          floor,       \* client_known: the admission floor (serve.rs)
          applied,     \* the backend's monotone applied-action counter
          handshaken,  \* hello/welcome completed
          byed,        \* the final bye was sent
          finalAck,    \* the shutdown's acknowledgement was sent
          staged,      \* a clipboard payload awaits emission
          stagedTotal, \* payloads ever staged by admitted actions
          emitted,     \* host effects ever emitted
          wire,        \* the ordered publication stream (server -> client)
          cliGen,      \* the client cache's applied generation (0 = none)
          hasView,     \* a first publication was applied
          poisoned,    \* the client needs a complete current snapshot
          ackFloor,    \* the client's last acknowledged generation (the
                       \* driver barrier: act only once the publication
                       \* reaching it was applied)
          drops,       \* client-side publication drops (witness budget)
          resyncs,     \* explicit resync requests
          recoveries,  \* resyncs requested while poisoned
          rogueActs,   \* rogue actions sent on deliberately bad bases
          staleRefusals,    \* honest StaleGeneration refusals
          futureRefusals,   \* honest FutureGeneration refusals
          foreignRefusals,  \* honest WrongIncarnation refusals
          sameGenPubs, \* same-generation deltas published (witness)
          poisons,     \* transitions into the poisoned state
          staleApplied,   \* stale-base applications (always 0)
          futureApplied,  \* future-base applications (always 0)
          foreignApplied, \* foreign-base applications (always 0)
          poisonedActs,   \* actions sent while poisoned (always 0)
          clearedBadly,   \* poison clears without a snapshot (always 0)
          emptyPubs,      \* publications repeating the previous stamp
          postByePubs     \* publications after the bye (always 0)

engVars == <<srvGen, srvCrev>>
pubVars == <<pubGen, pubCrev>>
admVars == <<floor, applied>>
sessVars == <<handshaken, byed, finalAck>>
effVars == <<staged, stagedTotal, emitted>>
cliVars == <<cliGen, hasView, poisoned, ackFloor>>
witVars == <<drops, resyncs, recoveries, rogueActs,
             staleRefusals, futureRefusals, foreignRefusals,
             sameGenPubs, poisons>>
mutVars == <<staleApplied, futureApplied, foreignApplied, poisonedActs,
             clearedBadly, emptyPubs, postByePubs>>

vars == <<engVars, pubVars, admVars, sessVars, effVars, wire, cliVars,
          witVars, mutVars>>

TypeOK ==
    /\ srvGen \in 0..GEN_MAX
    /\ srvCrev \in 0..CREV_MAX
    /\ pubGen \in 0..GEN_MAX
    /\ pubCrev \in 0..CREV_MAX
    /\ floor \in 0..GEN_MAX
    /\ applied \in 0..ACT_MAX
    /\ handshaken \in BOOLEAN
    /\ byed \in BOOLEAN
    /\ finalAck \in BOOLEAN
    /\ staged \in BOOLEAN
    /\ stagedTotal \in 0..EFF_MAX
    /\ emitted \in 0..(EFF_MAX + 1)
    /\ wire \in Seq(Pub)
    /\ cliGen \in 0..GEN_MAX
    /\ hasView \in BOOLEAN
    /\ poisoned \in BOOLEAN
    /\ ackFloor \in 0..GEN_MAX
    /\ drops \in 0..DROP_MAX
    /\ resyncs \in 0..RESYNC_MAX
    /\ recoveries \in 0..RESYNC_MAX
    /\ rogueActs \in 0..ROGUE_MAX
    /\ staleRefusals \in 0..2
    /\ futureRefusals \in 0..2
    /\ foreignRefusals \in 0..2
    /\ sameGenPubs \in 0..2
    /\ poisons \in 0..2
    /\ staleApplied \in 0..1
    /\ futureApplied \in 0..1
    /\ foreignApplied \in 0..1
    /\ poisonedActs \in 0..1
    /\ clearedBadly \in 0..1
    /\ emptyPubs \in 0..1
    /\ postByePubs \in 0..1

Cap2(x) == IF x < 2 THEN x + 1 ELSE x

Snap(g, c) ==
    [kind |-> "snap", inc |-> INC, gen |-> g, base |-> 0, crev |-> c]
Delta(g, b, c) ==
    [kind |-> "delta", inc |-> INC, gen |-> g, base |-> b, crev |-> c]

\* serve.rs admit(): incarnation first, then the future ceiling, then the
\* client-known floor.
AdmitRaw(base) ==
    IF base.inc # INC THEN "foreign"
    ELSE IF base.gen > pubGen THEN "future"
    ELSE IF base.gen < floor THEN "stale"
    ELSE "ok"

\* Faulty variants admit what the honest guard refuses.
Remitted(raw) ==
    IF MUTATION = 1 /\ raw = "stale" THEN "ok"
    ELSE IF MUTATION = 2 /\ raw = "future" THEN "ok"
    ELSE IF MUTATION = 3 /\ raw = "foreign" THEN "ok"
    ELSE raw

RefuseWith(raw) ==
    /\ staleRefusals' = IF raw = "stale" THEN Cap2(staleRefusals)
                        ELSE staleRefusals
    /\ futureRefusals' = IF raw = "future" THEN Cap2(futureRefusals)
                         ELSE futureRefusals
    /\ foreignRefusals' = IF raw = "foreign" THEN Cap2(foreignRefusals)
                          ELSE foreignRefusals

Init ==
    /\ srvGen = 1
    /\ srvCrev = 0
    /\ pubGen = 0
    /\ pubCrev = 0
    /\ floor = 0
    /\ applied = 0
    /\ handshaken = FALSE
    /\ byed = FALSE
    /\ finalAck = FALSE
    /\ staged = FALSE
    /\ stagedTotal = 0
    /\ emitted = 0
    /\ wire = <<>>
    /\ cliGen = 0
    /\ hasView = FALSE
    /\ poisoned = FALSE
    /\ ackFloor = 0
    /\ drops = 0
    /\ resyncs = 0
    /\ recoveries = 0
    /\ rogueActs = 0
    /\ staleRefusals = 0
    /\ futureRefusals = 0
    /\ foreignRefusals = 0
    /\ sameGenPubs = 0
    /\ poisons = 0
    /\ staleApplied = 0
    /\ futureApplied = 0
    /\ foreignApplied = 0
    /\ poisonedActs = 0
    /\ clearedBadly = 0
    /\ emptyPubs = 0
    /\ postByePubs = 0

\* Hello: the welcome delivers the incarnation; the initial publication is
\* a full snapshot and raises the admission floor (serve.rs hello ->
\* publish; the first publication is always a snapshot).
Hello ==
    /\ ~handshaken /\ ~byed
    /\ Len(wire) < QMAX
    /\ handshaken' = TRUE
    /\ wire' = Append(wire, Snap(srvGen, srvCrev))
    /\ pubGen' = srvGen
    /\ pubCrev' = srvCrev
    /\ floor' = srvGen
    /\ UNCHANGED <<engVars, applied, byed, finalAck, effVars, cliVars,
                   witVars, mutVars>>

\* The honest client acts on its applied view (this incarnation, the
\* cache's generation), never poisoned and only once the driver barrier
\* placed the cache at or past the last acknowledged generation.
\* MUTATION 4: a poisoned client sends an action anyway.
ClientAct ==
    /\ handshaken /\ ~byed
    /\ hasView
    /\ srvGen < GEN_MAX
    /\ applied < ACT_MAX
    /\ Len(wire) < QMAX
    /\ IF MUTATION = 4
       THEN poisonedActs' = IF poisoned THEN 1 ELSE poisonedActs
       ELSE /\ ~poisoned
            /\ cliGen >= ackFloor
            /\ UNCHANGED poisonedActs
    /\ LET base == [inc |-> INC, gen |-> cliGen]
           raw == AdmitRaw(base) IN
       IF Remitted(raw) = "ok"
       THEN \* admitted: apply, acknowledge, publish the moved view
            /\ srvGen' = srvGen + 1
            /\ applied' = applied + 1
            /\ floor' = srvGen + 1
            /\ ackFloor' = srvGen + 1
            /\ wire' = Append(wire, Delta(srvGen + 1, pubGen, srvCrev))
            /\ pubGen' = srvGen + 1
            \* the engine may stage one clipboard payload (a yank);
            \* emit_effects drains it exactly once
            /\ IF stagedTotal < EFF_MAX /\ ~staged
               THEN \E stage \in {0, 1} :
                        /\ staged' = (stage = 1)
                        /\ stagedTotal' = stagedTotal + stage
               ELSE UNCHANGED <<staged, stagedTotal>>
            /\ UNCHANGED <<srvCrev, pubCrev, emitted, handshaken, byed,
                           finalAck, cliGen, hasView, poisoned,
                           staleRefusals, futureRefusals, foreignRefusals,
                           drops, resyncs, recoveries, rogueActs,
                           sameGenPubs, poisons, staleApplied,
                           futureApplied, foreignApplied, clearedBadly,
                           emptyPubs, postByePubs>>
       ELSE \* refused: a typed outcome; nothing is applied
            /\ RefuseWith(raw)
            /\ UNCHANGED <<engVars, pubVars, admVars, sessVars, effVars,
                           wire, cliVars, drops, resyncs, recoveries,
                           rogueActs, sameGenPubs, poisons, staleApplied,
                           futureApplied, foreignApplied, clearedBadly,
                           emptyPubs, postByePubs>>

\* A rogue client (or a stale client of a previous backend process) sends
\* an action on a deliberately bad base. The honest server refuses typed;
\* mutants 1-3 apply it. Rogue application raises the floor exactly as an
\* honest application does — the rogue IS this session's client.
RogueBases ==
    (IF floor >= 1 THEN {[inc |-> INC, gen |-> floor - 1]} ELSE {})
    \cup (IF pubGen < GEN_MAX THEN {[inc |-> INC, gen |-> pubGen + 1]}
          ELSE {})
    \cup {[inc |-> FOREIGN, gen |-> pubGen]}

RogueAct ==
    /\ handshaken /\ ~byed
    /\ rogueActs < ROGUE_MAX
    /\ srvGen < GEN_MAX
    /\ applied < ACT_MAX
    /\ Len(wire) < QMAX
    /\ rogueActs' = rogueActs + 1
    /\ \E base \in RogueBases :
        LET raw == AdmitRaw(base) IN
        IF Remitted(raw) = "ok"
        THEN /\ srvGen' = srvGen + 1
             /\ applied' = applied + 1
             /\ floor' = srvGen + 1
             /\ wire' = Append(wire, Delta(srvGen + 1, pubGen, srvCrev))
             /\ pubGen' = srvGen + 1
             /\ staleApplied' = IF raw = "stale" THEN 1 ELSE staleApplied
             /\ futureApplied' = IF raw = "future" THEN 1
                                 ELSE futureApplied
             /\ foreignApplied' = IF raw = "foreign" THEN 1
                                  ELSE foreignApplied
             /\ UNCHANGED <<srvCrev, pubCrev, sessVars, effVars, cliVars,
                            staleRefusals, futureRefusals, foreignRefusals,
                            drops, resyncs, recoveries, sameGenPubs,
                            poisons, poisonedActs, clearedBadly, emptyPubs,
                            postByePubs>>
        ELSE /\ RefuseWith(raw)
             /\ UNCHANGED <<engVars, pubVars, floor, applied, sessVars,
                            effVars, wire, cliVars, drops, resyncs,
                            recoveries, sameGenPubs, poisons, mutVars>>

\* Explicit resynchronization: the acknowledgement carries the current
\* generation and the complete current snapshot follows — the only
\* recovery from a dropped delta (serve.rs resync). A snapshot raises the
\* admission floor.
Resync ==
    /\ handshaken /\ ~byed
    /\ resyncs < RESYNC_MAX
    /\ Len(wire) < QMAX
    /\ resyncs' = resyncs + 1
    /\ recoveries' = IF poisoned THEN recoveries + 1 ELSE recoveries
    /\ floor' = srvGen
    /\ ackFloor' = srvGen
    /\ wire' = Append(wire, Snap(srvGen, srvCrev))
    /\ pubGen' = srvGen
    /\ pubCrev' = srvCrev
    /\ UNCHANGED <<engVars, applied, sessVars, effVars, cliGen, hasView,
                   poisoned, drops, rogueActs, staleRefusals,
                   futureRefusals, foreignRefusals, sameGenPubs, poisons,
                   mutVars>>

\* An engine-side observation (terminal output, picker streaming, a save
\* completion) moves the view between requests: the generation advances,
\* or semantic state moves at the same generation (serve.rs
\* pending_observations — the same-generation delta). Asynchronous
\* publications never raise the admission floor: input is not racy.
AsyncPublish ==
    /\ handshaken /\ ~byed
    /\ Len(wire) < QMAX
    /\ \/ /\ srvGen < GEN_MAX
          /\ srvGen' = srvGen + 1
          /\ wire' = Append(wire, Delta(srvGen + 1, pubGen, srvCrev))
          /\ pubGen' = srvGen + 1
          /\ UNCHANGED <<srvCrev, pubCrev, sameGenPubs>>
       \/ /\ srvCrev < CREV_MAX
          /\ srvCrev' = srvCrev + 1
          \* a publication at the generation the previous publication
          \* already carried: the same-generation delta
          /\ sameGenPubs' = IF pubGen = srvGen THEN Cap2(sameGenPubs)
                            ELSE sameGenPubs
          /\ wire' = Append(wire, Delta(srvGen, pubGen, srvCrev + 1))
          /\ pubCrev' = srvCrev + 1
          /\ UNCHANGED <<srvGen, pubGen>>
    /\ UNCHANGED <<admVars, sessVars, effVars, cliVars, drops, resyncs,
                   recoveries, rogueActs, staleRefusals, futureRefusals,
                   foreignRefusals, poisons, mutVars>>

\* MUTATION 6: the server publishes a view identical to the last
\* publication — an empty success. Honest publish sends nothing when the
\* observation is unchanged.
EmptyPublish ==
    /\ MUTATION = 6
    /\ handshaken /\ ~byed
    /\ pubGen > 0
    /\ Len(wire) < QMAX
    /\ wire' = Append(wire, Delta(pubGen, pubGen, pubCrev))
    /\ emptyPubs' = 1
    /\ UNCHANGED <<engVars, pubVars, admVars, sessVars, effVars, cliVars,
                   drops, resyncs, recoveries, rogueActs, staleRefusals,
                   futureRefusals, foreignRefusals, sameGenPubs, poisons,
                   staleApplied, futureApplied, foreignApplied,
                   poisonedActs, clearedBadly, postByePubs>>

\* emit_effects: one host effect per staged payload, monotone ids (the id
\* ordering makes uniqueness structural; the count is the record).
\* MUTATION 7: the staged payload is not cleared and is emitted twice.
EmitEffect ==
    /\ staged /\ ~byed
    /\ emitted < EFF_MAX + 1
    /\ emitted' = emitted + 1
    /\ staged' = (MUTATION = 7)
    /\ UNCHANGED <<engVars, pubVars, admVars, sessVars, stagedTotal, wire,
                   cliVars, witVars, mutVars>>

\* The authorized orderly shutdown: the final acknowledgement, the engine
\* finishes its background work, the bye is the last word. Staged effects
\* are drained first. MUTATION 8: the bye goes out without the ack.
ClientShutdown ==
    /\ handshaken /\ ~byed
    /\ ~staged
    /\ finalAck' = (MUTATION # 8)
    /\ byed' = TRUE
    /\ UNCHANGED <<engVars, pubVars, admVars, handshaken, effVars, wire,
                   cliVars, witVars, mutVars>>

\* MUTATION 9: a publication goes out after the bye.
PostByePublish ==
    /\ MUTATION = 9
    /\ byed
    /\ Len(wire) < QMAX
    /\ wire' = Append(wire, Delta(srvGen, pubGen, srvCrev))
    /\ postByePubs' = 1
    /\ UNCHANGED <<engVars, pubVars, admVars, sessVars, effVars, cliVars,
                   witVars, staleApplied, futureApplied, foreignApplied,
                   poisonedActs, clearedBadly, emptyPubs>>

\* The client applies the head publication (client.rs apply): a snapshot
\* is valid against any prior state and clears poisoning; a delta applies
\* only onto its exact base from this incarnation. MUTATION 5: a poisoned
\* client accepts a delta as if it cleared poisoning.
ClientApply ==
    /\ Len(wire) > 0
    /\ ~byed
    /\ LET pub == Head(wire) IN
       /\ wire' = Tail(wire)
       /\ IF pub.kind = "snap"
          THEN /\ poisoned' = FALSE
               /\ hasView' = TRUE
               /\ cliGen' = pub.gen
               /\ UNCHANGED <<poisons, clearedBadly>>
          ELSE IF pub.inc # INC
               THEN \* a foreign delta poisons (IncarnationChanged)
                    /\ poisoned' = TRUE
                    /\ poisons' = IF poisoned THEN poisons
                                  ELSE Cap2(poisons)
                    /\ UNCHANGED <<cliGen, hasView, clearedBadly>>
               ELSE IF poisoned
                    THEN \* already poisoned: only a snapshot recovers
                         IF MUTATION = 5
                         THEN /\ poisoned' = FALSE
                              /\ cliGen' = pub.gen
                              /\ clearedBadly' = 1
                              /\ UNCHANGED <<hasView, poisons>>
                         ELSE UNCHANGED <<cliGen, hasView, poisoned,
                                          poisons, clearedBadly>>
                    ELSE IF pub.base # cliGen
                         THEN \* a dropped/reordered delta poisons
                              /\ poisoned' = TRUE
                              /\ poisons' = Cap2(poisons)
                              /\ UNCHANGED <<cliGen, hasView, clearedBadly>>
                         ELSE /\ cliGen' = pub.gen
                              /\ UNCHANGED <<hasView, poisoned, poisons,
                                             clearedBadly>>
    /\ UNCHANGED <<engVars, pubVars, admVars, sessVars, effVars, ackFloor,
                   drops, resyncs, recoveries, rogueActs, staleRefusals,
                   futureRefusals, foreignRefusals, sameGenPubs,
                   staleApplied, futureApplied, foreignApplied,
                   poisonedActs, emptyPubs, postByePubs>>

\* The transport dropped the head publication (a lost delta). The client
\* learns of the loss only from the next mismatched delta.
ClientDrop ==
    /\ Len(wire) > 0
    /\ ~byed
    /\ drops < DROP_MAX
    /\ wire' = Tail(wire)
    /\ drops' = drops + 1
    /\ UNCHANGED <<engVars, pubVars, admVars, sessVars, effVars, cliVars,
                   resyncs, recoveries, rogueActs, staleRefusals,
                   futureRefusals, foreignRefusals, sameGenPubs, poisons,
                   mutVars>>

Next ==
    \/ Hello
    \/ ClientAct
    \/ RogueAct
    \/ Resync
    \/ AsyncPublish
    \/ EmptyPublish
    \/ EmitEffect
    \/ ClientShutdown
    \/ PostByePublish
    \/ ClientApply
    \/ ClientDrop

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* VF14 named safety properties.                                          *)
(***************************************************************************)

\* No action was applied on a base below the client-known floor.
StaleNeverActs == staleApplied = 0

\* No action was applied on a base beyond the newest publication.
FutureNeverActs == futureApplied = 0

\* No action was applied on a foreign incarnation's base.
ForeignNeverActs == foreignApplied = 0

\* A poisoned client never acts.
PoisonedNeverActs == poisonedActs = 0

\* Poisoning clears only by applying a snapshot (resync).
PoisonedUntilSnapshot == clearedBadly = 0

\* No publication repeats the previous publication's stamp: there is no
\* empty success.
NoEmptyPublication == emptyPubs = 0

\* Host effects never exceed the payloads admitted actions staged.
EffectExactlyOnce == emitted <= stagedTotal

\* The bye follows the shutdown's final acknowledgement.
ByeAfterFinalAck == byed => finalAck

\* Nothing publishes after the bye.
NoPostByePublication == postByePubs = 0

\* The client cache never claims a generation the backend never published.
CliNeverAhead == cliGen <= pubGen

\* Publications never claim a generation the engine never prepared.
PublishedNeverAhead == pubGen <= srvGen

\* The bye never precedes a staged (unemitted) effect.
ByeNeverStrandsEffects == byed => ~staged

(***************************************************************************)
(* Non-vacuity witnesses: each must be REACHABLE in the honest model      *)
(* (checked as an expected-to-fail invariant over the coverage config).   *)
(***************************************************************************)

\* A publication was dropped on the wire.
WitnessNoDrop == drops = 0

\* A dropped delta poisoned the client.
WitnessNoPoison == poisons = 0

\* A poisoned client resynced.
WitnessNoRecovery == recoveries = 0

\* A clipboard effect was emitted.
WitnessNoEffect == emitted = 0

\* The server refused a stale base typed.
WitnessNoStaleRefusal == staleRefusals = 0

\* The server refused a future base typed.
WitnessNoFutureRefusal == futureRefusals = 0

\* The server refused a foreign incarnation's base typed.
WitnessNoForeignRefusal == foreignRefusals = 0

\* An orderly shutdown completed.
WitnessNoBye == ~byed

\* An asynchronous publication outran the client cache (the non-racy
\* input case: the client's next act on its older base is still admitted).
WitnessNoOutrun == ~(hasView /\ ~poisoned /\ pubGen > cliGen)

\* A same-generation delta was published (semantic state moved at one
\* preparation generation).
WitnessNoSameGenDelta == sameGenPubs = 0

=============================================================================
