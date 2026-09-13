# 0060 — Debugger: complete journeys, owned sessions and reusable presentation

Status: requested architectural/release handoff; implementation is not claimed.
This supersedes the debugger research in [0019](0019-debugger.md). Deliver the
specified TUI debugger before the final GUI parity arc in
[0061](0061-gui-windows-and-wsl.md). Integrated local program terminals consume
[0055's TUI milestone](0055-embedded-terminal-tui-and-gui.md); DAP's debug-console
REPL is a separate facility and does not require terminal emulation.

**Entry gate:** [0056](0056-architecture-prerequisites.md) AR01–AR16,
[0057](0057-core-verification-and-assurance.md) VF01–VF20,
[0058 worker](0058-unified-native-worker.md) WK01–WK20 and
[0059 completion](0059-nonblocking-code-completion.md) C01–C09 are complete first.
Architecture and baseline verification remain separate; the worker release then
unifies native services and requalifies its changes. This plan adds debugger behavior
through those contracts, not generic repairs, another helper or deferred core proofs.
DBG01–DBG16 acceptance remains unchanged.

## 1. Decision and release boundary

Use **DAP, with two supported adapter families tested deeply**:

- Rust and C/C++: LLVM **lldb-dap**, with LLVM 21+ as the initial supported baseline.
- Python: **debugpy**, using the selected execution environment's interpreter and
  a pinned tested 1.8-series-or-newer release in fixtures.

The client protocol and presentation remain adapter-independent; profile discovery,
launch arguments, pretty-printer setup and known capabilities live in small adapter
profiles. Do not import Zed's GPL debugger implementation or build a generic plugin
runtime to add two profiles.

The release includes local launch/attach, real SSH debugging and debugging inside
already-running containers under explicit execution capabilities. It does not
require container provisioning, public unauthenticated debug ports, automatic
adapter installation, remote PTYs or every language adapter. Basic independent
multi-session switching is included; automatically spawned child-session trees are
not implied.

### Required delivery ledger

All DBG requirements belong to this release. A severity label or a working local
demo does not authorize dropping the SSH/container, failure or integration paths.

| ID | Required result |
| --- | --- |
| DBG01 | Contextual launch/configuration discovery, adapter health/diagnostics, explicit build preparation and truthful zero-config Rust/Python journeys. |
| DBG02 | Primary `.strop/debug.json`, user profiles and checked `.vscode/launch.json` import; no silently ignored launch/build/variable requirements. |
| DBG03 | Bounded typed DAP client over the released framing/service contracts and 0058 worker streams, with correct initialize/configuration/launch sequencing, reverse requests and outcomes. |
| DBG04 | Source breakpoints with requested/verified/relocated/disabled/error state, conditions, hit conditions, logpoints and safe persistence. |
| DBG05 | Capability-driven exception filters and useful exception-stop details. |
| DBG06 | Threads, stack, selected-frame scopes and lazy/paged variable inspection with correct stopped-state lifetimes. |
| DBG07 | Watches, explicit evaluation, debug-console REPL, supported completion and explicit variable/expression editing. |
| DBG08 | Program output distinct from adapter logs/debug console; real local integrated-terminal `runInTerminal` through 0055. |
| DBG09 | Continue, pause, step over/in/out, restart and correct launch-stop versus attach-detach semantics. |
| DBG10 | Source identity/path mapping/sourceReference handling, dirty-source honesty and deliberate source-pane navigation/return. |
| DBG11 | Polished responsive TUI debugger workspace, real navigable buffers, portable keybindings and complete contextual help. |
| DBG12 | Real supported SSH debugging through 0058's authenticated native worker/context/lease contract, with no local-path fallback. |
| DBG13 | Real supported running-container debugging through 0058's selected-context/incarnation/native-supervision contract, with correct PIDs and cleanup. |
| DBG14 | Independent session ownership/switching, generation-keyed work, bounded physical requests and honest cancellation/disconnection. |
| DBG15 | Private capture/replay policy, explicit trust and no automatic target execution or attach on restore. |
| DBG16 | Readable crate/module split and full migration, required 0057/0058 proof/core-assurance gates, real adapters/transports, actual TUI evidence and release readiness. |

Each DBG ID gets an integration owner, implementation paths, exercised cases,
outcome/failure semantics and a checked status. Moving required work out needs
explicit user approval and a [0028](0028-roadmap-and-review.md) entry. The explicit
extension envelope in §14 is not a place to hide unfinished DBG requirements.

## 2. Research corrections and current evidence

The older 0019 made useful adapter/UI choices but contained unsafe simplifications:

- DAP resembles LSP's framing/service topology, but is **not JSON-RPC/LSP**. It has
  its own seq/request_seq envelopes, event semantics and reverse requests.
- A debug-console REPL uses DAP `evaluate` with context `repl`; it is not a terminal
  emulator problem. Program stdin/TTY behavior is the separate terminal problem.
- Ignoring `preLaunchTask` can debug an outdated executable. Unsupported imported
  task references must be diagnosed before launch, not silently dropped.
- VS Code `type: "lldb"` normally denotes CodeLLDB, not LLVM's `lldb-dap` profile.
  Do not silently relabel adapter-specific options.
- A compile_commands database describes compilation, not executable artifacts.
  C++ cannot honestly promise arbitrary-project zero-config launch from it alone.
- Multi-session state, exception controls and process selection are manageable
  scoped capabilities, not reasons to bake a single global debug frame into the
  engine.

### Grounding in this repository

At inspection the workspace version was 0.30.0 and another session was implementing
0054, including `strop-fs` and shared Directory sources. No project build/formatter
was run against that in-flight work.

The generic gaps below are pre-0056 evidence, not work assigned to this debugger
release. Their closure is an entry requirement; preserve the remaining domain
constraints when implementing debugger-specific behavior.

Reuse these actual owners:

- `strop-engine` owns semantic Editor state/services; its normal dependency list
  has no Ratatui/Crossterm. Its public mutable surface is still broad: avoid adding
  another bypass through the debugger.
- `editor/document/surfaces.rs` now has typed Directory and Output sources with
  view return records. Debug generated buffers should follow that ownership model.
- `editor/changes`, `transact`, `jumps`, `analysis` and source-backed collections
  already own edits, source observations, view returns and revision-aware work.
- The pre-worker SSH implementation used `command_supervised` and a Python-owned
  stdin lease. 0058 supersedes that implementation, preserving its qualified cleanup
  contract through the shared native worker; do not call the historical API here.
- The inspected container exec path's client/EOF comments were not cleanup proof.
  0056 closes the real lease/context gap; 0058 migrates/requalifies it before DAP.
- `Space d` is the diagnostics picker. **`Space D` is the debugger namespace**;
  do not steal diagnostics or ordinary Vim keys.

### Disposable runtime walkthrough

A small private C++ program was compiled with `g++ -g -O0`. A native debugger
walkthrough hit `multiply`, observed local `result = 42`, stepped back to `main`
and exited 0. The installed concrete LLVM adapter reported LLVM/lldb 21.0.0;
through lldb-dap it also hit the source breakpoint, evaluated `result` to 42 and
returned a stack containing `multiply`, `main` and runtime frames.

The shell-visible clang++/lldb-dap names initially resolved to a Swift toolchain
shim without a selected toolchain. The actual installed toolchain worked with a
probe-local selection. **Executable discovery is not health/readiness proof**;
report this distinction without changing the user's global toolchain settings.
Python debugpy was not installed in the inspected Python environment. No package
was installed merely to make this research appear complete. Python/SSH/container
interoperability remains mandatory implementation evidence, not a claim here.

## 3. Launch configurations and discovery

### One small configuration contract

Retain 0019's primary `.strop/debug.json` format: an array of profiles with
`adapter`, `label`, `request` and adapter-specific launch/attach options. Support
JSON comments/trailing commas as a bounded documented JSONC syntax; do not execute
configuration files. User profiles live in `$XDG_CONFIG_HOME/strop/debug.json`
(default `~/.config/strop/debug.json`).

A primary project file is authoritative even when empty; an invalid file is an
error, not permission to silently execute a VS Code fallback. Without that file,
import `.vscode/launch.json`'s supported configurations. Merge user profiles as
separately labelled origins; qualify duplicate display labels across origins and
reject duplicates within one file. Internal identity is origin + stable profile
label/ID, never array position alone.

Required adapters are named `lldb-dap` and `debugpy`. LLVM import accepts
`type: "lldb-dap"`; debugpy import accepts its current type and explicitly documented
legacy Python forms. CodeLLDB, cppdbg/cppvsdbg, compounds and extension command
variables are not silently converted. An import diagnostic names the unsupported
field/type and offers editing a Strop profile, not launching a guessed subset.

Example launch profile (program/build names are illustrative):

```json
[
  {
    "adapter": "lldb-dap",
    "label": "app — Debug",
    "request": "launch",
    "program": "${workspaceFolder}/build/app",
    "cwd": "${workspaceFolder}",
    "args": ["--fixture", "fixtures/input with spaces.json"],
    "build": {
      "command": "cmake",
      "args": ["--build", "${workspaceFolder}/build", "--config", "Debug"]
    },
    "console": "integratedTerminal"
  }
]
```

Strop owns profile metadata, execution context, build preparation and supported
variable substitution. Adapter-owned fields remain adapter data with located
validation for known fields; don't try to generically interpret every string as a
path or command. Hook fields such as LLDB initCommands are executable authority
and appear in the trust/launch review.

Supported substitutions have one resolver: workspaceFolder, active file/file
parent/relative file where meaningful, and named environment values from the
**selected execution namespace**. Missing values and incompatible active-file
origins are errors, not empty strings. No arbitrary `${command:...}` execution;
known process-picker requests resolve through Strop's own explicit picker.

`preLaunchTask` may map only to an explicitly configured equivalent Strop build
step. Otherwise reject with a repair path. No tasks.json interpreter, arbitrary
task-label guessing or implicit build success. Build commands are argv arrays;
requesting a shell command is a separately explicit authority, not path quoting.

### Contextual “Start debugging” picker

Show ready profiles/scenarios, origin, language, namespace, target and preparation
status. Missing adapters, invalid config and unsupported targets remain visible
with actionable diagnostics. Do not advertise Ready before a bounded version/
capability probe succeeds in the correct namespace.

Discovery is bounded and owned, never a workspace scan or build on the input
thread. Prefer read-only existing metadata; any configure/build/probe that executes
project/toolchain code uses the existing trust boundary.

| Language | No-profile journey | What must not be guessed |
| --- | --- | --- |
| Rust | Cargo package/bin/test metadata; choose a target/test, run the captured build/no-run step, select the matching compiler-artifact executable, launch through LLVM. | Don't scrape target/debug/deps filenames or choose the first executable. Package/target/features/profile/test identity must match. |
| Python | Selected/explicit interpreter, current file or chosen module; validate debugpy in that environment, then launch with captured cwd/args. | Don't install into or silently switch to a different global interpreter when the selected environment is unavailable. |
| C/C++ | Existing CMake file-API codemodel/artifacts or an explicit executable/profile; offer a configured build step. | compile_commands does not establish the link target; don't promise arbitrary C++ zero-config debugging. |

Use Cargo's versioned metadata and JSON compiler-artifact records. Preserve
non-Cargo stdout as build output; success requires the process/build-finished
outcome and the correct real artifact, not the presence of one JSON line. A stale
artifact from a failed build is not a successful launch target.

CMake reply files are CMake-owned and read through a reply index. If codemodel data
is absent, writing a client-strop query and running configure is an explicit setup
step; never do it just because the picker opened and never delete shared replies.
For Rust, integrate the selected toolchain's LLDB pretty-printer initialization
(`rustc --print sysroot`/rust-lldb's lldb_lookup.py contract). Missing/incompatible
printers produce a clear raw-value limitation, not invented pretty values.

## 4. Full user journeys and trigger points

### A. Debug a program or test

1. From a real source/excerpt, invoke `Space D s` or F5. Capture the active source,
   namespace, project and original view; open the scenario picker.
2. Select a scenario. Resolve config and show adapter, program, cwd, arguments,
   build step and authority. Trust grants are explicit and scoped; project files
   do not grant themselves execution permission.
3. Resolve unsaved relevant sources: **Save and prepare**, **Debug saved version**,
   or Cancel. Never silently save, discard or reload dirty text. If debugging an
   older disk/build view, surface that mismatch under §9.
4. Execute the owned build/preparation step if needed. Show a real searchable
   output buffer, progress/cancel and precise errors. Failure stays at this step;
   it does not fall through to an old binary.
5. Start the adapter; configure desired breakpoints/exceptions under the protocol
   sequence in §7. A progress state is not a fake “running” session.
6. On a stop, show why, update the execution marker, load the selected thread's
   stack/scopes and reveal the source in the session's source pane. Do not steal
   another session's or the user's unrelated typing focus.
7. Inspect/step/evaluate. Resume invalidates old inspection handles immediately.
8. Stop or let the program finish. Preserve output/exit reason and restore the
   original layout/view without closing user-created panes or losing edits.

### B. Attach to an existing process

`Space D a` opens an explicit namespace/process/endpoint selector. Show PID,
executable/name and available start identity; support manual PID when enumeration
is unavailable. Revalidate before attach. A process name is not a stable PID and
container host PIDs are not container PIDs.

LLVM attaches by pid, with optional program for symbols. Python supports its
listen/connect workflow; direct PID injection is offered only when the environment
supports its injection/ptrace prerequisites. Never expose an unauthenticated DAP
listener on all interfaces by default. Use loopback inside the selected namespace
and authenticated relays for SSH/container targets.

Attached processes are not owned debuggees. Default Stop **detaches** and resumes
them according to the adapter contract; explicit termination requires separate
user intent. No build, restart, kill-container or elevation is implied by Attach.

### C. Pause, inspect and change a value

Pause targets the selected session/thread according to negotiated execution
capabilities. Show Running/Pausing/Paused distinctly. Inspect stack → scopes →
children lazily. Show expensive scopes collapsed and named/indexed child counts.

Add a watch explicitly, evaluate a selection/word in the selected frame, or enter
a console expression. Watches authorize re-evaluation at subsequent valid stops;
automatic speculative evaluation on every mouse move is not the default. Values
may execute pretty-printers/getters/expressions: do not promise arbitrary evaluation
is pure or cancellable without effects.

Edit value is an explicit action, not text mutation of a generated variable row.
Use setVariable when supported; otherwise setExpression only with a real adapter-
provided evaluateName and capability. Do not construct an expression from an
ambiguous display label. Refresh/invalidate affected references after success;
uncertain mutations are not blindly retried.

### D. Exceptions, restart and failure

The exception selector is built from adapter-provided filters/defaults and supported
options. Show enabled filters and exceptionInfo when available. Don't hardcode
Python/C++ exception names or silently enable every filter.

Restart means rerun the captured launch profile, including its declared preparation
policy. Prefer DAP restart when supported and the newly prepared arguments/artifact
can be applied; otherwise perform a controlled owned launch restart. A failed
preparation never reports successful restart. Attach sessions are not implicitly
killed/restarted; offer explicit reattach where safe.

Lost adapter/SSH/container connection freezes current inspection data as stale,
shows the uncertain target state and retires handle authority. No reconnect followed
by replaying “continue”, “set value” or “launch” against a new session. Cancellation,
transport loss, target exit and adapter termination are different outcomes.

## 5. TUI components, layout and keys

The source document is the primary surface. Breakpoints and the execution point
are structured gutter decorations alongside Git/diagnostics, not another synthetic
source editor. A selected caller frame is not the actual current instruction;
show those markers/states distinctly.

Illustrative wide layout, **not an implementation capture**:

```text
Debug · app · local/Ubuntu · paused: breakpoint         Continue  Step  Stop
┌ src/main.rs ────────────────────────┬ Stack / Threads ─────────────────┐
│    40  let input = prepare();       │ > main            main.rs:42     │
│ b> 42  let result = calculate();    │   run             lib.rs:18      │
│    43  report(result);              ├ Variables / Watches ─────────────┤
│                                    │ ▸ input: Request                 │
│                                    │   result: 42                     │
├ Program output / Debug console / Adapter log ─────────────────────────┤
│ target output remains searchable; console expressions own a prompt    │
└──────────────────────────────────────────────────────────────────────┘
```

At 80×24, keep source + one useful auxiliary view, with explicit Stack/Variables/
Watches/Output/Console switching. Do not crush four panels into unreadable slivers.
Generated views remain real readonly buffers with typed row maps: motions, search,
yank and meaningful Enter/expand actions. Metadata, placeholders and group headers
cannot become fake frame/variable handles.

Use the existing card/field/tree/progress/receipt vocabulary. Strong source names,
quiet paths/types, aligned line information, bounded value clipping with disclosure,
a clear selected row and restrained raised/selected backgrounds. Loading, stale,
unavailable and error are not empty-success states. No per-cell parsing, full-tree
formatting or native request admission during paint.

### Portable canonical bindings

| Binding | Action |
| --- | --- |
| Space D s | Start/configure debugging. |
| Space D a | Attach to process/endpoint in the chosen namespace. |
| Space D b / Space D B | Toggle source breakpoint / edit condition-hit-log options. |
| Space D l / Space D e | Breakpoints / exception filters. |
| Space D c / Space D p | Continue / pause. |
| Space D n / i / o | Step over / in / out. |
| Space D r | Restart the selected launched session under its preparation policy. |
| Space D t / Space D d | Stop session with launch/attach-appropriate behavior / explicit detach where supported. |
| Space D f / Space D h | Stack frames / threads. |
| Space D v / Space D w / Space D W | Variables / watches / add watch expression. |
| Space D x | Explicit evaluation of selected text or current word, with editable expression. |
| Space D C / Space D O | Debug console / program output. |
| Space D S | Session selector. |

F5 start/continue, F6 pause, F9 breakpoint, F10 step over, F11 step in, Shift-F11
step out and Shift-F5 Stop are optional equivalent native key routes, not the only
way to operate. Register their exact decoded forms through the 0055-preserved input
path; do not assume ambiguous terminal Ctrl/Shift combinations work everywhere.
All bindings/actions and disabled reasons appear in common help. Space d remains
diagnostics. Terminal-input mode keeps its own child keys; leave it explicitly
before editor debugger chords.

The debug source pane is sticky: reuse a pane already showing that source, then
continue using the chosen pane during stepping. Respect preserveFocusHint and
background-session stops. Track the user's subsequent layout changes; teardown
removes only debugger-owned views and restores still-valid original view state.

## 6. State and source ownership

Use a session registry with one selected session, not global frame/variable fields.
A session owns launch/attach authority, transport incarnation, lifecycle, desired
breakpoints, thread states, selected frame, inspection requests, watches, output,
terminal association and source-pane return state.

Meaningful identities include:

- DebugSessionId + adapter/transport incarnation.
- LaunchAttemptId and immutable resolved launch context.
- StopEpoch; thread/frame/variable references scoped to that epoch/session.
- DesiredBreakpointRevision per resolved source; requested versus adapter binding.
- FrameSelectionEpoch, expression revision and variable-page owner.

Before any continue/step/restart request can run the target, retire old inspection
handles and show ResumePending. Do not wait for a continued event: DAP need not
emit one for a request that already implies continuation. A failed/uncertain resume
is not proof that all old handles are still valid; refresh only under an established
paused state.

Partial thread stops are real: allThreadsStopped missing/false does not mean every
thread is inspectable. Likewise allThreadsContinued has its own documented default.
Conservatively invalidate session inspection generations on relevant execution
transitions, then refetch paused-thread data. Thread IDs are not variable handles
and have a different lifetime.

Variable/stack data are paged on demand where supported. Never send count=0 while
claiming a bounded page: the protocol defines it as all variables. Enforce response,
text, depth and retained-memory limits even with a non-paging adapter. Truncation
is disclosed, not represented as “no more children.” Resolve cyclic/repeated object
references without recursively expanding forever.

## 7. DAP transport and lifecycle contract

Create `strop-dap` for the DAP client/protocol/service boundary, consuming 0056's
bounded framing, execution and lifecycle primitives. DAP envelopes and suspended-
state semantics remain this release's work; do not merge them with LSP or create
another generic RPC/supervision layer.

Required mechanics:

- Bounded ASCII headers and UTF-8 JSON bodies; byte Content-Length, malformed/
  duplicate/conflicting header handling, partial reads and owned writer ordering.
- Correct Request/Response/Event envelopes. Each actor's seq advances across its
  outgoing messages; response request_seq identifies the request. Incoming reverse
  requests are answered, not inserted into the client's pending-request map.
- Typed supported payloads and checked identifiers/coordinates; retain compatible
  unknown fields where appropriate. Unknown reverse requests get an explicit
  failure response, not a hang. Optional unsupported events do not invent state.
- Negotiated capabilities with absence=false; react to capability updates. Advertise
  only implemented client features for the selected namespace/profile.
- DAP columns/completion offsets use UTF-16 code units with the negotiated base;
  convert at the boundary to native buffer bytes. No LSP position-encoding
  negotiation is implied. Line/column 0/unknown source is not a real caret target.
- One shared owned async service/runtime rather than a fresh runtime per key.
  Bound actual requests, queues and retained responses, not just visible owners.

### Initialization sequence

```text
spawn/connect adapter
  -> initialize request
  <- initialize response/capabilities
  -> launch or attach request (may remain pending)
  <- initialized event
  -> desired setBreakpoints, supported exception/function configuration
  <- configuration responses
  -> configurationDone, if supported
  <- configurationDone response / launch-or-attach completion
  <- running/stopped/output events as the adapter progresses
```

Do not wait for the launch response before configuring breakpoints: adapters may
wait for configurationDone before answering launch. initialized can arrive at
legitimate points after initialize; model readiness as data, not one rigid happy-
path if-chain. Zero breakpoints and unsupported configurationDone are valid cases.
A stopped event can race request completion; preserve wire/session ordering.

### Breakpoints

setBreakpoints replaces the complete set for one source. Coalesce desired updates
per source, serialize their physical requests and reconcile only the matching
revision. Removing the last breakpoint still sends an empty set. A late response
must not resurrect a removed breakpoint or relabel a requested position as verified.

Keep requested and actual positions and adapter IDs separate. Unverified is a real
state; later breakpoint events may verify, move or reject it. Conditions/hit/log
syntax is adapter-specific and advertised only with the matching capability.
Persist user definitions privately, not adapter IDs or verified status; restore
creates no debug process or attach authority.

### Cancellation and termination

cancel is best effort; the original request still owns a terminal response.
Cancelling inspection revokes UI interest, not the process. Cancelling an evaluation
cannot be claimed to undo its effects. Preserve actual terminal responses and
progressEnd, and show a visible failed/uncertain operation where relevant.

For launched debuggees, use supported graceful terminate then explicit force policy;
otherwise disconnect follows the adapter's launched-target semantics. For attached
targets, disconnect/detach must not kill the target. terminate, disconnect, exited
and terminated are distinct messages. Retire/reap the adapter only under the owned
lifecycle, not because the debug panel closed.

## 8. Console, program I/O and reverse requests

There are three distinct streams:

1. **Debug console**: expression prompt/history plus evaluate/repl results.
2. **Program output/I/O**: DAP output events for noninteractive runs, or a real
   0055 terminal for interactive local programs.
3. **Adapter diagnostics**: stderr/protocol diagnostics, not program stdout and
   never DAP framing noise injected into the UI.

The REPL uses selected session/frame ownership; supported DAP completions use the
same field/list component as other completion, with their own request/coordinate
contract. Enter in this prompt evaluates, not edits source or sends to program
stdin. Expression history/capture is private and can contain secrets.

runInTerminal is admitted only for an active authorized launch/restart attempt.
Use captured namespace, cwd and exact argv; args[0] is the program and env values
of null remove variables. Do not concatenate an unescaped shell command. A trusted
adapter may launch its own helper rather than the configured inferior directly;
validate authority/lifetime, not an incorrect argv[0]==program assumption.

Reply only after a real spawn with correct PID meaning; a local docker/ssh client
PID is not the inferior's PID. Use one terminal/process owner shared with 0055,
not a second debugger-only PTY implementation. A cancelled launch cannot leave an
orphan terminal or report a spawn success for a different attempt.

The first client advertises integrated local terminal support only where usable.
Remote/container generated profiles use explicit noninteractive console settings;
requesting unsupported interactive/external terminal behavior is an early refusal,
not a silent fallback. Remote PTYs are a declared later capability.

Do not advertise startDebugging/automatic child sessions until implemented. For
Python's generated profiles explicitly disable subprocess auto-debugging; imported
profiles that require it get a diagnostic rather than a covert option rewrite.
Basic manually started independent sessions still work.

## 9. Sources, edits and filesystem relocation

DAP Source is not just a filename. Resolve native/URI path formats and source maps
inside the session namespace. sourceReference is a session-owned source request;
its content opens as a readonly debug-source document, never as a made-up local
path. Missing source is an honest source-info view with stack/address context,
not a jump to line 1 of an unrelated file.

Reuse 0042 resource identity and the existing open/jump/view contract. Map path
components, not string prefixes. A remote `/src/main.cpp` remains remote; an
in-container path remains in the selected container incarnation. Do not guess a
host bind mount or fall back to a same-spelled local file. Refuse unrepresentable
native paths at a UTF-8-only DAP boundary rather than using a lossy alias.

Source text and executable debug information can diverge. A source edit is not
hot reload. Bind breakpoints/execution locations to their source observation;
track changes through the existing edit journal only when mapping is unambiguous.
Otherwise disclose stale/mismatched source and offer the saved/debug-source view
or save/rebuild/restart. Don't move an instruction arrow onto newly inserted code
and imply the target executed it.

A 0054 rename preserves DocumentId but changes the resource binding. Update desired
breakpoint/source metadata, invalidate affected requests and distinguish the old
compiled path from the new editor path. Do not silently rewrite debug symbols or
overwrite dirty buffers to make a frame jump succeed. Generated sources/readonly
snapshots do not gain normal filesystem write authority or restored remote permits.

## 10. Remote and container composition

| Concern | Local / WSL | SSH | Existing container |
| --- | --- | --- | --- |
| Adapter and build | Native worker executes captured local argv/cwd/environment | Same worker protocol in the selected authenticated endpoint/root | Same worker inside the pinned container/user/cwd under debug execution authority |
| DAP stdio | Worker-owned pipes exposed as ordered data streams | Same framed worker streams; stderr cannot impersonate control | Same in-container native supervision, not Docker-client lifetime assumptions |
| Attach PID | OS PID + available birth observation | Remote PID in that endpoint | PID as seen inside the container, not docker top's host PID |
| Source | Resolved local location | Same remote resource namespace | Same container incarnation; explicit mappings only |
| Interactive I/O | 0055 integrated terminal | Later remote PTY capability; noninteractive debugging required now | Same bounded first-release distinction |
| Stop/detach | Ownership determines behavior | Same, with transport-loss uncertainty | Same; never stop/restart/remove the container as debugger cleanup |

All three paths consume 0058's real native worker protocol, deployment/handshake,
captured context and owned execution lease. SSH retains OpenSSH authentication and
native argument semantics. A partition can delay cleanup until dead-peer detection,
and descendants that deliberately escape the supervised group remain outside that
qualified guarantee. Do not reintroduce Python discovery or promise universal kill.

Container stdio consumes the native supervision migrated and requalified by 0058
from 0056 AR07. Debugger-specific launch/attach, PID/source interpretation and stop/
detach behavior live here. No SSH daemon or shell-interpolated path, and no assumption
that killing `docker exec` kills the adapter. An incomplete worker lease/incarnation
contract means the prerequisite gate has not passed.

Container debugging is a new scoped **execution** capability, not a change making
all container file operations writable or a sandbox guarantee. Running a debuggee
can modify its environment; say so. Missing ptrace permissions, CAP_SYS_PTRACE,
seccomp allowance, worker capability or adapter produce concrete guidance. Do not recreate
containers, disable seccomp, elevate or install packages automatically.

Already-listening debug endpoints are loopback-scoped in their own namespace.
Bridge/tunnel them through the authenticated execution boundary; don't open a
public unauthenticated debugpy port to avoid implementing routing. Target addresses,
credentials and path maps never come from a rendered stack row.

## 11. Crates, modules and cross-task contracts

0056 owns the shared infrastructure below. Only the DAP client and debugger-domain
engine/presentation extensions are new feature work in this release.

Recommended ownership, adjusted to the final in-flight file layout:

```text
strop-core / strop-workspace
    released 0056 identities, coordinates, framing, workers and resource contracts
strop-dap
    DAP envelopes, typed protocol, requests/capabilities and client service
strop-engine/editor/debug/
    configuration/discovery/build, sessions/lifecycle, breakpoints,
    threads/stack/scopes, variables/watches/evaluation, console/output,
    sources/navigation, namespace admission and presentation models
strop-remote / strop-containers / strop-terminal
    concrete execution/lease/PTY ownership, not debug UI
strop/render/debug/ then strop-gui/components/debug/
    presentation and input adaptation over the same semantic owners
```

One new DAP crate is sufficient initially; do not add separate framework, plugin,
task and debugger-model crates merely to draw boxes. Shared framing/supervision
is already owned by 0056 and reused by current consumers. A remaining generic
gap returns to that owner instead of acquiring a debugger-only implementation.

Break configuration, lifecycle, inspection, breakpoint, source and rendering work
into files by concern. No thousand-line debug.rs or renderer that issues DAP
requests. TUI/GUI views receive prepared readonly data and issue validated actions.
Use LSP references before exported changes; no obsolete aliases or unowned direct
calls to buffers/services left behind.

Shared contracts must be decided before parallel implementation: session/stop IDs,
execution context, request/outcome shape, breakpoint desired/bound distinction,
terminal ownership and debug-source projection. One integration owner owns admission
and lifecycle; sibling implementations do not invent separate receipt/capability enums.

## 12. Privacy, replay and error policy

Use 0056 AR08's admission/effect/capture classification. The following are
debugger-domain additions and acceptance checks, not a second privacy mechanism.

Debugging executes and can inspect/mutate program state. Config/build/hooks,
expression evaluation, attach and termination are explicit authorities. Merely
opening a project, restoring a breakpoint list or viewing config grants none.

Do not log full env, adapter traffic, watch values, expressions, memory or console
payloads by default. DAP logs and UI/frame captures can contain secrets; cover all
capture paths, not just request arguments. Private opt-in full capture must say
what is included. Metadata-only replay cannot claim identical secret value views.

Recorded sessions replay native-free: no adapter/build/process attach/terminal
spawn, host effects or filesystem operation. Restore user definitions/views only;
adapter handles, execution authority and live process ownership never deserialize
as a valid running session. Wrong namespace, stale session and unsupported capability
are named refusals, not empty values or unrelated installation advice.

## 13. Implementation and evidence gate

Entry requires completed 0056 architecture, 0057 baseline verification, 0058 native
worker/assurance migration and 0059 completion, with concrete maintained contracts.
Then implement DAP protocol/freshness → configuration/discovery/build → local
adapter journeys → source/gutter/views → watches/console/terminal → real SSH/
container debugger integration → multi-session/domain privacy → release evidence.
Independent work uses the shared contracts above; every DBG ID remains required.

### Extend the assured core with real debugger evidence

0057 delivers the human/TLA+ DAP reference contract, calibrated scenarios and the
whole-core proof/correspondence framework after architecture stabilizes. 0058 then
rebinds native execution/streams/context/deployment to its shared Rust worker. This
release binds the **actual DAP client and its editor/native effects** to that current
baseline. Model-only status or an inherited worker proof is not adapter verification.

Add exact production symbols, admitted observations and real adapter traces to the
guarantee ledger. Required DAP claims cover initialize/configuration/launch ordering,
complete-set breakpoint updates, session/stop/frame/variable lifetime, one-shot
responses, reverse-launch authority and attach-versus-launch stop semantics. Prove
the selected production admission/transition kernels through the shared Verus
boundary and run calibrated model/byte/runtime correspondence campaigns. Keep
third-party adapter/OS assumptions and any unproved model-to-code mapping explicit.

Run the required `model`, `verify`, `tlaps` and `core-assurance` gates on the
final candidate, alongside real LLVM/debugpy and SSH/container/terminal journeys.
An unsupported proof construct, solver failure or a mutation that never applied
does not pass. A new DAP capability cannot ship by merely inheriting the old model's
badge; its changed contract, proofs/calibration and implementation evidence are required.


Keep deterministic tests for:

- initialize/initialized/launch/configurationDone permutations, zero breakpoints,
  delayed launch response, failed setup and reverse requests;
- stale/cancelled stack/scopes/variables/evaluation after resume or frame/session
  switch, partial-thread stops and no continued event after continue;
- complete-set breakpoint replacement, empty-set removal, coalescing, relocation,
  unverified→verified changes and source edits/renames;
- UTF-16/byte boundaries, sourceReference content, nonexistent/foreign paths,
  expensive/paged/cyclic variables and value mutation invalidation;
- attach versus launch stop, restart/preparation failure, lost acknowledgements,
  process/adapter exit ordering, bounded output and private capture/replay;
- pending launch cancelled before/after runInTerminal spawn, exact argv/env and
  namespace/PID correctness, with real owned terminal outcomes.

Real implementation walkthroughs must include Rust binary + single test, C++
program and Python file/module; conditional/log/exception stops; step/stack/variables/
watch/evaluate/value edit; clean stop and attach-detach. Run SSH and container
fixtures that actually launch the chosen adapters/targets, inspect source and
prove owned cleanup without killing an attached process or the container.

Use fake adapters for adversarial ordering, not as substitutes for these real
adapter/transport tests. Golden cell-grid checks protect semantic gutter/selected/
stale/error distinctions. Actual TUI interaction at 140×40, 100×30 and 80×24 must
exercise layout, focus, nested terminal input, navigation/return and failures.
No screenshot mockups or successful spawn alone count as this evidence.

Run the integrated `docker compose run --build --rm test` gate (fmt, locked
all-target clippy/tests), all required 0057 proof/core-assurance lanes, pinned
adapter/SSH/container and package/static gates. Keep rustls-only/static rules.
Update existing help/docs/changelog, remove disposable scaffolding and complete
the DBG ledger before release. Do not validate another session's half-edited tree
for the present documentation-only task.

## 14. Explicit extension envelope

These are outside this first debugger release, not authorized removal of DBG work:

| Extension | Re-entry condition |
| --- | --- |
| More adapter families, CodeLLDB-specific compatibility, Go/JS/.NET/Java | Concrete user demand, profile/schema/transport fixtures and real full journeys. No name-based LLVM substitution. |
| Adapter-managed subprocess/child-session trees, compounds | startDebugging authority, parent/child lifetime and aggregate stop/restart contracts. Manual independent sessions remain required. |
| Remote/container interactive PTYs | 0055 transport/PTY capability and cleanup evidence; no public ports or local-terminal fallback. Noninteractive remote/container debugging ships now. |
| Reverse/time-travel debugging, instruction/data breakpoints, registers/memory/disassembly UI | Capability-specific data/lifetime/safety and useful navigation contract. Source-unavailable diagnostics remain required now. |
| Arbitrary tasks.json or extension-command evaluation | Separate task/runtime scope; unsupported preLaunchTask still fails before launch. |
| Automatic adapter/toolchain install, elevation, container provisioning | Distribution/trust/licensing and explicit execution ownership; missing tools remain actionable diagnostics. |
| Live source hot reload, automatic inline values, hover evaluation on every movement | Adapter-specific correctness, bounded work and side-effect policy; never fake source/executable agreement. |
| Production/core-dump/remote debug-server special modes | Explicit source/symbol/authority and recovery contract; not inferred from a generic attach field. |

## 15. Primary references

- [DAP overview and lifecycle](https://microsoft.github.io/debug-adapter-protocol/overview)
  and [schema](https://github.com/microsoft/debug-adapter-protocol/blob/main/debugAdapterProtocol.json):
  framing, sequencing, references, UTF-16, breakpoints, reverse requests and stop semantics.
- [LLVM lldb-dap](https://lldb.llvm.org/use/lldbdap.html): actual profile fields,
  console support from LLVM 21, capabilities, command/evaluation distinction.
- [debugpy settings](https://github.com/microsoft/debugpy/wiki/Debug-configuration-settings)
  and [CLI/attach reference](https://github.com/microsoft/debugpy/wiki/Command-Line-Reference):
  interpreter, console, processId/listen/connect and subprocess/security boundaries.
- [Zed debugger](https://zed.dev/docs/debugger): contextual scenarios, config/build
  flows, breakpoint options and sticky source-pane navigation. Inspiration, not code reuse.
- [Cargo external-tools JSON](https://doc.rust-lang.org/cargo/reference/external-tools.html)
  and [CMake file API](https://cmake.org/cmake/help/latest/manual/cmake-file-api.7.html):
  artifact discovery rather than filenames or compile-command guesses.
- [Rust rust-lldb wrapper](https://github.com/rust-lang/rust/blob/master/src/etc/rust-lldb):
  toolchain/sysroot pretty-printer initialization.
- [0055](0055-embedded-terminal-tui-and-gui.md), [0042](0042-shared-resource-identity.md),
  [0043](0043-change-plans.md), [0046](0046-engine-extraction.md) and
  [0036](0036-remote-workspace-execution.md): existing Strop ownership contracts.
- [0056](0056-architecture-prerequisites.md): completed shared-architecture entry
  contract, rather than generic fixes deferred into this debugger release.
- [0057](0057-core-verification-and-assurance.md): separate whole-core assurance
  baseline and mandatory feature-change impact, not a debugger-only protocol audit.
- [0059](0059-nonblocking-code-completion.md): preceding completion release.
- [0058](0058-unified-native-worker.md): completed native worker, leased execution/
  deployment protocol and assurance migration; no debugger-specific Python supervisor.
