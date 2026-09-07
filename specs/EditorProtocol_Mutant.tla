---- MODULE EditorProtocol_Mutant ----
(***************************************************************************)
(* KEPT MUTANT (R12): Deliver without the freshness guard.                *)
(*                                                                       *)
(* The stale/dead branch does what the 0023 cross-document bug did: it   *)
(* LANDS the ticket anyway and raises the misdelivery flag. TLC must     *)
(* kill this module by EXACTLY the NoMisapply invariant:                 *)
(* TypeOK, NoStalePane, NoWrongDocument, RevisionTracksPublications and  *)
(* TicketOneShot must all still hold — the mutant misdelivers, it does   *)
(* not corrupt types, panes, the journal, or the ticket machine. The     *)
(* gate (specs/gate.sh) fails unless the mutant's ONLY violated          *)
(* invariant is NoMisapply. If the mutant ever checks clean, NoMisapply  *)
(* lost its teeth; if any other invariant dies, the mutant drifted from  *)
(* the one bug it exists to embody.                                      *)
(*                                                                       *)
(* Executability: the bug is reachable — Arm a ticket, PublishLocal (or  *)
(* another Deliver) to move the revision, or CloseDoc the target, then   *)
(* Deliver: Fresh(r) is FALSE and the mutant applies anyway. (The R12    *)
(* repair: the old mutant bound misapplied' twice — once by the IF and   *)
(* once by a contradictory UNCHANGED — which disabled exactly the       *)
(* branch that carries the bug, so the mutant passed NoMisapply         *)
(* vacuously. It also carried a wrong module name,                      *)
(* EditorProtocol_Mutant_Mutant, which TLC cannot even load.)            *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS DOCS, PANES, REQS, MAXREV, MAXGEN

VARIABLES live, gen, rev, claims, paneOf, phase, ticket, landed, misapplied

NONE == 0
IDLE == 0
ARMED == 1
APPLIED == 2
DROPPED == 3

TypeOK ==
    /\ live \subseteq DOCS
    /\ gen \in [DOCS -> 0..MAXGEN]
    /\ rev \in [DOCS -> 0..MAXREV]
    /\ claims \in [DOCS -> SUBSET (0..MAXREV)]
    /\ paneOf \in [PANES -> DOCS \union {NONE}]
    /\ phase \in [REQS -> {IDLE, ARMED, APPLIED, DROPPED}]
    /\ ticket \in [REQS -> [doc: DOCS \union {NONE}, g: 0..MAXGEN, r: 0..MAXREV]]
    /\ landed \in [REQS -> [doc: DOCS \union {NONE}, g: 0..MAXGEN, r: 0..MAXREV]]
    /\ misapplied \in {FALSE, TRUE}

Init ==
    /\ live = {}
    /\ gen = [d \in DOCS |-> 0]
    /\ rev = [d \in DOCS |-> 0]
    /\ claims = [d \in DOCS |-> {}]
    /\ paneOf = [p \in PANES |-> NONE]
    /\ phase = [r \in REQS |-> IDLE]
    /\ ticket = [r \in REQS |-> [doc |-> NONE, g |-> 0, r |-> 0]]
    /\ landed = [r \in REQS |-> [doc |-> NONE, g |-> 0, r |-> 0]]
    /\ misapplied = FALSE

\* Everything below this line is EditorProtocol.tla verbatim except the
\* ELSE branch of Deliver — the single mutation.

OpenDoc(d) ==
    /\ d \notin live
    /\ gen[d] < MAXGEN
    /\ live' = live \union {d}
    /\ gen' = [gen EXCEPT ![d] = @ + 1]
    /\ rev' = [rev EXCEPT ![d] = 0]
    /\ claims' = [claims EXCEPT ![d] = {}]
    /\ UNCHANGED <<paneOf, phase, ticket, landed, misapplied>>

CloseDoc(d) ==
    /\ d \in live
    /\ live' = live \ {d}
    /\ paneOf' = [p \in PANES |->
        IF paneOf[p] = d
        THEN IF live \ {d} = {} THEN NONE ELSE CHOOSE x \in live \ {d} : TRUE
        ELSE paneOf[p]]
    /\ UNCHANGED <<gen, rev, claims, phase, ticket, landed, misapplied>>

SplitPane(p, d) ==
    /\ d \in live
    /\ paneOf' = [paneOf EXCEPT ![p] = d]
    /\ UNCHANGED <<live, gen, rev, claims, phase, ticket, landed, misapplied>>

PublishLocal(d) ==
    /\ d \in live
    /\ rev[d] < MAXREV
    /\ rev' = [rev EXCEPT ![d] = @ + 1]
    /\ claims' = [claims EXCEPT ![d] = @ \union {rev[d]}]
    /\ UNCHANGED <<live, gen, paneOf, phase, ticket, landed, misapplied>>

Arm(r, d) ==
    /\ d \in live
    /\ phase[r] = IDLE
    /\ phase' = [phase EXCEPT ![r] = ARMED]
    /\ ticket' = [ticket EXCEPT ![r] = [doc |-> d, g |-> gen[d], r |-> rev[d]]]
    /\ UNCHANGED <<live, gen, rev, claims, paneOf, landed, misapplied>>

Fresh(r) ==
    /\ ticket[r].doc \in live
    /\ ticket[r].g = gen[ticket[r].doc]
    /\ ticket[r].r = rev[ticket[r].doc]
    /\ rev[ticket[r].doc] < MAXREV

\* THE MUTATION: the ELSE branch applies the stale/dead ticket anyway
\* (and flags it) instead of dropping it. Everything else — including
\* rev/claims, which a real misdelivery does NOT get to publish
\* consistently — is the guarded spec.
Deliver(r) ==
    /\ phase[r] = ARMED
    /\ IF Fresh(r)
       THEN /\ phase' = [phase EXCEPT ![r] = APPLIED]
            /\ landed' = [landed EXCEPT ![r] = ticket[r]]
            /\ rev' = [rev EXCEPT ![ticket[r].doc] = @ + 1]
            /\ claims' = [claims EXCEPT ![ticket[r].doc] = @ \union {rev[ticket[r].doc]}]
            /\ UNCHANGED <<live, gen, paneOf, ticket, misapplied>>
       ELSE /\ misapplied' = TRUE
            /\ phase' = [phase EXCEPT ![r] = APPLIED]
            /\ landed' = [landed EXCEPT ![r] = ticket[r]]
            /\ UNCHANGED <<live, gen, rev, claims, paneOf, ticket>>

NoStalePane ==
    \A p \in PANES : paneOf[p] /= NONE => paneOf[p] \in live

NoWrongDocument ==
    \A r \in REQS :
        (phase[r] = APPLIED) =>
            /\ landed[r] = ticket[r]
            /\ \/ landed[r].doc \notin live
               \/ /\ landed[r].doc \in live
                  /\ \/ /\ landed[r].g = gen[landed[r].doc]
                        /\ landed[r].r < rev[landed[r].doc]
                     \/ landed[r].g < gen[landed[r].doc]

RevisionTracksPublications ==
    \A d \in DOCS :
        /\ rev[d] = Cardinality(claims[d])
        /\ \A c \in claims[d] : c < rev[d]

TicketOneShot ==
    \A r \in REQS :
        /\ (phase[r] = IDLE) <=> (ticket[r].doc = NONE)
        /\ (phase[r] = ARMED) => ticket[r].doc /= NONE
        /\ (phase[r] = APPLIED) <=> (landed[r].doc /= NONE)

NoMisapply == ~misapplied

Next ==
    \/ \E d \in DOCS : OpenDoc(d)
    \/ \E d \in DOCS : CloseDoc(d)
    \/ \E p \in PANES, d \in DOCS : SplitPane(p, d)
    \/ \E d \in DOCS : PublishLocal(d)
    \/ \E r \in REQS, d \in DOCS : Arm(r, d)
    \/ \E r \in REQS : Deliver(r)

Spec == Init /\ [][Next]_<<live, gen, rev, claims, paneOf, phase, ticket, landed, misapplied>>
=============================================================================
