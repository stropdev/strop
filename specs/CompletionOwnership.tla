---- MODULE CompletionOwnership ----
(***************************************************************************
0059 completion query/index/publication/acceptance ownership.

An epoch abstracts the complete source/view/revision/settings/service-binding
witness checked by context.rs and delivery.rs. Selection is independently
stamped. ChangeContext deliberately does NOT revoke the mailbox's query: this
conservative interleaving tests the engine guard independently of native
cancellation. Concrete correspondence tests must establish every epoch input;
this model does not prove path comparison, rope geometry, parsing or atomics.

One latest queued query and one executing query share a persistent index.
Retained records have native and optional UI owners. The finite charge domain
models whole-payload count/byte admission, not an RSS measurement. ReleaseUI
never destroys the last owner in the honest model; retirement is native-only.

Mutants: 1 queued-query append; 2 restart index on query; 3 stale native publish;
4 skip independent presentation freshness; 5 release native owner at UI
delivery; 6 bypass retained byte/count admission; 7 skip acceptance freshness.
***************************************************************************)
EXTENDS Integers, Sequences, FiniteSets, TLC
CONSTANTS MAXGEN, NSLOTS, RETAINED, BUDGET, MUTATION
VARIABLE s
vars == <<s>>
Ids == 1..MAXGEN
Slots == 1..NSLOTS
Empty == [query |-> 0, epoch |-> 0, choice |-> 0, kind |-> "rows",
          native |-> FALSE, ui |-> FALSE, cost |-> 1]
Occupied(p) == p.native \/ p.ui
Count(st) == Cardinality({i \in Slots : Occupied(st.data[i])})
Bytes(st) == Cardinality({i \in Slots : Occupied(st.data[i]) /\ st.data[i].cost = 1})
           + 2 * Cardinality({i \in Slots : Occupied(st.data[i]) /\ st.data[i].cost = 2})
NativeOwns(p) == s.enabled /\ p.query # 0 /\ p.query = s.current
                /\ (p.kind = "rows" \/ p.choice = s.choice)
Current(p) == NativeOwns(p) /\ p.epoch = s.epoch
Room(cost) == MUTATION = 6 \/ (Count(s) < RETAINED /\ Bytes(s) + cost <= BUDGET)
Payload(q, kind) == [query |-> q, epoch |-> s.owners[q], choice |-> s.choice,
                    kind |-> kind, native |-> TRUE, ui |-> FALSE,
                    cost |-> IF kind = "rows" THEN 2 ELSE 1]
Init == s = [next |-> 0, current |-> 0, epoch |-> 0, choice |-> 0,
             enabled |-> TRUE, queued |-> <<>>, working |-> 0,
             owners |-> [q \in Ids |-> 0], indexStep |-> 0,
             data |-> [i \in Slots |-> Empty], prepared |-> Empty,
             visible |-> 0, stalePublish |-> FALSE, staleApply |-> FALSE,
             staleView |-> FALSE, uiDestructor |-> FALSE, restarted |-> FALSE,
             applied |-> FALSE, retired |-> FALSE, cancelled |-> FALSE]
\* TLAPS 1.5 requires explicit records, not nested state EXCEPT updates.
StateRecord(newnext, newcurrent, newepoch, newchoice, newenabled, newqueued, newworking, newowners, newindexStep, newdata, newprepared, newvisible, newstalePublish, newstaleApply, newstaleView, newuiDestructor, newrestarted, newapplied, newretired, newcancelled) ==
    [next |-> newnext,
     current |-> newcurrent,
     epoch |-> newepoch,
     choice |-> newchoice,
     enabled |-> newenabled,
     queued |-> newqueued,
     working |-> newworking,
     owners |-> newowners,
     indexStep |-> newindexStep,
     data |-> newdata,
     prepared |-> newprepared,
     visible |-> newvisible,
     stalePublish |-> newstalePublish,
     staleApply |-> newstaleApply,
     staleView |-> newstaleView,
     uiDestructor |-> newuiDestructor,
     restarted |-> newrestarted,
     applied |-> newapplied,
     retired |-> newretired,
     cancelled |-> newcancelled]
