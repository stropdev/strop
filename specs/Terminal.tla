---- MODULE Terminal ----
(***************************************************************************)
(* Terminal session lifecycle safety (0057 VF12) over the completed 0055   *)
(* implementation and the 0065 modal experience (D1/D2), as implemented    *)
(* in crates/strop-terminal/src/{model,service} and consumed by            *)
(* crates/strop-engine/src/editor/terminal/lifecycle.rs.                   *)
(*                                                                       *)
(*   LIFECYCLE  a session is Starting -> Running -> Closing ->            *)
(*              Exited/Failed; the terminal phases are absorbing          *)
(*              (model/mod.rs Phase; Phase::live). The exit boundary      *)
(*              clears the paste-confirmation ticket, drops the service   *)
(*              handle and forces every pane out of terminal-input; the   *)
(*              final frame is RETAINED ("output retained") and one       *)
(*              truthful terminal message is announced                    *)
(*              (lifecycle.rs apply_terminal_update).                     *)
(*                                                                       *)
(*   FRAMES     frames carry their session identity and a strictly        *)
(*              increasing revision; the editor entry rejects stale or    *)
(*              foreign publications ("terminal publication has stale     *)
(*              or foreign identity"). A frame installs into the          *)
(*              document projection eagerly UNLESS a pane inspects the    *)
(*              terminal — the pinned snapshot is a correctness           *)
(*              invariant (0065 D2, 0055 §7): output never drags an       *)
(*              inspected view; the explicit :terminal-refresh            *)
(*              re-installs the latest frame through                      *)
(*              install_terminal_frame, and returning to input follows    *)
(*              the live output.                                          *)
(*                                                                       *)
(*   INPUT      keys reach the child only while a pane owns terminal-     *)
(*              input AND the session is live; Esc stays child-bound      *)
(*              (0065 D1); after exit no input is routed to the dead      *)
(*              child.                                                    *)
(*                                                                       *)
(*   GEOMETRY   one resize authority per session (0055 T09): the editor   *)
(*              admits resizes (prepare_terminal_geometry); a child-      *)
(*              initiated size change is never applied.                   *)
(*                                                                       *)
(*   EFFECTS    terminal output/title/OSC never becomes unapproved        *)
(*              editor mutation, filesystem access, clipboard access or   *)
(*              execution: title/cwd/bell are inert metadata and host     *)
(*              control is denied (Effect::{ClipboardWriteDenied,         *)
(*              HostControlDenied}; ReportedDirectory is never a          *)
(*              context change).                                          *)
(*                                                                       *)
(* THE invariants (VF12's named safety properties):                       *)
(*                                                                       *)
(*   TypeOK              every variable stays in its declared finite      *)
(*                       domain                                           *)
(*   FrameFresh          published revisions never exceed what the child  *)
(*                       produced, and no stale/foreign frame was ever    *)
(*                       accepted                                         *)
(*   PinnedNeverDragged  no frame was auto-installed into an inspected    *)
(*                       (pinned) view                                    *)
(*   TerminalAbsorbing   no session left a terminal phase                 *)
(*   ExitBoundary        a terminal phase implies: no paste ticket, no    *)
(*                       service handle, no pane in terminal-input        *)
(*   ExitAnnouncedOnce   the terminal boundary was announced at most      *)
(*                       once per session                                 *)
(*   InputNeverToDead    no key was routed to a non-live session          *)
(*   GeometryOwned       no child-initiated resize was applied            *)
(*   OutputNeverMutates  no child output was applied as a host effect     *)
(*                                                                       *)
(* Deliberately faulty variants (configs flip MUTATION):                  *)
(*   MUTATION = 1  the entry drops the freshness/identity guard and       *)
(*                 accepts a stale frame; must die by FrameFresh          *)
(*   MUTATION = 2  frames auto-install while a pane inspects — output     *)
(*                 drags the pinned view; must die by PinnedNeverDragged  *)
(*   MUTATION = 3  an exited session resurrects to running; must die by   *)
(*                 TerminalAbsorbing and ExitAnnouncedOnce                *)
(*   MUTATION = 4  the exit boundary leaves the pane in terminal-         *)
(*                 input and a key routes to the dead child; must die     *)
(*                 by InputNeverToDead and ExitBoundary                   *)
(*   MUTATION = 5  an OSC clipboard write is honored; must die by         *)
(*                 OutputNeverMutates                                     *)
(*   MUTATION = 6  a child-initiated resize is applied; must die by       *)
(*                 GeometryOwned                                          *)
(*                                                                       *)
(* Not modeled: VT parsing/grid semantics and fonts (the vendored         *)
(* emulator is a named trusted boundary, 0057 §6), byte budgets (T06's    *)
(* queue/memory bounds are modelled in LspWire/events lanes and natively  *)
(* tested), paste-confirmation UX flow, and liveness (a child that never  *)
(* exits is a progress matter). Closing-window drain output is folded     *)
(* into ChildOutput being enabled in Closing.                             *)
(***************************************************************************)
EXTENDS Integers, FiniteSets, TLC

CONSTANTS SESSIONS,    \* terminal sessions
          REV_MAX,     \* frame revision budget per session
          MUTATION     \* 0 = honest; faulty variants above

VARIABLES phase,       \* per session: none|starting|running|closing|exited|failed
          childRev,    \* latest revision the session worker produced
          published,   \* latest revision the editor entry accepted
          projected,   \* revision installed in the document projection
          inspecting,  \* a pane inspects the pinned snapshot
          inputMode,   \* the pane owns terminal-input
          paste,       \* a paste-confirmation ticket is outstanding
          service,     \* the service handle is alive
          geometry,    \* the admitted PTY geometry
          exitNotes,   \* terminal boundary announcements per session
          keysToChild, \* keys delivered to the child (witness accounting)
          staleAccepted,    \* stale/foreign frame accepts (always 0)
          autoInstalled,    \* auto-installs into a pinned view (always 0)
          resurrected,      \* terminal->live transitions (always 0)
          routedToDead,     \* keys routed to a dead session (always 0)
          hostEffects,      \* child output applied as host effect (always 0)
          childResizeApplied, \* child-applied resizes (always 0)
          refreshes,    \* explicit :terminal-refresh events
          lateDropped,  \* updates dropped after the session ended
          denied,       \* denied host-effect requests
          reentries,    \* inspect -> input -> inspect cycles
          closings      \* editor-initiated closes observed

Live(s) == phase[s] \in {"starting", "running", "closing"}
Terminal(s) == phase[s] \in {"exited", "failed"}

TypeOK ==
    /\ phase \in [SESSIONS -> {"none", "starting", "running", "closing",
                               "exited", "failed"}]
    /\ childRev \in [SESSIONS -> 0..REV_MAX]
    /\ published \in [SESSIONS -> 0..REV_MAX]
    /\ projected \in [SESSIONS -> 0..REV_MAX]
    /\ inspecting \in [SESSIONS -> BOOLEAN]
    /\ inputMode \in [SESSIONS -> BOOLEAN]
    /\ paste \in [SESSIONS -> BOOLEAN]
    /\ service \in [SESSIONS -> BOOLEAN]
    /\ geometry \in [SESSIONS -> {"g1", "g2"}]
    /\ exitNotes \in [SESSIONS -> 0..2]
    /\ keysToChild \in 0..2
    /\ staleAccepted \in 0..1
    /\ autoInstalled \in 0..1
    /\ resurrected \in 0..1
    /\ routedToDead \in 0..1
    /\ hostEffects \in 0..1
    /\ childResizeApplied \in 0..1
    /\ refreshes \in 0..2
    /\ lateDropped \in 0..2
    /\ denied \in 0..2
    /\ reentries \in 0..2
    /\ closings \in 0..2

Init ==
    /\ phase = [s \in SESSIONS |-> "none"]
    /\ childRev = [s \in SESSIONS |-> 0]
    /\ published = [s \in SESSIONS |-> 0]
    /\ projected = [s \in SESSIONS |-> 0]
    /\ inspecting = [s \in SESSIONS |-> FALSE]
    /\ inputMode = [s \in SESSIONS |-> FALSE]
    /\ paste = [s \in SESSIONS |-> FALSE]
    /\ service = [s \in SESSIONS |-> FALSE]
    /\ geometry = [s \in SESSIONS |-> "g1"]
    /\ exitNotes = [s \in SESSIONS |-> 0]
    /\ keysToChild = 0
    /\ staleAccepted = 0
    /\ autoInstalled = 0
    /\ resurrected = 0
    /\ routedToDead = 0
    /\ hostEffects = 0
    /\ childResizeApplied = 0
    /\ refreshes = 0
    /\ lateDropped = 0
    /\ denied = 0
    /\ reentries = 0
    /\ closings = 0

\* :terminal — the editor launches a session with its service.
Launch(s) ==
    /\ phase[s] = "none"
    /\ phase' = [phase EXCEPT ![s] = "starting"]
    /\ service' = [service EXCEPT ![s] = TRUE]
    /\ inputMode' = [inputMode EXCEPT ![s] = TRUE]
    /\ UNCHANGED <<childRev, published, projected, inspecting, paste,
                   geometry, exitNotes, keysToChild, staleAccepted,
                   autoInstalled, resurrected, routedToDead, hostEffects,
                   childResizeApplied, refreshes, lateDropped, denied,
                   reentries, closings>>

ChildStart(s) ==
    /\ phase[s] = "starting"
    /\ phase' = [phase EXCEPT ![s] = "running"]
    /\ UNCHANGED <<childRev, published, projected, inspecting, inputMode,
                   paste, service, geometry, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, resurrected, routedToDead,
                   hostEffects, childResizeApplied, refreshes, lateDropped,
                   denied, reentries, closings>>

\* The child produces output (a newer frame revision). Drain output
\* during Closing is folded in here.
ChildOutput(s) ==
    /\ phase[s] \in {"running", "closing"}
    /\ childRev[s] < REV_MAX
    /\ childRev' = [childRev EXCEPT ![s] = @ + 1]
    /\ UNCHANGED <<phase, published, projected, inspecting, inputMode,
                   paste, service, geometry, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, resurrected, routedToDead,
                   hostEffects, childResizeApplied, refreshes, lateDropped,
                   denied, reentries, closings>>

\* A frame update reaches the editor entry. THE GUARD: the session is
\* live, the revision is fresher than the accepted one and never beyond
\* what the child produced (the identity half — the frame names this
\* session — is folded into the revision check; MUTATION 1 drops both).
\* Installation: eager for the initial frame and for uninspected views;
\* NEVER while a pane inspects — the snapshot stays pinned.
PublishFrame(s) ==
    /\ Live(s)
    /\ \E v \in 1..childRev[s] :
        /\ (MUTATION = 1 /\ v <= published[s] /\ staleAccepted' = 1)
           \/ (v > published[s] /\ staleAccepted' = staleAccepted)
        /\ published' = [published EXCEPT ![s] = v]
        \* installation policy
        /\ IF projected[s] = 0 \/ ~inspecting[s] \/ MUTATION = 2
           THEN /\ projected' = [projected EXCEPT ![s] = v]
                /\ autoInstalled' = IF MUTATION = 2 /\ inspecting[s]
                                           /\ projected[s] # 0
                                    THEN 1 ELSE autoInstalled
           ELSE /\ projected' = projected
                /\ autoInstalled' = autoInstalled
    /\ UNCHANGED <<phase, childRev, inspecting, inputMode, paste, service,
                   geometry, exitNotes, keysToChild, resurrected,
                   routedToDead, hostEffects, childResizeApplied, refreshes,
                   lateDropped, denied, reentries, closings>>

\* The child exits or the launch fails: the terminal boundary. Paste
\* ticket cleared, service dropped, panes forced out of terminal-input,
\* one truthful announcement, output retained.
ChildExit(s) ==
    /\ Live(s)
    /\ \E outcome \in {"exited", "failed"} :
        phase' = [phase EXCEPT ![s] = outcome]
    /\ paste' = [paste EXCEPT ![s] = FALSE]
    /\ service' = [service EXCEPT ![s] = FALSE]
    \* MUTATION 4: the boundary forgets to force panes out of
    \* terminal-input — a later key routes to the dead child.
    /\ inputMode' = IF MUTATION = 4 THEN inputMode
                    ELSE [inputMode EXCEPT ![s] = FALSE]
    /\ exitNotes' = [exitNotes EXCEPT ![s] = @ + 1]
    /\ UNCHANGED <<childRev, published, projected, inspecting, geometry,
                   keysToChild, staleAccepted, autoInstalled, resurrected,
                   routedToDead, hostEffects, childResizeApplied, refreshes,
                   lateDropped, denied, reentries, closings>>

\* MUTATION 3: a terminal session resurrects. Never enabled honestly.
Resurrect(s) ==
    /\ MUTATION = 3
    /\ Terminal(s)
    /\ phase' = [phase EXCEPT ![s] = "running"]
    /\ resurrected' = 1
    /\ UNCHANGED <<childRev, published, projected, inspecting, inputMode,
                   paste, service, geometry, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, routedToDead, hostEffects,
                   childResizeApplied, refreshes, lateDropped, denied,
                   reentries, closings>>

\* Editor-initiated stop (:terminal-close): Running -> Closing; the
\* child's exit then completes the boundary. Closing a VIEW is not
\* killing the process and is not modelled as a session step.
CloseTerminal(s) ==
    /\ phase[s] = "running"
    /\ phase' = [phase EXCEPT ![s] = "closing"]
    /\ closings' = IF closings < 2 THEN closings + 1 ELSE closings
    /\ UNCHANGED <<childRev, published, projected, inspecting, inputMode,
                   paste, service, geometry, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, resurrected, routedToDead,
                   hostEffects, childResizeApplied, refreshes, lateDropped,
                   denied, reentries>>

\* A paste-confirmation ticket is raised while the session is live.
PasteRequest(s) ==
    /\ Live(s)
    /\ ~paste[s]
    /\ paste' = [paste EXCEPT ![s] = TRUE]
    /\ UNCHANGED <<phase, childRev, published, projected, inspecting,
                   inputMode, service, geometry, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, resurrected, routedToDead,
                   hostEffects, childResizeApplied, refreshes, lateDropped,
                   denied, reentries, closings>>

\* Ctrl-\ Ctrl-N (0065 D1): leave terminal-input for the pinned snapshot.
InspectEnter(s) ==
    /\ Live(s)
    /\ inputMode[s]
    /\ inputMode' = [inputMode EXCEPT ![s] = FALSE]
    /\ inspecting' = [inspecting EXCEPT ![s] = TRUE]
    /\ UNCHANGED <<phase, childRev, published, projected, paste, service,
                   geometry, exitNotes, keysToChild, staleAccepted,
                   autoInstalled, resurrected, routedToDead, hostEffects,
                   childResizeApplied, refreshes, lateDropped, denied,
                   reentries, closings>>

\* Returning to input follows the live output: the latest frame installs.
\* After exit there is no input to return to — the retained snapshot is
\* re-inspectable but inputMode stays off.
InspectExit(s) ==
    /\ inspecting[s]
    /\ inspecting' = [inspecting EXCEPT ![s] = FALSE]
    /\ inputMode' = [inputMode EXCEPT ![s] = Live(s)]
    /\ projected' = [projected EXCEPT ![s] = published[s]]
    /\ reentries' = IF ~Live(s) \/ reentries = 2 THEN reentries
                    ELSE reentries + 1
    /\ UNCHANGED <<phase, childRev, published, paste, service, geometry,
                   exitNotes, keysToChild, staleAccepted, autoInstalled,
                   resurrected, routedToDead, hostEffects,
                   childResizeApplied, refreshes, lateDropped, denied,
                   closings>>

\* D2's explicit :terminal-refresh: reinstall the latest published frame
\* into the inspected view. Explicit-only.
Refresh(s) ==
    /\ inspecting[s]
    /\ published[s] > 0
    /\ refreshes < 2
    /\ projected' = [projected EXCEPT ![s] = published[s]]
    /\ refreshes' = refreshes + 1
    /\ UNCHANGED <<phase, childRev, published, inspecting, inputMode, paste,
                   service, geometry, exitNotes, keysToChild, staleAccepted,
                   autoInstalled, resurrected, routedToDead, hostEffects,
                   childResizeApplied, lateDropped, denied, reentries,
                   closings>>

\* A key lands on the terminal: routed to the child only while a pane
\* owns terminal-input AND the session is live. A dead session is never
\* addressed (honestly unreachable: the exit boundary cleared the pane's
\* terminal-input; MUTATION 4 leaves it set and the key routes to the
\* dead child).
InputKey(s) ==
    /\ inputMode[s]
    /\ keysToChild < 2
    /\ IF Live(s)
       THEN /\ keysToChild' = keysToChild + 1
            /\ UNCHANGED routedToDead
       ELSE /\ MUTATION = 4
            /\ routedToDead' = 1
            /\ UNCHANGED keysToChild
    /\ UNCHANGED <<phase, childRev, published, projected, inspecting,
                   inputMode, paste, service, geometry, exitNotes,
                   staleAccepted, autoInstalled, resurrected, hostEffects,
                   childResizeApplied, refreshes, lateDropped, denied,
                   reentries, closings>>

\* Child output requests a host effect (OSC clipboard write / host
\* control). Honest: denied, recorded as a message. MUTATION 5 applies it.
ChildEffect(s) ==
    /\ Live(s)
    /\ denied < 2
    /\ IF MUTATION = 5
       THEN /\ hostEffects' = 1
            /\ UNCHANGED denied
       ELSE /\ denied' = denied + 1
            /\ UNCHANGED hostEffects
    /\ UNCHANGED <<phase, childRev, published, projected, inspecting,
                   inputMode, paste, service, geometry, exitNotes,
                   keysToChild, staleAccepted, autoInstalled, resurrected,
                   routedToDead, childResizeApplied, refreshes, lateDropped,
                   reentries, closings>>

\* The child asks for a new size. One geometry owner per session (T09):
\* the request is never applied — the honest model has no step for it
\* (a no-op is stuttering). MUTATION 6 honors it.
ChildResize(s) ==
    /\ MUTATION = 6
    /\ Live(s)
    /\ IF MUTATION = 6
       THEN /\ geometry' = [geometry EXCEPT ![s] = "g2"]
            /\ childResizeApplied' = 1
       ELSE UNCHANGED <<geometry, childResizeApplied>>
    /\ UNCHANGED <<phase, childRev, published, projected, inspecting,
                   inputMode, paste, service, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, resurrected, routedToDead,
                   hostEffects, refreshes, lateDropped, denied, reentries,
                   closings>>

\* The editor — the one resize authority — admits a viewport change.
EditorResize(s) ==
    /\ phase[s] # "none"
    /\ geometry[s] = "g1"
    /\ geometry' = [geometry EXCEPT ![s] = "g2"]
    /\ UNCHANGED <<phase, childRev, published, projected, inspecting,
                   inputMode, paste, service, exitNotes, keysToChild,
                   staleAccepted, autoInstalled, resurrected, routedToDead,
                   hostEffects, childResizeApplied, refreshes, lateDropped,
                   denied, reentries, closings>>

\* An update arrives after the session ended: the service handle is
\* gone, so it is dropped without effect.
LateUpdate(s) ==
    /\ Terminal(s)
    /\ lateDropped < 2
    /\ lateDropped' = lateDropped + 1
    /\ UNCHANGED <<phase, childRev, published, projected, inspecting,
                   inputMode, paste, service, geometry, exitNotes,
                   keysToChild, staleAccepted, autoInstalled, resurrected,
                   routedToDead, hostEffects, childResizeApplied, refreshes,
                   denied, reentries, closings>>

Next ==
    \/ \E s \in SESSIONS : Launch(s)
    \/ \E s \in SESSIONS : ChildStart(s)
    \/ \E s \in SESSIONS : ChildOutput(s)
    \/ \E s \in SESSIONS : PublishFrame(s)
    \/ \E s \in SESSIONS : ChildExit(s)
    \/ \E s \in SESSIONS : Resurrect(s)
    \/ \E s \in SESSIONS : CloseTerminal(s)
    \/ \E s \in SESSIONS : PasteRequest(s)
    \/ \E s \in SESSIONS : InspectEnter(s)
    \/ \E s \in SESSIONS : InspectExit(s)
    \/ \E s \in SESSIONS : Refresh(s)
    \/ \E s \in SESSIONS : InputKey(s)
    \/ \E s \in SESSIONS : ChildEffect(s)
    \/ \E s \in SESSIONS : ChildResize(s)
    \/ \E s \in SESSIONS : EditorResize(s)
    \/ \E s \in SESSIONS : LateUpdate(s)

vars == <<phase, childRev, published, projected, inspecting, inputMode,
          paste, service, geometry, exitNotes, keysToChild, staleAccepted,
          autoInstalled, resurrected, routedToDead, hostEffects,
          childResizeApplied, refreshes, lateDropped, denied, reentries,
          closings>>

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* VF12 named safety properties.                                          *)
(***************************************************************************)

\* Accepted frames never exceed what the child produced, and no stale or
\* foreign publication was ever accepted.
FrameFresh ==
    /\ \A s \in SESSIONS : published[s] <= childRev[s]
    /\ staleAccepted = 0

\* Output never drags an inspected (pinned) view.
PinnedNeverDragged == autoInstalled = 0

\* Terminal phases are absorbing.
TerminalAbsorbing == resurrected = 0

\* The exit boundary: a terminal phase implies no paste ticket, no
\* service handle and no pane in terminal-input.
ExitBoundary ==
    \A s \in SESSIONS : Terminal(s) =>
        /\ ~paste[s]
        /\ ~service[s]
        /\ ~inputMode[s]

\* One truthful terminal announcement per session.
ExitAnnouncedOnce == \A s \in SESSIONS : exitNotes[s] <= 1

\* No key is routed to a dead session.
InputNeverToDead == routedToDead = 0

\* The editor is the one geometry owner.
GeometryOwned == childResizeApplied = 0

\* Child output never becomes a host effect.
OutputNeverMutates == hostEffects = 0

(***************************************************************************)
(* Non-vacuity witnesses: each must be REACHABLE in the honest model      *)
(* (checked as an expected-to-fail invariant over the coverage config).   *)
(***************************************************************************)

\* A pinned view lags live output (the "new output" indicator case).
WitnessNoPinnedLag ==
    ~(\E s \in SESSIONS : inspecting[s] /\ published[s] > projected[s])

\* An explicit refresh re-installs the latest frame.
WitnessNoRefresh == refreshes = 0

\* A session exits with its final output retained in the projection.
WitnessNoExitRetained ==
    ~(\E s \in SESSIONS : Terminal(s) /\ projected[s] > 0)

\* A child host-effect request is denied.
WitnessNoDenied == denied = 0

\* An update arriving after exit is dropped.
WitnessNoLateDrop == lateDropped = 0

\* A pane leaves inspection back to input and inspects again.
WitnessNoReentry == reentries = 0

\* An editor-initiated close passes through Closing.
WitnessNoClosing == ~(\E s \in SESSIONS : phase[s] = "closing")

\* Two sessions run concurrently (identity separation is exercised).
WitnessNoTwoSessions ==
    ~(\E s1, s2 \in SESSIONS : s1 # s2 /\ Live(s1) /\ Live(s2))

\* A paste ticket is outstanding and cleared by the exit boundary.
WitnessNoPasteBoundary ==
    ~(\E s \in SESSIONS : Terminal(s) /\ exitNotes[s] > 0 /\ ~paste[s])

=============================================================================
