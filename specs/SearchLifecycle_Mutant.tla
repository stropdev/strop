---- MODULE SearchLifecycle_Mutant ----
(***************************************************************************)
(* KEPT MUTANT (0063 6.7 negative control): ProviderPartial publishes   *)
(* without the generation guard, and Accept consumes without the         *)
(* source-revision check. This mutant must die by exactly                *)
(* RowsCurrent + StaleAcceptsNever - any other kill is a modeling bug.   *)
(***************************************************************************)

EXTENDS Integers, FiniteSets, TLC

CONSTANTS GENS,        \* query generation budget (0..GENS)
          PATHS,       \* candidate files
          REV_MAX,     \* source revision budget
          WARM_MAX     \* concurrent warm server attaches

VARIABLES phase,       \* "closed" | "open"
          qgen,        \* the current query generation
          scope,       \* the opened scope identity
          inFlight,    \* running provider jobs: [gen, scope]
          rows,        \* published rows: [gen, scope, path, rev]
          accepted,    \* rows consumed by Enter
          staleAccepts, \* acceptance-with-moved-source events (always 0)
          truncated,   \* the index hit its bounds this generation
          completeF,   \* the picker's "no more coming" flag
          bufferRev,   \* the dirty source's revision
          warm,        \* warm servers in flight
          warmQueue    \* projects waiting for a warm slot

TypeOK ==
    /\ phase \in {"closed", "open"}
    /\ qgen \in 0..GENS
    /\ scope \in {"s1", "s2"}
    /\ \A j \in inFlight : j.gen \in 0..GENS /\ j.scope \in {"s1", "s2"}
    /\ \A r \in rows : r.gen \in 0..GENS /\ r.scope \in {"s1", "s2"}
                       /\ r.path \in PATHS /\ r.rev \in 0..REV_MAX
    /\ \A a \in accepted : a \in [gen: 0..GENS, scope: {"s1","s2"},
                                  path: PATHS, rev: 0..REV_MAX]
    /\ staleAccepts \in 0..5
    /\ truncated \in BOOLEAN
    /\ completeF \in BOOLEAN
    /\ bufferRev \in 0..REV_MAX
    /\ warm \in 0..WARM_MAX
    /\ warmQueue \in 0..3

Init ==
    /\ phase = "closed"
    /\ qgen = 0
    /\ scope = "s1"
    /\ inFlight = {}
    /\ rows = {}
    /\ accepted = {}
    /\ staleAccepts = 0
    /\ truncated = FALSE
    /\ completeF = FALSE
    /\ bufferRev = 0
    /\ warm = 0
    /\ warmQueue = 0

OpenPicker ==
    /\ phase = "closed"
    /\ phase' = "open"
    /\ rows' = {}
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ staleAccepts' = staleAccepts
    /\ truncated' = FALSE
    /\ completeF' = FALSE
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = warmQueue

ClosePopup ==
    /\ phase = "open"
    /\ phase' = "closed"
    /\ inFlight' = inFlight          \* jobs finish; publication is phase-guarded
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ staleAccepts' = staleAccepts
    /\ truncated' = truncated
    /\ completeF' = FALSE            \* the claim dies with the surface
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = 0                \* warm-up is on demand: the queue dies

\* Typing retires the old generation: its rows purge, its jobs may still
\* land — the publication guard sends them to the trace, never the list.
TypeQuery ==
    /\ phase = "open"
    /\ qgen < GENS
    /\ phase' = phase
    /\ qgen' = qgen + 1
    /\ rows' = {}
    /\ truncated' = FALSE
    /\ completeF' = FALSE
    /\ accepted' = accepted
    /\ scope' = scope
    /\ inFlight' = inFlight \cup {[gen |-> qgen + 1, scope |-> scope]}
    /\ staleAccepts' = staleAccepts
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = warmQueue

\* Partial results from a running job. THE GUARD: current generation,
\* open surface, same scope. The mutant drops generation and scope.
ProviderPartial(j) ==
    /\ phase = "open"
    /\ phase' = phase
    /\ j \in inFlight
    \* MUTANT: publishes whatever generation the job carried.
    /\ \E p \in PATHS :
          rows' = rows \cup {[gen |-> j.gen, scope |-> j.scope,
                              path |-> p, rev |-> bufferRev]}
    /\ inFlight' = inFlight
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ staleAccepts' = staleAccepts
    /\ truncated' = truncated
    /\ completeF' = completeF
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = warmQueue

\* The job's terminal event: it frees the stream and, for the current
\* generation, may set the honest completion flag.
ProviderDone(j) ==
    /\ j \in inFlight
    /\ phase' = phase
    /\ inFlight' = inFlight \ {j}
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ staleAccepts' = staleAccepts
    /\ completeF' = IF j.gen = qgen /\ j.scope = scope /\ phase = "open"
                     THEN ~truncated
                     ELSE completeF
    /\ truncated' = truncated
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = warmQueue