WithOwners(p, native, ui) ==
    [query |-> p.query, epoch |-> p.epoch, choice |-> p.choice, kind |-> p.kind,
     native |-> native, ui |-> ui, cost |-> p.cost]
NewQuery == /\ s.enabled /\ s.next < MAXGEN
            /\ LET q == s.next + 1 IN
               s' = StateRecord(
                   q,
                   q,
                   s.epoch,
                   s.choice,
                   s.enabled,
                   IF MUTATION = 1 THEN Append(s.queued, q) ELSE <<q>>,
                   s.working,
                   [s.owners EXCEPT ![q] = s.epoch],
                   IF MUTATION = 2 THEN 0 ELSE s.indexStep,
                   s.data,
                   s.prepared,
                   0,
                   s.stalePublish,
                   s.staleApply,
                   s.staleView,
                   s.uiDestructor,
                   s.restarted \/ (MUTATION = 2 /\ s.indexStep > 0),
                   s.applied,
                   s.retired,
                   s.cancelled
               )
IndexStep == /\ s.enabled /\ s.indexStep < 2
             /\ s' = StateRecord(
                    s.next,
                    s.current,
                    s.epoch,
                    s.choice,
                    s.enabled,
                    s.queued,
                    s.working,
                    s.owners,
                    s.indexStep + 1,
                    s.data,
                    s.prepared,
                    s.visible,
                    s.stalePublish,
                    s.staleApply,
                    s.staleView,
                    s.uiDestructor,
                    s.restarted,
                    s.applied,
                    s.retired,
                    s.cancelled
                )
BeginQuery == /\ s.enabled /\ s.working = 0 /\ Len(s.queued) > 0 /\ s.indexStep = 2
              /\ s' = StateRecord(
                     s.next,
                     s.current,
                     s.epoch,
                     s.choice,
                     s.enabled,
                     Tail(s.queued),
                     Head(s.queued),
                     s.owners,
                     s.indexStep,
                     s.data,
                     s.prepared,
                     s.visible,
                     s.stalePublish,
                     s.staleApply,
                     s.staleView,
                     s.uiDestructor,
                     s.restarted,
                     s.applied,
                     s.retired,
                     s.cancelled
                 )
PublishRows(i) == /\ i \in Slots /\ ~Occupied(s.data[i]) /\ s.working # 0 /\ Room(2)
                  /\ LET p == Payload(s.working, "rows") IN
                     /\ (NativeOwns(p) \/ MUTATION = 3)
                     /\ s' = StateRecord(
                            s.next,
                            s.current,
                            s.epoch,
                            s.choice,
                            s.enabled,
                            s.queued,
                            0,
                            s.owners,
                            s.indexStep,
                            [s.data EXCEPT ![i] = p],
                            s.prepared,
                            s.visible,
                            s.stalePublish \/ ~NativeOwns(p),
                            s.staleApply,
                            s.staleView,
                            s.uiDestructor,
                            s.restarted,
                            s.applied,
                            s.retired,
                            s.cancelled
                        )
DropObsoleteWork == /\ s.working # 0 /\ (~s.enabled \/ s.working # s.current)
                    /\ s' = StateRecord(
                           s.next,
                           s.current,
                           s.epoch,
                           s.choice,
                           s.enabled,
                           s.queued,
                           0,
                           s.owners,
                           s.indexStep,
                           s.data,
                           s.prepared,
                           s.visible,
                           s.stalePublish,
                           s.staleApply,
                           s.staleView,
                           s.uiDestructor,
                           s.restarted,
                           s.applied,
                           s.retired,
                           s.cancelled
                       )
Choose == /\ s.visible # 0 /\ Current(s.data[s.visible]) /\ s.choice < MAXGEN
          /\ s' = StateRecord(
                 s.next,
                 s.current,
                 s.epoch,
                 s.choice + 1,
                 s.enabled,
                 s.queued,
                 s.working,
                 s.owners,
                 s.indexStep,
                 s.data,
                 s.prepared,
                 s.visible,
                 s.stalePublish,
                 s.staleApply,
                 s.staleView,
                 s.uiDestructor,
                 s.restarted,
                 s.applied,
                 s.retired,
                 s.cancelled
             )
Prepare == /\ s.current # 0 /\ s.choice > 0 /\ s.visible # 0
           /\ Current(s.data[s.visible])
           /\ s' = StateRecord(
                  s.next,
                  s.current,
                  s.epoch,
                  s.choice,
                  s.enabled,
                  s.queued,
                  s.working,
                  s.owners,
                  s.indexStep,
                  s.data,
                  Payload(s.current, "plan"),
                  s.visible,
                  s.stalePublish,
                  s.staleApply,
                  s.staleView,
                  s.uiDestructor,
                  s.restarted,
                  s.applied,
                  s.retired,
                  s.cancelled
              )
PublishPlan(i) == /\ i \in Slots /\ ~Occupied(s.data[i]) /\ Room(1)
                  /\ s.prepared.query # 0 /\ NativeOwns(s.prepared)
                  /\ s' = StateRecord(
                         s.next,
                         s.current,
                         s.epoch,
                         s.choice,
                         s.enabled,
                         s.queued,
                         s.working,
                         s.owners,
                         s.indexStep,
                         [s.data EXCEPT ![i] = s.prepared],
                         Empty,
                         s.visible,
                         s.stalePublish,
                         s.staleApply,
                         s.staleView,
                         s.uiDestructor,
                         s.restarted,
                         s.applied,
                         s.retired,
                         s.cancelled
                     )
Acquire(i) == /\ i \in Slots /\ s.data[i].native /\ ~s.data[i].ui
              /\ s' = StateRecord(
                     s.next,
                     s.current,
                     s.epoch,
                     s.choice,
                     s.enabled,
                     s.queued,
                     s.working,
                     s.owners,
                     s.indexStep,
                     [s.data EXCEPT ![i] = WithOwners(s.data[i], MUTATION # 5, TRUE)],
                     s.prepared,
                     s.visible,
                     s.stalePublish,
                     s.staleApply,
                     s.staleView,
                     s.uiDestructor,
                     s.restarted,
                     s.applied,
                     s.retired,
                     s.cancelled
                 )
Show(i) == /\ i \in Slots /\ s.data[i].ui /\ s.data[i].kind = "rows"
           /\ (Current(s.data[i]) \/ MUTATION = 4)
           /\ s' = StateRecord(
                  s.next,
                  s.current,
                  s.epoch,
                  s.choice,
                  s.enabled,
                  s.queued,
                  s.working,
                  s.owners,
                  s.indexStep,
                  s.data,
                  s.prepared,
                  i,
                  s.stalePublish,
                  s.staleApply,
                  s.staleView \/ ~Current(s.data[i]),
                  s.uiDestructor,
                  s.restarted,
                  s.applied,
                  s.retired,
                  s.cancelled
              )
Apply(i) == /\ i \in Slots /\ s.data[i].ui /\ s.data[i].kind = "plan"
            /\ (Current(s.data[i]) \/ MUTATION = 7)
            /\ s' = StateRecord(
                   s.next,
                   0,
                   s.epoch,
                   s.choice,
                   s.enabled,
                   <<>>,
                   s.working,
                   s.owners,
                   s.indexStep,
                   s.data,
                   s.prepared,
                   0,
                   s.stalePublish,
                   s.staleApply \/ ~Current(s.data[i]),
                   s.staleView,
                   s.uiDestructor,
                   s.restarted,
                   TRUE,
                   s.retired,
                   s.cancelled
               )
ReleaseUI(i) == /\ i \in Slots /\ s.data[i].ui
                /\ s' = StateRecord(
                       s.next,
                       s.current,
                       s.epoch,
                       s.choice,
                       s.enabled,
                       s.queued,
                       s.working,
                       s.owners,
                       s.indexStep,
                       [s.data EXCEPT ![i] = WithOwners(s.data[i], s.data[i].native, FALSE)],
                       s.prepared,
                       IF s.visible = i THEN 0 ELSE s.visible,
                       s.stalePublish,
                       s.staleApply,
                       s.staleView,
                       s.uiDestructor \/ ~s.data[i].native,
                       s.restarted,
                       s.applied,
                       s.retired,
                       s.cancelled
                   )