\* The index hit its bounds: coverage is knowingly incomplete.
TruncateIndex ==
    /\ phase = "open"
    /\ phase' = phase
    /\ ~truncated
    /\ truncated' = TRUE
    /\ completeF' = FALSE
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ staleAccepts' = staleAccepts
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = warmQueue

\* The dirty source moves: every already-published row's revision is
\* now historical; acceptance must re-check (overlay in Rust).
EditBuffer ==
    /\ phase = "open"
    /\ bufferRev < REV_MAX
    /\ bufferRev' = bufferRev + 1
    /\ phase' = phase
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ staleAccepts' = staleAccepts
    /\ truncated' = truncated
    /\ completeF' = completeF
    /\ warm' = warm
    /\ warmQueue' = warmQueue

\* Enter consumes one row. THE GUARD: published now, current
\* generation, source revision still exact. The mutant drops the
\* revision check and its counter records the misacceptance.
Accept(r) ==
    /\ phase = "open"
    /\ r \in rows
    /\ r.gen = qgen
    /\ r.scope = scope
    \* MUTANT: no revision re-check - the dirty overlay is skipped.
    /\ accepted' = accepted \cup {r}
    /\ rows' = rows \ {r}
    /\ staleAccepts' = IF r.rev # bufferRev THEN staleAccepts + 1 ELSE staleAccepts
    /\ phase' = phase
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ truncated' = truncated
    /\ completeF' = completeF
    /\ bufferRev' = bufferRev
    /\ warm' = warm
    /\ warmQueue' = warmQueue

\* Bounded lazy warm-up of unopened projects (0063 §2).
WarmStart ==
    /\ phase = "open"
    /\ warmQueue > 0
    /\ warm < WARM_MAX
    /\ warm' = warm + 1
    /\ warmQueue' = warmQueue - 1
    /\ phase' = phase
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ staleAccepts' = staleAccepts
    /\ truncated' = truncated
    /\ completeF' = completeF
    /\ bufferRev' = bufferRev

WarmDone ==
    /\ warm > 0
    /\ warm' = warm - 1
    /\ phase' = phase
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ staleAccepts' = staleAccepts
    /\ truncated' = truncated
    /\ completeF' = completeF
    /\ bufferRev' = bufferRev
    /\ warmQueue' = warmQueue

WarmEnqueue ==
    /\ phase = "open"
    /\ warmQueue < 3
    /\ warmQueue' = warmQueue + 1
    /\ phase' = phase
    /\ rows' = rows
    /\ accepted' = accepted
    /\ qgen' = qgen
    /\ scope' = scope
    /\ inFlight' = inFlight
    /\ staleAccepts' = staleAccepts
    /\ truncated' = truncated
    /\ completeF' = completeF
    /\ bufferRev' = bufferRev
    /\ warm' = warm

\* Reopening on another scope: the session restarts cold.
Restart ==
    /\ phase \in {"open", "closed"}
    /\ scope' = IF scope = "s1" THEN "s2" ELSE "s1"
    /\ phase' = "open"
    /\ qgen' = qgen
    /\ rows' = {}
    /\ accepted' = accepted
    /\ inFlight' = {}
    /\ staleAccepts' = staleAccepts
    /\ truncated' = FALSE
    /\ completeF' = FALSE
    /\ bufferRev' = bufferRev
    /\ warm' = 0
    /\ warmQueue' = 0

Next ==
    \/ OpenPicker
    \/ ClosePopup
    \/ TypeQuery
    \/ \E j \in inFlight : ProviderPartial(j)
    \/ \E j \in inFlight : ProviderDone(j)
    \/ TruncateIndex
    \/ EditBuffer
    \/ \E r \in rows : Accept(r)
    \/ WarmStart
    \/ WarmDone
    \/ WarmEnqueue
    \/ Restart

Spec == Init /\ [][Next]_<<phase, qgen, scope, inFlight, rows, accepted,
                          staleAccepts, truncated, completeF, bufferRev,
                          warm, warmQueue>>

(***************************************************************************)
(* §6.5 named safety properties.                                          *)
(***************************************************************************)

\* No retired-query publication: rows only ever belong to the current
\* generation (TypeQuery purges; publication is generation-guarded).
RowsCurrent == \A r \in rows : r.gen = qgen

\* No foreign results: every row names the opened scope.
RowsInScope == \A r \in rows : r.scope = scope

\* No known-stale symbol acceptance: the counter only moves in the
\* mutant's unguarded accept (in this spec it stays zero forever).
StaleAcceptsNever == staleAccepts = 0

\* No false completeness: the flag requires the current generation's
\* untruncated terminal (bounds keep the surface honestly partial).
CompletionHonest ==
    completeF => /\ truncated = FALSE
                 /\ phase = "open"
                 /\ \A j \in inFlight : j.gen # qgen \/ j.scope # scope

\* Resource bound: warm-up concurrency never exceeds the budget.
WarmBounded == warm <= WARM_MAX

=============================================================================