Retire(i) == /\ i \in Slots /\ s.data[i].native /\ ~s.data[i].ui /\ ~NativeOwns(s.data[i])
             /\ s' = StateRecord(
                    s.next,
                    s.current,
                    s.epoch,
                    s.choice,
                    s.enabled,
                    s.queued,
                    s.working,
                    s.owners,
                    s.indexStep,
                    [s.data EXCEPT ![i] = Empty],
                    s.prepared,
                    s.visible,
                    s.stalePublish,
                    s.staleApply,
                    s.staleView,
                    s.uiDestructor,
                    s.restarted,
                    s.applied,
                    TRUE,
                    s.cancelled
                )
Cancel == /\ s.current # 0
          /\ s' = StateRecord(
                 s.next,
                 0,
                 s.epoch,
                 s.choice,
                 s.enabled,
                 <<>>,
                 s.working,
                 s.owners,
                 s.indexStep,
                 s.data,
                 s.prepared,
                 0,
                 s.stalePublish,
                 s.staleApply,
                 s.staleView,
                 s.uiDestructor,
                 s.restarted,
                 s.applied,
                 s.retired,
                 TRUE
             )
ChangeContext == /\ s.epoch < MAXGEN
                 /\ s' = StateRecord(
                        s.next,
                        s.current,
                        s.epoch + 1,
                        s.choice,
                        s.enabled,
                        s.queued,
                        s.working,
                        s.owners,
                        s.indexStep,
                        s.data,
                        s.prepared,
                        0,
                        s.stalePublish,
                        s.staleApply,
                        s.staleView,
                        s.uiDestructor,
                        s.restarted,
                        s.applied,
                        s.retired,
                        s.cancelled
                    )
Disable == /\ s.enabled
           /\ s' = StateRecord(
                  s.next,
                  0,
                  s.epoch,
                  s.choice,
                  FALSE,
                  <<>>,
                  s.working,
                  s.owners,
                  s.indexStep,
                  s.data,
                  s.prepared,
                  0,
                  s.stalePublish,
                  s.staleApply,
                  s.staleView,
                  s.uiDestructor,
                  s.restarted,
                  s.applied,
                  s.retired,
                  s.cancelled
              )
Next == NewQuery \/ IndexStep \/ BeginQuery \/ DropObsoleteWork \/ Choose \/ Prepare
        \/ Cancel \/ ChangeContext \/ Disable
        \/ \E i \in Slots : PublishRows(i) \/ PublishPlan(i) \/ Acquire(i) \/ Show(i)
                            \/ Apply(i) \/ ReleaseUI(i) \/ Retire(i)
Spec == Init /\ [][Next]_vars
PayloadType == [query : 0..MAXGEN, epoch : 0..MAXGEN, choice : 0..MAXGEN,
                kind : {"rows", "plan"}, native : BOOLEAN, ui : BOOLEAN, cost : 1..2]
TypeOK == /\ s.next \in 0..MAXGEN /\ s.current \in 0..MAXGEN /\ s.working \in 0..MAXGEN
          /\ s.epoch \in 0..MAXGEN /\ s.choice \in 0..MAXGEN /\ s.enabled \in BOOLEAN
          /\ s.queued \in Seq(Ids) /\ s.owners \in [Ids -> 0..MAXGEN]
          /\ s.indexStep \in 0..2 /\ s.data \in [Slots -> PayloadType]
          /\ s.prepared \in PayloadType /\ s.visible \in 0..NSLOTS
          /\ \A name \in {"stalePublish", "staleApply", "staleView", "uiDestructor",
                           "restarted", "applied", "retired", "cancelled"} : s[name] \in BOOLEAN
LatestOnly == Len(s.queued) <= 1
IndexNotRestartedByQuery == ~s.restarted
NativePublicationOwned == ~s.stalePublish
NoStalePresentationOrApply == ~s.staleView /\ ~s.staleApply
NativeFinalDestruction == ~s.uiDestructor
BoundedRetention == Count(s) <= RETAINED /\ Bytes(s) <= BUDGET
WitnessNoApply == ~s.applied
WitnessNoRetire == ~s.retired
WitnessNoCancel == ~s.cancelled
=============================================================================
